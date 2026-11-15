//! A full repack stopped after any of its writes leaves a store that reads
//! the same and is fsck clean (E310): new objects go in before what names
//! them, derivations last; unnamed garbage and derivations go before what
//! they name; replacements only once everything they name is in place.

use std::sync::Mutex;

use ikam_kernel::{Db, Error, Event, Id, MemStore, Repo, Store};

#[derive(Clone)]
enum Op {
    Write(Id, Vec<u8>),
    Replace(Id, Vec<u8>),
    Delete(Id),
    SetRef(String, Option<Id>, Id),
    DeleteRef(String, Id),
    Sync,
}

/// Passes every call on and records each write.
struct Recorder<S> {
    inner: S,
    ops: Mutex<Vec<Op>>,
}

impl<S> Recorder<S> {
    fn push(&self, op: Op) {
        self.ops.lock().unwrap().push(op);
    }
}

impl<S: Store> Store for Recorder<S> {
    fn sync(&mut self) -> Result<(), Error> {
        self.push(Op::Sync);
        self.inner.sync()
    }
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        let new = self.inner.write(id, encoded)?;
        if new {
            self.push(Op::Write(id, encoded.to_vec()));
        }
        Ok(new)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.push(Op::Replace(id, encoded.to_vec()));
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
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
        self.push(Op::Delete(*id));
        self.inner.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.inner.get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        self.push(Op::SetRef(name.into(), expected, new));
        self.inner.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.inner.refs()
    }
    fn delete_ref(&mut self, name: &str, expected: Id) -> Result<(), Error> {
        self.push(Op::DeleteRef(name.into(), expected));
        self.inner.delete_ref(name, expected)
    }
}

fn copy(src: &MemStore) -> MemStore {
    let mut out = MemStore::default();
    for id in src.ids().unwrap() {
        out.write(id, &src.read(&id).unwrap()).unwrap();
    }
    for (name, id) in src.refs().unwrap() {
        out.set_ref(&name, None, id).unwrap();
    }
    out
}

fn replay(store: &mut MemStore, ops: &[Op]) {
    for op in ops {
        match op {
            Op::Write(id, b) => {
                store.write(*id, b).unwrap();
            }
            Op::Replace(id, b) => store.replace(*id, b).unwrap(),
            Op::Delete(id) => store.delete(id).unwrap(),
            Op::SetRef(name, e, new) => store.set_ref(name, *e, *new).unwrap(),
            Op::DeleteRef(name, e) => store.delete_ref(name, *e).unwrap(),
            Op::Sync => store.sync().unwrap(),
        }
    }
}

fn row(i: u64, step: u64) -> Vec<u8> {
    format!(
        "{i},customer {},{}.{:02},order {} of the day,net {}",
        i % 37,
        (i * 7 + step) % 1000,
        (i + step) % 100,
        (i * 3 + step) % 50,
        30 + (i + step) % 4 * 15
    )
    .into_bytes()
}

/// `rows` rows, a first cold pass, then `batches` batches of `batch` upserts.
fn store(rows: u64, batches: u64, batch: u64) -> (MemStore, Db, Vec<(String, Vec<u8>)>) {
    let mut repo = Repo::new(MemStore::default());
    repo.plain_db_events = true;
    let empty = repo.db_empty().unwrap();
    let events: Vec<Event> = (0..rows)
        .map(|i| Event::upsert(&format!("t/{i:05}"), &row(i, 0), &format!("load-{i}")))
        .collect();
    let mut db = repo.db_append(&empty, &events).unwrap();
    repo.db_commit("main", &db, "load").unwrap();
    repo.repack_incremental().unwrap();
    for b in 1..=batches {
        let events: Vec<Event> = (0..batch)
            .map(|j| {
                let k = (b * 97 + j * 13) % rows;
                Event::upsert(&format!("t/{k:05}"), &row(k, b), &format!("edit-{b}-{j}"))
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
    }
    let scan = repo.db_scan(&db).unwrap();
    let base = copy(repo.store());
    (base, db, scan)
}

/// Stops a recorded full repack of `store(rows, batches, batch)` after
/// every `step`-th write (and after the last) and returns the stopping
/// points whose store does not scan the same or is not fsck clean.
fn unclean(rows: u64, batches: u64, batch: u64, step: usize) -> (Vec<usize>, usize) {
    let (base, db, scan) = store(rows, batches, batch);
    let mut repo = Repo::new(Recorder {
        inner: copy(&base),
        ops: Mutex::new(Vec::new()),
    });
    repo.plain_db_events = true;
    assert!(
        repo.repack().unwrap().applied,
        "the repack changes the store"
    );
    let ops = std::mem::take(&mut *repo.store().ops.lock().unwrap());
    assert!(ops.len() > 100, "{} writes", ops.len());
    let points: Vec<usize> = (0..=ops.len())
        .filter(|k| k % step == 0 || *k == ops.len())
        .collect();
    let bad = (points.iter())
        .filter(|&&k| {
            let mut store = copy(&base);
            replay(&mut store, &ops[..k]);
            let repo = Repo::new(store);
            !(repo.db_scan(&db).ok().as_ref() == Some(&scan)
                && repo.fsck_deep().map(|b| b.is_empty()).unwrap_or(false))
        })
        .copied()
        .collect();
    (bad, points.len())
}

#[test]
fn a_full_repack_stopped_after_any_write_reads_the_same() {
    let (bad, points) = unclean(400, 5, 40, 1);
    assert!(
        bad.is_empty(),
        "{} of {points} stopping points unclean, first after {}",
        bad.len(),
        bad[0]
    );
}

/// A larger store, where a content the repack turns into a group member is
/// still read as a context by a frame it has yet to rewrite.
#[test]
fn a_larger_full_repack_stopped_part_way_reads_the_same() {
    let (bad, points) = unclean(3000, 10, 100, 75);
    assert!(
        bad.is_empty(),
        "{} of {points} stopping points unclean, first after {}",
        bad.len(),
        bad[0]
    );
}
