//! E307: what a night's heartbeat refs cost the store.
//!
//! E306 moved heartbeats onto a ref of each run's own (`beat/<start key>`),
//! about ten commits per run, and never removed them. A night of thousands of
//! runs leaves thousands of refs and their commits. This builds FsStore stores
//! with the loop's ref plus RUNS beat refs of BEATS commits each, and times:
//! writing the beats, resolving and moving the loop's ref, listing refs, fsck
//! and gc; then deleting every beat ref, gc again, and the same reads.
//!
//!   cargo run --release --example ref_scale [-- RUNS...]   (BEATS env, default 10)

use std::time::Instant;

use ikam_kernel::{FsStore, Object, Repo, Store};

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn reads(repo: &mut Repo<FsStore>, tree: ikam_kernel::Id) -> String {
    let t = Instant::now();
    for _ in 0..1000 {
        repo.resolve("loop").unwrap();
    }
    let resolve_us = ms(t);
    let t = Instant::now();
    for i in 0..20 {
        repo.commit("loop", tree, &format!("move {i}")).unwrap();
    }
    let commit = ms(t) / 20.0;
    let t = Instant::now();
    let n = repo.store().refs().unwrap().len();
    let list = ms(t);
    let t = Instant::now();
    let bad = repo.fsck().unwrap().len();
    let fsck = ms(t);
    format!(
        "{n} refs; resolve {resolve_us:.1} us, loop commit {commit:.2} ms, list refs {list:.1} ms, \
         fsck {fsck:.0} ms ({bad} problems)"
    )
}

fn main() {
    let beats = env("BEATS", 10);
    let runs: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let runs = if runs.is_empty() {
        vec![100, 1000]
    } else {
        runs
    };
    for n in runs {
        let dir = std::env::temp_dir().join(format!("ikam-e307-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = dir.to_str().unwrap().to_string();
        let mut repo = Repo::new(FsStore::open(&d).unwrap());
        let tree = repo.put(&Object::tree(vec![]).unwrap()).unwrap();
        repo.commit("loop", tree, "start").unwrap();
        let t = Instant::now();
        for r in 0..n {
            for s in 0..beats {
                repo.commit(
                    &format!("beat/w{}-{r}", r % 16),
                    tree,
                    &format!("run {r} step {s}"),
                )
                .unwrap();
            }
        }
        let write = ms(t);
        println!(
            "{n} runs x {beats} beats: written in {write:.0} ms ({:.2} ms a beat)",
            write / (n * beats).max(1) as f64
        );
        println!("  with them:    {}", reads(&mut repo, tree));
        let t = Instant::now();
        let freed = repo.gc().unwrap();
        println!("  gc with them: {:.0} ms, {freed} objects freed", ms(t));
        let t = Instant::now();
        let mut store = FsStore::open(&d).unwrap();
        let mut deleted = 0;
        for (name, id) in store.refs().unwrap() {
            if name.starts_with("beat/") {
                store.delete_ref(&name, id).unwrap();
                deleted += 1;
            }
        }
        println!("  deleted {deleted} beat refs in {:.0} ms", ms(t));
        let mut repo = Repo::new(FsStore::open(&d).unwrap());
        let t = Instant::now();
        let freed = repo.gc().unwrap();
        println!("  gc after:     {:.0} ms, {freed} objects freed", ms(t));
        println!("  after:        {}", reads(&mut repo, tree));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
