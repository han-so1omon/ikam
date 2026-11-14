//! E266: drafts as branches.
//! `cargo run --release --example drafts`
//!
//! A draft is a ref under `drafts/` (`Repo::draft`), edited with ordinary
//! commits, then promoted into its target (`Repo::promote_draft`: a
//! fast-forward or a merge, then the draft's ref is deleted) or discarded
//! (`Repo::discard_draft`, and the next gc removes what only it reached).
//! On an FsStore with deferred sync (E227), over 200 generated files:
//!
//! (plus one shared file no draft owns)
//!
//! - `direct`: 1,000 edits committed straight to `main`, the baseline.
//! - `promote_ff`: the same 1,000 edits as 100 drafts of 10 edits, each
//!   promoted before the next opens (every promote is a fast-forward).
//! - `promote_merge`: 100 drafts opened at the same head, 10 edits each on
//!   files of their own, then promoted one after another (the first
//!   fast-forwards, the rest merge). Every tenth draft also edits the
//!   shared file, so the 9 after the first conflict and are refused.
//! - `discard`: 100 drafts of 10 edits, all discarded, then gc. Reports
//!   bytes on disk before the drafts, with them, after discarding and
//!   after gc, whether gc left exactly the objects that existed before the
//!   drafts, what it kept beyond them, and whether `main` reads the same.
//!
//! Prints one JSON line per case. Fixtures are synthetic.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use ikam_kernel::merge::Merged;
use ikam_kernel::{Edit, FsStore, Id, Object, Repo, Store};

const FILES: usize = 200;
const DRAFTS: usize = 100;
const EDITS: usize = 10;
/// A file no draft owns, edited by every tenth draft in `promote_merge`.
const SHARED: usize = FILES;

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0 >> 33
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn dir_bytes(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_bytes(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

fn note(rng: &mut Lcg, i: usize, step: usize) -> Vec<u8> {
    (0..40)
        .map(|j| {
            format!(
                "Paragraph {j} of note {i}: reading {} at step {step}.",
                rng.next() % 9973
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn path(i: usize) -> String {
    format!("notes/n{i:03}.md")
}

fn setup(dir: &Path) -> (Repo<FsStore>, Lcg) {
    let mut repo = Repo::new(FsStore::open_deferred(dir).unwrap());
    let mut rng = Lcg(7);
    let mut edits = Vec::new();
    for i in 0..FILES {
        let id = repo.put_content(&note(&mut rng, i, 0)).unwrap().id;
        edits.push(Edit::Put { path: path(i), id });
    }
    let id = repo.put_content(&note(&mut rng, SHARED, 0)).unwrap().id;
    edits.push(Edit::Put {
        path: path(SHARED),
        id,
    });
    let tree = repo.edit_tree(None, &edits).unwrap();
    repo.commit("main", tree, "setup").unwrap();
    (repo, rng)
}

/// One edit of file `f` committed on `name`. Returns its time.
fn edit(repo: &mut Repo<FsStore>, rng: &mut Lcg, name: &str, f: usize, step: usize) -> f64 {
    let t = Instant::now();
    let head = repo.resolve(name).unwrap();
    let Object::Commit(c) = repo.get(&head).unwrap() else {
        unreachable!()
    };
    let id: Id = repo.put_content(&note(rng, f, step)).unwrap().id;
    let tree = repo
        .edit_tree(Some(c.tree), &[Edit::Put { path: path(f), id }])
        .unwrap();
    repo.commit(name, tree, "edit").unwrap();
    ms(t)
}

/// Every file on `main` and its bytes.
fn files(repo: &Repo<FsStore>) -> Vec<Vec<u8>> {
    let Object::Commit(c) = repo.get(&repo.resolve("main").unwrap()).unwrap() else {
        unreachable!()
    };
    let mut out = Vec::new();
    let mut stack = vec![c.tree];
    while let Some(t) = stack.pop() {
        let Object::Tree(entries) = repo.get(&t).unwrap() else {
            unreachable!()
        };
        for e in entries {
            match repo.get(&e.id) {
                Ok(Object::Tree(_)) => stack.push(e.id),
                _ => out.push(repo.read_content(&e.id).unwrap()),
            }
        }
    }
    out.sort();
    out
}

fn direct() {
    let dir = tempfile::tempdir().unwrap();
    let (mut repo, mut rng) = setup(dir.path());
    let mut times = Vec::new();
    let t = Instant::now();
    for step in 1..=DRAFTS * EDITS {
        let f = rng.next() as usize % FILES;
        times.push(edit(&mut repo, &mut rng, "main", f, step));
    }
    let session = ms(t);
    println!(
        "{{\"case\":\"direct\",\"edits\":{},\"session_ms\":{session:.0},\"edit_p50\":{:.2},\"edit_p99\":{:.2}}}",
        DRAFTS * EDITS,
        pct(&mut times, 0.5),
        pct(&mut times, 0.99)
    );
}

fn promote_ff() {
    let dir = tempfile::tempdir().unwrap();
    let (mut repo, mut rng) = setup(dir.path());
    let (mut open, mut edits, mut promote) = (vec![], vec![], vec![]);
    let mut ff = 0;
    let t = Instant::now();
    for d in 0..DRAFTS {
        let t1 = Instant::now();
        let name = repo.draft("main", &format!("d{d:03}")).unwrap();
        open.push(ms(t1));
        for e in 0..EDITS {
            let f = rng.next() as usize % FILES;
            edits.push(edit(&mut repo, &mut rng, &name, f, d * EDITS + e + 1));
        }
        let t2 = Instant::now();
        let merged = repo.promote_draft("main", &name, "promote").unwrap();
        promote.push(ms(t2));
        ff += matches!(merged, Merged::FastForward(_)) as usize;
    }
    let session = ms(t);
    assert!(repo.drafts().unwrap().is_empty());
    assert!(repo.fsck().unwrap().is_empty());
    println!(
        "{{\"case\":\"promote_ff\",\"drafts\":{DRAFTS},\"edits\":{},\"session_ms\":{session:.0},\"open_p50\":{:.2},\"edit_p50\":{:.2},\"edit_p99\":{:.2},\"promote_p50\":{:.2},\"promote_p99\":{:.2},\"fast_forwards\":{ff}}}",
        DRAFTS * EDITS,
        pct(&mut open, 0.5),
        pct(&mut edits, 0.5),
        pct(&mut edits, 0.99),
        pct(&mut promote, 0.5),
        pct(&mut promote, 0.99)
    );
}

fn promote_merge() {
    let dir = tempfile::tempdir().unwrap();
    let (mut repo, mut rng) = setup(dir.path());
    let mut names = Vec::new();
    for d in 0..DRAFTS {
        let name = repo.draft("main", &format!("d{d:03}")).unwrap();
        // Files of its own: draft d edits files 2d and 2d+1 only.
        for e in 0..EDITS {
            let f = 2 * d + e % 2;
            edit(&mut repo, &mut rng, &name, f, d * EDITS + e + 1);
        }
        if d.is_multiple_of(10) {
            edit(&mut repo, &mut rng, &name, SHARED, 9_000 + d);
        }
        names.push(name);
    }
    let (mut promote, mut refused) = (vec![], vec![]);
    let (mut ff, mut merges, mut conflicts, mut unchanged) = (0, 0, 0, 0);
    let t = Instant::now();
    for name in &names {
        let main = repo.resolve("main").unwrap();
        let t1 = Instant::now();
        let merged = repo.promote_draft("main", name, "promote").unwrap();
        match merged {
            Merged::FastForward(_) => ff += 1,
            Merged::Commit(_) => merges += 1,
            Merged::Conflicts(_) => {
                conflicts += 1;
                refused.push(ms(t1));
                unchanged += (repo.resolve("main").unwrap() == main) as usize;
                continue;
            }
            Merged::UpToDate => {}
        }
        promote.push(ms(t1));
    }
    let session = ms(t);
    assert!(repo.fsck().unwrap().is_empty());
    println!(
        "{{\"case\":\"promote_merge\",\"drafts\":{DRAFTS},\"promote_session_ms\":{session:.0},\"promote_p50\":{:.2},\"promote_p99\":{:.2},\"refused_p50\":{:.2},\"fast_forwards\":{ff},\"merges\":{merges},\"conflicts\":{conflicts},\"main_unchanged_on_conflict\":{unchanged},\"drafts_left\":{}}}",
        pct(&mut promote, 0.5),
        pct(&mut promote, 0.99),
        pct(&mut refused, 0.5),
        repo.drafts().unwrap().len()
    );
}

fn discard() {
    let dir = tempfile::tempdir().unwrap();
    let (mut repo, mut rng) = setup(dir.path());
    let before: BTreeSet<Id> = repo.store().ids().unwrap().into_iter().collect();
    let main_files = files(&repo);
    let disk_before = dir_bytes(dir.path());
    let mut names = Vec::new();
    for d in 0..DRAFTS {
        let name = repo.draft("main", &format!("d{d:03}")).unwrap();
        for e in 0..EDITS {
            let f = rng.next() as usize % FILES;
            edit(&mut repo, &mut rng, &name, f, d * EDITS + e + 1);
        }
        names.push(name);
    }
    let disk_drafts = dir_bytes(dir.path());
    let objects_drafts = repo.store().ids().unwrap().len();
    let mut times = Vec::new();
    for name in &names {
        let t = Instant::now();
        repo.discard_draft(name).unwrap();
        times.push(ms(t));
    }
    let disk_discarded = dir_bytes(dir.path());
    let t = Instant::now();
    let freed = repo.gc().unwrap();
    let gc_ms = ms(t);
    let disk_gc = dir_bytes(dir.path());
    let after: BTreeSet<Id> = repo.store().ids().unwrap().into_iter().collect();
    // What gc kept beyond the objects from before the drafts: none may be
    // a draft's commit or tree. (Induction can re-store a live file as a
    // derivation from a template learned while the drafts were written.)
    let extra: Vec<&Id> = after.difference(&before).collect();
    let extra_commits_trees = extra
        .iter()
        .filter(|i| matches!(repo.get(i), Ok(Object::Commit(_) | Object::Tree(_))))
        .count();
    let reads_same = files(&repo) == main_files;
    let t = Instant::now();
    let clean = repo.fsck().unwrap().is_empty();
    let fsck_ms = ms(t);
    println!(
        "{{\"case\":\"discard\",\"drafts\":{DRAFTS},\"edits\":{},\"objects_before\":{},\"objects_with_drafts\":{objects_drafts},\"objects_after_gc\":{},\"same_objects_as_before\":{},\"kept_beyond_before\":{},\"kept_commits_or_trees\":{extra_commits_trees},\"main_reads_same\":{reads_same},\"disk_before\":{disk_before},\"disk_with_drafts\":{disk_drafts},\"disk_after_discard\":{disk_discarded},\"disk_after_gc\":{disk_gc},\"discard_p50\":{:.2},\"gc_ms\":{gc_ms:.0},\"gc_freed\":{freed},\"fsck_ms\":{fsck_ms:.0},\"fsck_clean\":{clean}}}",
        DRAFTS * EDITS,
        before.len(),
        after.len(),
        after == before,
        extra.len(),
        pct(&mut times, 0.5)
    );
}

fn main() {
    direct();
    promote_ff();
    promote_merge();
    discard();
}
