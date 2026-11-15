//! E298: a handle reading what another handle on the same store (another
//! process, here a second `Repo` on one directory) derives after this
//! handle's ledger was built.

use ikam_kernel::{Error, Form, FsStore, Repo};

fn blob(seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..4096)
        .map(|_| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
        })
        .collect()
}

/// A put of a near-copy of a stored blob, by the other handle, that is
/// stored as a derivation; returns its bytes and id.
fn derived_by_other(dir: &std::path::Path) -> (Vec<u8>, ikam_kernel::Id) {
    let mut other = Repo::new(FsStore::open(dir).unwrap());
    let mut bytes = blob(1);
    bytes[100] ^= 0x5A;
    bytes.extend_from_slice(b" edited");
    let put = other.put_content(&bytes).unwrap();
    assert_eq!(put.form, Form::Derived);
    (bytes, put.id)
}

fn reader(dir: &std::path::Path, refresh: bool) -> Repo<FsStore> {
    let mut r = Repo::new(FsStore::open(dir).unwrap());
    r.refresh_ledger = refresh;
    // Build its ledger now, before the other handle writes.
    let base = r.put_content(&blob(1)).unwrap().id;
    assert_eq!(r.read_content(&base).unwrap(), blob(1));
    r
}

#[test]
fn without_refresh_the_other_handles_derivation_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let r = reader(dir.path(), false);
    let (_, id) = derived_by_other(dir.path());
    assert!(matches!(r.read_content(&id), Err(Error::NotFound(_))));
    // A fresh handle reads it.
    let fresh = Repo::new(FsStore::open(dir.path()).unwrap());
    assert!(fresh.read_content(&id).is_ok());
}

#[test]
fn with_refresh_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let r = reader(dir.path(), true);
    let (bytes, id) = derived_by_other(dir.path());
    assert_eq!(r.read_content(&id).unwrap(), bytes);
    assert!(r.has_content(&id).unwrap());
}

#[test]
fn a_missing_content_is_still_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let r = reader(dir.path(), true);
    let missing = ikam_kernel::Id::of_content(b"never stored");
    assert!(matches!(r.read_content(&missing), Err(Error::NotFound(_))));
}

/// E299: two handles on one directory (as two processes would be), each on
/// its own thread, putting near-copies of the same contents at once. A put
/// may replace a stored copy with a derivation and drop it; the other
/// handle may decide the same and find the copy already gone, which is not
/// an error.
#[test]
fn a_copy_another_handle_dropped_first_is_not_an_error() {
    for trial in 0..4u64 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        {
            let mut seed = Repo::new(FsStore::open(&path).unwrap());
            for i in 0..12 {
                seed.put_content(&blob(i)).unwrap();
            }
        }
        let edit = move |w: usize, k: usize| {
            let mut bytes = blob(((k as u64) + trial) % 12);
            bytes[64 * (w + 1)] ^= 0x5A;
            bytes[2048 + k] ^= 0xA5;
            bytes.extend_from_slice(format!("t{trial} w{w} k{k}").as_bytes());
            bytes
        };
        let hs: Vec<_> = (0..4)
            .map(|w| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut r = Repo::new(FsStore::open(&path).unwrap());
                    r.refresh_ledger = true;
                    for k in 0..30 {
                        let bytes = edit(w, k);
                        let id = r.put_content(&bytes).unwrap().id;
                        assert_eq!(r.read_content(&id).unwrap(), bytes);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let fresh = Repo::new(FsStore::open(&path).unwrap());
        for w in 0..4 {
            for k in 0..30 {
                let bytes = edit(w, k);
                let id = ikam_kernel::Id::of_content(&bytes);
                assert_eq!(fresh.read_content(&id).unwrap(), bytes);
            }
        }
        assert!(fresh.fsck().unwrap().is_empty());
    }
}
