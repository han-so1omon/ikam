//! Path edits on trees: put, remove, rename and copy files and directories
//! by path, writing only the trees on the changed paths (E110).
//!
//! Laws (tested in tests/worktree.rs):
//! - after `edit_tree(root, edits)`, every path reads what the edits, applied
//!   in order to the files of `root`, leave there, and nothing else exists;
//! - untouched subtrees keep their ids (nothing below them is rewritten);
//! - edits that leave the files as they were return `root` itself.

use std::collections::BTreeMap;

use crate::{Error, Id, Kind, Object, Repo, Store, TreeEntry};

/// One change to a tree, by `/`-separated path. Content is stored first
/// (`put_content`), so an edit names it by id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Create or replace the file at `path`, creating missing directories.
    Put { path: String, id: Id },
    /// Create an empty directory (and its parents) if missing.
    Mkdir { path: String },
    /// Remove the file or directory at `path`; missing paths are ignored.
    Remove { path: String },
    /// Move the file or directory at `from` to `to`, replacing what is there.
    Rename { from: String, to: String },
    /// Copy the file or directory at `from` to `to`, replacing what is there.
    Copy { from: String, to: String },
}

/// A tree loaded only along edited paths.
#[derive(Clone)]
enum Node {
    /// An entry as stored, not loaded.
    Stored(Kind, Id),
    /// A directory being edited.
    Dir(BTreeMap<String, Node>),
}

/// Split a path into its names, rejecting empty, `.` and `..` names.
pub fn split_path(path: &str) -> Result<Vec<&str>, Error> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() || parts.iter().any(|p| *p == "." || *p == "..") {
        return Err(Error::InvalidName(path.into()));
    }
    Ok(parts)
}

impl<S: Store> Repo<S> {
    /// Apply `edits` in order to tree `root` (`None`: an empty tree) and
    /// store the resulting tree. Only directories on edited paths are read
    /// and rewritten.
    pub fn edit_tree(&mut self, root: Option<Id>, edits: &[Edit]) -> Result<Id, Error> {
        let began = self.begin_memo();
        let hinting = self.begin_hints();
        let out = self.edit_tree_inner(root, edits);
        self.end_hints(hinting);
        self.end_memo(began);
        out
    }

    fn edit_tree_inner(&mut self, root: Option<Id>, edits: &[Edit]) -> Result<Id, Error> {
        let mut top = match root {
            Some(id) => self.load_dir(&id)?,
            None => BTreeMap::new(),
        };
        for e in edits {
            match e {
                Edit::Put { path, id } => {
                    let parts = split_path(path)?;
                    let (dir, name) = self.dir_for(&mut top, &parts, true)?;
                    dir.insert(name.into(), Node::Stored(Kind::File, *id));
                }
                Edit::Mkdir { path } => {
                    let parts = split_path(path)?;
                    let (dir, name) = self.dir_for(&mut top, &parts, true)?;
                    match dir.get(name) {
                        Some(Node::Dir(_) | Node::Stored(Kind::Tree, _)) => {}
                        Some(_) => return Err(Error::InvalidName(path.clone())),
                        None => {
                            dir.insert(name.into(), Node::Dir(BTreeMap::new()));
                        }
                    }
                }
                Edit::Remove { path } => {
                    let parts = split_path(path)?;
                    if let Ok((dir, name)) = self.dir_for(&mut top, &parts, false) {
                        dir.remove(name);
                    }
                }
                Edit::Rename { from, to } | Edit::Copy { from, to } => {
                    let src = split_path(from)?;
                    let dst = split_path(to)?;
                    if dst.starts_with(&src) && dst.len() > src.len() {
                        return Err(Error::InvalidName(to.clone())); // into itself
                    }
                    let (dir, name) = self
                        .dir_for(&mut top, &src, false)
                        .map_err(|_| Error::InvalidName(from.clone()))?;
                    let node = if matches!(e, Edit::Rename { .. }) {
                        dir.remove(name)
                    } else {
                        dir.get(name).cloned()
                    }
                    .ok_or_else(|| Error::InvalidName(from.clone()))?;
                    let (dir, name) = self.dir_for(&mut top, &dst, true)?;
                    dir.insert(name.into(), node);
                }
            }
        }
        self.write_dir(top)
    }

    fn load_dir(&self, id: &Id) -> Result<BTreeMap<String, Node>, Error> {
        let Object::Tree(entries) = self.get(id)? else {
            return Err(Error::WrongKind(*id));
        };
        Ok(entries
            .into_iter()
            .map(|e| (e.name, Node::Stored(e.kind, e.id)))
            .collect())
    }

    /// The directory holding the last name of `parts`, loading (and with
    /// `create`, making) the directories above it. A file in the way is
    /// replaced only when creating.
    fn dir_for<'a, 'p>(
        &self,
        mut dir: &'a mut BTreeMap<String, Node>,
        parts: &[&'p str],
        create: bool,
    ) -> Result<(&'a mut BTreeMap<String, Node>, &'p str), Error> {
        let (name, parents) = parts.split_last().expect("split_path is never empty");
        for p in parents {
            let entry = match dir.get(*p) {
                Some(Node::Dir(_)) => None,
                Some(Node::Stored(Kind::Tree, id)) => Some(self.load_dir(id)?),
                _ if create => Some(BTreeMap::new()),
                _ => return Err(Error::InvalidName((*p).into())),
            };
            if let Some(loaded) = entry {
                dir.insert((*p).into(), Node::Dir(loaded));
            }
            let Some(Node::Dir(next)) = dir.get_mut(*p) else {
                unreachable!()
            };
            dir = next;
        }
        Ok((dir, name))
    }

    fn write_dir(&mut self, dir: BTreeMap<String, Node>) -> Result<Id, Error> {
        let mut entries = Vec::with_capacity(dir.len());
        for (name, node) in dir {
            let (kind, id) = match node {
                Node::Stored(kind, id) => (kind, id),
                Node::Dir(sub) => (Kind::Tree, self.write_dir(sub)?),
            };
            entries.push(TreeEntry { name, kind, id });
        }
        self.put(&Object::tree(entries)?)
    }
}
