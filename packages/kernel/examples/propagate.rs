//! E155: one edit to a shared fragment, landed in every file that holds it.
//!
//! `cargo run --release --example propagate`
//!
//! Corpus: every md file of `tests/fixtures/cases`. Workloads:
//! - `natural/<k>`: the shared paragraph blocks with the largest fan-out
//!   (boilerplate the case generator wrote into many files), each revised;
//! - `fanout/<n>`: a notice paragraph inserted into the middle of `n` md
//!   files (copies under `fan/`), then revised.
//!
//! Approaches, each on a fresh store holding the whole corpus:
//! - `scan-rewrite`: today's path. Find the files that hold the old bytes
//!   by reading every file, splice each, `put_content` it, edit the tree.
//! - `index-rewrite`: the same, with the files found through a fragment
//!   index kept beside the store instead of a scan.
//! - `render`: files stored as `render` over their blocks
//!   (`put_fragmented`); `propagate` writes one fragment and one fact per
//!   file and hashes each file once; the tree is edited the same way.
//!
//! Per approach and workload: median ms of 5 runs (setup excluded), bytes
//! written, files touched, bytes the edit read or produced, and a check
//! that every file reads back as expected (others unchanged) and fsck is
//! clean. One `setup` line per storage form: stored bytes, ingest ms, and a
//! read pass over every file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use ikam_kernel::{Edit, Id, MemStore, Repo, Store, blocks};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn md_files() -> Vec<(String, Vec<u8>)> {
    let root = repo_root().join("tests/fixtures/cases");
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

/// Stored bytes by encoding tag (content "B"/"Z"/..., tree "T", commit "C",
/// derivation "D").
fn kinds(repo: &Repo<MemStore>) -> HashMap<char, usize> {
    let s = repo.store();
    let mut out = HashMap::new();
    for id in s.ids().unwrap() {
        let e = s.read(&id).unwrap();
        *out.entry(*e.first().unwrap_or(&b'?') as char).or_default() += e.len();
    }
    out
}

fn delta(a: &HashMap<char, usize>, b: &HashMap<char, usize>) -> String {
    let mut ks: Vec<&char> = b.keys().collect();
    ks.sort();
    let parts: Vec<String> = ks
        .into_iter()
        .filter(|k| b[k] != a.get(k).copied().unwrap_or(0))
        .map(|k| format!("\"{k}\":{}", b[k] - a.get(k).copied().unwrap_or(0)))
        .collect();
    format!("{{{}}}", parts.join(","))
}

#[derive(Clone, Copy, PartialEq)]
enum How {
    Scan,
    Index,
    Render,
    /// `render` with short derivation records (E104's `short_records`).
    RenderShort,
    /// `render-short` with only shared blocks standing alone (E157).
    RenderRuns,
}

impl How {
    fn render(self) -> bool {
        matches!(self, How::Render | How::RenderShort | How::RenderRuns)
    }

    fn name(self) -> &'static str {
        match self {
            How::Scan => "scan-rewrite",
            How::Index => "index-rewrite",
            How::Render => "render",
            How::RenderShort => "render-short",
            How::RenderRuns => "render-runs",
        }
    }
}

/// Replace whole blocks equal to `old` by `new`.
fn splice(b: &[u8], old: &[u8], new: &[u8]) -> Vec<u8> {
    blocks(b)
        .into_iter()
        .flat_map(|r| {
            if &b[r.clone()] == old {
                new.to_vec()
            } else {
                b[r].to_vec()
            }
        })
        .collect()
}

fn holds(b: &[u8], old: &[u8]) -> bool {
    blocks(b).into_iter().any(|r| &b[r] == old)
}

struct Setup {
    repo: Repo<MemStore>,
    tree: Id,
    ids: HashMap<String, Id>,
    /// Block id -> paths, for `index-rewrite` (kept beside the store).
    side: HashMap<Id, Vec<String>>,
    /// The fragment index, held as a server would hold it (E152).
    ix: Option<ikam_kernel::FragmentIndex>,
}

/// Ids of the blocks that two or more files hold.
fn shared_blocks(files: &[(String, Vec<u8>)]) -> HashSet<Id> {
    let mut seen: HashMap<Id, usize> = HashMap::new();
    for (_, b) in files {
        let mine: HashSet<Id> = blocks(b)
            .into_iter()
            .map(|r| Id::of_content(&b[r]))
            .collect();
        for id in mine {
            *seen.entry(id).or_default() += 1;
        }
    }
    seen.into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(id, _)| id)
        .collect()
}

fn setup(files: &[(String, Vec<u8>)], how: How) -> (Setup, f64) {
    let t = Instant::now();
    let mut repo = Repo::new(MemStore::default());
    repo.short_records = matches!(how, How::RenderShort | How::RenderRuns);
    let shared = shared_blocks(files);
    let mut edits = Vec::new();
    let mut ids = HashMap::new();
    let mut side: HashMap<Id, Vec<String>> = HashMap::new();
    for (path, b) in files {
        let id = if how.render() {
            repo.put_fragmented(b, (how == How::RenderRuns).then_some(&shared))
                .unwrap()
        } else {
            repo.put_content(b).unwrap().id
        };
        if how == How::Index {
            for r in blocks(b) {
                side.entry(Id::of_content(&b[r]))
                    .or_default()
                    .push(path.clone());
            }
        }
        ids.insert(path.clone(), id);
        edits.push(Edit::Put {
            path: path.clone(),
            id,
        });
    }
    let tree = repo.edit_tree(None, &edits).unwrap();
    repo.commit("main", tree, "corpus").unwrap();
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let ix = how.render().then(|| repo.fragment_index(&tree).unwrap());
    (
        Setup {
            repo,
            tree,
            ids,
            side,
            ix,
        },
        ms,
    )
}

struct Outcome {
    ms: f64,
    by_kind: String,
    written: usize,
    touched: usize,
    work: usize,
}

fn run(files: &[(String, Vec<u8>)], how: How, old: &[u8], new: &[u8]) -> Outcome {
    let (mut s, _) = setup(files, how);
    let before = stored(&s.repo);
    let before_kinds = kinds(&s.repo);
    let t = Instant::now();
    let mut work = 0;
    let edits: Vec<Edit> = match how {
        How::Scan | How::Index => {
            let paths: Vec<String> = if how == How::Scan {
                files
                    .iter()
                    .filter(|(p, _)| {
                        let b = s.repo.read_content(&s.ids[p]).unwrap();
                        work += b.len();
                        holds(&b, old)
                    })
                    .map(|(p, _)| p.clone())
                    .collect()
            } else {
                let mut seen = HashSet::new();
                s.side
                    .get(&Id::of_content(old))
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| seen.insert(p.clone()))
                    .collect()
            };
            paths
                .into_iter()
                .map(|path| {
                    let b = s.repo.read_content(&s.ids[&path]).unwrap();
                    let nb = splice(&b, old, new);
                    work += b.len() + nb.len();
                    let id = s.repo.put_content(&nb).unwrap().id;
                    Edit::Put { path, id }
                })
                .collect()
        }
        How::Render | How::RenderShort | How::RenderRuns => {
            let ix = s.ix.as_mut().unwrap();
            let edits = s.repo.propagate(ix, &Id::of_content(old), new).unwrap();
            for e in &edits {
                if let Edit::Put { id, .. } = e {
                    work += s.repo.read_content(id).map(|b| b.len()).unwrap_or(0);
                }
            }
            edits
        }
    };
    let tree = s.repo.edit_tree(Some(s.tree), &edits).unwrap();
    s.repo.commit("main", tree, "propagate").unwrap();
    let ms = t.elapsed().as_secs_f64() * 1e3;
    // Exactness: every file reads its expected bytes.
    let changed: HashMap<String, Id> = edits
        .iter()
        .filter_map(|e| match e {
            Edit::Put { path, id } => Some((path.clone(), *id)),
            _ => None,
        })
        .collect();
    for (path, b) in files {
        let expect = splice(b, old, new);
        let id = changed.get(path).copied().unwrap_or(s.ids[path]);
        assert_eq!(s.repo.read_content(&id).unwrap(), expect, "{path}");
        assert_eq!(changed.contains_key(path), holds(b, old), "{path} touched");
    }
    assert!(s.repo.fsck().unwrap().is_empty());
    Outcome {
        ms,
        by_kind: delta(&before_kinds, &kinds(&s.repo)),
        written: stored(&s.repo) - before,
        touched: changed.len(),
        work,
    }
}

fn revise(b: &[u8]) -> Vec<u8> {
    let body = b.trim_ascii_end();
    let mut v = body.to_vec();
    v.extend_from_slice(b" (revised)");
    v.extend_from_slice(&b[body.len()..]);
    v
}

fn report(workload: &str, files: &[(String, Vec<u8>)], old: &[u8]) {
    let new = revise(old);
    for how in [
        How::Scan,
        How::Index,
        How::Render,
        How::RenderShort,
        How::RenderRuns,
    ] {
        let mut runs: Vec<Outcome> = (0..5).map(|_| run(files, how, old, &new)).collect();
        runs.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        let m = &runs[2];
        println!(
            "{{\"workload\":\"{workload}\",\"how\":\"{}\",\"fragment_bytes\":{},\"ms\":{:.3},\"written\":{},\"touched\":{},\"work\":{},\"written_by_kind\":{}}}",
            how.name(),
            old.len(),
            m.ms,
            m.written,
            m.touched,
            m.work,
            m.by_kind
        );
    }
}

fn setup_line(files: &[(String, Vec<u8>)]) {
    for how in [How::Scan, How::Render, How::RenderShort, How::RenderRuns] {
        let mut best = f64::MAX;
        let mut s = None;
        for _ in 0..3 {
            let (x, ms) = setup(files, how);
            best = best.min(ms);
            s = Some(x);
        }
        let mut s = s.unwrap();
        let before = stored(&s.repo);
        let t = Instant::now();
        let frags = if how.render() {
            s.repo.fragment_index(&s.tree).unwrap().by_fragment.len()
        } else {
            s.side.len()
        };
        let index_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let mut n = 0;
        for _ in 0..5 {
            for id in s.ids.values() {
                n += s.repo.read_content(id).unwrap().len();
            }
        }
        let read_ms = t.elapsed().as_secs_f64() * 1e3 / 5.0;
        s.repo.repack().unwrap();
        let t = Instant::now();
        for _ in 0..5 {
            for id in s.ids.values() {
                s.repo.read_content(id).unwrap();
            }
        }
        let repacked_read_ms = t.elapsed().as_secs_f64() * 1e3 / 5.0;
        let _ = before;
        println!(
            "{{\"setup\":\"{}\",\"files\":{},\"input\":{},\"stored\":{},\"ingest_ms\":{best:.1},\"read_pass_ms\":{read_ms:.2},\"read_bytes\":{},\"repacked\":{},\"repacked_read_pass_ms\":{repacked_read_ms:.2},\"index_fragments\":{frags},\"index_ms\":{index_ms:.2}}}",
            how.name(),
            files.len(),
            files.iter().map(|f| f.1.len()).sum::<usize>(),
            before,
            n / 5,
            stored(&s.repo)
        );
    }
}

fn main() {
    let files = md_files();
    setup_line(&files);
    // Natural: the shared blocks with the largest fan-out (ties by bytes).
    let mut fan: HashMap<&[u8], HashSet<&str>> = HashMap::new();
    for (p, b) in &files {
        for r in blocks(b) {
            fan.entry(&b[r]).or_default().insert(p);
        }
    }
    let mut top: Vec<(&[u8], usize)> = fan
        .iter()
        .filter(|(b, _)| b.trim_ascii().len() > 30)
        .map(|(b, ps)| (*b, ps.len()))
        .collect();
    top.sort_by(|a, b| (b.1, b.0.len(), b.0).cmp(&(a.1, a.0.len(), a.0)));
    for (k, (old, n)) in top.iter().take(3).enumerate() {
        report(&format!("natural/{k}/fanout-{n}"), &files, old);
    }
    // Synthetic fan-out.
    let notice: &[u8] = b"Notice: figures in this document are unaudited and may change after the quarterly review.\n\n";
    for n in [1usize, 4, 16, 64, 256] {
        let mut corpus = files.clone();
        for (i, (p, b)) in files.iter().cycle().take(n).enumerate() {
            let rs = blocks(b);
            let at = rs.get(rs.len() / 2).map_or(b.len(), |r| r.start);
            let mut v = b[..at].to_vec();
            v.extend_from_slice(notice);
            v.extend_from_slice(&b[at..]);
            corpus.push((format!("fan/{i}/{p}"), v));
        }
        report(&format!("fanout/{n}"), &corpus, notice);
    }
}
