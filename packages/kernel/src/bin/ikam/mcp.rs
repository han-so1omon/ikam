//! `ikam mcp` (E359): the live handlers as tools an AI agent can call, over
//! the Model Context Protocol's stdio transport (JSON-RPC 2.0, one message a
//! line on stdin and stdout; logs go to stderr).
//!
//! - One process owns the store through a `Live` (as `ikam live` does), so an
//!   edit stores only what changed and every read sees pending edits.
//! - Edits wait in the working tree until the agent calls `commit` with a
//!   message of its own; nothing is committed on a timer. Edits still pending
//!   when stdin closes are committed as "mcp: N change(s) left at exit", so
//!   no work is lost.
//! - Reads and diffs take a revision (`at`, `from`, `to`): a commit id or a
//!   prefix of one, a ref, or `work` for the working tree (the default).
//! - Drafts (E266) are refs under `drafts/`: `draft_open` moves the session
//!   onto one, `draft_promote` merges it into the ref the session started on.
//! - Output is capped (`MAX_TEXT`) and says how to page on, so one call never
//!   floods the agent's context.

use std::io::{BufRead, Write};

use ikam_kernel::merge::Merged;
use ikam_kernel::{Id, Kind};

use crate::corpus::{self, err};
use crate::json::{self, Json};
use crate::live::{self, Live, Op};
use crate::text;

/// Characters of text a tool returns at most.
pub const MAX_TEXT: usize = 64 * 1024;

const VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

pub struct Session {
    live: Live,
    store_dir: String,
    /// The ref the session started on; drafts are made at, and promoted to, it.
    target: String,
}

/// Serve one client on `input`/`output` until `input` closes.
pub fn run(
    store_dir: &str,
    refname: &str,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<(), String> {
    let mut s = Session::open(store_dir, refname)?;
    for line in input.lines() {
        let line = line.map_err(err)?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = s.handle(&line) {
            writeln!(output, "{}", reply.encode()).map_err(err)?;
            output.flush().map_err(err)?;
        }
    }
    let n = s.live.pending();
    if n > 0 {
        let msg = format!("mcp: {n} change(s) left at exit");
        if let Some(c) = s.live.flush_with(true, Some(&msg))? {
            eprintln!("{msg}: committed as {c}");
        }
    }
    Ok(())
}

impl Session {
    pub fn open(store_dir: &str, refname: &str) -> Result<Session, String> {
        let mut live = Live::open(store_dir, refname)?;
        live.index_all(false)?;
        Ok(Session {
            live,
            store_dir: store_dir.into(),
            target: refname.into(),
        })
    }

    /// The reply to one JSON-RPC message, or `None` for a notification.
    pub fn handle(&mut self, line: &str) -> Option<Json> {
        let msg = match json::parse(line) {
            Ok(m) => m,
            Err(e) => return Some(error(Json::Null, -32700, &format!("parse error: {e}"))),
        };
        let id = msg.get("id").cloned()?;
        let method = msg.get("method").and_then(Json::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Json::Obj(Vec::new()));
        let result = match method {
            "initialize" => initialize(&params),
            "ping" => Json::Obj(Vec::new()),
            "tools/list" => obj(vec![("tools", Json::Arr(tools()))]),
            "tools/call" => {
                let name = params.get("name").and_then(Json::as_str).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(Json::Obj(Vec::new()));
                let (text, is_error) = match self.call(name, &args) {
                    Ok(t) => (t, false),
                    Err(e) => (e, true),
                };
                let content = obj(vec![("type", s("text")), ("text", s(&cap(text)))]);
                obj(vec![
                    ("content", Json::Arr(vec![content])),
                    ("isError", Json::Bool(is_error)),
                ])
            }
            _ => return Some(error(id, -32601, &format!("method not found: {method}"))),
        };
        Some(obj(vec![
            ("jsonrpc", s("2.0")),
            ("id", id),
            ("result", result),
        ]))
    }

    /// Run tool `name`; `Err` is a tool error the agent reads, not a protocol one.
    pub fn call(&mut self, name: &str, a: &Json) -> Result<String, String> {
        match name {
            "list" => self.list(&opt(a, "path").unwrap_or_default(), opt(a, "at").as_deref()),
            "read" => self.read(
                &req(a, "path")?,
                opt(a, "at").as_deref(),
                num(a, "start_line"),
                num(a, "end_line"),
            ),
            "search" => Ok(self.search(&req(a, "query")?, num(a, "limit").unwrap_or(50))),
            "passages" => Ok(self.passages(&req(a, "question")?, num(a, "k").unwrap_or(8))),
            "write" => {
                let path = req(a, "path")?;
                let content = req(a, "content")?;
                self.apply(
                    Op::Put(path.clone(), content.into_bytes()),
                    &format!("wrote {path}"),
                )
            }
            "edit" => self.edit(
                &req(a, "path")?,
                &req(a, "old")?,
                &req(a, "new")?,
                flag(a, "all"),
            ),
            "remove" => {
                let path = live::norm(&req(a, "path")?);
                if self.live.stat(&path)?.is_none() {
                    return Err(format!("{path}: no such file or folder"));
                }
                self.apply(Op::Remove(path.clone()), &format!("removed {path}"))
            }
            "move" => {
                let (from, to) = (req(a, "from")?, req(a, "to")?);
                if self.live.stat(&live::norm(&from))?.is_none() {
                    return Err(format!("{from}: no such file or folder"));
                }
                self.apply(
                    Op::Rename(from.clone(), to.clone()),
                    &format!("moved {from} to {to}"),
                )
            }
            "status" => self.status(),
            "commit" => match self.live.flush_with(true, Some(&req(a, "message")?))? {
                Some(c) => Ok(format!("committed {c} on {}", self.live.refname)),
                None => Ok("nothing to commit".into()),
            },
            "history" => self.history(opt(a, "path").as_deref(), num(a, "limit").unwrap_or(20)),
            "diff" => self.diff(
                opt(a, "from").as_deref(),
                opt(a, "to").as_deref(),
                opt(a, "path").as_deref(),
            ),
            "drafts" => self.drafts(),
            "draft_open" => self.draft_open(&req(a, "name")?),
            "draft_promote" => self.draft_promote(opt(a, "message").as_deref()),
            "draft_discard" => self.draft_discard(),
            _ => Err(format!("no tool {name}")),
        }
    }

    /// The tree at revision `rev` (`None` or `work`: the working tree).
    fn tree_at(&mut self, rev: Option<&str>) -> Result<Id, String> {
        match rev {
            None | Some("work") | Some("") => Ok(self.live.work),
            Some(r) => corpus::tree(&self.live.repo, &corpus::lookup(&self.live.repo, r)?),
        }
    }

    fn stat_in(&mut self, root: Id, path: &str) -> Result<Option<(Kind, Id)>, String> {
        let mut cur = (Kind::Tree, root);
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if cur.0 != Kind::Tree {
                return Ok(None);
            }
            match self.live.entries(&cur.1)?.iter().find(|e| e.name == part) {
                Some(e) => cur = (e.kind, e.id),
                None => return Ok(None),
            }
        }
        Ok(Some(cur))
    }

    fn list(&mut self, path: &str, at: Option<&str>) -> Result<String, String> {
        let root = self.tree_at(at)?;
        let path = live::norm(path);
        let Some((Kind::Tree, id)) = self.stat_in(root, &path)? else {
            return Err(format!("{path}: not a folder"));
        };
        let mut out = Vec::new();
        for e in self.live.entries(&id)?.iter() {
            match e.kind {
                Kind::Tree => out.push(format!("{}/", e.name)),
                Kind::File => out.push(format!("{}\t{} bytes", e.name, self.live.size(&e.id)?)),
                Kind::Graph => out.push(format!("{}\t(graph)", e.name)),
            }
        }
        Ok(if out.is_empty() {
            "(empty)".into()
        } else {
            out.join("\n")
        })
    }

    /// A file's text (lines `start..=end`, numbered from 1); a binary file's
    /// extracted text.
    fn read(
        &mut self,
        path: &str,
        at: Option<&str>,
        start: Option<usize>,
        end: Option<usize>,
    ) -> Result<String, String> {
        let path = live::norm(path);
        let root = self.tree_at(at)?;
        let Some((Kind::File, id)) = self.stat_in(root, &path)? else {
            return Err(format!("{path}: no such file"));
        };
        let bytes = self.live.read(&id)?;
        let (head, body) = match std::str::from_utf8(&bytes) {
            Ok(t) => (String::new(), t.to_string()),
            Err(_) => {
                let doc = text::extract(&path, &bytes);
                let parts: Vec<String> = doc
                    .parts
                    .iter()
                    .map(|p| {
                        if p.name.is_empty() {
                            p.text.clone()
                        } else {
                            format!("--- {}\n{}", p.name, p.text)
                        }
                    })
                    .collect();
                (
                    format!("binary file, {} bytes; its extracted text:\n", bytes.len()),
                    parts.join("\n"),
                )
            }
        };
        let lines: Vec<&str> = body.split_inclusive('\n').collect();
        let first = start.unwrap_or(1).max(1);
        let last = end.unwrap_or(lines.len()).min(lines.len());
        if lines.is_empty() || first > last {
            return Ok(format!(
                "{head}{path}: {} line(s), none in that range",
                lines.len()
            ));
        }
        let mut out = String::new();
        let mut shown = first - 1;
        for line in &lines[first - 1..last] {
            if out.len() + line.len() > MAX_TEXT - 200 && shown >= first {
                break;
            }
            out.push_str(line);
            shown += 1;
        }
        let mut msg = format!(
            "{head}{path}: lines {first}-{shown} of {}\n{out}",
            lines.len()
        );
        if shown < last {
            msg.push_str(&format!(
                "\n(cut at line {shown}; read on with start_line={})",
                shown + 1
            ));
        }
        Ok(msg)
    }

    fn search(&mut self, query: &str, limit: usize) -> String {
        let (hits, total) = text::search(&self.live.docs, query, limit);
        let mut out = format!("{total} file(s) contain {query:?} in the working tree\n");
        for h in hits {
            let part = if h.part.is_empty() {
                String::new()
            } else {
                format!(" ({})", h.part)
            };
            out.push_str(&format!("{}{part}\n", h.path));
            for (n, line) in h.lines {
                out.push_str(&format!("  {n}: {line}\n"));
            }
        }
        out
    }

    fn passages(&mut self, question: &str, k: usize) -> String {
        let found = text::retrieve(&self.live.docs, question, k);
        if found.is_empty() {
            return "no passage shares a word with the question".into();
        }
        let mut out = String::new();
        for p in found {
            let part = if p.part.is_empty() {
                String::new()
            } else {
                format!(" ({})", p.part)
            };
            out.push_str(&format!(
                "{}{part} line {} (score {:.2})\n{}\n\n",
                p.path, p.first_line, p.score, p.text
            ));
        }
        out
    }

    fn apply(&mut self, op: Op, done: &str) -> Result<String, String> {
        if let Op::Put(_, b) = &op
            && !self.live.has_room(b.len())
        {
            return Err("the store is full (--max-store)".into());
        }
        let cost = self.live.apply(vec![op])?;
        Ok(format!(
            "{done} ({} new bytes stored); {} change(s) pending, call commit to keep them",
            cost.new_bytes,
            self.live.pending()
        ))
    }

    /// Replace `old` with `new` in a text file: once (it must occur once)
    /// or, with `all`, everywhere.
    fn edit(&mut self, path: &str, old: &str, new: &str, all: bool) -> Result<String, String> {
        let path = live::norm(path);
        let Some((Kind::File, id)) = self.live.stat(&path)? else {
            return Err(format!("{path}: no such file"));
        };
        let bytes = self.live.read(&id)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| format!("{path}: not a text file; use write"))?;
        if old.is_empty() {
            return Err("old is empty".into());
        }
        let n = text.matches(old).count();
        if n == 0 {
            return Err(format!("{path}: old text not found"));
        }
        if n > 1 && !all {
            return Err(format!(
                "{path}: old text occurs {n} times; give more of it, or all=true"
            ));
        }
        let changed = if all {
            text.replace(old, new)
        } else {
            text.replacen(old, new, 1)
        };
        self.apply(
            Op::Put(path.clone(), changed.into_bytes()),
            &format!("edited {path} ({n} place(s))"),
        )
    }

    fn status(&mut self) -> Result<String, String> {
        let mut changes = Vec::new();
        let (base, work) = (self.live.base_tree(), Some(self.live.work));
        self.live.diff(base, work, "", &mut changes)?;
        let head = self.live.head().map_or("none".into(), |c| c.to_string());
        let mut out = format!(
            "ref {} at {head}; {} edit(s) pending\n",
            self.live.refname,
            self.live.pending()
        );
        out.push_str(&if changes.is_empty() {
            "working tree equals the head".into()
        } else {
            changes.join("\n")
        });
        Ok(out)
    }

    fn history(&mut self, path: Option<&str>, limit: usize) -> Result<String, String> {
        let mut out = Vec::new();
        if let Some(path) = path {
            for (c, msg, at) in self
                .live
                .versions(&live::norm(path))?
                .into_iter()
                .take(limit)
            {
                let what = at.map_or("removed".into(), |id| {
                    format!("content {}", &id.to_hex()[..12])
                });
                out.push(format!("{c} {msg}: {what}"));
            }
        } else {
            for (c, msg, changes) in self.live.history(limit)? {
                out.push(format!("{c} {msg}\n  {}", changes.join("\n  ")));
            }
        }
        Ok(if out.is_empty() {
            "no commits".into()
        } else {
            out.join("\n")
        })
    }

    /// Changed paths between two revisions (default: head to working tree),
    /// or with `path`, a unified diff of that file's text.
    fn diff(
        &mut self,
        from: Option<&str>,
        to: Option<&str>,
        path: Option<&str>,
    ) -> Result<String, String> {
        let a = match from {
            None => self.live.base_tree(),
            Some(r) => Some(self.tree_at(Some(r))?),
        };
        let b = self.tree_at(to)?;
        let Some(path) = path else {
            let mut changes = Vec::new();
            self.live.diff(a, Some(b), "", &mut changes)?;
            return Ok(if changes.is_empty() {
                "no changes".into()
            } else {
                changes.join("\n")
            });
        };
        let path = live::norm(path);
        let mut text_at = |root: Option<Id>| -> Result<String, String> {
            let Some(root) = root else {
                return Ok(String::new());
            };
            match self.stat_in(root, &path)? {
                Some((Kind::File, id)) => {
                    Ok(String::from_utf8_lossy(&self.live.read(&id)?).into_owned())
                }
                _ => Ok(String::new()),
            }
        };
        let (x, y) = (text_at(a)?, text_at(Some(b))?);
        if x == y {
            return Ok(format!("{path}: no changes"));
        }
        let d = similar::TextDiff::from_lines(&x, &y);
        Ok(d.unified_diff()
            .context_radius(3)
            .header(&format!("a/{path}"), &format!("b/{path}"))
            .to_string())
    }

    fn drafts(&mut self) -> Result<String, String> {
        let list = self.live.repo.drafts().map_err(err)?;
        let mut out = format!(
            "session on {} (target {})\n",
            self.live.refname, self.target
        );
        for (name, head) in list {
            out.push_str(&format!("{name} at {head}\n"));
        }
        Ok(out)
    }

    fn reopen(&mut self, refname: &str) -> Result<(), String> {
        self.live = Live::open(&self.store_dir, refname)?;
        self.live.index_all(false)
    }

    fn draft_open(&mut self, name: &str) -> Result<String, String> {
        if self.live.refname != self.target {
            return Err(format!(
                "already on {}; promote or discard it first",
                self.live.refname
            ));
        }
        if self.live.pending() > 0 {
            return Err("commit pending edits first".into());
        }
        let draft = self.live.repo.draft(&self.target, name).map_err(err)?;
        self.reopen(&draft)?;
        Ok(format!(
            "on {draft}; edits and commits stay off {} until draft_promote",
            self.target
        ))
    }

    fn on_draft(&self) -> Result<String, String> {
        let r = self.live.refname.clone();
        if r.starts_with(ikam_kernel::drafts::DRAFTS) {
            Ok(r)
        } else {
            Err("not on a draft".into())
        }
    }

    fn draft_promote(&mut self, message: Option<&str>) -> Result<String, String> {
        let draft = self.on_draft()?;
        let msg = message.map_or_else(|| format!("promote {draft}"), String::from);
        self.live.flush_with(true, Some(&msg))?;
        let target = self.target.clone();
        let merged = self
            .live
            .repo
            .promote_draft(&target, &draft, &msg)
            .map_err(err)?;
        let out = match merged {
            Merged::Conflicts(c) => {
                return Err(format!(
                    "conflicts, nothing written; still on {draft}: {c:?}"
                ));
            }
            Merged::UpToDate => format!("{target} already had {draft}"),
            Merged::FastForward(id) => format!("{target} fast-forwarded to {id}"),
            Merged::Commit(id) => format!("{target} merged {draft} as {id}"),
        };
        self.reopen(&target)?;
        Ok(out)
    }

    fn draft_discard(&mut self) -> Result<String, String> {
        let draft = self.on_draft()?;
        self.live.repo.discard_draft(&draft).map_err(err)?;
        let target = self.target.clone();
        self.reopen(&target)?;
        Ok(format!("discarded {draft}; back on {target}"))
    }
}

fn initialize(params: &Json) -> Json {
    let asked = params
        .get("protocolVersion")
        .and_then(Json::as_str)
        .unwrap_or("");
    let version = if VERSIONS.contains(&asked) {
        asked
    } else {
        "2025-06-18"
    };
    obj(vec![
        ("protocolVersion", s(version)),
        (
            "capabilities",
            obj(vec![(
                "tools",
                obj(vec![("listChanged", Json::Bool(false))]),
            )]),
        ),
        (
            "serverInfo",
            obj(vec![
                ("name", s("ikam")),
                ("version", s(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
        (
            "instructions",
            s(
                "A versioned file store. Edits wait in the working tree until commit, which takes your message. \
               Revisions (at, from, to) are a commit id or prefix, a ref, or work (the working tree, the default).",
            ),
        ),
    ])
}

fn tools() -> Vec<Json> {
    let rev = (
        "at",
        "string",
        "revision: commit id or prefix, ref, or work (default)",
    );
    vec![
        tool(
            "list",
            "List a folder: subfolders end in /, files give their size.",
            &[("path", "string", "folder, default the root"), rev],
            &[],
        ),
        tool(
            "read",
            "Read a file's text, or a binary file's extracted text (docx, xlsx, pdf, tar members). Lines number from 1.",
            &[
                ("path", "string", "file"),
                rev,
                ("start_line", "integer", "first line"),
                ("end_line", "integer", "last line"),
            ],
            &["path"],
        ),
        tool(
            "search",
            "Files of the working tree whose text contains a string (case-insensitive), with matching lines.",
            &[
                ("query", "string", "text to find"),
                ("limit", "integer", "files at most (50)"),
            ],
            &["query"],
        ),
        tool(
            "passages",
            "Passages that share the most words with a question, best first.",
            &[
                ("question", "string", "question or keywords"),
                ("k", "integer", "passages (8)"),
            ],
            &["question"],
        ),
        tool(
            "write",
            "Create or replace a file with text content. Pending until commit.",
            &[
                ("path", "string", "file"),
                ("content", "string", "the whole new text"),
            ],
            &["path", "content"],
        ),
        tool(
            "edit",
            "Replace old text with new in a file. old must occur exactly once unless all is true. Pending until commit.",
            &[
                ("path", "string", "file"),
                ("old", "string", "exact text to replace"),
                ("new", "string", "replacement"),
                ("all", "boolean", "replace every occurrence"),
            ],
            &["path", "old", "new"],
        ),
        tool(
            "remove",
            "Remove a file or folder. Pending until commit.",
            &[("path", "string", "file or folder")],
            &["path"],
        ),
        tool(
            "move",
            "Rename or move a file or folder. Pending until commit.",
            &[("from", "string", "path"), ("to", "string", "new path")],
            &["from", "to"],
        ),
        tool(
            "status",
            "The ref, its head commit and the paths pending edits changed.",
            &[],
            &[],
        ),
        tool(
            "commit",
            "Commit pending edits with a message.",
            &[("message", "string", "commit message")],
            &["message"],
        ),
        tool(
            "history",
            "Commits newest first with the paths each changed; with path, the commits where that file changed.",
            &[
                ("path", "string", "file"),
                ("limit", "integer", "commits (20)"),
            ],
            &[],
        ),
        tool(
            "diff",
            "Paths changed between two revisions (default: head to working tree); with path, a unified diff of that file.",
            &[
                ("from", "string", "revision (default head)"),
                ("to", "string", "revision (default work)"),
                ("path", "string", "file"),
            ],
            &[],
        ),
        tool("drafts", "The session's ref and the open drafts.", &[], &[]),
        tool(
            "draft_open",
            "Open a draft at the session's ref and move onto it: edits and commits stay off the ref until draft_promote.",
            &[("name", "string", "draft name")],
            &["name"],
        ),
        tool(
            "draft_promote",
            "Commit pending edits, merge the draft into the ref it was opened at, delete it and move back.",
            &[("message", "string", "message (default: promote NAME)")],
            &[],
        ),
        tool(
            "draft_discard",
            "Delete the draft (its pending edits too) and move back to the ref.",
            &[],
            &[],
        ),
    ]
}

fn tool(name: &str, description: &str, props: &[(&str, &str, &str)], required: &[&str]) -> Json {
    let props = props
        .iter()
        .map(|(k, t, d)| {
            (
                k.to_string(),
                obj(vec![("type", s(t)), ("description", s(d))]),
            )
        })
        .collect();
    let schema = obj(vec![
        ("type", s("object")),
        ("properties", Json::Obj(props)),
        (
            "required",
            Json::Arr(required.iter().map(|r| s(r)).collect()),
        ),
    ]);
    obj(vec![
        ("name", s(name)),
        ("description", s(description)),
        ("inputSchema", schema),
    ])
}

fn error(id: Json, code: i64, message: &str) -> Json {
    let e = obj(vec![
        ("code", Json::Num(code as f64)),
        ("message", s(message)),
    ]);
    obj(vec![("jsonrpc", s("2.0")), ("id", id), ("error", e)])
}

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn s(v: &str) -> Json {
    Json::Str(v.into())
}

fn cap(mut t: String) -> String {
    if t.len() > MAX_TEXT {
        t.truncate(t.floor_char_boundary(MAX_TEXT));
        t.push_str("\n(cut: output over 64 KiB)");
    }
    t
}

fn opt(a: &Json, k: &str) -> Option<String> {
    a.get(k).and_then(Json::as_str).map(String::from)
}

fn req(a: &Json, k: &str) -> Result<String, String> {
    opt(a, k).ok_or_else(|| format!("missing argument {k}"))
}

fn num(a: &Json, k: &str) -> Option<usize> {
    match a.get(k) {
        Some(Json::Num(n)) if *n >= 0.0 => Some(*n as usize),
        Some(Json::Str(t)) => t.parse().ok(),
        _ => None,
    }
}

fn flag(a: &Json, k: &str) -> bool {
    matches!(a.get(k), Some(Json::Bool(true)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str) -> (std::path::PathBuf, Session) {
        let dir = std::env::temp_dir().join(format!("ikam-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Session::open(dir.to_str().unwrap(), "main").unwrap();
        (dir, s)
    }

    fn call(s: &mut Session, name: &str, args: &str) -> Result<String, String> {
        s.call(name, &json::parse(args).unwrap())
    }

    #[test]
    fn handshake_list_and_errors() {
        let (dir, mut s) = session("hs");
        let r = s.handle(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#).unwrap();
        assert_eq!(
            r.get("result")
                .unwrap()
                .get("protocolVersion")
                .unwrap()
                .as_str(),
            Some("2025-06-18")
        );
        assert!(
            s.handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );
        let r = s
            .handle(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#)
            .unwrap();
        assert_eq!(
            r.get("result")
                .unwrap()
                .get("tools")
                .unwrap()
                .as_arr()
                .len(),
            16
        );
        let r = s
            .handle(r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#)
            .unwrap();
        assert!(r.get("error").is_some());
        assert!(s.handle("{oops").unwrap().get("error").is_some());
        let r = s.handle(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read","arguments":{"path":"x"}}}"#).unwrap();
        assert_eq!(
            r.get("result").unwrap().get("isError"),
            Some(&Json::Bool(true))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn edit_commit_diff_and_read_at_a_commit() {
        let (dir, mut s) = session("edit");
        call(
            &mut s,
            "write",
            r#"{"path":"a/b.txt","content":"one\ntwo\nthree\n"}"#,
        )
        .unwrap();
        assert!(
            call(&mut s, "commit", r#"{"message":"first"}"#)
                .unwrap()
                .starts_with("committed")
        );
        let first = s.live.head().unwrap().to_string();
        assert!(
            call(&mut s, "edit", r#"{"path":"a/b.txt","old":"o","new":"0"}"#)
                .unwrap_err()
                .contains("2 times")
        );
        call(
            &mut s,
            "edit",
            r#"{"path":"a/b.txt","old":"two","new":"2"}"#,
        )
        .unwrap();
        assert!(call(&mut s, "status", "{}").unwrap().contains("~ a/b.txt"));
        let d = call(&mut s, "diff", r#"{"path":"a/b.txt"}"#).unwrap();
        assert!(d.contains("-two") && d.contains("+2"), "{d}");
        call(&mut s, "commit", r#"{"message":"second, by the agent"}"#).unwrap();
        assert!(
            call(&mut s, "history", "{}")
                .unwrap()
                .contains("second, by the agent")
        );
        let old = call(
            &mut s,
            "read",
            &format!(r#"{{"path":"a/b.txt","at":"{first}","start_line":2,"end_line":2}}"#),
        )
        .unwrap();
        assert!(old.ends_with("two\n"), "{old}");
        assert!(
            call(&mut s, "read", r#"{"path":"a/b.txt"}"#)
                .unwrap()
                .contains("2\nthree")
        );
        assert!(
            call(&mut s, "search", r#"{"query":"THREE"}"#)
                .unwrap()
                .starts_with("1 file(s)")
        );
        assert_eq!(
            call(&mut s, "list", r#"{"path":"a"}"#).unwrap(),
            "b.txt\t12 bytes"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drafts_stay_off_the_ref_until_promoted() {
        let (dir, mut s) = session("draft");
        call(&mut s, "write", r#"{"path":"f","content":"v1"}"#).unwrap();
        call(&mut s, "commit", r#"{"message":"v1"}"#).unwrap();
        call(&mut s, "draft_open", r#"{"name":"try"}"#).unwrap();
        call(&mut s, "write", r#"{"path":"f","content":"v2"}"#).unwrap();
        call(&mut s, "commit", r#"{"message":"v2 on the draft"}"#).unwrap();
        assert!(
            call(&mut s, "read", r#"{"path":"f","at":"main"}"#)
                .unwrap()
                .ends_with("v1")
        );
        call(&mut s, "draft_promote", "{}").unwrap();
        assert_eq!(s.live.refname, "main");
        assert!(
            call(&mut s, "read", r#"{"path":"f"}"#)
                .unwrap()
                .ends_with("v2")
        );
        call(&mut s, "draft_open", r#"{"name":"drop"}"#).unwrap();
        call(&mut s, "write", r#"{"path":"f","content":"v3"}"#).unwrap();
        call(&mut s, "draft_discard", "{}").unwrap();
        assert!(
            call(&mut s, "read", r#"{"path":"f"}"#)
                .unwrap()
                .ends_with("v2")
        );
        assert!(call(&mut s, "drafts", "{}").unwrap().lines().count() == 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn long_reads_are_cut_and_say_where_to_go_on() {
        let (dir, mut s) = session("long");
        let big: String = (0..20_000).map(|i| format!("line {i}\n")).collect();
        call(
            &mut s,
            "write",
            &format!(r#"{{"path":"big","content":{}}}"#, json::quote(&big)),
        )
        .unwrap();
        let r = call(&mut s, "read", r#"{"path":"big"}"#).unwrap();
        assert!(
            r.len() <= MAX_TEXT && r.contains("read on with start_line="),
            "{}",
            r.len()
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
