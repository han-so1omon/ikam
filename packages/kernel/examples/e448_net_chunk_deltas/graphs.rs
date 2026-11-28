//! General graphs through `put_graph` (the owner's widened scope): a
//! sparse power-law link graph with short text payloads (page titles) and
//! a property graph with 1-4 KB payloads, each through 25 versions of
//! local edits, stored as a prolly tree of chunks whose nodes point at
//! their payloads (fragments), and as one text file for comparison.
//! Synthetic data only.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use ikam_kernel::{Arg, Edge, Id, MemStore, Node, Repo, Target, node_key};

use crate::measure::{HEAVY_READ_WEIGHT, time};
use crate::{configure, size};

const VERSIONS: usize = 25;
const PATH: &str = "graphs/g.txt";

/// A graph version: node label to (payload, targets' labels).
pub type G = BTreeMap<String, (String, Vec<String>)>;

/// A fixed pseudo-random stream (splitmix64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// A Zipf-like index in `0..n` (exponent about 1): low indices are hubs.
    fn zipf(&mut self, n: usize) -> usize {
        ((n as f64).powf(self.unit()) as usize)
            .saturating_sub(1)
            .min(n - 1)
    }
}

const WORDS: [&str; 32] = [
    "river",
    "station",
    "history",
    "garden",
    "north",
    "festival",
    "bridge",
    "council",
    "valley",
    "museum",
    "harbor",
    "school",
    "market",
    "island",
    "theory",
    "orchestra",
    "season",
    "league",
    "castle",
    "railway",
    "forest",
    "library",
    "district",
    "election",
    "mountain",
    "canal",
    "village",
    "album",
    "temple",
    "airport",
    "novel",
    "county",
];

fn title(r: &mut Rng, i: usize) -> String {
    let n = 2 + r.below(4);
    let mut t: Vec<&str> = (0..n).map(|_| WORDS[r.below(WORDS.len())]).collect();
    t[0] = WORDS[i % WORDS.len()];
    format!("{} {}", t.join(" "), i)
}

fn properties(r: &mut Rng, i: usize) -> String {
    let len = 1024 + r.below(3072);
    let mut out = format!("id: {i}\nkind: {}\n", WORDS[i % 7]);
    let mut k = 0;
    while out.len() < len {
        out += &format!(
            "p{k}: {} {} {}\n",
            WORDS[r.below(WORDS.len())],
            r.below(100_000),
            WORDS[r.below(WORDS.len())]
        );
        k += 1;
    }
    out
}

pub fn label(i: usize) -> String {
    format!("n{i}")
}

/// A sparse power-law link graph: `n` nodes, about `m` edges, with
/// Zipf-chosen sources and targets (hubs on both sides).
pub fn links(n: usize, m: usize) -> G {
    let mut r = Rng(1);
    let mut g: G = (0..n)
        .map(|i| (label(i), (title(&mut r, i), Vec::new())))
        .collect();
    let mut out: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for _ in 0..m {
        let (a, b) = (r.zipf(n), r.zipf(n));
        if a != b {
            out[a].insert(b);
        }
    }
    for (i, targets) in out.into_iter().enumerate() {
        let mut t: Vec<usize> = targets.into_iter().collect();
        t.sort();
        g.get_mut(&label(i)).unwrap().1 = t.into_iter().map(label).collect();
    }
    g
}

/// A property graph: `n` nodes with 1-4 KB payloads, 3 edges each.
pub fn props(n: usize) -> G {
    let mut r = Rng(2);
    (0..n)
        .map(|i| {
            let edges = (0..3).map(|_| label(r.below(n))).collect();
            (label(i), (properties(&mut r, i), edges))
        })
        .collect()
}

/// Version `k` from version `k - 1`: local edits around one node `u`,
/// by `k`: edges of `u` and its neighbours added and removed, payloads
/// of the neighbourhood edited, or a small subgraph attached to `u`.
pub fn edit(g: &mut G, k: usize) -> &'static str {
    let mut r = Rng(1000 + k as u64);
    let keys: Vec<String> = g.keys().cloned().collect();
    let u = keys[r.below(keys.len())].clone();
    let mut near: Vec<String> = vec![u.clone()];
    near.extend(g[&u].1.iter().take(9).cloned());
    match k % 3 {
        0 => {
            for (j, v) in near.iter().enumerate() {
                let edges = &mut g.get_mut(v).unwrap().1;
                if j % 2 == 0 || edges.is_empty() {
                    edges.push(keys[r.below(keys.len())].clone());
                } else {
                    edges.remove(r.below(edges.len()));
                }
                edges.sort();
                edges.dedup();
            }
            "edges of a node and its neighbours added and removed"
        }
        1 => {
            for v in &near {
                let p = &mut g.get_mut(v).unwrap().0;
                let at = p.len() / 2;
                let at = (0..=at).rev().find(|&i| p.is_char_boundary(i)).unwrap();
                p.insert_str(at, &format!(" e{k}"));
            }
            "payloads of a node and its neighbours edited"
        }
        _ => {
            let fresh: Vec<String> = (0..10).map(|j| format!("s{k}_{j}")).collect();
            for (j, f) in fresh.iter().enumerate() {
                let edges = vec![fresh[(j + 1) % 10].clone(), u.clone()];
                g.insert(f.clone(), (format!("added {k} {j} {}", WORDS[j]), edges));
            }
            g.get_mut(&u).unwrap().1.push(fresh[0].clone());
            g.get_mut(&u).unwrap().1.sort();
            "a small subgraph attached to a node"
        }
    }
}

/// Each node's edges sorted by label and without repeats, as a read
/// returns them.
fn canon(mut g: G) -> G {
    for (_, e) in g.values_mut() {
        e.sort();
        e.dedup();
    }
    g
}

pub fn chain(first: G) -> Vec<G> {
    let mut out = vec![canon(first)];
    for k in 2..=VERSIONS {
        let mut next = out.last().unwrap().clone();
        edit(&mut next, k);
        out.push(canon(next));
    }
    out
}

fn nodes(g: &G) -> Vec<Node> {
    g.iter()
        .map(|(l, (p, edges))| Node {
            label: l.clone(),
            target: Target::Content(Arg::Whole(Id::of_content(p.as_bytes()))),
            edges: edges
                .iter()
                .map(|t| Edge {
                    to: node_key(t),
                    label: t.clone(),
                })
                .collect(),
        })
        .collect()
}

/// The graph as one text file: a line per node, its payload escaped.
fn to_text(g: &G) -> String {
    let mut out = String::new();
    for (l, (p, edges)) in g {
        out += &format!(
            "{l}\t{}\t{}\n",
            p.replace('\\', "\\\\").replace('\n', "\\n"),
            edges.join(" ")
        );
    }
    out
}

fn from_text(t: &str) -> G {
    t.lines()
        .map(|line| {
            let mut f = line.splitn(3, '\t');
            let l = f.next().unwrap().to_string();
            let p = f.next().unwrap().replace("\\n", "\n").replace("\\\\", "\\");
            let e = f.next().unwrap();
            let edges = e.split(' ').filter(|s| !s.is_empty()).map(String::from);
            (l, (p, edges.collect()))
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Text,
    Tree,
}

/// Store one version; returns the root (tree) or the text's content id.
fn put(repo: &mut Repo<MemStore>, form: Form, tree: &mut Option<Id>, g: &G) -> Id {
    let (edit, id) = match form {
        Form::Text => {
            let id = repo.put_content(to_text(g).as_bytes()).unwrap().id;
            let e = ikam_kernel::Edit::Put {
                path: PATH.into(),
                id,
            };
            (e, id)
        }
        Form::Tree => {
            for (p, _) in g.values() {
                repo.put_content(p.as_bytes()).unwrap();
            }
            let id = repo.put_graph(nodes(g)).unwrap();
            (
                ikam_kernel::Edit::PutGraph {
                    path: PATH.into(),
                    id,
                },
                id,
            )
        }
    };
    *tree = Some(repo.edit_tree(*tree, &[edit]).unwrap());
    id
}

fn read(repo: &Repo<MemStore>, form: Form, id: &Id) -> G {
    match form {
        Form::Text => from_text(std::str::from_utf8(&repo.read_content(id).unwrap()).unwrap()),
        Form::Tree => {
            let mut payloads: HashMap<Id, String> = HashMap::new();
            let mut g = G::new();
            for n in repo.graph_nodes(id).unwrap() {
                let Target::Content(Arg::Whole(c)) = n.target else {
                    panic!("payload")
                };
                let p = payloads
                    .entry(c)
                    .or_insert_with(|| String::from_utf8(repo.read_content(&c).unwrap()).unwrap())
                    .clone();
                let mut e: Vec<String> = n.edges.into_iter().map(|e| e.label).collect();
                e.sort();
                g.insert(n.label, (p, e));
            }
            g
        }
    }
}

pub struct Row {
    pub label: String,
    pub first: usize,
    pub added: Vec<usize>,
    pub heavy: (usize, f64),
    pub reuse: (usize, usize),
    pub write_us: f64,
    pub read_last_us: f64,
    pub delta_us: f64,
    pub delta_chunks: (f64, f64),
    pub applied: (usize, usize),
    pub fsck: usize,
}

fn chain_store(
    form: Form,
    opt: &str,
    weight: f64,
    gs: &[G],
) -> (Repo<MemStore>, Vec<usize>, Vec<Id>, f64) {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = weight;
    configure(&mut repo, opt);
    let (mut tree, mut sizes, mut roots, mut write_s) = (None, Vec::new(), Vec::new(), 0.0);
    for g in gs {
        let before = size(&repo);
        let at = Instant::now();
        roots.push(put(&mut repo, form, &mut tree, g));
        write_s += at.elapsed().as_secs_f64();
        sizes.push(size(&repo) - before);
        repo.commit("main", tree.unwrap(), "v").unwrap();
    }
    (repo, sizes, roots, write_s)
}

/// Apply a node delta to the earlier version's nodes.
fn apply(before: &[Node], changes: &[(String, Option<Node>, Option<Node>)]) -> Vec<Node> {
    let mut m: BTreeMap<String, Node> = before
        .iter()
        .map(|n| (n.label.clone(), n.clone()))
        .collect();
    for (l, _, after) in changes {
        match after {
            Some(n) => m.insert(l.clone(), n.clone()),
            None => m.remove(l),
        };
    }
    m.into_values().collect()
}

pub fn measure(form: Form, opt: &str, label: &str, gs: &[G]) -> Row {
    let last = gs.len() - 1;
    let (mut repo, sizes, roots, write_s) =
        chain_store(form, opt, ikam_kernel::DEFAULT_READ_WEIGHT, gs);
    let mut seen: HashSet<Id> = HashSet::new();
    let mut reuse = (0, 0);
    if form == Form::Tree {
        for (k, root) in roots.iter().enumerate() {
            let ids = crate::anatomy::chunks(&repo, root);
            let mut objs: Vec<Id> = ids.iter().map(|c| c.1).collect();
            for n in repo.graph_nodes(root).unwrap() {
                objs.push(n.target_id());
            }
            if k > 0 {
                reuse.1 += objs.len();
                reuse.0 += objs.iter().filter(|i| seen.contains(i)).count();
            }
            seen.extend(objs);
        }
    }
    assert!(read(&repo, form, &roots[last]) == gs[last]);
    assert!(read(&repo, form, &roots[0]) == gs[0]);
    let read_last_us = time(|| read(&repo, form, &roots[last]));
    let (mut delta_us, mut chunks, mut applied) = (0.0, (0, 0), (0, 0));
    repo.prune_diff = true;
    for k in 1..gs.len() {
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
                delta_us += time(|| repo.graph_diff(&roots[k - 1], &roots[k]).unwrap());
                let (d, c) = repo
                    .graph_diff_counted(&roots[k - 1], &roots[k], true)
                    .unwrap();
                let (full, cu) = repo
                    .graph_diff_counted(&roots[k - 1], &roots[k], false)
                    .unwrap();
                assert_eq!(d, full);
                chunks.0 += c;
                chunks.1 += cu;
                applied.1 += 1;
                let before = repo.graph_nodes(&roots[k - 1]).unwrap();
                if apply(&before, &d) == repo.graph_nodes(&roots[k]).unwrap() {
                    applied.0 += 1;
                }
            }
        }
    }
    let fsck = repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
    let (_, heavy_sizes, _, _) = chain_store(form, opt, HEAVY_READ_WEIGHT, gs);
    let e = last as f64;
    Row {
        label: label.into(),
        first: sizes[0],
        added: sizes[1..].to_vec(),
        heavy: (
            heavy_sizes[0],
            heavy_sizes[1..].iter().sum::<usize>() as f64 / e,
        ),
        reuse,
        write_us: write_s * 1e6 / gs.len() as f64,
        read_last_us,
        delta_us: delta_us / e,
        delta_chunks: (chunks.0 as f64 / e, chunks.1 as f64 / e),
        applied,
        fsck,
    }
}

trait TargetId {
    fn target_id(&self) -> Id;
}

impl TargetId for Node {
    fn target_id(&self) -> Id {
        match self.target {
            Target::Content(a) => a.id(),
            _ => panic!("payload"),
        }
    }
}

pub fn run(which: &str, rows: &[String]) {
    let (name, first) = match which {
        "links" => (
            "power-law links, 100k nodes, ~1M edges",
            links(100_000, 1_000_000),
        ),
        "links-small" => (
            "power-law links, 10k nodes, ~100k edges",
            links(10_000, 100_000),
        ),
        "props" => ("property graph, 10k nodes, 1-4 KB payloads", props(10_000)),
        _ => panic!("graph {which}"),
    };
    let edges: usize = first.values().map(|v| v.1.len()).sum();
    let payload: usize = first.values().map(|v| v.0.len()).sum();
    eprintln!(
        "{name}: {} nodes, {edges} edges, {payload} payload bytes",
        first.len()
    );
    for k in 2..=4 {
        let mut x = first.clone();
        eprintln!("  v{k}: {}", edit(&mut x, k));
    }
    let gs = chain(first);
    let mut out = Vec::new();
    for r in rows {
        let (form, opt) = match r.split_once('+') {
            Some((f, o)) => (f, o.to_string()),
            None => (r.as_str(), "base".to_string()),
        };
        let form = if form == "text" {
            Form::Text
        } else {
            Form::Tree
        };
        let at = Instant::now();
        out.push(measure(form, &opt, r, &gs));
        eprintln!("  {r} done in {:.1} s", at.elapsed().as_secs_f64());
    }
    println!("\n## {name} ({edges} edges), {VERSIONS} versions\n");
    println!(
        "| row | bytes v1 | a version adds, mean / max | heavy: v1, a version adds | objects reused | write µs | read v25 µs | delta µs | delta chunks (unpruned) | applied | fsck |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for r in &out {
        println!(
            "| {} | {} | {:.0} / {} | {}, {:.0} | {} | {:.0} | {:.0} | {:.0} | {:.1} ({:.1}) | {}/{} | {} |",
            r.label,
            r.first,
            r.added.iter().sum::<usize>() as f64 / r.added.len() as f64,
            r.added.iter().max().unwrap(),
            r.heavy.0,
            r.heavy.1,
            if r.reuse.1 > 0 {
                format!("{:.1}%", 100.0 * r.reuse.0 as f64 / r.reuse.1 as f64)
            } else {
                "–".into()
            },
            r.write_us,
            r.read_last_us,
            r.delta_us,
            r.delta_chunks.0,
            r.delta_chunks.1,
            r.applied.0,
            r.applied.1,
            r.fsck
        );
    }
}
