//! E265: what a merge costs. `cargo run --release --example branch_merge -- [N...]`
//!
//! A repository with a database of N generated rows and N/10 generated
//! files (in 100 folders) on `main`, and a branch `side` at the same
//! commit. Each side then makes C changes (C = 100 and 1,000: database
//! upserts and one file edit per ten), committed in batches of 100, and
//! `side` is merged into `main`. With no shared keys the merge must
//! succeed and its views equal a rebuild of the merged log; with 10% of
//! `side`'s keys also written by `main`, it must report exactly those.
//! Prints one JSON line per case: merge milliseconds, objects and bytes
//! read from the store, and the milliseconds a full rebuild of the merged
//! log takes (what a merge that refolds everything would pay). As a
//! control, the same events appended straight onto main's head, on a copy
//! of the store, with and without `Repo::needed_index` (E129): today's
//! append can scan the store when it drops an object, and that is not the
//! merge's cost.
//! Fixtures are synthetic.

mod db_common;

use std::collections::BTreeSet;
use std::time::Instant;

use db_common::{Counting, Lcg, row};
use ikam_kernel::merge::{Conflict, Merged};
use ikam_kernel::{Edit, Event, Id, Object, Repo};

fn tree_of(repo: &Repo<Counting>, commit: Id) -> Id {
    match repo.get(&commit).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    }
}

fn build(n: u64) -> Repo<Counting> {
    let mut repo = Repo::new(Counting::default());
    let mut rng = Lcg(42);
    let files = n / 10;
    let edits: Vec<Edit> = (0..files)
        .map(|i| Edit::Put {
            path: format!("d{:02}/f{i:07}.csv", i % 100),
            id: repo.put_content(row(&mut rng, i).as_bytes()).unwrap().id,
        })
        .collect();
    let tree = repo.edit_tree(None, &edits).unwrap();
    repo.commit("main", tree, "files").unwrap();
    let mut db = repo.db_empty().unwrap();
    let mut start = 0;
    while start < n {
        let end = (start + 10_000).min(n);
        let events: Vec<Event> = (start..end)
            .map(|i| Event::upsert(&key(i), row(&mut rng, i).as_bytes(), &format!("load-{i}")))
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        start = end;
    }
    repo.db_commit("main", &db, "load").unwrap();
    repo
}

fn key(i: u64) -> String {
    format!("t/{i:09}")
}

/// Changes on `name`, in commits of 100: upserts of the given keys and a
/// file edit for every tenth, in files of this side's parity.
fn changes(repo: &mut Repo<Counting>, name: &str, keys: &[u64], files: u64, side: u64) {
    let tag = ["m", "s"][side as usize];
    let mut rng = Lcg(keys.first().copied().unwrap_or(1) ^ 99);
    for chunk in keys.chunks(100) {
        let head = repo.resolve(name).unwrap();
        let db = repo.db_at(&head).unwrap();
        let events: Vec<Event> = chunk
            .iter()
            .map(|&k| {
                Event::upsert(
                    &key(k),
                    format!("{tag}:{}", rng.next()).as_bytes(),
                    &format!("{tag}-{k}"),
                )
            })
            .collect();
        let db = repo.db_append(&db, &events).unwrap();
        let edits: Vec<Edit> = chunk
            .iter()
            .step_by(10)
            .map(|&k| {
                let f = (k % (files / 2)) * 2 + side;
                Edit::Put {
                    path: format!("d{:02}/f{f:07}.csv", f % 100),
                    id: repo
                        .put_content(format!("{tag}:{k}").as_bytes())
                        .unwrap()
                        .id,
                }
            })
            .collect();
        let tree = repo.edit_tree(Some(tree_of(repo, head)), &edits).unwrap();
        repo.commit(name, tree, "files").unwrap();
        repo.db_commit(name, &db, "db").unwrap();
    }
}

fn case(base: &Repo<Counting>, n: u64, c: usize, overlap: bool, needed: bool) {
    let mut repo = Repo::new(base.store().copy());
    repo.needed_index = needed;
    let head = repo.resolve("main").unwrap();
    repo.branch("side", head).unwrap();
    let mut rng = Lcg(7 + c as u64);
    // Distinct keys for each side; with `overlap`, 10% of side's are main's.
    let mut used = BTreeSet::new();
    let mut pick = |rng: &mut Lcg| loop {
        let k = rng.next() % n;
        if used.insert(k) {
            return k;
        }
    };
    let mine: Vec<u64> = (0..c).map(|_| pick(&mut rng)).collect();
    let mut theirs: Vec<u64> = (0..c).map(|_| pick(&mut rng)).collect();
    let planted = if overlap { c / 10 } else { 0 };
    theirs[..planted].copy_from_slice(&mine[..planted]);
    // Files touched on both sides are conflicts too (tests/merge.rs);
    // here main edits even-numbered files and side odd ones.
    let files = n / 10;
    changes(&mut repo, "main", &mine, files, 0);
    changes(&mut repo, "side", &theirs, files, 1);
    // The control: the same events appended straight onto main's head,
    // on a copy, as an append without a merge would cost.
    let control = {
        let mut copy = Repo::new(repo.store().copy());
        copy.needed_index = needed;
        let (m, s) = (copy.resolve("main").unwrap(), copy.resolve("side").unwrap());
        let (md, sd) = (copy.db_at(&m).unwrap(), copy.db_at(&s).unwrap());
        let events: Vec<Event> = copy
            .db_events(&sd)
            .unwrap()
            .into_iter()
            .filter(|(q, _)| *q > n)
            .map(|(_, e)| e)
            .collect();
        // Warm the handle as the merge's is warm (it made the changes).
        copy.db_append(&md, &[Event::upsert("warm", b"up", "warm-up")])
            .unwrap();
        copy.store().take();
        let t = Instant::now();
        if !overlap {
            copy.db_append(&md, &events).unwrap();
        }
        (t.elapsed().as_secs_f64() * 1e3, copy.store().take().reads)
    };
    repo.store().take();
    let t = Instant::now();
    let out = repo.merge("main", "side", "merge").unwrap();
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let io = repo.store().take();
    let (result, keys, paths) = match &out {
        Merged::Commit(_) => ("merged", 0, 0),
        Merged::Conflicts(cs) => (
            "conflicts",
            cs.iter().filter(|c| matches!(c, Conflict::Key(_))).count(),
            cs.iter().filter(|c| matches!(c, Conflict::Path(_))).count(),
        ),
        other => panic!("unexpected {other:?}"),
    };
    let head = repo.resolve("main").unwrap();
    let db = repo.db_at(&head).unwrap();
    let t = Instant::now();
    let rebuilt = repo.db_rebuild(&db.log).unwrap();
    let rebuild_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(rebuilt, db, "views equal a rebuild of the merged log");
    match out {
        Merged::Commit(_) => assert_eq!(repo.db_head(&db.log).unwrap(), n + 2 * c as u64),
        _ => assert_eq!(keys, planted, "every planted key found, no other"),
    }
    println!(
        "{{\"rows\":{n},\"needed_index\":{needed},\"files\":{files},\"changes_each_side\":{c},\"planted\":{planted},\"result\":\"{result}\",\"key_conflicts\":{keys},\"path_conflicts\":{paths},\"merge_ms\":{ms:.1},\"reads\":{},\"read_bytes\":{},\"append_only_ms\":{:.1},\"append_only_reads\":{},\"rebuild_ms\":{rebuild_ms:.0}}}",
        io.reads, io.bytes, control.0, control.1
    );
}

fn main() {
    let sizes: Vec<u64> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let sizes = if sizes.is_empty() {
        vec![100_000]
    } else {
        sizes
    };
    for n in sizes {
        let base = build(n);
        for needed in [false, true] {
            for c in [100, 1_000] {
                for overlap in [false, true] {
                    case(&base, n, c, overlap, needed);
                }
            }
        }
    }
}
