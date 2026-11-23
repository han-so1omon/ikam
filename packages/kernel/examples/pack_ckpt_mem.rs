//! E384: what a pack store's checkpoints cost in memory and time. Writes N
//! small objects to a `PackStore` with lazy checkpoints and filters (as
//! `db serve` opens one), syncing every 2,000 writes as a commit would, so
//! a checkpoint follows each 64 MB of records; prints the time, the
//! process's peak resident memory (VmHWM, Linux /proc) and the slowest sync.
//!   cargo run --release --example pack_ckpt_mem -- N DIR
use std::time::Instant;

use ikam_kernel::backends::{PackOptions, PackStore};
use ikam_kernel::{Id, Store};

fn hwm() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with("VmHWM"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0)
        / 1024
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: u64 = a[1].parse().unwrap();
    let dir = std::path::PathBuf::from(&a[2]);
    let _ = std::fs::remove_dir_all(&dir);
    let opts = PackOptions {
        lazy_checkpoint: true,
        filters: true,
        ..Default::default()
    };
    let mut st = PackStore::open_with(&dir, opts).unwrap();
    let before = hwm();
    let t0 = Instant::now();
    let mut slowest = 0f64;
    for i in 0..n {
        let body = format!("row {i} with a few words of text {}", i * 7919 % 10007);
        st.write(Id::of(body.as_bytes()), body.as_bytes()).unwrap();
        if i % 2000 == 1999 {
            let t = Instant::now();
            st.sync().unwrap();
            slowest = slowest.max(t.elapsed().as_secs_f64());
        }
    }
    st.sync().unwrap();
    println!(
        "{{\"objects\":{n},\"s\":{:.1},\"peak_mb\":{},\"peak_before_mb\":{before},\"slowest_sync_ms\":{:.0}}}",
        t0.elapsed().as_secs_f64(),
        hwm(),
        slowest * 1000.0
    );
}
