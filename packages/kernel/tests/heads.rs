//! E135: `Store::read_head` returns an encoding's first bytes, and a
//! projection pass built from heads finds the same derivations.

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

#[test]
fn read_head_is_a_prefix_of_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut fs = FsStore::open(dir.path()).unwrap();
    let mut mem = MemStore::default();
    for (i, len) in [0usize, 1, 255, 256, 257, 10_000].into_iter().enumerate() {
        let bytes = noise(i as u64, len);
        let id = Id::of_content(&bytes);
        fs.write(id, &bytes).unwrap();
        mem.write(id, &bytes).unwrap();
        for n in [0, 1, 256, 20_000] {
            let want = &bytes[..n.min(len)];
            assert_eq!(fs.read_head(&id, n).unwrap(), want);
            assert_eq!(mem.read_head(&id, n).unwrap(), want);
        }
    }
    let missing = Id::of_content(b"never written");
    assert!(fs.read_head(&missing, 8).is_err());
    assert!(mem.read_head(&missing, 8).is_err());
}

#[test]
fn a_reopened_store_finds_every_derivation() {
    let dir = tempfile::tempdir().unwrap();
    let mut ids = Vec::new();
    let mut want = Vec::new();
    {
        let mut repo = Repo::new(FsStore::open(dir.path()).unwrap());
        let base = noise(7, 50_000);
        ids.push(repo.put_content(&base).unwrap().id);
        for i in 0..20 {
            let mut v = base.clone();
            v.splice(1000 * i..1000 * i, format!("edit {i}").into_bytes());
            ids.push(repo.put_content(&v).unwrap().id);
        }
        // The writing handle's ledger started from an empty store and took
        // each record as it was written: no read of stored heads.
        for id in &ids {
            want.push(format!("{:?}", repo.derivations(id).unwrap()));
        }
    }
    assert!(
        want.iter().any(|d| d != "[]"),
        "the test stored no derivations"
    );
    let fresh = Repo::new(FsStore::open(dir.path()).unwrap());
    for (id, w) in ids.iter().zip(&want) {
        assert_eq!(&format!("{:?}", fresh.derivations(id).unwrap()), w);
    }
}
