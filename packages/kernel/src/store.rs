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
    /// The first `n` bytes of the encoding stored under `id`, or all of it
    /// if it is shorter. A store that can read part of an object (a file, a
    /// range GET) fetches only those bytes; the default reads it all and
    /// cuts it (E135).
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        let mut out = self.read(id)?;
        out.truncate(n);
        Ok(out)
    }
    /// Length of the encoding stored under `id` (E147). A store that keeps
    /// lengths (a file's size, a pack's index) answers without reading the
    /// object; the default reads it.
    fn stored_len(&self, id: &Id) -> Result<usize, Error> {
        self.read(id).map(|b| b.len())
    }
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

/// A store several `Repo`s share (E232): each call takes a lock for that
/// one object or ref only, so readers working from fixed roots interleave
/// with a writer object by object instead of waiting for its whole append.
/// Objects are immutable once written and a reader holds the roots it
/// started from, so what it reads does not change under it. Clones share
/// the store.
pub struct SharedStore<S>(std::sync::Arc<std::sync::RwLock<S>>);

impl<S> Clone for SharedStore<S> {
    fn clone(&self) -> Self {
        SharedStore(self.0.clone())
    }
}

impl<S: Store> SharedStore<S> {
    pub fn new(store: S) -> SharedStore<S> {
        SharedStore(std::sync::Arc::new(std::sync::RwLock::new(store)))
    }

    fn r(&self) -> std::sync::RwLockReadGuard<'_, S> {
        self.0.read().unwrap_or_else(|e| e.into_inner())
    }

    fn w(&self) -> std::sync::RwLockWriteGuard<'_, S> {
        self.0.write().unwrap_or_else(|e| e.into_inner())
    }
}

impl<S: Store> Store for SharedStore<S> {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        self.w().write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.w().replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.r().read(id)
    }
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        self.r().read_head(id, n)
    }
    fn stored_len(&self, id: &Id) -> Result<usize, Error> {
        self.r().stored_len(id)
    }
    fn has(&self, id: &Id) -> bool {
        self.r().has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.r().ids()
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        self.r().ids_with_prefix(prefix)
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        self.w().delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.r().get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        self.w().set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.r().refs()
    }
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

    fn stored_len(&self, id: &Id) -> Result<usize, Error> {
        self.objects
            .get(id)
            .map(Vec::len)
            .ok_or(Error::NotFound(*id))
    }

    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        let bytes = self.objects.get(id).ok_or(Error::NotFound(*id))?;
        Ok(bytes[..n.min(bytes.len())].to_vec())
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
    /// E227: with `open_deferred`, objects written since the last ref move,
    /// under their temporary names. Made durable and renamed into place
    /// before the next ref move (`sync`).
    pending: Option<BTreeMap<Id, PathBuf>>,
}

impl FsStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<FsStore, Error> {
        let root = root.into();
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join("refs"))?;
        Ok(FsStore {
            root,
            pending: None,
        })
    }

    /// The same layout, but an object is made durable only when a ref next
    /// moves, so a burst of writes costs one round of syncs instead of one
    /// sync each (E227). Until then it waits under a temporary name in its
    /// directory: a crash leaves only temporary files (`.pending-*`, never
    /// read), never a partial object under its id, and no ref can name an
    /// object that is not durable.
    pub fn open_deferred(root: impl Into<PathBuf>) -> Result<FsStore, Error> {
        let mut s = FsStore::open(root)?;
        s.pending = Some(BTreeMap::new());
        Ok(s)
    }

    fn path(&self, id: &Id) -> PathBuf {
        let hex = id.to_hex();
        self.root.join("objects").join(&hex[..2]).join(&hex[2..])
    }

    /// Where `id`'s bytes are now: its waiting file, or its own path.
    fn located(&self, id: &Id) -> PathBuf {
        match self.pending.as_ref().and_then(|p| p.get(id)) {
            Some(tmp) => tmp.clone(),
            None => self.path(id),
        }
    }

    /// Make every waiting object durable and move it to its own path
    /// (E227). Nothing to do for a store opened with `open`. On Apple
    /// systems each file gets a plain fsync and the ref's own sync, a full
    /// flush of the drive, makes them durable together.
    pub fn sync(&mut self) -> Result<(), Error> {
        let Some(pending) = self.pending.as_mut() else {
            return Ok(());
        };
        // Linux: one syncfs writes every waiting file and directory entry
        // (renames included) instead of one fsync each.
        #[cfg(target_os = "linux")]
        if !pending.is_empty() {
            let mut moved = Vec::new();
            while let Some((id, tmp)) = pending.pop_first() {
                let hex = id.to_hex();
                let path = self.root.join("objects").join(&hex[..2]).join(&hex[2..]);
                if let Err(e) = fs::rename(&tmp, &path) {
                    pending.insert(id, tmp);
                    return Err(e.into());
                }
                moved.push(path);
            }
            use std::os::fd::AsRawFd;
            let root = fs::File::open(&self.root)?;
            // SAFETY: syncfs on an open descriptor held for the call.
            if unsafe { libc::syncfs(root.as_raw_fd()) } != 0 {
                // Not known durable: remove them (absent before `write`),
                // so no later write skips one that a crash may have cut.
                let e = io::Error::last_os_error();
                for path in moved {
                    let _ = fs::remove_file(path);
                }
                return Err(e.into());
            }
            return Ok(());
        }
        let mut dirs = std::collections::BTreeSet::new();
        while let Some((id, tmp)) = pending.pop_first() {
            let done = push_to_disk(&fs::File::open(&tmp)?).and_then(|()| {
                let hex = id.to_hex();
                let path = self.root.join("objects").join(&hex[..2]).join(&hex[2..]);
                fs::rename(&tmp, &path)?;
                dirs.insert(path.parent().unwrap().to_path_buf());
                Ok(())
            });
            if let Err(e) = done {
                pending.insert(id, tmp); // still waiting; the ref does not move
                return Err(e.into());
            }
        }
        for dir in dirs {
            push_to_disk(&fs::File::open(dir)?)?;
        }
        Ok(())
    }
}

impl Drop for FsStore {
    /// Objects written but never named by a ref are still kept, as with
    /// `open`: a later process may name them (`ikam put`, then a commit).
    fn drop(&mut self) {
        let _ = self.sync();
    }
}

/// Write a file's data to the drive: `sync_all`, except on Apple systems,
/// where `sync_all` also flushes the drive's cache (F_FULLFSYNC) and a plain
/// fsync is enough before a later full flush.
fn push_to_disk(file: &fs::File) -> io::Result<()> {
    #[cfg(target_vendor = "apple")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: fsync on an open descriptor this function borrows.
        if unsafe { libc::fsync(file.as_raw_fd()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(target_vendor = "apple"))]
    file.sync_all()
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
        if self.has(&id) {
            return Ok(false);
        }
        let path = self.path(&id);
        match self.pending.as_mut() {
            None => write_atomic(&path, encoded)?,
            Some(pending) => {
                let dir = path.parent().unwrap();
                fs::create_dir_all(dir)?;
                let name = path.file_name().unwrap().to_string_lossy();
                // Unique per handle and write, so two handles never share one.
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let tmp = dir.join(format!(".pending-{}-{n}-{name}", std::process::id()));
                fs::write(&tmp, encoded)?;
                pending.insert(id, tmp);
            }
        }
        Ok(true)
    }

    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        // A replacement is durable at once, as with `open` (rename is atomic
        // over an old file); a waiting copy of the same object is let go.
        if let Some(tmp) = self.pending.as_mut().and_then(|p| p.remove(&id)) {
            let _ = fs::remove_file(tmp);
        }
        write_atomic(&self.path(&id), encoded)
    }

    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        not_found_as(fs::read(self.located(id)), || Error::NotFound(*id))
    }

    fn stored_len(&self, id: &Id) -> Result<usize, Error> {
        not_found_as(fs::metadata(self.located(id)), || Error::NotFound(*id))
            .map(|m| m.len() as usize)
    }

    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        use std::io::Read;
        let file = not_found_as(fs::File::open(self.located(id)), || Error::NotFound(*id))?;
        let mut out = Vec::with_capacity(n);
        file.take(n as u64).read_to_end(&mut out)?;
        Ok(out)
    }

    fn has(&self, id: &Id) -> bool {
        self.pending.as_ref().is_some_and(|p| p.contains_key(id)) || self.path(id).exists()
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
        if let Some(pending) = &self.pending {
            out.extend(pending.keys());
        }
        // Directory order depends on the filesystem (ext4 hashes names with a
        // per-filesystem seed, APFS sorts them). Callers that pick among
        // equal choices in id order then store the same encodings everywhere,
        // as from a `MemStore` (E136).
        out.sort_unstable();
        Ok(out)
    }

    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        let Some(first) = prefix.first() else {
            return self.ids();
        };
        let dir = self.root.join("objects").join(format!("{first:02x}"));
        let entries = match fs::read_dir(dir) {
            Ok(entries) => Some(entries),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for file in entries.into_iter().flatten() {
            let name = file?.file_name().to_string_lossy().into_owned();
            if let Ok(id) = format!("{first:02x}{name}").parse::<Id>()
                && id.as_bytes().starts_with(prefix)
            {
                out.push(id);
            }
        }
        if let Some(pending) = &self.pending {
            out.extend(
                pending
                    .keys()
                    .filter(|id| id.as_bytes().starts_with(prefix)),
            );
        }
        out.sort_unstable(); // as `ids` (E136)
        Ok(out)
    }

    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        if let Some(tmp) = self.pending.as_mut().and_then(|p| p.remove(id)) {
            fs::remove_file(tmp)?;
            return Ok(());
        }
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
        self.sync()?; // E227: what the ref names is durable before it moves
        let path = self.root.join("refs").join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        // The lock file serializes writers through an OS lock on it, not its
        // existence: a writer killed mid-update leaves the file behind, and
        // an existence lock would refuse every later update of the ref
        // (E122). The OS drops the lock with the process.
        let lock = path.with_file_name(format!(
            "{}.lock",
            path.file_name().unwrap().to_string_lossy()
        ));
        let conflict = || Error::RefConflict(name.into());
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock)
            .map_err(|_| conflict())?;
        file.try_lock().map_err(|_| conflict())?;
        // The file locked must still be the one at the lock path: a writer
        // that opened it just before another renamed it into place holds
        // the ref file itself.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let held = file.metadata()?.ino();
            if fs::metadata(&lock).map(|m| m.ino()).ok() != Some(held) {
                return Err(conflict());
            }
        }
        file.set_len(0)?;
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
