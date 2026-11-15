//! E298: writer processes on one FsStore.
//!
//! E292 to E295 ran every writer as a thread on one MemStore, sharing one
//! handle's ledger and indexes. A loop runner more likely runs one process
//! per worker on a store on disk, each with a handle of its own. This
//! starts W copies of itself on one FsStore directory, each taking the loop
//! net's candidates through start, WORK_MS (env, default 40) of work and
//! finish, committing every firing with `db_commit_at` (E292) or, for
//! contrast, `db_commit`. Afterwards the parent reads the head through a
//! fresh handle: candidates measured, runs, lost starts, refused commits,
//! errors, time (each run stops after 30 s), replay and fsck.
//!
//!   cargo run --release --example net_processes [-- N]
//!   (WS=1,2,4 picks the process counts; TUNED=1 as in E295;
//!   MODES=at,commit picks the commit ways; REFRESH=1 turns on
//!   `refresh_ledger`)

use std::process::Command;
use std::time::{Duration, Instant};

use ikam_kernel::net::Net;
use ikam_kernel::{Db, Error, FsStore, Repo};

const REF: &str = "loop";
const CPUS: u32 = 2;

fn tuned() -> bool {
    std::env::var("TUNED").as_deref() == Ok("1")
}

fn work_ms() -> u64 {
    std::env::var("WORK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40)
}

fn loop_net() -> Net {
    Net::parse(&format!(
        "\
net loop 1
place cpu {CPUS}
place queued colored
place running colored
place measured colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
"
    ))
    .unwrap()
}

fn open(dir: &str) -> Repo<FsStore> {
    // DEFERRED=1: objects made durable at the next ref move (E227, E300).
    let store = if std::env::var("DEFERRED").as_deref() == Ok("1") {
        FsStore::open_deferred(dir).unwrap()
    } else {
        FsStore::open(dir).unwrap()
    };
    let mut repo = Repo::new(store);
    repo.refresh_ledger = std::env::var("REFRESH").as_deref() == Ok("1");
    if tuned() {
        repo.read_weight = 1.0;
        repo.read_cache = 64 << 20;
    }
    repo
}

fn try_count(repo: &Repo<FsStore>, db: &Db, place: &str) -> Result<usize, Error> {
    Ok(repo
        .net_marking(db, "loop")?
        .get(place)
        .map(|m| m.values().sum::<u64>() as usize)
        .unwrap_or(0))
}

fn count(repo: &Repo<FsStore>, db: &Db, place: &str) -> usize {
    try_count(repo, db, place).unwrap()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[derive(Default)]
struct Tally {
    runs: usize,
    lost: usize,
    retries: usize,
    errors: Vec<String>,
}

/// Commit; Ok(false) when refused and the caller must reread.
fn commit(
    repo: &mut Repo<FsStore>,
    at: bool,
    base: ikam_kernel::Id,
    db: &Db,
) -> Result<bool, Error> {
    let r = if at {
        repo.db_commit_at(REF, base, db, "step")
    } else {
        repo.db_commit(REF, db, "step")
    };
    match r {
        Ok(_) => Ok(true),
        Err(Error::RefConflict(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

fn child(dir: &str, w: usize, n: usize, at: bool) -> Tally {
    let net = loop_net();
    let mut repo = open(dir);
    let mut t = Tally::default();
    let mut rng = Rng(0x9E37_79B9 ^ ((w as u64 + 1) * 0x1000_0001));
    let mut k = 0usize;
    let stop = Instant::now() + Duration::from_secs(30);
    let mut step = |repo: &mut Repo<FsStore>, t: &mut Tally| -> Result<bool, Error> {
        let head = repo.resolve(REF)?;
        let db = repo.db_at(&head)?;
        if try_count(repo, &db, "measured")? >= n {
            return Ok(false);
        }
        let starts: Vec<String> = repo
            .net_enabled_limit(&db, &net, true, 64)?
            .into_iter()
            .filter(|(tr, _)| tr == "start")
            .filter_map(|(_, tok)| tok)
            .collect();
        if starts.is_empty() {
            std::thread::sleep(Duration::from_millis(1));
            return Ok(true);
        }
        let tok = starts[(rng.next() % starts.len() as u64) as usize].clone();
        k += 1;
        let db = repo
            .net_fire(&db, &net, "start", Some(&tok), &format!("w{w}-{k}"))?
            .0;
        if !commit(repo, at, head, &db)? {
            t.retries += 1;
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(work_ms()));
        t.runs += 1;
        loop {
            let head = repo.resolve(REF)?;
            let db = repo.db_at(&head)?;
            k += 1;
            match repo.net_fire(&db, &net, "finish", Some(&tok), &format!("w{w}-{k}")) {
                Ok((db, _)) => {
                    if commit(repo, at, head, &db)? {
                        return Ok(true);
                    }
                    t.retries += 1;
                }
                Err(_) => {
                    t.lost += 1;
                    return Ok(true);
                }
            }
        }
    };
    while Instant::now() < stop {
        match step(&mut repo, &mut t) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                t.errors.push(e.to_string());
                if t.errors.len() > 20 {
                    break;
                }
                // A fresh handle, as a restarted worker would have.
                repo = open(dir);
            }
        }
    }
    t
}

fn parent(n: usize) {
    let net = loop_net();
    let ws: Vec<usize> = std::env::var("WS")
        .ok()
        .map(|v| v.split(',').map(|x| x.parse().unwrap()).collect())
        .unwrap_or(vec![1, 2, 4]);
    let modes: Vec<String> = std::env::var("MODES")
        .ok()
        .map(|v| v.split(',').map(String::from).collect())
        .unwrap_or(vec!["at".into(), "commit".into()]);
    println!(
        "W processes on one FsStore, {n} candidates, {} ms of work, cpu {CPUS}{}\n",
        work_ms(),
        if tuned() {
            "; read_weight 1, read cache, sum hash"
        } else {
            ""
        }
    );
    if std::env::var("REFRESH").as_deref() == Ok("1") {
        println!("refresh_ledger on (E298)\n");
    }
    println!(
        "commit        W  measured  runs  lost  retries  errors      ms  firings  replay  fsck"
    );
    let me = std::env::current_exe().unwrap();
    for mode in &modes {
        for &w in &ws {
            let dir =
                std::env::temp_dir().join(format!("ikam-e298-{}-{mode}-{w}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let d = dir.to_str().unwrap().to_string();
            {
                let mut repo = open(&d);
                let db = repo.db_empty().unwrap();
                let mut db = if tuned() {
                    repo.net_init_sum(&db, &net).unwrap()
                } else {
                    repo.net_init(&db, &net).unwrap()
                };
                for i in 0..n {
                    db = repo
                        .net_fire(
                            &db,
                            &net,
                            "propose",
                            Some(&format!("c{i}")),
                            &format!("p{i}"),
                        )
                        .unwrap()
                        .0;
                }
                repo.db_commit(REF, &db, "proposed").unwrap();
            }
            let t0 = Instant::now();
            let kids: Vec<_> = (0..w)
                .map(|i| {
                    Command::new(&me)
                        .env("E298_CHILD", format!("{d}|{i}|{n}|{mode}"))
                        .output_spawn()
                })
                .collect();
            let (mut runs, mut lost, mut retries, mut errors) = (0, 0, 0, Vec::new());
            for kid in kids {
                let out = kid.wait_with_output().unwrap();
                let s = String::from_utf8_lossy(&out.stdout);
                for line in s.lines() {
                    let mut f = line.splitn(2, ' ');
                    match (f.next(), f.next()) {
                        (Some("runs"), Some(v)) => runs += v.parse::<usize>().unwrap(),
                        (Some("lost"), Some(v)) => lost += v.parse::<usize>().unwrap(),
                        (Some("retries"), Some(v)) => retries += v.parse::<usize>().unwrap(),
                        (Some("error"), Some(v)) => errors.push(v.to_string()),
                        _ => {}
                    }
                }
                if !out.status.success() {
                    errors.push(format!(
                        "exit {}: {}",
                        out.status,
                        String::from_utf8_lossy(&out.stderr)
                            .lines()
                            .find(|l| l.contains("panicked") || l.contains("called"))
                            .unwrap_or("")
                    ));
                }
            }
            let ms = t0.elapsed().as_millis();
            let mut repo = open(&d);
            let db = repo.db_at(&repo.resolve(REF).unwrap()).unwrap();
            let replay = match repo.net_replay(&db, &net) {
                Ok(m) if m == repo.net_marking(&db, "loop").unwrap() => "equal".to_string(),
                Ok(_) => "differs".into(),
                Err(e) => format!("fails ({e})"),
            };
            let fsck = repo.fsck().unwrap().len();
            println!(
                "{:<12} {:>2} {:>9} {:>5} {:>5} {:>8} {:>7} {:>7} {:>8}  {replay}  {fsck}",
                if mode == "at" {
                    "db_commit_at"
                } else {
                    "db_commit"
                },
                w,
                count(&repo, &db, "measured"),
                runs,
                lost,
                retries,
                errors.len(),
                ms,
                repo.net_firings(&db, "loop").unwrap().len(),
            );
            let mut seen = std::collections::BTreeMap::<String, usize>::new();
            for e in &errors {
                *seen.entry(e.clone()).or_default() += 1;
            }
            for (e, c) in seen {
                println!("    {c} x {e}");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

trait SpawnOut {
    fn output_spawn(&mut self) -> std::process::Child;
}
impl SpawnOut for Command {
    fn output_spawn(&mut self) -> std::process::Child {
        self.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }
}

fn main() {
    if let Ok(spec) = std::env::var("E298_CHILD") {
        let f: Vec<&str> = spec.split('|').collect();
        let t = child(
            f[0],
            f[1].parse().unwrap(),
            f[2].parse().unwrap(),
            f[3] == "at",
        );
        println!("runs {}\nlost {}\nretries {}", t.runs, t.lost, t.retries);
        for e in t.errors {
            println!("error {e}");
        }
        return;
    }
    let n = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    parent(n);
}
