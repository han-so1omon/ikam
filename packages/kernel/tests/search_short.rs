//! E132: patterns of one or two bytes are answered from the trigram index,
//! with exactly the results of a scan; E149: so are patterns of up to
//! three bytes from the postings alone.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use ikam_kernel::{Error, Id, MemStore, Repo, Store};

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

fn check(contents: &[Vec<u8>], patterns: &[Vec<u8>]) {
    let mut repo = Repo::new(MemStore::default());
    let mut files: Vec<_> = contents
        .iter()
        .map(|c| repo.put_content(c).unwrap().id)
        .collect();
    repo.build_search_index(&files).unwrap();
    // A content the index does not cover is always scanned.
    files.push(repo.put_content(b"unindexed Q3 abQ3ab").unwrap().id);
    let index = repo.search_index().unwrap().unwrap();
    for p in patterns {
        let (mut scan, _) = repo.search(p, &files, None).unwrap();
        repo.exact_short_search = false;
        let (mut indexed, _) = repo.search(p, &files, Some(&index)).unwrap();
        // E149: from the postings alone, the same results.
        repo.exact_short_search = true;
        let (mut exact, _) = repo.search(p, &files, Some(&index)).unwrap();
        repo.exact_short_search = false;
        scan.sort();
        indexed.sort();
        exact.sort();
        assert_eq!(scan, indexed, "pattern {:?}", String::from_utf8_lossy(p));
        assert_eq!(scan, exact, "pattern {:?}", String::from_utf8_lossy(p));
    }
}

#[test]
fn exact_short_search_reads_no_indexed_match() {
    let mut repo = Repo::new(Counting {
        inner: MemStore::default(),
        reads: 0.into(),
    });
    let files: Vec<_> = (0..20)
        .map(|i| {
            let body = format!("file {i}: revenue in Q3 grew, see section {i}\n").repeat(40);
            repo.put_content(body.as_bytes()).unwrap().id
        })
        .collect();
    repo.build_search_index(&files).unwrap();
    let index = repo.search_index().unwrap().unwrap();
    for p in [&b"Q3"[..], b"Q3 ", b"zz"] {
        repo.store().reads.swap(0, Relaxed);
        let (hits, _) = repo.search(p, &files, Some(&index)).unwrap();
        let scanned = repo.store().reads.swap(0, Relaxed);
        repo.exact_short_search = true;
        let (exact, _) = repo.search(p, &files, Some(&index)).unwrap();
        repo.exact_short_search = false;
        assert_eq!(hits, exact);
        assert_eq!(
            repo.store().reads.swap(0, Relaxed),
            0,
            "reads for {:?}",
            String::from_utf8_lossy(p)
        );
        assert!(hits.is_empty() || scanned > 0);
    }
}

#[test]
fn short_patterns_match_a_scan() {
    let contents: Vec<Vec<u8>> = [
        &b"Q3"[..],
        b"x",
        b"",
        b"the Q3 report",
        b"nothing here",
        b"Q",
        b"aQ3",
        b"Q3a",
        b"q3 lower case",
    ]
    .iter()
    .map(|c| c.to_vec())
    .collect();
    let patterns: Vec<Vec<u8>> = [&b"Q3"[..], b"Q", b"3", b"x", b"zz", b"a", b"q3", b" "]
        .iter()
        .map(|p| p.to_vec())
        .collect();
    check(&contents, &patterns);
}

#[test]
fn every_short_pattern_over_a_small_alphabet() {
    let alphabet = b"abQ3";
    let mut x = 7u64;
    let mut next = || {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (x >> 33) as usize
    };
    let contents: Vec<Vec<u8>> = (0..60)
        .map(|_| {
            let n = next() % 8;
            (0..n).map(|_| alphabet[next() % 4]).collect()
        })
        .collect();
    let mut patterns: Vec<Vec<u8>> = alphabet.iter().map(|&a| vec![a]).collect();
    for &a in alphabet {
        for &b in alphabet {
            patterns.push(vec![a, b]);
            for &c in alphabet {
                patterns.push(vec![a, b, c]);
            }
        }
    }
    patterns.push(b"abQ3".to_vec());
    check(&contents, &patterns);
}
