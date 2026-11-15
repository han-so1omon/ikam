//! E281: how often a long run should save its progress.
//!
//! A runner takes N candidates through the loop net on an FsStore,
//! committing to a ref after every append (as in E271). Each candidate's
//! work is TICKS steps of TICK_MS (a hash chain that sleeps between steps).
//! Every K steps the runner saves the chain with `net_checkpoint`; after a
//! crash it resumes from `net_resume`. Crashes come at random times, with a
//! mean time between them of M ms of wall time (exponential, seeded): at
//! each step and before each append the runner checks whether the next
//! crash is due, and if so drops everything in memory and reopens.
//!
//! For each M and K: total wall time, work steps lost, checkpoints and
//! their p50 cost C. Young's rule puts the best interval near
//! sqrt(2 C M); the run says whether the log's own cost follows it.
//! Every run ends with all candidates waiting on the owner, every finish
//! output equal to the chain computed without crashes, replay equal, fsck
//! clean.
//!
//!   cargo run --release --example net_checkpoint [-- N]

use std::time::{Duration, Instant};

use ikam_kernel::net::Net;
use ikam_kernel::{Db, FsStore, Repo};

const TICK_MS: u64 = 5;
const TICKS: u32 = 160;
const REF: &str = "loop-run";

const LOOP: &str = "\
net loop 1
place cpu 1
place queued colored
place running colored
place measured colored
place waiting colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
transition ask pure: measured -> waiting
transition merge human: waiting ->
";

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn exp(&mut self, mean: f64) -> f64 {
        let u = ((self.next() >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
        -mean * u.ln()
    }
}

/// One step of a candidate's work.
fn step(state: [u8; 32]) -> [u8; 32] {
    std::thread::sleep(Duration::from_millis(TICK_MS));
    *blake3::hash(&state).as_bytes()
}

fn chain(tok: &str) -> [u8; 32] {
    let mut s = *blake3::hash(tok.as_bytes()).as_bytes();
    for _ in 0..TICKS {
        s = *blake3::hash(&s).as_bytes();
    }
    s
}

fn encode(i: u32, s: &[u8; 32]) -> Vec<u8> {
    let mut v = format!("step {i}\n").into_bytes();
    v.extend_from_slice(s);
    v
}

fn decode(v: &[u8]) -> (u32, [u8; 32]) {
    let nl = v.iter().position(|b| *b == b'\n').unwrap();
    let i = std::str::from_utf8(&v[5..nl]).unwrap().parse().unwrap();
    (i, v[nl + 1..].try_into().unwrap())
}

struct Runner {
    dir: std::path::PathBuf,
    repo: Repo<FsStore>,
    db: Db,
    net: Net,
    rng: Rng,
    mean_ms: f64,
    due: Instant,
    crashes: usize,
    steps_run: u64,
    ckpt_us: Vec<u128>,
}

fn open(dir: &std::path::Path, net: &Net) -> (Repo<FsStore>, Db) {
    let mut repo = Repo::new(FsStore::open(dir).unwrap());
    let db = match repo.resolve(REF) {
        Ok(c) => repo.db_at(&c).unwrap(),
        Err(_) => {
            let db = repo.db_empty().unwrap();
            let db = repo.net_init_sum(&db, net).unwrap();
            repo.db_commit(REF, &db, "init").unwrap();
            db
        }
    };
    (repo, db)
}

impl Runner {
    /// If the next crash is due: drop memory, reopen, draw the next one.
    fn crashed(&mut self) -> bool {
        if Instant::now() < self.due {
            return false;
        }
        self.crashes += 1;
        (self.repo, self.db) = open(&self.dir, &self.net);
        self.due = Instant::now() + Duration::from_secs_f64(self.rng.exp(self.mean_ms) / 1000.0);
        true
    }

    fn commit(&mut self, db: Db) {
        self.repo.db_commit(REF, &db, "step").unwrap();
        self.db = db;
    }

    fn fire(&mut self, t: &str, tok: &str, out: Option<&[u8]>) -> bool {
        if self.crashed() {
            return false;
        }
        let key = format!("{t}-{tok}");
        let (db, _) = match out {
            None => self.repo.net_fire(&self.db, &self.net, t, Some(tok), &key),
            Some(o) => self
                .repo
                .net_fire_output(&self.db, &self.net, t, Some(tok), &key, o),
        }
        .unwrap();
        self.commit(db);
        true
    }

    fn place_of(&self, tok: &str) -> Option<&'static str> {
        ["queued", "running", "measured", "waiting"]
            .into_iter()
            .find(|p| {
                self.repo
                    .db_get(&self.db, &format!("net/loop/m/{p}/{tok}"))
                    .unwrap()
                    .is_some()
            })
    }

    /// The work, resuming from the last checkpoint; None on a crash.
    fn work(&mut self, tok: &str, every: Option<u32>) -> Option<[u8; 32]> {
        let key = format!("finish-{tok}");
        let (mut i, mut s) = match self.repo.net_resume(&self.db, "loop", &key).unwrap() {
            Some(v) => decode(&v),
            None => (0, *blake3::hash(tok.as_bytes()).as_bytes()),
        };
        while i < TICKS {
            if self.crashed() {
                return None;
            }
            s = step(s);
            i += 1;
            self.steps_run += 1;
            if every.is_some_and(|k| i % k == 0 && i < TICKS) {
                if self.crashed() {
                    return None;
                }
                let start = Instant::now();
                let db = self
                    .repo
                    .net_checkpoint(&self.db, "loop", &key, &encode(i, &s))
                    .unwrap();
                self.commit(db);
                self.ckpt_us.push(start.elapsed().as_micros());
            }
        }
        Some(s)
    }

    fn run(&mut self, tok: &str, every: Option<u32>) {
        loop {
            match self.place_of(tok) {
                None => {
                    self.fire("propose", tok, None);
                }
                Some("queued") => {
                    self.fire("start", tok, None);
                }
                Some("running") => {
                    if let Some(s) = self.work(tok, every) {
                        self.fire("finish", tok, Some(&s));
                    }
                }
                Some("measured") => {
                    self.fire("ask", tok, None);
                }
                Some(_) => return,
            }
        }
    }
}

fn main() {
    let n: usize = (std::env::args().nth(1))
        .and_then(|a| a.parse().ok())
        .unwrap_or(10);
    let net = Net::parse(LOOP).unwrap();
    let tokens: Vec<String> = (0..n).map(|i| format!("E{i:03}")).collect();
    let want: Vec<[u8; 32]> = tokens.iter().map(|t| chain(t)).collect();
    println!(
        "{n} candidates, {TICKS} steps of {TICK_MS} ms each ({} ms of work per candidate)",
        TICKS as u64 * TICK_MS
    );
    for mean_ms in [500.0, 2000.0] {
        println!("mean time between crashes {mean_ms:.0} ms:");
        for every in [
            Some(1),
            Some(4),
            Some(8),
            Some(16),
            Some(32),
            Some(64),
            None,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (repo, db) = open(dir.path(), &net);
            let mut rng = Rng(0x5EED_0000_0000_0281);
            let due = Instant::now() + Duration::from_secs_f64(rng.exp(mean_ms) / 1000.0);
            let mut r = Runner {
                dir: dir.path().to_path_buf(),
                repo,
                db,
                net: net.clone(),
                rng,
                mean_ms,
                due,
                crashes: 0,
                steps_run: 0,
                ckpt_us: vec![],
            };
            let start = Instant::now();
            for tok in &tokens {
                r.run(tok, every);
            }
            let secs = start.elapsed().as_secs_f64();
            let (repo, db) = open(dir.path(), &net);
            let m = repo.net_marking(&db, "loop").unwrap();
            assert_eq!(repo.net_replay(&db, &net).unwrap(), m, "replay");
            assert_eq!(m.get("waiting").map(|w| w.len()), Some(n));
            for (tok, w) in tokens.iter().zip(&want) {
                assert_eq!(
                    repo.net_verify(&db, "loop", &format!("finish-{tok}"), w)
                        .unwrap(),
                    Some(true),
                    "output of {tok}"
                );
            }
            assert!(repo.fsck().unwrap().is_empty(), "fsck");
            let mut c = r.ckpt_us.clone();
            c.sort_unstable();
            let c_ms = if c.is_empty() {
                0.0
            } else {
                c[c.len() / 2] as f64 / 1000.0
            };
            let lost = r.steps_run - n as u64 * TICKS as u64;
            println!(
                "  every {:>4}: {:>6.1} s, {:>3} crashes, {:>4} steps lost ({:>5} ms), {:>4} checkpoints at p50 {:.1} ms",
                every.map_or("none".to_string(), |k| format!("{}ms", k as u64 * TICK_MS)),
                secs,
                r.crashes,
                lost,
                lost * TICK_MS,
                c.len(),
                c_ms
            );
        }
    }
}
