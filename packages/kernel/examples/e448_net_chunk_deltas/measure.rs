//! E446's measurement (`examples/e446_net_fragments.rs`, `measure`), for
//! the text and tree forms only, with E448's options applied to the repo:
//! bytes at the default and heavy weight, reuse, write, read and delta
//! times (each the median of three fastest-of runs), deltas applied, fsck,
//! and optionally repack.

use std::collections::HashSet;
use std::time::Instant;

use ikam_kernel::net::{Layout, Net};
use ikam_kernel::{Arg, Graph, Id, MemStore, Object, Repo, Target};

use crate::{PATH, configure, size};

/// The bench's heavy workload weight (README, E069), as E446 uses it.
pub const HEAVY_READ_WEIGHT: f64 = 48.8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    Text,
    Tree,
}

/// Fastest of repeated runs of `f`, in µs: at least 3 and 200 ms (E446).
fn fastest<T>(f: &mut impl FnMut() -> T) -> f64 {
    let (mut best, mut total, mut runs) = (f64::MAX, 0.0, 0);
    while runs < 3 || total < 0.2 {
        let at = Instant::now();
        std::hint::black_box(f());
        let t = at.elapsed().as_secs_f64();
        best = best.min(t);
        total += t;
        runs += 1;
    }
    best * 1e6
}

/// Median of three `fastest` timings: other builds share the machine.
pub fn time<T>(mut f: impl FnMut() -> T) -> f64 {
    let mut t = [fastest(&mut f), fastest(&mut f), fastest(&mut f)];
    t.sort_by(f64::total_cmp);
    t[1]
}

fn reach(repo: &Repo<MemStore>, form: Form, root: &Id) -> Vec<(Id, usize)> {
    if form == Form::Text {
        return vec![(*root, repo.read_content(root).unwrap().len())];
    }
    let mut out = Vec::new();
    let mut stack = vec![*root];
    while let Some(id) = stack.pop() {
        let obj = repo.get(&id).unwrap();
        out.push((id, obj.encode().len()));
        match obj {
            Object::Graph(Graph::Children(_, cs)) => stack.extend(cs.into_iter().map(|c| c.1)),
            Object::Graph(Graph::Nodes(nodes)) => {
                for n in nodes {
                    if let Target::Content(Arg::Whole(c)) = n.target {
                        out.push((c, repo.read_content(&c).unwrap().len()));
                    }
                }
            }
            _ => panic!("not a graph"),
        }
    }
    out
}

pub struct Row {
    pub label: String,
    pub n: usize,
    pub first: usize,
    pub added: Vec<usize>,
    pub reuse_objects: (usize, usize),
    pub reuse_bytes: (usize, usize),
    pub write_us: f64,
    pub read_first_us: f64,
    pub read_last_us: f64,
    pub delta_us: f64,
    pub delta_chunks: f64,
    pub unpruned_chunks: f64,
    pub applied: (usize, usize),
    pub fsck: usize,
    pub heavy: (usize, f64),
    pub heavy_read_last_us: f64,
    pub repack: Option<(usize, f64)>,
    pub chunks: usize,
}

fn put_version(repo: &mut Repo<MemStore>, form: Form, tree: &mut Option<Id>, net: &Net) -> Id {
    let (t, id) = match form {
        Form::Text => repo.put_net_file(*tree, PATH, net).unwrap(),
        Form::Tree => repo
            .put_net_fragment_file(*tree, PATH, net, Layout::Tree)
            .unwrap(),
    };
    *tree = Some(t);
    id
}

fn read(repo: &Repo<MemStore>, form: Form, rev: &Id) -> Net {
    match form {
        Form::Text => repo.net_file(rev, PATH).unwrap(),
        Form::Tree => repo.net_fragment_file(rev, PATH).unwrap(),
    }
}

/// The chain stored at `weight`: per-version growth, commits, roots.
fn store_chain(
    form: Form,
    opt: &str,
    weight: f64,
    nets: &[Net],
) -> (Repo<MemStore>, Vec<usize>, Vec<Id>, Vec<Id>, f64) {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = weight;
    configure(&mut repo, opt);
    let (mut tree, mut sizes, mut commits, mut roots) = (None, Vec::new(), Vec::new(), Vec::new());
    let mut write_s = 0.0;
    for (k, net) in nets.iter().enumerate() {
        let before = size(&repo);
        let at = Instant::now();
        roots.push(put_version(&mut repo, form, &mut tree, net));
        write_s += at.elapsed().as_secs_f64();
        sizes.push(size(&repo) - before);
        commits.push(
            repo.commit("main", tree.unwrap(), &format!("v{}", k + 1))
                .unwrap(),
        );
    }
    (repo, sizes, commits, roots, write_s)
}

pub fn measure(form: Form, opt: &str, nets: &[Net], repack: bool) -> Row {
    let n = nets[0].transitions.len();
    let last = nets.len() - 1;
    let (mut repo, sizes, commits, roots, write_s) =
        store_chain(form, opt, ikam_kernel::DEFAULT_READ_WEIGHT, nets);
    let mut seen: HashSet<Id> = HashSet::new();
    let (mut reuse_objects, mut reuse_bytes) = ((0, 0), (0, 0));
    for (k, root) in roots.iter().enumerate() {
        let objs = reach(&repo, form, root);
        if k > 0 {
            for (id, len) in &objs {
                reuse_objects.1 += 1;
                reuse_bytes.1 += len;
                if seen.contains(id) {
                    reuse_objects.0 += 1;
                    reuse_bytes.0 += len;
                }
            }
        }
        seen.extend(objs.iter().map(|(id, _)| *id));
    }
    for (k, c) in commits.iter().enumerate() {
        assert!(read(&repo, form, c) == nets[k], "{form:?} {opt} v{}", k + 1);
    }
    let read_first_us = time(|| read(&repo, form, &commits[0]));
    let read_last_us = time(|| read(&repo, form, &commits[last]));
    let (mut delta_us, mut chunks, mut unpruned, mut applied) = (0.0, 0, 0, (0, 0));
    for k in 1..nets.len() {
        match form {
            Form::Text => {
                delta_us += time(|| {
                    let x = repo.read_content(&roots[k - 1]).unwrap();
                    let y = repo.read_content(&roots[k]).unwrap();
                    similar::TextDiff::from_lines(
                        std::str::from_utf8(&x).unwrap(),
                        std::str::from_utf8(&y).unwrap(),
                    )
                    .iter_all_changes()
                    .filter(|c| c.tag() != similar::ChangeTag::Equal)
                    .count()
                })
            }
            Form::Tree => {
                delta_us += time(|| repo.net_delta(&roots[k - 1], &roots[k]).unwrap());
                let d = repo.net_delta(&roots[k - 1], &roots[k]).unwrap();
                let full = repo
                    .net_delta_with(&roots[k - 1], &roots[k], false)
                    .unwrap();
                assert_eq!(
                    (&d.places, &d.transitions, &d.links),
                    (&full.places, &full.transitions, &full.links)
                );
                chunks += d.chunks_read;
                unpruned += full.chunks_read;
                applied.1 += 1;
                if d.apply(&nets[k - 1]).unwrap() == nets[k] {
                    applied.0 += 1;
                }
            }
        }
    }
    if form == Form::Tree {
        let d = repo.net_delta(&roots[0], &roots[last]).unwrap();
        applied.1 += 1;
        if d.apply(&nets[0]).unwrap() == nets[last] {
            applied.0 += 1;
        }
    }
    let fsck = repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
    let nchunks = match form {
        Form::Text => 0,
        Form::Tree => crate::anatomy::chunks(&repo, &roots[last]).len(),
    };
    let repack = repack.then(|| {
        let at = Instant::now();
        repo.repack().unwrap();
        let s = at.elapsed().as_secs_f64();
        for k in [0, last] {
            assert!(read(&repo, form, &commits[k]) == nets[k]);
        }
        assert_eq!(repo.fsck().unwrap().len(), 0);
        (size(&repo), s)
    });
    let (heavy_repo, heavy_sizes, heavy_commits, _, _) =
        store_chain(form, opt, HEAVY_READ_WEIGHT, nets);
    assert!(read(&heavy_repo, form, &heavy_commits[last]) == nets[last]);
    let heavy_read_last_us = time(|| read(&heavy_repo, form, &heavy_commits[last]));
    let e = last as f64;
    Row {
        label: opt.to_string(),
        n,
        first: sizes[0],
        added: sizes[1..].to_vec(),
        reuse_objects,
        reuse_bytes,
        write_us: write_s * 1e6 / nets.len() as f64,
        read_first_us,
        read_last_us,
        delta_us: delta_us / e,
        delta_chunks: chunks as f64 / e,
        unpruned_chunks: unpruned as f64 / e,
        applied,
        fsck,
        heavy: (
            heavy_sizes[0],
            heavy_sizes[1..].iter().sum::<usize>() as f64 / e,
        ),
        heavy_read_last_us,
        repack,
        chunks: nchunks,
    }
}

/// Read v25 for each row, rows interleaved over 7 rounds (each a fastest-of
/// timing) so that load from other builds falls on every row alike; the
/// median per row, at the default weight and on the heavy-weight store.
pub fn reads(n: usize, rows: &[String]) {
    let nets = crate::chain::chain(n);
    let last = nets.len() - 1;
    let mut stores = Vec::new();
    for r in rows {
        let (form, opt) = match r.split_once('+') {
            Some((f, o)) => (f, o),
            None => (r.as_str(), "base"),
        };
        let form = if form == "text" {
            Form::Text
        } else {
            Form::Tree
        };
        for w in [ikam_kernel::DEFAULT_READ_WEIGHT, HEAVY_READ_WEIGHT] {
            let (repo, _, commits, _, write_s) = store_chain(form, opt, w, &nets);
            stores.push((r.clone(), w, form, repo, commits[last], write_s));
        }
    }
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); stores.len()];
    for _ in 0..7 {
        for (i, (_, _, form, repo, c, _)) in stores.iter().enumerate() {
            times[i].push(fastest(&mut || read(repo, *form, c)));
        }
    }
    println!("\n## read v25 at {n} transitions (µs, median of 7 interleaved rounds)\n");
    println!("| row | weight | read v25 | write a version (one pass) |");
    println!("| --- | --- | --- | --- |");
    for (i, (r, w, _, _, _, write_s)) in stores.iter().enumerate() {
        let mut t = times[i].clone();
        t.sort_by(f64::total_cmp);
        println!(
            "| {r} | {w} | {:.0} | {:.0} |",
            t[3],
            write_s * 1e6 / nets.len() as f64
        );
    }
}
