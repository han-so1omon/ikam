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
    input: impl BufRead,
    mut output: impl Write,
) -> Result<(), String> {
    let mut s = Session::open(store_dir, refname)?;
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
            "initialize" => initialize(&params),
            "ping" => Json::Obj(Vec::new()),
            "tools/list" => obj(vec![("tools", Json::Arr(tools()))]),
            "tools/call" => self.tools_call(&params),
            _ => return Some(error(id, -32601, &format!("method not found: {method}"))),
        };
        Some(obj(vec![
            ("jsonrpc", s("2.0")),
            ("id", id),
            ("result", result),
        ]))
    }

    /// A 2026-07-28 request, already checked by `modern_request`.
    fn handle_modern(&mut self, id: Json, method: &str, params: &Json) -> Json {
        let (mut result, cacheable) = match method {
            "server/discover" => (obj(discover_fields()), true),
            "tools/list" => (obj(vec![("tools", Json::Arr(tools()))]), true),
            "tools/call" => (self.tools_call(params), false),
            // `initialize` and `ping` are gone from this era; every other
            // method is unknown to both.
            _ => return error(id, -32601, &format!("method not found: {method}")),
        };
        modern_result(&mut result, cacheable);
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

fn capabilities() -> Json {
    obj(vec![(
        "tools",
        obj(vec![("listChanged", Json::Bool(false))]),
    )])
}

fn server_info() -> Json {
    obj(vec![
        ("name", s("ikam")),
        ("version", s(env!("CARGO_PKG_VERSION"))),
    ])
}

/// `server/discover`'s fields, before `modern_result` adds the era's own.
fn discover_fields() -> Vec<(&'static str, Json)> {
    let versions = VERSIONS
        .iter()
        .chain(MODERN_VERSIONS)
        .map(|v| s(v))
        .collect();
    vec![
        ("supportedVersions", Json::Arr(versions)),
        ("capabilities", capabilities()),
        ("instructions", s(INSTRUCTIONS)),
    ]
}

/// What every 2026-07-28 result carries: `resultType`, the server's info in
/// `_meta`, and for the cacheable lists a time to keep them (`private`:
/// a store's server is one person's).
fn modern_result(result: &mut Json, cacheable: bool) {
    if let Json::Obj(f) = result {
        f.push(("resultType".into(), s("complete")));
        if cacheable {
            f.push(("ttlMs".into(), Json::Num(LIST_TTL_MS as f64)));
            f.push(("cacheScope".into(), s("private")));
        }
        f.push(("_meta".into(), obj(vec![(SERVER_INFO, server_info())])));
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
        ("capabilities", capabilities()),
        ("serverInfo", server_info()),
        ("instructions", s(INSTRUCTIONS)),
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
