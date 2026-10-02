//! Directory trees <-> tree objects. Regular files and directories only;
//! symlinks and special files are skipped, and permissions are not recorded.

use std::fs;
use std::path::Path;

use crate::{Error, Id, Kind, Object, Repo, Store, TreeEntry};

/// Store the directory at `dir` and return its tree id. Entries whose path
/// equals `skip` (e.g. the store itself) are left out.
pub fn snapshot<S: Store>(repo: &mut Repo<S>, dir: &Path, skip: &Path) -> Result<Id, Error> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if same_path(&path, skip) {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|n| Error::InvalidName(n.to_string_lossy().into()))?;
        let ty = entry.file_type()?;
        let (kind, id) = if ty.is_dir() {
            (Kind::Tree, snapshot(repo, &path, skip)?)
        } else if ty.is_file() {
            (Kind::File, repo.put_content(&fs::read(&path)?)?.id)
        } else {
            continue;
        };
        entries.push(TreeEntry { name, kind, id });
    }
    repo.put(&Object::tree(entries)?)
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Write tree `id` into `dest`, which must not exist yet.
pub fn checkout<S: Store>(repo: &Repo<S>, id: &Id, dest: &Path) -> Result<(), Error> {
    fs::create_dir(dest)?;
    let Object::Tree(entries) = repo.get(id)? else {
        return Err(Error::WrongKind(*id));
    };
    for e in entries {
        let path = dest.join(&e.name);
        match e.kind {
            Kind::Tree => checkout(repo, &e.id, &path)?,
            Kind::File => fs::write(&path, repo.read_content(&e.id)?)?,
        }
    }
    Ok(())
}

/// Tree id of a commit, or the id itself if it already names a tree.
pub fn tree_of<S: Store>(repo: &Repo<S>, id: &Id) -> Result<Id, Error> {
    match repo.get(id)? {
        Object::Commit(c) => Ok(c.tree),
        Object::Tree(_) => Ok(*id),
        _ => Err(Error::WrongKind(*id)),
    }
}
