//! E454: stored graph updates made faster without changing how a chunk is
//! stored. E450's harness (E451's combined options with `E450_E451=1`),
//! with E452's result layout and E454's update options chosen by the
//! environment, so the baselines and the options run in one session:
//!
//! - `E454_LAYOUT=label`: E452's label-ordered results (default: hash);
//! - `E454=overlay,shared,whole=N`: E454's `UpdateOptions` (default: none);
//! - `E454_HEAVY=1`: also a run at the heavy read weight (bytes only);
//! - `E454_APPROX=1`: E450's in-memory approximate PageRank measure.
//!
//! "Incl. delta + index" is what a computation waits on: the delta, and the
//! new index for the computations that read it (with `overlay` they read
//! the earlier index and the delta instead, so the index is not counted for
//! them; it is still written, and reported in its own row).
//!
//!   cargo run --release --example e454_graph_update_speed [-- WORKLOADS [REPS [fast]]]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;
use std::time::Instant;

use ikam_kernel::graph_compute::{
    Compute, Computed, Deg, Entry, GraphDelta, Key, Layout, MemGraph, UpdateOptions,
};
use ikam_kernel::net::{Arc, Kind, Net, Place, Transition};
use ikam_kernel::{
    Arg, ChunkSizes, Edge, Edit, Graph, Id, MemStore, Node, Object, Repo, Store, Target, node_key,
};

const VERSIONS: usize = 25;
const HEAVY_READ_WEIGHT: f64 = 48.8;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn edge(to: &str, label: &str) -> Edge {
    Edge {
        to: node_key(to),
        label: label.into(),
    }
}

/// A graph's versions as node sets, with each node's payload text.
struct Workload {
    name: String,
    /// Version 1's nodes, then each later version's edits.
    first: Vec<Node>,
    edits: Vec<BTreeMap<String, Option<Node>>>,
    /// Payload contents to put before the version that names them.
    payloads: Vec<Vec<String>>,
    roots: Vec<String>,
}

const WORDS: [&str; 24] = [
    "graph", "node", "edge", "version", "chunk", "store", "delta", "index", "page", "link",
    "record", "table", "batch", "train", "sample", "search", "merge", "commit", "tree", "root",
    "fragment", "share", "walk", "rank",
];

fn words(rng: &mut Rng, n: usize) -> String {
    (0..n)
        .map(|_| WORDS[rng.below(WORDS.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

/// E451's combined options, on with `E450_E451=1`, for the input graph and
/// every result graph alike: E449's decoded-chunk cache (and its fast net
/// read, which these graphs do not use), E448's chunk frames with parent
/// bounds 16 to 128 KiB, E451's short chunk records, and E447's inline
/// payloads (up to `INLINE_MAX` bytes held in the node, not stored apart).
fn e451() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("E450_E451").is_ok_and(|v| v == "1"))
}

/// The largest payload held inline under `E450_E451=1` (E447's link-graph
/// setting).
const INLINE_MAX: usize = 256;

fn content(text: &str) -> Target {
    if e451() && text.len() <= INLINE_MAX {
        return Target::Inline(text.as_bytes().to_vec());
    }
    Target::Content(Arg::Whole(Id::of_content(text.as_bytes())))
}

fn node(label: String, target: Target, mut edges: Vec<Edge>) -> Node {
    edges.sort();
    edges.dedup();
    Node {
        label,
        target,
        edges,
    }
}

/// Local edits to `g` around a random centre: edges removed and added near
/// it, payloads edited, a small subgraph added, sometimes a node removed.
#[allow(clippy::too_many_arguments)]
fn local_edits(
    g: &BTreeMap<String, Node>,
    by_key: &HashMap<[u8; 8], String>,
    rng: &mut Rng,
    v: usize,
    scale: usize,
    payload: &mut dyn FnMut(&mut Rng, &str) -> String,
    pick_target: &mut dyn FnMut(&mut Rng) -> String,
    elabel: &dyn Fn(&mut Rng) -> String,
) -> (BTreeMap<String, Option<Node>>, Vec<String>) {
    let labels: Vec<&String> = g.keys().collect();
    let mut e: BTreeMap<String, Option<Node>> = BTreeMap::new();
    let mut texts = Vec::new();
    let cur = |e: &BTreeMap<String, Option<Node>>, l: &str| match e.get(l) {
        Some(x) => x.clone(),
        None => g.get(l).cloned(),
    };
    // The centre and its out-neighbourhood (present nodes).
    let centre = labels[rng.below(labels.len())].clone();
    let mut near: Vec<String> = vec![centre.clone()];
    for ed in &g[&centre].edges {
        if let Some(l) = by_key.get(&ed.to)
            && g.contains_key(l)
        {
            near.push(l.clone());
        }
    }
    let pick_near = |rng: &mut Rng| near[rng.below(near.len())].clone();
    for _ in 0..scale {
        let l = pick_near(rng);
        if let Some(mut n) = cur(&e, &l)
            && !n.edges.is_empty()
        {
            n.edges.remove(rng.below(n.edges.len()));
            e.insert(l, Some(n));
        }
    }
    for _ in 0..scale {
        let l = pick_near(rng);
        let t = pick_target(rng);
        if let Some(mut n) = cur(&e, &l) {
            n.edges.push(edge(&t, &elabel(rng)));
            let n = node(n.label, n.target, n.edges);
            e.insert(l, Some(n));
        }
    }
    for _ in 0..scale.div_ceil(2) {
        let l = pick_near(rng);
        if let Some(n) = cur(&e, &l) {
            let text = payload(rng, &l);
            let n = node(n.label, content(&text), n.edges);
            texts.push(text);
            e.insert(l, Some(n));
        }
    }
    // A small subgraph hung off the centre.
    let k = scale.div_ceil(2).max(2);
    let new: Vec<String> = (0..k).map(|j| format!("{centre}~{v}.{j}")).collect();
    for (j, l) in new.iter().enumerate() {
        let mut edges = vec![edge(&new[(j + 1) % k], &elabel(rng))];
        if j == 0 {
            edges.push(edge(&centre, &elabel(rng)));
        }
        edges.push(edge(&pick_target(rng), &elabel(rng)));
        let text = payload(rng, l);
        e.insert(l.clone(), Some(node(l.clone(), content(&text), edges)));
        texts.push(text);
    }
    if let Some(mut n) = cur(&e, &centre) {
        n.edges.push(edge(&new[0], &elabel(rng)));
        e.insert(centre.clone(), Some(node(n.label, n.target, n.edges)));
    }
    if v.is_multiple_of(4) {
        let l = pick_near(rng);
        if l != centre {
            e.insert(l, None);
        }
    }
    (e, texts)
}

#[allow(clippy::too_many_arguments)]
fn chain(
    name: &str,
    first: Vec<Node>,
    first_payloads: Vec<String>,
    rng: &mut Rng,
    scale: usize,
    payload: &mut dyn FnMut(&mut Rng, &str) -> String,
    pick_target: &mut dyn FnMut(&mut Rng, &[String]) -> String,
    elabel: &dyn Fn(&mut Rng) -> String,
    roots: Vec<String>,
) -> Workload {
    let mut g: BTreeMap<String, Node> =
        first.iter().map(|n| (n.label.clone(), n.clone())).collect();
    let mut by_key: HashMap<[u8; 8], String> = g.keys().map(|l| (node_key(l), l.clone())).collect();
    let mut edits = Vec::new();
    let mut payloads = vec![first_payloads];
    for v in 2..=VERSIONS {
        let all: Vec<String> = g.keys().cloned().collect();
        let mut pt = |r: &mut Rng| pick_target(r, &all);
        let (e, texts) = local_edits(&g, &by_key, rng, v, scale, payload, &mut pt, elabel);
        for (l, n) in &e {
            match n {
                Some(n) => {
                    by_key.insert(node_key(l), l.clone());
                    g.insert(l.clone(), n.clone())
                }
                None => g.remove(l),
            };
        }
        edits.push(e);
        payloads.push(texts);
    }
    Workload {
        name: name.into(),
        first,
        edits,
        payloads,
        roots,
    }
}

/// About 100k pages and 1M links: out-degrees from a Pareto law (mean
/// about 10), targets drawn with a strong preference for a few pages.
fn link_graph() -> Workload {
    let n = 100_000;
    let mut rng = Rng(0x2545f4914f6cdd1d);
    let labels: Vec<String> = (0..n).map(|i| format!("page/{i:06}")).collect();
    // A few pages draw most links; which ones is scattered over the labels.
    let pref = |rng: &mut Rng, all: &[String]| -> String {
        let u = rng.unit();
        let raw = ((all.len() as f64) * u * u * u) as usize;
        all[(raw * 7919 + 13) % all.len()].clone()
    };
    let mut first = Vec::new();
    let mut payloads = Vec::new();
    for (i, l) in labels.iter().enumerate() {
        let d = ((5.0 / rng.unit().max(1e-9).sqrt()) as usize).min(2000);
        let mut edges = Vec::new();
        for _ in 0..d {
            let t = pref(&mut rng, &labels);
            if &t != l {
                edges.push(edge(&t, ""));
            }
        }
        let title = format!("Page {i}: {}", words(&mut rng, 3));
        first.push(node(l.clone(), content(&title), edges));
        payloads.push(title);
    }
    let roots = (0..10).map(|i| labels[i * 9973].clone()).collect();
    let mut pay = |rng: &mut Rng, l: &str| format!("Page {l}: {}", words(rng, 3));
    let mut pt = |rng: &mut Rng, all: &[String]| pref(rng, all);
    chain(
        "link",
        first,
        payloads,
        &mut rng,
        10,
        &mut pay,
        &mut pt,
        &|_| String::new(),
        roots,
    )
}

/// About 10k records with 1-4 KB payloads and typed edges.
fn property_graph() -> Workload {
    let n = 10_000;
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let labels: Vec<String> = (0..n).map(|i| format!("rec/{i:05}")).collect();
    let types = ["knows", "owns", "cites", "likes"];
    let elabel = move |rng: &mut Rng| types[rng.below(4)].to_string();
    let pref = |rng: &mut Rng, all: &[String]| -> String {
        let u = rng.unit();
        all[((all.len() as f64) * u * u) as usize % all.len()].clone()
    };
    let body = |rng: &mut Rng, l: &str| {
        let len = 1024 + rng.below(3072);
        let mut s = format!("{l}\n");
        while s.len() < len {
            s.push_str(&words(rng, 8));
            s.push('\n');
        }
        s
    };
    let mut first = Vec::new();
    let mut payloads = Vec::new();
    for l in &labels {
        let d = 1 + rng.below(9);
        let edges = (0..d)
            .map(|_| edge(&pref(&mut rng, &labels), &elabel(&mut rng)))
            .collect();
        let text = body(&mut rng, l);
        first.push(node(l.clone(), content(&text), edges));
        payloads.push(text);
    }
    let roots = (0..5).map(|i| labels[i * 1999].clone()).collect();
    let mut pay = body;
    let mut pt = |rng: &mut Rng, all: &[String]| pref(rng, all);
    chain(
        "prop", first, payloads, &mut rng, 6, &mut pay, &mut pt, &elabel, roots,
    )
}

// E446's synthetic net and its edit chain (examples/e446_net_fragments.rs).
fn arc(place: String, weight: u32) -> Arc {
    Arc { place, weight }
}

fn synth_net(n: usize) -> Net {
    let np = (n / 2).max(4);
    let places = (0..np)
        .map(|i| Place {
            name: format!("p{i}"),
            colored: false,
            initial: if i.is_multiple_of(7) { 3 } else { 0 },
        })
        .collect();
    let kinds = [Kind::Pure, Kind::Cmd, Kind::Agent, Kind::Human];
    let transitions = (0..n)
        .map(|i| {
            let a = (i * 7 + 1) % np;
            let mut b = (i * 13 + 5) % np;
            if b == a {
                b = (b + 1) % np;
            }
            let c = (i * 3 + 2) % np;
            let kind = kinds[i % 4];
            Transition {
                name: format!("t{i}"),
                inputs: vec![
                    arc(format!("p{a}"), 1 + (i % 3) as u32),
                    arc(format!("p{b}"), 1),
                ],
                outputs: vec![arc(format!("p{c}"), 1)],
                kind,
                capability: (kind == Kind::Cmd).then(|| "bench".to_string()),
            }
        })
        .collect();
    Net {
        name: "s".into(),
        version: 1,
        places,
        transitions,
    }
}

fn first_arc(t: &mut Transition) -> &mut u32 {
    match t.inputs.first_mut() {
        Some(a) => &mut a.weight,
        None => &mut t.outputs[0].weight,
    }
}

fn edit_net(net: &mut Net, k: usize) {
    net.version = k as u32;
    let pick = |len: usize| (k.wrapping_mul(2_654_435_761) >> 7) % len.max(1);
    match k % 5 {
        0 => {
            let j = pick(net.transitions.len());
            net.transitions[j].name = format!("{}r{k}", net.transitions[j].name);
        }
        1 => {
            let j = pick(net.transitions.len());
            let w = first_arc(&mut net.transitions[j]);
            *w = *w % 5 + 1;
        }
        2 => {
            let j = pick(net.transitions.len() + 1);
            let q = format!("q{k}");
            let at = pick(net.places.len() + 1);
            net.places.insert(
                at,
                Place {
                    name: q.clone(),
                    colored: false,
                    initial: 0,
                },
            );
            let other = net.places[0].name.clone();
            net.transitions.insert(
                j,
                Transition {
                    name: format!("n{k}"),
                    inputs: vec![arc(other, 1)],
                    outputs: vec![arc(q, 2)],
                    kind: Kind::Pure,
                    capability: None,
                },
            );
        }
        3 => {
            let j = 1 + pick(net.places.len() - 1);
            let gone = net.places.remove(j).name;
            let keep = net.places[0].name.clone();
            for t in &mut net.transitions {
                t.inputs.retain(|a| a.place != gone);
                t.outputs.retain(|a| a.place != gone);
                if t.inputs.is_empty() && t.outputs.is_empty() {
                    t.outputs.push(arc(keep.clone(), 1));
                }
            }
        }
        _ => {
            let start = pick(net.transitions.len());
            for d in 0..20.min(net.transitions.len()) {
                let j = (start + d) % net.transitions.len();
                let w = first_arc(&mut net.transitions[j]);
                *w = *w % 5 + 1;
            }
        }
    }
}

/// A net as a graph: places and transitions are nodes, each arc an edge
/// (place to transition for an input, transition to place for an output)
/// labelled with its weight.
fn net_nodes(net: &Net) -> Vec<Node> {
    let mut out: Vec<Node> = net
        .places
        .iter()
        .map(|p| node(format!("p/{}", p.name), Target::None, Vec::new()))
        .collect();
    let mut place_edges: HashMap<String, Vec<Edge>> = HashMap::new();
    for t in &net.transitions {
        let tl = format!("t/{}", t.name);
        for a in &t.inputs {
            place_edges
                .entry(format!("p/{}", a.place))
                .or_default()
                .push(edge(&tl, &format!("in {}", a.weight)));
        }
        let edges = (t.outputs.iter())
            .map(|a| edge(&format!("p/{}", a.place), &format!("out {}", a.weight)))
            .collect();
        out.push(node(tl, Target::None, edges));
    }
    for n in out.iter_mut() {
        if let Some(e) = place_edges.remove(&n.label) {
            *n = node(n.label.clone(), Target::None, e);
        }
    }
    out
}

fn net_workload(n: usize) -> Workload {
    let mut nets = vec![synth_net(n)];
    for k in 2..=VERSIONS {
        let mut next = nets.last().unwrap().clone();
        edit_net(&mut next, k);
        nets.push(next);
    }
    let first = net_nodes(&nets[0]);
    let mut edits = Vec::new();
    for w in nets.windows(2) {
        let a: BTreeMap<String, Node> = net_nodes(&w[0])
            .into_iter()
            .map(|n| (n.label.clone(), n))
            .collect();
        let b: BTreeMap<String, Node> = net_nodes(&w[1])
            .into_iter()
            .map(|n| (n.label.clone(), n))
            .collect();
        let mut e = BTreeMap::new();
        for (l, n) in &b {
            if a.get(l) != Some(n) {
                e.insert(l.clone(), Some(n.clone()));
            }
        }
        for l in a.keys().filter(|l| !b.contains_key(*l)) {
            e.insert(l.clone(), None);
        }
        edits.push(e);
    }
    // Roots: the places marked initially (a marking spreading).
    let roots = (nets[0].places.iter())
        .filter(|p| p.initial > 0)
        .map(|p| format!("p/{}", p.name))
        .collect();
    Workload {
        name: format!("net{}k", n / 1000),
        first,
        edits,
        payloads: vec![Vec::new(); VERSIONS],
        roots,
    }
}

fn computes(w: &Workload) -> Vec<Compute> {
    vec![
        Compute::Degree,
        Compute::Hops {
            roots: w.roots.clone(),
            k: Some(2),
        },
        Compute::Hops {
            roots: w.roots.clone(),
            k: None,
        },
        Compute::Components,
        Compute::PageRank {
            iters: 20,
            quantum: 12,
        },
    ]
}

/// Stored bytes of an object: a tree or graph chunk is stored as a pointer
/// to the content holding its encoding (`dict.rs`), so both count.
fn stored_size(repo: &Repo<MemStore>, id: &Id) -> usize {
    let s = repo.store().read(id).unwrap();
    match s.as_slice() {
        [t, 2, rest @ ..] if rest.len() == 32 && t.is_ascii_lowercase() => {
            let c = Id::from_bytes(rest.try_into().unwrap());
            s.len() + repo.store().read(&c).map_or(0, |b| b.len())
        }
        _ => s.len(),
    }
}

/// Every chunk id of a graph with its stored and its logical (encoded)
/// size.
fn chunks(repo: &Repo<MemStore>, root: &Id) -> HashMap<Id, (usize, usize)> {
    let mut out = HashMap::new();
    let mut stack = vec![*root];
    while let Some(id) = stack.pop() {
        if out.contains_key(&id) {
            continue;
        }
        let obj = repo.get(&id).unwrap();
        out.insert(id, (stored_size(repo, &id), obj.encode().len()));
        if let Object::Graph(Graph::Children(_, c)) = obj {
            stack.extend(c.into_iter().map(|c| c.1));
        }
    }
    out
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// Per computation, per version (2..=25), what one chain measured.
#[derive(Default, Clone)]
struct Series {
    full_s: Vec<f64>,
    inc_s: Vec<f64>,
    mem_full_s: Vec<f64>,
    mem_inc_s: Vec<f64>,
    full_chunks: Vec<usize>,
    inc_chunks: Vec<usize>,
    changed: Vec<usize>,
    added_bytes: Vec<usize>,
    reuse: Vec<(usize, usize, usize, usize)>,
    equal: usize,
    mem_equal: usize,
}

struct Run {
    write_s: Vec<f64>,
    delta_s: Vec<f64>,
    delta_chunks: Vec<usize>,
    mem_apply_s: Vec<f64>,
    mem_load_s: Vec<f64>,
    by: BTreeMap<String, Series>,
    first_bytes: BTreeMap<String, usize>,
    graph_first_bytes: usize,
    graph_logical: usize,
    graph_chunks: usize,
    checks: String,
    /// The bounded approximate PageRank update: time, nodes re-iterated,
    /// largest and mean relative error against the exact ranks, share of
    /// stored (quantized) values equal to the exact result's.
    approx: Vec<(f64, usize, f64, f64, f64)>,
    /// Store growth from each update (every result, index and their
    /// contents).
    update_bytes: Vec<usize>,
}

fn put_payloads(repo: &mut Repo<MemStore>, texts: &[String]) {
    for t in texts {
        if e451() && t.len() <= INLINE_MAX {
            continue;
        }
        repo.put_content(t.as_bytes()).unwrap();
    }
}

fn store_size(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn mem_values(mem: &MemGraph, c: &Compute) -> Vec<(Key, Vec<Edge>)> {
    mem.compute(c)
}

/// The in-memory baseline's values of one computation, kept between
/// versions.
enum MemVals {
    Deg(HashMap<Key, Deg>),
    Dist(HashMap<Key, u32>),
    Comp(HashMap<Key, u64>),
    None,
}

/// A store with the options the run asks for: `fast` turns on the store's
/// existing speed options (E261 plain hot writes, E294 read cache, E138 read
/// memo), the same for every form measured.
/// E452's layout and E454's options, from the environment.
fn layout() -> Layout {
    match std::env::var("E454_LAYOUT").as_deref() {
        Ok("label") => Layout::Label,
        _ => Layout::Hash,
    }
}

fn options() -> UpdateOptions {
    let mut o = UpdateOptions::default();
    for opt in std::env::var("E454").unwrap_or_default().split(',') {
        match opt {
            "overlay" => o.overlay = true,
            "shared" => o.shared_labels = true,
            "lean" => o.lean_reads = true,
            "sparse" => o.sparse_lookups = true,
            "pagerank" => o.pagerank_kept = true,
            "editdelta" => {}
            "" => {}
            w if w.starts_with("whole=") => o.whole_result_leaves = w[6..].parse().unwrap(),
            other => panic!("unknown E454 option {other}"),
        }
    }
    o
}

/// `E454=editdelta`: the delta comes from the edits
/// (`graph_delta_of_edits`) instead of a diff of the two versions.
fn edit_delta() -> bool {
    std::env::var("E454").is_ok_and(|v| v.split(',').any(|o| o == "editdelta"))
}

fn new_repo(heavy: bool, fast: bool) -> Repo<MemStore> {
    let mut repo = Repo::new(MemStore::default());
    repo.graph_result_layout = layout();
    repo.graph_update = options();
    if heavy {
        repo.read_weight = HEAVY_READ_WEIGHT;
    }
    if fast {
        repo.plain_hot_writes = true;
        repo.read_cache = 512 << 20;
        repo.read_memo = true;
    }
    if e451() {
        repo.graph_cache = 1 << 16;
        repo.net_fast_read = true;
        repo.object_contexts = true;
        repo.graph_chunks = ChunkSizes {
            leaf: (4096, 32768),
            parent: (16384, 131072),
        };
        repo.short_chunk_records = true;
    }
    repo
}

fn run_chain(w: &Workload, heavy: bool, full_too: bool, fast: bool) -> Run {
    let mut repo = new_repo(heavy, fast);
    // The full recompute runs in a store of its own holding the same graph
    // versions and its own earlier results, so it pays for the chunks it
    // writes (the update's store already holds them).
    let mut full_repo = new_repo(heavy, fast);
    if full_too {
        put_payloads(&mut full_repo, &w.payloads[0]);
        let r = full_repo.put_graph(w.first.clone()).unwrap();
        full_repo.graph_compute(&r, &computes(w)).unwrap();
    }
    let mut full_root = None;
    let cs = computes(w);
    put_payloads(&mut repo, &w.payloads[0]);
    let before = store_size(&repo);
    let mut root = repo.put_graph(w.first.clone()).unwrap();
    let graph_first_bytes = store_size(&repo) - before;
    let graph_logical: usize = chunks(&repo, &root).values().map(|c| c.1).sum();
    let graph_chunks = chunks(&repo, &root).len();
    let (mut done, _) = repo.graph_compute(&root, &cs).unwrap();
    let mut first_bytes = BTreeMap::new();
    first_bytes.insert(
        "index".to_string(),
        chunks(&repo, &done.index)
            .values()
            .map(|c| c.0)
            .sum::<usize>(),
    );
    for c in &cs {
        let b = chunks(&repo, &done.results[&c.spec()])
            .values()
            .map(|c| c.0)
            .sum();
        first_bytes.insert(c.spec(), b);
    }
    let mut steps: BTreeMap<String, Id> = BTreeMap::new();
    let mut tree = None;
    let commit = |repo: &mut Repo<MemStore>,
                  done: &Computed,
                  steps: &mut BTreeMap<String, Id>,
                  tree: &mut Option<Id>| {
        let mut edits = vec![Edit::PutGraph {
            path: "graph".into(),
            id: done.graph,
        }];
        let specs: Vec<String> = std::iter::once("index".to_string())
            .chain(done.results.keys().cloned())
            .collect();
        for (i, spec) in specs.iter().enumerate() {
            let s = repo
                .put_graph_step(done, spec, steps.get(spec).copied())
                .unwrap();
            steps.insert(spec.clone(), s);
            edits.push(Edit::PutGraph {
                path: format!("steps/{i}"),
                id: s,
            });
        }
        *tree = Some(repo.edit_tree(*tree, &edits).unwrap());
        repo.commit("main", tree.unwrap(), "v").unwrap();
    };
    commit(&mut repo, &done, &mut steps, &mut tree);
    let mut mem = MemGraph::from_nodes(&w.first);
    let mut mem_vals: Vec<MemVals> = cs
        .iter()
        .map(|c| match c {
            Compute::Degree => MemVals::Deg(mem.values(c)),
            Compute::Hops { .. } => MemVals::Dist(mem.values(c)),
            Compute::Components => MemVals::Comp(mem.values(c)),
            _ => MemVals::None,
        })
        .collect();
    let mut mem_index: HashMap<Key, Entry> = mem.values(&Compute::Index);
    let mut run = Run {
        write_s: Vec::new(),
        delta_s: Vec::new(),
        delta_chunks: Vec::new(),
        mem_apply_s: Vec::new(),
        mem_load_s: Vec::new(),
        by: BTreeMap::new(),
        first_bytes,
        graph_first_bytes,
        graph_logical,
        graph_chunks,
        checks: String::new(),
        approx: Vec::new(),
        update_bytes: Vec::new(),
    };
    let mut pr_prev = mem.pagerank_raw(20);
    let mut nodes_now: BTreeMap<String, Node> = w
        .first
        .iter()
        .map(|n| (n.label.clone(), n.clone()))
        .collect();
    for (vi, e) in w.edits.iter().enumerate() {
        put_payloads(&mut repo, &w.payloads[vi + 1]);
        for (l, n) in e {
            match n {
                Some(n) => nodes_now.insert(l.clone(), n.clone()),
                None => nodes_now.remove(l),
            };
        }
        let (next, delta) = if edit_delta() {
            // E454: the delta from the edits, then the write.
            let at = Instant::now();
            let delta = repo.graph_delta_of_edits(&root, e).unwrap();
            run.delta_s.push(at.elapsed().as_secs_f64());
            let at = Instant::now();
            let next = repo.graph_edit(&root, e.clone()).unwrap();
            run.write_s.push(at.elapsed().as_secs_f64());
            assert_eq!(delta.nodes, repo.graph_delta(&root, &next).unwrap().nodes);
            (next, delta)
        } else {
            let at = Instant::now();
            let next = repo.graph_edit(&root, e.clone()).unwrap();
            run.write_s.push(at.elapsed().as_secs_f64());
            let at = Instant::now();
            let delta: GraphDelta = repo.graph_delta(&root, &next).unwrap();
            run.delta_s.push(at.elapsed().as_secs_f64());
            (next, delta)
        };
        run.delta_chunks.push(delta.chunks_read);
        let prev_chunks: BTreeMap<String, HashMap<Id, (usize, usize)>> =
            std::iter::once(("index".to_string(), done.index))
                .chain(done.results.iter().map(|(k, v)| (k.clone(), *v)))
                .map(|(k, id)| (k, chunks(&repo, &id)))
                .collect();
        let ids_before: HashSet<Id> = repo.store().ids().unwrap().into_iter().collect();
        let (inc, costs) = repo
            .graph_compute_update(&done, &next, Some(&delta), &cs)
            .unwrap();
        let grown: usize = (repo.store().ids().unwrap().iter())
            .filter(|id| !ids_before.contains(id))
            .map(|id| repo.store().read(id).unwrap().len())
            .sum();
        run.update_bytes.push(grown);
        for cost in &costs {
            let s = run.by.entry(cost.spec.clone()).or_default();
            s.inc_s.push(cost.seconds);
            s.inc_chunks.push(cost.chunks_read);
            s.changed.push(cost.changed);
            let id = if cost.spec == "index" {
                inc.index
            } else {
                inc.results[&cost.spec]
            };
            let now = chunks(&repo, &id);
            let prev = &prev_chunks[&cost.spec];
            let new_bytes: usize = now
                .iter()
                .filter(|(k, _)| !prev.contains_key(*k))
                .map(|(_, b)| b.0)
                .sum();
            let new_logical: usize = now
                .iter()
                .filter(|(k, _)| !prev.contains_key(*k))
                .map(|(_, b)| b.1)
                .sum();
            let all_logical: usize = now.values().map(|b| b.1).sum();
            let reused = now.keys().filter(|k| prev.contains_key(*k)).count();
            s.added_bytes.push(new_bytes);
            s.reuse
                .push((reused, now.len(), all_logical - new_logical, all_logical));
        }
        if full_too {
            // Full recompute of each computation, alone, over the fragments,
            // in its own store.
            put_payloads(&mut full_repo, &w.payloads[vi + 1]);
            let fr = full_root.unwrap_or_else(|| full_repo.put_graph(w.first.clone()).unwrap());
            let fnext = full_repo.graph_edit(&fr, e.clone()).unwrap();
            assert_eq!(fnext, next);
            full_root = Some(fnext);
            let repo = &mut full_repo;
            let at = Instant::now();
            let (full_index, _) = repo.graph_compute(&next, &[]).unwrap();
            let t_index = at.elapsed().as_secs_f64();
            let s = run.by.get_mut("index").unwrap();
            s.full_s.push(t_index);
            s.full_chunks.push(run.graph_chunks);
            s.equal += (full_index.index == inc.index) as usize;
            for c in &cs {
                let (id, cost) = repo.graph_compute_one(&next, c).unwrap();
                let s = run.by.get_mut(&c.spec()).unwrap();
                s.full_s.push(cost.seconds);
                s.full_chunks.push(cost.chunks_read);
                s.equal += (id == inc.results[&c.spec()]) as usize;
            }
            // The in-memory baseline: apply the delta, update, compare.
            let at = Instant::now();
            mem.apply(&delta);
            run.mem_apply_s.push(at.elapsed().as_secs_f64());
            let at = Instant::now();
            let fresh_nodes: Vec<Node> = nodes_now.values().cloned().collect();
            let fresh = MemGraph::from_nodes(&fresh_nodes);
            run.mem_load_s.push(at.elapsed().as_secs_f64());
            let at = Instant::now();
            mem.update_index(&mut mem_index, &delta);
            let t = at.elapsed().as_secs_f64();
            let s = run.by.get_mut("index").unwrap();
            s.mem_inc_s.push(t);
            let at = Instant::now();
            let full_vals = mem_values(&fresh, &Compute::Index);
            s.mem_full_s.push(at.elapsed().as_secs_f64());
            let mut mine: Vec<(Key, Entry)> =
                mem_index.iter().map(|(k, v)| (*k, v.clone())).collect();
            mine.sort();
            let theirs: Vec<(Key, Entry)> = full_vals
                .iter()
                .map(|(k, e)| {
                    (
                        *k,
                        <Entry as ikam_kernel::graph_compute::Val>::dec(e).unwrap(),
                    )
                })
                .collect();
            s.mem_equal += (mine == theirs) as usize;
            for (c, mv) in cs.iter().zip(mem_vals.iter_mut()) {
                let at = Instant::now();
                let full_vals = mem_values(&fresh, c);
                let t_full = at.elapsed().as_secs_f64();
                let at = Instant::now();
                let ok = match (c, mv) {
                    (Compute::Degree, MemVals::Deg(m)) => {
                        mem.update_degree(m, &delta);
                        let t = at.elapsed().as_secs_f64();
                        (t, same(m, &full_vals))
                    }
                    (Compute::Hops { roots, k }, MemVals::Dist(m)) => {
                        mem.update_hops(m, &delta, roots, *k);
                        let t = at.elapsed().as_secs_f64();
                        (t, same(m, &full_vals))
                    }
                    (Compute::Components, MemVals::Comp(m)) => {
                        mem.update_components(m, &delta);
                        let t = at.elapsed().as_secs_f64();
                        (t, same(m, &full_vals))
                    }
                    // PageRank has no incremental update: it is the full
                    // recompute.
                    _ => (t_full, true),
                };
                let s = run.by.get_mut(&c.spec()).unwrap();
                s.mem_full_s.push(t_full);
                s.mem_inc_s.push(ok.0);
                s.mem_equal += ok.1 as usize;
            }
        }
        if full_too && std::env::var_os("E454_APPROX").is_some() {
            let at = Instant::now();
            let (approx, area) = mem.pagerank_local(&pr_prev, &delta, 2, 20);
            let t = at.elapsed().as_secs_f64();
            let exact = mem.pagerank_raw(20);
            let (mut worst, mut sum, mut same_q) = (0f64, 0f64, 0usize);
            for (k, x) in &exact {
                let a = approx.get(k).copied().unwrap_or(0) as f64;
                let rel = (a - *x as f64).abs() / *x as f64;
                worst = worst.max(rel);
                sum += rel;
                same_q += ((a as u64) >> 12 == x >> 12) as usize;
            }
            run.approx.push((
                t,
                area,
                worst,
                sum / exact.len() as f64,
                same_q as f64 / exact.len() as f64,
            ));
            pr_prev = exact;
        }
        root = next;
        done = inc;
        commit(&mut repo, &done, &mut steps, &mut tree);
    }
    if full_too {
        let gc = repo.gc().unwrap();
        let fsck = repo.fsck().unwrap().len();
        let deep = repo.fsck_deep().unwrap().len();
        let mut checked = 0;
        let mut ok = 0;
        for s in steps.values() {
            checked += 1;
            ok += repo.graph_step_check(s).unwrap() as usize;
        }
        run.checks = format!(
            "gc deleted {gc}; fsck {fsck} and fsck_deep {deep} failures; last version's {ok}/{checked} steps rebuilt equal after gc"
        );
    }
    run
}

fn same<V: ikam_kernel::graph_compute::Val + Ord>(
    m: &HashMap<Key, V>,
    full: &[(Key, Vec<Edge>)],
) -> bool {
    let mut mine: Vec<(Key, V)> = m.iter().map(|(k, v)| (*k, v.clone())).collect();
    mine.sort();
    let theirs: Vec<(Key, V)> = full.iter().map(|(k, e)| (*k, V::dec(e).unwrap())).collect();
    mine == theirs
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let which: Vec<String> = args
        .get(1)
        .map_or("link,prop,net1k,net10k", |s| s.as_str())
        .split(',')
        .map(String::from)
        .collect();
    let reps: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
    if which == ["links"] {
        links(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20_000));
        return;
    }
    let fast = args.get(3).is_some_and(|s| s == "fast");
    println!(
        "store options: {}",
        if fast {
            "fast (plain hot writes, read cache, read memo)"
        } else {
            "default"
        }
    );
    println!("layout {:?}, E454 options {:?}", layout(), options());
    if e451() {
        println!(
            "E451 options (E450_E451=1): graph_cache 65536, net_fast_read, object_contexts, \
             graph_chunks leaf 4096:32768 parent 16384:131072, short_chunk_records, \
             payloads up to {INLINE_MAX} B inline"
        );
    }
    for name in which {
        let at = Instant::now();
        let w = match name.as_str() {
            "link" => link_graph(),
            "prop" => property_graph(),
            "net1k" => net_workload(1000),
            "net10k" => net_workload(10_000),
            _ => panic!("unknown workload {name}"),
        };
        let edges: usize = w.first.iter().map(|n| n.edges.len()).sum();
        let edit_nodes: f64 = mean(&w.edits.iter().map(|e| e.len() as f64).collect::<Vec<_>>());
        println!(
            "## {} : {} nodes, {} edges, {:.1} nodes edited a version (built in {:.1} s)",
            w.name,
            w.first.len(),
            edges,
            edit_nodes,
            at.elapsed().as_secs_f64()
        );
        let mut runs = Vec::new();
        for r in 0..reps {
            let at = Instant::now();
            runs.push(run_chain(&w, false, true, fast));
            eprintln!("{} rep {r}: {:.1} s", w.name, at.elapsed().as_secs_f64());
        }
        let heavy = std::env::var_os("E454_HEAVY").map(|_| run_chain(&w, true, false, fast));
        let r0 = &runs[0];
        println!(
            "input graph: v1 {} bytes stored ({} encoded) in {} chunks; a version's write {:.0} us; delta {:.0} us reading {:.1} chunks; in-memory: delta applied {:.0} us, whole graph loaded {:.0} us",
            r0.graph_first_bytes,
            r0.graph_logical,
            r0.graph_chunks,
            median(runs.iter().map(|r| mean(&r.write_s)).collect()) * 1e6,
            median(runs.iter().map(|r| mean(&r.delta_s)).collect()) * 1e6,
            mean(
                &r0.delta_chunks
                    .iter()
                    .map(|x| *x as f64)
                    .collect::<Vec<_>>()
            ),
            median(runs.iter().map(|r| mean(&r.mem_apply_s)).collect()) * 1e6,
            median(runs.iter().map(|r| mean(&r.mem_load_s)).collect()) * 1e6,
        );
        println!(
            "store growth from an update (index and every result): {:.0} bytes at the default weight, {:.0} at the heavy weight",
            mean(
                &r0.update_bytes
                    .iter()
                    .map(|x| *x as f64)
                    .collect::<Vec<_>>()
            ),
            heavy.as_ref().map_or(f64::NAN, |h| mean(
                &h.update_bytes.iter().map(|x| *x as f64).collect::<Vec<_>>()
            )),
        );
        println!("checks: {}", r0.checks);
        println!();
        println!(
            "| computation | full over fragments (us) | incremental (us) | speedup | incl. delta + index (us) | speedup | in-memory full (us) | in-memory incremental (us) | chunks read full / incremental | keys changed | result v1 bytes | result adds a version: default / heavy (bytes) | result chunks reused from the version before (count / encoded bytes) | incremental = full | in-memory = full |"
        );
        println!(
            "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"
        );
        let delta_us = median(runs.iter().map(|r| mean(&r.delta_s)).collect()) * 1e6;
        let index_us = median(runs.iter().map(|r| mean(&r.by["index"].inc_s)).collect()) * 1e6;
        let specs: Vec<String> = std::iter::once("index".to_string())
            .chain(computes(&w).iter().map(Compute::spec))
            .collect();
        for spec in specs {
            let m = |f: &dyn Fn(&Series) -> &Vec<f64>| -> f64 {
                median(runs.iter().map(|r| mean(f(&r.by[&spec]))).collect()) * 1e6
            };
            let full = m(&|s| &s.full_s);
            let inc = m(&|s| &s.inc_s);
            let mf = m(&|s| &s.mem_full_s);
            let mi = m(&|s| &s.mem_inc_s);
            let s0 = &r0.by[&spec];
            let total = match spec.as_str() {
                "index" => inc + delta_us,
                s if s.starts_with("pagerank") => inc,
                s if s.starts_with("degree") => inc + delta_us,
                _ if options().overlay => inc + delta_us,
                _ => inc + delta_us + index_us,
            };
            let fc = mean(&s0.full_chunks.iter().map(|x| *x as f64).collect::<Vec<_>>());
            let ic = mean(&s0.inc_chunks.iter().map(|x| *x as f64).collect::<Vec<_>>());
            let ch = mean(&s0.changed.iter().map(|x| *x as f64).collect::<Vec<_>>());
            let add = mean(&s0.added_bytes.iter().map(|x| *x as f64).collect::<Vec<_>>());
            let hadd = heavy.as_ref().map_or(f64::NAN, |h| {
                mean(
                    &h.by[&spec]
                        .added_bytes
                        .iter()
                        .map(|x| *x as f64)
                        .collect::<Vec<_>>(),
                )
            });
            let (ru, rn, rb, ra) = s0.reuse.iter().fold((0, 0, 0, 0), |a, x| {
                (a.0 + x.0, a.1 + x.1, a.2 + x.2, a.3 + x.3)
            });
            let name = if spec == "index" {
                "index".to_string()
            } else {
                Compute::parse(&spec).unwrap().name().to_string()
            };
            let n = w.edits.len();
            println!(
                "| {name} | {full:.0} | {inc:.0} | {:.1}x | {total:.0} | {:.1}x | {mf:.0} | {mi:.1} | {fc:.0} / {ic:.1} | {ch:.1} | {} | {add:.0} / {hadd:.0} | {:.1}% / {:.1}% | {}/{n} | {}/{n} |",
                full / inc,
                full / total,
                r0.first_bytes[&spec],
                100.0 * ru as f64 / rn as f64,
                100.0 * rb as f64 / ra as f64,
                runs.iter().map(|r| r.by[&spec].equal).min().unwrap(),
                runs.iter().map(|r| r.by[&spec].mem_equal).min().unwrap(),
            );
        }
        let ap = &r0.approx;
        println!();
        if ap.is_empty() {
            continue;
        }
        println!(
            "PageRank, bounded approximate update (2 hops from the delta, 20 iterations, in memory): {:.0} us a version against {:.0} us for the exact in-memory recompute; {:.0} nodes re-iterated of {}; error against the exact ranks: largest {:.2e}, mean {:.2e}; stored values equal to the exact result's {:.2}%",
            median(
                runs.iter()
                    .map(|r| mean(&r.approx.iter().map(|a| a.0).collect::<Vec<_>>()))
                    .collect()
            ) * 1e6,
            median(
                runs.iter()
                    .map(|r| mean(&r.by[&computes(&w)[4].spec()].mem_full_s))
                    .collect()
            ) * 1e6,
            mean(&ap.iter().map(|a| a.1 as f64).collect::<Vec<_>>()),
            w.first.len(),
            ap.iter().map(|a| a.2).fold(0.0, f64::max),
            mean(&ap.iter().map(|a| a.3).collect::<Vec<_>>()),
            100.0 * mean(&ap.iter().map(|a| a.4).collect::<Vec<_>>()),
        );
        println!();
    }
}

fn links(objects: usize) {
    let mut src = Repo::new(MemStore::default());
    let mut rng = Rng(0x7f4a7c15);
    let mut tree = None;
    let mut commits = 0;
    let grow = |src: &mut Repo<MemStore>, rng: &mut Rng, tree: &mut Option<Id>, n: usize| {
        for c in 0..n {
            let mut edits = Vec::new();
            for _ in 0..4 {
                let path = format!(
                    "d{:03}/e{:02}/f{:02}",
                    rng.below(300),
                    rng.below(10),
                    rng.below(20)
                );
                let id = src
                    .put_content(format!("{path} {c} {}", rng.next()).as_bytes())
                    .unwrap()
                    .id;
                edits.push(Edit::Put { path, id });
            }
            *tree = Some(src.edit_tree(*tree, &edits).unwrap());
            src.commit("main", tree.unwrap(), "c").unwrap();
        }
    };
    let count = |src: &Repo<MemStore>| {
        let s = src.store();
        s.ids()
            .unwrap()
            .iter()
            .filter(|id| {
                let b = s.read(id).unwrap();
                !matches!(
                    b.first(),
                    Some(
                        b'B' | b'W' | b'Z' | b'Y' | b'X' | b'R' | b'P' | b'K' | b'S' | b'Q' | b'M'
                    )
                )
            })
            .count()
    };
    while count(&src) < objects {
        grow(&mut src, &mut rng, &mut tree, 100);
        commits += 100;
    }
    let ids = src.store().ids().unwrap();
    let objs = count(&src);
    println!(
        "## links: {} stored ids, {objs} objects (trees and commits), {commits} commits",
        ids.len()
    );
    let mut ix_repo = Repo::new(MemStore::default());
    ix_repo.plain_hot_writes = true;
    let at = Instant::now();
    let (mut ix, u) = ix_repo.link_index_update(&src, None).unwrap();
    let build = at.elapsed().as_secs_f64();
    let ix_bytes = store_size(&ix_repo);
    // E454: the same links in segments by commit order, in a store of
    // their own.
    let mut seg_repo = Repo::new(MemStore::default());
    seg_repo.plain_hot_writes = true;
    let at = Instant::now();
    let (mut seg, su) = seg_repo.link_segments_update(&src, None).unwrap();
    let seg_build = at.elapsed().as_secs_f64();
    let seg_bytes = store_size(&seg_repo);
    let sample: Vec<Id> = (0..200).map(|_| ids[rng.below(ids.len())]).collect();
    let mut scan_s = Vec::new();
    let mut ix_s = Vec::new();
    let mut found = 0;
    let mut seg_s = Vec::new();
    for (i, id) in sample.iter().enumerate() {
        let at = Instant::now();
        let mut a = src.used_by(id).unwrap();
        if i < 20 {
            scan_s.push(at.elapsed().as_secs_f64());
        }
        a.sort();
        let at = Instant::now();
        let b = ix_repo.used_by_indexed(&ix, id).unwrap();
        ix_s.push(at.elapsed().as_secs_f64());
        assert_eq!(a, b);
        let at = Instant::now();
        let c = seg_repo.used_by_segments(&seg, id).unwrap();
        seg_s.push(at.elapsed().as_secs_f64());
        assert_eq!(a, c);
        found += b.len();
    }
    // Updates: 100 more commits at a time, UPDATES times; every tenth
    // with a gc of loose trees, so objects are lost too.
    let updates: usize = std::env::var("E454_UPDATES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);
    let mut up_s = Vec::new();
    let mut up_added = Vec::new();
    let mut up_read = Vec::new();
    let mut seg_up_s = Vec::new();
    let mut seg_up_read = Vec::new();
    let mut seg_merged = 0;
    let mut seg_counts = Vec::new();
    let mut seg_after_s = Vec::new();
    let mut ix_after_s = Vec::new();
    let mut removed = 0;
    for i in 0..updates {
        grow(&mut src, &mut rng, &mut tree, 100);
        if i % 4 == 3 {
            // Loose trees no ref holds, then a gc: the index loses them.
            let mut loose = tree;
            for j in 0..20 {
                let id = src
                    .put_content(format!("loose {i} {j}").as_bytes())
                    .unwrap()
                    .id;
                let path = format!("loose/{j}");
                loose = Some(src.edit_tree(loose, &[Edit::Put { path, id }]).unwrap());
            }
            let _ = loose;
            let (next, _) = ix_repo.link_index_update(&src, Some(&ix)).unwrap();
            ix = next;
            let (next, _) = seg_repo.link_segments_update(&src, Some(&seg)).unwrap();
            seg = next;
            src.gc().unwrap();
        }
        let at = Instant::now();
        let (next, u) = ix_repo.link_index_update(&src, Some(&ix)).unwrap();
        up_s.push(at.elapsed().as_secs_f64());
        up_added.push(u.added as f64);
        up_read.push(u.index_chunks_read as f64);
        removed += u.removed;
        ix = next;
        let at = Instant::now();
        let (next, su) = seg_repo.link_segments_update(&src, Some(&seg)).unwrap();
        seg_up_s.push(at.elapsed().as_secs_f64());
        seg_up_read.push(su.chunks_read as f64);
        seg_merged += su.merged;
        seg = next;
        seg_counts.push(seg.segments.len() as f64);
    }
    for id in sample.iter().take(50) {
        let mut a = src.used_by(id).unwrap();
        a.sort();
        let at = Instant::now();
        let b = ix_repo.used_by_indexed(&ix, id).unwrap();
        ix_after_s.push(at.elapsed().as_secs_f64());
        let at = Instant::now();
        let c = seg_repo.used_by_segments(&seg, id).unwrap();
        seg_after_s.push(at.elapsed().as_secs_f64());
        assert_eq!(a, b);
        assert_eq!(a, c);
    }
    let seg_bytes_after = store_size(&seg_repo);
    let ix_bytes_after = store_size(&ix_repo);
    println!();
    println!("| measure | value |");
    println!("| --- | --- |");
    println!(
        "| used_by, full scan (median of 20 queries) | {:.0} us |",
        median(scan_s) * 1e6
    );
    println!(
        "| used_by through the link index (median of 200) | {:.0} us |",
        median(ix_s.clone()) * 1e6
    );
    println!("| sources found (200 queries) | {found} |");
    println!(
        "| index build ({} objects read) | {:.0} ms |",
        u.objects_read,
        build * 1e3
    );
    println!("| index stored (graph and index, plain writes) | {ix_bytes} bytes |");
    println!(
        "| update after 100 commits (median of 5; {:.0} objects added, {:.0} index chunks read) | {:.1} ms |",
        mean(&up_added),
        mean(&up_read),
        median(up_s.clone()) * 1e3
    );
    println!("| objects lost to gc over the updates | {removed} |");
    println!("| results equal to the scan | 250/250 |");
    println!();
    println!("| measure | E452 link index | E454 segments |");
    println!("| --- | --- | --- |");
    println!(
        "| build | {:.0} ms | {:.0} ms ({} objects read) |",
        build * 1e3,
        seg_build * 1e3,
        su.objects_read
    );
    println!("| stored at build | {ix_bytes} bytes | {seg_bytes} bytes |");
    println!(
        "| stored after {updates} updates | {ix_bytes_after} bytes | {seg_bytes_after} bytes |"
    );
    println!(
        "| update after 100 commits, median of {updates} | {:.1} ms | {:.1} ms |",
        median(up_s.clone()) * 1e3,
        median(seg_up_s.clone()) * 1e3
    );
    println!(
        "| update, mean of {updates} (merges included) | {:.1} ms | {:.1} ms |",
        mean(&up_s) * 1e3,
        mean(&seg_up_s) * 1e3
    );
    println!(
        "| update, slowest | {:.1} ms | {:.1} ms |",
        up_s.iter().copied().fold(0.0, f64::max) * 1e3,
        seg_up_s.iter().copied().fold(0.0, f64::max) * 1e3
    );
    println!(
        "| chunks read by an update (mean) | {:.0} index chunks | {:.0} segment chunks ({seg_merged} merges) |",
        mean(&up_read),
        mean(&seg_up_read)
    );
    println!(
        "| used_by at build (median of 200) | {:.0} us | {:.0} us |",
        median(ix_s.clone()) * 1e6,
        median(seg_s.clone()) * 1e6
    );
    println!(
        "| used_by after the updates (median of 50) | {:.0} us | {:.0} us ({:.0} segments) |",
        median(ix_after_s) * 1e6,
        median(seg_after_s) * 1e6,
        seg_counts.last().copied().unwrap_or(0.0)
    );
}
