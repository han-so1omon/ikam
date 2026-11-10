//! E181: one paragraph shared by 64 md files, edited once, carried to the
//! 64 md files and their 64 html and 64 docx renders (192 endpoints), and
//! stored in the kernel.
//!
//! Inputs from `scripts/fanout_prep.py OUT_DIR` (the renders spliced and
//! checked there byte-identical to full re-renders). Run:
//! `cargo run --release --example fanout_endpoints -- OUT_DIR`
//!
//! Forms, each on a fresh store holding the old 192 files in one commit:
//! - `files`: every file put with `put_content`; the edit puts the 192 new
//!   files the same way;
//! - `md-runs`: md files as renders where shared blocks stand alone (E157),
//!   the edit one `propagate`; html and docx with `put_content`.
//!
//! Per form: stored bytes at rest, bytes the edit adds per format and ms
//! (median of 3); every file read back exact and fsck clean.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use ikam_kernel::{Edit, Id, MemStore, Repo, Store, blocks};

fn bytes_of(repo: &Repo<MemStore>, ids: impl Iterator<Item = Id>) -> usize {
    ids.map(|id| repo.store().read(&id).unwrap().len()).sum()
}

fn all(repo: &Repo<MemStore>) -> HashSet<Id> {
    repo.store().ids().unwrap().into_iter().collect()
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("OUT_DIR from fanout_prep.py");
    let load = |side: &str| -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = std::fs::read_dir(Path::new(&dir).join(side))
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read(&p).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    };
    let (old, new) = (load("old"), load("new"));
    let ext = |p: &str| p.rsplit('.').next().unwrap().to_string();
    for form in ["files", "md-runs"] {
        let mut runs = Vec::new();
        for _ in 0..3 {
            let mut repo = Repo::new(MemStore::default());
            let runs_md = form == "md-runs";
            repo.short_records = runs_md;
            let mut seen: HashMap<Id, usize> = HashMap::new();
            for (p, b) in &old {
                if p.ends_with(".md") {
                    let mine: HashSet<Id> = blocks(b)
                        .into_iter()
                        .map(|r| Id::of_content(&b[r]))
                        .collect();
                    for id in mine {
                        *seen.entry(id).or_default() += 1;
                    }
                }
            }
            let shared: HashSet<Id> = seen.into_iter().filter(|e| e.1 > 1).map(|e| e.0).collect();
            let mut edits = Vec::new();
            for (p, b) in &old {
                let id = if runs_md && p.ends_with(".md") {
                    repo.put_fragmented(b, Some(&shared)).unwrap()
                } else {
                    repo.put_content(b).unwrap().id
                };
                edits.push(Edit::Put {
                    path: p.clone(),
                    id,
                });
            }
            let tree = repo.edit_tree(None, &edits).unwrap();
            repo.commit("main", tree, "old").unwrap();
            let at_rest = bytes_of(&repo, all(&repo).into_iter());
            let mut ix = runs_md.then(|| repo.fragment_index(&tree).unwrap());
            let mut by_fmt: HashMap<String, usize> = HashMap::new();
            let t = Instant::now();
            let mut edits = Vec::new();
            if let Some(ix) = ix.as_mut() {
                let before = all(&repo);
                let o = &old.iter().find(|f| f.0.ends_with(".md")).unwrap().1;
                let n = &new.iter().find(|f| f.0.ends_with(".md")).unwrap().1;
                // The one changed block: the notice.
                let (ob, nb) = (blocks(o), blocks(n));
                let k = ob
                    .iter()
                    .zip(&nb)
                    .position(|(a, b)| o[a.clone()] != n[b.clone()])
                    .unwrap();
                edits = repo
                    .propagate(ix, &Id::of_content(&o[ob[k].clone()]), &n[nb[k].clone()])
                    .unwrap();
                let added = all(&repo).difference(&before).copied().collect::<Vec<_>>();
                by_fmt.insert("md".into(), bytes_of(&repo, added.into_iter()));
            }
            for (p, b) in &new {
                if runs_md && p.ends_with(".md") {
                    continue;
                }
                let before = all(&repo);
                let id = repo.put_content(b).unwrap().id;
                let added = all(&repo).difference(&before).copied().collect::<Vec<_>>();
                *by_fmt.entry(ext(p)).or_default() += bytes_of(&repo, added.into_iter());
                edits.push(Edit::Put {
                    path: p.clone(),
                    id,
                });
            }
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let tree = repo.edit_tree(Some(tree), &edits).unwrap();
            repo.commit("main", tree, "notice revised").unwrap();
            let ids: HashMap<String, Id> = repo
                .fragment_index(&tree)
                .unwrap()
                .files
                .into_iter()
                .map(|(p, (id, _))| (p, id))
                .collect();
            for (p, b) in &new {
                assert_eq!(&repo.read_content(&ids[p]).unwrap(), b, "{p}");
            }
            assert!(repo.fsck().unwrap().is_empty());
            runs.push((ms, at_rest, by_fmt));
        }
        runs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (ms, at_rest, f) = &runs[1];
        let g = |k: &str| f.get(k).copied().unwrap_or(0);
        println!(
            "{{\"form\":\"{form}\",\"files\":{},\"at_rest\":{at_rest},\"md\":{},\"html\":{},\"docx\":{},\"written\":{},\"ms\":{ms:.2}}}",
            old.len(),
            g("md"),
            g("html"),
            g("docx"),
            g("md") + g("html") + g("docx")
        );
    }
}
