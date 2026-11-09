//! E145: a write batch keeps what its writes read, and can be handed bytes
//! the caller already holds; it stores exactly what plain writes store.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use ikam_kernel::{Error, Id, MemStore, Repo, Store};

#[derive(Default)]
struct Counting {
    inner: MemStore,
    reads: AtomicU64,
}

impl Store for Counting {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        self.inner.write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Relaxed);
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

fn text(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed;
    let mut out = Vec::new();
    while out.len() < n {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        out.extend_from_slice(format!("line {} says {}\n", out.len(), x >> 40).as_bytes());
    }
    out
}

fn edited(b: &[u8], k: u8) -> Vec<u8> {
    let mut v = b.to_vec();
    v.splice(3000..3000, [b'Q', b'0' + k]);
    v
}

enum Hand {
    Nothing,
    Bytes,
    WrongBytes,
}

/// Three files on a fresh handle scoped to them; read all three in a
/// session, then write an edit of each in one batch. Returns the store
/// reads of the writes and every stored object.
fn run(batch: bool, hand: Hand) -> (u64, Vec<(Id, Vec<u8>)>) {
    let mut first = Repo::new(MemStore::default());
    let files: Vec<Vec<u8>> = (0..3).map(|i| text(i, 8000)).collect();
    let ids: Vec<Id> = files
        .iter()
        .map(|f| first.put_content(f).unwrap().id)
        .collect();
    let mut store = Counting::default();
    for id in first.store().ids().unwrap() {
        store.write(id, &first.store().read(&id).unwrap()).unwrap();
    }
    let mut repo = Repo::new(store);
    repo.read_memo = true;
    repo.set_seed_scope(Some(ids.clone()));
    let s = repo.session(1 << 20);
    for id in &ids {
        s.read_content(id).unwrap();
    }
    let held = s.into_contents();
    assert_eq!(held.len(), 3);
    // The ledger is built by any first lookup; keep it out of the count.
    repo.has_content(&Id::of_content(b"x")).unwrap();
    repo.store().reads.store(0, Relaxed);
    let began = batch && repo.begin_batch();
    match hand {
        Hand::Nothing => {}
        Hand::Bytes => held.iter().for_each(|(id, b)| repo.keep_read(id, b)),
        Hand::WrongBytes => held.iter().for_each(|(id, _)| repo.keep_read(id, b"x")),
    }
    let new: Vec<Id> = (0..3)
        .map(|i| repo.put_content(&edited(&files[i], i as u8)).unwrap().id)
        .collect();
    repo.end_batch(began);
    let reads = repo.store().reads.load(Relaxed);
    for (i, id) in new.iter().enumerate() {
        assert_eq!(repo.read_content(id).unwrap(), edited(&files[i], i as u8));
    }
    assert!(repo.fsck().unwrap().is_empty());
    let mut objects: Vec<(Id, Vec<u8>)> = repo
        .store()
        .ids()
        .unwrap()
        .into_iter()
        .map(|i| (i, repo.store().inner.read(&i).unwrap()))
        .collect();
    objects.sort();
    (reads, objects)
}

#[test]
fn a_batch_reads_less_and_stores_the_same() {
    let (plain, objects) = run(false, Hand::Nothing);
    let (batched, b_objects) = run(true, Hand::Nothing);
    let (handed, h_objects) = run(true, Hand::Bytes);
    assert_eq!(b_objects, objects);
    assert_eq!(h_objects, objects);
    assert!(
        batched < plain,
        "{batched} reads in a batch, {plain} without"
    );
    // Handed the files, each write reads only the stored copy of its old
    // version, for the length a template plan's cost needs.
    assert_eq!(handed, 3, "{handed} reads when handed the files");
}

#[test]
fn wrong_bytes_and_bytes_outside_a_batch_are_ignored() {
    let (plain, objects) = run(false, Hand::Nothing);
    let (wrong, w_objects) = run(true, Hand::WrongBytes);
    let (outside, o_objects) = run(false, Hand::Bytes);
    assert_eq!(w_objects, objects);
    assert_eq!(o_objects, objects);
    assert!(wrong > 0);
    assert_eq!(outside, plain);
}
