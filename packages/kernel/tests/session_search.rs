//! E142: a read session's search keeps what it decodes for the reads
//! after it, and finds what `Repo::search` finds.

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

fn repo() -> (Repo<Counting>, Vec<Id>) {
    let mut repo = Repo::new(Counting::default());
    let mut files = Vec::new();
    for i in 0..200 {
        let mut text = format!("note {i}: a list of things to do this week\n").repeat(20);
        if i % 7 == 0 {
            text.push_str("revenue in Q3 grew\n");
        }
        files.push(repo.put_content(text.as_bytes()).unwrap().id);
    }
    // Kept by a commit, then repacked: small files go into groups.
    let edits: Vec<ikam_kernel::Edit> = files
        .iter()
        .enumerate()
        .map(|(i, id)| ikam_kernel::Edit::Put {
            path: format!("notes/{i:03}.md"),
            id: *id,
        })
        .collect();
    let tree = repo.edit_tree(None, &edits).unwrap();
    repo.commit("main", tree, "").unwrap();
    repo.repack().unwrap();
    (repo, files)
}

#[test]
fn a_session_search_finds_the_same_and_reads_its_hits_once() {
    let (mut repo, files) = repo();
    repo.build_search_index(&files).unwrap();
    let index = repo.search_index().unwrap();
    for pattern in [&b"Q3"[..], b"revenue", b"nothing like this"] {
        let (plain, _) = repo.search(pattern, &files, index.as_ref()).unwrap();
        let s = repo.session(64 << 20);
        let (hits, _) = s.search(pattern, &files, index.as_ref()).unwrap();
        assert_eq!(hits, plain);
        let before = repo.store().reads.load(Relaxed);
        for id in &hits {
            let got = s.read_content(id).unwrap();
            assert_eq!(got, repo.read_content(id).unwrap());
        }
        // The session's reads cost nothing; the checks above each read.
        let plain_reads: u64 = {
            let b = repo.store().reads.load(Relaxed);
            for id in &hits {
                repo.read_content(id).unwrap();
            }
            repo.store().reads.load(Relaxed) - b
        };
        let after = repo.store().reads.load(Relaxed);
        assert_eq!(after - before, 2 * plain_reads, "{pattern:?}");
    }
}

#[test]
fn a_session_search_without_room_still_finds_the_same() {
    let (mut repo, files) = repo();
    repo.build_search_index(&files).unwrap();
    let index = repo.search_index().unwrap();
    let (plain, _) = repo.search(b"Q3", &files, index.as_ref()).unwrap();
    let s = repo.session(16);
    assert_eq!(s.search(b"Q3", &files, index.as_ref()).unwrap().0, plain);
}
