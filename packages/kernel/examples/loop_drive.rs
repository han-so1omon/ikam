//! E386: a night driven by its loop net, one firing at a time, as the work
//! happens (E373's journal recorded the night after it was done).
//!
//! The net is `loop_journal`'s night net. Each call opens the store, fires
//! one transition, commits to the ref `night/<LABEL>` and exits, so a killed
//! runner loses nothing it reported: the next call starts from the stored
//! marking. The agent asks the net what to do next instead of choosing:
//!
//!   cargo run --release --example loop_drive -- STORE LABEL plan E386 E387 ...
//!   cargo run --release --example loop_drive -- STORE LABEL next
//!   cargo run --release --example loop_drive -- STORE LABEL start|keep|close ENNN
//!   cargo run --release --example loop_drive -- STORE LABEL finish ENNN [LOG_MD]
//!   cargo run --release --example loop_drive -- STORE LABEL check [LOG_MD]
//!
//! `plan` fires `propose` for each id (a token on `queued`). `next` prints
//! the first firing the net enables (`net_enabled_limit`, E280, human
//! transitions left out): `start` of the lowest queued id while the cpu is
//! free, else `finish` of the running one. `finish` records the
//! experiment's log entry (from LOG_MD, default this checkout's log) as the
//! firing's output (E271), so fire it only once the entry is final; an
//! entry edited later shows as a mismatch in `check`. `check` reads the
//! store alone: replay equal to the marking, each finish output equal to
//! the entry in LOG_MD (or reported as changed), fsck clean, and the order
//! of the night from the ref's commits. No kernel change.

#[path = "loop_common/mod.rs"]
mod loop_common;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ikam_kernel::net::Net;
use ikam_kernel::{Db, FsStore, Repo};
use loop_common::docs;
use loop_common::entries;

const NIGHT: &str = "\
net night 1
place cpu 1
place queued colored
place running colored
place measured colored
place waiting colored
place closed colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
transition keep pure: measured -> waiting
transition close pure: measured -> closed
transition merge human: waiting ->
";

fn entry(log: &Path, tok: &str) -> String {
    let id: u32 = tok.trim_start_matches('E').parse().expect("id like E386");
    let dir = log
        .parent()
        .and_then(Path::parent)
        .expect("LOG_MD under docs/experiments");
    entries(dir)
        .remove(&id)
        .unwrap_or_else(|| panic!("no log entry for {tok} in {}", log.display()))
}

fn fire(
    repo: &mut Repo<FsStore>,
    net: &Net,
    db: &Db,
    t: &str,
    tok: &str,
    out: Option<&[u8]>,
) -> Db {
    let key = format!("{t}-{tok}");
    let r = match out {
        Some(b) => repo.net_fire_output(db, net, t, Some(tok), &key, b),
        None => repo.net_fire(db, net, t, Some(tok), &key),
    };
    r.unwrap_or_else(|e| panic!("{key}: {e:?}")).0
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, label, cmd) = (PathBuf::from(&args[0]), &args[1], args[2].as_str());
    let rest = &args[3..];
    let default_log = docs().join("experiments/log.md");
    std::fs::create_dir_all(&dir).unwrap();
    let net = Net::parse(NIGHT).unwrap();
    let refname = format!("night/{label}");
    let mut repo = Repo::new(FsStore::open(&dir).unwrap());
    let mut db = match repo.resolve(&refname) {
        Ok(c) => repo.db_at(&c).unwrap(),
        Err(_) => {
            let db = repo.db_empty().unwrap();
            repo.net_init_sum(&db, &net).unwrap()
        }
    };
    match cmd {
        "plan" => {
            for tok in rest {
                db = fire(&mut repo, &net, &db, "propose", tok, None);
            }
        }
        "start" | "keep" | "close" => db = fire(&mut repo, &net, &db, cmd, &rest[0], None),
        "finish" => {
            let log = rest.get(1).map(PathBuf::from).unwrap_or(default_log);
            let e = entry(&log, &rest[0]);
            db = fire(&mut repo, &net, &db, "finish", &rest[0], Some(e.as_bytes()));
        }
        "next" => {
            let next = repo.net_enabled_limit(&db, &net, true, 1).unwrap();
            match next.first() {
                Some((t, tok)) => println!("{t} {}", tok.as_deref().unwrap_or("")),
                None => println!("idle"),
            }
            return;
        }
        "check" => {
            let log = rest.first().map(PathBuf::from).unwrap_or(default_log);
            return check(&repo, &db, &net, &refname, &log);
        }
        _ => panic!("unknown command {cmd}"),
    }
    let msg = format!("{cmd} {} at {}", rest.join(" "), now());
    let c = repo.db_commit(&refname, &db, &msg).unwrap();
    println!("{msg} -> {}", &c.to_string()[..12]);
}

fn check(repo: &Repo<FsStore>, db: &Db, net: &Net, refname: &str, log: &Path) {
    let m = repo.net_marking(db, "night").unwrap();
    assert_eq!(repo.net_replay(db, net).unwrap(), m, "replay");
    assert!(repo.fsck().unwrap().is_empty(), "fsck");
    let firings = repo.net_firings(db, "night").unwrap();
    let mut changed = 0;
    for f in firings.iter().filter(|f| f.transition == "finish") {
        let tok = f.token.as_deref().unwrap();
        let same = repo
            .net_verify(db, "night", &f.key, entry(log, tok).as_bytes())
            .unwrap();
        if same != Some(true) {
            changed += 1;
            println!("  {tok}: log entry changed since its finish firing");
        }
    }
    let places = ["queued", "running", "waiting", "closed"];
    let held: Vec<String> = places
        .iter()
        .map(|p| format!("{p} {}", m.get(*p).map_or(0, |t| t.len())))
        .collect();
    println!(
        "{refname}: {} firings, replay equal, fsck clean, {} of {} finish outputs equal to the log; {}",
        firings.len(),
        firings.iter().filter(|f| f.transition == "finish").count() - changed,
        firings.iter().filter(|f| f.transition == "finish").count(),
        held.join(", ")
    );
    let Ok(head) = repo.resolve(refname) else {
        return;
    };
    let mut log = repo.log(head).unwrap();
    log.reverse();
    for (_, c) in log {
        println!("  {}", c.message);
    }
}
