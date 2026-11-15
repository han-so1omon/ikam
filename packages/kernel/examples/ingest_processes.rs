//! E299: writer processes putting near-copies of the same contents on one
//! FsStore.
//!
//! E296 checked that threads sharing one handle's ledger lose nothing when
//! they put near-copies at once. Processes share nothing but the directory:
//! a put can drop a stored copy it replaces with a derivation (a template
//! fill) after checking that no stored encoding needs it, while another
//! process derives something from that same copy. This starts W copies of
//! itself on one store seeded with BASE blobs; each puts M near-copies of
//! blobs the others are editing too and reads each back at once (with
//! `refresh_ledger`, E298). Afterwards a fresh handle reads every content
//! and every seed blob, and fsck runs.
//!
//!   cargo run --release --example ingest_processes [-- TRIALS]
//!   (W, M and BASE as env; defaults 4, 60 and 24)

use std::process::{Command, Stdio};

use ikam_kernel::{Form, FsStore, Id, Repo};

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

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

/// Process `w`'s `k`-th content: a near-copy of a blob every process edits.
fn edit(trial: usize, w: usize, k: usize, base: usize) -> Vec<u8> {
    let j = (k + trial) % base;
    let mut bytes = blob(j);
    bytes[64 * (w + 1)] ^= 0x5A;
    bytes[2048 + k] ^= 0xA5;
    bytes.extend_from_slice(format!("t{trial} w{w} k{k}").as_bytes());
    bytes
}

fn child(dir: &str, trial: usize, w: usize) {
    let (m, base) = (env("M", 60), env("BASE", 24));
    let mut repo = Repo::new(FsStore::open(dir).unwrap());
    repo.refresh_ledger = true;
    let (mut derived, mut fails, mut errors) = (0, 0, 0);
    for k in 0..m {
        let bytes = edit(trial, w, k, base);
        match repo.put_content(&bytes) {
            Ok(put) => {
                if put.form == Form::Derived {
                    derived += 1;
                }
                if repo
                    .read_content(&put.id)
                    .map(|b| b != bytes)
                    .unwrap_or(true)
                {
                    fails += 1;
                }
            }
            Err(e) => {
                errors += 1;
                eprintln!("put error: {e}");
            }
        }
    }
    println!("{derived} {fails} {errors}");
}

fn main() {
    if let Ok(spec) = std::env::var("E299_CHILD") {
        let f: Vec<&str> = spec.split('|').collect();
        child(f[0], f[1].parse().unwrap(), f[2].parse().unwrap());
        return;
    }
    let trials: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let (w, m, base) = (env("W", 4), env("M", 60), env("BASE", 24));
    let me = std::env::current_exe().unwrap();
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    let (mut derived, mut during, mut errors, mut after, mut seeds_lost, mut fsck) =
        (0, 0, 0, 0, 0, 0);
    for trial in 0..trials {
        let dir = std::env::temp_dir().join(format!("ikam-e299-{}-{trial}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = dir.to_str().unwrap().to_string();
        {
            let mut seed = Repo::new(FsStore::open(&d).unwrap());
            for i in 0..base {
                seed.put_content(&blob(i)).unwrap();
            }
        }
        let kids: Vec<_> = (0..w)
            .map(|i| {
                Command::new(&me)
                    .env("E299_CHILD", format!("{d}|{trial}|{i}"))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for kid in kids {
            let out = kid.wait_with_output().unwrap();
            for line in String::from_utf8_lossy(&out.stderr).lines() {
                let kind = line.split(':').take(2).collect::<Vec<_>>().join(":");
                let kind: String = kind
                    .split(' ')
                    .map(|t| if t.len() == 64 { "<id>" } else { t })
                    .collect::<Vec<_>>()
                    .join(" ");
                *kinds.entry(kind).or_insert(0usize) += 1;
            }
            let s = String::from_utf8_lossy(&out.stdout);
            let v: Vec<usize> = s
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            if v.len() == 3 && out.status.success() {
                derived += v[0];
                during += v[1];
                errors += v[2];
            } else {
                errors += 1;
            }
        }
        let fresh = Repo::new(FsStore::open(&d).unwrap());
        for i in 0..w {
            for k in 0..m {
                let bytes = edit(trial, i, k, base);
                if fresh
                    .read_content(&Id::of_content(&bytes))
                    .map(|b| b != bytes)
                    .unwrap_or(true)
                {
                    after += 1;
                }
            }
        }
        for i in 0..base {
            let b = blob(i);
            if fresh
                .read_content(&Id::of_content(&b))
                .map(|x| x != b)
                .unwrap_or(true)
            {
                seeds_lost += 1;
            }
        }
        fsck += fresh.fsck().unwrap().len();
        let _ = std::fs::remove_dir_all(&dir);
    }
    println!(
        "{trials} trials x {w} processes x {m} puts over {base} seed blobs: {derived} stored as derivations; \
         reads failed right after the put {during}, put errors {errors}; through a fresh handle afterwards: \
         {after} puts and {seeds_lost} seeds unreadable, fsck {fsck}"
    );
    for (k, c) in kinds {
        println!("  {c} x {k}");
    }
}
