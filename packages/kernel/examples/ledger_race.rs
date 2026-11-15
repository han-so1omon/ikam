//! E296: do the two ledger races E292 found by reading the code happen?
//!
//! The ledger (derivation records by output) is built from the store on
//! first use and then kept current by each write. Siblings on one store
//! share it. Two races were read from `repo.rs`, not seen:
//! - a build lists the store's ids while another handle records a
//!   derivation; the record finds no ledger and adds nothing, and the build
//!   then installs a ledger without it;
//! - a handle that writes a record another handle wrote first (the same
//!   bytes) does not add it, though the other may not have added it yet.
//!
//! Either leaves content that is stored only as a derivation unreadable
//! through the shared ledger, though a fresh handle reads it.
//!
//! Each trial: a fresh handle on a store of BASE blobs (no ledger yet), W
//! siblings each putting M near-copies of the blobs (stored as slices of
//! them) and reading each back at once; then every content is read again
//! through the shared ledger, and through a fresh handle.
//!
//!   cargo run --release --example ledger_race [-- TRIALS]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

use ikam_kernel::{Form, Id, MemStore, Repo, SharedStore};

const BASE: usize = 400;
const W: usize = 8;
const M: usize = 40;

fn blob(i: usize) -> Vec<u8> {
    let mut x = (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..4096)
        .map(|_| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
        })
        .collect()
}

fn main() {
    let trials: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let shared = Arc::new(std::env::var("SAME").as_deref() == Ok("1"));
    let (mut derived, mut during, mut after, mut fresh_fail) = (0, 0, 0, 0);
    for trial in 0..trials {
        let store = SharedStore::new(MemStore::default());
        {
            let mut seed = Repo::new(store.clone());
            for i in 0..BASE {
                seed.put_content(&blob(i)).unwrap();
            }
        }
        let top = Repo::new(store.clone());
        let fails = Arc::new(AtomicUsize::new(0));
        let n_derived = Arc::new(AtomicUsize::new(0));
        let hs: Vec<_> = (0..W)
            .map(|w| {
                let mut r = top.sibling();
                let (fails, n_derived, shared) = (fails.clone(), n_derived.clone(), shared.clone());
                std::thread::spawn(move || {
                    let mut ids: Vec<Id> = Vec::new();
                    for k in 0..M {
                        // With SAME=1 every writer puts the same contents, so
                        // two handles often write one record.
                        let who = if *shared { 0 } else { w };
                        let j = (who * M + k + trial) % BASE;
                        let mut bytes = blob(j);
                        bytes[100 + who] ^= 0x5A;
                        bytes.extend_from_slice(format!("w{who} k{k}").as_bytes());
                        let put = r.put_content(&bytes).unwrap();
                        if put.form == Form::Derived {
                            n_derived.fetch_add(1, SeqCst);
                        }
                        if r.read_content(&put.id).map(|b| b != bytes).unwrap_or(true) {
                            fails.fetch_add(1, SeqCst);
                        }
                        ids.push(put.id);
                    }
                    ids
                })
            })
            .collect();
        let ids: Vec<Id> = hs.into_iter().flat_map(|h| h.join().unwrap()).collect();
        derived += n_derived.load(SeqCst);
        during += fails.load(SeqCst);
        let fresh = Repo::new(store.clone());
        for id in &ids {
            if top.read_content(id).is_err() {
                after += 1;
            }
            if fresh.read_content(id).is_err() {
                fresh_fail += 1;
            }
        }
    }
    println!(
        "{trials} trials x {W} writers x {M} puts ({}): {derived} stored as derivations; reads failed: {during} right after the put, {after} after the writers stopped (shared ledger), {fresh_fail} through a fresh handle",
        if *shared {
            "the same contents"
        } else {
            "different contents"
        }
    );
}
