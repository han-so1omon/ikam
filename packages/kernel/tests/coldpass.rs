//! Laws of the incremental cold pass (E268): every file reads the same,
//! the store stays fsck clean, nothing from before the last pass is
//! re-encoded, a pass with nothing new does nothing, and a full repack
//! still works afterwards.

use std::collections::BTreeMap;

use ikam_kernel::{
    COLD_PASS_REF, ColdPassOpts, Edit, Event, FsStore, Id, MemStore, Object, Repo, Store,
};

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0 >> 33
    }
}

fn note(rng: &mut Lcg, i: usize, step: usize) -> Vec<u8> {
    (0..30)
        .map(|j| {
            format!(
                "Line {j} of note {i}: value {} at {step}.",
                rng.next() % 997
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn tiny(i: usize, step: usize) -> Vec<u8> {
    format!("{{\"id\":{i},\"step\":{step},\"ok\":true}}").into_bytes()
}

/// Files on `main`: 40 notes and 40 tiny records. Returns every file's bytes.
fn session<S: Store>(
    repo: &mut Repo<S>,
    rng: &mut Lcg,
    step: usize,
    n: usize,
) -> BTreeMap<String, Vec<u8>> {
    let root = repo
        .resolve("main")
        .ok()
        .map(|c| match repo.get(&c).unwrap() {
            Object::Commit(c) => c.tree,
            _ => unreachable!(),
        });
    let mut edits = Vec::new();
    let mut files = BTreeMap::new();
    for i in 0..n {
        let (path, bytes) = if i % 2 == 0 {
            (format!("notes/n{i:02}.md"), note(rng, i, step))
        } else {
            (format!("recs/r{i:02}.json"), tiny(i, step))
        };
        let id = repo.put_content(&bytes).unwrap().id;
        edits.push(Edit::Put {
            path: path.clone(),
            id,
        });
        files.insert(path, bytes);
    }
    let tree = repo.edit_tree(root, &edits).unwrap();
    repo.commit("main", tree, "session").unwrap();
    files
}

fn read_all<S: Store>(repo: &Repo<S>, files: &BTreeMap<String, Vec<u8>>) {
    let mut tree = match repo.get(&repo.resolve("main").unwrap()).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    };
    let top = tree;
    for (path, bytes) in files {
        tree = top;
        let mut id = None;
        for part in path.split('/') {
            let Object::Tree(entries) = repo.get(&tree).unwrap() else {
                panic!("not a tree")
            };
            let e = entries.into_iter().find(|e| e.name == part).unwrap();
            tree = e.id;
            id = Some(e.id);
        }
        assert_eq!(&repo.read_content(&id.unwrap()).unwrap(), bytes, "{path}");
    }
}

fn encodings<S: Store>(repo: &Repo<S>) -> BTreeMap<Id, Vec<u8>> {
    repo.store()
        .ids()
        .unwrap()
        .into_iter()
        .map(|id| (id, repo.store().read(&id).unwrap()))
        .collect()
}

#[test]
fn a_pass_keeps_every_file_and_leaves_old_encodings_alone() {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Lcg(3);
    session(&mut repo, &mut rng, 0, 80);
    let first = repo.repack_incremental().unwrap();
    assert!(first.new > 0, "a first pass sees everything as new");
    assert!(first.after <= first.before);
    let old = encodings(&repo);
    let files = session(&mut repo, &mut rng, 1, 80);
    let second = repo.repack_incremental().unwrap();
    assert!(second.new > 0 && second.new < old.len() + second.new);
    for (id, encoded) in &old {
        if let Ok(now) = repo.store().read(id) {
            assert_eq!(
                &now, encoded,
                "an object from before the last pass is untouched"
            );
        }
    }
    read_all(&repo, &files);
    assert!(repo.fsck_deep().unwrap().is_empty());
    assert!(repo.store().get_ref(COLD_PASS_REF).unwrap().is_some());
}

#[test]
fn a_pass_with_nothing_new_does_nothing() {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Lcg(5);
    let files = session(&mut repo, &mut rng, 0, 40);
    repo.repack_incremental().unwrap();
    let before = encodings(&repo);
    let again = repo.repack_incremental().unwrap();
    assert_eq!(again.new, 0);
    assert_eq!(encodings(&repo), before);
    read_all(&repo, &files);
}

#[test]
fn small_new_files_are_grouped_and_read_the_same() {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Lcg(7);
    let opts = ColdPassOpts { group: true };
    session(&mut repo, &mut rng, 0, 80);
    repo.repack_incremental_with(opts).unwrap();
    let files = session(&mut repo, &mut rng, 1, 80);
    let pass = repo.repack_incremental_with(opts).unwrap();
    assert!(pass.after < pass.before, "{pass:?}");
    read_all(&repo, &files);
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn a_full_repack_and_gc_still_work_after_passes() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open_deferred(dir.path()).unwrap());
    let mut rng = Lcg(11);
    session(&mut repo, &mut rng, 0, 40);
    repo.repack_incremental().unwrap();
    let files = session(&mut repo, &mut rng, 1, 40);
    repo.repack_incremental().unwrap();
    repo.gc().unwrap();
    read_all(&repo, &files);
    repo.repack().unwrap();
    read_all(&repo, &files);
    assert!(repo.fsck_deep().unwrap().is_empty());
    // The marker survives gc and repack, and the next pass sees only what
    // the repack wrote as new objects, if anything.
    assert!(repo.store().get_ref(COLD_PASS_REF).unwrap().is_some());
    repo.repack_incremental().unwrap();
    read_all(&repo, &files);
}

fn database(opts: ColdPassOpts) {
    let mut repo = Repo::new(MemStore::default());
    repo.plain_db_events = true;
    repo.needed_index = true;
    let empty = repo.db_empty().unwrap();
    let ev = |i: u64, s: u64| {
        Event::upsert(
            &format!("t/{i:05}"),
            format!("{},{},row {i} step {s},{}", i * 7, s, i % 13).as_bytes(),
            &format!("e{s}-{i}"),
        )
    };
    let n = 3_000;
    let mut db = repo
        .db_append(&empty, &(0..n).map(|i| ev(i, 0)).collect::<Vec<_>>())
        .unwrap();
    repo.db_commit("main", &db, "load").unwrap();
    repo.repack_incremental_with(opts).unwrap();
    // Appends after a pass encode against what the pass wrote; the next
    // pass must leave those contexts and groups alone (the first version
    // regrouped them: hundreds of fsck errors after a second pass).
    for s in 1..=12 {
        db = repo
            .db_append(
                &db,
                &(0..100)
                    .map(|i| ev((i * 7_919 + s * 31) % n, s))
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        repo.db_commit("main", &db, "append").unwrap();
    }
    let scan = repo.db_scan(&db).unwrap();
    repo.repack_incremental_with(opts).unwrap();
    assert!(repo.fsck_deep().unwrap().is_empty());
    assert_eq!(repo.db_scan(&db).unwrap(), scan);
    assert_eq!(repo.db_rebuild(&db.log).unwrap(), db);
    db = repo.db_append(&db, &[ev(1, 99)]).unwrap();
    repo.db_commit("main", &db, "append").unwrap();
    repo.repack_incremental_with(opts).unwrap();
    assert_eq!(repo.db_rebuild(&db.log).unwrap(), db);
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn a_database_stays_consistent_over_passes() {
    database(ColdPassOpts::default());
}

#[test]
fn a_grouped_database_stays_consistent_over_passes() {
    database(ColdPassOpts { group: true });
}

#[test]
fn grouping_is_off_by_default() {
    assert!(!ColdPassOpts::default().group);
}
