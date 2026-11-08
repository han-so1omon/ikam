//! A small HTTP/1.1 server for `ikam live`: keep-alive, Content-Length and
//! chunked bodies, `Expect: 100-continue`, one thread per connection. Only
//! 127.0.0.1 is bound.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

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
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        423 => "Locked",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        507 => "Insufficient Storage",
        _ => "Status",
    }
}

pub type Handler = dyn Fn(Request) -> Response + Send + Sync;

pub fn serve(port: u16, handler: Arc<Handler>) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let handler = handler.clone();
        std::thread::spawn(move || {
            if let Err(e) = connection(stream, &*handler)
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

fn connection(stream: TcpStream, handler: &Handler) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    let _ = stream.set_nodelay(true);
    let mut out = stream.try_clone()?;
    let mut input = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(());
        }
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
            let mut h = String::new();
            if input.read_line(&mut h)? == 0 {
                return Ok(());
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
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
        if get("expect").is_some_and(|e| e.eq_ignore_ascii_case("100-continue")) {
            out.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        }
        let body = if get("transfer-encoding")
            .is_some_and(|t| t.to_ascii_lowercase().contains("chunked"))
        {
            chunked(&mut input)?
        } else {
            let n: usize = get("content-length")
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            let mut body = vec![0; n];
            input.read_exact(&mut body)?;
            body
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

fn chunked(input: &mut BufReader<TcpStream>) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size = String::new();
        input.read_line(&mut size)?;
        let size = size.trim().split(';').next().unwrap_or("0");
        let n = usize::from_str_radix(size, 16)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size"))?;
        if n == 0 {
            // Trailers, then the blank line.
            loop {
                let mut t = String::new();
                if input.read_line(&mut t)? == 0 || t.trim().is_empty() {
                    return Ok(body);
                }
            }
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
}
