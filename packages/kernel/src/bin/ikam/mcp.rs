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
use std::ops::DerefMut;

use ikam_kernel::merge::Merged;
use ikam_kernel::{Id, Kind};

use crate::corpus::{self, err};
use crate::json::{self, Json};
use crate::live::{self, Live, Op};
use crate::text;

/// Characters of text a tool returns at most.
pub const MAX_TEXT: usize = 64 * 1024;

const VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// The tools over one `Live`: owned by `ikam mcp`, or a lock on the one
/// `ikam live` and `host` serve (E360), so the same handlers answer both.
pub struct Session<L: DerefMut<Target = Live> = Box<Live>> {
    pub live: L,
    /// E407 (`--skills [PATH]`, off): serve skills; `Some(None)` is the
    /// built-in skill only, `Some(Some(PATH))` adds each skill folder under
    /// PATH at the session's ref head.
    pub skills: Option<Option<String>>,
}

/// E388 (off): `--rank-files` builds the file ranker at start rather than
/// on the first `rank`; `--file-vectors PREFIX` loads vectors for files.
#[derive(Default)]
pub struct RankFlags {
    pub rank_files: bool,
    pub file_vectors: Option<String>,
}

impl RankFlags {
    /// Take the flags out of `args`.
    pub fn take(args: &mut Vec<String>) -> Result<RankFlags, String> {
        let before = args.len();
        args.retain(|a| a != "--rank-files");
        let rank_files = args.len() != before;
        let file_vectors = match args.iter().position(|a| a == "--file-vectors") {
            Some(i) if i + 1 < args.len() => {
                let v = args.remove(i + 1);
                args.remove(i);
                Some(v)
            }
            Some(_) => return Err("--file-vectors PREFIX".into()),
            None => None,
        };
        Ok(RankFlags {
            rank_files,
            file_vectors,
        })
    }

    pub fn apply(&self, live: &mut Live) -> Result<(), String> {
        if self.rank_files {
            live.index_files();
        }
        if let Some(p) = &self.file_vectors {
            live.load_file_vectors(p)?;
        }
        Ok(())
    }
}

/// Serve one client on `input`/`output` until `input` closes.
pub fn run(
    store_dir: &str,
    refname: &str,
    passage_index: bool,
    rank: &RankFlags,
    skills: Option<Option<String>>,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<(), String> {
    let mut s = Session::open(store_dir, refname)?;
    s.skills = skills;
    if passage_index {
        s.live.index_passages();
    }
    rank.apply(&mut s.live)?;
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
            say!("{msg}: committed as {c}");
        }
    }
    Ok(())
}

impl Session {
    pub fn open(store_dir: &str, refname: &str) -> Result<Session, String> {
        let mut live = Live::open(store_dir, refname)?;
        live.index_all(false)?;
        Ok(Session {
            live: Box::new(live),
            skills: None,
        })
    }
}

/// Tools that change nothing, which `ikam live` serves on GET (E360).
/// E360: how long edits made through the tools wait for their `commit`
/// before a served `Live`'s timed commit takes them.
pub const HOLD: std::time::Duration = std::time::Duration::from_secs(600);

pub const READ_TOOLS: &[&str] = &[
    "list", "read", "search", "passages", "rank", "status", "history", "diff", "drafts",
];

/// The tool list with its schemas, as `tools/list` gives it.
pub fn tool_list() -> Json {
    Json::Arr(tools())
}

/// `text` cut to `MAX_TEXT`, as a tool result is.
pub fn capped(text: String) -> String {
    cap(text)
}

impl<L: DerefMut<Target = Live>> Session<L> {
    /// The reply to one JSON-RPC message, or `None` for a notification.
    ///
    /// E406: two protocol eras over the same handlers. A request whose
    /// `params._meta` names `io.modelcontextprotocol/protocolVersion` is a
    /// 2026-07-28 request (stateless: no handshake, the client's version and
    /// capabilities on every request) and gets that era's answer; any other
    /// request is answered as before, byte for byte.
    pub fn handle(&mut self, line: &str) -> Option<Json> {
        let msg = match json::parse(line) {
            Ok(m) => m,
            Err(e) => return Some(error(Json::Null, -32700, &format!("parse error: {e}"))),
        };
        let id = msg.get("id").cloned()?;
        let method = msg.get("method").and_then(Json::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Json::Obj(Vec::new()));
        match modern_request(method, &params) {
            Ok(false) => {}
            Ok(true) => return Some(self.handle_modern(id, method, &params)),
            Err((code, message, data)) => return Some(error_data(id, code, &message, data)),
        }
        let result = match method {
            "initialize" => initialize(&params, self.skills.is_some()),
            "ping" => Json::Obj(Vec::new()),
            "tools/list" => obj(vec![("tools", Json::Arr(tools()))]),
            "tools/call" => self.tools_call(&params),
            _ => match self.skills_method(method, &params) {
                Some(Ok(mut r)) => {
                    // The extension's own results carry its cache fields
                    // in either era.
                    if matches!(
                        method,
                        "skills/list" | "skills/get" | "resources/directory/read"
                    ) && let Json::Obj(f) = &mut r
                    {
                        f.push(("resultType".into(), s("complete")));
                        f.push(("ttlMs".into(), Json::Num(0.0)));
                        f.push(("cacheScope".into(), s("private")));
                    }
                    r
                }
                Some(Err((code, m))) => return Some(error(id, code, &m)),
                None => return Some(error(id, -32601, &format!("method not found: {method}"))),
            },
        };
        Some(obj(vec![
            ("jsonrpc", s("2.0")),
            ("id", id),
            ("result", result),
        ]))
    }

    /// A 2026-07-28 request, already checked by `modern_request`.
    fn handle_modern(&mut self, id: Json, method: &str, params: &Json) -> Json {
        let (mut result, ttl) = match method {
            "server/discover" => (
                obj(discover_fields(self.skills.is_some())),
                Some(LIST_TTL_MS),
            ),
            "tools/list" => (obj(vec![("tools", Json::Arr(tools()))]), Some(LIST_TTL_MS)),
            "tools/call" => (self.tools_call(params), None),
            // `initialize` and `ping` are gone from this era; every other
            // method is unknown to both.
            _ => match self.skills_method(method, params) {
                // Skills change with a commit: cacheable, but stale at once.
                Some(Ok(r)) => (r, Some(0)),
                Some(Err((code, m))) => return error(id, code, &m),
                None => return error(id, -32601, &format!("method not found: {method}")),
            },
        };
        modern_result(&mut result, ttl);
        obj(vec![("jsonrpc", s("2.0")), ("id", id), ("result", result)])
    }

    /// `tools/call`'s result, the same in both eras.
    fn tools_call(&mut self, params: &Json) -> Json {
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

    /// Run tool `name`; `Err` is a tool error the agent reads, not a protocol one.
    pub fn call(&mut self, name: &str, a: &Json) -> Result<String, String> {
        match name {
            "list" => self.list(&opt(a, "path").unwrap_or_default(), opt(a, "at").as_deref()),
            "read" => self.read(
                &req(a, "path")?,
                opt(a, "at").as_deref(),
                num(a, "start_line"),
                num(a, "end_line"),
                num(a, "start_char"),
            ),
            "search" => Ok(self.search(&req(a, "query")?, num(a, "limit").unwrap_or(50))),
            "passages" => Ok(self.passages(&req(a, "question")?, num(a, "k").unwrap_or(8))),
            "rank" => self.rank(&req(a, "query")?, a),
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
        start_char: Option<usize>,
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
        let skip = start_char.unwrap_or(1).max(1) - 1;
        let room = MAX_TEXT - 200;
        for (i, line) in lines[first - 1..last].iter().enumerate() {
            // `start_char` applies to the first line shown.
            let line: &str = if i == 0 && skip > 0 {
                line.char_indices()
                    .nth(skip)
                    .map_or("", |(b, _)| &line[b..])
            } else {
                line
            };
            if out.len() + line.len() > room {
                if shown >= first {
                    break;
                }
                // One line longer than an answer: show its start and say
                // where it goes on, so the rest stays reachable.
                let mut cut = room.min(line.len());
                while !line.is_char_boundary(cut) {
                    cut -= 1;
                }
                let next = skip + line[..cut].chars().count() + 1;
                return Ok(format!(
                    "{head}{path}: line {first} of {} (from character {})\n{}\n(line {first} is cut; read on with start_line={first} and start_char={next})",
                    lines.len(),
                    skip + 1,
                    &line[..cut]
                ));
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
        let found = self.live.retrieve(question, k);
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

    /// E388: files ranked for `query`, each with its best passages.
    fn rank(&mut self, query: &str, a: &Json) -> Result<String, String> {
        let k = num(a, "k").unwrap_or(10).clamp(1, 1000);
        let per = num(a, "passages").unwrap_or(2).min(20);
        let vector: Option<Vec<f32>> = match a.get("vector") {
            None | Some(Json::Null) => None,
            Some(Json::Arr(v)) => Some(
                v.iter()
                    .map(|x| match x {
                        Json::Num(n) => Ok(*n as f32),
                        _ => Err("vector holds numbers only".to_string()),
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Some(_) => return Err("vector is an array of numbers".into()),
        };
        let fuse = match opt(a, "fuse").as_deref() {
            Some("rrf") => live::Fuse::Rrf,
            Some("convex") | None => match a.get("alpha") {
                None => live::Fuse::Convex(ikam_kernel::fusion::DEFAULT_ALPHA),
                Some(Json::Num(x)) if (0.0..=1.0).contains(x) => live::Fuse::Convex(*x as f32),
                Some(_) => return Err("alpha is a number from 0 to 1".into()),
            },
            Some(_) => return Err("fuse is convex or rrf".into()),
        };
        let ranked = self.live.rank_files(query, vector.as_deref(), k, fuse)?;
        let hybrid = vector.is_some() && self.live.ranker.as_ref().is_some_and(|r| r.dim > 0);
        if ranked.is_empty() {
            return Ok("no file shares a word with the query".into());
        }
        let mut out = format!(
            "{} file(s), best first ({})\n",
            ranked.len(),
            if hybrid {
                "keywords and vectors"
            } else {
                "keywords"
            }
        );
        for (i, (path, score)) in ranked.iter().enumerate() {
            out.push_str(&format!("{}. {path} (score {score:.6})\n", i + 1));
            for p in self.live.passages_in(path, query, per) {
                let part = if p.part.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", p.part)
                };
                out.push_str(&format!("   line {}{part}:\n", p.first_line));
                for line in p.text.lines().take(6) {
                    out.push_str(&format!("     {line}\n"));
                }
            }
        }
        Ok(out)
    }

    fn apply(&mut self, op: Op, done: &str) -> Result<String, String> {
        if let Op::Put(_, b) = &op
            && !self.live.has_room(b.len())
        {
            return Err("the store is full (--max-store)".into());
        }
        let cost = self.live.apply(vec![op])?;
        // E360: on a served `Live`, the timed commit leaves these edits
        // for the agent's own `commit` (and message), for up to HOLD.
        self.live.hold_until = Some(std::time::Instant::now() + HOLD);
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
        let mut bytes_at = |root: Option<Id>| -> Result<Vec<u8>, String> {
            let Some(root) = root else {
                return Ok(Vec::new());
            };
            match self.stat_in(root, &path)? {
                Some((Kind::File, id)) => Ok(self.live.read(&id)?.to_vec()),
                _ => Ok(Vec::new()),
            }
        };
        let (xb, yb) = (bytes_at(a)?, bytes_at(Some(b))?);
        if xb == yb {
            return Ok(format!("{path}: no changes"));
        }
        // Compare bytes, not lossy text: two different non-UTF-8 files can
        // decode to the same replacement characters.
        let (Ok(x), Ok(y)) = (std::str::from_utf8(&xb), std::str::from_utf8(&yb)) else {
            return Ok(format!(
                "{path}: binary file changed ({} bytes to {} bytes)",
                xb.len(),
                yb.len()
            ));
        };
        let d = similar::TextDiff::from_lines(x, y);
        Ok(d.unified_diff()
            .context_radius(3)
            .header(&format!("a/{path}"), &format!("b/{path}"))
            .to_string())
    }

    fn drafts(&mut self) -> Result<String, String> {
        let list = self.live.repo.drafts().map_err(err)?;
        let mut out = format!(
            "session on {} (target {})\n",
            self.live.refname, self.live.home
        );
        for (name, head) in list {
            out.push_str(&format!("{name} at {head}\n"));
        }
        Ok(out)
    }

    fn draft_open(&mut self, name: &str) -> Result<String, String> {
        if self.live.refname != self.live.home {
            return Err(format!(
                "already on {}; promote or discard it first",
                self.live.refname
            ));
        }
        if self.live.pending() > 0 {
            return Err("commit pending edits first".into());
        }
        let home = self.live.home.clone();
        let draft = self.live.repo.draft(&home, name).map_err(err)?;
        self.live.switch_ref(&draft)?;
        Ok(format!(
            "on {draft}; edits and commits stay off {} until draft_promote",
            self.live.home
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
        let target = self.live.home.clone();
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
        self.live.switch_ref(&target)?;
        Ok(out)
    }

    fn draft_discard(&mut self) -> Result<String, String> {
        let draft = self.on_draft()?;
        self.live.repo.discard_draft(&draft).map_err(err)?;
        let target = self.live.home.clone();
        self.live.switch_ref_dropping(&target)?;
        Ok(format!("discarded {draft}; back on {target}"))
    }
}

/// E406: the per-request envelope's protocol versions; the handshake's are
/// `VERSIONS`.
pub const MODERN_VERSIONS: &[&str] = &["2026-07-28"];

const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// E406: how long a client may keep `server/discover` and `tools/list`:
/// both are fixed for the life of this binary.
pub const LIST_TTL_MS: u64 = 3_600_000;

const INSTRUCTIONS: &str = "A versioned file store. Edits wait in the working tree until commit, which takes your message. \
               Revisions (at, from, to) are a commit id or prefix, a ref, or work (the working tree, the default).";

/// E407: with `--skills`, the instructions point to the built-in skill.
const SKILL_INSTRUCTIONS: &str = "A versioned file store: edits wait until commit, which takes your message.\n\
               How to use it: skill://ikam-store/SKILL.md (a skill, or read it as a resource).";

/// Whether a request is in the 2026-07-28 envelope (`true`), the handshake
/// era (`false`), or a malformed or unsupported modern request (`Err`: code,
/// message, data). Only the version key marks a request as modern, so a
/// handshake-era client's own `_meta` (a progress token) changes nothing.
fn modern_request(method: &str, params: &Json) -> Result<bool, (i64, String, Option<Json>)> {
    let meta = params.get("_meta");
    let version = meta.and_then(|m| m.get(PROTOCOL_VERSION));
    let version = match version {
        None if method == "server/discover" => {
            return Err((
                -32602,
                format!("server/discover needs _meta.{PROTOCOL_VERSION}"),
                None,
            ));
        }
        None => return Ok(false),
        Some(v) => v.as_str().unwrap_or(""),
    };
    if !MODERN_VERSIONS.contains(&version) {
        let supported = VERSIONS
            .iter()
            .chain(MODERN_VERSIONS)
            .map(|v| s(v))
            .collect();
        let data = obj(vec![
            ("requested", s(version)),
            ("supported", Json::Arr(supported)),
        ]);
        return Err((
            -32022,
            format!("unsupported protocol version: {version}"),
            Some(data),
        ));
    }
    match meta.and_then(|m| m.get(CLIENT_CAPABILITIES)) {
        Some(Json::Obj(_)) => Ok(true),
        _ => Err((-32602, format!("missing _meta.{CLIENT_CAPABILITIES}"), None)),
    }
}

fn capabilities(skills: bool) -> Json {
    let mut c = vec![("tools", obj(vec![("listChanged", Json::Bool(false))]))];
    if skills {
        // E407: skills ride on resources; the extension is declared in
        // `extensions`, which is listed only when an extension is on.
        c.push(("resources", Json::Obj(Vec::new())));
        c.push((
            "extensions",
            obj(vec![(
                SKILLS_EXT,
                obj(vec![("directoryRead", Json::Bool(true))]),
            )]),
        ));
    }
    obj(c)
}

fn instructions(skills: bool) -> &'static str {
    if skills {
        SKILL_INSTRUCTIONS
    } else {
        INSTRUCTIONS
    }
}

fn server_info() -> Json {
    obj(vec![
        ("name", s("ikam")),
        ("version", s(env!("CARGO_PKG_VERSION"))),
    ])
}

/// `server/discover`'s fields, before `modern_result` adds the era's own.
fn discover_fields(skills: bool) -> Vec<(&'static str, Json)> {
    let versions = VERSIONS
        .iter()
        .chain(MODERN_VERSIONS)
        .map(|v| s(v))
        .collect();
    vec![
        ("supportedVersions", Json::Arr(versions)),
        ("capabilities", capabilities(skills)),
        ("instructions", s(instructions(skills))),
    ]
}

/// What every 2026-07-28 result carries: `resultType`, the server's info in
/// `_meta`, and for a cacheable result a time to keep it (`private`:
/// a store's server is one person's).
fn modern_result(result: &mut Json, ttl_ms: Option<u64>) {
    if let Json::Obj(f) = result {
        f.push(("resultType".into(), s("complete")));
        if let Some(ttl) = ttl_ms {
            f.push(("ttlMs".into(), Json::Num(ttl as f64)));
            f.push(("cacheScope".into(), s("private")));
        }
        match f.iter_mut().find(|(k, _)| k == "_meta") {
            Some((_, Json::Obj(m))) => m.push((SERVER_INFO.into(), server_info())),
            _ => f.push(("_meta".into(), obj(vec![(SERVER_INFO, server_info())]))),
        }
    }
}

fn initialize(params: &Json, skills: bool) -> Json {
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
        ("capabilities", capabilities(skills)),
        ("serverInfo", server_info()),
        ("instructions", s(instructions(skills))),
    ])
}

// ---- E407: skills (`--skills [PATH]`) --------------------------------------

const SKILLS_EXT: &str = "io.modelcontextprotocol/skills";

/// Skills a `skills/list` page holds.
pub const SKILLS_PAGE: usize = 50;
/// The extension's limits on one skill: refused above them, never cut.
pub const SKILL_MAX_FILES: usize = 512;
pub const SKILL_MAX_BYTES: u64 = 16 << 20;

const BUILTIN: &str = "ikam-store";
const BUILTIN_SKILL_MD: &[u8] = include_bytes!("skills/ikam-store/SKILL.md");

/// `--skills [PATH]` out of `args`: `None` when absent; a following
/// argument that is not a flag is PATH.
pub fn take_skills(args: &mut Vec<String>) -> Option<Option<String>> {
    let i = args.iter().position(|a| a == "--skills")?;
    args.remove(i);
    if i < args.len() && !args[i].starts_with("--") {
        Some(Some(args.remove(i)))
    } else {
        Some(None)
    }
}

/// One skill: its folder's files, `SKILL.md` first, the rest by path.
struct Skill {
    name: String,
    description: String,
    /// The frontmatter's other top-level keys with a value on their line,
    /// passed through as the extension asks (`true`/`false` as booleans,
    /// the rest as strings). Keys whose value is nested are left out.
    other: Vec<(String, Json)>,
    files: Vec<(String, Vec<u8>)>,
}

type Front = (String, String, Vec<(String, Json)>);

/// `name`, `description` and the other top-level keys from a `SKILL.md`'s
/// frontmatter.
fn frontmatter(md: &[u8]) -> Result<Front, String> {
    let text = std::str::from_utf8(md).map_err(|_| "SKILL.md is not UTF-8".to_string())?;
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err("SKILL.md has no frontmatter".into());
    }
    let (mut name, mut description, mut other) = (None, None, Vec::new());
    for l in lines {
        if l.trim() == "---" {
            break;
        }
        // Top-level keys only: an indented or list line belongs to a key
        // above it.
        if l.starts_with([' ', '\t', '-', '#']) {
            continue;
        }
        if let Some((k, v)) = l.split_once(':') {
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            match k.trim() {
                "name" => name = Some(v),
                "description" => description = Some(v),
                _ if v.is_empty() => {}
                k if !other.iter().any(|(o, _)| o == k) => other.push((
                    k.to_string(),
                    match v.as_str() {
                        "true" => Json::Bool(true),
                        "false" => Json::Bool(false),
                        _ => s(&v),
                    },
                )),
                _ => {}
            }
        }
    }
    match (name, description) {
        (Some(n), Some(d)) if !n.is_empty() && !d.is_empty() => Ok((n, d, other)),
        _ => Err("SKILL.md frontmatter needs name and description".into()),
    }
}

/// A skill entry as the extension defines it: the `SKILL.md` uri, the
/// frontmatter, and every file once (`SKILL.md` first) with its digest and
/// size.
fn manifest(k: &Skill) -> Json {
    let resources = k
        .files
        .iter()
        .map(|(path, bytes)| {
            obj(vec![
                ("uri", s(&format!("skill://{}/{path}", k.name))),
                ("digest", s(&format!("sha256:{}", hex(&sha256(bytes))))),
                ("size", Json::Num(bytes.len() as f64)),
            ])
        })
        .collect();
    let mut front = vec![
        ("name".to_string(), s(&k.name)),
        ("description".to_string(), s(&k.description)),
    ];
    front.extend(k.other.iter().cloned());
    obj(vec![
        ("uri", s(&format!("skill://{}/SKILL.md", k.name))),
        ("frontmatter", Json::Obj(front)),
        ("resources", Json::Arr(resources)),
    ])
}

/// `skill://NAME/REST` as (NAME, REST); REST may be empty.
fn skill_uri(uri: &str) -> Result<(&str, &str), (i64, String)> {
    let rest = uri
        .strip_prefix("skill://")
        .ok_or_else(|| (-32602, format!("not a skill:// uri: {uri}")))?;
    let (name, path) = rest.split_once('/').unwrap_or((rest, ""));
    if name.is_empty() || path.split('/').any(|p| p == "..") {
        return Err((-32602, format!("bad skill uri: {uri}")));
    }
    Ok((name, path.trim_end_matches('/')))
}

fn mime(path: &str, text: bool) -> &'static str {
    match (path.rsplit('.').next(), text) {
        (Some("md"), true) => "text/markdown",
        (_, true) => "text/plain",
        _ => "application/octet-stream",
    }
}

impl<L: DerefMut<Target = Live>> Session<L> {
    /// A skills or resources method's result, `None` when skills are off or
    /// the method is not one of theirs.
    #[allow(clippy::type_complexity)]
    fn skills_method(
        &mut self,
        method: &str,
        params: &Json,
    ) -> Option<Result<Json, (i64, String)>> {
        self.skills.as_ref()?;
        let uri = || {
            params
                .get("uri")
                .and_then(Json::as_str)
                .map(String::from)
                .ok_or_else(|| (-32602, "missing uri".to_string()))
        };
        Some(match method {
            "skills/list" => self.skills_list(params.get("cursor").and_then(Json::as_str)),
            "skills/get" => uri().and_then(|u| {
                let (name, _) = skill_uri(&u)?;
                Ok(obj(vec![("skill", manifest(&self.skill(name)?))]))
            }),
            "resources/list" => self.skills_resources(),
            "resources/read" => uri().and_then(|u| self.skill_read(&u)),
            "resources/directory/read" => uri().and_then(|u| self.skill_dir(&u)),
            _ => return None,
        })
    }

    /// Every skill: the built-in first, then the store's by name, and the
    /// folders refused with their reasons.
    fn all_skills(&mut self) -> (Vec<Skill>, Vec<(String, String)>) {
        let (name, description, other) = frontmatter(BUILTIN_SKILL_MD).expect("built-in skill");
        let mut skills = vec![Skill {
            name,
            description,
            other,
            files: vec![("SKILL.md".into(), BUILTIN_SKILL_MD.to_vec())],
        }];
        let mut refused = Vec::new();
        if let Some(Some(root)) = self.skills.clone() {
            match self.store_skills(&root) {
                Ok(found) => {
                    for (folder, k) in found {
                        match k {
                            Ok(k) if k.name == BUILTIN => {
                                refused.push((folder, "the built-in skill has this name".into()))
                            }
                            Ok(k) => skills.push(k),
                            Err(why) => refused.push((folder, why)),
                        }
                    }
                }
                Err(why) => refused.push((root, why)),
            }
        }
        for (folder, why) in &refused {
            say!("skills: {folder} not listed: {why}");
        }
        (skills, refused)
    }

    /// The skill folders under `root` at the session's ref head (never
    /// pending edits): each folder with a `SKILL.md`, read whole or refused.
    #[allow(clippy::type_complexity)]
    fn store_skills(&mut self, root: &str) -> Result<Vec<(String, Result<Skill, String>)>, String> {
        let head = self.live.refname.clone();
        let Ok(tree) = self.tree_at(Some(&head)) else {
            return Ok(Vec::new()); // no commit on the ref yet
        };
        let root = live::norm(root);
        let Some((Kind::Tree, dir)) = self.stat_in(tree, &root)? else {
            return Err(format!("{root}: not a folder at the head of {head}"));
        };
        let mut out = Vec::new();
        for e in self.live.entries(&dir)?.iter() {
            if e.kind != Kind::Tree {
                continue;
            }
            let has_md = self
                .live
                .entries(&e.id)?
                .iter()
                .any(|f| f.name == "SKILL.md" && f.kind == Kind::File);
            if !has_md {
                continue;
            }
            let folder = format!("{root}/{}", e.name)
                .trim_start_matches('/')
                .to_string();
            out.push((folder, self.read_skill(&e.name, e.id)));
        }
        Ok(out)
    }

    fn read_skill(&mut self, folder: &str, dir: Id) -> Result<Skill, String> {
        // Count and size every file before reading any, so a refused skill
        // costs only its listing.
        let mut paths = Vec::new();
        let mut stack = vec![(String::new(), dir)];
        let mut total = 0u64;
        while let Some((prefix, id)) = stack.pop() {
            for e in self.live.entries(&id)?.iter() {
                let path = format!("{prefix}{}", e.name);
                match e.kind {
                    Kind::Tree => stack.push((format!("{path}/"), e.id)),
                    Kind::File => {
                        total += self.live.size(&e.id)?;
                        paths.push((path, e.id));
                    }
                    Kind::Graph => {}
                }
            }
            if paths.len() > SKILL_MAX_FILES {
                return Err(format!(
                    "more than {SKILL_MAX_FILES} files (the extension's limit)"
                ));
            }
            if total > SKILL_MAX_BYTES {
                return Err(format!(
                    "{total} bytes or more, over {SKILL_MAX_BYTES} (the extension's limit)"
                ));
            }
        }
        paths.sort_by(|a, b| (a.0 != "SKILL.md", &a.0).cmp(&(b.0 != "SKILL.md", &b.0)));
        let mut files = Vec::with_capacity(paths.len());
        for (path, id) in paths {
            files.push((path, self.live.read(&id)?.to_vec()));
        }
        let (name, description, other) = frontmatter(&files[0].1)?;
        if name != folder {
            return Err(format!("its SKILL.md names it {name}, not {folder}"));
        }
        Ok(Skill {
            name,
            description,
            other,
            files,
        })
    }

    fn skill(&mut self, name: &str) -> Result<Skill, (i64, String)> {
        let (skills, refused) = self.all_skills();
        if let Some(k) = skills.into_iter().find(|k| k.name == name) {
            return Ok(k);
        }
        match refused
            .iter()
            .find(|(f, _)| f.rsplit('/').next() == Some(name))
        {
            Some((f, why)) => Err((-32602, format!("skill {name} refused ({f}): {why}"))),
            None => Err((-32602, format!("no skill {name}"))),
        }
    }

    fn skills_list(&mut self, cursor: Option<&str>) -> Result<Json, (i64, String)> {
        let start = match cursor {
            None => 0,
            Some(c) => c
                .parse::<usize>()
                .map_err(|_| (-32602, format!("bad cursor: {c}")))?,
        };
        let (skills, refused) = self.all_skills();
        let page: Vec<Json> = skills
            .iter()
            .skip(start)
            .take(SKILLS_PAGE)
            .map(manifest)
            .collect();
        let mut f = vec![("skills", Json::Arr(page))];
        if start + SKILLS_PAGE < skills.len() {
            f.push(("nextCursor", s(&(start + SKILLS_PAGE).to_string())));
        }
        if !refused.is_empty() {
            let r = refused
                .iter()
                .map(|(folder, why)| obj(vec![("folder", s(folder)), ("reason", s(why))]))
                .collect();
            f.push(("_meta", obj(vec![("ikam/refused", Json::Arr(r))])));
        }
        Ok(obj(f))
    }

    /// Each skill's `SKILL.md` as a resource, for a client without skills.
    fn skills_resources(&mut self) -> Result<Json, (i64, String)> {
        let (skills, _) = self.all_skills();
        let r = skills
            .iter()
            .map(|k| {
                obj(vec![
                    ("uri", s(&format!("skill://{}/SKILL.md", k.name))),
                    ("name", s(&k.name)),
                    ("description", s(&k.description)),
                    ("mimeType", s("text/markdown")),
                ])
            })
            .collect();
        Ok(obj(vec![("resources", Json::Arr(r))]))
    }

    fn skill_read(&mut self, uri: &str) -> Result<Json, (i64, String)> {
        let (name, path) = skill_uri(uri)?;
        let k = self.skill(name)?;
        let path = if path.is_empty() { "SKILL.md" } else { path };
        let Some((_, bytes)) = k.files.iter().find(|(p, _)| p == path) else {
            return Err((-32602, format!("no file {path} in skill {name}")));
        };
        let uri = format!("skill://{name}/{path}");
        let content = match std::str::from_utf8(bytes) {
            Ok(text) => obj(vec![
                ("uri", s(&uri)),
                ("mimeType", s(mime(path, true))),
                ("text", s(text)),
            ]),
            Err(_) => obj(vec![
                ("uri", s(&uri)),
                ("mimeType", s(mime(path, false))),
                ("blob", s(&base64(bytes))),
            ]),
        };
        Ok(obj(vec![("contents", Json::Arr(vec![content]))]))
    }

    /// A skill folder's direct children as the extension's `resources`:
    /// folders as `inode/directory` (no trailing `/`), files with their
    /// type.
    fn skill_dir(&mut self, uri: &str) -> Result<Json, (i64, String)> {
        let (name, dir) = skill_uri(uri)?;
        let k = self.skill(name)?;
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut entries: Vec<Json> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (path, bytes) in &k.files {
            let Some(rest) = path.strip_prefix(&prefix) else {
                continue;
            };
            match rest.split_once('/') {
                Some((sub, _)) => {
                    if seen.insert(sub.to_string()) {
                        entries.push(obj(vec![
                            ("uri", s(&format!("skill://{name}/{prefix}{sub}"))),
                            ("name", s(sub)),
                            ("mimeType", s("inode/directory")),
                        ]));
                    }
                }
                None => entries.push(obj(vec![
                    ("uri", s(&format!("skill://{name}/{path}"))),
                    ("name", s(rest)),
                    (
                        "mimeType",
                        s(mime(path, std::str::from_utf8(bytes).is_ok())),
                    ),
                ])),
            }
        }
        if entries.is_empty() {
            return Err((-32602, format!("no folder {dir} in skill {name}")));
        }
        Ok(obj(vec![("resources", Json::Arr(entries))]))
    }
}

/// SHA-256 (FIPS 180-4), for the extension's `sha256:` digests. A short
/// local implementation rather than a crate: the kernel pins its
/// dependencies, and digests here cover at most 16 MiB a skill.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Standard base64 with padding, for a binary skill file's `blob`.
fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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
                (
                    "start_char",
                    "integer",
                    "first character of start_line (for a line longer than one answer)",
                ),
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
            "rank",
            "Files ranked by how well their words match a query (BM25 over each whole file), each with its best passages. With vector (the query's embedding) and file vectors loaded, keyword and vector ranks are fused.",
            &[
                ("query", "string", "question or keywords"),
                ("k", "integer", "files (10)"),
                ("passages", "integer", "passages a file (2)"),
                (
                    "vector",
                    "array",
                    "query vector, as many numbers as the file vectors",
                ),
                ("fuse", "string", "convex (default) or rrf"),
                ("alpha", "number", "convex weight on keywords, 0 to 1 (0.5)"),
            ],
            &["query"],
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
                if *t == "array" {
                    obj(vec![
                        ("type", s(t)),
                        ("items", obj(vec![("type", s("number"))])),
                        ("description", s(d)),
                    ])
                } else {
                    obj(vec![("type", s(t)), ("description", s(d))])
                },
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
    error_data(id, code, message, None)
}

fn error_data(id: Json, code: i64, message: &str, data: Option<Json>) -> Json {
    let mut e = vec![("code", Json::Num(code as f64)), ("message", s(message))];
    if let Some(d) = data {
        e.push(("data", d));
    }
    obj(vec![("jsonrpc", s("2.0")), ("id", id), ("error", obj(e))])
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
    match a.get(k) {
        Some(Json::Bool(b)) => *b,
        Some(Json::Str(t)) => matches!(t.as_str(), "true" | "1"),
        _ => false,
    }
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
            17
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
    fn two_eras_over_one_session() {
        let (dir, mut ss) = session("eras");
        let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;
        let legacy_list = ss
            .handle(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
            .unwrap();
        let d = ss
            .handle(&format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"server/discover","params":{{{meta}}}}}"#
            ))
            .unwrap();
        let d = d.get("result").unwrap();
        assert_eq!(d.get("resultType").unwrap().as_str(), Some("complete"));
        assert!(
            d.get("supportedVersions")
                .unwrap()
                .as_arr()
                .contains(&s("2026-07-28"))
        );
        assert!(d.get("_meta").unwrap().get(SERVER_INFO).is_some());
        // The same tools in both eras; the modern list adds only its own fields.
        let m = ss
            .handle(&format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{{meta}}}}}"#
            ))
            .unwrap();
        let mut m = m;
        if let Json::Obj(f) = &mut m
            && let Some((_, Json::Obj(r))) = f.iter_mut().find(|(k, _)| k == "result")
        {
            r.retain(|(k, _)| {
                !["resultType", "ttlMs", "cacheScope", "_meta"].contains(&k.as_str())
            });
        }
        assert_eq!(m.encode(), legacy_list.encode());
        let w = ss.handle(&format!(r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"write","arguments":{{"path":"a.txt","content":"x"}},{meta}}}}}"#)).unwrap();
        assert_eq!(
            w.get("result").unwrap().get("isError"),
            Some(&Json::Bool(false))
        );
        // A handshake-era read sees the modern write: one store, one session.
        let r = ss.handle(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"read","arguments":{"path":"a.txt"}}}"#).unwrap();
        assert!(r.encode().contains(r"lines 1-1 of 1\nx"), "{}", r.encode());
        assert!(r.get("result").unwrap().get("resultType").is_none());
        // Errors: an unknown version names the supported ones; capabilities
        // are required; ping and initialize are not in the modern era.
        let e = ss.handle(r#"{"jsonrpc":"2.0","id":5,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2099-01-01","io.modelcontextprotocol/clientCapabilities":{}}}}"#).unwrap();
        let e = e.get("error").unwrap();
        assert_eq!(e.get("code"), Some(&Json::Num(-32022.0)));
        assert!(
            e.get("data")
                .unwrap()
                .get("supported")
                .unwrap()
                .as_arr()
                .contains(&s("2025-11-25"))
        );
        let e = ss.handle(r#"{"jsonrpc":"2.0","id":6,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#).unwrap();
        assert_eq!(
            e.get("error").unwrap().get("code"),
            Some(&Json::Num(-32602.0))
        );
        for m in ["ping", "initialize"] {
            let e = ss
                .handle(&format!(
                    r#"{{"jsonrpc":"2.0","id":7,"method":"{m}","params":{{{meta}}}}}"#
                ))
                .unwrap();
            assert_eq!(
                e.get("error").unwrap().get("code"),
                Some(&Json::Num(-32601.0))
            );
        }
        // A handshake-era client's own _meta leaves its answer as it was.
        let p = ss
            .handle(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"progressToken":1}}}"#)
            .unwrap();
        assert_eq!(p.encode(), legacy_list.encode());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sha256_and_base64_match_known_vectors() {
        let h = |d: &[u8]| hex(&sha256(d));
        assert_eq!(
            h(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            h(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            h(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            h(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        // Lengths at the padding edges (55, 56, 63, 64 bytes) against each
        // other's prefix property is not a check; compare with known digests.
        assert_eq!(
            h(&[b'a'; 64]),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    fn rpc(s: &mut Session, id: u32, method: &str, params: &str) -> Json {
        s.handle(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#
        ))
        .unwrap()
    }

    fn result(r: &Json) -> &Json {
        r.get("result").unwrap_or_else(|| panic!("{}", r.encode()))
    }

    /// name -> (digest, size) for every file of every listed skill, after
    /// checking each against the bytes `resources/read` serves.
    fn listed(s: &mut Session) -> Vec<(String, String, f64)> {
        let mut out = Vec::new();
        let r = rpc(s, 1, "skills/list", "{}");
        // The extension's cache fields, in this (handshake) era too.
        for f in ["resultType", "ttlMs", "cacheScope"] {
            assert!(result(&r).get(f).is_some(), "{f}");
        }
        for k in result(&r).get("skills").unwrap().as_arr() {
            // The entry's shape: uri, frontmatter and resources, nothing
            // named at the top.
            assert!(k.get("name").is_none() && k.get("files").is_none());
            let front = k.get("frontmatter").unwrap();
            let name = front.get("name").unwrap().as_str().unwrap();
            assert!(front.get("description").is_some());
            assert_eq!(
                k.get("uri").unwrap().as_str().unwrap(),
                format!("skill://{name}/SKILL.md")
            );
            let res = k.get("resources").unwrap().as_arr();
            assert_eq!(res[0].get("uri"), k.get("uri"), "SKILL.md first");
            for f in res {
                let uri = f.get("uri").unwrap().as_str().unwrap().to_string();
                let read = rpc(s, 2, "resources/read", &format!(r#"{{"uri":"{uri}"}}"#));
                let c = &result(&read).get("contents").unwrap().as_arr()[0];
                let bytes = c.get("text").unwrap().as_str().unwrap().as_bytes().to_vec();
                let digest = f.get("digest").unwrap().as_str().unwrap().to_string();
                assert_eq!(digest, format!("sha256:{}", hex(&sha256(&bytes))), "{uri}");
                let size = match f.get("size") {
                    Some(Json::Num(n)) => *n,
                    _ => panic!(),
                };
                assert_eq!(size, bytes.len() as f64);
                out.push((uri, digest, size));
            }
        }
        out
    }

    #[test]
    fn frontmatter_passes_other_top_level_keys_through() {
        let md = b"---\nname: a\ndescription: b\nlicense: MIT\ndisable-model-invocation: true\nmetadata:\n  owner: x\n- y\n---\nbody\n";
        let (n, d, other) = frontmatter(md).unwrap();
        assert_eq!((n.as_str(), d.as_str()), ("a", "b"));
        assert_eq!(
            other,
            [
                ("license".to_string(), s("MIT")),
                ("disable-model-invocation".to_string(), Json::Bool(true)),
            ]
        );
        let k = Skill {
            name: n,
            description: d,
            other,
            files: vec![("SKILL.md".into(), md.to_vec())],
        };
        let m = manifest(&k);
        let f = m.get("frontmatter").unwrap();
        assert_eq!(f.get("license").unwrap().as_str(), Some("MIT"));
        assert!(f.get("metadata").is_none() && f.get("owner").is_none());
    }

    #[test]
    fn skills_follow_commits_and_refuse_over_limits() {
        let (dir, mut s) = session("skills");
        // Off: the methods are unknown and the handshake is as before.
        let off_init = initialize(
            &json::parse(r#"{"protocolVersion":"2025-11-25"}"#).unwrap(),
            false,
        );
        assert!(rpc(&mut s, 1, "skills/list", "{}").get("error").is_some());
        s.skills = Some(Some("skills".into()));
        let on_init = initialize(
            &json::parse(r#"{"protocolVersion":"2025-11-25"}"#).unwrap(),
            true,
        );
        assert!(
            off_init
                .get("capabilities")
                .unwrap()
                .get("extensions")
                .is_none()
        );
        let ext = on_init
            .get("capabilities")
            .unwrap()
            .get("extensions")
            .unwrap();
        assert!(ext.get(SKILLS_EXT).is_some());
        // The built-in skill, before the store has any.
        let l = listed(&mut s);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].0, "skill://ikam-store/SKILL.md");
        // A skill written but not committed is not listed.
        call(&mut s, "write", r#"{"path":"skills/demo/SKILL.md","content":"---\nname: demo\ndescription: a demo\n---\nfirst\n"}"#).unwrap();
        call(
            &mut s,
            "write",
            r#"{"path":"skills/demo/ref/notes.txt","content":"notes\n"}"#,
        )
        .unwrap();
        assert_eq!(listed(&mut s).len(), 1);
        let c1 = call(&mut s, "commit", r#"{"message":"demo skill"}"#).unwrap();
        let c1 = c1.split_whitespace().nth(1).unwrap().to_string();
        let l = listed(&mut s);
        assert_eq!(
            l.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(),
            [
                "skill://ikam-store/SKILL.md",
                "skill://demo/SKILL.md",
                "skill://demo/ref/notes.txt"
            ]
        );
        let old_digest = l[1].1.clone();
        // Folders, one level at a time.
        let d = rpc(
            &mut s,
            3,
            "resources/directory/read",
            r#"{"uri":"skill://demo/"}"#,
        );
        let names: Vec<_> = result(&d)
            .get("resources")
            .unwrap()
            .as_arr()
            .iter()
            .map(|e| {
                [
                    e.get("uri").unwrap().as_str().unwrap().to_string(),
                    e.get("name").unwrap().as_str().unwrap().to_string(),
                    e.get("mimeType").unwrap().as_str().unwrap().to_string(),
                ]
            })
            .collect();
        assert_eq!(
            names,
            [
                ["skill://demo/SKILL.md", "SKILL.md", "text/markdown"],
                ["skill://demo/ref", "ref", "inode/directory"]
            ]
        );
        assert!(result(&d).get("cacheScope").is_some());
        let d = rpc(
            &mut s,
            3,
            "resources/directory/read",
            r#"{"uri":"skill://demo/ref"}"#,
        );
        assert_eq!(
            result(&d).get("resources").unwrap().as_arr()[0]
                .get("mimeType")
                .unwrap()
                .as_str(),
            Some("text/plain")
        );
        // Edited and committed: the new digest at once; the old bytes at
        // the old commit.
        call(
            &mut s,
            "edit",
            r#"{"path":"skills/demo/SKILL.md","old":"first","new":"second"}"#,
        )
        .unwrap();
        assert_eq!(
            listed(&mut s)[1].1,
            old_digest,
            "pending edits are not listed"
        );
        call(&mut s, "commit", r#"{"message":"demo v2"}"#).unwrap();
        let l = listed(&mut s);
        assert_ne!(l[1].1, old_digest);
        let old = call(
            &mut s,
            "read",
            &format!(r#"{{"path":"skills/demo/SKILL.md","at":"{c1}"}}"#),
        )
        .unwrap();
        assert!(old.contains("first"), "{old}");
        // A folder whose name differs from its frontmatter's is not listed,
        // and the list says why.
        call(&mut s, "write", r#"{"path":"skills/wrong/SKILL.md","content":"---\nname: other\ndescription: x\n---\n"}"#).unwrap();
        // Over the file limit: refused with the reason, never cut.
        call(
            &mut s,
            "write",
            r#"{"path":"skills/big/SKILL.md","content":"---\nname: big\ndescription: x\n---\n"}"#,
        )
        .unwrap();
        for i in 0..SKILL_MAX_FILES {
            call(
                &mut s,
                "write",
                &format!(r#"{{"path":"skills/big/f/{i}.txt","content":"{i}"}}"#),
            )
            .unwrap();
        }
        call(&mut s, "commit", r#"{"message":"two bad skills"}"#).unwrap();
        let r = rpc(&mut s, 4, "skills/list", "{}");
        let names: Vec<_> = result(&r)
            .get("skills")
            .unwrap()
            .as_arr()
            .iter()
            .map(|k| {
                k.get("frontmatter")
                    .unwrap()
                    .get("name")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(names, ["ikam-store", "demo"]);
        let why = result(&r)
            .get("_meta")
            .unwrap()
            .get("ikam/refused")
            .unwrap()
            .encode();
        assert!(
            why.contains("skills/wrong") && why.contains("names it other"),
            "{why}"
        );
        assert!(
            why.contains("skills/big") && why.contains("more than 512 files"),
            "{why}"
        );
        let g = rpc(&mut s, 5, "skills/get", r#"{"uri":"skill://big/SKILL.md"}"#);
        let m = g
            .get("error")
            .unwrap()
            .get("message")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert!(m.contains("refused") && m.contains("512"), "{m}");
        let g = rpc(&mut s, 6, "skills/get", r#"{"uri":"skill://demo"}"#);
        assert_eq!(
            result(&g)
                .get("skill")
                .unwrap()
                .get("resources")
                .unwrap()
                .as_arr()
                .len(),
            2
        );
        assert!(
            rpc(
                &mut s,
                7,
                "resources/read",
                r#"{"uri":"skill://demo/../x"}"#
            )
            .get("error")
            .is_some()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn skills_page_with_a_cursor_and_ride_the_modern_envelope() {
        let (dir, mut s) = session("skillpage");
        s.skills = Some(Some("sk".into()));
        for i in 0..60 {
            call(&mut s, "write", &format!(r#"{{"path":"sk/s{i:02}/SKILL.md","content":"---\nname: s{i:02}\ndescription: skill {i}\n---\n"}}"#)).unwrap();
        }
        call(&mut s, "commit", r#"{"message":"60 skills"}"#).unwrap();
        let r = rpc(&mut s, 1, "skills/list", "{}");
        assert_eq!(
            result(&r).get("skills").unwrap().as_arr().len(),
            SKILLS_PAGE
        );
        let next = result(&r)
            .get("nextCursor")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let r = rpc(
            &mut s,
            2,
            "skills/list",
            &format!(r#"{{"cursor":"{next}"}}"#),
        );
        assert_eq!(
            result(&r).get("skills").unwrap().as_arr().len(),
            61 - SKILLS_PAGE
        );
        assert!(result(&r).get("nextCursor").is_none());
        let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;
        let r = rpc(&mut s, 3, "skills/list", &format!("{{{meta}}}"));
        assert_eq!(
            result(&r).get("resultType").unwrap().as_str(),
            Some("complete")
        );
        let d = rpc(&mut s, 4, "server/discover", &format!("{{{meta}}}"));
        let c = result(&d).get("capabilities").unwrap();
        assert!(
            c.get("resources").is_some() && c.get("extensions").unwrap().get(SKILLS_EXT).is_some()
        );
        assert!(
            result(&d)
                .get("instructions")
                .unwrap()
                .as_str()
                .unwrap()
                .contains("skill://ikam-store/SKILL.md")
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

    /// A line longer than one answer is shown in pieces, each saying where
    /// the next starts; a diff of two non-UTF-8 files that decode alike
    /// still reports the change.
    #[test]
    fn long_lines_page_and_binary_diffs_compare_bytes() {
        let (dir, mut s) = session("longline");
        let line: String = (0..30_000)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let line = format!("{line}é{line}{line}\nnext\n");
        call(
            &mut s,
            "write",
            &format!(r#"{{"path":"one","content":{}}}"#, json::quote(&line)),
        )
        .unwrap();
        let mut got = String::new();
        let mut from = 1;
        for _ in 0..10 {
            let r = call(
                &mut s,
                "read",
                &format!(r#"{{"path":"one","start_line":1,"end_line":1,"start_char":{from}}}"#),
            )
            .unwrap();
            let body = r.split_once('\n').unwrap().1;
            match body.rsplit_once("\n(line 1 is cut; read on with start_line=1 and start_char=") {
                Some((piece, rest)) => {
                    got.push_str(piece);
                    from = rest.trim_end_matches(')').parse().unwrap();
                }
                None => {
                    got.push_str(body);
                    break;
                }
            }
        }
        assert_eq!(got, line.lines().next().unwrap().to_string() + "\n");
        call(&mut s, "commit", r#"{"message":"text"}"#).unwrap();
        let first = s.live.head().unwrap().to_hex();
        s.live
            .apply(vec![Op::Put("bin".into(), vec![0x80, 1])])
            .unwrap();
        s.live.flush_with(true, Some("bin a")).unwrap();
        let a = s.live.head().unwrap().to_hex();
        s.live
            .apply(vec![Op::Put("bin".into(), vec![0x81, 1])])
            .unwrap();
        s.live.flush_with(true, Some("bin b")).unwrap();
        let b = s.live.head().unwrap().to_hex();
        let d = call(
            &mut s,
            "diff",
            &format!(r#"{{"from":"{a}","to":"{b}","path":"bin"}}"#),
        )
        .unwrap();
        assert!(d.contains("binary file changed"), "{d}");
        let d = call(
            &mut s,
            "diff",
            &format!(r#"{{"from":"{first}","to":"{a}","path":"one"}}"#),
        )
        .unwrap();
        assert!(d.contains("no changes"), "{d}");
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
    #[test]
    fn rank_orders_files_follows_edits_and_fuses_vectors() {
        // E388: whole files ranked by BM25, kept as files change, with
        // their best passages; vectors by content join the ranking only
        // when the call gives a query vector.
        let (dir, mut s) = session("rank");
        let files = [
            (
                "a.md",
                "vector search in a graph store\nvector vector index",
            ),
            ("b.md", "graph store notes"),
            ("c.md", "nothing alike here"),
        ];
        for (p, t) in files {
            call(
                &mut s,
                "write",
                &format!(r#"{{"path":"{p}","content":"{}"}}"#, t.replace('\n', "\\n")),
            )
            .unwrap();
        }
        let r = call(&mut s, "rank", r#"{"query":"vector store"}"#).unwrap();
        assert!(r.starts_with("2 file(s), best first (keywords)"), "{r}");
        let order: Vec<&str> = r.lines().filter(|l| l.contains(". ")).collect();
        assert!(
            order[0].starts_with("1. a.md") && order[1].starts_with("2. b.md"),
            "{r}"
        );
        assert!(r.contains("     vector search in a graph store"), "{r}");
        // An edit moves the ranking; a removal drops the file.
        call(
            &mut s,
            "write",
            r#"{"path":"b.md","content":"vector vector vector store store"}"#,
        )
        .unwrap();
        let r = call(&mut s, "rank", r#"{"query":"vector store","k":1}"#).unwrap();
        assert!(r.contains("1. b.md"), "{r}");
        call(&mut s, "remove", r#"{"path":"b.md"}"#).unwrap();
        let r = call(&mut s, "rank", r#"{"query":"store"}"#).unwrap();
        assert!(r.starts_with("1 file(s)") && r.contains("1. a.md"), "{r}");
        // Vectors: c.md points the way the query does.
        let pre = dir.join("vecs");
        std::fs::write(pre.with_extension("paths"), "a.md\nc.md\nmissing.md\n").unwrap();
        let flat: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0, 0.5, 0.5];
        std::fs::write(
            pre.with_extension("f32"),
            ikam_kernel::vectors::from_f32s(&flat),
        )
        .unwrap();
        assert_eq!(s.live.load_file_vectors(pre.to_str().unwrap()).unwrap(), 2);
        let r = call(&mut s, "rank", r#"{"query":"store","vector":[0,1]}"#).unwrap();
        assert!(
            r.starts_with("2 file(s), best first (keywords and vectors)"),
            "{r}"
        );
        let r = call(
            &mut s,
            "rank",
            r#"{"query":"store","vector":[0,1],"alpha":0.2,"k":1}"#,
        )
        .unwrap();
        assert!(r.contains("1. c.md"), "{r}");
        let r = call(
            &mut s,
            "rank",
            r#"{"query":"store","vector":[0,1],"fuse":"rrf"}"#,
        )
        .unwrap();
        assert!(r.contains("a.md") && r.contains("c.md"), "{r}");
        assert!(call(&mut s, "rank", r#"{"query":"store","vector":[1]}"#).is_err());
        assert!(call(&mut s, "rank", r#"{"query":"store","alpha":2}"#).is_err());
        // A changed file loses its vector: c.md is now found by neither list.
        call(&mut s, "write", r#"{"path":"c.md","content":"changed"}"#).unwrap();
        let r = call(&mut s, "rank", r#"{"query":"store","vector":[0,1]}"#).unwrap();
        assert!(r.starts_with("1 file(s)") && !r.contains("c.md"), "{r}");
        assert_eq!(
            call(&mut s, "rank", r#"{"query":"absent"}"#).unwrap(),
            "no file shares a word with the query"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
