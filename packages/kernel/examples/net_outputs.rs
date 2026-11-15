//! E271: recording each run's output with its firing.
//!
//! A runner takes N candidates through the loop net on an FsStore, one
//! append at a time, committing the database to a ref after every append.
//! Each candidate's work (between start and finish) is a deterministic
//! computation that sleeps WORK_MS to stand for a bench run. Crashes are
//! injected with a seeded generator: before any append, with probability
//! P_CRASH, the runner drops everything in memory, reopens the store and
//! resumes from the ref. The runner keeps no state of its own: it reads the
//! marking to find each candidate's next step.
//!
//! Three modes, the same seed:
//! - `fire only` (E269): the finish firing is the commit point; work whose
//!   firing was lost to a crash is redone.
//! - `output, then fire`: the output is appended and committed on its own
//!   first, and a resumed runner that finds it fires without redoing the
//!   work (the first design tried for E271).
//! - `fire with output`: `net_fire_output` puts the output in the finish
//!   firing's own append; the record names it.
//!
//! Reported: crashes, work runs, redone runs, the finish firing's p50, and
//! the checks (replay equal, outputs readable, fsck). Then `net_verify`
//! recomputes every output: all match for the deterministic job, and a job
//! whose output carries its run count is caught on every candidate.
//!
//!   cargo run --release --example net_outputs [-- N]

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use ikam_kernel::net::Net;
use ikam_kernel::{Db, Event, FsStore, Repo};

const WORK_MS: u64 = 20;
const P_CRASH: f64 = 0.15;
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
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    FireOnly,
    OutputThenFire,
    FireWithOutput,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::FireOnly => "fire only",
            Mode::OutputThenFire => "output, then fire",
            Mode::FireWithOutput => "fire with output",
        }
    }
}

fn work(tok: &str, run: usize, nondeterministic: bool) -> Vec<u8> {
    std::thread::sleep(Duration::from_millis(WORK_MS));
    let mut out = format!("result of {tok}: ").into_bytes();
    out.extend_from_slice(blake3::hash(tok.as_bytes()).to_hex().as_bytes());
    if nondeterministic {
        out.extend_from_slice(format!(" run {run}").as_bytes());
    }
    out
}

struct Runner {
    dir: std::path::PathBuf,
    repo: Repo<FsStore>,
    db: Db,
    net: Net,
    rng: Rng,
    mode: Mode,
    crashes: usize,
    work_runs: usize,
    redone: usize,
    finish_us: Vec<u128>,
}

fn open(dir: &std::path::Path, net: &Net) -> (Repo<FsStore>, Db) {
    let mut repo = Repo::new(FsStore::open(dir).unwrap());
    let db = match repo.resolve(REF) {
        Ok(c) => repo.db_at(&c).unwrap(),
        Err(_) => {
            let db = repo.db_empty().unwrap();
            let db = repo.net_init(&db, net).unwrap();
            repo.db_commit(REF, &db, "init").unwrap();
            db
        }
    };
    (repo, db)
}

impl Runner {
    /// Before an append: maybe crash (drop memory, reopen, resume).
    fn crashed(&mut self) -> bool {
        if self.rng.chance(P_CRASH) {
            self.crashes += 1;
            (self.repo, self.db) = open(&self.dir, &self.net);
            return true;
        }
        false
    }

    fn commit(&mut self, db: Db) {
        self.repo.db_commit(REF, &db, "step").unwrap();
        self.db = db;
    }

    fn fire(&mut self, t: &str, tok: &str, output: Option<&[u8]>) {
        if self.crashed() {
            return;
        }
        let key = format!("{t}-{tok}");
        let start = Instant::now();
        let (db, _) = match output {
            None => self.repo.net_fire(&self.db, &self.net, t, Some(tok), &key),
            Some(o) => self
                .repo
                .net_fire_output(&self.db, &self.net, t, Some(tok), &key, o),
        }
        .unwrap();
        if t == "finish" {
            self.finish_us.push(start.elapsed().as_micros());
        }
        self.commit(db);
    }

    /// One step for the first token not yet waiting on the owner; false
    /// when every token waits.
    fn step(&mut self, tokens: &[String], worked: &mut BTreeSet<String>) -> bool {
        let m = self.repo.net_marking(&self.db, "loop").unwrap();
        for tok in tokens {
            let place = ["queued", "running", "measured", "waiting"]
                .into_iter()
                .find(|p| m.get(*p).is_some_and(|t| t.contains_key(tok)));
            match place {
                None => self.fire("propose", tok, None),
                Some("queued") => self.fire("start", tok, None),
                Some("running") => {
                    let out_key = format!("net/loop/out/finish-{tok}");
                    let have = self.mode == Mode::OutputThenFire
                        && self.repo.db_get(&self.db, &out_key).unwrap().is_some();
                    let out = if have {
                        None
                    } else {
                        self.work_runs += 1;
                        if !worked.insert(tok.clone()) {
                            self.redone += 1;
                        }
                        Some(work(tok, self.work_runs, false))
                    };
                    match self.mode {
                        Mode::FireOnly => self.fire("finish", tok, None),
                        Mode::FireWithOutput => self.fire("finish", tok, out.as_deref()),
                        Mode::OutputThenFire => {
                            if let Some(o) = out {
                                if self.crashed() {
                                    return true;
                                }
                                let id = self.repo.put_content(&o).unwrap().id;
                                let ev =
                                    Event::upsert(&out_key, id.to_string().as_bytes(), &out_key);
                                let db = self.repo.db_append(&self.db, &[ev]).unwrap();
                                self.commit(db);
                            }
                            self.fire("finish", tok, None);
                        }
                    }
                }
                Some("measured") => self.fire("ask", tok, None),
                Some(_) => continue,
            }
            return true;
        }
        false
    }
}

fn main() {
    let n: usize = (std::env::args().nth(1))
        .and_then(|a| a.parse().ok())
        .unwrap_or(200);
    let net = Net::parse(LOOP).unwrap();
    let tokens: Vec<String> = (0..n).map(|i| format!("E{i:04}")).collect();
    for mode in [Mode::FireOnly, Mode::OutputThenFire, Mode::FireWithOutput] {
        let dir = tempfile::tempdir().unwrap();
        let (repo, db) = open(dir.path(), &net);
        let mut r = Runner {
            dir: dir.path().to_path_buf(),
            repo,
            db,
            net: net.clone(),
            rng: Rng(0xC0FF_EE00_1234_5678),
            mode,
            crashes: 0,
            work_runs: 0,
            redone: 0,
            finish_us: vec![],
        };
        let mut worked = BTreeSet::new();
        let start = Instant::now();
        while r.step(&tokens, &mut worked) {}
        let secs = start.elapsed().as_secs_f64();

        // Reopen cold and check the run from disk alone.
        let (repo, db) = open(dir.path(), &net);
        let m = repo.net_marking(&db, "loop").unwrap();
        assert_eq!(repo.net_replay(&db, &net).unwrap(), m, "replay");
        assert_eq!(m.get("waiting").map(|w| w.len()), Some(n), "all wait");
        let firings = repo.net_firings(&db, "loop").unwrap();
        assert_eq!(firings.len(), 4 * n);
        let named = firings.iter().filter(|f| f.output.is_some()).count();
        assert!(repo.fsck().unwrap().is_empty(), "fsck");
        let mut us = r.finish_us.clone();
        us.sort_unstable();
        println!(
            "{}: {n} candidates, {} crashes, {} work runs, {} redone ({} ms of work lost), finish firing p50 {:.1} ms, {:.0} s in all; {} firings ({named} naming an output), replay equal, all waiting on the owner, fsck clean",
            mode.name(),
            r.crashes,
            r.work_runs,
            r.redone,
            r.redone as u64 * WORK_MS,
            us[us.len() / 2] as f64 / 1000.0,
            secs,
            firings.len()
        );
        if mode == Mode::FireWithOutput {
            // Recompute every output and compare with the record.
            for nondeterministic in [false, true] {
                let start = Instant::now();
                let (mut ok, mut bad) = (0, 0);
                for (i, tok) in tokens.iter().enumerate() {
                    let out = work(tok, n + i + 1, nondeterministic);
                    match repo
                        .net_verify(&db, "loop", &format!("finish-{tok}"), &out)
                        .unwrap()
                    {
                        Some(true) => ok += 1,
                        Some(false) => bad += 1,
                        None => panic!("no output recorded for {tok}"),
                    }
                }
                println!(
                    "  verify, {} job: {ok} match, {bad} differ ({:.1} s, mostly the work)",
                    if nondeterministic {
                        "nondeterministic"
                    } else {
                        "deterministic"
                    },
                    start.elapsed().as_secs_f64()
                );
            }
        }
    }
}
