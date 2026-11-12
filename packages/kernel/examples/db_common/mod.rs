//! Shared by the E240 to E244 harnesses: a store that counts reads, and a
//! generated table. Fixtures are synthetic; no real data is used.

#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use ikam_kernel::{Error, Id, MemStore, Store};

#[derive(Default)]
pub struct Counting {
    pub inner: MemStore,
    reads: AtomicU64,
    bytes: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Io {
    pub reads: u64,
    pub bytes: u64,
}

impl Counting {
    pub fn take(&self) -> Io {
        Io {
            reads: self.reads.swap(0, Relaxed),
            bytes: self.bytes.swap(0, Relaxed),
        }
    }

    /// A fresh counting store holding the same objects and refs.
    pub fn copy(&self) -> Counting {
        let mut inner = MemStore::default();
        for id in self.inner.ids().unwrap() {
            inner.write(id, &self.inner.read(&id).unwrap()).unwrap();
        }
        for (name, id) in self.inner.refs().unwrap() {
            inner.set_ref(&name, None, id).unwrap();
        }
        Counting {
            inner,
            ..Default::default()
        }
    }

    /// Stored bytes of every object.
    pub fn stored(&self) -> usize {
        let ids = self.inner.ids().unwrap();
        ids.iter()
            .map(|id| self.inner.read(id).unwrap().len())
            .sum()
    }
}

impl Store for Counting {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        self.inner.write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        let out = self.inner.read(id)?;
        self.reads.fetch_add(1, Relaxed);
        self.bytes.fetch_add(out.len() as u64, Relaxed);
        Ok(out)
    }
    fn has(&self, id: &Id) -> bool {
        self.inner.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.inner.ids()
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        self.inner.ids_with_prefix(prefix)
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        self.inner.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.inner.get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        self.inner.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.inner.refs()
    }
}

/// A deterministic generator.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// Row `i` of a generated append-only table: a time, a sensor id, a
/// reading with two decimals, a count and a short status word, as CSV.
pub fn row(rng: &mut Lcg, i: u64) -> String {
    let status = ["ok", "ok", "ok", "warn", "idle"][rng.next() as usize % 5];
    let reading = 2000 + (i % 977) as i64 * 3 - (rng.next() % 50) as i64;
    format!(
        "{},s{:03},{}.{:02},{},{}",
        1_700_000_000 + i * 60,
        rng.next() % 64,
        reading / 100,
        reading % 100,
        rng.next() % 5000,
        status
    )
}
