//! WebDAV (RFC 4918, class 1 and 2) over the live tree, under `/dav/`, so
//! the store mounts as a disk with nothing to install: on a Mac, Finder's
//! Go > Connect to Server, `http://127.0.0.1:8737/dav/`; on Linux, davfs2
//! or a file manager. Every write is an edit (see `live.rs`). Locks are
//! granted and remembered but not enforced (one user). Desktop metadata
//! files (`._*`, `.DS_Store`) are kept in memory for the client only.

use std::time::SystemTime;

use ikam_kernel::{Id, Kind};

use crate::http::{Request, Response, decode, enc, esc};
use crate::live::{Live, Op, http_date, is_junk, iso_date, norm};

pub const PREFIX: &str = "/dav";

/// The live path a request names (`""` is the root), or `None` outside
/// `/dav`.
pub fn dav_path(raw: &str) -> Option<String> {
    let rest = raw.strip_prefix(PREFIX)?;
    if !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    let p = decode(rest, false);
    if p.split('/').any(|s| s == "..") {
        return None;
    }
    Some(norm(&p))
}

fn href(path: &str, dir: bool) -> String {
    let mut h = format!("{PREFIX}/{}", enc(path));
    if dir && !h.ends_with('/') {
        h.push('/');
    }
    h.replace("//", "/")
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or("")
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

fn junk_path(path: &str) -> bool {
    is_junk(name_of(path))
}

/// What a client sees once `--max-store` is reached.
pub const FULL: &str =
    "this ikam store is full, so new content is refused; reading and deleting still work";

pub fn handle(live: &mut Live, req: &Request) -> Response {
    let Some(path) = dav_path(&req.path) else {
        return Response::text(400, "bad path");
    };
    if matches!(
        req.method.as_str(),
        "PUT" | "MKCOL" | "COPY" | "MOVE" | "LOCK" | "PROPPATCH"
    ) && !live.has_room(req.body.len())
    {
        return Response::text(507, FULL);
    }
    let r = match req.method.as_str() {
        "OPTIONS" => Ok(Response::new(200)
            .with("DAV", "1, 2")
            .with("MS-Author-Via", "DAV")
            .with(
                "Allow",
                "OPTIONS, GET, HEAD, PUT, DELETE, MKCOL, COPY, MOVE, PROPFIND, PROPPATCH, LOCK, UNLOCK",
            )),
        "PROPFIND" => propfind(live, req, &path),
        "PROPPATCH" => Ok(proppatch(&path, live.stat(&path).ok().flatten().is_some()
            || live.junk.contains_key(&path))),
        "GET" | "HEAD" => get(live, req, &path),
        "PUT" => put(live, req, &path),
        "DELETE" => delete(live, &path),
        "MKCOL" => mkcol(live, req, &path),
        "MOVE" | "COPY" => move_copy(live, req, &path),
        "LOCK" => lock(live, req, &path),
        "UNLOCK" => {
            live.locks.remove(&path);
            Ok(Response::new(204))
        }
        _ => Ok(Response::new(405)),
    };
    r.unwrap_or_else(|e| Response::text(500, e))
}

enum Node {
    Dir,
    File(Id, u64),
    Junk(u64, SystemTime),
}

fn node(live: &mut Live, path: &str) -> Result<Option<Node>, String> {
    if let Some((b, t)) = live.junk.get(path) {
        return Ok(Some(Node::Junk(b.len() as u64, *t)));
    }
    Ok(match live.stat(path)? {
        Some((Kind::Tree, _)) => Some(Node::Dir),
        Some((Kind::File, id)) => Some(Node::File(id, live.size(&id)?)),
        _ => None,
    })
}

fn is_dir(live: &mut Live, path: &str) -> Result<bool, String> {
    Ok(matches!(live.stat(path)?, Some((Kind::Tree, _))))
}

fn propfind(live: &mut Live, req: &Request, path: &str) -> Result<Response, String> {
    let Some(n) = node(live, path)? else {
        return Ok(Response::new(404));
    };
    let depth = req.header("depth").unwrap_or("infinity");
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n",
    );
    xml += &response_xml(live, path, &n);
    if matches!(n, Node::Dir) && depth != "0" {
        let Some((_, tree)) = live.stat(path)? else {
            unreachable!()
        };
        let entries = live.entries(&tree)?;
        for e in entries.iter() {
            let child = if path.is_empty() {
                e.name.clone()
            } else {
                format!("{path}/{}", e.name)
            };
            let c = match e.kind {
                Kind::Tree => Node::Dir,
                Kind::File => Node::File(e.id, live.size(&e.id)?),
                Kind::Graph => continue,
            };
            xml += &response_xml(live, &child, &c);
        }
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        let junk: Vec<(String, u64, SystemTime)> = live
            .junk
            .iter()
            .filter(|(p, _)| {
                p.strip_prefix(&prefix)
                    .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
            })
            .map(|(p, (b, t))| (p.clone(), b.len() as u64, *t))
            .collect();
        for (p, len, t) in junk {
            xml += &response_xml(live, &p, &Node::Junk(len, t));
        }
    }
    xml += "</D:multistatus>\n";
    Ok(Response::new(207).body("application/xml; charset=utf-8", xml))
}

fn response_xml(live: &Live, path: &str, n: &Node) -> String {
    let (dir, len, etag, mtime) = match n {
        Node::Dir => (true, 0, None, live.mtime(path)),
        Node::File(id, len) => (false, *len, Some(id.to_hex()), live.mtime(path)),
        Node::Junk(len, t) => (false, *len, Some(format!("j{len}")), *t),
    };
    let name = if path.is_empty() {
        "ikam"
    } else {
        name_of(path)
    };
    let mut p = format!(
        "<D:displayname>{}</D:displayname><D:getlastmodified>{}</D:getlastmodified><D:creationdate>{}</D:creationdate>",
        esc(name),
        http_date(mtime),
        iso_date(live.started.min(mtime))
    );
    if dir {
        p += "<D:resourcetype><D:collection/></D:resourcetype>";
        if path.is_empty() {
            p += "<D:quota-available-bytes>1099511627776</D:quota-available-bytes><D:quota-used-bytes>0</D:quota-used-bytes>";
        }
    } else {
        p += &format!(
            "<D:resourcetype/><D:getcontentlength>{len}</D:getcontentlength><D:getcontenttype>{}</D:getcontenttype><D:getetag>\"{}\"</D:getetag>",
            mime(path),
            etag.unwrap_or_default()
        );
    }
    p += "<D:supportedlock><D:lockentry><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockentry></D:supportedlock>";
    p += &match live.locks.get(path) {
        Some(token) => format!(
            "<D:lockdiscovery>{}</D:lockdiscovery>",
            active_lock(path, token)
        ),
        None => "<D:lockdiscovery/>".into(),
    };
    format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop>{p}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>\n",
        href(path, dir)
    )
}

fn proppatch(path: &str, exists: bool) -> Response {
    if !exists {
        return Response::new(404);
    }
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\"><D:response><D:href>{}</D:href><D:propstat><D:prop/><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>\n",
        href(path, false)
    );
    Response::new(207).body("application/xml; charset=utf-8", xml)
}

/// The self-test rewrites its results in place on every run, so a reader
/// revalidates them each time rather than showing a cached copy; the ETag
/// and Last-Modified keep that a cheap 304 when nothing changed.
fn fresh(r: Response, path: &str) -> Response {
    if path == "results" || path.starts_with("results/") {
        r.with("Cache-Control", "no-cache")
    } else {
        r
    }
}

fn get(live: &mut Live, req: &Request, path: &str) -> Result<Response, String> {
    get_cached(live, req, path).map(|r| fresh(r, path))
}

fn get_cached(live: &mut Live, req: &Request, path: &str) -> Result<Response, String> {
    let (bytes, etag, mtime) = match node(live, path)? {
        None => return Ok(Response::new(404)),
        Some(Node::Dir) => {
            let Some((_, tree)) = live.stat(path)? else {
                unreachable!()
            };
            let mut html = format!("<!doctype html><title>/{}</title><ul>", esc(path));
            for e in live.entries(&tree)?.iter() {
                let child = format!("{path}/{}", e.name);
                html += &format!(
                    "<li><a href=\"{}\">{}</a></li>",
                    href(&norm(&child), e.kind == Kind::Tree),
                    esc(&e.name)
                );
            }
            return Ok(Response::html(html + "</ul>"));
        }
        Some(Node::Junk(..)) => {
            let (b, t) = live.junk[path].clone();
            (std::sync::Arc::new(b), "junk".to_string(), t)
        }
        Some(Node::File(id, _)) => (live.read(&id)?, id.to_hex(), live.mtime(path)),
    };
    let etag = format!("\"{etag}\"");
    if req.header("if-none-match") == Some(etag.as_str()) {
        return Ok(Response::new(304).with("ETag", etag));
    }
    // Files are shown, never run, at this origin.
    let base = Response::new(200)
        .with("Content-Security-Policy", "sandbox")
        .with("X-Content-Type-Options", "nosniff")
        .with("ETag", etag)
        .with("Last-Modified", http_date(mtime))
        .with("Accept-Ranges", "bytes");
    if let Some(range) = req.header("range").and_then(|r| r.strip_prefix("bytes="))
        && !range.contains(',')
    {
        let len = bytes.len() as u64;
        let (a, b) = range.split_once('-').unwrap_or((range, ""));
        let (start, end) = match (a.trim().parse::<u64>(), b.trim().parse::<u64>()) {
            (Ok(s), Ok(e)) => (s, e.min(len.saturating_sub(1))),
            (Ok(s), Err(_)) => (s, len.saturating_sub(1)),
            (Err(_), Ok(n)) => (len.saturating_sub(n), len.saturating_sub(1)),
            _ => (0, len.saturating_sub(1)),
        };
        if start >= len || start > end {
            return Ok(Response::new(416).with("Content-Range", format!("bytes */{len}")));
        }
        let mut r = base
            .with("Content-Range", format!("bytes {start}-{end}/{len}"))
            .body(mime(path), bytes[start as usize..=end as usize].to_vec());
        r.status = 206;
        return Ok(r);
    }
    Ok(base.body(mime(path), bytes.to_vec()))
}

fn put(live: &mut Live, req: &Request, path: &str) -> Result<Response, String> {
    if path.is_empty() {
        return Ok(Response::new(405));
    }
    if !is_dir(live, parent_of(path))? {
        return Ok(Response::new(409));
    }
    if junk_path(path) {
        let fresh = live
            .junk
            .insert(path.into(), (req.body.clone(), SystemTime::now()))
            .is_none();
        return Ok(Response::new(if fresh { 201 } else { 204 }));
    }
    let existed = match live.stat(path)? {
        Some((Kind::Tree, _)) => return Ok(Response::new(405)),
        Some(_) => true,
        None => false,
    };
    let cost = live.apply(vec![Op::Put(path.into(), req.body.clone())])?;
    let etag = live
        .stat(path)?
        .map(|(_, id)| id.to_hex())
        .unwrap_or_default();
    // What the edit cost, for clients that measure it (the live bench).
    Ok(Response::new(if existed { 204 } else { 201 })
        .with("ETag", format!("\"{etag}\""))
        .with("X-Ikam-Cost", cost_line(&cost)))
}

fn delete(live: &mut Live, path: &str) -> Result<Response, String> {
    if live.junk.remove(path).is_some() {
        return Ok(Response::new(204));
    }
    if path.is_empty() {
        return Ok(Response::new(403));
    }
    if live.stat(path)?.is_none() {
        return Ok(Response::new(404));
    }
    let prefix = format!("{path}/");
    live.junk.retain(|p, _| !p.starts_with(&prefix));
    live.locks.remove(path);
    let cost = live.apply(vec![Op::Remove(path.into())])?;
    Ok(Response::new(204).with("X-Ikam-Cost", cost_line(&cost)))
}

fn mkcol(live: &mut Live, req: &Request, path: &str) -> Result<Response, String> {
    if !req.body.is_empty() {
        return Ok(Response::new(415));
    }
    if path.is_empty() || live.stat(path)?.is_some() || live.junk.contains_key(path) {
        return Ok(Response::new(405));
    }
    if !is_dir(live, parent_of(path))? {
        return Ok(Response::new(409));
    }
    live.apply(vec![Op::Mkdir(path.into())])?;
    Ok(Response::new(201))
}

/// The live path a `Destination` header names (absolute URL or path).
fn destination(req: &Request) -> Option<String> {
    let d = req.header("destination")?;
    let path = match d.find("://") {
        Some(i) => &d[i + 3..][d[i + 3..].find('/').unwrap_or(d.len() - i - 3)..],
        None => d,
    };
    dav_path(path.split('?').next().unwrap_or(path))
}

fn move_copy(live: &mut Live, req: &Request, path: &str) -> Result<Response, String> {
    let Some(dest) = destination(req) else {
        return Ok(Response::new(400));
    };
    let is_move = req.method == "MOVE";
    if dest == path {
        return Ok(Response::new(403));
    }
    if path.is_empty() || dest.is_empty() || dest.starts_with(&format!("{path}/")) {
        return Ok(Response::new(409));
    }
    let overwrite = !req
        .header("overwrite")
        .is_some_and(|o| o.eq_ignore_ascii_case("F"));
    if !is_dir(live, parent_of(&dest))? {
        return Ok(Response::new(409));
    }
    let dest_exists = live.stat(&dest)?.is_some() || live.junk.contains_key(&dest);
    if dest_exists && !overwrite {
        return Ok(Response::new(412));
    }
    let status = if dest_exists { 204 } else { 201 };
    // Desktop metadata moves in memory; a real file named like it would
    // not be stored, so it moves too.
    if let Some(j) = live.junk.get(path).cloned() {
        if is_move {
            live.junk.remove(path);
        }
        if junk_path(&dest) {
            live.junk.insert(dest, j);
        } else {
            live.apply(vec![Op::Put(dest, j.0)])?;
        }
        return Ok(Response::new(status));
    }
    let Some((kind, _)) = live.stat(path)? else {
        return Ok(Response::new(404));
    };
    let mut ops = Vec::new();
    if dest_exists {
        live.junk.remove(&dest);
        ops.push(Op::Remove(dest.clone()));
    }
    let shallow = req.header("depth") == Some("0") && kind == Kind::Tree && !is_move;
    ops.push(if shallow {
        Op::Mkdir(dest.clone())
    } else if is_move {
        Op::Rename(path.into(), dest.clone())
    } else {
        Op::Copy(path.into(), dest.clone())
    });
    if is_move && let Some(t) = live.locks.remove(path) {
        live.locks.insert(dest.clone(), t);
    }
    let cost = live.apply(ops)?;
    Ok(Response::new(status).with("X-Ikam-Cost", cost_line(&cost)))
}

fn cost_line(c: &crate::live::Cost) -> String {
    format!(
        "new_bytes={} store_ms={:.2} tree_ms={:.2} index_ms={:.2}",
        c.new_bytes, c.store_ms, c.tree_ms, c.index_ms
    )
}

fn active_lock(path: &str, token: &str) -> String {
    format!(
        "<D:activelock><D:locktype><D:write/></D:locktype><D:lockscope><D:exclusive/></D:lockscope><D:depth>infinity</D:depth><D:timeout>Second-3600</D:timeout><D:locktoken><D:href>{token}</D:href></D:locktoken><D:lockroot><D:href>{}</D:href></D:lockroot></D:activelock>",
        href(path, false)
    )
}

fn lock(live: &mut Live, req: &Request, path: &str) -> Result<Response, String> {
    let mut status = 200;
    if node(live, path)?.is_none() {
        // A lock on an unmapped URL creates an empty resource (RFC 4918 9.10.4).
        if path.is_empty() || !is_dir(live, parent_of(path))? {
            return Ok(Response::new(409));
        }
        if junk_path(path) {
            live.junk
                .insert(path.into(), (Vec::new(), SystemTime::now()));
        } else {
            live.apply(vec![Op::Put(path.into(), Vec::new())])?;
        }
        status = 201;
    }
    let token = match live.locks.get(path) {
        Some(t) if req.body.is_empty() => t.clone(), // a refresh
        _ => {
            let seed = format!("{path}{:?}", SystemTime::now());
            let h = Id::of(seed.as_bytes()).to_hex();
            format!(
                "opaquelocktoken:{}-{}-{}-{}-{}",
                &h[..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..32]
            )
        }
    };
    live.locks.insert(path.into(), token.clone());
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:prop xmlns:D=\"DAV:\"><D:lockdiscovery>{}</D:lockdiscovery></D:prop>\n",
        active_lock(path, &token)
    );
    let mut r = Response::new(200)
        .with("Lock-Token", format!("<{token}>"))
        .body("application/xml; charset=utf-8", xml);
    r.status = status;
    Ok(r)
}

pub fn mime(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "md" | "txt" | "csv" | "tsv" | "rs" | "py" | "toml" | "html" | "htm" | "js" | "css"
        | "xml" | "yaml" | "yml" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "zip" | "whl" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(dav_path("/dav"), Some(String::new()));
        assert_eq!(dav_path("/dav/"), Some(String::new()));
        assert_eq!(dav_path("/dav/a%20b/c.txt"), Some("a b/c.txt".into()));
        assert_eq!(dav_path("/dav/../x"), None);
        assert_eq!(dav_path("/davx"), None);
        assert_eq!(href("a b/c", true), "/dav/a%20b/c/");
        assert_eq!(href("", true), "/dav/");
    }

    fn req(method: &str, path: &str, body: &[u8]) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: String::new(),
            headers: vec![("Host".into(), "127.0.0.1:1".into())],
            body: body.to_vec(),
        }
    }

    #[test]
    fn a_full_store_refuses_new_content_but_reads_and_deletes() {
        let dir = std::env::temp_dir().join(format!("ikam-dav-full-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut l = Live::open(dir.to_str().unwrap(), "main").unwrap();
        let start = l.used_bytes();
        l.max_store = Some(start + 4096);
        assert_eq!(
            handle(&mut l, &req("PUT", "/dav/a.txt", b"small")).status,
            201
        );
        let big = vec![b'x'; 8192];
        let r = handle(&mut l, &req("PUT", "/dav/big.bin", &big));
        assert_eq!(r.status, 507);
        assert_eq!(handle(&mut l, &req("GET", "/dav/a.txt", b"")).status, 200);
        // Fill it with distinct small files until it refuses.
        let mut refused = false;
        for i in 0..200 {
            let body = format!("{i:04} {}", "y".repeat(200));
            if handle(
                &mut l,
                &req("PUT", &format!("/dav/f{i}.txt"), body.as_bytes()),
            )
            .status
                == 507
            {
                refused = true;
                break;
            }
        }
        assert!(refused, "the store never filled");
        assert!(
            l.used_bytes() <= start + 4096 + 512,
            "went well past the cap"
        );
        // At the cap, even an empty request that adds content is refused.
        l.max_store = Some(l.used_bytes());
        assert_eq!(handle(&mut l, &req("MKCOL", "/dav/d", b"")).status, 507);
        // Desktop metadata counts too.
        assert_eq!(
            handle(&mut l, &req("PUT", "/dav/._a.txt", &[0; 64])).status,
            507
        );
        assert_eq!(
            handle(&mut l, &req("DELETE", "/dav/a.txt", b"")).status,
            204
        );
        assert_eq!(handle(&mut l, &req("PROPFIND", "/dav/", b"")).status, 207);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn results_are_revalidated_and_other_files_are_not() {
        let dir = std::env::temp_dir().join(format!("ikam-dav-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut l = Live::open(dir.to_str().unwrap(), "main").unwrap();
        assert_eq!(
            handle(&mut l, &req("MKCOL", "/dav/results", b"")).status,
            201
        );
        for p in [
            "/dav/results/latest.json",
            "/dav/a.txt",
            "/dav/resultsx.txt",
        ] {
            assert_eq!(handle(&mut l, &req("PUT", p, b"{}")).status, 201, "{p}");
        }
        let cache = |r: &Response| {
            r.headers
                .iter()
                .find(|(k, _)| k == "Cache-Control")
                .map(|(_, v)| v.clone())
        };
        let r = handle(&mut l, &req("GET", "/dav/results/latest.json", b""));
        assert_eq!((r.status, cache(&r).as_deref()), (200, Some("no-cache")));
        let etag = r
            .headers
            .iter()
            .find(|(k, _)| k == "ETag")
            .unwrap()
            .1
            .clone();
        assert!(r.headers.iter().any(|(k, _)| k == "Last-Modified"));
        // Revalidating an unchanged copy is a 304 that says the same.
        let mut again = req("GET", "/dav/results/latest.json", b"");
        again.headers.push(("If-None-Match".into(), etag));
        let r = handle(&mut l, &again);
        assert_eq!((r.status, cache(&r).as_deref()), (304, Some("no-cache")));
        let r = handle(&mut l, &req("HEAD", "/dav/results/latest.json", b""));
        assert_eq!(cache(&r).as_deref(), Some("no-cache"));
        let r = handle(&mut l, &req("GET", "/dav/results/", b""));
        assert_eq!(cache(&r).as_deref(), Some("no-cache"));
        for p in ["/dav/a.txt", "/dav/resultsx.txt"] {
            let r = handle(&mut l, &req("GET", p, b""));
            assert_eq!((r.status, cache(&r)), (200, None), "{p}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
