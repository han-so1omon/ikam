//! Fragment renders (E155): a file stored as `render(f1, ..., fn)` over its
//! fragments, so that one edit to a fragment reaches every file that
//! renders it without re-planning any of them.
//!
//! `render` concatenates its arguments, like `concat`, but it is a fact,
//! not a storage plan: repack keeps it (and may drop the file's own bytes),
//! so the file stays addressable by its fragments. Fragments are ordinary
//! content, stored and deduplicated by `put_content`.
//!
//! Cutting is structure-aware (E153): a fragment is a paragraph block,
//! ending after a run of blank lines, cut at a line boundary past 16 KiB.
//! A file id is still the BLAKE3 of its bytes, so a propagated edit hashes
//! every file it reaches once; it never runs the matcher on them.
//!
//! Laws (tests/fragment.rs): every file reads back as put; after
//! `propagate`, every file that rendered the old fragment reads as its old
//! bytes with that fragment's bytes replaced, and nothing else changes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use crate::grant::inside;
use crate::{Arg, Denied, Edit, Error, Grant, Id, Kind, Object, Repo, Store, func};

/// Longest block before it is cut at the next line boundary.
const MAX_BLOCK: usize = 16 * 1024;

/// Paragraph blocks of `b`, covering it exactly, in order.
pub fn blocks(b: &[u8]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let (mut s, mut i) = (0, 0);
    while i < b.len() {
        if b[i] == b'\n' {
            let mut j = i + 1;
            let blank = j < b.len() && (b[j] == b'\n' || b[j..].starts_with(b"\r\n"));
            if blank || j - s > MAX_BLOCK {
                while j < b.len() && (b[j] == b'\n' || b[j] == b'\r') {
                    j += 1;
                }
                out.push(s..j);
                (s, i) = (j, j);
                continue;
            }
        }
        i += 1;
    }
    if s < b.len() {
        out.push(s..b.len());
    }
    out
}

/// Which files (by path) render each fragment: a projection of a tree and
/// the ledger, rebuilt on demand and updated by `propagate`.
#[derive(Clone, Debug, Default)]
pub struct FragmentIndex {
    pub by_fragment: HashMap<Id, Vec<String>>,
    /// Each file's id and the fragments it renders (none if not a render).
    pub files: BTreeMap<String, (Id, Vec<Id>)>,
    /// The blocks of each fragment that holds more than one (E157), so a
    /// block inside a run is found too. Blocks are indexed in `by_fragment`.
    pub inner: HashMap<Id, Vec<Id>>,
}

impl<S: Store> Repo<S> {
    /// Store `bytes` as fragments plus one `render` fact. With `shared`,
    /// only blocks whose ids it holds stand alone; each run of other blocks
    /// is one fragment (E157). Without it, every block is a fragment. A
    /// content of one fragment is stored as itself.
    pub fn put_fragmented(
        &mut self,
        bytes: &[u8],
        shared: Option<&HashSet<Id>>,
    ) -> Result<Id, Error> {
        let mut rs: Vec<Range<usize>> = Vec::new();
        let mut run_open = false;
        for r in blocks(bytes) {
            let alone = shared.is_none_or(|s| s.contains(&Id::of_content(&bytes[r.clone()])));
            match rs.last_mut() {
                Some(last) if run_open && !alone => last.end = r.end,
                _ => rs.push(r),
            }
            run_open = !alone;
        }
        if rs.len() < 2 {
            return Ok(self.put_content(bytes)?.id);
        }
        let mut args = Vec::with_capacity(rs.len());
        for r in rs {
            args.push(Arg::Whole(self.put_content(&bytes[r])?.id));
        }
        let put = self.put_derivation(bytes, func::render(), args)?;
        Ok(put
            .ok_or(Error::Exec("render does not reproduce its input".into()))?
            .id)
    }

    /// The fragments a file renders, if it is stored as a render.
    pub fn fragments_of(&self, id: &Id) -> Result<Option<Vec<Id>>, Error> {
        Ok(self
            .derivations(id)?
            .into_iter()
            .find(|d| d.func == func::render())
            .map(|d| d.args.iter().map(Arg::id).collect()))
    }

    /// The fragment index of every file under `tree`.
    pub fn fragment_index(&self, tree: &Id) -> Result<FragmentIndex, Error> {
        let mut ix = FragmentIndex::default();
        let mut stack = vec![(String::new(), *tree)];
        while let Some((prefix, t)) = stack.pop() {
            let Object::Tree(entries) = self.get(&t)? else {
                return Err(Error::WrongKind(t));
            };
            for e in entries {
                let path = if prefix.is_empty() {
                    e.name.clone()
                } else {
                    format!("{prefix}/{}", e.name)
                };
                match e.kind {
                    Kind::Tree => stack.push((path, e.id)),
                    _ => {
                        let frags = self.fragments_of(&e.id)?;
                        self.index_inner(&mut ix, frags.as_deref().unwrap_or(&[]))?;
                        ix.add(&path, e.id, frags);
                    }
                }
            }
        }
        Ok(ix)
    }

    /// Record the blocks of fragments not seen before.
    fn index_inner(&self, ix: &mut FragmentIndex, frags: &[Id]) -> Result<(), Error> {
        for f in frags {
            if ix.inner.contains_key(f) || ix.by_fragment.contains_key(f) {
                continue;
            }
            let b = self.read_content(f)?;
            let rs = blocks(&b);
            let ids = if rs.len() > 1 {
                rs.into_iter().map(|r| Id::of_content(&b[r])).collect()
            } else {
                vec![]
            };
            ix.inner.insert(*f, ids);
        }
        Ok(())
    }

    /// Replace fragment `old` by `new_bytes` in every file of `ix` that
    /// renders it: one new fragment, one new `render` fact per file, and the
    /// tree edits to apply. Files are hashed, never re-planned.
    pub fn propagate(
        &mut self,
        ix: &mut FragmentIndex,
        old: &Id,
        new_bytes: &[u8],
    ) -> Result<Vec<Edit>, Error> {
        self.propagate_granted(ix, old, new_bytes, None)?
            .map_err(|(path, d)| Error::Exec(format!("{path}: {d:?}")))
    }

    /// `propagate` under a goal's grant (E159). Every file the edit reaches
    /// must be granted at the version indexed, and each range the edit
    /// changes (the old fragment's bytes in that file) must lie inside its
    /// write ranges. The ranges come from the fragments' lengths, so no
    /// file is diffed. Every file is checked before anything is written: a
    /// refused edit writes nothing and names the first file refused.
    pub fn propagate_granted(
        &mut self,
        ix: &mut FragmentIndex,
        old: &Id,
        new_bytes: &[u8],
        grant: Option<&Grant>,
    ) -> Result<Result<Vec<Edit>, (String, Denied)>, Error> {
        let mut paths = ix.by_fragment.get(old).cloned().unwrap_or_default();
        paths.dedup();
        // Plan: each file's new fragments, and the ranges the edit changes.
        let mut plans: Vec<(String, Id, Vec<Part>)> = Vec::with_capacity(paths.len());
        for path in paths {
            let file = ix.files[&path].0;
            let frags = self.fragments_of(&file)?.ok_or(Error::WrongKind(file))?;
            let (mut at, mut changed, mut parts) = (0u64, Vec::new(), Vec::new());
            for f in &frags {
                let holds = ix.inner.get(f).is_some_and(|bs| bs.contains(old));
                let len = if f == old || holds {
                    let b = self.read_content(f)?;
                    if f == old {
                        changed.push(at..at + b.len() as u64);
                        parts.push(Part::New);
                    } else {
                        // A block inside a run: the run is rewritten.
                        let mut spliced = Vec::with_capacity(b.len());
                        for r in blocks(&b) {
                            if Id::of_content(&b[r.clone()]) == *old {
                                changed.push(at + r.start as u64..at + r.end as u64);
                                spliced.extend_from_slice(new_bytes);
                            } else {
                                spliced.extend_from_slice(&b[r]);
                            }
                        }
                        parts.push(Part::Splice(spliced));
                    }
                    b.len() as u64
                } else {
                    parts.push(Part::Keep(*f));
                    self.content_len(f)?
                };
                at += len;
            }
            if let Some(g) = grant {
                let Some(fg) = g.file(&path) else {
                    return Ok(Err((path, Denied::NoFile)));
                };
                if fg.id != file {
                    return Ok(Err((path, Denied::Stale)));
                }
                if let Some(r) = changed.iter().find(|r| !inside(&fg.write, r)) {
                    return Ok(Err((path, Denied::Outside(r.clone()))));
                }
            }
            plans.push((path, file, parts));
        }
        // Write.
        let new = self.put_content(new_bytes)?.id;
        let mut edits = Vec::with_capacity(plans.len());
        for (path, file, parts) in plans {
            let mut args = Vec::with_capacity(parts.len());
            for p in parts {
                args.push(Arg::Whole(match p {
                    Part::Keep(f) => f,
                    Part::New => new,
                    Part::Splice(b) => self.put_content(&b)?.id,
                }));
            }
            let (bytes, _) = self.evaluate(&func::render(), &args)?;
            let put = self.put_derivation(&bytes, func::render(), args)?;
            let out = put.ok_or(Error::Corrupt(file))?.id;
            ix.remove(&path);
            let frags = self.fragments_of(&out)?;
            self.index_inner(ix, frags.as_deref().unwrap_or(&[]))?;
            ix.add(&path, out, frags);
            edits.push(Edit::Put { path, id: out });
        }
        Ok(Ok(edits))
    }

    /// Bytes of a content, from its stored form or by rebuilding it.
    fn content_len(&self, id: &Id) -> Result<u64, Error> {
        Ok(self.read_content(id)?.len() as u64)
    }
}

/// One fragment of a planned render.
enum Part {
    Keep(Id),
    New,
    Splice(Vec<u8>),
}

impl FragmentIndex {
    fn add(&mut self, path: &str, id: Id, frags: Option<Vec<Id>>) {
        let frags = frags.unwrap_or_default();
        let keys: Vec<Id> = frags
            .iter()
            .flat_map(|f| std::iter::once(*f).chain(self.inner.get(f).cloned().unwrap_or_default()))
            .collect();
        for f in keys {
            let v = self.by_fragment.entry(f).or_default();
            if !v.iter().any(|p| p == path) {
                v.push(path.to_string());
            }
        }
        self.files.insert(path.to_string(), (id, frags));
    }

    fn remove(&mut self, path: &str) {
        for f in self.files.remove(path).map(|e| e.1).unwrap_or_default() {
            let inner = self.inner.get(&f).cloned().unwrap_or_default();
            for k in std::iter::once(f).chain(inner) {
                if let Some(v) = self.by_fragment.get_mut(&k) {
                    v.retain(|p| p != path);
                }
            }
        }
    }
}
