//! E143: with `read_memo` on, a template candidate uses the bytes its
//! template search just read instead of reading the stored copy again.

use std::collections::HashMap;
use std::sync::Mutex;

use ikam_kernel::{Error, Id, MemStore, Repo, Store};

#[derive(Default)]
struct Counting {
    inner: MemStore,
    reads: Mutex<HashMap<Id, u32>>,
}

impl Store for Counting {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        self.inner.write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        *self.reads.lock().unwrap().entry(*id).or_default() += 1;
        self.inner.read(id)
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

/// An invoice from one generator: long shared boilerplate, short fields.
fn invoice(n: u32) -> Vec<u8> {
    let field = |k: u32| {
        format!(
            "{:x}",
            (n.wrapping_mul(2654435761) ^ k.wrapping_mul(40503)) % 100_000
        )
    };
    let mut out = String::new();
    for k in 0..20 {
        out.push_str(&format!(
            "<section id=\"{k}\"><label>Line item {k}: standard terms and conditions apply; see master agreement clause {k}.</label><value>{}</value></section>\n",
            field(k)
        ));
    }
    out.into_bytes()
}

/// Stores a few invoices, then on a fresh handle (memo on or off) builds the
/// seed index and stores one more. Returns the new id, the bytes written by
/// the last put, how often each stored object was read by it, and every
/// stored object.
type Run = (Id, usize, HashMap<Id, u32>, Vec<(Id, Vec<u8>)>);

fn run(memo: bool) -> Run {
    let mut first = Repo::new(MemStore::default());
    for n in 0..6 {
        first.put_content(&invoice(n)).unwrap();
    }
    let mut store = Counting::default();
    for id in first.store().ids().unwrap() {
        store.write(id, &first.store().read(&id).unwrap()).unwrap();
    }
    let mut repo = Repo::new(store);
    repo.read_memo = memo;
    // Build the seed index outside the measured put.
    repo.put_content(b"something unrelated to build the index")
        .unwrap();
    repo.store().reads.lock().unwrap().clear();
    let before = repo.bytes_written;
    let id = repo.put_content(&invoice(100)).unwrap().id;
    let written = repo.bytes_written - before;
    let reads = repo.store().reads.lock().unwrap().clone();
    assert_eq!(repo.read_content(&id).unwrap(), invoice(100));
    assert!(repo.fsck().unwrap().is_empty());
    let mut objects: Vec<(Id, Vec<u8>)> = repo
        .store()
        .ids()
        .unwrap()
        .into_iter()
        .map(|i| (i, repo.store().inner.read(&i).unwrap()))
        .collect();
    objects.sort();
    (id, written, reads, objects)
}

#[test]
fn a_template_candidate_reads_its_template_once() {
    let (id_off, written_off, reads_off, objects_off) = run(false);
    let (id_on, written_on, reads_on, objects_on) = run(true);
    // The same result, byte for byte.
    assert_eq!(id_on, id_off);
    assert_eq!(written_on, written_off);
    assert_eq!(objects_on, objects_off);
    // Without the memo the template is read twice (once as a template, once
    // as the content a fill would replace); with it, every object once.
    assert!(reads_off.values().any(|&n| n > 1), "{reads_off:?}");
    assert!(reads_on.values().all(|&n| n == 1), "{reads_on:?}");
    assert!(reads_on.values().sum::<u32>() < reads_off.values().sum::<u32>());
}
