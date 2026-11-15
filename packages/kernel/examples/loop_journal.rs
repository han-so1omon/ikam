//! The night's runs recorded in a loop net on an IKAM store: each
//! experiment from FIRST to LAST is a token taken through propose, start
//! and finish (the finish firing names the experiment's log entry as its
//! output, E271), then to `waiting` (kept: an option for the owner to
//! merge, a human transition) or `closed` (measured or refuted: nothing to
//! merge). The net uses the sum hash (E279) and the store is an FsStore
//! committed to the ref `night/<LABEL>`; reruns change nothing (idempotent
//! keys). Then the run is checked from disk: replay, every output equal to
//! the log entry, fsck.
//!
//!   cargo run --release --example loop_journal -- FIRST LAST LABEL [STORE_DIR] [--json]
//!
//! STORE_DIR defaults to target/loop.ikam. No kernel change.

#[path = "loop_common/mod.rs"]
mod loop_common;

use ikam_kernel::net::Net;
use ikam_kernel::{FsStore, Repo};
use loop_common::*;

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let first: u32 = pos[0].trim_start_matches('E').parse().unwrap();
    let last: u32 = pos[1].trim_start_matches('E').parse().unwrap();
    let label = pos[2];
    let dir = pos.get(3).map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/loop.ikam")
    });
    std::fs::create_dir_all(&dir).unwrap();
    let docs = docs();
    let rows = rows(&docs);
    let entries = entries(&docs);
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
    let mut done = Vec::new();
    for id in first..=last {
        let (Some(row), Some(entry)) = (rows.iter().find(|r| r.id == id), entries.get(&id)) else {
            continue;
        };
        let tok = format!("E{id:03}");
        let close = if row.verdict == "kept" {
            "keep"
        } else {
            "close"
        };
        for t in ["propose", "start", "finish", close] {
            let key = format!("{t}-{tok}");
            db = if t == "finish" {
                repo.net_fire_output(&db, &net, t, Some(&tok), &key, entry.as_bytes())
            } else {
                repo.net_fire(&db, &net, t, Some(&tok), &key)
            }
            .unwrap()
            .0;
        }
        done.push((tok, row.clone(), entry.clone()));
    }
    let commit = repo.db_commit(&refname, &db, "night journal").unwrap();

    // Check from disk alone.
    let mut repo = Repo::new(FsStore::open(&dir).unwrap());
    let db = repo.db_at(&repo.resolve(&refname).unwrap()).unwrap();
    let m = repo.net_marking(&db, "night").unwrap();
    assert_eq!(repo.net_replay(&db, &net).unwrap(), m, "replay");
    for (tok, _, entry) in &done {
        let ok = repo
            .net_verify(&db, "night", &format!("finish-{tok}"), entry.as_bytes())
            .unwrap();
        assert_eq!(ok, Some(true), "output of {tok}");
    }
    assert!(repo.fsck().unwrap().is_empty(), "fsck");
    let firings = repo.net_firings(&db, "night").unwrap();
    let place = |tok: &str| {
        ["waiting", "closed"]
            .into_iter()
            .find(|p| m.get(*p).is_some_and(|t| t.contains_key(tok)))
            .unwrap_or("?")
    };
    if json {
        for (tok, row, _) in &done {
            let out = firings
                .iter()
                .find(|f| f.key == format!("finish-{tok}"))
                .and_then(|f| f.output)
                .unwrap();
            println!(
                "{{\"id\":{},\"slug\":{},\"verdict\":{},\"place\":{},\"output\":{}}}",
                json_str(tok),
                json_str(&row.slug),
                json_str(&row.verdict),
                json_str(place(tok)),
                json_str(&out.to_string()[..12])
            );
        }
        return;
    }
    println!(
        "night/{label}: commit {}, {} experiments, {} firings, replay equal, outputs equal to the log, fsck clean",
        &commit.to_string()[..12],
        done.len(),
        firings.len()
    );
    for (tok, row, _) in &done {
        println!(
            "  {tok} {:<22} {:<9} -> {}",
            row.slug,
            row.verdict,
            place(tok)
        );
    }
}
