//! E300: with `FsStore::open_deferred` (E227), new objects wait under
//! temporary names until the next ref move, so a crash before it leaves
//! only temporary files. A put that replaces a stored copy with a
//! derivation also deletes that copy: if the delete is not deferred too, a
//! crash in between loses the content.

use ikam_kernel::{Form, FsStore, Id, Repo};

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

#[test]
fn a_crash_after_a_put_that_drops_a_copy_loses_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let seeds: Vec<Vec<u8>> = (0..24).map(blob).collect();
    {
        let mut r = Repo::new(FsStore::open_deferred(dir.path()).unwrap());
        for s in &seeds {
            r.put_content(s).unwrap();
        }
    } // dropped: made durable
    let mut r = Repo::new(FsStore::open_deferred(dir.path()).unwrap());
    let mut derived = 0;
    let before = ikam_kernel::Store::ids(r.store()).unwrap();
    for w in 0..4usize {
        for k in 0..60usize {
            let mut bytes = seeds[k % 24].clone();
            bytes[64 * (w + 1)] ^= 0x5A;
            bytes[2048 + k] ^= 0xA5;
            bytes.extend_from_slice(format!("w{w} k{k}").as_bytes());
            if r.put_content(&bytes).unwrap().form == Form::Derived {
                derived += 1;
            }
        }
    }
    let after = ikam_kernel::Store::ids(r.store()).unwrap();
    let deleted = before.iter().filter(|i| !after.contains(i)).count();
    assert!(deleted > 0, "no put dropped a stored copy");
    assert!(derived > 0);
    // The process dies here: nothing is made durable, nothing renamed.
    std::mem::forget(r);
    let fresh = Repo::new(FsStore::open(dir.path()).unwrap());
    let lost: Vec<usize> = (0..24)
        .filter(|&i| {
            fresh
                .read_content(&Id::of_content(&seeds[i]))
                .map(|b| b != seeds[i])
                .unwrap_or(true)
        })
        .collect();
    assert!(lost.is_empty(), "seeds lost after the crash: {lost:?}");
    assert!(fresh.fsck().unwrap().is_empty());
}

#[test]
fn after_a_sync_the_dropped_copies_are_gone_and_everything_reads() {
    let dir = tempfile::tempdir().unwrap();
    let seeds: Vec<Vec<u8>> = (0..24).map(blob).collect();
    {
        let mut r = Repo::new(FsStore::open_deferred(dir.path()).unwrap());
        for s in &seeds {
            r.put_content(s).unwrap();
        }
    }
    let mut puts = Vec::new();
    {
        let mut r = Repo::new(FsStore::open_deferred(dir.path()).unwrap());
        for w in 0..4usize {
            for k in 0..60usize {
                let mut bytes = seeds[k % 24].clone();
                bytes[64 * (w + 1)] ^= 0x5A;
                bytes[2048 + k] ^= 0xA5;
                bytes.extend_from_slice(format!("w{w} k{k}").as_bytes());
                let id = r.put_content(&bytes).unwrap().id;
                assert_eq!(r.read_content(&id).unwrap(), bytes);
                puts.push((id, bytes));
            }
        }
    } // dropped: synced
    let fresh = Repo::new(FsStore::open(dir.path()).unwrap());
    let stored = seeds
        .iter()
        .filter(|s| ikam_kernel::Store::has(fresh.store(), &Id::of_content(s)))
        .count();
    assert!(stored < seeds.len(), "the dropped copies were removed");
    for s in &seeds {
        assert_eq!(&fresh.read_content(&Id::of_content(s)).unwrap(), s);
    }
    for (id, bytes) in &puts {
        assert_eq!(&fresh.read_content(id).unwrap(), bytes);
    }
    assert!(fresh.fsck().unwrap().is_empty());
}
