//! E412: how much of a `db serve` pack store is earlier versions of its
//! views, which only its commit history still reaches?
//!
//! `cargo run --release --example db_squash STORE_DIR [cold|cold-group]`
//!
//! Every append commits the database, and each commit keeps the one before
//! it as parent, so the store holds every version of every view chunk an
//! append rewrote, though the log at the head alone rebuilds them. On a
//! copy of a stopped server's store: points each `db/` ref at a commit of
//! its current tree with no parent, collects what no ref needs and cleans
//! the packs, then checks every database reads the same. Prints the bytes
//! and objects before and after, and the time each step took. With `cold`
//! (or `cold-group`), then runs the incremental cold pass (E268; grouping
//! small contents with `cold-group`) and cleans the packs again. Before and
//! after each step it times two reads of the first database: every key
//! under `t/latest/`, and the keys of the last 100 under `i/` counted.

use std::time::Instant;

use ikam_kernel::backends::{PackOptions, PackStore};
use ikam_kernel::{ColdPassOpts, Commit, Object, Repo, Store};

/// Milliseconds of a prefix scan and of a count over the last keys of
/// `i/`, on the first `db/` ref's database.
fn reads<S: Store>(repo: &mut Repo<S>, name: &str) -> (f64, f64) {
    let head = repo.store().get_ref(name).unwrap().unwrap();
    let db = repo.db_at(&head).unwrap();
    let t = Instant::now();
    repo.db_prefix(&db, "t/latest/", usize::MAX, false).unwrap();
    let scan = t.elapsed().as_secs_f64() * 1000.0;
    let last = repo.db_prefix(&db, "i/", 100, true).unwrap();
    let from = last.last().map(|(k, _)| k.clone()).unwrap_or_default();
    let t = Instant::now();
    repo.db_count(&db, &from, Some("i0")).unwrap();
    (scan, t.elapsed().as_secs_f64() * 1000.0)
}

fn du(dir: &std::path::Path) -> u64 {
    let mut total = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let m = e.metadata().unwrap();
        total += if m.is_dir() { du(&e.path()) } else { m.len() };
    }
    total
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("STORE_DIR"));
    let opts = PackOptions {
        lazy_checkpoint: true,
        ref_log: false,
        filters: true,
    };
    let before = du(&dir);
    let cold = std::env::args().nth(2);
    let mut repo = Repo::new(PackStore::open_with(&dir, opts).unwrap());
    let objects = repo.store().ids().unwrap().len();
    let refs: Vec<_> = repo
        .store()
        .refs()
        .unwrap()
        .into_iter()
        .filter(|(n, _)| n.starts_with("db/"))
        .collect();
    let (mut counts, mut squash) = (Vec::new(), Vec::new());
    let first = refs.first().expect("no database").0.clone();
    let reads_before = reads(&mut repo, &first);
    let t = Instant::now();
    for (name, head) in &refs {
        let db = repo.db_at(head).unwrap();
        counts.push(repo.db_count(&db, "", None).unwrap());
        let Object::Commit(c) = repo.get(head).unwrap() else {
            panic!("{name} is not a commit")
        };
        let squashed = repo
            .put(&Object::Commit(Commit {
                tree: c.tree,
                parents: Vec::new(),
                message: c.message,
            }))
            .unwrap();
        squash.push((name.clone(), *head, squashed));
    }
    let mut store = repo.into_store();
    for (name, head, squashed) in &squash {
        store.set_ref(name, Some(*head), *squashed).unwrap();
    }
    store.sync().unwrap();
    let squash_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let mut repo = Repo::new(store);
    let deleted = repo.gc().unwrap();
    let mut store = repo.into_store();
    store.sync().unwrap();
    let gc_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    store.compact().unwrap();
    let compact_s = t.elapsed().as_secs_f64();
    let after_objects = store.ids().unwrap().len();
    drop(store);
    let after = du(&dir);
    let mut repo = Repo::new(PackStore::open_with(&dir, opts).unwrap());
    let reads_squashed = reads(&mut repo, &first);
    let mut cold_out = String::new();
    if let Some(mode) = &cold {
        let t = Instant::now();
        let c = repo
            .repack_incremental_with(ColdPassOpts {
                group: mode == "cold-group",
            })
            .unwrap();
        let mut store = repo.into_store();
        store.sync().unwrap();
        let pass_s = t.elapsed().as_secs_f64();
        let t = Instant::now();
        store.compact().unwrap();
        let compact_s = t.elapsed().as_secs_f64();
        drop(store);
        repo = Repo::new(PackStore::open_with(&dir, opts).unwrap());
        let r = reads(&mut repo, &first);
        cold_out = format!(
            ",\"cold\":\"{mode}\",\"cold_new\":{},\"cold_before\":{},\"cold_after\":{},\"cold_s\":{pass_s:.2},\"cold_compact_s\":{compact_s:.2},\"bytes_after_cold\":{},\"reads_after_cold_ms\":[{:.1},{:.1}]",
            c.new,
            c.before,
            c.after,
            du(&dir),
            r.0,
            r.1
        );
    }
    let same = refs.iter().zip(&counts).all(|((name, _), n)| {
        let head = repo.store().get_ref(name).unwrap().unwrap();
        let db = repo.db_at(&head).unwrap();
        repo.db_count(&db, "", None).unwrap() == *n
    });
    println!(
        "{{\"refs\":{},\"keys\":{counts:?},\"bytes_before\":{before},\"bytes_after\":{after},\"objects_before\":{objects},\"objects_after\":{after_objects},\"deleted\":{deleted},\"squash_s\":{squash_s:.2},\"gc_s\":{gc_s:.2},\"compact_s\":{compact_s:.2},\"reads_same\":{same},\"reads_before_ms\":[{:.1},{:.1}],\"reads_squashed_ms\":[{:.1},{:.1}]{cold_out}}}",
        refs.len(),
        reads_before.0,
        reads_before.1,
        reads_squashed.0,
        reads_squashed.1
    );
}
