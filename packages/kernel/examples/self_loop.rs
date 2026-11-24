//! E400: the self-improvement loop's state on an IKAM store. Two levels in
//! one net: candidates (one change to the kernel each) go through propose,
//! start, finish and keep or close, as in the night net; ranking policies go
//! through variant and adopt or reject, and the policy in use is retired when
//! another is adopted. Every firing's output is the record of that step (a
//! candidate's spec, its measured result, a policy and its replay score), so
//! the loop's whole history is the net's firings and nothing else.
//!
//!   cargo run --release --example self_loop -- STORE fire T TOKEN [FILE]
//!   cargo run --release --example self_loop -- STORE history
//!   cargo run --release --example self_loop -- STORE check
//!
//! `fire` fires one transition for TOKEN (key `T-TOKEN`, so a retry changes
//! nothing), with FILE's bytes as its output, and commits to the ref
//! `loop/self`. `history` prints every firing in log order as a JSON line,
//! with its output when it has one. `check` reads the store alone: replay
//! equal to the marking, every output readable, fsck clean, at most one
//! policy in use, and no `merge` firing (merging is the owner's, a human
//! transition the loop never fires). The driver is
//! `experiments/self-loop/loop.py`. No kernel change.

#[path = "loop_common/mod.rs"]
mod loop_common;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ikam_kernel::net::Net;
use ikam_kernel::{Db, FsStore, Repo};
use loop_common::json_str;

const SELF: &str = "\
net self 1
place cpu 1
place queued colored
place running colored
place measured colored
place waiting colored
place closed colored
place policy colored
place trial colored
place retired colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
transition keep pure: measured -> waiting
transition close pure: measured -> closed
transition merge human: waiting ->
transition variant agent rank: -> trial
transition adopt pure: trial -> policy
transition reject pure: trial -> retired
transition retire pure: policy -> retired
";
const REF: &str = "loop/self";

fn open(dir: &PathBuf, net: &Net) -> (Repo<FsStore>, Db) {
    std::fs::create_dir_all(dir).unwrap();
    let mut repo = Repo::new(FsStore::open(dir).unwrap());
    let db = match repo.resolve(REF) {
        Ok(c) => repo.db_at(&c).unwrap(),
        Err(_) => {
            let db = repo.db_empty().unwrap();
            repo.net_init_sum(&db, net).unwrap()
        }
    };
    (repo, db)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, cmd) = (PathBuf::from(&args[0]), args[1].as_str());
    let net = Net::parse(SELF).unwrap();
    let (mut repo, db) = open(&dir, &net);
    match cmd {
        "fire" => {
            let (t, tok) = (&args[2], &args[3]);
            assert!(
                t != "merge",
                "merge is a human transition: the loop never fires it"
            );
            let out = args.get(4).map(|f| std::fs::read(f).unwrap());
            let key = format!("{t}-{tok}");
            let r = match &out {
                Some(b) => repo.net_fire_output(&db, &net, t, Some(tok), &key, b),
                None => repo.net_fire(&db, &net, t, Some(tok), &key),
            };
            let db = r.unwrap_or_else(|e| panic!("{key}: {e:?}")).0;
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let c = repo.db_commit(REF, &db, &format!("{key} at {at}")).unwrap();
            println!("{key} -> {}", &c.to_string()[..12]);
        }
        "history" => {
            for f in repo.net_firings(&db, "self").unwrap() {
                let out = match repo.net_output(&db, "self", &f.key).unwrap() {
                    Some((_, b)) => json_str(&String::from_utf8(b).unwrap()),
                    None => "null".into(),
                };
                println!(
                    "{{\"key\":{},\"transition\":{},\"token\":{},\"output\":{out}}}",
                    json_str(&f.key),
                    json_str(&f.transition),
                    json_str(f.token.as_deref().unwrap_or(""))
                );
            }
        }
        "check" => check(&repo, &db, &net),
        _ => panic!("unknown command {cmd}"),
    }
}

fn check(repo: &Repo<FsStore>, db: &Db, net: &Net) {
    let m = repo.net_marking(db, "self").unwrap();
    assert_eq!(repo.net_replay(db, net).unwrap(), m, "replay");
    assert!(repo.fsck().unwrap().is_empty(), "fsck");
    let firings = repo.net_firings(db, "self").unwrap();
    for f in &firings {
        assert!(f.transition != "merge", "a merge was fired");
        if f.output.is_some() {
            assert!(repo.net_output(db, "self", &f.key).unwrap().is_some());
        }
    }
    let held = |p: &str| m.get(p).map_or(0, |t| t.values().sum::<u64>());
    assert!(held("policy") <= 1, "more than one policy in use");
    let places = [
        "queued", "running", "waiting", "closed", "policy", "retired",
    ];
    let counts: Vec<String> = places.iter().map(|p| format!("{p} {}", held(p))).collect();
    println!(
        "{REF}: {} firings, replay equal, outputs readable, fsck clean; {}",
        firings.len(),
        counts.join(", ")
    );
}
