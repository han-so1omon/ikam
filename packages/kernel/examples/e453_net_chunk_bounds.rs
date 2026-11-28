//! E453: chunk bounds for fragment nets picked from the net's size and
//! the read weight (`Repo::net_chunk_policy`), against the default bounds
//! and fixed bounds, on E446's 25-version chain at several sizes.
//!
//!   cargo run --release --example e453_net_chunk_bounds -- nets [SIZES] [RUNS]
//!   cargo run --release --example e453_net_chunk_bounds -- sweep [SIZES] [RUNS]
//!   cargo run --release --example e453_net_chunk_bounds -- fire [CANDIDATES]
//!
//! SIZES is a comma-separated list (default 300,1000,3000,10000). Times are
//! medians of RUNS runs (default 5); run `nets` three times and take the
//! median of the three (other builds share the machine). `E453_FORMS`
//! keeps the rows whose name contains one of its comma-separated parts.
//! The nets and the chain are E446's, copied from E451's harness so the
//! examples build apart.

use std::collections::HashSet;
use std::time::Instant;

use ikam_kernel::net::{Arc, Kind, Layout, Net, NetChunkPolicy, Place, Transition};
use ikam_kernel::{ChunkSizes, Graph, Id, MemStore, Object, Repo, Store, Target};

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

/// How a store of the chain is set up.
#[derive(Clone)]
struct Opts {
    /// None: E422's text file.
    layout: Option<Layout>,
    /// E449's fast read, cache and delta write.
    e449: bool,
    /// E448's frames and E451's short chunk records.
    frames: bool,
    /// Fixed bounds (None: the default 4 KiB to 32 KiB).
    chunks: Option<ChunkSizes>,
    /// E453's policy.
    policy: Option<NetChunkPolicy>,
}

const TEXT: Opts = Opts {
    layout: None,
    e449: false,
    frames: false,
    chunks: None,
    policy: None,
};

const fn kib(min: usize, max: usize) -> (usize, usize) {
    (min * 1024, max * 1024)
}

/// The default bounds.
const D: ChunkSizes = ChunkSizes {
    leaf: kib(4, 32),
    parent: kib(4, 32),
};
/// E448's "P16K".
const P16K: ChunkSizes = ChunkSizes {
    leaf: kib(4, 32),
    parent: kib(16, 128),
};
/// E451's "L16K".
const L16K: ChunkSizes = ChunkSizes {
    leaf: kib(16, 128),
    parent: kib(16, 128),
};
/// E448's "S256".
const S256: ChunkSizes = ChunkSizes {
    leaf: (256, 2048),
    parent: (256, 2048),
};

fn bounds(leaf: (usize, usize), parent: (usize, usize)) -> ChunkSizes {
    ChunkSizes { leaf, parent }
}

fn apply(repo: &mut Repo<MemStore>, o: &Opts) {
    repo.net_fast_read = o.e449;
    repo.graph_cache = if o.e449 { 1 << 16 } else { 0 };
    repo.net_delta_write = o.e449;
    repo.object_contexts = o.frames;
    repo.short_chunk_records = o.frames;
    if let Some(c) = o.chunks {
        repo.graph_chunks = c;
    }
    repo.net_chunk_policy = o.policy.clone();
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
    /// Leaves and parents of version 1; leaves and parents new a version.
    v1_chunks: (usize, usize),
    new_chunks: (f64, f64),
}

/// Leaves and parents under `root` not in `seen`, added to it.
fn count_new(repo: &Repo<MemStore>, root: &Id, seen: &mut HashSet<Id>) -> (usize, usize) {
    let (mut stack, mut out) = (vec![*root], (0, 0));
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        match repo.get(&id).unwrap() {
            Object::Graph(Graph::Children(_, cs)) => {
                out.1 += 1;
                stack.extend(cs.into_iter().map(|c| c.1));
            }
            Object::Graph(Graph::Nodes(nodes)) => {
                out.0 += 1;
                for n in nodes {
                    if let Target::Content(a) = n.target {
                        seen.insert(a.id());
                    }
                }
            }
            _ => panic!("not a graph"),
        }
    }
    out
}

fn build(nets: &[Net], o: &Opts, weight: f64) -> Built {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = weight;
    apply(&mut repo, o);
    let (mut tree, mut commits, mut roots) = (None, vec![], vec![]);
    let (mut sizes, mut later_s) = (vec![], 0.0);
    let mut seen = HashSet::new();
    let (mut v1_chunks, mut new) = ((0, 0), (0, 0));
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
        if o.layout.is_some() {
            let c = count_new(&repo, &root, &mut seen);
            match roots.is_empty() {
                true => v1_chunks = c,
                false => new = (new.0 + c.0, new.1 + c.1),
            }
        }
        roots.push(root);
    }
    let later = &sizes[1..];
    let v = (nets.len() - 1) as f64;
    Built {
        repo,
        commits,
        roots,
        first: sizes[0],
        added: later.iter().sum::<usize>() as f64 / later.len() as f64,
        later_us: later_s * 1e6 / v,
        v1_chunks,
        new_chunks: (new.0 as f64 / v, new.1 as f64 / v),
    }
}

fn read_v(repo: &Repo<MemStore>, o: &Opts, c: &Id) -> Net {
    match o.layout {
        None => repo.net_file(c, PATH).unwrap(),
        Some(_) => repo.net_fragment_file(c, PATH).unwrap(),
    }
}

/// The policies compared (E453). Sizes are the encoded bytes of the net's
/// inline nodes (`NetChunkPolicy::net_bytes`): about 12.8 KiB at 300
/// transitions, 45 KiB at 1,000, 146 KiB at 3,000 and 511 KiB at 10,000.
/// Parents are 16 to 128 KiB wherever the leaves are not the default's.
fn policies() -> Vec<(&'static str, NetChunkPolicy)> {
    let parents = kib(16, 128);
    let leaf = |min: usize| bounds((min, min * 8), parents);
    let never = f64::INFINITY;
    // A: one step. Nets of up to 64 KiB of nodes get 8 to 64 KiB leaves;
    // larger nets the default.
    let a = NetChunkPolicy {
        steps: vec![(64 << 10, leaf(8 << 10))],
        heavy_from: never,
        heavy: vec![],
    };
    // B: three steps. Up to 24 KiB one chunk (16 to 128 KiB leaves), up to
    // 192 KiB 6 to 48 KiB leaves, up to 384 KiB P16K, then the default.
    let b = NetChunkPolicy {
        steps: vec![
            (24 << 10, leaf(16 << 10)),
            (192 << 10, leaf(6 << 10)),
            (384 << 10, P16K),
        ],
        heavy_from: never,
        heavy: vec![],
    };
    // C (`NetChunkPolicy::by_size`): leaves scaled to the net, about six
    // leaves: the least leaf is a sixth of the net's bytes, at least 4 KiB
    // and at most 8 KiB, in 1 KiB steps, for nets of up to 1 MiB (so
    // 10,000 transitions get 8 to 64 KiB leaves); larger nets the default.
    let c = NetChunkPolicy::by_size();
    assert_eq!(c.pick(45 << 10, 0.001, D), leaf(7 << 10));
    // W: B at the default weight; at the heavy weight (from 10), 2 to
    // 16 KiB leaves up to 192 KiB (writes and a version's bytes before
    // v1), then the default.
    let w = NetChunkPolicy {
        heavy_from: 10.0,
        heavy: vec![(192 << 10, leaf(2 << 10))],
        ..b.clone()
    };
    vec![("A", a), ("B", b), ("C", c), ("W", w)]
}

/// The rows: the text, the default and fixed bounds, and the policies,
/// each over the two E451 bases (inline + E449; and with frames and SCR).
fn net_forms() -> Vec<(String, Opts)> {
    let base = |frames: bool| Opts {
        layout: Some(Layout::Inline),
        e449: true,
        frames,
        ..TEXT
    };
    let mut forms = vec![("text (E422)".to_string(), TEXT)];
    for (tag, frames) in [("", false), (" + frames + SCR", true)] {
        let fixed: Vec<(&str, ChunkSizes)> = vec![
            ("default", D),
            ("P16K", P16K),
            ("L8K", bounds(kib(8, 64), kib(16, 128))),
            ("L16K", L16K),
            ("S256", S256),
        ];
        for (name, c) in fixed {
            forms.push((
                format!("fixed {name}{tag}"),
                Opts {
                    chunks: Some(c),
                    ..base(frames)
                },
            ));
        }
        for (name, p) in policies() {
            forms.push((
                format!("policy {name}{tag}"),
                Opts {
                    policy: Some(p),
                    ..base(frames)
                },
            ));
        }
    }
    if let Ok(only) = std::env::var("E453_FORMS") {
        forms.retain(|(n, _)| n.starts_with("text") || only.split(',').any(|o| n.contains(o)));
    }
    forms
}

fn show(c: Option<ChunkSizes>) -> String {
    let k = |(a, b): (usize, usize)| match a >= 1024 && b >= 1024 {
        true => format!("{}K:{}K", a / 1024, b / 1024),
        false => format!("{a}:{b}"),
    };
    match c {
        Some(c) if c.leaf == c.parent => k(c.leaf),
        Some(c) => format!("{}/{}", k(c.leaf), k(c.parent)),
        None => "–".into(),
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Every row's store is built once (bytes, chunks, a first write time),
/// then built again `WRITES - 1` more times for the write time and read
/// `runs` times cold, the rows interleaved in each round so load falls on
/// every row alike; medians. The cold read is also timed E451's way, each
/// row's `runs` reads in a row ("in a row"): the processor's caches then
/// hold the row's store, which the interleaved read finds cold too.
const WRITES: usize = 3;

fn nets_mode(sizes: &[usize], runs: usize) {
    println!(
        "## nets: E446's 25-version chain (write: median of {WRITES} builds; cold read: median of {runs} rounds; rows interleaved)\n"
    );
    let forms = net_forms();
    for &n in sizes {
        let nets = chain(n);
        let last = nets.len() - 1;
        for weight in [ikam_kernel::DEFAULT_READ_WEIGHT, HEAVY_READ_WEIGHT] {
            println!("### {n} transitions, read weight {weight}\n");
            println!(
                "| form | bounds | bytes v1 | x text | a version adds | x text | write µs (v2-25) | x text | read v25 cold µs | x text | cold, in a row µs | x text | delta µs | leaves / parents v1 | new a version | fsck |"
            );
            println!(
                "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"
            );
            let built: Vec<Built> = forms.iter().map(|(_, o)| build(&nets, o, weight)).collect();
            let mut writes: Vec<Vec<f64>> = built.iter().map(|b| vec![b.later_us]).collect();
            for _ in 1..WRITES {
                for (i, (_, o)) in forms.iter().enumerate() {
                    writes[i].push(build(&nets, o, weight).later_us);
                }
            }
            let mut colds: Vec<Vec<f64>> = vec![Vec::new(); forms.len()];
            for _ in 0..runs {
                for (i, (_, o)) in forms.iter().enumerate() {
                    let b = &built[i];
                    b.repo.graph_cache_clear();
                    let at = Instant::now();
                    std::hint::black_box(read_v(&b.repo, o, &b.commits[last]));
                    colds[i].push(at.elapsed().as_secs_f64() * 1e6);
                }
            }
            // E451's way: each row read `runs` times in a row, each read
            // after emptying the graph cache.
            let in_a_row: Vec<f64> = (forms.iter().zip(&built))
                .map(|((_, o), b)| {
                    time_with(
                        runs,
                        || b.repo.graph_cache_clear(),
                        || read_v(&b.repo, o, &b.commits[last]),
                    )
                })
                .collect();
            let mut text = (0usize, 0.0, 0.0, 0.0, 0.0);
            for (i, (name, o)) in forms.iter().enumerate() {
                let b = &built[i];
                let repo = &b.repo;
                for (k, net) in nets.iter().enumerate() {
                    assert_eq!(&read_v(repo, o, &b.commits[k]), net, "{name} v{}", k + 1);
                }
                let kept = match o.layout {
                    Some(_) => repo
                        .net_chunk_bounds(&nets[0].name)
                        .or(o.chunks)
                        .or(Some(D)),
                    None => None,
                };
                let (write, cold) = (median(writes[i].clone()), median(colds[i].clone()));
                let mut delta_us = 0.0;
                if o.layout.is_some() {
                    for k in 1..nets.len() {
                        let d = repo.net_delta(&b.roots[k - 1], &b.roots[k]).unwrap();
                        assert_eq!(d.apply(&nets[k - 1]).unwrap(), nets[k]);
                        delta_us += time_with(
                            runs.min(3),
                            || {},
                            || repo.net_delta(&b.roots[k - 1], &b.roots[k]).unwrap(),
                        );
                    }
                }
                if o.layout.is_none() {
                    text = (b.first, b.added, write, cold, in_a_row[i]);
                }
                let fsck = repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
                println!(
                    "| {name} | {} | {} | {:.2} | {:.0} | {:.2} | {:.0} | {:.2} | {:.0} | {:.2} | {:.0} | {:.2} | {:.0} | {} / {} | {:.1} / {:.1} | {fsck} |",
                    show(kept),
                    b.first,
                    b.first as f64 / text.0 as f64,
                    b.added,
                    b.added / text.1,
                    write,
                    write / text.2,
                    cold,
                    cold / text.3,
                    in_a_row[i],
                    in_a_row[i] / text.4,
                    delta_us / last as f64,
                    b.v1_chunks.0,
                    b.v1_chunks.1,
                    b.new_chunks.0,
                    b.new_chunks.1,
                );
            }
            println!();
        }
    }
}

/// Fixed leaf bounds over a grid, inline + E449, both weights: what the
/// bounds do to v1, a version, the write and the cold read at each size.
fn sweep_mode(sizes: &[usize], runs: usize) {
    let grid: Vec<(usize, usize)> = std::env::var("E453_GRID")
        .map(|g| {
            g.split(',')
                .map(|p| {
                    let (a, b) = p.split_once(':').unwrap();
                    (a.parse().unwrap(), b.parse().unwrap())
                })
                .collect()
        })
        .unwrap_or_else(|_| {
            vec![
                (256, 2048),
                (1024, 8192),
                (2048, 16384),
                (4096, 32768),
                (8192, 65536),
                (16384, 131072),
                (32768, 262144),
            ]
        });
    let frames = std::env::var("E453_FRAMES").is_ok();
    println!("## sweep: leaf bounds, parents 16K:128K, inline + E449 (frames + SCR: {frames})\n");
    println!(
        "| transitions | weight | leaf | node bytes | bytes v1 (x text) | adds (x text) | write µs (x text) | cold µs (x text) | leaves / parents v1 | new a version |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for &n in sizes {
        let nets = chain(n);
        for weight in [ikam_kernel::DEFAULT_READ_WEIGHT, HEAVY_READ_WEIGHT] {
            let t = build(&nets, &TEXT, weight);
            let tc = time_with(
                runs,
                || {},
                || read_v(&t.repo, &TEXT, t.commits.last().unwrap()),
            );
            for &leaf in &grid {
                let o = Opts {
                    layout: Some(Layout::Inline),
                    e449: true,
                    frames,
                    chunks: Some(bounds(leaf, kib(16, 128))),
                    policy: None,
                };
                let b = build(&nets, &o, weight);
                let c = *b.commits.last().unwrap();
                let cold = time_with(
                    runs,
                    || b.repo.graph_cache_clear(),
                    || read_v(&b.repo, &o, &c),
                );
                let bytes = NetChunkPolicy::net_bytes(&nets[0], Layout::Inline);
                println!(
                    "| {n} | {weight} | {} | {bytes} | {} ({:.2}) | {:.0} ({:.2}) | {:.0} ({:.2}) | {:.0} ({:.2}) | {} / {} | {:.1} / {:.1} |",
                    show(Some(ChunkSizes::uniform(leaf.0, leaf.1))),
                    b.first,
                    b.first as f64 / t.first as f64,
                    b.added,
                    b.added / t.added,
                    b.later_us,
                    b.later_us / t.later_us,
                    cold,
                    cold / tc,
                    b.v1_chunks.0,
                    b.v1_chunks.1,
                    b.new_chunks.0,
                    b.new_chunks.1,
                );
            }
        }
    }
    println!();
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
fn firings(candidates: usize, big: usize, o: &Opts, at: bool) -> (f64, String, bool, f64) {
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

/// Firings on E422's night net edited 9 times mid-run, alone and inside
/// 10,000 more transitions, tracked in the text and as inline fragments
/// under the default bounds, E453's best fixed bounds and policy C.
fn fire_mode(candidates: usize) {
    println!("## firings (median of 3 runs)\n");
    let base = |chunks: Option<ChunkSizes>, policy: Option<NetChunkPolicy>| Opts {
        layout: Some(Layout::Inline),
        e449: true,
        chunks,
        policy,
        ..TEXT
    };
    for big in [0, 10000] {
        let candidates = if big > 0 {
            (candidates / 3).max(10)
        } else {
            candidates
        };
        println!("### night net inside {big} more transitions, {candidates} candidates\n");
        println!("| tracked by | bounds | fire µs each | replay µs | replay |");
        println!("| --- | --- | --- | --- | --- |");
        let mut markings = Vec::new();
        for (name, o, at) in [
            ("text (E422)", TEXT, false),
            ("inline + E449, default", base(None, None), false),
            ("inline + E449, default, by key", base(None, None), true),
            (
                "inline + E449, fixed L8K",
                base(Some(bounds(kib(8, 64), kib(16, 128))), None),
                false,
            ),
            (
                "inline + E449, policy C",
                base(None, Some(NetChunkPolicy::by_size())),
                false,
            ),
            (
                "inline + E449, policy C, by key",
                base(None, Some(NetChunkPolicy::by_size())),
                true,
            ),
        ] {
            let mut runs: Vec<(f64, String, bool, f64)> =
                (0..3).map(|_| firings(candidates, big, &o, at)).collect();
            let fire = median(runs.iter().map(|r| r.0).collect());
            let replay = median(runs.iter().map(|r| r.3).collect());
            let (_, m, ok, _) = runs.pop().unwrap();
            let net = night(1, big);
            let kept = match o.layout {
                None => "–".to_string(),
                Some(_) => {
                    let mut repo = Repo::new(MemStore::default());
                    apply(&mut repo, &o);
                    repo.put_net_fragments(&net, Layout::Inline).unwrap();
                    show(repo.net_chunk_bounds(&net.name).or(o.chunks).or(Some(D)))
                }
            };
            println!(
                "| {name} | {kept} | {fire:.0} | {replay:.0} | {} |",
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

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "nets".into());
    let sizes: Vec<usize> = args
        .next()
        .map(|s| s.split(',').map(|x| x.parse().unwrap()).collect())
        .unwrap_or_else(|| vec![300, 1000, 3000, 10000]);
    let runs: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    match mode.as_str() {
        "nets" => nets_mode(&sizes, runs),
        "sweep" => sweep_mode(&sizes, runs),
        "fire" => fire_mode(sizes[0]),
        m => panic!("unknown mode {m}"),
    }
}
