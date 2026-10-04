//! Raw storage: encoded objects by id, plus refs (the only mutable state).
//! Stores do no verification; `Repo` checks hashes on every read.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::{Error, Id};

pub trait Store {
    /// Write `encoded` under `id` unless present. Returns true if newly written.
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error>;
    /// Atomically replace the encoding stored under `id` (written or not).
    /// Callers must only swap encodings of the same object or content.
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error>;
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error>;
    fn has(&self, id: &Id) -> bool;
    fn ids(&self) -> Result<Vec<Id>, Error>;
    /// Stored ids starting with `prefix` (an abbreviated id, as in git).
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error>;
    fn delete(&mut self, id: &Id) -> Result<(), Error>;

    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error>;
    /// Compare-and-swap: move `name` to `new` only if it currently equals `expected`.
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error>;
    fn refs(&self) -> Result<Vec<(String, Id)>, Error>;
}

/// Ref names are `/`-separated segments of `[A-Za-z0-9._-]`, none starting with `.`.
pub fn check_ref_name(name: &str) -> Result<(), Error> {
    let ok = name.split('/').all(|seg| {
        !seg.is_empty()
            && !seg.starts_with('.')
            && !seg.ends_with(".lock")
            && seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    });
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidName(name.into()))
    }
}

#[derive(Default)]
pub struct MemStore {
    objects: BTreeMap<Id, Vec<u8>>,
    refs: HashMap<String, Id>,
}

impl Store for MemStore {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        if self.objects.contains_key(&id) {
            return Ok(false);
        }
        self.objects.insert(id, encoded.to_vec());
        Ok(true)
    }

    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.objects.insert(id, encoded.to_vec());
        Ok(())
    }

    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.objects.get(id).cloned().ok_or(Error::NotFound(*id))
    }

    fn has(&self, id: &Id) -> bool {
        self.objects.contains_key(id)
    }

    fn ids(&self) -> Result<Vec<Id>, Error> {
        Ok(self.objects.keys().copied().collect())
    }

    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        Ok(self
            .objects
            .range(Id::from_bytes(prefix_start(prefix))..)
            .map(|(id, _)| *id)
            .take_while(|id| id.as_bytes().starts_with(prefix))
            .collect())
    }

    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        self.objects.remove(id);
        Ok(())
    }

    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        check_ref_name(name)?;
        Ok(self.refs.get(name).copied())
    }

    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        if self.get_ref(name)? != expected {
            return Err(Error::RefConflict(name.into()));
        }
        self.refs.insert(name.into(), new);
        Ok(())
    }

    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        Ok(self.refs.iter().map(|(k, v)| (k.clone(), *v)).collect())
    }
}

/// Git-style layout: `objects/<hex[..2]>/<hex[2..]>` and `refs/<name>`.
pub struct FsStore {
    root: PathBuf,
}

impl FsStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<FsStore, Error> {
        let root = root.into();
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join("refs"))?;
        Ok(FsStore { root })
    }

    fn path(&self, id: &Id) -> PathBuf {
        let hex = id.to_hex();
        self.root.join("objects").join(&hex[..2]).join(&hex[2..])
    }
}

/// The smallest id starting with `prefix`.
fn prefix_start(prefix: &[u8]) -> [u8; 32] {
    let mut start = [0u8; 32];
    let n = prefix.len().min(32);
    start[..n].copy_from_slice(&prefix[..n]);
    start
}

/// Write-then-rename so a crash never leaves a partial file in place.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), Error> {
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".tmp-{}", std::process::id()));
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn not_found_as<T>(r: io::Result<T>, missing: impl FnOnce() -> Error) -> Result<T, Error> {
    r.map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            missing()
        } else {
            Error::Io(e)
        }
    })
}

impl Store for FsStore {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        let path = self.path(&id);
        if path.exists() {
            return Ok(false);
        }
        write_atomic(&path, encoded)?;
        Ok(true)
    }

    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        write_atomic(&self.path(&id), encoded) // rename is atomic over an old file
    }

    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        not_found_as(fs::read(self.path(id)), || Error::NotFound(*id))
    }

    fn has(&self, id: &Id) -> bool {
        self.path(id).exists()
    }

    fn ids(&self) -> Result<Vec<Id>, Error> {
        let mut out = Vec::new();
        for dir in fs::read_dir(self.root.join("objects"))? {
            let dir = dir?;
            let prefix = dir.file_name().to_string_lossy().into_owned();
            for file in fs::read_dir(dir.path())? {
                let name = file?.file_name().to_string_lossy().into_owned();
                if let Ok(id) = format!("{prefix}{name}").parse() {
                    out.push(id);
                }
            }
        }
        Ok(out)
    }

    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        let Some(first) = prefix.first() else {
            return self.ids();
        };
        let dir = self.root.join("objects").join(format!("{first:02x}"));
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for file in entries {
            let name = file?.file_name().to_string_lossy().into_owned();
            if let Ok(id) = format!("{first:02x}{name}").parse::<Id>()
                && id.as_bytes().starts_with(prefix)
            {
                out.push(id);
            }
        }
        Ok(out)
    }

    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        not_found_as(fs::remove_file(self.path(id)), || Error::NotFound(*id))
    }

    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        check_ref_name(name)?;
        match fs::read_to_string(self.root.join("refs").join(name)) {
            Ok(s) => Ok(Some(s.trim().parse()?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        check_ref_name(name)?;
        let path = self.root.join("refs").join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        // The lock file serializes writers; create_new fails if another holds it.
        let lock = path.with_file_name(format!(
            "{}.lock",
            path.file_name().unwrap().to_string_lossy()
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .map_err(|_| Error::RefConflict(name.into()))?;
        let result = (|| {
            if self.get_ref(name)? != expected {
                return Err(Error::RefConflict(name.into()));
            }
            file.write_all(format!("{new}\n").as_bytes())?;
            file.sync_all()?;
            fs::rename(&lock, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&lock);
        }
        result
    }

    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        let mut out = Vec::new();
        let mut stack = vec![self.root.join("refs")];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(rel) = path.strip_prefix(self.root.join("refs")) {
                    let name = rel.to_string_lossy().into_owned();
                    if let Ok(Some(id)) = self.get_ref(&name) {
                        out.push((name, id));
                    }
                }
            }
        }
        Ok(out)
    }
}
