//! `ikam live`: one process owns a store and presents its current tree as a
//! filesystem you can change while it runs (E110).
//!
//! - An edit (put, remove, rename, copy, mkdir) stores new content with
//!   `put_content` and rewrites only the trees on the changed path. Nothing
//!   is repacked: an edit costs about what storing that one file costs.
//!   `repack` stays a separate step, run when you choose.
//! - Edits show at once to every reader. They are committed to the ref a
//!   moment after the last one (`commit_after`), so a burst of edits (a
//!   Finder copy) is one commit. If the ref moves under us (another `ikam`
//!   command), pending edits are re-applied on top of it.
//! - Caches that are only ever a copy of what the store says, keyed by
//!   content or tree id so they cannot go stale: file sizes (also kept on
//!   disk in `<store>/cache/sizes`), tree entries, recently read bytes, and
//!   each file's extracted text for search and questions.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use ikam_kernel::{Edit, FsStore, Id, Kind, Object, Repo, Store, TreeEntry, snapshot};

use crate::corpus::err;
use crate::text::{self, Doc};

/// Bytes of recently read content kept in memory.
const BYTE_CACHE: usize = 256 << 20;

pub struct Live {
    pub repo: Repo<FsStore>,
    pub store_dir: PathBuf,
    pub refname: String,
    /// The commit pending edits apply on, and its tree.
    base: Option<Id>,
    base_tree: Option<Id>,
    pending: Vec<Edit>,
    /// Current tree: `base_tree` with `pending` applied.
    pub work: Id,
    last_edit: Instant,
    pub commit_after: Duration,
    trees: HashMap<Id, Arc<Vec<TreeEntry>>>,
    sizes: HashMap<Id, u64>,
    sizes_file: Option<std::fs::File>,
    bytes: VecDeque<(Id, Arc<Vec<u8>>)>,
    bytes_held: usize,
    /// Current files (path -> content id) and their text.
    pub files: BTreeMap<String, Id>,
    pub docs: HashMap<String, Arc<Doc>>,
    extracted: HashMap<Id, Arc<Doc>>,
    /// Edit times by path (the store records none); start time otherwise.
    mtimes: HashMap<String, SystemTime>,
    pub started: SystemTime,
    /// Recent activity, newest last.
    pub events: VecDeque<String>,
    pub llm: bool,
    /// Desktop metadata files (`is_junk`) a WebDAV client wrote: kept in
    /// memory for the client, never stored.
    pub junk: BTreeMap<String, (Vec<u8>, SystemTime)>,
    /// WebDAV locks by path (advisory: one user, never enforced).
    pub locks: HashMap<String, String>,
}

/// What an edit cost.
#[derive(Default, Debug)]
pub struct Cost {
    pub store_ms: f64,
    pub tree_ms: f64,
    pub index_ms: f64,
    pub new_bytes: usize,
}

/// A change requested through any front end; content as bytes.
pub enum Op {
    Put(String, Vec<u8>),
    Mkdir(String),
    Remove(String),
    Rename(String, String),
    Copy(String, String),
}

pub fn norm(path: &str) -> String {
    path.split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect::<Vec<_>>()
        .join("/")
}

impl Live {
    pub fn open(store_dir: &str, refname: &str) -> Result<Live, String> {
        let repo = Repo::new(FsStore::open(store_dir).map_err(err)?);
        let store_dir = PathBuf::from(store_dir);
        let cache_dir = store_dir.join("cache");
        std::fs::create_dir_all(&cache_dir).map_err(err)?;
        let mut sizes = HashMap::new();
        if let Ok(s) = std::fs::read_to_string(cache_dir.join("sizes")) {
            for line in s.lines() {
                if let Some((id, n)) = line.split_once(' ')
                    && let (Ok(id), Ok(n)) = (id.parse::<Id>(), n.parse())
                {
                    sizes.insert(id, n);
                }
            }
        }
        let sizes_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(cache_dir.join("sizes"))
            .ok();
        let mut live = Live {
            repo,
            store_dir,
            refname: refname.into(),
            base: None,
            base_tree: None,
            pending: Vec::new(),
            work: Id::of(b""),
            last_edit: Instant::now(),
            commit_after: Duration::from_millis(1500),
            trees: HashMap::new(),
            sizes,
            sizes_file,
            bytes: VecDeque::new(),
            bytes_held: 0,
            files: BTreeMap::new(),
            docs: HashMap::new(),
            extracted: HashMap::new(),
            mtimes: HashMap::new(),
            started: SystemTime::now(),
            events: VecDeque::new(),
            llm: false,
            junk: BTreeMap::new(),
            locks: HashMap::new(),
        };
        live.base = live.repo.store().get_ref(refname).map_err(err)?;
        live.base_tree = live
            .base
            .map(|c| snapshot::tree_of(&live.repo, &c))
            .transpose()
            .map_err(err)?;
        live.work = match live.base_tree {
            Some(t) => t,
            None => live.repo.edit_tree(None, &[]).map_err(err)?,
        };
        Ok(live)
    }

    /// Extract every file's text (startup), reporting progress.
    pub fn index_all(&mut self, progress: bool) -> Result<(), String> {
        let t = Instant::now();
        self.refresh()?;
        if progress {
            let bytes: usize = self.docs.values().map(|d| d.bytes()).sum();
            eprintln!(
                "indexed {} files ({} bytes of text) in {:.1} s",
                self.docs.len(),
                bytes,
                t.elapsed().as_secs_f64()
            );
        }
        Ok(())
    }

    /// The commit the ref points at, as this process last saw it.
    pub fn head(&self) -> Option<Id> {
        self.base
    }

    /// The text search and questions read for a content, if extracted.
    pub fn extracted_text(&self, id: &Id) -> Option<String> {
        let doc = self.extracted.get(id)?;
        Some(
            doc.parts
                .iter()
                .map(|p| {
                    if p.name.is_empty() {
                        p.text.clone()
                    } else {
                        format!("== {} ==\n{}", p.name, p.text)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Bytes of object files on disk (what `du` of `objects/` counts,
    /// without block rounding).
    pub fn store_bytes(&self) -> u64 {
        fn walk(p: &Path) -> u64 {
            let Ok(rd) = std::fs::read_dir(p) else {
                return 0;
            };
            rd.flatten()
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() => walk(&e.path()),
                    _ => e.metadata().map(|m| m.len()).unwrap_or(0),
                })
                .sum()
        }
        walk(&self.store_dir.join("objects"))
    }

    pub fn log(&mut self, msg: String) {
        let t = humantime(SystemTime::now());
        self.events.push_back(format!("{t} {msg}"));
        while self.events.len() > 200 {
            self.events.pop_front();
        }
    }

    // ---- reading -------------------------------------------------------

    pub fn entries(&mut self, tree: &Id) -> Result<Arc<Vec<TreeEntry>>, String> {
        if let Some(e) = self.trees.get(tree) {
            return Ok(e.clone());
        }
        let Object::Tree(entries) = self.repo.get(tree).map_err(err)? else {
            return Err("not a tree".into());
        };
        let e = Arc::new(entries);
        self.trees.insert(*tree, e.clone());
        Ok(e)
    }

    /// What is at `path` in the current tree: `(Kind::Tree, id)` for a
    /// directory (the root is `""`), `(Kind::File, id)` for a file.
    pub fn stat(&mut self, path: &str) -> Result<Option<(Kind, Id)>, String> {
        let mut cur = (Kind::Tree, self.work);
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if cur.0 != Kind::Tree {
                return Ok(None);
            }
            let entries = self.entries(&cur.1)?;
            match entries.iter().find(|e| e.name == part) {
                Some(e) => cur = (e.kind, e.id),
                None => return Ok(None),
            }
        }
        Ok(Some(cur))
    }

    pub fn read(&mut self, id: &Id) -> Result<Arc<Vec<u8>>, String> {
        if let Some(pos) = self.bytes.iter().position(|(i, _)| i == id) {
            let item = self.bytes.remove(pos).unwrap();
            let b = item.1.clone();
            self.bytes.push_back(item);
            return Ok(b);
        }
        let b = Arc::new(self.repo.read_content(id).map_err(err)?);
        self.note_size(id, b.len() as u64);
        self.hold(*id, b.clone());
        Ok(b)
    }

    fn hold(&mut self, id: Id, b: Arc<Vec<u8>>) {
        if b.len() > BYTE_CACHE / 4 {
            return;
        }
        self.bytes_held += b.len();
        self.bytes.push_back((id, b));
        while self.bytes_held > BYTE_CACHE {
            let (_, old) = self.bytes.pop_front().unwrap();
            self.bytes_held -= old.len();
        }
    }

    pub fn size(&mut self, id: &Id) -> Result<u64, String> {
        match self.sizes.get(id) {
            Some(n) => Ok(*n),
            None => Ok(self.read(id)?.len() as u64),
        }
    }

    fn note_size(&mut self, id: &Id, n: u64) {
        if self.sizes.insert(*id, n).is_none()
            && let Some(f) = &mut self.sizes_file
        {
            let _ = writeln!(f, "{id} {n}");
        }
    }

    pub fn mtime(&self, path: &str) -> SystemTime {
        self.mtimes.get(path).copied().unwrap_or(self.started)
    }

    // ---- changing ------------------------------------------------------

    /// Apply `ops` to the current tree as one change, return its cost.
    pub fn apply(&mut self, ops: Vec<Op>) -> Result<Cost, String> {
        self.sync()?;
        let mut cost = Cost::default();
        let written = self.repo.bytes_written;
        let mut edits = Vec::new();
        let mut summary = Vec::new();
        let t = Instant::now();
        for op in ops {
            match op {
                Op::Put(path, bytes) => {
                    let path = norm(&path);
                    let id = self.repo.put_content(&bytes).map_err(err)?.id;
                    self.note_size(&id, bytes.len() as u64);
                    if let std::collections::hash_map::Entry::Vacant(e) = self.extracted.entry(id) {
                        let ti = Instant::now();
                        e.insert(Arc::new(text::extract(&path, &bytes)));
                        cost.index_ms += ti.elapsed().as_secs_f64() * 1e3;
                    }
                    self.hold(id, Arc::new(bytes));
                    summary.push(format!("put {path}"));
                    self.mtimes.insert(path.clone(), SystemTime::now());
                    edits.push(Edit::Put { path, id });
                }
                Op::Mkdir(path) => {
                    let path = norm(&path);
                    summary.push(format!("mkdir {path}"));
                    edits.push(Edit::Mkdir { path });
                }
                Op::Remove(path) => {
                    let path = norm(&path);
                    summary.push(format!("rm {path}"));
                    edits.push(Edit::Remove { path });
                }
                Op::Rename(from, to) => {
                    let (from, to) = (norm(&from), norm(&to));
                    summary.push(format!("mv {from} {to}"));
                    self.mtimes.insert(to.clone(), SystemTime::now());
                    edits.push(Edit::Rename { from, to });
                }
                Op::Copy(from, to) => {
                    let (from, to) = (norm(&from), norm(&to));
                    summary.push(format!("cp {from} {to}"));
                    self.mtimes.insert(to.clone(), SystemTime::now());
                    edits.push(Edit::Copy { from, to });
                }
            }
        }
        cost.store_ms = t.elapsed().as_secs_f64() * 1e3 - cost.index_ms;
        let t = Instant::now();
        self.work = self.repo.edit_tree(Some(self.work), &edits).map_err(err)?;
        self.pending.extend(edits);
        cost.tree_ms = t.elapsed().as_secs_f64() * 1e3;
        self.last_edit = Instant::now();
        let t = Instant::now();
        self.refresh()?;
        cost.index_ms += t.elapsed().as_secs_f64() * 1e3;
        cost.new_bytes = self.repo.bytes_written - written;
        let mut msg = summary.join(", ");
        if msg.len() > 200 {
            msg = format!(
                "{}… ({} changes)",
                &msg[..msg.floor_char_boundary(200)],
                summary.len()
            );
        }
        self.log(format!(
            "{msg}: {:.0} ms ({} new bytes stored)",
            cost.store_ms + cost.tree_ms + cost.index_ms,
            cost.new_bytes
        ));
        Ok(cost)
    }

    /// Rebuild the path view and text index from the current tree. Only
    /// files whose content changed are extracted again.
    fn refresh(&mut self) -> Result<(), String> {
        let mut files = BTreeMap::new();
        let root = self.work;
        self.walk(&root, "", &mut files)?;
        let gone: Vec<String> = self
            .docs
            .keys()
            .filter(|p| !files.contains_key(*p))
            .cloned()
            .collect();
        for p in gone {
            self.docs.remove(&p);
        }
        // Contents not yet extracted: many at startup, so in parallel.
        let todo: Vec<(&String, &Id)> = {
            let mut seen = HashSet::new();
            files
                .iter()
                .filter(|(_, id)| !self.extracted.contains_key(*id) && seen.insert(**id))
                .collect()
        };
        if todo.len() > 8 {
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
            let repo = &self.repo;
            let done: Vec<(Id, Arc<Doc>)> = std::thread::scope(|s| {
                let chunks: Vec<_> = todo
                    .chunks(todo.len().div_ceil(threads))
                    .map(|chunk| {
                        s.spawn(move || {
                            chunk
                                .iter()
                                .filter_map(|(path, id)| {
                                    let bytes = repo.read_content(id).ok()?;
                                    Some((**id, Arc::new(text::extract(path, &bytes))))
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                chunks.into_iter().flat_map(|h| h.join().unwrap()).collect()
            });
            self.extracted.extend(done);
        }
        for (path, id) in &files {
            if self.files.get(path) == Some(id) && self.docs.contains_key(path) {
                continue;
            }
            let doc = match self.extracted.get(id) {
                Some(d) => d.clone(),
                None => {
                    let bytes = self.read(id)?;
                    let d = Arc::new(text::extract(path, &bytes));
                    self.extracted.insert(*id, d.clone());
                    d
                }
            };
            self.docs.insert(path.clone(), doc);
        }
        // Extracted text of contents no longer in the tree is let go.
        let live: HashSet<&Id> = files.values().collect();
        self.extracted.retain(|id, _| live.contains(id));
        self.files = files;
        Ok(())
    }

    fn walk(
        &mut self,
        tree: &Id,
        prefix: &str,
        out: &mut BTreeMap<String, Id>,
    ) -> Result<(), String> {
        for e in self.entries(tree)?.iter() {
            let path = format!("{prefix}{}", e.name);
            match e.kind {
                Kind::Tree => self.walk(&e.id, &format!("{path}/"), out)?,
                Kind::File => {
                    out.insert(path, e.id);
                }
                Kind::Graph => {}
            }
        }
        Ok(())
    }

    /// Follow the ref if something else moved it, keeping pending edits.
    pub fn sync(&mut self) -> Result<(), String> {
        let head = self.repo.store().get_ref(&self.refname).map_err(err)?;
        if head == self.base {
            return Ok(());
        }
        self.base = head;
        self.base_tree = head
            .map(|c| snapshot::tree_of(&self.repo, &c))
            .transpose()
            .map_err(err)?;
        self.work = self
            .repo
            .edit_tree(self.base_tree, &self.pending)
            .map_err(err)?;
        self.log("the ref moved outside this server: now showing it".into());
        self.refresh()
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Commit pending edits if the last was `commit_after` ago (or `now`).
    pub fn flush(&mut self, now: bool) -> Result<Option<Id>, String> {
        if self.pending.is_empty() || (!now && self.last_edit.elapsed() < self.commit_after) {
            return Ok(None);
        }
        self.sync()?;
        if Some(self.work) == self.base_tree {
            self.pending.clear();
            return Ok(None);
        }
        let n = self.pending.len();
        let message = format!("live: {n} change(s)");
        let commit = self
            .repo
            .commit(&self.refname, self.work, &message)
            .map_err(err)?;
        self.base = Some(commit);
        self.base_tree = Some(self.work);
        self.pending.clear();
        self.log(format!(
            "committed {n} change(s) as {}",
            &commit.to_hex()[..12]
        ));
        Ok(Some(commit))
    }

    /// Repack the store (all live content re-planned); commits first.
    pub fn repack(&mut self) -> Result<String, String> {
        self.flush(true)?;
        let t = Instant::now();
        let r = self.repo.repack().map_err(err)?;
        if self.base_tree.is_none() {
            // Nothing committed yet: the empty working tree was reachable from
            // no ref, so the repack's gc removed it. Store it again.
            self.work = self.repo.edit_tree(None, &[]).map_err(err)?;
        }
        let msg = format!(
            "repack: {} -> {} bytes, applied={}, {:.1} s",
            r.before,
            r.after,
            r.applied,
            t.elapsed().as_secs_f64()
        );
        self.log(msg.clone());
        Ok(msg)
    }

    /// Commits of the ref, newest first, with the files each changed.
    pub fn history(&mut self, limit: usize) -> Result<Vec<(Id, String, Vec<String>)>, String> {
        let Some(head) = self.base else {
            return Ok(Vec::new());
        };
        let log = self.repo.log(head).map_err(err)?;
        let mut out = Vec::new();
        for (i, (id, c)) in log.iter().enumerate().take(limit) {
            let parent = log.get(i + 1).map(|(_, p)| p.tree);
            let mut changes = Vec::new();
            self.diff(parent, Some(c.tree), "", &mut changes)?;
            out.push((*id, c.message.clone(), changes));
        }
        Ok(out)
    }

    /// Changed paths between two trees (`+` added, `-` removed, `~` changed),
    /// skipping equal subtrees.
    pub fn diff(
        &mut self,
        a: Option<Id>,
        b: Option<Id>,
        prefix: &str,
        out: &mut Vec<String>,
    ) -> Result<(), String> {
        if a == b {
            return Ok(());
        }
        let ea = match a {
            Some(t) => self.entries(&t)?,
            None => Arc::new(Vec::new()),
        };
        let eb = match b {
            Some(t) => self.entries(&t)?,
            None => Arc::new(Vec::new()),
        };
        let mut names: Vec<&String> = ea.iter().chain(eb.iter()).map(|e| &e.name).collect();
        names.sort();
        names.dedup();
        for name in names {
            let x = ea.iter().find(|e| &e.name == name);
            let y = eb.iter().find(|e| &e.name == name);
            let path = format!("{prefix}{name}");
            let tree = |e: Option<&TreeEntry>| e.filter(|e| e.kind == Kind::Tree).map(|e| e.id);
            match (x, y) {
                (Some(x), Some(y)) if x.id == y.id && x.kind == y.kind => {}
                _ if tree(x).is_some() || tree(y).is_some() => {
                    if let Some(x) = x.filter(|x| x.kind == Kind::File) {
                        let _ = x;
                        out.push(format!("- {path}"));
                    }
                    self.diff(tree(x), tree(y), &format!("{path}/"), out)?;
                    if y.is_some_and(|y| y.kind == Kind::File) {
                        out.push(format!("+ {path}"));
                    }
                }
                (Some(_), Some(_)) => out.push(format!("~ {path}")),
                (Some(_), None) => out.push(format!("- {path}")),
                (None, Some(_)) => out.push(format!("+ {path}")),
                (None, None) => {}
            }
        }
        Ok(())
    }

    /// Versions of `path`: commits, newest first, where its content changed.
    pub fn versions(&mut self, path: &str) -> Result<Vec<(Id, String, Option<Id>)>, String> {
        let Some(head) = self.base else {
            return Ok(Vec::new());
        };
        let log = self.repo.log(head).map_err(err)?;
        let mut out: Vec<(Id, String, Option<Id>)> = Vec::new();
        let mut last: Option<Option<Id>> = None;
        for (id, c) in log.iter().rev() {
            let mut cur = (Kind::Tree, c.tree);
            let mut found = Some(());
            for part in path.split('/').filter(|p| !p.is_empty()) {
                let entries = self.entries(&cur.1)?;
                match entries.iter().find(|e| e.name == part) {
                    Some(e) if cur.0 == Kind::Tree => cur = (e.kind, e.id),
                    _ => {
                        found = None;
                        break;
                    }
                }
            }
            let at = found.filter(|_| cur.0 == Kind::File).map(|_| cur.1);
            if last != Some(at) {
                out.push((*id, c.message.clone(), at));
                last = Some(at);
            }
        }
        out.reverse();
        Ok(out)
    }
}

/// Watch `dir` and mirror it into the tree under `prefix`: new and changed
/// files are stored, deleted ones removed, every `every`. The prefix is the
/// watcher's: what is under it in the tree follows the folder.
pub fn watch(live: Arc<std::sync::Mutex<Live>>, dir: PathBuf, prefix: String, every: Duration) {
    let mut seen: HashMap<String, (u64, SystemTime)> = HashMap::new();
    let mut first = true;
    loop {
        let mut now = HashMap::new();
        scan(&dir, &dir, &mut now);
        let mut ops = Vec::new();
        let tree_files: Vec<(String, Id)> = if first {
            let l = live.lock().unwrap();
            l.files
                .iter()
                .filter(|(p, _)| prefix.is_empty() || p.starts_with(&format!("{prefix}/")))
                .map(|(p, id)| (p.clone(), *id))
                .collect()
        } else {
            Vec::new()
        };
        let in_tree: HashMap<&str, Id> = tree_files.iter().map(|(p, i)| (p.as_str(), *i)).collect();
        for (rel, meta) in &now {
            if seen.get(rel) == Some(meta) {
                continue;
            }
            let Ok(bytes) = std::fs::read(dir.join(rel)) else {
                continue;
            };
            let path = join(&prefix, rel);
            if first && in_tree.get(path.as_str()) == Some(&Id::of_content(&bytes)) {
                continue;
            }
            ops.push(Op::Put(path, bytes));
        }
        if first {
            for (p, _) in &tree_files {
                let rel = p.strip_prefix(&format!("{prefix}/")).unwrap_or(p);
                if !now.contains_key(rel) {
                    ops.push(Op::Remove(p.clone()));
                }
            }
        } else {
            for rel in seen.keys() {
                if !now.contains_key(rel) {
                    ops.push(Op::Remove(join(&prefix, rel)));
                }
            }
        }
        if !ops.is_empty() {
            let n = ops.len();
            let mut l = live.lock().unwrap();
            match l.apply(ops) {
                Ok(c) => l.log(format!(
                    "watch {}: {n} file change(s) in {:.0} ms",
                    dir.display(),
                    c.store_ms + c.tree_ms + c.index_ms
                )),
                Err(e) => l.log(format!("watch {}: failed: {e}", dir.display())),
            }
        }
        seen = now;
        first = false;
        std::thread::sleep(every);
    }
}

fn join(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else {
        format!("{prefix}/{rel}")
    }
}

fn scan(root: &Path, dir: &Path, out: &mut HashMap<String, (u64, SystemTime)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if is_junk(&name) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_dir() {
            scan(root, &path, out);
        } else if ft.is_file()
            && let Ok(m) = e.metadata()
            && let Ok(rel) = path.strip_prefix(root)
        {
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.insert(
                rel,
                (m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
            );
        }
    }
}

/// Files a desktop writes for itself (Finder metadata, editor swap files):
/// never stored.
pub fn is_junk(name: &str) -> bool {
    name.starts_with("._")
        || name == ".DS_Store"
        || name == ".localized"
        || name.starts_with(".Spotlight-")
        || name == ".Trashes"
        || name == ".fseventsd"
        || name.starts_with(".metadata_never_index")
        || name.starts_with(".~lock.")
        || name.ends_with(".swp") && name.starts_with('.')
        || name == ".ikam"
}

pub fn humantime(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}Z")
}

/// RFC 1123 date, as HTTP and WebDAV want it.
pub fn http_date(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let (y, mo, d) = civil(days);
    let wd = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize];
    let mon = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][mo as usize - 1];
    format!(
        "{wd}, {d:02} {mon} {y} {:02}:{:02}:{:02} GMT",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// ISO 8601 UTC (WebDAV creationdate).
pub fn iso_date(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let (y, mo, d) = civil(days);
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day) (Howard Hinnant's algorithm).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_794_268_800); // 2026-11-10
        assert_eq!(http_date(t), "Tue, 10 Nov 2026 00:00:00 GMT");
        assert_eq!(iso_date(t), "2026-11-10T00:00:00Z");
    }

    #[test]
    fn edits_after_repacking_an_empty_store() {
        let dir = std::env::temp_dir().join(format!("ikam-live-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut l = Live::open(dir.to_str().unwrap(), "main").unwrap();
        l.repack().unwrap();
        l.apply(vec![Op::Mkdir("made".into())]).unwrap();
        l.apply(vec![Op::Put("made/a.txt".into(), b"hello".to_vec())])
            .unwrap();
        l.flush(true).unwrap().expect("a commit");
        assert!(l.repo.fsck_deep().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
