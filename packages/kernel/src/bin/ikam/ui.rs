//! The web page of `ikam live`: browse the current tree, open, edit, upload,
//! rename and delete files, search, ask questions, and see what changed.
//! Changes go through the WebDAV endpoint (`fetch` with PUT, DELETE, MOVE),
//! the same path a mounted folder takes. `/api/...` answers in JSON, for
//! scripts and the live bench.

use std::sync::Mutex;
use std::time::Instant;

use ikam_kernel::{Id, Kind};

use crate::http::{Request, Response, enc, esc};
use crate::json::{Json, quote};
use crate::live::{Live, humantime, norm};
use crate::{ask, corpus, json as jsonfmt, mcp, serve, text};

const PREVIEW: usize = 256 * 1024;

pub fn handle(live: &Mutex<Live>, req: &Request) -> Response {
    let route = req.path.trim_start_matches('/');
    let (name, rest) = route.split_once('/').unwrap_or((route, ""));
    let result = match (req.method.as_str(), name) {
        ("GET", "") => dir_page(
            &mut live.lock().unwrap(),
            &req.param("dir").unwrap_or_default(),
        ),
        ("GET", "file") => {
            let mut l = live.lock().unwrap();
            match rest.parse::<Id>() {
                Ok(id) => file_page(&mut l, None, id),
                Err(_) => {
                    let path = norm(&req.param("path").unwrap_or_default());
                    match l.stat(&path) {
                        Ok(Some((Kind::File, id))) => file_page(&mut l, Some(&path), id),
                        Ok(_) => Err(format!("no file {path}")),
                        Err(e) => Err(e),
                    }
                }
            }
        }
        ("GET", "edit") => edit_page(
            &mut live.lock().unwrap(),
            &req.param("path").unwrap_or_default(),
        ),
        ("GET", "search") => {
            search_page(&live.lock().unwrap(), &req.param("q").unwrap_or_default())
        }
        ("GET", "ask") => ask_page(
            live,
            &req.param("q").unwrap_or_default(),
            req.param("llm").is_some(),
        ),
        ("GET", "log") => log_page(&mut live.lock().unwrap()),
        ("GET", "stats") => stats_page(&live.lock().unwrap()),
        ("GET", "api") => return api(live, rest, req),
        ("POST", "api") => return api(live, rest, req),
        _ => return Response::text(404, "no such page"),
    };
    match result {
        Ok(r) => r,
        Err(e) => Response::new(404).body(
            "text/html; charset=utf-8",
            page("Not found", &format!("<p>{}</p>", esc(&e))),
        ),
    }
}

fn page(title: &str, body: &str) -> Vec<u8> {
    format!(
        "<!doctype html><meta charset=utf-8><base href=\"/\"><meta name=viewport content=\"width=device-width\"><title>{t}</title><style>{}{EXTRA}</style>\
<nav><a href=\"./\">files</a> · <a href=\"log\">changes</a> · <a href=\"stats\">store</a>\
<form action=\"search\"><input name=q placeholder=\"search text\"></form>\
<form action=\"ask\"><input name=q placeholder=\"ask a question\" size=30></form></nav><h1>{t}</h1>{body}<script>{SCRIPT}</script>",
        serve::STYLE,
        t = esc(title),
    )
    .into_bytes()
}

fn html(title: &str, body: &str) -> Response {
    Response::new(200).body("text/html; charset=utf-8", page(title, body))
}

const EXTRA: &str = "nav{display:flex;gap:.6rem;align-items:center;flex-wrap:wrap}nav form{margin:0}\
button{font:inherit}textarea{width:100%;font:13px/1.4 ui-monospace,monospace}.ev{font:12px ui-monospace,monospace;color:#777}\
.bar{display:flex;gap:.5rem;flex-wrap:wrap;margin:.5rem 0}";

/// Edits through WebDAV; the mutating API calls carry `X-Ikam: 1`, which a
/// cross-site form cannot send.
const SCRIPT: &str = r#"
async function dav(method, path, body, headers) {
  const r = await fetch('dav/' + path.split('/').map(encodeURIComponent).join('/'),
    {method, body, headers: Object.assign({'X-Ikam': '1'}, headers || {})});
  if (!r.ok) { alert(method + ' ' + path + ': ' + r.status); throw r.status; }
  return r;
}
async function api(name) {
  const r = await fetch('api/' + name, {method: 'POST', headers: {'X-Ikam': '1'}});
  const t = await r.text(); alert(t); location.reload();
}
async function upload(dir, input) {
  for (const f of input.files) await dav('PUT', (dir ? dir + '/' : '') + f.name, f);
  location.reload();
}
async function newfile(dir) {
  const n = prompt('New file name'); if (!n) return;
  const p = (dir ? dir + '/' : '') + n;
  await dav('PUT', p, ''); location = 'edit?path=' + encodeURIComponent(p);
}
async function newdir(dir) {
  const n = prompt('New folder name'); if (!n) return;
  await dav('MKCOL', (dir ? dir + '/' : '') + n); location.reload();
}
async function rm(p) { if (confirm('Delete ' + p + '?')) { await dav('DELETE', p); location.reload(); } }
async function mv(p) {
  const to = prompt('Move or rename to', p); if (!to || to == p) return;
  await dav('MOVE', p, null, {Destination: new URL('dav/' + to.split('/').map(encodeURIComponent).join('/'), document.baseURI).pathname});
  location.reload();
}
async function save(p) {
  await dav('PUT', p, document.getElementById('body').value);
  location = 'file?path=' + encodeURIComponent(p);
}
"#;

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.into()
    } else {
        format!("{dir}/{name}")
    }
}

fn js(s: &str) -> String {
    esc(&quote(s))
}

fn dir_page(l: &mut Live, dir: &str) -> Result<Response, String> {
    let dir = norm(dir);
    let tree = match l.stat(&dir)? {
        Some((Kind::Tree, id)) => id,
        _ => return Err(format!("no folder {dir}")),
    };
    let entries = l.entries(&tree)?;
    let mut crumbs = "<a href=\"./\">/</a>".to_string();
    let mut acc = String::new();
    for part in dir.split('/').filter(|p| !p.is_empty()) {
        acc = join(&acc, part);
        crumbs += &format!(" <a href=\"?dir={}\">{}</a> /", enc(&acc), esc(part));
    }
    let mut body = format!(
        "<p>{crumbs}</p><div class=bar><label><button onclick=\"this.nextElementSibling.click()\">Upload</button><input type=file multiple hidden onchange='upload({d},this)'></label>\
<button onclick='newfile({d})'>New file</button><button onclick='newdir({d})'>New folder</button>\
<button onclick=\"api('commit')\">Commit now ({})</button><button onclick=\"api('repack')\">Repack</button></div><table>",
        l.pending(),
        d = js(&dir),
    );
    for e in entries.iter() {
        let path = join(&dir, &e.name);
        let actions = format!(
            "<button onclick='mv({p})'>move</button> <button onclick='rm({p})'>delete</button>",
            p = js(&path)
        );
        match e.kind {
            Kind::Tree => {
                body += &format!(
                    "<tr><td><a href=\"?dir={}\">{}/</a></td><td></td><td></td><td>{actions}</td></tr>",
                    enc(&path),
                    esc(&e.name)
                )
            }
            Kind::File => {
                let size = l.size(&e.id)?;
                body += &format!(
                    "<tr><td><a href=\"file?path={}\">{}</a></td><td class=n>{size}</td><td class=s>{}</td><td>{actions}</td></tr>",
                    enc(&path),
                    esc(&e.name),
                    humantime(l.mtime(&path))
                )
            }
            Kind::Graph => {}
        }
    }
    body += "</table><h2>Activity</h2>";
    for ev in l.events.iter().rev().take(30) {
        body += &format!("<div class=ev>{}</div>", esc(ev));
    }
    Ok(html(&format!("ikam live: /{dir}"), &body))
}

fn file_page(l: &mut Live, path: Option<&str>, id: Id) -> Result<Response, String> {
    let bytes = l.read(&id)?;
    let mut body = String::new();
    if let Some(path) = path {
        body += &format!(
            "<p class=bar>{} bytes · <a href=\"dav/{p}\">download</a> · <a href=\"edit?path={p}\">edit</a> \
<button onclick='mv({j})'>move</button> <button onclick='rm({j})'>delete</button></p>",
            bytes.len(),
            p = enc(path),
            j = js(path),
        );
    }
    body += &serve::storage(&l.repo, &id)?;
    if let Some(path) = path {
        let versions = l.versions(path)?;
        if versions.len() > 1 || l.pending() > 0 {
            body += "<details><summary>Versions</summary><table>";
            for (commit, msg, at) in versions {
                let what = match at {
                    Some(v) => format!(
                        "<a href=\"file/{v}\"><code>{}</code></a>",
                        &v.to_hex()[..10]
                    ),
                    None => "deleted".into(),
                };
                body += &format!(
                    "<tr><td><code>{}</code></td><td>{what}</td><td>{}</td></tr>",
                    &commit.to_hex()[..10],
                    esc(&msg)
                );
            }
            body += "</table></details>";
        }
    }
    match std::str::from_utf8(&bytes[..bytes.len().min(PREVIEW)]) {
        Ok(t) => body += &format!("<pre>{}</pre>", esc(t)),
        Err(_) => match l.extracted_text(&id) {
            Some(t) if !t.is_empty() => {
                body += &format!(
                    "<p class=s>Binary file; the text search and questions read:</p><pre>{}</pre>",
                    esc(&t[..floor(&t, PREVIEW)])
                )
            }
            _ => body += "<p>Binary file.</p>",
        },
    }
    let title = path.map_or_else(|| id.to_hex()[..12].to_string(), str::to_string);
    Ok(html(&title, &body))
}

fn floor(s: &str, n: usize) -> usize {
    let mut n = n.min(s.len());
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    n
}

fn edit_page(l: &mut Live, path: &str) -> Result<Response, String> {
    let path = norm(path);
    let text = match l.stat(&path)? {
        Some((Kind::File, id)) => {
            let b = l.read(&id)?;
            String::from_utf8(b.to_vec()).map_err(|_| format!("{path} is not text"))?
        }
        _ => String::new(),
    };
    Ok(html(
        &format!("edit {path}"),
        &format!(
            "<textarea id=body rows=30>{}</textarea><p><button onclick='save({})'>Save</button></p>",
            esc(&text),
            js(&path)
        ),
    ))
}

fn search_page(l: &Live, q: &str) -> Result<Response, String> {
    let t = Instant::now();
    let (hits, total) = text::search(&l.docs, q, 200);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let mut body = format!(
        "<p>{total} file(s) or archive member(s) contain “{}” ({ms:.1} ms over {} files).</p>",
        esc(q),
        l.docs.len()
    );
    for h in hits {
        let member = if h.part.is_empty() {
            String::new()
        } else {
            format!(" <span class=s>({})</span>", esc(&h.part))
        };
        body += &format!(
            "<p><a href=\"file?path={}\">{}</a>{member}</p>",
            enc(&h.path),
            esc(&h.path)
        );
        if !h.lines.is_empty() {
            let lines: Vec<String> = h
                .lines
                .iter()
                .map(|(n, s)| format!("{n}: {}", esc(s)))
                .collect();
            body += &format!("<pre>{}</pre>", lines.join("\n"));
        }
    }
    Ok(html(&format!("search {q}"), &body))
}

fn ask_page(live: &Mutex<Live>, q: &str, llm: bool) -> Result<Response, String> {
    let t = Instant::now();
    let (passages, allowed) = {
        let l = live.lock().unwrap();
        (l.retrieve(q, 8), l.llm)
    };
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let mut body = String::new();
    if llm && allowed {
        // The lock is not held while the model writes.
        match ask::answer(q, &passages) {
            Ok(a) => body += &format!("<h2>Answer</h2><pre>{}</pre>", esc(&a)),
            Err(e) => body += &format!("<p>No answer: {}</p>", esc(&e)),
        }
    } else if allowed {
        body += &format!(
            "<p><a href=\"ask?q={}&llm=1\">Write an answer from these sources with Claude</a></p>",
            enc(q)
        );
    }
    body += &format!(
        "<h2>Sources</h2><p class=s>{} passage(s), {ms:.1} ms.</p>",
        passages.len()
    );
    for (i, p) in passages.iter().enumerate() {
        body += &format!(
            "<p><a href=\"file?path={}\">{}</a> <span class=s>score {:.2}</span></p><pre>{}</pre>",
            enc(&p.path),
            esc(&ask::source_label(i + 1, p)),
            p.score,
            esc(&p.text)
        );
    }
    Ok(html(&format!("ask: {q}"), &body))
}

fn log_page(l: &mut Live) -> Result<Response, String> {
    let mut body = format!("<p>{} change(s) not yet committed.</p>", l.pending());
    for (id, msg, changes) in l.history(50)? {
        body += &format!(
            "<h3><code>{}</code> {}</h3><pre>",
            &id.to_hex()[..12],
            esc(&msg)
        );
        for c in changes.iter().take(100) {
            body += &esc(c);
            body.push('\n');
        }
        if changes.len() > 100 {
            body += &format!("… and {} more\n", changes.len() - 100);
        }
        body += "</pre>";
    }
    Ok(html("changes", &body))
}

fn stats_page(l: &Live) -> Result<Response, String> {
    let s = corpus::stats(&l.repo, l.head().as_ref()).map_err(corpus::err)?;
    let mut body = format!(
        "<p>{} files ({} distinct-content bytes) at the last commit; {} bytes stored in all ({:.1}%).</p><table>",
        s.files,
        s.input,
        s.stored,
        100.0 * s.stored as f64 / s.input.max(1) as f64
    );
    for (k, (n, b)) in &s.kinds {
        body += &format!("<tr><td>{k}</td><td class=n>{n}</td><td class=n>{b}</td></tr>");
    }
    Ok(html("store", &(body + "</table>")))
}

fn json(v: Json) -> Response {
    Response::new(200).body("application/json", v.encode())
}

fn num(x: f64) -> Json {
    Json::Num(x)
}

/// E360: the agent tools (`ikam mcp`) over HTTP, on the served `Live`.
/// Tools that change nothing answer GET with their arguments in the query;
/// the rest take POST, with a JSON object of arguments as the body (or the
/// query). The answer is `{"text": ..., "error": false}`, or status 400
/// with `"error": true`. Drafts move the whole served session onto the
/// draft until it is promoted or discarded.
fn tool(live: &Mutex<Live>, name: &str, req: &Request) -> Response {
    let known = mcp::tool_list();
    let known = known
        .as_arr()
        .iter()
        .any(|t| t.get("name").and_then(Json::as_str) == Some(name));
    if !known {
        return Response::text(404, format!("no tool {name}"));
    }
    let reads = mcp::READ_TOOLS.contains(&name);
    if req.method != "POST" && !reads {
        return Response::text(405, format!("{name} changes the store: send it with POST"));
    }
    let args = if req.method == "POST" && !req.body.is_empty() {
        match std::str::from_utf8(&req.body)
            .map_err(|e| e.to_string())
            .and_then(jsonfmt::parse)
        {
            Ok(a @ Json::Obj(_)) => a,
            Ok(_) => return Response::text(400, "the body must be a JSON object"),
            Err(e) => return Response::text(400, format!("bad JSON: {e}")),
        }
    } else {
        let keys: Vec<&str> = req
            .query
            .split('&')
            .filter_map(|kv| kv.split_once('=').map(|(k, _)| k).or(Some(kv)))
            .filter(|k| !k.is_empty())
            .collect();
        Json::Obj(
            keys.into_iter()
                .filter_map(|k| Some((k.to_string(), Json::Str(req.param(k)?))))
                .collect(),
        )
    };
    let mut session = mcp::Session {
        live: live.lock().unwrap(),
    };
    let (status, text, error) = match session.call(name, &args) {
        Ok(t) => (200, t, false),
        Err(e) => (400, e, true),
    };
    let body = Json::Obj(vec![
        ("text".into(), Json::Str(mcp::capped(text))),
        ("error".into(), Json::Bool(error)),
    ]);
    Response::new(status).body("application/json", body.encode())
}

fn api(live: &Mutex<Live>, name: &str, req: &Request) -> Response {
    if req.method == "POST" && req.header("x-ikam").is_none() {
        return Response::text(403, "missing X-Ikam header");
    }
    let r = (|| -> Result<Response, String> {
        Ok(match (req.method.as_str(), name) {
            ("GET", "status") => {
                let l = live.lock().unwrap();
                json(Json::Obj(vec![
                    ("files".into(), num(l.files.len() as f64)),
                    ("pending".into(), num(l.pending() as f64)),
                    (
                        "head".into(),
                        l.head().map_or(Json::Null, |h| Json::Str(h.to_hex())),
                    ),
                    ("store_bytes".into(), num(l.store_bytes() as f64)),
                    (
                        "text_bytes".into(),
                        num(l.docs.values().map(|d| d.bytes()).sum::<usize>() as f64),
                    ),
                ]))
            }
            ("GET", "docs") => {
                // Extracted text per file, largest first.
                let l = live.lock().unwrap();
                let mut v: Vec<(&String, usize)> =
                    l.docs.iter().map(|(p, d)| (p, d.bytes())).collect();
                v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
                json(Json::Arr(
                    v.into_iter()
                        .map(|(p, n)| Json::Arr(vec![Json::Str(p.clone()), num(n as f64)]))
                        .collect(),
                ))
            }
            ("GET", "search") => {
                let q = req.param("q").unwrap_or_default();
                let l = live.lock().unwrap();
                let t = Instant::now();
                let (hits, total) = text::search(&l.docs, &q, 1000);
                let ms = t.elapsed().as_secs_f64() * 1e3;
                json(Json::Obj(vec![
                    ("total".into(), num(total as f64)),
                    ("ms".into(), num(ms)),
                    (
                        "hits".into(),
                        Json::Arr(
                            hits.into_iter()
                                .map(|h| {
                                    Json::Obj(vec![
                                        ("path".into(), Json::Str(h.path)),
                                        ("part".into(), Json::Str(h.part)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ]))
            }
            ("GET", "ask") => {
                let q = req.param("q").unwrap_or_default();
                let (passages, ms) = {
                    let l = live.lock().unwrap();
                    let t = Instant::now();
                    let p = l.retrieve(&q, 8);
                    (p, t.elapsed().as_secs_f64() * 1e3)
                };
                let answer = if req.param("llm").is_some() && live.lock().unwrap().llm {
                    Some(ask::answer(&q, &passages))
                } else {
                    None
                };
                let mut fields = vec![
                    ("ms".into(), num(ms)),
                    (
                        "sources".into(),
                        Json::Arr(
                            passages
                                .iter()
                                .map(|p| {
                                    Json::Obj(vec![
                                        ("path".into(), Json::Str(p.path.clone())),
                                        ("part".into(), Json::Str(p.part.clone())),
                                        ("line".into(), num(p.first_line as f64)),
                                        ("score".into(), num(p.score)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ];
                match answer {
                    Some(Ok(a)) => fields.push(("answer".into(), Json::Str(a))),
                    Some(Err(e)) => fields.push(("error".into(), Json::Str(e))),
                    None => {}
                }
                json(Json::Obj(fields))
            }
            ("GET", "tools") => json(mcp::tool_list()),
            (_, t) if t.starts_with("tool/") => tool(live, &t["tool/".len()..], req),
            ("POST", "commit") => {
                // E360: `?message=` names the commit.
                let mut l = live.lock().unwrap();
                match l.flush_with(true, req.param("message").as_deref())? {
                    Some(c) => Response::text(200, format!("committed {}", &c.to_hex()[..12])),
                    None => Response::text(200, "nothing to commit"),
                }
            }
            ("POST", "repack") => Response::text(200, live.lock().unwrap().repack()?),
            ("POST", "gc") => Response::text(200, live.lock().unwrap().gc()?),
            ("POST", "fsck") => {
                let mut l = live.lock().unwrap();
                l.flush(true)?;
                let failures = l.repo.fsck_deep().map_err(corpus::err)?;
                Response::text(200, format!("fsck failures={}", failures.len()))
            }
            _ => Response::text(404, "no such call"),
        })
    })();
    r.unwrap_or_else(|e| Response::text(500, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(live: &Mutex<Live>, method: &str, target: &str, body: &str) -> (u16, Json) {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let req = Request {
            method: method.into(),
            path: path.into(),
            query: query.into(),
            headers: vec![("X-Ikam".into(), "1".into())],
            body: body.as_bytes().to_vec(),
        };
        let r = handle(live, &req);
        let j = jsonfmt::parse(&String::from_utf8_lossy(&r.body))
            .unwrap_or_else(|_| Json::Str(String::from_utf8_lossy(&r.body).into()));
        (r.status, j)
    }

    fn text(j: &Json) -> String {
        j.get("text")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string()
    }

    /// E360: drafts, commit messages, history and reads at a commit over
    /// HTTP, through the same handlers as `ikam mcp`.
    #[test]
    fn agent_tools_over_http() {
        let dir = std::env::temp_dir().join(format!("ikam-ui-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut l = Live::open(dir.to_str().unwrap(), "main").unwrap();
        l.index_all(false).unwrap();
        let live = Mutex::new(l);
        let (st, j) = call(&live, "GET", "/api/tools", "");
        assert_eq!(st, 200);
        assert!(j.as_arr().len() >= 16);
        // Writes go by POST; a GET of one is refused, unknown tools are 404.
        let w = r#"{"path":"notes/a.md","content":"one\ntwo\n"}"#;
        assert_eq!(call(&live, "GET", "/api/tool/write?path=x", "").0, 405);
        assert_eq!(call(&live, "GET", "/api/tool/nope", "").0, 404);
        assert_eq!(call(&live, "POST", "/api/tool/write", w).0, 200);
        // The server's timed commit leaves tool edits for their own commit.
        {
            let mut l = live.lock().unwrap();
            l.commit_after = std::time::Duration::ZERO;
            assert_eq!(l.flush(false).unwrap(), None);
            assert_eq!(l.pending(), 1);
        }
        let (st, j) = call(&live, "POST", "/api/tool/commit", r#"{"message":"first"}"#);
        assert_eq!(st, 200, "{}", text(&j));
        // `/api/commit` takes a message too.
        let e = r#"{"path":"notes/a.md","old":"two","new":"three"}"#;
        assert_eq!(call(&live, "POST", "/api/tool/edit", e).0, 200);
        let (st, _) = call(&live, "POST", "/api/commit?message=second%20edit", "");
        assert_eq!(st, 200);
        let (st, j) = call(&live, "GET", "/api/tool/history?limit=5", "");
        assert_eq!(st, 200);
        let h = text(&j);
        assert!(h.contains("first") && h.contains("second edit"), "{h}");
        // Read at the first commit: the text before the edit.
        let first = h
            .lines()
            .find(|l| l.contains("first"))
            .and_then(|l| l.split_whitespace().next())
            .unwrap()
            .to_string();
        let (st, j) = call(
            &live,
            "GET",
            &format!("/api/tool/read?path=notes/a.md&at={first}"),
            "",
        );
        assert_eq!(st, 200, "{}", text(&j));
        assert!(text(&j).contains("two") && !text(&j).contains("three"));
        // A draft keeps its commits off main until promoted.
        assert_eq!(
            call(&live, "POST", "/api/tool/draft_open", r#"{"name":"try"}"#).0,
            200
        );
        let w2 = r#"{"path":"notes/b.md","content":"draft\n"}"#;
        assert_eq!(call(&live, "POST", "/api/tool/write", w2).0, 200);
        assert_eq!(
            call(
                &live,
                "POST",
                "/api/tool/commit",
                r#"{"message":"on draft"}"#
            )
            .0,
            200
        );
        let main_has_b = |live: &Mutex<Live>| {
            call(live, "GET", "/api/tool/read?path=notes/b.md&at=main", "").0 == 200
        };
        assert!(!main_has_b(&live));
        let (st, j) = call(&live, "POST", "/api/tool/draft_promote", "");
        assert_eq!(st, 200, "{}", text(&j));
        assert!(main_has_b(&live));
        assert_eq!(live.lock().unwrap().refname, "main");
        // A tool error is a 400 with `error: true`.
        let (st, j) = call(&live, "POST", "/api/tool/draft_discard", "");
        assert_eq!((st, j.get("error")), (400, Some(&Json::Bool(true))));
        // POSTs without the header a cross-site form cannot send are refused.
        let req = Request {
            method: "POST".into(),
            path: "/api/tool/write".into(),
            query: String::new(),
            headers: Vec::new(),
            body: w.as_bytes().to_vec(),
        };
        assert_eq!(handle(&live, &req).status, 403);
        let _ = std::fs::remove_dir_all(dir);
    }
}
