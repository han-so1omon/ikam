//! L0 object stores. Law: `get(put(o)) == o`; `put` is idempotent; `get`
//! re-hashes what it reads, so corruption is detected rather than returned.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::{Error, Id, Object};

pub trait Store {
    /// Store pre-encoded bytes under `id`. Returns true if newly written.
    /// Callers must pass `id == Id::of(encoded)`; use [`Store::put`].
    fn put_encoded(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error>;

    /// Raw encoded bytes for `id`, unverified.
    fn get_encoded(&self, id: &Id) -> Result<Vec<u8>, Error>;

    fn put(&mut self, obj: &Object) -> Result<(Id, bool), Error> {
        let encoded = obj.encode();
        let id = Id::of(&encoded);
        let fresh = self.put_encoded(id, &encoded)?;
        Ok((id, fresh))
    }

    fn get(&self, id: &Id) -> Result<Object, Error> {
        let encoded = self.get_encoded(id)?;
        if Id::of(&encoded) != *id {
            return Err(Error::Corrupt(*id));
        }
        Object::decode(&encoded)
    }
}

#[derive(Default)]
pub struct MemStore {
    objects: HashMap<Id, Vec<u8>>,
}

impl MemStore {
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

impl Store for MemStore {
    fn put_encoded(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        if self.objects.contains_key(&id) {
            return Ok(false);
        }
        self.objects.insert(id, encoded.to_vec());
        Ok(true)
    }

    fn get_encoded(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.objects.get(id).cloned().ok_or(Error::NotFound(*id))
    }
}

/// Git-style loose objects: `<root>/objects/<hex[..2]>/<hex[2..]>`.
pub struct FsStore {
    root: PathBuf,
}

impl FsStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<FsStore, Error> {
        let root = root.into();
        fs::create_dir_all(root.join("objects"))?;
        Ok(FsStore { root })
    }

    fn path(&self, id: &Id) -> PathBuf {
        let hex = id.to_hex();
        self.root.join("objects").join(&hex[..2]).join(&hex[2..])
    }
}

impl Store for FsStore {
    fn put_encoded(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        let path = self.path(&id);
        if path.exists() {
            return Ok(false);
        }
        let dir = path.parent().unwrap();
        fs::create_dir_all(dir)?;
        // Write-then-rename so a crash never leaves a partial object in place.
        let tmp = dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            &id.to_hex()[..16]
        ));
        let mut file = fs::File::create(&tmp)?;
        file.write_all(encoded)?;
        file.sync_all()?;
        fs::rename(&tmp, &path)?;
        Ok(true)
    }

    fn get_encoded(&self, id: &Id) -> Result<Vec<u8>, Error> {
        fs::read(self.path(id)).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::NotFound(*id),
            _ => Error::Io(e),
        })
    }
}
