//! E305: what `refresh_ledger` (E298) costs on a large store.
//!
//! A miss with the option on lists the store once and reads the objects
//! the ledger has not seen. This measures, on FsStore stores of N stored
//! objects: a plain read of stored content; a read of content that is
//! really missing, off and on (after the first miss, which builds the
//! ledger); and a read of content another handle derived
//! after this one's ledger was built (on; off it fails).
//!
//!   cargo run --release --example refresh_cost [-- N...]

use std::time::Instant;

use ikam_kernel::{Form, FsStore, Id, Repo};

fn blob(i: u64, len: usize) -> Vec<u8> {
    let mut x = (i + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..len)
        .map(|_| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
        })
        .collect()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let sizes: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    let sizes = if sizes.is_empty() {
        vec![1_000, 10_000, 50_000]
    } else {
        sizes
    };
    println!(
        "objects | stored read ms | first miss (ledger build) ms | missing, off ms | missing, on ms | other's derivation, on ms"
    );
    for n in sizes {
        let dir = std::env::temp_dir().join(format!("ikam-e305-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut seed = Repo::new(FsStore::open_deferred(&dir).unwrap());
            for i in 0..n {
                seed.put_content(&blob(i, 64)).unwrap();
            }
            seed.put_content(&blob(1_000_000, 4096)).unwrap();
        }
        let mut reader = Repo::new(FsStore::open(&dir).unwrap());
        let stored = Id::of_content(&blob(7, 64));
        reader.read_content(&stored).unwrap(); // builds the ledger
        let t = Instant::now();
        for _ in 0..100 {
            reader.read_content(&stored).unwrap();
        }
        let stored_ms = ms(t) / 100.0;
        let missing = Id::of_content(b"not here");
        // The first miss builds the ledger (a read of every object's head).
        let t = Instant::now();
        assert!(reader.read_content(&missing).is_err());
        let build_ms = ms(t);
        let t = Instant::now();
        assert!(reader.read_content(&missing).is_err());
        let off_ms = ms(t);
        reader.refresh_ledger = true;
        let mut on = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            assert!(reader.read_content(&missing).is_err());
            on.push(ms(t));
        }
        let derived = {
            let mut other = Repo::new(FsStore::open(&dir).unwrap());
            let mut bytes = blob(1_000_000, 4096);
            bytes[100] ^= 0x5A;
            bytes.extend_from_slice(b" edited");
            let put = other.put_content(&bytes).unwrap();
            assert_eq!(put.form, Form::Derived);
            put.id
        };
        let t = Instant::now();
        reader.read_content(&derived).unwrap();
        let other_ms = ms(t);
        println!(
            "{n} | {stored_ms:.3} | {build_ms:.1} | {off_ms:.3} | {:.1} to {:.1} | {other_ms:.1}",
            on.iter().cloned().fold(f64::MAX, f64::min),
            on.iter().cloned().fold(0.0, f64::max)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
