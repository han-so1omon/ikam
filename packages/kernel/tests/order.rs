//! E136: file-store listings come in id order, and the same writes store the
//! same encodings in a file store as in a memory store.

use ikam_kernel::{FsStore, Id, MemStore, Repo, Store};

fn noise(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 56) as u8
        })
        .collect()
}

/// Many near-copies of a few bases, so the matcher has several equal
/// sources to choose from.
fn contents() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for b in 0..4 {
        let base = noise(b, 20_000);
        for i in 0..12 {
            let mut v = base.clone();
            v.splice(1500 * i..1500 * i, format!("copy {b}/{i}").into_bytes());
            out.push(v);
        }
    }
    out
}

fn encodings<S: Store>(repo: &Repo<S>) -> Vec<(Id, Vec<u8>)> {
    let mut ids = repo.store().ids().unwrap();
    ids.sort();
    ids.into_iter()
        .map(|id| (id, repo.store().read(&id).unwrap()))
        .collect()
}

/// Writes `contents` in two batches through two handles (the second built
/// over what the first stored, so its seed index comes from a listing).
fn store_in<S: Store>(open: impl Fn() -> S) -> Vec<(Id, Vec<u8>)> {
    let all = contents();
    let (a, b) = all.split_at(all.len() / 2);
    let first_store = {
        let mut repo = Repo::new(open());
        for c in a {
            repo.put_content(c).unwrap();
        }
        encodings(&repo)
    };
    let mut store = open();
    for (id, e) in &first_store {
        store.write(*id, e).unwrap();
    }
    let mut repo = Repo::new(store);
    for c in b {
        repo.put_content(c).unwrap();
    }
    assert!(repo.fsck().unwrap().is_empty());
    encodings(&repo)
}

#[test]
fn a_file_store_and_a_memory_store_store_the_same_bytes() {
    let mem = store_in(MemStore::default);
    let dirs: Vec<_> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
    let n = std::cell::Cell::new(0);
    let fs = store_in(|| {
        let i = n.get();
        n.set(i + 1);
        FsStore::open(dirs[i].path()).unwrap()
    });
    assert_eq!(mem.len(), fs.len());
    assert!(mem == fs, "encodings differ between the stores");
}

#[test]
fn file_store_listings_come_in_id_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut fs = FsStore::open(dir.path()).unwrap();
    for i in 0..500u32 {
        let b = i.to_le_bytes();
        fs.write(Id::of_content(&b), &b).unwrap();
    }
    let ids = fs.ids().unwrap();
    assert_eq!(ids.len(), 500);
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
    let first = *ids[0].as_bytes().first().unwrap();
    let some = fs.ids_with_prefix(&[first]).unwrap();
    assert!(!some.is_empty() && some.windows(2).all(|w| w[0] < w[1]));
}
