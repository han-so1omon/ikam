//! E167: one md paragraph edit carried to the text's html and docx renders,
//! stored in the kernel, against storing each re-rendered file whole.
//!
//! Inputs come from `scripts/endpoints_prep.py OUT_DIR` (72 edits over the
//! 24 texts of fixtures/multiformat, each html and docx checked there
//! byte-identical to a full pandoc re-render). Run:
//! `cargo run --release --example endpoints -- OUT_DIR`
//!
//! Per edit, two forms, each on a fresh store holding the old md, html and
//! docx:
//! - `files`: every file stored with `put_content` (the matcher plans the
//!   new version against what is stored);
//! - `fragments`: the html stored as a render over its block pieces
//!   (`put_cut`), so the edit adds one new piece and a new render; md and
//!   docx as in `files`.
//!
//! Reports per form: the bytes of the objects the edit adds, per format and
//! in all, and ms; every file read back byte-exact and fsck clean.

use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;
use std::time::Instant;

use ikam_kernel::{Id, MemStore, Repo, Store};

fn added(repo: &Repo<MemStore>, before: &HashSet<Id>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .filter(|id| !before.contains(id))
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn ids(repo: &Repo<MemStore>) -> HashSet<Id> {
    repo.store().ids().unwrap().into_iter().collect()
}

fn cuts(path: &Path) -> Vec<Range<usize>> {
    let s = std::fs::read_to_string(path).unwrap();
    let v: Vec<usize> = s
        .trim()
        .trim_matches(['[', ']'])
        .split(',')
        .map(|x| x.trim().parse().unwrap())
        .collect();
    v.windows(2).map(|w| w[0]..w[1]).collect()
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("OUT_DIR from endpoints_prep.py");
    let mut cases: Vec<_> = Vec::new();
    for t in std::fs::read_dir(&dir).unwrap() {
        for e in std::fs::read_dir(t.unwrap().path()).unwrap() {
            cases.push(e.unwrap().path());
        }
    }
    cases.sort();
    let mut tot = std::collections::BTreeMap::<String, f64>::new();
    for c in &cases {
        let rd = |f: &str| std::fs::read(c.join(f)).unwrap();
        let (m0, m1, h0, h1, d0, d1) = (
            rd("old.md"),
            rd("new.md"),
            rd("old.html"),
            rd("new.html"),
            rd("old.docx"),
            rd("new.docx"),
        );
        let (k0, k1) = (
            cuts(&c.join("old.html.cuts")),
            cuts(&c.join("new.html.cuts")),
        );
        for form in ["files", "fragments"] {
            let mut repo = Repo::new(MemStore::default());
            let put_html = |repo: &mut Repo<MemStore>, b: &[u8], k: &[Range<usize>]| {
                if form == "files" {
                    repo.put_content(b).unwrap().id
                } else {
                    repo.put_cut(b, k, None).unwrap()
                }
            };
            repo.put_content(&m0).unwrap();
            put_html(&mut repo, &h0, &k0);
            repo.put_content(&d0).unwrap();
            let mut row = Vec::new();
            let t = Instant::now();
            let b = ids(&repo);
            let im = repo.put_content(&m1).unwrap().id;
            row.push(("md", added(&repo, &b)));
            let b = ids(&repo);
            let ih = put_html(&mut repo, &h1, &k1);
            row.push(("html", added(&repo, &b)));
            let b = ids(&repo);
            let id = repo.put_content(&d1).unwrap().id;
            row.push(("docx", added(&repo, &b)));
            let ms = t.elapsed().as_secs_f64() * 1e3;
            assert_eq!(repo.read_content(&im).unwrap(), m1);
            assert_eq!(repo.read_content(&ih).unwrap(), h1);
            assert_eq!(repo.read_content(&id).unwrap(), d1);
            assert!(repo.fsck().unwrap().is_empty());
            let all: usize = row.iter().map(|r| r.1).sum();
            let name = c.strip_prefix(&dir).unwrap().display();
            println!(
                "{{\"case\":\"{name}\",\"form\":\"{form}\",\"md\":{},\"html\":{},\"docx\":{},\"written\":{all},\"ms\":{ms:.2},\"html_bytes\":{},\"docx_bytes\":{}}}",
                row[0].1,
                row[1].1,
                row[2].1,
                h1.len(),
                d1.len()
            );
            for (k, v) in row
                .iter()
                .map(|r| (r.0, r.1 as f64))
                .chain([("written", all as f64), ("ms", ms)])
            {
                *tot.entry(format!("{form}.{k}")).or_default() += v;
            }
        }
    }
    let n = cases.len() as f64;
    let mean: Vec<String> = tot
        .iter()
        .map(|(k, v)| format!("\"{k}\":{:.1}", v / n))
        .collect();
    println!(
        "{{\"summary\":true,\"edits\":{},{}}}",
        cases.len(),
        mean.join(",")
    );
}
