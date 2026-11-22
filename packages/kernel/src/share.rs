//! Scoped shares between separate projects (E315 to E318, opt-in, library
//! only).
//!
//! A project is one store with its own graph; nothing crosses between
//! stores unless its owner opens a **scope** to another project:
//!
//! - `Path`: the files under a prefix;
//! - `Section`: byte ranges of one file version (as section grants, E140);
//! - `Fragments`: listed fragments (content ids), as fragment grants (E155);
//! - `Records`: a key range of the database at the share's commit;
//! - `Graph`: the nodes under a label prefix of a graph at a path.
//!
//! A [`Share`] gives one receiving project a [`Right`] on a scope, pinned to
//! one commit of the source, until an expiry. It is sealed with the source
//! project's key and logged in the source's own `shares` database, so the
//! source lists and revokes its shares, and a share that is revoked or
//! replaced no longer checks. Every read through a share returns only bytes
//! of its scope, from file, record or node layers, never storage objects (a
//! group or dictionary object can hold bytes of other files, E126).
//!
//! A merge ([`import_share`]) copies a share's scope into the
//! receiving project's own store: bytes are put again there, so the two
//! stores share nothing. Each import is a commit on the receiver's
//! `import/<project>/<share>` ref, built on the previous import, and is then
//! merged into the target ref with `Repo::merge` (E265), so merging again
//! is three-way against what was merged last and a change on both sides is
//! a conflict. Who may authorize a merge is the caller's decision. A
//! `Section` import is the file at its path holding only the granted
//! ranges, joined in order; a `Fragments` import is `fragments/<id>` for
//! each fragment; a `Graph` import is a graph at its path with the share's
//! nodes, each node's bytes put again (a range as its own content) and
//! only the edges among them (E365).

use std::ops::Range;

use crate::merge::Merged;
use crate::worktree::split_path;
use crate::{Db, Edit, Error, Event, FileGrant, Id, Kind, Node, Object, Repo, Store};

/// The ref holding a project's share log (a database: `share/<name>`).
pub const SHARES_REF: &str = "shares";

const TAG: &[u8; 16] = b"ikam:share v1\0\0\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Path(String),
    Section {
        path: String,
        id: Id,
        ranges: Vec<Range<u64>>,
    },
    Fragments(Vec<Id>),
    Records {
        start: String,
        end: Option<String>,
    },
    Graph {
        path: String,
        prefix: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Right {
    Read,
    Propose,
    Merge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub from: String,
    pub to: String,
    pub commit: Id,
    pub scope: Scope,
    pub right: Right,
    /// Seconds since the Unix epoch after which the share no longer checks.
    pub expires: u64,
}

/// Why a share was refused. Callers serving other projects should answer
/// every one alike, as if the thing did not exist.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    Seal,
    Project,
    Expired,
    Revoked,
    Right,
    Outside,
    NotMergeable,
    Store(String),
}

impl From<Error> for Refused {
    fn from(e: Error) -> Refused {
        Refused::Store(e.to_string())
    }
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u64).to_le_bytes());
    out.extend_from_slice(b);
}

impl Share {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = TAG.to_vec();
        for s in [&self.name, &self.from, &self.to] {
            put_bytes(&mut out, s.as_bytes());
        }
        out.extend_from_slice(self.commit.as_bytes());
        out.push(self.right as u8);
        out.extend_from_slice(&self.expires.to_le_bytes());
        match &self.scope {
            Scope::Path(p) => {
                out.push(0);
                put_bytes(&mut out, p.as_bytes());
            }
            Scope::Section { path, id, ranges } => {
                out.push(1);
                put_bytes(&mut out, path.as_bytes());
                out.extend_from_slice(id.as_bytes());
                out.extend_from_slice(&(ranges.len() as u64).to_le_bytes());
                for r in ranges {
                    out.extend_from_slice(&r.start.to_le_bytes());
                    out.extend_from_slice(&r.end.to_le_bytes());
                }
            }
            Scope::Fragments(ids) => {
                out.push(2);
                out.extend_from_slice(&(ids.len() as u64).to_le_bytes());
                ids.iter().for_each(|i| out.extend_from_slice(i.as_bytes()));
            }
            Scope::Records { start, end } => {
                out.push(3);
                put_bytes(&mut out, start.as_bytes());
                match end {
                    Some(e) => {
                        out.push(1);
                        put_bytes(&mut out, e.as_bytes());
                    }
                    None => out.push(0),
                }
            }
            Scope::Graph { path, prefix } => {
                out.push(4);
                put_bytes(&mut out, path.as_bytes());
                put_bytes(&mut out, prefix.as_bytes());
            }
        }
        out
    }

    /// The share `encode` wrote; none for anything else.
    pub fn decode(bytes: &[u8]) -> Option<Share> {
        let mut r = Reader(bytes.strip_prefix(&TAG[..])?);
        let name = r.text()?;
        let from = r.text()?;
        let to = r.text()?;
        let commit = r.id()?;
        let right = match r.byte()? {
            0 => Right::Read,
            1 => Right::Propose,
            2 => Right::Merge,
            _ => return None,
        };
        let expires = r.u64()?;
        let scope = match r.byte()? {
            0 => Scope::Path(r.text()?),
            1 => {
                let path = r.text()?;
                let id = r.id()?;
                let n = r.u64()?;
                let mut ranges = Vec::new();
                for _ in 0..n {
                    ranges.push(r.u64()?..r.u64()?);
                }
                Scope::Section { path, id, ranges }
            }
            2 => {
                let n = r.u64()?;
                let mut ids = Vec::new();
                for _ in 0..n {
                    ids.push(r.id()?);
                }
                Scope::Fragments(ids)
            }
            3 => {
                let start = r.text()?;
                let end = match r.byte()? {
                    0 => None,
                    1 => Some(r.text()?),
                    _ => return None,
                };
                Scope::Records { start, end }
            }
            4 => Scope::Graph {
                path: r.text()?,
                prefix: r.text()?,
            },
            _ => return None,
        };
        let share = Share {
            name,
            from,
            to,
            commit,
            scope,
            right,
            expires,
        };
        (r.0.is_empty() && share.encode() == bytes).then_some(share)
    }

    pub fn seal(&self, key: &[u8; 32]) -> [u8; 32] {
        *blake3::keyed_hash(key, &self.encode()).as_bytes()
    }

    /// Whether the file at `path` is inside a `Path` or `Section` scope.
    pub fn covers_path(&self, path: &str) -> bool {
        if path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return false;
        }
        match &self.scope {
            Scope::Path(p) => under(path, p),
            Scope::Section { path: p, .. } => p == path,
            _ => false,
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn id(&mut self) -> Option<Id> {
        let b: [u8; 32] = self.take(32)?.try_into().ok()?;
        Some(Id::from_bytes(b))
    }
    fn text(&mut self) -> Option<String> {
        let n = usize::try_from(self.u64()?).ok()?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

/// `path` is `prefix` or below it ("" covers everything).
fn under(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    prefix.is_empty()
        || path == prefix
        || (path.starts_with(prefix) && path.as_bytes().get(prefix.len()) == Some(&b'/'))
}

fn key_of(name: &str) -> String {
    format!("share/{name}")
}

impl<S: Store> Repo<S> {
    fn shares_db(&mut self) -> Result<Db, Error> {
        match self.store.get_ref(SHARES_REF)? {
            Some(c) => self.db_at(&c),
            None => self.db_empty(),
        }
    }

    /// Log `share` in this (the source) project and answer its seal.
    pub fn share_issue(&mut self, share: &Share, key: &[u8; 32]) -> Result<[u8; 32], Error> {
        let db = self.shares_db()?;
        let tag = share.seal(key);
        let idem = format!("issue/{}", blake3::hash(&tag).to_hex());
        let db = self.db_append(
            &db,
            &[Event::upsert(&key_of(&share.name), &share.encode(), &idem)],
        )?;
        self.db_commit(SHARES_REF, &db, &format!("share {}", share.name))?;
        Ok(tag)
    }

    /// Remove the share `name` from the log; true if it was there.
    pub fn share_revoke(&mut self, name: &str) -> Result<bool, Error> {
        let db = self.shares_db()?;
        if self.db_get(&db, &key_of(name))?.is_none() {
            return Ok(false);
        }
        let head = self.db_head(&db.log)?;
        let idem = format!("revoke/{name}/{head}");
        let db = self.db_append(&db, &[Event::remove(&key_of(name), false, &idem)])?;
        self.db_commit(SHARES_REF, &db, &format!("revoke {name}"))?;
        Ok(true)
    }

    /// The shares this project has issued and not revoked.
    pub fn shares(&mut self) -> Result<Vec<(String, Vec<u8>)>, Error> {
        let db = self.shares_db()?;
        self.db_prefix(&db, "share/", usize::MAX, false)
    }

    /// The logged share named `name`, if it has not been revoked.
    pub fn share_named(&mut self, name: &str) -> Result<Option<Share>, Error> {
        let db = self.shares_db()?;
        Ok(self
            .db_get(&db, &key_of(name))?
            .and_then(|b| Share::decode(&b)))
    }

    /// Whether `share` (with `tag`) lets project `to` use `right` at `now`.
    pub fn share_check(
        &mut self,
        share: &Share,
        tag: &[u8; 32],
        key: &[u8; 32],
        to: &str,
        right: Right,
        now: u64,
    ) -> Result<(), Refused> {
        if blake3::keyed_hash(key, &share.encode()) != blake3::Hash::from(*tag) {
            return Err(Refused::Seal);
        }
        if share.to != to {
            return Err(Refused::Project);
        }
        if now >= share.expires {
            return Err(Refused::Expired);
        }
        let db = self.shares_db()?;
        if self.db_get(&db, &key_of(&share.name))?.as_deref() != Some(&share.encode()[..]) {
            return Err(Refused::Revoked);
        }
        let ok = match right {
            Right::Read => true,
            Right::Propose => share.right == Right::Propose,
            Right::Merge => share.right == Right::Merge,
        };
        if ok { Ok(()) } else { Err(Refused::Right) }
    }

    fn tree_at(&self, commit: &Id) -> Result<Id, Error> {
        match self.get(commit)? {
            Object::Commit(c) => Ok(c.tree),
            _ => Err(Error::WrongKind(*commit)),
        }
    }

    /// The entry at `path` under tree `root`.
    fn entry(&self, root: Id, path: &str) -> Result<Option<(Kind, Id)>, Error> {
        let (mut kind, mut id) = (Kind::Tree, root);
        for part in split_path(path)? {
            let Object::Tree(entries) = self.get(&id)? else {
                return Ok(None);
            };
            match entries.into_iter().find(|e| e.name == part) {
                Some(e) => (kind, id) = (e.kind, e.id),
                None => return Ok(None),
            }
        }
        Ok(Some((kind, id)))
    }

    fn files_under(&self, id: Id, at: &str, out: &mut Vec<(String, Id)>) -> Result<(), Error> {
        let Object::Tree(entries) = self.get(&id)? else {
            return Ok(());
        };
        for e in entries {
            let path = if at.is_empty() {
                e.name.clone()
            } else {
                format!("{at}/{}", e.name)
            };
            match e.kind {
                Kind::File => out.push((path, e.id)),
                Kind::Tree if !(at.is_empty() && e.name == "db") => {
                    self.files_under(e.id, &path, out)?
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The files a `Path` or `Section` share covers at its commit.
    pub fn share_files(&self, share: &Share) -> Result<Vec<(String, Id)>, Refused> {
        let root = self.tree_at(&share.commit)?;
        let mut out = Vec::new();
        match &share.scope {
            Scope::Path(p) if p.trim_end_matches('/').is_empty() => {
                self.files_under(root, "", &mut out)?
            }
            Scope::Path(p) => match self.entry(root, p)? {
                Some((Kind::File, id)) => out.push((p.trim_end_matches('/').to_string(), id)),
                Some((Kind::Tree, id)) => {
                    self.files_under(id, p.trim_end_matches('/'), &mut out)?
                }
                _ => {}
            },
            Scope::Section { path, id, .. } => {
                if self.entry(root, path)? == Some((Kind::File, *id)) {
                    out.push((path.clone(), *id));
                }
            }
            _ => return Err(Refused::Outside),
        }
        Ok(out)
    }

    /// Bytes `range` (or all) of the file at `path` through `share`.
    pub fn share_read(
        &self,
        share: &Share,
        path: &str,
        range: Option<Range<u64>>,
    ) -> Result<Vec<u8>, Refused> {
        if !share.covers_path(path) {
            return Err(Refused::Outside);
        }
        let root = self.tree_at(&share.commit)?;
        let Some((Kind::File, id)) = self.entry(root, path)? else {
            return Err(Refused::Outside);
        };
        if let Scope::Section {
            id: granted,
            ranges,
            ..
        } = &share.scope
        {
            let r = range.clone().ok_or(Refused::Outside)?;
            if id != *granted
                || r.start >= r.end
                || !ranges.iter().any(|g| g.start <= r.start && r.end <= g.end)
            {
                return Err(Refused::Outside);
            }
        }
        match range {
            Some(r) if r.start < r.end => Ok(self.read_range(&id, r.start, r.end - r.start)?.0),
            Some(_) => Err(Refused::Outside),
            None => Ok(self.read_content(&id)?),
        }
    }

    /// The bytes of fragment `id` through a `Fragments` share.
    pub fn share_fragment(&self, share: &Share, id: &Id) -> Result<Vec<u8>, Refused> {
        match &share.scope {
            Scope::Fragments(ids) if ids.contains(id) => Ok(self.read_content(id)?),
            _ => Err(Refused::Outside),
        }
    }

    /// Records with keys in `start..end`, clipped to a `Records` share.
    pub fn share_records(
        &mut self,
        share: &Share,
        start: &str,
        end: Option<&str>,
    ) -> Result<Vec<(String, Vec<u8>)>, Refused> {
        let Scope::Records { start: s, end: e } = &share.scope else {
            return Err(Refused::Outside);
        };
        let lo = start.max(s.as_str()).to_string();
        let hi = match (end, e.as_deref()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if hi.is_some_and(|h| h <= lo.as_str()) {
            return Ok(Vec::new());
        }
        let db = self.db_at(&share.commit)?;
        Ok(self.db_range(&db, &lo, hi, usize::MAX, false)?)
    }

    /// Graph nodes whose labels start with `prefix`, clipped to a `Graph`
    /// share.
    pub fn share_nodes(&self, share: &Share, prefix: &str) -> Result<Vec<Node>, Refused> {
        let Scope::Graph { path, prefix: p } = &share.scope else {
            return Err(Refused::Outside);
        };
        let narrow = if prefix.starts_with(p.as_str()) {
            prefix
        } else if p.starts_with(prefix) {
            p.as_str()
        } else {
            return Ok(Vec::new());
        };
        let root = self.tree_at(&share.commit)?;
        let Some((Kind::Graph, g)) = self.entry(root, path)? else {
            return Err(Refused::Outside);
        };
        Ok(self.graph_prefix(&g, narrow)?)
    }

    /// Paths (or record keys) in a share's scope whose bytes contain
    /// `pattern`. Only scoped bytes are searched, so a hit never comes
    /// from outside the scope.
    pub fn share_search(&mut self, share: &Share, pattern: &[u8]) -> Result<Vec<String>, Refused> {
        let has = |b: &[u8]| pattern.is_empty() || b.windows(pattern.len()).any(|w| w == pattern);
        match &share.scope {
            Scope::Path(_) => {
                let files = self.share_files(share)?;
                let ids: Vec<Id> = files.iter().map(|f| f.1).collect();
                let (hits, _) = self.search(pattern, &ids, None)?;
                Ok(files
                    .into_iter()
                    .filter(|f| hits.contains(&f.1))
                    .map(|f| f.0)
                    .collect())
            }
            Scope::Section { path, ranges, .. } => {
                for r in ranges {
                    if has(&self.share_read(share, path, Some(r.clone()))?) {
                        return Ok(vec![path.clone()]);
                    }
                }
                Ok(Vec::new())
            }
            Scope::Fragments(ids) => {
                let mut out = Vec::new();
                for id in ids {
                    if has(&self.read_content(id)?) {
                        out.push(id.to_hex());
                    }
                }
                Ok(out)
            }
            Scope::Records { .. } => {
                let rows = self.share_records(share, "", None)?;
                Ok(rows
                    .into_iter()
                    .filter(|(_, v)| has(v))
                    .map(|(k, _)| k)
                    .collect())
            }
            Scope::Graph { .. } => {
                let nodes = self.share_nodes(share, "")?;
                Ok(nodes
                    .into_iter()
                    .filter(|n| has(n.label.as_bytes()))
                    .map(|n| n.label)
                    .collect())
            }
        }
    }

    /// Edits proposed through a `Propose` share, committed to the source
    /// project's `proposals/<share>` ref (from the share's commit) for its
    /// owner to merge. A `Path` share takes any file under its prefix; a
    /// `Section` share only changes inside its ranges (E140's check).
    pub fn share_propose(
        &mut self,
        share: &Share,
        edits: &[(String, Vec<u8>)],
        message: &str,
    ) -> Result<Id, Refused> {
        if share.right != Right::Propose {
            return Err(Refused::Right);
        }
        let refname = format!("proposals/{}", share.name);
        let base = match self.store.get_ref(&refname)? {
            Some(c) => c,
            None => {
                self.branch(&refname, share.commit)?;
                share.commit
            }
        };
        let mut tree_edits = Vec::new();
        for (path, bytes) in edits {
            if !share.covers_path(path) {
                return Err(Refused::Outside);
            }
            if let Scope::Section { id, ranges, .. } = &share.scope {
                let old = self.read_content(id)?;
                FileGrant::new(path, *id, ranges.clone(), ranges.clone())
                    .check_edit(&old, bytes)
                    .map_err(|_| Refused::Outside)?;
            }
            let id = self.put_content(bytes)?.id;
            tree_edits.push(Edit::Put {
                path: path.clone(),
                id,
            });
        }
        let tree = self.edit_tree(Some(self.tree_at(&base)?), &tree_edits)?;
        Ok(self.commit(&refname, tree, message)?)
    }
}

/// Copy a `Merge` share's scope from `src` into `dst` and merge it into
/// `dst`'s ref `into` (created if missing). The caller checks the share
/// first (`share_check` with `Right::Merge`) and decides who authorizes.
pub fn import_share<S: Store, T: Store>(
    src: &mut Repo<S>,
    share: &Share,
    dst: &mut Repo<T>,
    into: &str,
    message: &str,
) -> Result<Merged, Refused> {
    if share.right != Right::Merge {
        return Err(Refused::Right);
    }
    let import = format!("import/{}/{}", share.from, share.name);
    let prev = dst.store.get_ref(&import)?;
    let note = format!(
        "{message}\n\nimport {} share {} at {}",
        share.from,
        share.name,
        share.commit.to_hex()
    );
    let head = match &share.scope {
        Scope::Path(_) => {
            let files = src.share_files(share)?;
            let mut edits = Vec::new();
            let mut root = None;
            if let Some(p) = prev {
                let old = dst.tree_at(&p)?;
                let mut have = Vec::new();
                dst.files_under(old, "", &mut have)?;
                edits.extend(have.into_iter().map(|(path, _)| Edit::Remove { path }));
                root = Some(old);
            }
            for (path, id) in files {
                let id = dst.put_content(&src.read_content(&id)?)?.id;
                edits.push(Edit::Put { path, id });
            }
            let tree = dst.edit_tree(root, &edits)?;
            dst.commit(&import, tree, &note)?
        }
        Scope::Records { .. } => {
            let rows = src.share_records(share, "", None)?;
            let db = match prev {
                Some(p) => dst.db_at(&p)?,
                None => dst.db_empty()?,
            };
            let old = dst.db_scan(&db)?;
            let seq = dst.db_head(&db.log)?;
            let idem = |k: &str| format!("import/{}/{seq}/{k}", share.commit.to_hex());
            let mut events: Vec<Event> = old
                .iter()
                .filter(|(k, _)| rows.binary_search_by(|r| r.0.as_str().cmp(k)).is_err())
                .map(|(k, _)| Event::remove(k, false, &idem(k)))
                .collect();
            for (k, v) in &rows {
                if old.binary_search_by(|o| o.0.cmp(k)).map(|i| &old[i].1) != Ok(v) {
                    events.push(Event::upsert(k, v, &idem(k)));
                }
            }
            let db = dst.db_append(&db, &events)?;
            if prev.is_none() {
                let empty = dst.edit_tree(None, &[])?;
                dst.commit(&import, empty, &note)?;
            }
            dst.db_commit(&import, &db, &note)?
        }
        // E365: the other scopes import what a reader of the share sees,
        // as a fresh tree each time (the scope's state at the share's
        // commit), so merging again is three-way against the last import.
        Scope::Section { path, id, ranges } => {
            if src.share_files(share)?.is_empty() {
                return Err(Refused::Outside);
            }
            let mut bytes = Vec::new();
            for r in joined(ranges) {
                bytes.extend(src.read_range(id, r.start, r.end - r.start)?.0);
            }
            let id = dst.put_content(&bytes)?.id;
            let edits = [Edit::Put {
                path: path.clone(),
                id,
            }];
            let tree = dst.edit_tree(None, &edits)?;
            dst.commit(&import, tree, &note)?
        }
        Scope::Fragments(ids) => {
            let mut edits = Vec::new();
            for f in ids {
                let id = dst.put_content(&src.share_fragment(share, f)?)?.id;
                edits.push(Edit::Put {
                    path: format!("fragments/{}", f.to_hex()),
                    id,
                });
            }
            let tree = dst.edit_tree(None, &edits)?;
            dst.commit(&import, tree, &note)?
        }
        Scope::Graph { path, .. } => {
            let graph = copy_nodes(src, &src.share_nodes(share, "")?, dst)?;
            let empty = dst.edit_tree(None, &[])?;
            let tree = put_entry(dst, empty, &split_path(path)?, Kind::Graph, graph)?;
            dst.commit(&import, tree, &note)?
        }
    };
    if dst.store.get_ref(into)?.is_none() {
        dst.branch(into, head)?;
        return Ok(Merged::FastForward(head));
    }
    Ok(dst.merge(into, &import, message)?)
}

/// `ranges` sorted, with overlapping or touching ones joined.
fn joined(ranges: &[Range<u64>]) -> Vec<Range<u64>> {
    let mut rs: Vec<Range<u64>> = ranges.iter().filter(|r| r.start < r.end).cloned().collect();
    rs.sort_by_key(|r| r.start);
    let mut out: Vec<Range<u64>> = Vec::new();
    for r in rs {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// `nodes` (from a share) as a graph in `dst`: each node's bytes put again
/// there, a range as its own content (the content it is a range of may
/// hold bytes outside the share), and only the edges between the nodes.
/// A node that points at another object is refused, since that object is
/// not in the scope.
fn copy_nodes<S: Store, T: Store>(
    src: &Repo<S>,
    nodes: &[Node],
    dst: &mut Repo<T>,
) -> Result<Id, Refused> {
    use crate::{Arg, Target};
    let keys: std::collections::HashSet<[u8; 8]> = nodes
        .iter()
        .map(|n| crate::object::node_key(&n.label))
        .collect();
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        let target = match &n.target {
            Target::None => Target::None,
            Target::Content(Arg::Whole(id)) => {
                Target::Content(Arg::Whole(dst.put_content(&src.read_content(id)?)?.id))
            }
            Target::Content(Arg::Range { id, start, len }) => {
                let bytes = src.read_range(id, *start, *len)?.0;
                Target::Content(Arg::Whole(dst.put_content(&bytes)?.id))
            }
            _ => return Err(Refused::NotMergeable),
        };
        let mut edges = n.edges.clone();
        edges.retain(|e| keys.contains(&e.to));
        out.push(Node {
            label: n.label.clone(),
            target,
            edges,
        });
    }
    Ok(dst.put_graph(out)?)
}

/// Tree `root` with the entry at `parts` set to `(kind, id)`, making
/// directories as needed.
fn put_entry<T: Store>(
    dst: &mut Repo<T>,
    root: Id,
    parts: &[&str],
    kind: Kind,
    id: Id,
) -> Result<Id, Refused> {
    let mut entries = match dst.get(&root)? {
        Object::Tree(es) => es,
        _ => return Err(Refused::Store(format!("{root}: not a tree"))),
    };
    let (name, rest) = (parts[0], &parts[1..]);
    let (kind, id) = if rest.is_empty() {
        (kind, id)
    } else {
        let sub = match entries.iter().find(|e| e.name == name) {
            Some(e) if e.kind == Kind::Tree => e.id,
            _ => dst.edit_tree(None, &[])?,
        };
        (Kind::Tree, put_entry(dst, sub, rest, kind, id)?)
    };
    entries.retain(|e| e.name != name);
    entries.push(crate::TreeEntry {
        name: name.to_string(),
        kind,
        id,
    });
    Ok(dst.put(&Object::tree(entries)?)?)
}
