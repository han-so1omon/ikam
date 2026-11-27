//! E422: nets as files, firings naming the version they ran under.
//!
//! 1. A run: three nets in one graph (night, train, small), fired
//!    interleaved for N candidates, the night net edited every N/10
//!    candidates and committed as a file each time; the same run on
//!    untracked nets for comparison. Firing time, record bytes, replay.
//! 2. History: every commit's night net read back; old firings replay
//!    against the versions they name.
//! 3. A net stored as a file (text) against a net stored as a graph object
//!    (`put_net`): bytes a version, bytes an edit adds, read time.
//!
//!   cargo run --release --example e422_net_files [-- N]

use std::time::Instant;

use ikam_kernel::net::Net;
use ikam_kernel::{Db, Id, MemStore, Repo, Store};

const NIGHT: &str = "\
net night 1
place cpu 4
place queued colored
place running colored
place measured colored
transition propose agent: -> queued
transition start cmd: queued, cpu -> running
transition finish pure: running -> measured, cpu
";
const TRAIN: &str = "\
net train 1
place gpu 2
place batch colored
place trained colored
transition load cmd: -> batch
transition step cmd: batch, gpu -> trained, gpu
";
const SMALL: &str = "\
net small 1
place a 1000000
place b
transition move pure: a -> b
";

fn size(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// The night net's version `k`: version 1, and from version 2 a check on
/// measured candidates whose name changes with each version (`check<k>`),
/// so no single version can replay the whole run.
fn night(k: u32) -> Net {
    let mut text = NIGHT.replace("net night 1", &format!("net night {k}"));
    if k > 1 {
        text += "place checked colored\n";
        text += &format!("transition check{k} pure: measured -> checked\n");
    }
    Net::parse(&text).unwrap()
}

struct Run {
    fire_ms: f64,
    firings: usize,
    record_bytes: usize,
    replay_ms: f64,
    ok: bool,
    commits: Vec<(Id, u32)>,
}

fn run(repo: &mut Repo<MemStore>, n: usize, track: bool, sum: bool) -> (Db, Run) {
    let (train, small) = (Net::parse(TRAIN).unwrap(), Net::parse(SMALL).unwrap());
    let mut db = repo.db_empty().unwrap();
    let mut cur = night(1);
    for net in [&cur, &train, &small] {
        db = if sum {
            repo.net_init_sum(&db, net).unwrap()
        } else {
            repo.net_init(&db, net).unwrap()
        };
        if track {
            db = repo.net_track_versions(&db, net).unwrap();
        }
    }
    let every = (n / 10).max(1);
    let (mut tree, mut commits) = (None, Vec::new());
    let mut commit = |repo: &mut Repo<MemStore>, net: &Net, tree: &mut Option<Id>| {
        let (t, _) = repo.put_net_file(*tree, "nets/night.net", net).unwrap();
        *tree = Some(t);
        commits.push((repo.commit("main", t, "night").unwrap(), net.version));
    };
    for net in [&train, &small] {
        tree = Some(
            repo.put_net_file(tree, &format!("nets/{}.net", net.name), net)
                .unwrap()
                .0,
        );
    }
    commit(repo, &cur, &mut tree);
    let (mut fire_ms, mut firings) = (0.0, 0);
    let mut fire = |repo: &mut Repo<MemStore>,
                    db: &mut Db,
                    net: &Net,
                    t: &str,
                    tok: Option<&str>,
                    key: String| {
        let at = Instant::now();
        *db = repo.net_fire(db, net, t, tok, &key).unwrap().0;
        fire_ms += ms(at);
        firings += 1;
    };
    for i in 0..n {
        if i > 0 && i % every == 0 {
            cur = night(cur.version + 1);
            commit(repo, &cur, &mut tree);
        }
        let c = format!("c{i}");
        let tok = Some(c.as_str());
        fire(repo, &mut db, &cur, "propose", tok, format!("p{i}"));
        fire(repo, &mut db, &train, "load", tok, format!("l{i}"));
        fire(repo, &mut db, &cur, "start", tok, format!("s{i}"));
        fire(repo, &mut db, &small, "move", None, format!("m{i}"));
        fire(repo, &mut db, &train, "step", tok, format!("t{i}"));
        fire(repo, &mut db, &cur, "finish", tok, format!("f{i}"));
        if cur.version > 1 {
            fire(
                repo,
                &mut db,
                &cur,
                &format!("check{}", cur.version),
                tok,
                format!("k{i}"),
            );
        }
    }
    let mut record_bytes = 0;
    for name in ["night", "train", "small"] {
        for f in repo.net_firings(&db, name).unwrap() {
            record_bytes += repo
                .db_get(&db, &format!("net/{name}/fire/{}", f.key))
                .unwrap()
                .unwrap()
                .len();
        }
    }
    let at = Instant::now();
    let ok = if track {
        ["night", "train", "small"].iter().all(|name| {
            repo.net_replay_versions(&db, name).unwrap() == repo.net_marking(&db, name).unwrap()
        })
    } else {
        // Untracked: replay against the last version of each net.
        [&cur, &train, &small].iter().all(|net| {
            repo.net_replay(&db, net)
                .is_ok_and(|m| m == repo.net_marking(&db, &net.name).unwrap())
        })
    };
    let replay_ms = ms(at);
    (
        db,
        Run {
            fire_ms,
            firings,
            record_bytes,
            replay_ms,
            ok,
            commits,
        },
    )
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);

    println!("## 1. a run: three nets, {n} candidates, the night net edited 9 times\n");
    println!("| nets | firings | fire µs each | record bytes each | replay ms | replays |");
    println!("| --- | --- | --- | --- | --- | --- |");
    let mut results = Vec::new();
    for (track, sum) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut repo = Repo::new(MemStore::default());
        let (db, r) = run(&mut repo, n, track, sum);
        println!(
            "| {}{} | {} | {:.1} | {:.1} | {:.1} | {} |",
            if track { "tracked" } else { "untracked" },
            if sum { ", sum hash" } else { "" },
            r.firings,
            r.fire_ms * 1e3 / r.firings as f64,
            r.record_bytes as f64 / r.firings as f64,
            r.replay_ms,
            match (r.ok, track) {
                (true, _) => "equal marking",
                (false, true) => "FAILS",
                (false, false) => "fails (no one version holds every transition fired)",
            },
        );
        results.push((repo, db, r));
    }

    println!("\n## 2. history\n");
    let (repo, db, r) = &results[1];
    let mut read_ok = 0;
    for (c, k) in &r.commits {
        if repo.net_file(c, "nets/night.net").unwrap() == night(*k) {
            read_ok += 1;
        }
    }
    let firings = repo.net_firings(db, "night").unwrap();
    let mut by_version: std::collections::BTreeMap<u32, usize> = Default::default();
    for f in &firings {
        let v = repo.net_version(&f.net.unwrap()).unwrap().version;
        *by_version.entry(v).or_default() += 1;
    }
    println!(
        "- {} commits; the night net read back at each: {read_ok}/{}",
        r.commits.len(),
        r.commits.len()
    );
    println!("- night firings by the version they name: {by_version:?}");
    let last = r.commits.last().unwrap().0;
    println!(
        "- nets in the last commit: {:?}",
        repo.net_files(&last)
            .unwrap()
            .iter()
            .map(|(p, _)| p.as_str())
            .collect::<Vec<_>>()
    );
    let (_, _, ru) = &results[0];
    let untracked_old = {
        let (repo, db, _) = &results[0];
        repo.net_replay(db, &night(1)).is_ok()
    };
    println!(
        "- untracked, replaying against version 1 instead of the last: {}",
        if untracked_old {
            "passes"
        } else {
            "fails (the run used versions it cannot name)"
        }
    );
    println!(
        "- replay against the last version only, untracked: {} ({} firings)",
        if ru.ok {
            "passes"
        } else {
            "fails (earlier checks were renamed)"
        },
        ru.firings
    );
    println!("- fsck_deep failures: {}", repo.fsck_deep().unwrap().len());

    println!("\n## 3. a net as a file or as a graph object\n");
    println!("| form | bytes v1 | bytes an edit adds | read µs |");
    println!("| --- | --- | --- | --- |");
    for form in ["file", "graph"] {
        let mut repo = Repo::new(MemStore::default());
        let mut added = Vec::new();
        let mut tree = None;
        let mut ids = Vec::new();
        let mut first = 0;
        for k in 1..=10 {
            let before = size(&repo);
            let net = night(k);
            let id = if form == "file" {
                let (t, _) = repo.put_net_file(tree, "night.net", &net).unwrap();
                tree = Some(t);
                t
            } else {
                repo.put_net(&net).unwrap()
            };
            ids.push(id);
            let grown = size(&repo) - before;
            if k == 1 {
                first = grown;
            } else {
                added.push(grown);
            }
        }
        let at = Instant::now();
        let reps = 200;
        for _ in 0..reps {
            for (i, id) in ids.iter().enumerate() {
                let net = if form == "file" {
                    repo.net_file(id, "night.net").unwrap()
                } else {
                    repo.net_at(id).unwrap()
                };
                assert_eq!(net.version, i as u32 + 1);
            }
        }
        let read_us = ms(at) * 1e3 / (reps * ids.len()) as f64;
        println!(
            "| {form} | {first} | {:.0} | {read_us:.1} |",
            added.iter().sum::<usize>() as f64 / added.len() as f64
        );
    }
    let (a, b) = (night(1).to_text(), night(2).to_text());
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let changed = lb.iter().filter(|l| !la.contains(l)).count()
        + la.iter().filter(|l| !lb.contains(l)).count();
    println!(
        "\n- text diff, version 1 to 2: {changed} lines of {}",
        lb.len()
    );
}
