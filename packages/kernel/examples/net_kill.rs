//! E301: worker processes killed in the middle of the loop.
//!
//! A night's workers die: the laptop sleeps, a run is killed for memory.
//! This runs W worker processes (as E298) over the loop net's N candidates
//! on one FsStore and, every KILL_MS (env, default 700), kills one at random
//! with SIGKILL and starts a new one in its place. When no candidate is left
//! queued, the candidates a killed worker had started are still in
//! `running`, with nobody to finish them: the parent counts them, puts them
//! back with a `requeue` transition (running -> queued, cpu), and lets W
//! new workers finish without kills. Reported: kills, candidates stranded,
//! measured at the end, replay, fsck, every candidate's content readable,
//! and temporary files left behind.
//!
//!   cargo run --release --example net_kill [-- N]
//!   (W, KILL_MS, WORK_MS as env; DEFERRED=1 opens the store with
//!   `open_deferred`)

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ikam_kernel::net::Net;
use ikam_kernel::{Db, Error, FsStore, Id, Repo};

const REF: &str = "loop";

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn loop_net() -> Net {
    Net::parse(
        "\
net loop 1
place cpu 2
place queued colored
place running colored
place measured colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
transition requeue pure: running -> queued, cpu
",
    )
    .unwrap()
}

fn open(dir: &str) -> Repo<FsStore> {
    let store = if std::env::var("DEFERRED").as_deref() == Ok("1") {
        FsStore::open_deferred(dir).unwrap()
    } else {
        FsStore::open(dir).unwrap()
    };
    let mut repo = Repo::new(store);
    repo.refresh_ledger = true;
    repo.read_weight = 1.0;
    repo.read_cache = 64 << 20;
    repo
}

fn tokens(repo: &Repo<FsStore>, db: &Db, place: &str) -> Result<Vec<String>, Error> {
    Ok(repo
        .net_marking(db, "loop")?
        .get(place)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default())
}

/// The bytes a candidate's run produces: what the finish firing records.
fn output(tok: &str) -> Vec<u8> {
    format!("result of {tok}\n").repeat(200).into_bytes()
}

fn child(dir: &str, w: u64) {
    let net = loop_net();
    let mut repo = open(dir);
    let work = Duration::from_millis(env("WORK_MS", 40));
    let mut k = 0u64;
    let mut idle = 0;
    loop {
        let r: Result<bool, Error> = (|| {
            let head = repo.resolve(REF)?;
            let db = repo.db_at(&head)?;
            let queued = tokens(&repo, &db, "queued")?;
            let Some(tok) = queued
                .get((w as usize * 7 + k as usize) % queued.len().max(1))
                .cloned()
            else {
                return Ok(false);
            };
            k += 1;
            let db = match repo.net_fire(&db, &net, "start", Some(&tok), &format!("w{w}-{k}")) {
                Ok((db, _)) => db,
                Err(_) => return Ok(true), // no cpu free
            };
            if repo.db_commit_at(REF, head, &db, "start").is_err() {
                return Ok(true);
            }
            std::thread::sleep(work);
            loop {
                let head = repo.resolve(REF)?;
                let db = repo.db_at(&head)?;
                let Ok((db, _)) = repo.net_fire_output(
                    &db,
                    &net,
                    "finish",
                    Some(&tok),
                    &format!("fin-{tok}"),
                    &output(&tok),
                ) else {
                    return Ok(true);
                };
                if repo.db_commit_at(REF, head, &db, "finish").is_ok() {
                    return Ok(true);
                }
            }
        })();
        match r {
            Ok(true) => idle = 0,
            Ok(false) => {
                idle += 1;
                if idle > 50 {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                eprintln!("error {e}");
                repo = open(dir);
            }
        }
    }
}

/// REQUEUE=1: what a supervisor does when it sees worker `w` die: put back
/// every candidate whose latest start was `w`'s and that is still running.
fn requeue_dead(repo: &mut Repo<FsStore>, net: &Net, w: u64, seq: &mut u64) -> usize {
    loop {
        let head = repo.resolve(REF).unwrap();
        let db = repo.db_at(&head).unwrap();
        let running = tokens(repo, &db, "running").unwrap();
        let mut last = std::collections::HashMap::new();
        for f in repo.net_firings(&db, "loop").unwrap() {
            if f.transition == "start"
                && let Some(t) = f.token
            {
                last.insert(t, f.key);
            }
        }
        let prefix = format!("w{w}-");
        let mine: Vec<&String> = running
            .iter()
            .filter(|t| last.get(*t).is_some_and(|k| k.starts_with(&prefix)))
            .collect();
        if mine.is_empty() {
            return 0;
        }
        let mut db = db;
        for tok in &mine {
            *seq += 1;
            db = repo
                .net_fire(&db, net, "requeue", Some(tok), &format!("rq{seq}"))
                .unwrap()
                .0;
        }
        if repo.db_commit_at(REF, head, &db, "requeue").is_ok() {
            return mine.len();
        }
    }
}

fn spawn(me: &std::path::Path, d: &str, w: u64) -> Child {
    Command::new(me)
        .env("E301_CHILD", format!("{d}|{w}"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn main() {
    if let Ok(spec) = std::env::var("E301_CHILD") {
        let (d, w) = spec.split_once('|').unwrap();
        child(d, w.parse().unwrap());
        return;
    }
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let (w, kill_ms) = (env("W", 2), env("KILL_MS", 700));
    let net = loop_net();
    let me = std::env::current_exe().unwrap();
    let dir = std::env::temp_dir().join(format!("ikam-e301-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.to_str().unwrap().to_string();
    {
        let mut repo = open(&d);
        let db = repo.db_empty().unwrap();
        let mut db = repo.net_init_sum(&db, &net).unwrap();
        for i in 0..n {
            db = repo
                .net_fire(
                    &db,
                    &net,
                    "propose",
                    Some(&format!("c{i:03}")),
                    &format!("p{i}"),
                )
                .unwrap()
                .0;
        }
        repo.db_commit(REF, &db, "proposed").unwrap();
    }
    let t0 = Instant::now();
    let mut next = 0u64;
    let mut kids: Vec<Child> = (0..w)
        .map(|_| {
            next += 1;
            spawn(&me, &d, next)
        })
        .collect();
    let (mut kills, mut errors) = (0, 0);
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut last_kill = Instant::now();
    let supervise = std::env::var("REQUEUE").as_deref() == Ok("1");
    let mut sup = open(&d);
    let (mut seq, mut requeued) = (0u64, 0usize);
    let mut ids: Vec<u64> = (1..=w).collect();
    while !kids.is_empty() && t0.elapsed() < Duration::from_secs(120) {
        std::thread::sleep(Duration::from_millis(5));
        let mut keep = Vec::new();
        for k in kids.iter_mut() {
            keep.push(!matches!(k.try_wait(), Ok(Some(_))));
        }
        let mut it = keep.iter();
        ids.retain(|_| *it.next().unwrap());
        kids.retain_mut(|k| match k.try_wait() {
            Ok(Some(_)) => {
                let mut s = String::new();
                use std::io::Read;
                let _ = k.stderr.take().map(|mut e| e.read_to_string(&mut s));
                errors += s.lines().filter(|l| l.starts_with("error")).count();
                false
            }
            _ => true,
        });
        if last_kill.elapsed() >= Duration::from_millis(kill_ms) && !kids.is_empty() {
            last_kill = Instant::now();
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let i = (rng % kids.len() as u64) as usize;
            if kids[i].try_wait().ok().flatten().is_none() {
                let mut gone = kids.remove(i);
                let _ = gone.kill();
                let _ = gone.wait();
                let dead = ids.remove(i);
                if supervise {
                    requeued += requeue_dead(&mut sup, &net, dead, &mut seq);
                }
                kills += 1;
                next += 1;
                kids.push(spawn(&me, &d, next));
                ids.push(next);
            }
        }
    }
    let first_ms = t0.elapsed().as_millis();
    drop(sup);
    let mut repo = open(&d);
    let head = repo.resolve(REF).unwrap();
    let db = repo.db_at(&head).unwrap();
    let stranded = tokens(&repo, &db, "running").unwrap();
    let measured_first = tokens(&repo, &db, "measured").unwrap().len();
    let mut db = db;
    for (i, tok) in stranded.iter().enumerate() {
        db = repo
            .net_fire(&db, &net, "requeue", Some(tok), &format!("rq{i}"))
            .unwrap()
            .0;
    }
    repo.db_commit_at(REF, head, &db, "requeue").unwrap();
    drop(repo);
    let t1 = Instant::now();
    let kids: Vec<Child> = (0..w)
        .map(|_| {
            next += 1;
            spawn(&me, &d, next)
        })
        .collect();
    for k in kids {
        let out = k.wait_with_output().unwrap();
        errors += String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| l.starts_with("error"))
            .count();
    }
    let second_ms = t1.elapsed().as_millis();
    let mut repo = open(&d);
    let db = repo.db_at(&repo.resolve(REF).unwrap()).unwrap();
    let measured = tokens(&repo, &db, "measured").unwrap();
    let replay = match repo.net_replay(&db, &net) {
        Ok(m) if m == repo.net_marking(&db, "loop").unwrap() => "equal".to_string(),
        Ok(_) => "differs".into(),
        Err(e) => format!("fails ({e})"),
    };
    let unreadable = measured
        .iter()
        .filter(|tok| {
            repo.net_output(&db, "loop", &format!("fin-{tok}"))
                .ok()
                .flatten()
                .map(|(id, bytes)| id != Id::of_content(&output(tok)) || bytes != output(tok))
                .unwrap_or(true)
        })
        .count();
    let fsck = repo.fsck().unwrap().len();
    let mut leftovers = 0;
    for sub in std::fs::read_dir(dir.join("objects")).unwrap() {
        for f in std::fs::read_dir(sub.unwrap().path()).unwrap() {
            if f.unwrap().file_name().to_string_lossy().starts_with('.') {
                leftovers += 1;
            }
        }
    }
    println!(
        "{w} workers, {n} candidates, a kill every {kill_ms} ms{}: {kills} kills ({requeued} candidates requeued by the supervisor), {errors} errors; \
         first pass {first_ms} ms, {measured_first} measured, {} stranded in running; \
         after requeue {second_ms} ms: {} measured, replay {replay}, {unreadable} outputs wrong or unreadable, \
         fsck {fsck}, {leftovers} temporary files left",
        match (std::env::var("DEFERRED").as_deref() == Ok("1"), supervise) {
            (true, true) => ", deferred, supervised",
            (true, false) => ", deferred",
            (false, true) => ", supervised",
            _ => "",
        },
        stranded.len(),
        measured.len(),
    );
    let _ = std::fs::remove_dir_all(&dir);
}
