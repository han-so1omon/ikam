//! A small HTTP/1.1 server for `ikam live`: keep-alive, Content-Length and
//! chunked bodies, `Expect: 100-continue`, one thread per connection. Only
//! 127.0.0.1 is bound. `Limits` bounds what one client can make it hold: a
//! body over `max_body` gets 413 without being read, a connection past
//! `max_conns` gets 503, and header lines and counts are capped always.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// A header line longer than this, or more headers than `MAX_HEADERS`, ends
/// the connection with 431.
const MAX_LINE: usize = 16 << 10;
const MAX_HEADERS: usize = 100;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest request body read, in bytes; none for no limit.
    pub max_body: Option<usize>,
    /// Connections served at once; more get 503 and are closed.
    pub max_conns: Option<usize>,
    /// How long a connection may sit idle between requests.
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_body: None,
            max_conns: None,
            idle: Duration::from_secs(120),
        }
    }
}

pub struct Request {
    pub method: String,
    /// Path as sent, still percent-encoded, without the query.
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn param(&self, key: &str) -> Option<String> {
        self.query
            .split('&')
            .filter_map(|kv| kv.split_once('=').or(Some((kv, ""))))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| decode(v, true))
    }
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn with(mut self, k: &str, v: impl Into<String>) -> Response {
        self.headers.push((k.into(), v.into()));
        self
    }

    pub fn body(mut self, kind: &str, body: impl Into<Vec<u8>>) -> Response {
        self.body = body.into();
        self.with("Content-Type", kind)
    }

    pub fn html(body: String) -> Response {
        Response::new(200).body("text/html; charset=utf-8", body)
    }

    pub fn text(status: u16, msg: impl Into<String>) -> Response {
        Response::new(status).body("text/plain; charset=utf-8", msg.into())
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        303 => "See Other",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        423 => "Locked",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
        _ => "Status",
    }
}

pub type Handler = dyn Fn(Request) -> Response + Send + Sync;

pub fn serve(port: u16, handler: Arc<Handler>, limits: Limits) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        if limits
            .max_conns
            .is_some_and(|max| open.load(Ordering::SeqCst) >= max)
        {
            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
            let _ = stream.write_all(
                b"HTTP/1.1 503 Service Unavailable\r\nRetry-After: 5\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            // Read what the client already sent, briefly, so closing does not
            // reset the connection before it reads the 503.
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
            let _ = stream.read(&mut [0u8; 4096]);
            continue;
        }
        let handler = handler.clone();
        let open = open.clone();
        open.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let result = connection(stream, &*handler, limits);
            open.fetch_sub(1, Ordering::SeqCst);
            if let Err(e) = result
                && !matches!(
                    e.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                )
            {
                eprintln!("{} connection: {e}", crate::live::stamp());
            }
        });
    }
    Ok(())
}

/// One line, at most `MAX_LINE` bytes; none at the end of input.
fn line(input: &mut BufReader<TcpStream>) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = input
        .by_ref()
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > MAX_LINE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            TOO_LONG,
        ));
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

const TOO_LONG: &str = "header line too long";
const TOO_LARGE: &str = "body too large";

fn refuse(out: &mut TcpStream, status: u16, msg: &str) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{msg}",
        reason(status),
        msg.len()
    );
    out.write_all(head.as_bytes())?;
    out.flush()?;
    // Take a little of what is still coming, so closing does not reset the
    // connection before the client reads the refusal.
    let _ = out.shutdown(std::net::Shutdown::Write);
    let _ = out.set_read_timeout(Some(Duration::from_millis(100)));
    let mut sink = [0u8; 8192];
    for _ in 0..8 {
        if !matches!(out.read(&mut sink), Ok(n) if n > 0) {
            break;
        }
    }
    Ok(())
}

fn is(e: &std::io::Error, what: &str) -> bool {
    e.kind() == std::io::ErrorKind::InvalidData && e.to_string() == what
}

fn connection(stream: TcpStream, handler: &Handler, limits: Limits) -> std::io::Result<()> {
    stream.set_read_timeout(Some(limits.idle))?;
    let _ = stream.set_nodelay(true);
    let mut out = stream.try_clone()?;
    let mut input = BufReader::new(stream);
    loop {
        let line = match line(&mut input) {
            Ok(Some(l)) => l,
            Ok(None) => return Ok(()),
            Err(e) if is(&e, TOO_LONG) => return refuse(&mut out, 431, "request line too long"),
            Err(e) => return Err(e),
        };
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let target = parts.next().unwrap_or("/").to_string();
        let version = parts.next().unwrap_or("HTTP/1.0").to_string();
        let mut headers = Vec::new();
        loop {
            let h = match self::line(&mut input) {
                Ok(Some(h)) => h,
                Ok(None) => return Ok(()),
                Err(e) if is(&e, TOO_LONG) => {
                    return refuse(&mut out, 431, "header line too long");
                }
                Err(e) => return Err(e),
            };
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if headers.len() >= MAX_HEADERS {
                return refuse(&mut out, 431, "too many headers");
            }
            if let Some((k, v)) = h.split_once(':') {
                headers.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
        let get = |name: &str| {
            headers
                .iter()
                .find(|(k, _): &&(String, String)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        let is_chunked =
            get("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked"));
        let length: Option<usize> = if is_chunked {
            None
        } else {
            match get("content-length") {
                None => Some(0),
                Some(n) => match n.parse() {
                    Ok(n) => Some(n),
                    Err(_) => return refuse(&mut out, 400, "bad Content-Length"),
                },
            }
        };
        let too_large = |n: usize| limits.max_body.is_some_and(|max| n > max);
        if length.is_some_and(too_large) {
            // Refused before reading it (or sending 100 Continue).
            return refuse(&mut out, 413, &too_large_msg(limits));
        }
        if get("expect").is_some_and(|e| e.eq_ignore_ascii_case("100-continue")) {
            out.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        }
        let body = match length {
            None => match chunked(&mut input, limits.max_body) {
                Ok(b) => b,
                Err(e) if is(&e, TOO_LARGE) => {
                    return refuse(&mut out, 413, &too_large_msg(limits));
                }
                Err(e) => return Err(e),
            },
            Some(n) => {
                let mut body = vec![0; n];
                input.read_exact(&mut body)?;
                body
            }
        };
        let close = get("connection").is_some_and(|c| c.eq_ignore_ascii_case("close"))
            || (version == "HTTP/1.0"
                && !get("connection").is_some_and(|c| c.eq_ignore_ascii_case("keep-alive")));
        let (path, query) = match target.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (target.clone(), String::new()),
        };
        let head_only = method == "HEAD";
        let req = Request {
            method,
            path,
            query,
            headers,
            body,
        };
        let resp = handler(req);
        let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status));
        for (k, v) in &resp.headers {
            head += &format!("{k}: {v}\r\n");
        }
        if !resp
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        {
            head += &format!("Content-Length: {}\r\n", resp.body.len());
        }
        head += if close {
            "Connection: close\r\n\r\n"
        } else {
            "\r\n"
        };
        out.write_all(head.as_bytes())?;
        if !head_only && resp.status != 204 && resp.status != 304 {
            out.write_all(&resp.body)?;
        }
        out.flush()?;
        if close {
            return Ok(());
        }
    }
}

fn too_large_msg(limits: Limits) -> String {
    format!(
        "uploads are limited to {} bytes here",
        limits.max_body.unwrap_or(0)
    )
}

fn chunked(input: &mut BufReader<TcpStream>, max: Option<usize>) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let size = line(input)?.unwrap_or_default();
        let size = size.trim().split(';').next().unwrap_or("0");
        let n = usize::from_str_radix(size, 16)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size"))?;
        if n == 0 {
            // Trailers, then the blank line.
            for _ in 0..MAX_HEADERS {
                match line(input)? {
                    None => return Ok(body),
                    Some(t) if t.trim().is_empty() => return Ok(body),
                    Some(_) => {}
                }
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                TOO_LONG,
            ));
        }
        if max.is_some_and(|max| body.len().saturating_add(n) > max) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                TOO_LARGE,
            ));
        }
        let start = body.len();
        body.resize(start + n, 0);
        input.read_exact(&mut body[start..])?;
        let mut crlf = [0u8; 2];
        input.read_exact(&mut crlf)?;
    }
}

/// Percent-decode; with `plus`, `+` reads as a space (form encoding).
pub fn decode(s: &str, plus: bool) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        match b[i] {
            b'+' if plus => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into()
}

/// Percent-encode a path (keeping `/`) or a query value.
pub fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_and_plus() {
        assert_eq!(decode("a%20b+c", true), "a b c");
        assert_eq!(decode("a%20b+c", false), "a b+c");
        assert_eq!(decode("100%", false), "100%");
        assert_eq!(decode("%C3%A9", false), "é");
        assert_eq!(decode(&enc("dir/a b é.txt"), false), "dir/a b é.txt");
    }

    fn start(limits: Limits) -> u16 {
        let port = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let handler: Arc<Handler> = Arc::new(|req: Request| {
            if req.path == "/slow" {
                std::thread::sleep(Duration::from_millis(600));
            }
            Response::text(200, format!("got {}", req.body.len()))
        });
        std::thread::spawn(move || serve(port, handler, limits));
        for _ in 0..100 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return port;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("server did not start");
    }

    fn send(port: u16, raw: &[u8]) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let _ = s.write_all(raw);
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn limits_refuse_large_bodies_long_headers_and_extra_connections() {
        let port = start(Limits {
            max_body: Some(100),
            max_conns: Some(2),
            idle: Duration::from_secs(2),
        });
        let ok = send(
            port,
            b"PUT /a HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
        );
        assert!(
            ok.starts_with("HTTP/1.1 200") && ok.ends_with("got 5"),
            "{ok}"
        );
        // Refused from the header alone: no body sent, no 100 Continue.
        let r = send(port, b"PUT /a HTTP/1.1\r\nHost: x\r\nContent-Length: 999999999\r\nExpect: 100-continue\r\n\r\n");
        assert!(r.starts_with("HTTP/1.1 413"), "{r}");
        assert!(!r.contains("100 Continue"), "{r}");
        let mut chunks =
            b"PUT /a HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for _ in 0..5 {
            chunks.extend_from_slice(b"20\r\n");
            chunks.extend_from_slice(&[b'z'; 32]);
            chunks.extend_from_slice(b"\r\n");
        }
        chunks.extend_from_slice(b"0\r\n\r\n");
        let r = send(port, &chunks);
        assert!(r.starts_with("HTTP/1.1 413"), "{r}");
        let mut long = b"GET / HTTP/1.1\r\nX-Long: ".to_vec();
        long.extend(std::iter::repeat_n(b'a', MAX_LINE + 10));
        long.extend_from_slice(b"\r\n\r\n");
        let r = send(port, &long);
        assert!(r.starts_with("HTTP/1.1 431"), "{r}");
        let mut many = b"GET / HTTP/1.1\r\n".to_vec();
        for i in 0..=MAX_HEADERS {
            many.extend_from_slice(format!("X-{i}: v\r\n").as_bytes());
        }
        many.extend_from_slice(b"\r\n");
        let r = send(port, &many);
        assert!(r.starts_with("HTTP/1.1 431"), "{r}");
        // Two idle connections fill the cap; a third is turned away.
        let open = || {
            // One answered request each, so both are being served.
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 512];
            while !got.ends_with(b"got 0") {
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0, "closed early");
                got.extend_from_slice(&buf[..n]);
            }
            s
        };
        let a = open();
        let b = open();
        let r = send(
            port,
            b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(r.starts_with("HTTP/1.1 503"), "{r}");
        drop((a, b));
        std::thread::sleep(Duration::from_millis(200));
        let r = send(
            port,
            b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(r.starts_with("HTTP/1.1 200"), "{r}");
    }

    #[test]
    fn no_limits_by_default() {
        let port = start(Limits::default());
        let mut raw =
            b"PUT /a HTTP/1.1\r\nHost: x\r\nContent-Length: 300000\r\nConnection: close\r\n\r\n"
                .to_vec();
        raw.extend(std::iter::repeat_n(b'q', 300_000));
        let r = send(port, &raw);
        assert!(r.ends_with("got 300000"), "{}", &r[..r.len().min(200)]);
    }
}
