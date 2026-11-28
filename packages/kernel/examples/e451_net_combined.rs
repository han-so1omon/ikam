//! E451: E449's read, write and firing paths over E447's inline form,
//! measured together on E446's chain and on two general graphs, beside
//! text (E422), E446's tree, E447 alone and E449 alone.
//!
//!   cargo run --release --example e451_net_combined -- nets [SIZES] [RUNS]
//!   cargo run --release --example e451_net_combined -- fire [CANDIDATES]
//!   cargo run --release --example e451_net_combined -- graphs [SCALE]
//!
//! SIZES is a comma-separated list (default 1000,10000). Times are medians
//! of RUNS runs (default 7) of each measurement; run the whole harness
//! three times and take the median of the three (other builds share the
//! machine). The nets and the 25-version chain are E446's, copied so the
//! examples build apart.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Instant;

use ikam_kernel::net::{Arc, Kind, Layout, Net, Place, Transition};
use ikam_kernel::{
    Arg, ChunkSizes, Edge, Graph, Id, MemStore, Node, Object, Repo, Store, Target, node_key,
};

const VERSIONS: usize = 25;
const PATH: &str = "nets/s.net";
/// The planner's weight under the bench's heavy workload (E069, E446).
const HEAVY_READ_WEIGHT: f64 = 48.8;

fn arc(place: String, weight: u32) -> Arc {
    Arc { place, weight }
}

/// A synthetic net of `n` transitions over `max(4, n/2)` places (E446).
fn synth(n: usize) -> Net {
    let np = (n / 2).max(4);
    let places = (0..np)
        .map(|i| Place {
            name: format!("p{i}"),
            colored: false,
            initial: if i % 7 == 0 { 3 } else { 0 },
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

/// Version `k` from version `k - 1`: one of E446's five edits, by `k`.
fn edit(net: &mut Net, k: usize) {
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
            let m = (net.transitions.len() / 2).clamp(1, 20);
            let j = pick(net.transitions.len() - m + 1);
            for t in &mut net.transitions[j..j + m] {
                let w = first_arc(t);
                *w = *w % 4 + 1;
            }
        }
    }
}

fn chain(n: usize) -> Vec<Net> {
    let mut nets = vec![synth(n)];
    for k in 2..=VERSIONS {
        let mut next = nets.last().unwrap().clone();
        edit(&mut next, k);
        next.check().unwrap();
        nets.push(next);
    }
    nets
}

/// Median of `runs` runs of `f` (each after `setup`), in µs.
fn time_with<T>(runs: usize, mut setup: impl FnMut(), mut f: impl FnMut() -> T) -> f64 {
    let mut all = Vec::new();
    for _ in 0..runs {
        setup();
        let at = Instant::now();
        std::hint::black_box(f());
        all.push(at.elapsed().as_secs_f64());
    }
    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    all[all.len() / 2] * 1e6
}

fn size(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

/// How a store of the chain is set up: the form, E449's options, and the
/// store-level options of part 3.
#[derive(Clone, Copy)]
struct Opts {
    /// None: E422's text file.
    layout: Option<Layout>,
    fast: bool,
    cache: usize,
    delta: bool,
    short_records: bool,
    short_chunk_records: bool,
    /// E448: chunk contents as frames against a related chunk.
    frames: bool,
    /// E448: chunk bounds (None: the default 4 KiB to 32 KiB).
    chunks: Option<ChunkSizes>,
}

/// E448's parent bounds "P16K": leaves as by default, parents 16 to 128 KiB.
const P16K: ChunkSizes = ChunkSizes {
    leaf: (4096, 32768),
    parent: (16384, 131072),
};

/// E448's heavy-weight bounds "S256": 256 to 2,048 B at every level.
const S256: ChunkSizes = ChunkSizes {
    leaf: (256, 2048),
    parent: (256, 2048),
};

const TEXT: Opts = Opts {
    layout: None,
    fast: false,
    cache: 0,
    delta: false,
    short_records: false,
    short_chunk_records: false,
    frames: false,
    chunks: None,
};

fn plain(layout: Layout) -> Opts {
    Opts {
        layout: Some(layout),
        ..TEXT
    }
}

fn with_e449(layout: Layout) -> Opts {
    Opts {
        layout: Some(layout),
        fast: true,
        cache: 1 << 16,
        delta: true,
        ..TEXT
    }
}

fn apply(repo: &mut Repo<MemStore>, o: Opts) {
    repo.net_fast_read = o.fast;
    repo.graph_cache = o.cache;
    repo.net_delta_write = o.delta;
    repo.short_records = o.short_records;
    store_opts(repo, o);
}

fn store_opts(repo: &mut Repo<MemStore>, o: Opts) {
    repo.short_chunk_records = o.short_chunk_records;
    repo.object_contexts = o.frames;
    if let Some(c) = o.chunks {
        repo.graph_chunks = c;
    }
}

/// One store of the chain in one form.
struct Built {
    repo: Repo<MemStore>,
    commits: Vec<Id>,
    roots: Vec<Id>,
    first: usize,
    added: f64,
    /// Mean write of versions 2 to 25, µs.
    later_us: f64,
    reuse: f64,
}

fn build(nets: &[Net], o: Opts, weight: f64) -> Built {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = weight;
    apply(&mut repo, o);
    let (mut tree, mut commits, mut roots) = (None, vec![], vec![]);
    let (mut sizes, mut later_s) = (vec![], 0.0);
    let (mut seen, mut reused, mut total) = (HashSet::new(), 0usize, 0usize);
    for net in nets {
        let before = size(&repo);
        let at = Instant::now();
        let (t, root) = match o.layout {
            None => repo.put_net_file(tree, PATH, net).unwrap(),
            Some(l) => repo.put_net_fragment_file(tree, PATH, net, l).unwrap(),
        };
        if !roots.is_empty() {
            later_s += at.elapsed().as_secs_f64();
        }
        sizes.push(size(&repo) - before);
        tree = Some(t);
        commits.push(repo.commit("main", t, "v").unwrap());
        roots.push(root);
        if o.layout.is_some() {
            let objs = objects(&repo, &root);
            if roots.len() > 1 {
                total += objs.len();
                reused += objs.iter().filter(|i| seen.contains(*i)).count();
            }
            seen.extend(objs);
        }
    }
    let later = &sizes[1..];
    Built {
        repo,
        commits,
        roots,
        first: sizes[0],
        added: later.iter().sum::<usize>() as f64 / later.len() as f64,
        later_us: later_s * 1e6 / (nets.len() - 1) as f64,
        reuse: if total == 0 {
            0.0
        } else {
            100.0 * reused as f64 / total as f64
        },
    }
}

/// A graph version's own objects: its chunks and the contents they name.
fn objects(repo: &Repo<MemStore>, root: &Id) -> Vec<Id> {
    let mut out = Vec::new();
    let mut stack = vec![*root];
    while let Some(id) = stack.pop() {
        out.push(id);
        match repo.get(&id).unwrap() {
            Object::Graph(Graph::Children(_, cs)) => stack.extend(cs.into_iter().map(|c| c.1)),
            Object::Graph(Graph::Nodes(nodes)) => {
                for n in nodes {
                    if let Target::Content(a) = n.target {
                        out.push(a.id());
                    }
                }
            }
            _ => panic!("not a graph"),
        }
    }
    out
}

/// The net forms compared: E422's text, E446, E447 alone, E449 alone,
/// and the combinations (E451).
fn net_forms() -> Vec<(&'static str, Opts)> {
    let frames = |o: Opts| Opts {
        frames: true,
        chunks: Some(P16K),
        ..o
    };
    let mut forms = vec![
        ("text (E422)", TEXT),
        ("E446 tree", plain(Layout::Tree)),
        ("E447 inline", plain(Layout::Inline)),
        ("E447 inline-flat", plain(Layout::InlineFlat)),
        ("E448 tree + frames, P16K", frames(plain(Layout::Tree))),
        (
            "E449 tree + fast read, cache, delta write",
            with_e449(Layout::Tree),
        ),
        ("E451 inline + E449 options", with_e449(Layout::Inline)),
        (
            "E451 inline-flat + E449 options",
            with_e449(Layout::InlineFlat),
        ),
        ("E451 inline + frames P16K", frames(plain(Layout::Inline))),
        (
            "E451 inline + frames P16K + E449 options",
            frames(with_e449(Layout::Inline)),
        ),
        (
            "E451 inline-flat + frames + E449 options",
            Opts {
                frames: true,
                ..with_e449(Layout::InlineFlat)
            },
        ),
        (
            "E451 inline + frames P16K + E449 + short_chunk_records",
            Opts {
                short_chunk_records: true,
                ..frames(with_e449(Layout::Inline))
            },
        ),
        (
            "E451 inline + E449 + short_chunk_records",
            Opts {
                short_chunk_records: true,
                ..with_e449(Layout::Inline)
            },
        ),
        (
            "E451 inline + E449 + short_records",
            Opts {
                short_records: true,
                ..with_e449(Layout::Inline)
            },
        ),
        (
            "E451 inline + frames L16K + E449 + short_chunk_records",
            Opts {
                short_chunk_records: true,
                chunks: Some(ChunkSizes::uniform(16384, 131072)),
                ..frames(with_e449(Layout::Inline))
            },
        ),
        (
            "E451 inline + S256 + E449 options",
            Opts {
                chunks: Some(S256),
                ..with_e449(Layout::Inline)
            },
        ),
    ];
    if let Ok(only) = std::env::var("E451_FORMS") {
        forms.retain(|(n, _)| only.split(',').any(|o| n.contains(o)));
    }
    forms
}

fn read_v(repo: &Repo<MemStore>, o: Opts, c: &Id) -> Net {
    match o.layout {
        None => repo.net_file(c, PATH).unwrap(),
        Some(_) => repo.net_fragment_file(c, PATH).unwrap(),
    }
}

fn nets_mode(sizes: &[usize], runs: usize) {
    println!("## nets: E446's 25-version chain (times: median of {runs} runs)\n");
    for &n in sizes {
        let nets = chain(n);
        let last = nets.len() - 1;
        for weight in [ikam_kernel::DEFAULT_READ_WEIGHT, HEAVY_READ_WEIGHT] {
            println!("### {n} transitions, read weight {weight}\n");
            println!(
                "| form | bytes v1 | x text | a version adds | x text | reused | write µs (v2-25) | read v25 cold µs | of which chunks µs | read v25 after v24 µs | delta µs (mean of 24) | fsck |"
            );
            println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
            let (mut text_v1, mut text_add) = (0, 0.0);
            for (name, o) in net_forms() {
                let b = build(&nets, o, weight);
                if o.layout.is_none() {
                    (text_v1, text_add) = (b.first, b.added);
                }
                let repo = &b.repo;
                for (k, net) in nets.iter().enumerate() {
                    assert_eq!(&read_v(repo, o, &b.commits[k]), net, "{name} v{}", k + 1);
                }
                let (c24, c25) = (b.commits[last - 1], b.commits[last]);
                let cold = time_with(runs, || repo.graph_cache_clear(), || read_v(repo, o, &c25));
                let chunks = match o.layout {
                    Some(_) => format!(
                        "{:.0}",
                        time_with(runs, || {}, || repo.graph_nodes(&b.roots[last]).unwrap())
                    ),
                    None => "–".into(),
                };
                let warm = match o.cache > 0 {
                    true => format!(
                        "{:.0}",
                        time_with(
                            runs,
                            || {
                                repo.graph_cache_clear();
                                read_v(repo, o, &c24);
                            },
                            || read_v(repo, o, &c25),
                        )
                    ),
                    false => "–".into(),
                };
                let mut delta_us = 0.0;
                for k in 1..nets.len() {
                    delta_us += match o.layout {
                        None => time_with(
                            runs.min(3),
                            || {},
                            || {
                                let x = repo.read_content(&b.roots[k - 1]).unwrap();
                                let y = repo.read_content(&b.roots[k]).unwrap();
                                similar::TextDiff::from_lines(
                                    std::str::from_utf8(&x).unwrap(),
                                    std::str::from_utf8(&y).unwrap(),
                                )
                                .iter_all_changes()
                                .filter(|c| c.tag() != similar::ChangeTag::Equal)
                                .count()
                            },
                        ),
                        Some(_) => {
                            let d = repo.net_delta(&b.roots[k - 1], &b.roots[k]).unwrap();
                            assert_eq!(d.apply(&nets[k - 1]).unwrap(), nets[k]);
                            time_with(
                                runs.min(3),
                                || {},
                                || repo.net_delta(&b.roots[k - 1], &b.roots[k]).unwrap(),
                            )
                        }
                    };
                }
                let fsck = repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
                println!(
                    "| {name} | {} | {:.2} | {:.0} | {:.2} | {:.1}% | {:.0} | {:.0} | {chunks} | {warm} | {:.0} | {fsck} |",
                    b.first,
                    b.first as f64 / text_v1 as f64,
                    b.added,
                    b.added / text_add,
                    b.reuse,
                    b.later_us,
                    cold,
                    delta_us / (nets.len() - 1) as f64,
                );
            }
            println!();
        }
    }
}

const NIGHT: &str = "\
net night 1
place cpu 4
place queued colored
place running colored
place measured colored
transition propose agent: -> queued
transition start cmd: queued, cpu -> running
transition finish pure: running -> measured, cpu
";

/// E422's night net at version `k`, inside `big` synthetic transitions.
fn night(k: u32, big: usize) -> Net {
    let mut text = NIGHT.replace("net night 1", &format!("net night {k}"));
    if k > 1 {
        text += "place checked colored\n";
        text += &format!("transition check{k} pure: measured -> checked\n");
    }
    let mut net = Net::parse(&text).unwrap();
    if big > 0 {
        let s = synth(big);
        let mut places = s.places;
        places.extend(net.places);
        net.places = places;
        net.transitions.extend(s.transitions);
    }
    net
}

/// E422's run (the net edited 9 times mid-run) tracked in form `o`,
/// fired whole or by key (`at`): µs a firing, the marking, whether replay
/// gives it, replay's µs.
fn firings(candidates: usize, big: usize, o: Opts, at: bool) -> (f64, String, bool, f64) {
    let mut repo = Repo::new(MemStore::default());
    apply(&mut repo, o);
    let mut cur = night(1, big);
    let mut db = repo.db_empty().unwrap();
    db = repo.net_init(&db, &cur).unwrap();
    db = match o.layout {
        None => repo.net_track_versions(&db, &cur).unwrap(),
        Some(l) => repo.net_track_fragments(&db, &cur, l).unwrap(),
    };
    let mut root = o.layout.map(|l| repo.put_net_fragments(&cur, l).unwrap());
    let every = (candidates / 10).max(1);
    let (mut fire_s, mut count) = (0.0, 0);
    for i in 0..candidates {
        if i > 0 && i % every == 0 {
            cur = night(cur.version + 1, big);
            if at {
                root = Some(repo.put_net_fragments(&cur, o.layout.unwrap()).unwrap());
            }
        }
        let c = format!("c{i}");
        let mut steps = vec![("propose", "p"), ("start", "s"), ("finish", "f")];
        let check = format!("check{}", cur.version);
        if cur.version > 1 {
            steps.push((check.as_str(), "k"));
        }
        for (t, k) in steps {
            let key = format!("{k}{i}");
            let t0 = Instant::now();
            db = match at {
                true => repo.net_fire_at(&db, root.as_ref().unwrap(), t, Some(&c), &key),
                false => repo.net_fire(&db, &cur, t, Some(&c), &key),
            }
            .unwrap()
            .0;
            fire_s += t0.elapsed().as_secs_f64();
            count += 1;
        }
    }
    let m = repo.net_marking(&db, "night").unwrap();
    let t0 = Instant::now();
    let ok = repo.net_replay_versions(&db, "night").unwrap() == m;
    let replay = t0.elapsed().as_secs_f64() * 1e6;
    (fire_s * 1e6 / count as f64, format!("{m:?}"), ok, replay)
}

fn fire_mode(candidates: usize) {
    println!("## firings: E422's night net edited 9 times mid-run (median of 3 runs)\n");
    for big in [0, 10000] {
        let candidates = if big > 0 {
            (candidates / 3).max(10)
        } else {
            candidates
        };
        println!(
            "### {}, {candidates} candidates\n",
            match big {
                0 => "the night net alone".to_string(),
                b => format!("the night net inside {b} more transitions"),
            }
        );
        println!("| tracked by | fire µs each | replay µs | replay |");
        println!("| --- | --- | --- | --- |");
        let mut markings = Vec::new();
        for (name, o, at) in [
            ("text (E422)", TEXT, false),
            ("E446 tree", plain(Layout::Tree), false),
            ("E447 inline", plain(Layout::Inline), false),
            ("E449 tree + options", with_e449(Layout::Tree), false),
            (
                "E449 tree, net_fire_at by key",
                with_e449(Layout::Tree),
                true,
            ),
            (
                "E451 inline + E449 options",
                with_e449(Layout::Inline),
                false,
            ),
            (
                "E451 inline, net_fire_at by key",
                with_e449(Layout::Inline),
                true,
            ),
            (
                "E451 inline-flat + E449 options",
                with_e449(Layout::InlineFlat),
                false,
            ),
        ] {
            let mut us = vec![];
            let mut last = None;
            for _ in 0..3 {
                let r = firings(candidates, big, o, at);
                us.push((r.0, r.3));
                last = Some(r);
            }
            let mut f: Vec<f64> = us.iter().map(|u| u.0).collect();
            f.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mut rp: Vec<f64> = us.iter().map(|u| u.1).collect();
            rp.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let (_, m, ok, _) = last.unwrap();
            println!(
                "| {name} | {:.0} | {:.0} | {} |",
                f[1],
                rp[1],
                if ok { "equal marking" } else { "FAILS" }
            );
            markings.push(m);
        }
        println!(
            "\n- final markings equal across all rows: {}\n",
            markings.windows(2).all(|w| w[0] == w[1])
        );
    }
}

/// A deterministic pseudo-random sequence (splitmix64).
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
}

const WORDS: &[&str] = &[
    "river", "stone", "garden", "signal", "harbor", "lantern", "meadow", "orbit", "paper",
    "quartz", "ridge", "summit", "timber", "valley", "willow", "amber", "beacon", "canyon",
    "delta", "ember", "forest", "glacier", "island", "jungle", "kernel", "ledger", "marble",
];

fn words(rng: &mut Rng, n: usize) -> String {
    (0..n)
        .map(|_| WORDS[rng.below(WORDS.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

/// A general graph in memory: label to (payload, edge targets).
type G = BTreeMap<String, (String, BTreeSet<String>)>;

fn label(i: usize) -> String {
    format!("n{i:06}")
}

/// A sparse power-law link graph: `n` pages, about `deg` links each to
/// targets drawn with a heavy tail, a short title each.
fn link_graph(n: usize, deg: usize) -> G {
    let mut rng = Rng(7);
    let mut g = G::new();
    for i in 0..n {
        let k = 1 + rng.below(2 * deg - 1);
        let mut to = BTreeSet::new();
        for _ in 0..k {
            // u^3 puts most links on the first pages.
            let u = (rng.next() >> 11) as f64 / (1u64 << 53) as f64;
            let j = ((u * u * u) * n as f64) as usize % n;
            if j != i {
                to.insert(label(j));
            }
        }
        let w = 3 + rng.below(4);
        let title = format!("Page {i}: {}", words(&mut rng, w));
        g.insert(label(i), (title, to));
    }
    g
}

/// A property graph: `n` records of 1 to 4 KB, a few edges each.
fn property_graph(n: usize) -> G {
    let mut rng = Rng(11);
    let mut g = G::new();
    for i in 0..n {
        let len = 1024 + rng.below(3072);
        let mut body = format!("{{\"id\": {i}, \"kind\": \"{}\"", WORDS[i % WORDS.len()]);
        let mut f = 0;
        while body.len() < len {
            body += &format!(", \"f{f}\": \"{}\"", words(&mut rng, 4));
            f += 1;
        }
        body += "}";
        let to = (0..1 + rng.below(4)).map(|_| label(rng.below(n))).collect();
        g.insert(label(i), (body, to));
    }
    g
}

/// Version `k` of a graph from `k - 1`: local edits (links added and
/// removed, payloads edited, and every third version a small subgraph).
/// Returns the labels it touched.
fn graph_edit_step(g: &mut G, k: usize, rng: &mut Rng) -> BTreeSet<String> {
    let n = g.len();
    let keys: Vec<String> = g.keys().cloned().collect();
    let mut touched = BTreeSet::new();
    for _ in 0..10 {
        let a = &keys[rng.below(n)];
        let b = keys[rng.below(n)].clone();
        let e = g.get_mut(a).unwrap();
        if !e.1.remove(&b) {
            e.1.insert(b);
        }
        touched.insert(a.clone());
    }
    for _ in 0..5 {
        let a = &keys[rng.below(n)];
        let e = g.get_mut(a).unwrap();
        e.0 = format!("{} (edit {k})", e.0);
        touched.insert(a.clone());
    }
    if k.is_multiple_of(3) {
        let base = keys[rng.below(n)].clone();
        for j in 0..5 {
            let l = format!("{base}-v{k}-{j}");
            let to = [base.clone(), format!("{base}-v{k}-{}", (j + 1) % 5)].into();
            g.insert(l.clone(), (format!("added in version {k}, part {j}"), to));
            touched.insert(l);
        }
        g.get_mut(&base).unwrap().1.insert(format!("{base}-v{k}-0"));
        touched.insert(base);
    }
    touched
}

/// A node of `g`: its payload a content (`inline` None) or put by
/// `put_payload` with that limit.
fn node_of(
    repo: &mut Repo<MemStore>,
    l: &str,
    payload: &str,
    to: &BTreeSet<String>,
    inline: Option<usize>,
) -> Node {
    let target = match inline {
        None => Target::Content(Arg::Whole(repo.put_content(payload.as_bytes()).unwrap().id)),
        Some(max) => repo.put_payload(payload.as_bytes(), max).unwrap(),
    };
    Node {
        label: l.to_string(),
        target,
        edges: to
            .iter()
            .map(|t| Edge {
                to: node_key(t),
                label: format!("link {t}"),
            })
            .collect(),
    }
}

struct GraphBuilt {
    repo: Repo<MemStore>,
    roots: Vec<Id>,
    first: usize,
    added: f64,
    write_us: f64,
    reuse: f64,
}

/// The chain: version 1 by `put_graph`, each later one by `graph_edit`
/// of the nodes it touched.
fn graph_build(g0: &G, o: Opts, inline: Option<usize>, weight: f64) -> GraphBuilt {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = weight;
    apply(&mut repo, o);
    let (mut roots, mut sizes, mut write_s) = (vec![], vec![], 0.0);
    let (mut seen, mut reused, mut total) = (HashSet::new(), 0usize, 0usize);
    let mut rng = Rng(99);
    let mut g = g0.clone();
    for k in 0..VERSIONS {
        let touched = match k {
            0 => BTreeSet::new(),
            _ => graph_edit_step(&mut g, k + 1, &mut rng),
        };
        let before = size(&repo);
        let at = Instant::now();
        let root = if k == 0 {
            let mut nodes = Vec::with_capacity(g.len());
            for (l, (p, to)) in &g {
                nodes.push(node_of(&mut repo, l, p, to, inline));
            }
            repo.put_graph(nodes).unwrap()
        } else {
            let mut e = BTreeMap::new();
            for l in &touched {
                let n = g.get(l).map(|(p, to)| node_of(&mut repo, l, p, to, inline));
                e.insert(l.clone(), n);
            }
            repo.graph_edit(roots.last().unwrap(), e).unwrap()
        };
        if k > 0 {
            write_s += at.elapsed().as_secs_f64();
        }
        sizes.push(size(&repo) - before);
        let objs = objects(&repo, &root);
        if k > 0 {
            total += objs.len();
            reused += objs.iter().filter(|i| seen.contains(*i)).count();
        }
        seen.extend(objs);
        roots.push(root);
    }
    let later = &sizes[1..];
    GraphBuilt {
        repo,
        roots,
        first: sizes[0],
        added: later.iter().sum::<usize>() as f64 / later.len() as f64,
        write_us: write_s * 1e6 / (VERSIONS - 1) as f64,
        reuse: 100.0 * reused as f64 / total as f64,
    }
}

/// Every payload of a version: plain (`graph_nodes` and `read_content`
/// each) or by `graph_read`.
fn read_whole(repo: &Repo<MemStore>, root: &Id, fast: bool) -> usize {
    if fast {
        return repo
            .graph_read(root)
            .unwrap()
            .iter()
            .flat_map(|l| l.nodes.iter())
            .map(|(_, p)| p.as_ref().map_or(0, |p| p.len()))
            .sum();
    }
    let mut total = 0;
    for n in repo.graph_nodes(root).unwrap() {
        total += match n.target {
            Target::Content(Arg::Whole(id)) => repo.read_content(&id).unwrap().len(),
            Target::Inline(b) => b.len(),
            _ => 0,
        };
    }
    total
}

fn graphs_mode(scale: f64, runs: usize) {
    println!("## general graphs: 25 versions of local edits (times: median of {runs} runs)\n");
    let cases = [
        (
            "link graph",
            link_graph((100_000.0 * scale) as usize, 10),
            256,
        ),
        (
            "property graph",
            property_graph((10_000.0 * scale) as usize),
            2048,
        ),
    ];
    let cached = Opts {
        cache: 1 << 16,
        ..TEXT
    };
    for (name, g0, limit) in cases {
        let edges: usize = g0.values().map(|v| v.1.len()).sum();
        let bytes: usize = g0.values().map(|v| v.0.len()).sum();
        println!(
            "### {name}: {} nodes, {edges} edges, {bytes} payload bytes\n",
            g0.len()
        );
        println!(
            "| form | bytes v1 | a version adds | heavy: adds | reused | write µs (v2-25) | read v25 cold µs | read v25 after v24 µs | delta µs | fsck |"
        );
        println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
        let forms: Vec<(String, Opts, Option<usize>)> = vec![
            (
                "payloads as contents (E446), plain reads".into(),
                TEXT,
                None,
            ),
            (
                format!("inline up to {limit} B (E447), plain reads"),
                TEXT,
                Some(limit),
            ),
            (
                "payloads as contents, E449 graph_read + cache".into(),
                cached,
                None,
            ),
            (
                format!("E451: inline up to {limit} B + E449 graph_read + cache"),
                cached,
                Some(limit),
            ),
            (
                "payloads as contents + frames P16K (E448)".into(),
                Opts {
                    frames: true,
                    chunks: Some(P16K),
                    ..TEXT
                },
                None,
            ),
            (
                format!("E451: inline up to {limit} B + frames P16K + E449 graph_read + cache"),
                Opts {
                    frames: true,
                    chunks: Some(P16K),
                    ..cached
                },
                Some(limit),
            ),
            (
                "E451 + short_records".into(),
                Opts {
                    short_records: true,
                    ..cached
                },
                Some(limit),
            ),
            (
                "E451 + short_chunk_records".into(),
                Opts {
                    short_chunk_records: true,
                    ..cached
                },
                Some(limit),
            ),
        ];
        for (fname, o, inline) in forms {
            let heavy = graph_build(&g0, o, inline, HEAVY_READ_WEIGHT).added;
            let b = graph_build(&g0, o, inline, ikam_kernel::DEFAULT_READ_WEIGHT);
            let repo = &b.repo;
            let (r24, r25) = (b.roots[VERSIONS - 2], b.roots[VERSIONS - 1]);
            let fast = o.cache > 0;
            assert_eq!(read_whole(repo, &r25, fast), read_whole(repo, &r25, false));
            let cold = time_with(
                runs,
                || repo.graph_cache_clear(),
                || read_whole(repo, &r25, fast),
            );
            let warm = match fast {
                true => format!(
                    "{:.0}",
                    time_with(
                        runs,
                        || {
                            repo.graph_cache_clear();
                            read_whole(repo, &r24, true);
                        },
                        || read_whole(repo, &r25, true),
                    )
                ),
                false => "–".into(),
            };
            let delta = time_with(runs, || {}, || repo.graph_diff(&r24, &r25).unwrap().len());
            let fsck = repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
            println!(
                "| {fname} | {} | {:.0} | {heavy:.0} | {:.1}% | {:.0} | {cold:.0} | {warm} | {delta:.0} | {fsck} |",
                b.first, b.added, b.reuse, b.write_us
            );
        }
        println!();
    }
}

/// Cold reads of version 25 by thread count (`read_threads`,
/// `net_fragments_par`), interleaved with the text's read round by round
/// so load falls on each alike.
fn par_mode(sizes: &[usize], runs: usize) {
    println!("## cold read of version 25 by threads (median of {runs} interleaved rounds)\n");
    println!("| transitions | form | text µs | 1 thread | 2 threads | 3 threads |");
    println!("| --- | --- | --- | --- | --- | --- |");
    for &n in sizes {
        let nets = chain(n);
        let last = nets.len() - 1;
        let text = build(&nets, TEXT, ikam_kernel::DEFAULT_READ_WEIGHT);
        for (name, o) in [
            ("E449 tree + options", with_e449(Layout::Tree)),
            ("E451 inline + E449 options", with_e449(Layout::Inline)),
            (
                "E451 inline + frames P16K + E449 options",
                Opts {
                    frames: true,
                    chunks: Some(P16K),
                    ..with_e449(Layout::Inline)
                },
            ),
        ] {
            let mut b = build(&nets, o, ikam_kernel::DEFAULT_READ_WEIGHT);
            let r25 = b.roots[last];
            let mut all = vec![vec![]; 4];
            for _ in 0..runs {
                let t0 = Instant::now();
                std::hint::black_box(read_v(&text.repo, TEXT, &text.commits[last]));
                all[0].push(t0.elapsed().as_secs_f64());
                for (th, times) in all.iter_mut().enumerate().skip(1) {
                    b.repo.read_threads = th;
                    b.repo.graph_cache_clear();
                    let t0 = Instant::now();
                    let net = b.repo.net_fragments_par(&r25).unwrap();
                    times.push(t0.elapsed().as_secs_f64());
                    assert_eq!(net, nets[last]);
                }
            }
            let med = |v: &mut Vec<f64>| {
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                v[v.len() / 2] * 1e6
            };
            let m: Vec<f64> = all.iter_mut().map(med).collect();
            println!(
                "| {n} | {name} | {:.0} | {:.0} | {:.0} | {:.0} |",
                m[0], m[1], m[2], m[3]
            );
        }
    }
    println!();
}

/// What each version adds to the store, by kind of entry, for one form:
/// chunk pointer objects (`g 2 content`, 34 B), derivation records,
/// contents (chunk bodies and anything else stored as bytes), and trees
/// and commits; with the leaf and parent chunks each version rewrites.
fn breakdown_mode(sizes: &[usize]) {
    println!("## what a version adds, by entry (mean of versions 2-25, default weight)\n");
    println!(
        "| transitions | form | adds | chunk pointers | derivation records (count) | contents | trees, commits, other | leaves / parents rewritten |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for &n in sizes {
        let nets = chain(n);
        for (name, o) in net_forms() {
            let mut repo = Repo::new(MemStore::default());
            apply(&mut repo, o);
            let mut tree = None;
            let mut seen: HashSet<Id> = HashSet::new();
            let mut sum = [0usize; 6];
            let mut records = 0;
            let mut chunks = (0, 0);
            for (k, net) in nets.iter().enumerate() {
                let before: HashSet<Id> = repo.store().ids().unwrap().into_iter().collect();
                let (t, root) = match o.layout {
                    None => repo.put_net_file(tree, PATH, net).unwrap(),
                    Some(l) => repo.put_net_fragment_file(tree, PATH, net, l).unwrap(),
                };
                tree = Some(t);
                repo.commit("main", t, "v").unwrap();
                if k == 0 {
                    if o.layout.is_some() {
                        seen.extend(objects(&repo, &root));
                    }
                    continue;
                }
                for id in repo.store().ids().unwrap() {
                    if before.contains(&id) {
                        continue;
                    }
                    let e = repo.store().read(&id).unwrap();
                    let class = match e.as_slice() {
                        [b'g', 2, ..] => 1,
                        [b't', 2, ..] => 4,
                        [b'd' | b'D', ..] => {
                            records += 1;
                            2
                        }
                        [b'c' | b'C' | b't' | b'T', ..] => 4,
                        _ => 3,
                    };
                    sum[0] += e.len();
                    sum[class] += e.len();
                }
                if o.layout.is_some() {
                    let mut stack = vec![root];
                    while let Some(id) = stack.pop() {
                        if seen.contains(&id) {
                            continue;
                        }
                        seen.insert(id);
                        match repo.get(&id).unwrap() {
                            Object::Graph(Graph::Children(_, cs)) => {
                                chunks.1 += 1;
                                stack.extend(cs.into_iter().map(|c| c.1));
                            }
                            _ => chunks.0 += 1,
                        }
                    }
                }
            }
            let v = (nets.len() - 1) as f64;
            println!(
                "| {n} | {name} | {:.0} | {:.0} | {:.0} ({:.1}) | {:.0} | {:.0} | {:.1} / {:.1} |",
                sum[0] as f64 / v,
                sum[1] as f64 / v,
                sum[2] as f64 / v,
                records as f64 / v,
                sum[3] as f64 / v,
                sum[4] as f64 / v,
                chunks.0 as f64 / v,
                chunks.1 as f64 / v,
            );
        }
    }
    println!();
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "nets".into());
    let arg = args.next();
    let runs: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(7);
    match mode.as_str() {
        "nets" => {
            let sizes: Vec<usize> = arg
                .map(|s| s.split(',').map(|x| x.parse().unwrap()).collect())
                .unwrap_or_else(|| vec![1000, 10000]);
            nets_mode(&sizes, runs)
        }
        "breakdown" => breakdown_mode(
            &arg.map(|s| {
                s.split(',')
                    .map(|x| x.parse().unwrap())
                    .collect::<Vec<usize>>()
            })
            .unwrap_or_else(|| vec![1000, 10000]),
        ),
        "par" => par_mode(
            &arg.map(|s| {
                s.split(',')
                    .map(|x| x.parse().unwrap())
                    .collect::<Vec<usize>>()
            })
            .unwrap_or_else(|| vec![1000, 10000]),
            runs,
        ),
        "fire" => fire_mode(arg.map_or(300, |a| a.parse().unwrap())),
        "graphs" => graphs_mode(arg.map_or(1.0, |a| a.parse().unwrap()), runs.min(5)),
        m => panic!("unknown mode {m}"),
    }
}
