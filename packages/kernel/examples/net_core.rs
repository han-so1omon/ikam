//! E269: a Petri net of work states on the database log.
//!
//! Runs the experiment-loop net (candidates queue, take one cpu slot, run,
//! free it, wait for the owner, and the owner's merge) for N candidates,
//! five firings each, and measures:
//!
//! - fire latency (p50, p99) on MemStore and on FsStore;
//! - that replaying the recorded firings alone gives the stored marking;
//! - that a retried firing (same key) changes nothing and a reused key for
//!   another firing is refused;
//! - that the scheduler's enabled list never offers the human transition;
//! - that the database folds to the same views from scratch, and fsck.
//!
//! Then the same run with the owner never merging, so finished candidates
//! pile up in `waiting`: a firing reads the whole marking, so its cost
//! grows with the tokens held.
//!
//!   cargo run --release --example net_core [-- N]
//!
//! N defaults to 2000 (10000 firings). Nothing outside a temp directory is
//! written.

use std::time::Instant;

use ikam_kernel::net::{Arc, Kind, Net, Place, Transition};
use ikam_kernel::{Error, FsStore, MemStore, Repo, Store};

fn net() -> Net {
    let place = |n: &str, colored: bool, initial: u32| Place {
        name: n.into(),
        colored,
        initial,
    };
    let arcs = |ps: &[&str]| {
        ps.iter()
            .map(|p| Arc {
                place: p.to_string(),
                weight: 1,
            })
            .collect()
    };
    let tr = |name: &str, i: &[&str], o: &[&str], kind: Kind, cap: Option<&str>| Transition {
        name: name.into(),
        inputs: arcs(i),
        outputs: arcs(o),
        kind,
        capability: cap.map(String::from),
    };
    Net {
        name: "loop".into(),
        version: 1,
        places: vec![
            place("cpu", false, 1),
            place("queued", true, 0),
            place("running", true, 0),
            place("measured", true, 0),
            place("waiting", true, 0),
        ],
        transitions: vec![
            tr("propose", &[], &["queued"], Kind::Agent, Some("propose")),
            tr(
                "start",
                &["queued", "cpu"],
                &["running"],
                Kind::Cmd,
                Some("bench"),
            ),
            tr(
                "finish",
                &["running"],
                &["measured", "cpu"],
                Kind::Pure,
                None,
            ),
            tr("ask", &["measured"], &["waiting"], Kind::Pure, None),
            tr("merge", &["waiting"], &[], Kind::Human, None),
        ],
    }
}

fn pct(v: &mut [u128], p: f64) -> f64 {
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p) as usize] as f64 / 1000.0
}

struct Run {
    p50: f64,
    p99: f64,
    firings: usize,
    replay_ms: f64,
    last_tokens: usize,
}

fn run<S: Store>(repo: &mut Repo<S>, n: usize, merge: bool) -> Run {
    let net = net();
    let g = repo.put_net(&net).unwrap();
    assert_eq!(repo.net_at(&g).unwrap().transitions, net.transitions);
    let db = repo.db_empty().unwrap();
    let mut db = repo.net_init(&db, &net).unwrap();
    let mut us: Vec<u128> = Vec::new();
    let steps: &[&str] = if merge {
        &["propose", "start", "finish", "ask", "merge"]
    } else {
        &["propose", "start", "finish", "ask"]
    };
    for i in 0..n {
        let tok = format!("E{i:05}");
        for t in steps {
            let key = format!("{t}-{tok}");
            let start = Instant::now();
            let (next, _) = repo.net_fire(&db, &net, t, Some(&tok), &key).unwrap();
            us.push(start.elapsed().as_micros());
            db = next;
        }
        if i == 0 {
            // A retry is a no-op; a reused key for another firing is refused.
            let (same, _) = repo
                .net_fire(&db, &net, "ask", Some(&tok), &format!("ask-{tok}"))
                .unwrap();
            assert_eq!(same, db, "a retried firing changed the database");
            assert!(matches!(
                repo.net_fire(&db, &net, "start", Some("E99999"), &format!("start-{tok}")),
                Err(Error::IdempotencyConflict(_))
            ));
        }
        // The scheduler is never offered the owner's transition.
        if i % 97 == 0 {
            let auto = repo.net_enabled(&db, &net, true).unwrap();
            assert!(auto.iter().all(|(t, _)| t != "merge"));
        }
    }
    let marking = repo.net_marking(&db, "loop").unwrap();
    let start = Instant::now();
    let replayed = repo.net_replay(&db, &net).unwrap();
    let replay_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(replayed, marking, "replay differs from the stored marking");
    assert_eq!(repo.db_rebuild(&db.log).unwrap().kv, db.kv);
    let firings = repo.net_firings(&db, "loop").unwrap().len();
    assert_eq!(firings, n * steps.len());
    let last_tokens = marking.values().map(|t| t.len()).sum();
    Run {
        p50: pct(&mut us, 0.5),
        p99: pct(&mut us, 0.99),
        firings,
        replay_ms,
        last_tokens,
    }
}

fn main() {
    let n: usize = (std::env::args().nth(1))
        .and_then(|a| a.parse().ok())
        .unwrap_or(2000);
    let mut mem = Repo::new(MemStore::default());
    let r = run(&mut mem, n, true);
    println!(
        "mem, owner merges: {} firings, fire p50 {:.2} ms p99 {:.2} ms, replay {:.0} ms, {} tokens at the end",
        r.firings, r.p50, r.p99, r.replay_ms, r.last_tokens
    );
    let dir = tempfile::tempdir().unwrap();
    let mut fs = Repo::new(FsStore::open(dir.path()).unwrap());
    let r = run(&mut fs, n, true);
    assert!(fs.fsck().unwrap().is_empty(), "fsck");
    println!(
        "fs, owner merges: {} firings, fire p50 {:.2} ms p99 {:.2} ms, replay {:.0} ms, fsck clean",
        r.firings, r.p50, r.p99, r.replay_ms
    );
    let mut mem = Repo::new(MemStore::default());
    let r = run(&mut mem, n, false);
    println!(
        "mem, owner never merges: {} firings, fire p50 {:.2} ms p99 {:.2} ms, {} tokens waiting at the end",
        r.firings, r.p50, r.p99, r.last_tokens
    );
}
