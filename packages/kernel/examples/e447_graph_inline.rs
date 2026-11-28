//! E447 beyond nets: graph objects stored with `put_graph`, their node
//! payloads as contents of their own (each a content and a 32-byte id, as
//! E446 stores a net's elements) or inline in the leaf chunks
//! (`Target::Inline`), each put through a 25-version chain of local edits
//! made with `graph_edit`.
//!
//! - links: a sparse power-law link graph, 100k nodes and about 1M edges,
//!   a short title on each node.
//! - props: a property graph, 10k nodes with 1-4 KB of text each and a
//!   few edges.
//!
//! Each version: add and remove edges on a few nodes, edit a few payloads,
//! add a small subgraph. Per graph and form: bytes v1, bytes a version adds
//! at the default and the heavy read weight, chunks and payload objects
//! reused, write (one `graph_edit`), read (every node and payload of the
//! last version) and delta (`graph_diff` of consecutive versions, the
//! payloads of changed nodes read) time, medians of runs.
//!
//!   cargo run --release --example e447_graph_inline [-- links,props [SCALE]]
//!
//! SCALE divides the node counts (default 1).

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use ikam_kernel::{Arg, Edge, Graph, Id, MemStore, Node, Object, Repo, Store, Target, node_key};

const VERSIONS: usize = 25;
const HEAVY_READ_WEIGHT: f64 = 48.8;

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
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Power-law index below `n`: small indices far more often.
    fn skewed(&mut self, n: usize) -> usize {
        ((n as f64) * self.unit().powi(3)) as usize % n
    }
}

/// A synthetic vocabulary: pronounceable words of 2-4 syllables.
fn vocab(n: usize) -> Vec<String> {
    let syl = [
        "ka", "lo", "mi", "ren", "to", "sa", "vel", "dor", "ni", "pa", "qua", "ti", "ber", "on",
        "es", "ul", "ra", "ze", "mon", "fi",
    ];
    let mut rng = Rng(7);
    (0..n)
        .map(|_| {
            let k = 2 + rng.below(3);
            (0..k).map(|_| syl[rng.below(syl.len())]).collect()
        })
        .collect()
}

fn words(rng: &mut Rng, vocab: &[String], n: usize) -> String {
    let mut out = String::new();
    for i in 0..n {
        if i > 0 {
            out.push(if rng.below(12) == 0 { '\n' } else { ' ' });
        }
        out += &vocab[rng.skewed(vocab.len())];
    }
    out
}

#[derive(Clone)]
struct G {
    /// label -> (payload, edge targets)
    nodes: BTreeMap<String, (String, Vec<String>)>,
}

fn label(kind: &str, i: usize) -> String {
    format!("{kind}{i:06}")
}

fn links_graph(n: usize, vocab: &[String]) -> G {
    let mut rng = Rng(1);
    let mut nodes = BTreeMap::new();
    for i in 0..n {
        let len = 2 + rng.below(4);
        let title = words(&mut rng, vocab, len);
        // Out-degree: mostly a few, some many (mean about 10).
        let deg = 1 + (25.0 * rng.unit().powi(2)) as usize + rng.below(6);
        let targets = (0..deg).map(|_| label("page", rng.skewed(n))).collect();
        nodes.insert(label("page", i), (title, targets));
    }
    G { nodes }
}

fn props_graph(n: usize, vocab: &[String]) -> G {
    let mut rng = Rng(2);
    let mut nodes = BTreeMap::new();
    for i in 0..n {
        let len = 150 + rng.below(450); // about 1-4 KB of words
        let body = words(&mut rng, vocab, len);
        let deg = 1 + rng.below(5);
        let targets = (0..deg).map(|_| label("item", rng.below(n))).collect();
        nodes.insert(label("item", i), (body, targets));
    }
    G { nodes }
}

/// Version `k` from the one before: local edits.
fn edit(g: &mut G, k: usize, vocab: &[String], kind: &str) -> Vec<String> {
    let mut rng = Rng(1000 + k as u64);
    let labels: Vec<String> = g.nodes.keys().cloned().collect();
    let mut touched = Vec::new();
    for _ in 0..5 {
        // Add and remove edges.
        let l = &labels[rng.below(labels.len())];
        let t = labels[rng.skewed(labels.len())].clone();
        let e = &mut g.nodes.get_mut(l).unwrap().1;
        e.push(t);
        if e.len() > 2 {
            e.remove(0);
        }
        touched.push(l.clone());
    }
    for _ in 0..3 {
        // Edit a payload: replace a few words.
        let l = &labels[rng.below(labels.len())];
        let p = &mut g.nodes.get_mut(l).unwrap().0;
        let mut w: Vec<&str> = p.split(' ').collect();
        let fresh = words(&mut rng, vocab, 2);
        let at = rng.below(w.len());
        w[at] = &fresh;
        *p = w.join(" ");
        touched.push(l.clone());
    }
    // A small subgraph: 5 new nodes linked among themselves and to one old.
    let old = labels[rng.below(labels.len())].clone();
    let len = if kind == "item" { 300 } else { 3 };
    for j in 0..5 {
        let l = format!("{kind}x{k:03}{j}");
        let mut targets = vec![format!("{kind}x{k:03}{}", (j + 1) % 5)];
        if j == 0 {
            targets.push(old.clone());
        }
        g.nodes
            .insert(l.clone(), (words(&mut rng, vocab, len), targets));
        touched.push(l);
    }
    touched
}

#[derive(Clone, Copy, PartialEq)]
enum Form {
    Content,
    Inline,
    /// `put_payload` with this inline limit.
    Upto(usize),
}

fn node(
    repo: &mut Repo<MemStore>,
    form: Form,
    l: &str,
    (payload, targets): &(String, Vec<String>),
) -> Node {
    let target = match form {
        Form::Content => {
            Target::Content(Arg::Whole(repo.put_content(payload.as_bytes()).unwrap().id))
        }
        Form::Inline => Target::Inline(payload.as_bytes().to_vec()),
        Form::Upto(max) => repo.put_payload(payload.as_bytes(), max).unwrap(),
    };
    let mut edges: Vec<Edge> = targets
        .iter()
        .map(|t| Edge {
            to: node_key(t),
            label: "link".into(),
        })
        .collect();
    edges.sort();
    edges.dedup();
    Node {
        label: l.into(),
        target,
        edges,
    }
}

fn size(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.stored_len(id).unwrap())
        .sum()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn time<T>(mut f: impl FnMut() -> T) -> f64 {
    let (mut runs, mut total) = (Vec::new(), 0.0);
    while runs.len() < 3 || total < 0.2 {
        let at = Instant::now();
        std::hint::black_box(f());
        let t = at.elapsed().as_secs_f64();
        runs.push(t);
        total += t;
    }
    median(runs) * 1e6
}

fn read_all(repo: &Repo<MemStore>, root: &Id) -> usize {
    let mut bytes = 0;
    for n in repo.graph_nodes(root).unwrap() {
        bytes += match &n.target {
            Target::Content(a) => repo.read_content(&a.id()).unwrap().len(),
            Target::Inline(b) => b.len(),
            _ => 0,
        };
    }
    bytes
}

/// Chunk ids under `root` and the payload contents its nodes name.
fn objects(repo: &Repo<MemStore>, root: &Id) -> (Vec<Id>, Vec<Id>, usize) {
    let (mut chunks, mut payloads, mut stack, mut nodes) = (Vec::new(), Vec::new(), vec![*root], 0);
    while let Some(id) = stack.pop() {
        chunks.push(id);
        match repo.get(&id).unwrap() {
            Object::Graph(Graph::Children(_, cs)) => stack.extend(cs.into_iter().map(|c| c.1)),
            Object::Graph(Graph::Nodes(ns)) => {
                nodes += ns.len();
                payloads.extend(ns.iter().filter_map(|n| match &n.target {
                    Target::Content(a) => Some(a.id()),
                    _ => None,
                }))
            }
            _ => unreachable!(),
        }
    }
    (chunks, payloads, nodes)
}

struct Row {
    first: usize,
    adds: f64,
    heavy: (usize, f64),
    chunks_reused: (usize, usize),
    payloads_reused: (usize, usize),
    write_us: f64,
    read_us: f64,
    delta_us: f64,
    nodes: usize,
}

fn chain(base: &G, vocab: &[String], kind: &str, form: Form, weight: f64, full: bool) -> Row {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = weight;
    repo.prune_diff = true;
    let mut g = base.clone();
    let mut nodes = Vec::new();
    for (l, v) in &g.nodes {
        nodes.push(node(&mut repo, form, l, v));
    }
    let mut roots = vec![repo.put_graph(nodes).unwrap()];
    let first = size(&repo);
    let (mut writes, mut chunks_reused, mut payloads_reused) = (Vec::new(), (0, 0), (0, 0));
    let mut seen: HashSet<Id> = HashSet::new();
    let (c, p, _) = objects(&repo, &roots[0]);
    seen.extend(c.into_iter().chain(p));
    for k in 2..=VERSIONS {
        let touched = edit(&mut g, k, vocab, kind);
        let at = Instant::now();
        let mut edits = BTreeMap::new();
        for l in touched {
            let n = node(&mut repo, form, &l, &g.nodes[&l]);
            edits.insert(l, Some(n));
        }
        let root = repo.graph_edit(roots.last().unwrap(), edits).unwrap();
        writes.push(at.elapsed().as_secs_f64() * 1e6);
        if full {
            let (c, p, _) = objects(&repo, &root);
            for (ids, acc) in [(&c, &mut chunks_reused), (&p, &mut payloads_reused)] {
                acc.1 += ids.len();
                acc.0 += ids.iter().filter(|id| seen.contains(id)).count();
            }
            seen.extend(c.into_iter().chain(p));
        }
        roots.push(root);
    }
    let adds = (size(&repo) - first) as f64 / (VERSIONS - 1) as f64;
    let last = *roots.last().unwrap();
    // Every version reads back as the chain built it.
    let want: Vec<Node> = {
        let mut r = Repo::new(MemStore::default());
        let mut ns: Vec<Node> = g
            .nodes
            .iter()
            .map(|(l, v)| node(&mut r, form, l, v))
            .collect();
        ns.sort_by(|a, b| a.label.cmp(&b.label));
        ns
    };
    assert!(
        repo.graph_nodes(&last).unwrap() == want,
        "last version reads back"
    );
    let (mut read_us, mut delta_us, mut nodes) = (0.0, 0.0, 0);
    if full {
        read_us = time(|| read_all(&repo, &last));
        let mut ds = Vec::new();
        for k in 1..roots.len() {
            ds.push(time(|| {
                let mut b = 0;
                for (_, _, after) in repo.graph_diff(&roots[k - 1], &roots[k]).unwrap() {
                    if let Some(Node {
                        target: Target::Content(a),
                        ..
                    }) = after
                    {
                        b += repo.read_content(&a.id()).unwrap().len();
                    }
                }
                b
            }));
        }
        delta_us = median(ds);
        nodes = objects(&repo, &last).2;
        assert!(repo.fsck().unwrap().is_empty() && repo.fsck_deep().unwrap().is_empty());
    }
    Row {
        first,
        adds,
        heavy: (0, 0.0),
        chunks_reused,
        payloads_reused,
        write_us: median(writes),
        read_us,
        delta_us,
        nodes,
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let which: Vec<String> = args
        .next()
        .map(|s| s.split(',').map(String::from).collect())
        .unwrap_or_else(|| vec!["links".into(), "props".into()]);
    let scale: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let vocab = vocab(3000);
    println!(
        "| graph | payloads | nodes | edges | payload bytes | bytes v1 | a version adds | heavy: v1, adds | chunks reused | payload objects reused | write µs | read µs | delta µs |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for w in &which {
        let (g, kind) = match w.as_str() {
            "links" => (links_graph(100_000 / scale, &vocab), "page"),
            "props" => (props_graph(10_000 / scale, &vocab), "item"),
            _ => panic!("links or props"),
        };
        let edges: usize = g.nodes.values().map(|v| v.1.len()).sum();
        let payload: usize = g.nodes.values().map(|v| v.0.len()).sum();
        let forms = match kind {
            "item" => vec![Form::Content, Form::Inline, Form::Upto(2048)],
            _ => vec![Form::Content, Form::Inline],
        };
        for form in forms {
            let at = Instant::now();
            let mut r = chain(
                &g,
                &vocab,
                kind,
                form,
                ikam_kernel::DEFAULT_READ_WEIGHT,
                true,
            );
            let h = chain(&g, &vocab, kind, form, HEAVY_READ_WEIGHT, false);
            r.heavy = (h.first, h.adds);
            eprintln!("{w} done in {:.1} s", at.elapsed().as_secs_f64());
            let pct = |p: (usize, usize)| match p.1 {
                0 => "–".to_string(),
                _ => format!("{:.1}%", 100.0 * p.0 as f64 / p.1 as f64),
            };
            println!(
                "| {w} | {} | {} | {edges} | {payload} | {} | {:.0} | {}, {:.0} | {} | {} | {:.0} | {:.0} | {:.0} |",
                match form {
                    Form::Content => "a content each (as E446)".to_string(),
                    Form::Inline => "inline (E447)".into(),
                    Form::Upto(m) => format!("inline up to {m} B, else a content"),
                },
                r.nodes,
                r.first,
                r.adds,
                r.heavy.0,
                r.heavy.1,
                pct(r.chunks_reused),
                pct(r.payloads_reused),
                r.write_us,
                r.read_us,
                r.delta_us,
            );
        }
    }
}
