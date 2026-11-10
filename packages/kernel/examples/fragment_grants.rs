//! E159: one fragment edit under a goal's sealed grant (E140), checked
//! across every file it reaches.
//!
//! `cargo run --release --example fragment_grants`
//!
//! Store: the cases' md files as renders with shared blocks standing alone
//! (E157), plus a notice paragraph inserted into `n` copies (`fan/`). The
//! goal "revise the notice" is granted, per file, the notice's byte range
//! for writing. For n in 16, 64, 256, one JSON line per case:
//! - `ungranted`: the same edit with no grant (E155), for the check's cost;
//! - `granted`: every reached file granted; `propagate_granted` checks the
//!   ranges from fragment lengths (no diff) and writes;
//! - `diff-check`: the same files checked the E140 way, `Grant::check_edit`
//!   diffing each file's old and new bytes (what a caller holding only
//!   files would run), for comparison;
//! - `half`: half the files granted: refused, nothing written;
//! - `other-range`: each file granted a different block: refused;
//! - `stale`: the grant names the version before an earlier edit: refused.
//!
//! - `fragment-grant` (E162): a `FragmentGrant` naming the notice within
//!   `fan/`, checked from the index; `fragment-outside`: within `fan/1/`,
//!   refused; `fragment-stale`: naming another fragment, refused.
//!
//! Each line: ms (median of 5), whether it was allowed, stored bytes
//! written, files, and the sealed grant's encoded size.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use ikam_kernel::{
    Edit, FileGrant, FragmentGrant, FragmentIndex, Grant, Id, MemStore, Repo, Store, blocks,
};

fn md_files() -> Vec<(String, Vec<u8>)> {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/cases")
        .canonicalize()
        .unwrap();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if !p.to_string_lossy().contains("_image_tools") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "md") {
                let name = p
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((name, std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn stored(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

const NOTICE: &[u8] =
    b"Notice: figures in this document are unaudited and may change after the quarterly review.\n\n";

struct Setup {
    repo: Repo<MemStore>,
    tree: Id,
    ix: FragmentIndex,
    corpus: Vec<(String, Vec<u8>)>,
}

fn setup(files: &[(String, Vec<u8>)], n: usize) -> Setup {
    let mut corpus = files.to_vec();
    for (i, (p, b)) in files.iter().cycle().take(n).enumerate() {
        let rs = blocks(b);
        let at = rs.get(rs.len() / 2).map_or(b.len(), |r| r.start);
        let mut v = b[..at].to_vec();
        v.extend_from_slice(NOTICE);
        v.extend_from_slice(&b[at..]);
        corpus.push((format!("fan/{i}/{p}"), v));
    }
    let mut seen: HashMap<Id, usize> = HashMap::new();
    for (_, b) in &corpus {
        let mine: HashSet<Id> = blocks(b)
            .into_iter()
            .map(|r| Id::of_content(&b[r]))
            .collect();
        for id in mine {
            *seen.entry(id).or_default() += 1;
        }
    }
    let shared: HashSet<Id> = seen.into_iter().filter(|e| e.1 > 1).map(|e| e.0).collect();
    let mut repo = Repo::new(MemStore::default());
    repo.short_records = true;
    let mut edits = Vec::new();
    for (p, b) in &corpus {
        let id = repo.put_fragmented(b, Some(&shared)).unwrap();
        edits.push(Edit::Put {
            path: p.clone(),
            id,
        });
    }
    let tree = repo.edit_tree(None, &edits).unwrap();
    repo.commit("main", tree, "corpus").unwrap();
    let ix = repo.fragment_index(&tree).unwrap();
    Setup {
        repo,
        tree,
        ix,
        corpus,
    }
}

/// The byte ranges of block `target` in `b`.
fn ranges_of(b: &[u8], target: &[u8]) -> Vec<std::ops::Range<u64>> {
    blocks(b)
        .into_iter()
        .filter(|r| &b[r.clone()] == target)
        .map(|r| r.start as u64..r.end as u64)
        .collect()
}

/// A grant for `paths` (all reached files when None): each file's notice
/// range, or with `other`, the block after it.
#[allow(clippy::single_range_in_vec_init)]
fn grant(s: &Setup, paths: &[String], other: bool, stale: bool) -> Grant {
    let mut files = Vec::new();
    for p in paths {
        let b = &s.corpus.iter().find(|f| &f.0 == p).unwrap().1;
        let mut w = ranges_of(b, NOTICE);
        if other {
            let rs = blocks(b);
            let k = rs.iter().position(|r| &b[r.clone()] == NOTICE).unwrap();
            let r = rs.get(k + 1).or(rs.get(k - 1)).unwrap();
            w = vec![r.start as u64..r.end as u64];
        }
        let id = if stale {
            Id::of_content(b"an earlier version")
        } else {
            s.ix.files[p].0
        };
        files.push(FileGrant::new(p, id, w.clone(), w));
    }
    Grant {
        goal: "revise the notice".into(),
        files,
    }
}

fn main() {
    let files = md_files();
    let key = [7u8; 32];
    let new: Vec<u8> = NOTICE
        .trim_ascii_end()
        .iter()
        .chain(b" (revised)\n\n")
        .copied()
        .collect();
    let old = Id::of_content(NOTICE);
    for n in [16usize, 64, 256] {
        let reached: Vec<String> = setup(&files, n).ix.by_fragment[&old].clone();
        for case in [
            "ungranted",
            "granted",
            "diff-check",
            "half",
            "other-range",
            "stale",
            "fragment-grant",
            "fragment-outside",
            "fragment-stale",
        ] {
            let mut runs = Vec::new();
            for _ in 0..5 {
                let mut s = setup(&files, n);
                let paths: Vec<String> = if case == "half" {
                    reached.iter().take(reached.len() / 2).cloned().collect()
                } else {
                    reached.clone()
                };
                let g = grant(&s, &paths, case == "other-range", case == "stale");
                let fg = FragmentGrant {
                    goal: "revise the notice".into(),
                    fragment: if case == "fragment-stale" {
                        Id::of_content(b"another fragment")
                    } else {
                        old
                    },
                    within: if case == "fragment-outside" {
                        "fan/1/"
                    } else {
                        "fan/"
                    }
                    .into(),
                };
                let fragment_case = case.starts_with("fragment");
                let (tag, ftag) = (g.seal(&key), fg.seal(&key));
                let size = if fragment_case {
                    fg.encode().len()
                } else {
                    g.encode().len()
                };
                let before = stored(&s.repo);
                let t = Instant::now();
                let allowed = if fragment_case {
                    assert!(fg.verify(&key, &ftag));
                    match s
                        .repo
                        .propagate_fragment_granted(&mut s.ix, &old, &new, &fg)
                        .unwrap()
                    {
                        Ok(edits) => {
                            let tree = s.repo.edit_tree(Some(s.tree), &edits).unwrap();
                            s.repo.commit("main", tree, "notice").unwrap();
                            true
                        }
                        Err(_) => false,
                    }
                } else if case == "diff-check" {
                    assert!(g.verify(&key, &tag));
                    // The E140 check: diff each file's old and new bytes.
                    let mut ok = true;
                    for p in &reached {
                        let id = s.ix.files[p].0;
                        let b = s.repo.read_content(&id).unwrap();
                        let nb: Vec<u8> = blocks(&b)
                            .into_iter()
                            .flat_map(|r| {
                                if &b[r.clone()] == NOTICE {
                                    new.clone()
                                } else {
                                    b[r].to_vec()
                                }
                            })
                            .collect();
                        ok &= g.check_edit(p, &id, &b, &nb).is_ok();
                    }
                    ok
                } else {
                    assert!(g.verify(&key, &tag));
                    let gr = (case != "ungranted").then_some(&g);
                    match s.repo.propagate_granted(&mut s.ix, &old, &new, gr).unwrap() {
                        Ok(edits) => {
                            let tree = s.repo.edit_tree(Some(s.tree), &edits).unwrap();
                            s.repo.commit("main", tree, "notice").unwrap();
                            true
                        }
                        Err(_) => false,
                    }
                };
                let ms = t.elapsed().as_secs_f64() * 1e3;
                let written = stored(&s.repo) - before;
                if !matches!(
                    case,
                    "granted" | "diff-check" | "ungranted" | "fragment-grant"
                ) {
                    assert!(
                        !allowed && written == 0,
                        "{case}: refused edits write nothing"
                    );
                    assert_eq!(s.ix.by_fragment[&old].len(), reached.len());
                }
                if case == "granted" || case == "fragment-grant" {
                    assert!(allowed);
                    assert!(s.repo.fsck().unwrap().is_empty());
                }
                runs.push((ms, allowed, written, size));
            }
            runs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let (ms, allowed, written, size) = runs[2];
            println!(
                "{{\"n\":{n},\"case\":\"{case}\",\"files\":{},\"ms\":{ms:.3},\"allowed\":{allowed},\"written\":{written},\"grant_bytes\":{size}}}",
                reached.len()
            );
        }
    }
}
