//! `ikam serve`: a local, read-only web view of a store. Browse commits and
//! trees, read or download any file, search a revision, see how a file is
//! stored (bytes, or a verified derivation) and what the store holds.
//! Plain std HTTP on 127.0.0.1, one request at a time.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

use ikam_kernel::{Id, Kind, Object, Repo, Store};

use crate::corpus::{self, err};

pub fn serve<S: Store>(repo: &Repo<S>, port: u16) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(err)?;
    eprintln!("serving on http://127.0.0.1:{port}/");
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if let Err(e) = handle(repo, stream) {
            eprintln!("request failed: {e}");
        }
    }
    Ok(())
}

fn handle<S: Store>(repo: &Repo<S>, mut stream: TcpStream) -> Result<(), String> {
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(err)?;
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let param = |k: &str| {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(key, _)| *key == k)
            .map(|(_, v)| decode(v))
    };
    let (status, kind, body) = match route(repo, path, &param) {
        Ok(Page::Html(title, html)) => (
            "200 OK",
            "text/html; charset=utf-8".into(),
            page(&title, &html),
        ),
        Ok(Page::Raw(mime, bytes)) => ("200 OK", mime, bytes),
        Err(e) => (
            "404 Not Found",
            "text/html; charset=utf-8".into(),
            page("Not found", &format!("<p>{}</p>", esc(&e))),
        ),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).map_err(err)?;
    stream.write_all(&body).map_err(err)
}

enum Page {
    Html(String, String),
    Raw(String, Vec<u8>),
}

fn route<S: Store>(
    repo: &Repo<S>,
    path: &str,
    param: &dyn Fn(&str) -> Option<String>,
) -> Result<Page, String> {
    let rev = param("rev").unwrap_or_else(|| "main".into());
    let id = |s: &str| s.parse::<Id>().map_err(err);
    match path
        .trim_start_matches('/')
        .split_once('/')
        .unwrap_or((path.trim_start_matches('/'), ""))
    {
        ("", _) => home(repo),
        ("tree", t) => tree_page(repo, id(t)?, &param("path").unwrap_or_default()),
        ("file", f) => file_page(repo, id(f)?, &param("path").unwrap_or_default()),
        ("raw", f) => {
            let name = param("path").unwrap_or_default();
            Ok(Page::Raw(
                mime(&name).into(),
                repo.read_content(&id(f)?).map_err(err)?,
            ))
        }
        ("search", _) => search_page(repo, &rev, &param("q").unwrap_or_default()),
        ("stats", _) => stats_page(repo, &rev),
        _ => Err(format!("no page {path}")),
    }
}

fn home<S: Store>(repo: &Repo<S>) -> Result<Page, String> {
    let mut html = search_form("main", "");
    for (name, id) in repo.store().refs().map_err(err)? {
        if !matches!(repo.get(&id), Ok(Object::Commit(_))) {
            continue;
        }
        html += &format!("<h2>{}</h2><table>", esc(&name));
        for (c_id, c) in repo.log(id).map_err(err)?.iter().take(50) {
            html += &format!(
                "<tr><td><a href=\"/tree/{}\"><code>{}</code></a></td><td>{}</td></tr>",
                c.tree,
                &c_id.to_hex()[..12],
                esc(&c.message)
            );
        }
        html += "</table>";
    }
    html += "<p><a href=\"/stats\">What the store holds</a></p>";
    Ok(Page::Html("ikam".into(), html))
}

fn tree_page<S: Store>(repo: &Repo<S>, id: Id, path: &str) -> Result<Page, String> {
    let Object::Tree(entries) = repo.get(&id).map_err(err)? else {
        return Err("not a tree".into());
    };
    let mut html = format!("<p><a href=\"/\">home</a> / {}</p><table>", esc(path));
    for e in entries {
        let sub = if path.is_empty() {
            e.name.clone()
        } else {
            format!("{path}/{}", e.name)
        };
        let (kind, size) = match e.kind {
            Kind::Tree => ("tree", String::new()),
            Kind::File => (
                "file",
                repo.read_content(&e.id)
                    .map(|b| b.len().to_string())
                    .unwrap_or_default(),
            ),
            Kind::Graph => continue,
        };
        html += &format!(
            "<tr><td><a href=\"/{kind}/{}?path={}\">{}{}</a></td><td class=n>{size}</td></tr>",
            e.id,
            enc(&sub),
            esc(&e.name),
            if kind == "tree" { "/" } else { "" }
        );
    }
    let title = if path.is_empty() {
        "ikam: /".to_string()
    } else {
        format!("ikam: {path}")
    };
    Ok(Page::Html(title, html + "</table>"))
}

const PREVIEW: usize = 256 * 1024;

fn file_page<S: Store>(repo: &Repo<S>, id: Id, path: &str) -> Result<Page, String> {
    let bytes = repo.read_content(&id).map_err(err)?;
    let mut html = format!(
        "<p><a href=\"/\">home</a> / {} &middot; {} bytes &middot; <a href=\"/raw/{id}?path={}\">download</a></p>",
        esc(path),
        bytes.len(),
        enc(path)
    );
    html += &storage(repo, &id)?;
    match std::str::from_utf8(&bytes[..bytes.len().min(PREVIEW)]) {
        Ok(text) => html += &format!("<pre>{}</pre>", esc(text)),
        Err(_) => match corpus::readable_text(repo, &id, PREVIEW).map_err(err)? {
            Some(text) => {
                html += &format!(
                    "<p class=s>Binary file; the text in its parts:</p><pre>{}</pre>",
                    esc(&text)
                )
            }
            None => html += "<p>Binary file.</p>",
        },
    }
    Ok(Page::Html(format!("ikam: {path}"), html))
}

/// How the kernel holds a file: as stored bytes, or reconstructed by a
/// verified derivation from other contents.
pub fn storage<S: Store>(repo: &Repo<S>, id: &Id) -> Result<String, String> {
    let derivations = repo.derivations(id).map_err(err)?;
    if derivations.is_empty() {
        let stored = repo.store().read(id).map(|e| e.len()).unwrap_or(0);
        return Ok(format!(
            "<p class=s>Stored as bytes ({stored} encoded).</p>"
        ));
    }
    let d = &derivations[0];
    let args: Vec<String> = d
        .args
        .iter()
        .take(12)
        .map(|a| {
            format!(
                "<a href=\"/file/{0}\"><code>{1}</code></a>",
                a.id(),
                &a.id().to_hex()[..10]
            )
        })
        .collect();
    let more = d.args.len().saturating_sub(12);
    Ok(format!(
        "<p class=s>Derived (verified) by <code>{}</code> from {} input(s): {}{}</p>",
        &d.func.to_hex()[..10],
        d.args.len(),
        args.join(" "),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    ))
}

fn search_page<S: Store>(repo: &Repo<S>, rev: &str, q: &str) -> Result<Page, String> {
    let mut html = search_form(rev, q);
    if !q.is_empty() {
        let root = corpus::tree(repo, &corpus::lookup(repo, rev)?)?;
        let hits = corpus::grep(repo, &root, q).map_err(err)?;
        html += &format!("<p>{} file(s) contain it.</p>", hits.len());
        for h in hits {
            html += &format!(
                "<p><a href=\"/file/{}?path={}\">{}</a></p>",
                h.id,
                enc(&h.path),
                esc(&h.path)
            );
            if !h.lines.is_empty() {
                let lines: Vec<String> = h
                    .lines
                    .iter()
                    .map(|(n, l)| format!("{n}: {}", esc(l)))
                    .collect();
                html += &format!("<pre>{}</pre>", lines.join("\n"));
            }
        }
    }
    Ok(Page::Html(format!("ikam: search {q}"), html))
}

fn stats_page<S: Store>(repo: &Repo<S>, rev: &str) -> Result<Page, String> {
    let rev_id = corpus::lookup(repo, rev).ok();
    let s = corpus::stats(repo, rev_id.as_ref()).map_err(err)?;
    let mut html = format!(
        "<p><a href=\"/\">home</a></p><p>{} files ({} distinct-content bytes) in {rev}; {} bytes stored in all ({:.1}%).</p><table>",
        s.files,
        s.input,
        s.stored,
        100.0 * s.stored as f64 / s.input.max(1) as f64
    );
    for (k, (n, b)) in &s.kinds {
        html += &format!("<tr><td>{k}</td><td class=n>{n}</td><td class=n>{b}</td></tr>");
    }
    Ok(Page::Html("ikam: stats".into(), html + "</table>"))
}

fn search_form(rev: &str, q: &str) -> String {
    format!(
        "<form action=\"/search\"><input name=q value=\"{}\" placeholder=\"search text\" autofocus> <input name=rev value=\"{}\" size=10> <button>Search</button></form>",
        esc(q),
        esc(rev)
    )
}

fn page(title: &str, body: &str) -> Vec<u8> {
    format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><title>{}</title><style>{STYLE}</style><h1>{}</h1>{body}",
        esc(title),
        esc(title)
    )
    .into_bytes()
}

pub const STYLE: &str = "body{font:15px/1.45 system-ui,sans-serif;max-width:60rem;margin:1rem auto;padding:0 1rem;color:#222;background:#fff}\
pre{background:#f5f5f5;padding:.6rem;overflow-x:auto;white-space:pre-wrap}td{padding:.1rem .8rem .1rem 0}.n{text-align:right;font-variant-numeric:tabular-nums}\
.s{color:#555}input{font:inherit;padding:.25rem}@media(prefers-color-scheme:dark){body{background:#111;color:#ddd}pre{background:#1d1d1d}a{color:#8ab4f8}.s{color:#aaa}}";

fn mime(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "html" | "htm" => "text/plain; charset=utf-8", // shown as text, never run
        "md" | "txt" | "csv" | "tsv" | "rs" | "py" | "toml" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into()
}
