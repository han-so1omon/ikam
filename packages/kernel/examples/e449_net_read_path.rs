//! E449: where the time of a fragment-form net's read and write goes, and
//! what the fast read path (options, off by default) does to it.
//!
//! The nets and the 25-version edit chain are E446's (`e446_net_fragments`,
//! copied here so the two stay independent).
//!
//!   cargo run --release --example e449_net_read_path -- profile [SIZES]
//!   cargo run --release --example e449_net_read_path -- nets [SIZES]
//!   cargo run --release --example e449_net_read_path -- fire [CANDIDATES]
//!   cargo run --release --example e449_net_read_path -- graphs [SCALE]
//!
//! SIZES is a comma-separated list (default 1000,10000). Times are the
//! median of repeated runs (other builds share the machine).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Instant;

use ikam_kernel::net::{Arc, Kind, Layout, Net, Place, Transition};
use ikam_kernel::{Arg, Edge, Graph, Id, MemStore, Node, Object, Repo, Store, Target, node_key};

const VERSIONS: usize = 25;
const PATH: &str = "nets/s.net";

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

/// Median of `runs` runs of `f`, in µs (at least 5 runs and 300 ms).
fn time<T>(mut f: impl FnMut() -> T) -> f64 {
    let (mut all, mut total) = (Vec::new(), 0.0);
    while all.len() < 5 || total < 0.3 {
        let at = Instant::now();
        std::hint::black_box(f());
        let t = at.elapsed().as_secs_f64();
        all.push(t);
        total += t;
    }
    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    all[all.len() / 2] * 1e6
}

/// Text and tree-fragment stores holding the whole chain, one commit a
/// version; returns the repos, the commits and the fragment roots.
type Stores = (Repo<MemStore>, Vec<Id>, Repo<MemStore>, Vec<Id>, Vec<Id>);

fn stores(nets: &[Net]) -> Stores {
    let mut text = Repo::new(MemStore::default());
    let mut frag = Repo::new(MemStore::default());
    let (mut tt, mut ft) = (None, None);
    let (mut tc, mut fc, mut roots) = (vec![], vec![], vec![]);
    for net in nets {
        let (t, _) = text.put_net_file(tt, PATH, net).unwrap();
        tt = Some(t);
        tc.push(text.commit("main", t, "v").unwrap());
        let (t, root) = frag
            .put_net_fragment_file(ft, PATH, net, Layout::Tree)
            .unwrap();
        ft = Some(t);
        fc.push(frag.commit("main", t, "v").unwrap());
        roots.push(root);
    }
    (text, tc, frag, fc, roots)
}

fn lines(net: &Net) -> Vec<String> {
    net.to_text().lines().map(|l| format!("{l}\n")).collect()
}

fn profile(n: usize) {
    let nets = chain(n);
    let last = nets.len() - 1;
    let (text, tc, frag, fc, roots) = stores(&nets);
    let net = &nets[last];
    let root = roots[last];
    println!("\n### {n} transitions, version {VERSIONS}\n");
    println!("| step | µs |\n| --- | --- |");
    let row = |name: &str, us: f64| println!("| {name} | {us:.0} |");
    row(
        "text: read v25 whole (net_file)",
        time(|| text.net_file(&tc[last], PATH).unwrap()),
    );
    let tid = net.version_id();
    row(
        "text: read the content",
        time(|| text.read_content(&tid).unwrap()),
    );
    let body = net.to_text();
    row("text: Net::parse", time(|| Net::parse(&body).unwrap()));
    row("Net::check", time(|| net.check().unwrap()));
    row(
        "fragments: read v25 whole (net_fragment_file)",
        time(|| frag.net_fragment_file(&fc[last], PATH).unwrap()),
    );
    row(
        "fragments: graph_nodes (chunks fetched, decoded)",
        time(|| frag.graph_nodes(&root).unwrap()),
    );
    let nodes = frag.graph_nodes(&root).unwrap();
    let ids: Vec<Id> = nodes
        .iter()
        .map(|n| match n.target {
            Target::Content(Arg::Whole(id)) => id,
            _ => panic!(),
        })
        .collect();
    row(
        &format!("fragments: read_content of {} fragments", ids.len()),
        time(|| {
            ids.iter()
                .map(|id| frag.read_content(id).unwrap().len())
                .sum::<usize>()
        }),
    );
    let ls = lines(net);
    row(
        "fragments: hash every line (Id::of_content)",
        time(|| {
            ls.iter()
                .map(|l| Id::of_content(l.as_bytes()))
                .collect::<Vec<_>>()
        }),
    );
    row(
        "fragments: print every line (to_text)",
        time(|| net.to_text()),
    );
    row(
        "fragments: clone the Net",
        time(|| std::hint::black_box(net.clone())),
    );
    // Writes: version 25 over version 24, as the chain writes it.
    let mut t2 = Repo::new(MemStore::default());
    let mut f2 = Repo::new(MemStore::default());
    let (tt, _) = t2.put_net_file(None, PATH, &nets[last - 1]).unwrap();
    let (ft, _) = f2
        .put_net_fragment_file(None, PATH, &nets[last - 1], Layout::Tree)
        .unwrap();
    // After the first run the version exists, as in a firing's re-put.
    let at = Instant::now();
    t2.put_net_file(Some(tt), PATH, net).unwrap();
    row("text: write v25 (new)", at.elapsed().as_secs_f64() * 1e6);
    let at = Instant::now();
    f2.put_net_fragment_file(Some(ft), PATH, net, Layout::Tree)
        .unwrap();
    row(
        "fragments: write v25 (new)",
        at.elapsed().as_secs_f64() * 1e6,
    );
    row(
        "text: write v25 again (all stored)",
        time(|| t2.put_net_file(Some(tt), PATH, net).unwrap()),
    );
    row(
        "fragments: write v25 again (all stored)",
        time(|| {
            f2.put_net_fragment_file(Some(ft), PATH, net, Layout::Tree)
                .unwrap()
        }),
    );
    row(
        "fragments: put_content of every line (all stored)",
        time(|| {
            for l in &ls {
                f2.put_content(l.as_bytes()).unwrap();
            }
        }),
    );
    row(
        "fragments: put_graph of the nodes (all stored)",
        time(|| f2.put_graph(nodes.clone()).unwrap()),
    );
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

/// The planner's weight under the bench's heavy workload (E069, E446).
const HEAVY_READ_WEIGHT: f64 = 48.8;

/// How a fragment store is set up.
#[derive(Clone, Copy)]
struct Opts {
    fast: bool,
    cache: usize,
    threads: usize,
    delta: bool,
}

const E446: Opts = Opts {
    fast: false,
    cache: 0,
    threads: 0,
    delta: false,
};

fn apply(repo: &mut Repo<MemStore>, o: Opts) {
    repo.net_fast_read = o.fast;
    repo.graph_cache = o.cache;
    repo.read_threads = o.threads;
    repo.net_delta_write = o.delta;
}

/// One store of the chain in one form: bytes, write time, commits, roots.
struct Built {
    repo: Repo<MemStore>,
    commits: Vec<Id>,
    roots: Vec<Id>,
    first: usize,
    added: f64,
    write_us: f64,
    /// Mean write of versions 2 to 25.
    later_us: f64,
    reuse: f64,
}

fn build(nets: &[Net], text: bool, o: Opts, weight: f64) -> Built {
    build_in(nets, text, o, weight, true)
}

/// A fragment store written with `object_templates` as given.
fn build_with(nets: &[Net], o: Opts, weight: f64, templates: bool) -> Built {
    build_in(nets, false, o, weight, templates)
}

fn build_in(nets: &[Net], text: bool, o: Opts, weight: f64, templates: bool) -> Built {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = weight;
    repo.object_templates = templates;
    apply(&mut repo, o);
    let (mut tree, mut commits, mut roots) = (None, vec![], vec![]);
    let (mut sizes, mut write_s, mut first_s) = (vec![], 0.0, 0.0);
    let (mut seen, mut reused, mut total) = (HashSet::new(), 0usize, 0usize);
    for net in nets {
        let before = size(&repo);
        if roots.len() == 1 {
            first_s = write_s;
        }
        let at = Instant::now();
        let (t, root) = if text {
            repo.put_net_file(tree, PATH, net).unwrap()
        } else {
            repo.put_net_fragment_file(tree, PATH, net, Layout::Tree)
                .unwrap()
        };
        write_s += at.elapsed().as_secs_f64();
        sizes.push(size(&repo) - before);
        tree = Some(t);
        commits.push(repo.commit("main", t, "v").unwrap());
        roots.push(root);
        if !text {
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
        write_us: write_s * 1e6 / nets.len() as f64,
        later_us: (write_s - first_s) * 1e6 / (nets.len() - 1) as f64,
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

fn nets_mode(sizes: &[usize]) {
    println!("## nets: 25 versions, tree layout (times: median of runs)\n");
    for weight in [ikam_kernel::DEFAULT_READ_WEIGHT, HEAVY_READ_WEIGHT] {
        for &n in sizes {
            nets_size(n, weight);
        }
    }
}

fn nets_size(n: usize, weight: f64) {
    let nets = chain(n);
    let last = nets.len() - 1;
    let runs = if n >= 10000 { 7 } else { 15 };
    println!("### {n} transitions, read weight {weight}\n");
    let text = build(&nets, true, E446, weight);
    let base = build(&nets, false, E446, weight);
    let fast = Opts {
        fast: true,
        cache: 0,
        threads: 0,
        delta: false,
    };
    let cached = Opts {
        cache: 1 << 16,
        ..fast
    };
    let par = Opts {
        threads: 3,
        ..cached
    };
    let delta = Opts {
        delta: true,
        ..cached
    };
    let dw = build(&nets, false, delta, weight);
    assert_eq!(dw.roots, base.roots, "delta write gives the same roots");
    // The planner's template search on new chunks (E144) is most of what a
    // delta write has left: the same write with it off, for the owner.
    let dwt = {
        let mut b = build_with(&nets, delta, weight, false);
        assert_eq!(b.roots, base.roots);
        b.repo = Repo::new(MemStore::default());
        b
    };
    println!(
        "| form | bytes v1 | bytes a version adds | objects reused | write µs a version (all 25) | write µs, versions 2-25 |"
    );
    println!("| --- | --- | --- | --- | --- | --- |");
    for (name, b) in [
        ("text (E422)", &text),
        ("fragments, tree (E446)", &base),
        ("fragments, tree, delta write (E449)", &dw),
        ("fragments, tree, delta write, object_templates off", &dwt),
    ] {
        println!(
            "| {name} | {} | {:.0} | {:.1}% | {:.0} | {:.0} |",
            b.first, b.added, b.reuse, b.write_us, b.later_us
        );
    }
    println!();
    // Reads of version 25.
    let tr = &text.repo;
    let t_us = time_with(
        runs,
        || {},
        || tr.net_file(&text.commits[last], PATH).unwrap(),
    );
    let mut repo = base.repo;
    let (c25, c24) = (base.commits[last], base.commits[last - 1]);
    let read = |r: &Repo<MemStore>, c: &Id| r.net_fragment_file(c, PATH).unwrap();
    assert_eq!(read(&repo, &c25), nets[last]);
    let e446_us = time_with(runs, || {}, || read(&repo, &c25));
    apply(&mut repo, fast);
    assert_eq!(read(&repo, &c25), nets[last]);
    let fast_us = time_with(runs, || {}, || read(&repo, &c25));
    apply(&mut repo, cached);
    let cold = time_with(runs, || repo.graph_cache_clear(), || read(&repo, &c25));
    let warm = time_with(
        runs,
        || {
            repo.graph_cache_clear();
            read(&repo, &c24);
        },
        || read(&repo, &c25),
    );
    let (hits, misses) = {
        repo.graph_cache_clear();
        read(&repo, &c24);
        let (h0, m0) = repo.graph_cache_stats();
        assert_eq!(read(&repo, &c25), nets[last]);
        let (h, m) = repo.graph_cache_stats();
        (h - h0, m - m0)
    };
    let hot = time_with(runs, || {}, || read(&repo, &c25));
    apply(&mut repo, par);
    let r25 = base.roots[last];
    assert_eq!(repo.net_fragments_par(&r25).unwrap(), nets[last]);
    let par_us = time_with(
        runs,
        || repo.graph_cache_clear(),
        || repo.net_fragments_par(&r25).unwrap(),
    );
    // A Net held at version 24 brought to 25 by the delta (E446's view).
    let r24 = base.roots[last - 1];
    let held = nets[last - 1].clone();
    let apply_us = time_with(
        runs,
        || {},
        || repo.net_delta(&r24, &r25).unwrap().apply(&held).unwrap(),
    );
    // One transition and its places, by key.
    let t = nets[last].transitions[n / 2].name.clone();
    let part_cold = time_with(
        runs,
        || repo.graph_cache_clear(),
        || repo.net_fragments_part(&r25, &[t.as_str()]).unwrap(),
    );
    let part_warm = time_with(
        runs,
        || {},
        || repo.net_fragments_part(&r25, &[t.as_str()]).unwrap(),
    );
    println!("| read of version 25 | µs | against text |");
    println!("| --- | --- | --- |");
    let rows = [
        ("text (E422): net_file", t_us),
        ("fragments (E446): net_fragment_file", e446_us),
        ("E449 fast read, no cache", fast_us),
        ("E449 fast read + chunk cache, cold (cache emptied)", cold),
        ("E449 fast read + chunk cache, after reading v24", warm),
        ("E449 fast read + chunk cache, v25 kept (hot)", hot),
        (
            "E449 fast read, cold, 3 threads (net_fragments_par)",
            par_us,
        ),
        ("E446 delta v24 to v25 applied to a held v24", apply_us),
        ("E449 one transition and its places by key, cold", part_cold),
        (
            "E449 one transition and its places by key, cached",
            part_warm,
        ),
    ];
    for (name, us) in rows {
        println!("| {name} | {us:.0} | {:.2}x |", us / t_us);
    }
    println!(
        "\n- after reading v24, reading v25 decoded {misses} chunks and took {hits} from the cache\n"
    );
    // fsck, and every version read the same by both paths.
    apply(&mut repo, cached);
    for (net, root) in nets.iter().zip(&base.roots) {
        assert_eq!(&repo.net_fragments_full(root).unwrap(), net);
        assert_eq!(&repo.net_fragments_fast(root).unwrap(), net);
    }
    let fsck = repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
    let fsck_dw = dw.repo.fsck().unwrap().len() + dw.repo.fsck_deep().unwrap().len();
    println!("- fsck and fsck_deep problems: E446 store {fsck}, delta-write store {fsck_dw}\n");
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

/// E422's night net at version `k`, inside `big` synthetic transitions
/// (0: the night net alone).
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

#[derive(Clone, Copy)]
enum Track {
    Text,
    Fragments(Opts),
    /// Fragments, each firing read by key from the version's root.
    At(Opts),
}

/// E422's run (the net edited 9 times mid-run): µs a firing, the marking,
/// whether replay gives it, and replay's µs.
fn firings(candidates: usize, big: usize, track: Track) -> (f64, String, bool, f64) {
    let mut repo = Repo::new(MemStore::default());
    if let Track::Fragments(o) | Track::At(o) = track {
        apply(&mut repo, o);
    }
    let mut cur = night(1, big);
    let mut db = repo.db_empty().unwrap();
    db = repo.net_init(&db, &cur).unwrap();
    db = match track {
        Track::Text => repo.net_track_versions(&db, &cur).unwrap(),
        _ => repo.net_track_fragments(&db, &cur, Layout::Tree).unwrap(),
    };
    let mut root = repo.put_net_fragments(&cur, Layout::Tree).unwrap();
    let every = (candidates / 10).max(1);
    let (mut fire_s, mut count) = (0.0, 0);
    for i in 0..candidates {
        if i > 0 && i % every == 0 {
            cur = night(cur.version + 1, big);
            if let Track::At(_) = track {
                root = repo.put_net_fragments(&cur, Layout::Tree).unwrap();
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
            let at = Instant::now();
            db = match track {
                Track::At(_) => repo.net_fire_at(&db, &root, t, Some(&c), &key),
                _ => repo.net_fire(&db, &cur, t, Some(&c), &key),
            }
            .unwrap()
            .0;
            fire_s += at.elapsed().as_secs_f64();
            count += 1;
        }
    }
    let m = repo.net_marking(&db, "night").unwrap();
    let at = Instant::now();
    let ok = repo.net_replay_versions(&db, "night").unwrap() == m;
    let replay = at.elapsed().as_secs_f64() * 1e6;
    (fire_s * 1e6 / count as f64, format!("{m:?}"), ok, replay)
}

fn fire_mode(candidates: usize) {
    println!("## firings: E422's night net edited 9 times mid-run, {candidates} candidates\n");
    let fast = Opts {
        fast: true,
        cache: 1 << 16,
        threads: 0,
        delta: true,
    };
    for big in [0, 10000] {
        // The big net's text and E446 firings write 85 KB to 1.2 MB a
        // firing: a third of the candidates.
        let candidates = if big > 0 {
            (candidates / 3).max(10)
        } else {
            candidates
        };
        println!(
            "### {}, {candidates} candidates\n",
            match big {
                0 => "the night net (5 to 6 places, 3 to 4 transitions)".to_string(),
                b => format!("the night net inside {b} more transitions"),
            }
        );
        println!("| tracked by | fire µs each | replay µs | replay |");
        println!("| --- | --- | --- | --- |");
        let mut markings = Vec::new();
        for (name, track) in [
            ("text (E422)", Track::Text),
            ("fragments, tree (E446)", Track::Fragments(E446)),
            (
                "fragments, tree, E449 delta write + fast read",
                Track::Fragments(fast),
            ),
            ("fragments, tree, E449 net_fire_at by key", Track::At(fast)),
        ] {
            let mut us = vec![];
            let mut last = None;
            for _ in 0..3 {
                let r = firings(candidates, big, track);
                us.push((r.0, r.3));
                last = Some(r);
            }
            us.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let (_, m, ok, _) = last.unwrap();
            let mut rp: Vec<f64> = us.iter().map(|u| u.1).collect();
            rp.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!(
                "| {name} | {:.1} | {:.0} | {} |",
                us[1].0,
                rp[1],
                if ok { "equal marking" } else { "FAILS" }
            );
            markings.push(m);
        }
        println!(
            "\n- final markings equal across the four: {}\n",
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

fn node_of(l: &str, payload: &str, to: &BTreeSet<String>) -> Node {
    Node {
        label: l.to_string(),
        target: Target::Content(Arg::Whole(Id::of_content(payload.as_bytes()))),
        edges: to
            .iter()
            .map(|t| Edge {
                to: node_key(t),
                label: format!("link {t}"),
            })
            .collect(),
    }
}

fn follow(e: &Edge) -> Option<String> {
    e.label.strip_prefix("link ").map(String::from)
}

struct GraphBuilt {
    repo: Repo<MemStore>,
    roots: Vec<Id>,
    first: usize,
    added: f64,
    write_us: f64,
    reuse: f64,
}

/// The chain stored one version at a time: whole (every payload put,
/// `put_graph`) or by edits (the touched payloads put, `graph_edit`).
/// The versions are made as they are written (the same chain every time:
/// the edits are seeded), so only one is held at once. Returns the last.
fn graph_build(g0: &G, edits: bool, weight: f64) -> (GraphBuilt, G) {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = weight;
    let (mut roots, mut sizes, mut write_s) = (vec![], vec![], 0.0);
    let (mut seen, mut reused, mut total) = (HashSet::new(), 0usize, 0usize);
    let mut rng = Rng(99);
    let mut g = g0.clone();
    for k in 0..VERSIONS {
        let touched = match k {
            0 => BTreeSet::new(),
            _ => graph_edit_step(&mut g, k + 1, &mut rng),
        };
        let g = &g;
        let before = size(&repo);
        let at = Instant::now();
        let root = if k == 0 || !edits {
            let mut nodes = Vec::with_capacity(g.len());
            for (l, (p, to)) in g {
                repo.put_content(p.as_bytes()).unwrap();
                nodes.push(node_of(l, p, to));
            }
            repo.put_graph(nodes).unwrap()
        } else {
            let mut e = BTreeMap::new();
            for l in &touched {
                let n = g.get(l).map(|(p, to)| {
                    repo.put_content(p.as_bytes()).unwrap();
                    node_of(l, p, to)
                });
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
    let built = GraphBuilt {
        repo,
        roots,
        first: sizes[0],
        added: later.iter().sum::<usize>() as f64 / later.len() as f64,
        write_us: write_s * 1e6 / (VERSIONS - 1) as f64,
        reuse: 100.0 * reused as f64 / total as f64,
    };
    (built, g)
}

fn graphs_mode(scale: f64) {
    println!("## general graphs: 25 versions of local edits (times: median of 5 runs)\n");
    let cases = [
        ("link graph", link_graph((100_000.0 * scale) as usize, 10)),
        (
            "property graph",
            property_graph((10_000.0 * scale) as usize),
        ),
    ];
    for (name, g0) in cases {
        let edges: usize = g0.values().map(|v| v.1.len()).sum();
        let bytes: usize = g0.values().map(|v| v.0.len()).sum();
        println!(
            "### {name}: {} nodes, {edges} edges, {bytes} payload bytes\n",
            g0.len()
        );
        println!(
            "| write | read weight | bytes v1 | bytes a version adds | objects reused | write µs, versions 2-25 |"
        );
        println!("| --- | --- | --- | --- | --- | --- |");
        let mut keep = None;
        for weight in [ikam_kernel::DEFAULT_READ_WEIGHT, HEAVY_READ_WEIGHT] {
            for edits in [false, true] {
                let (b, last) = graph_build(&g0, edits, weight);
                println!(
                    "| {} | {weight} | {} | {:.0} | {:.1}% | {:.0} |",
                    if edits {
                        "graph_edit, touched payloads put"
                    } else {
                        "put_graph, every payload put"
                    },
                    b.first,
                    b.added,
                    b.reuse,
                    b.write_us
                );
                if edits && weight == ikam_kernel::DEFAULT_READ_WEIGHT {
                    keep = Some((b, last));
                }
            }
        }
        let (b, g) = keep.unwrap();
        let mut repo = b.repo;
        let (r24, r25) = (b.roots[VERSIONS - 2], b.roots[VERSIONS - 1]);
        let plain = |repo: &Repo<MemStore>| {
            let nodes = repo.graph_nodes(&r25).unwrap();
            let mut total = 0;
            for n in &nodes {
                if let Target::Content(Arg::Whole(id)) = n.target {
                    total += repo.read_content(&id).unwrap().len();
                }
            }
            total
        };
        let fast = |repo: &Repo<MemStore>| -> usize {
            repo.graph_read(&r25)
                .unwrap()
                .iter()
                .flat_map(|l| l.nodes.iter())
                .map(|(_, p)| p.as_ref().map_or(0, |p| p.len()))
                .sum()
        };
        assert_eq!(plain(&repo), fast(&repo));
        let runs = 5;
        let plain_us = time_with(runs, || {}, || plain(&repo));
        let nocache = time_with(runs, || {}, || fast(&repo));
        repo.graph_cache = 1 << 16;
        let cold = time_with(runs, || repo.graph_cache_clear(), || fast(&repo));
        let warm = time_with(
            runs,
            || {
                repo.graph_cache_clear();
                repo.graph_read(&r24).unwrap();
            },
            || fast(&repo),
        );
        repo.read_threads = 3;
        let par = time_with(
            runs,
            || repo.graph_cache_clear(),
            || repo.graph_read_par(&r25).unwrap().len(),
        );
        repo.read_threads = 0;
        repo.prune_diff = true;
        let delta = time_with(runs, || {}, || repo.graph_diff(&r24, &r25).unwrap().len());
        // Partial reads: 100 labels by key, and 2-hop neighbourhoods of 10.
        let keys: Vec<&String> = g.keys().collect();
        let mut rng = Rng(5);
        let picks: Vec<String> = (0..100)
            .map(|_| keys[rng.below(keys.len())].clone())
            .collect();
        repo.graph_cache = 0;
        let by_key_plain = time_with(
            runs,
            || {},
            || {
                picks
                    .iter()
                    .filter(|l| repo.graph_node(&r25, l).unwrap().is_some())
                    .count()
            },
        );
        repo.graph_cache = 1 << 16;
        let by_key_cold = time_with(
            runs,
            || repo.graph_cache_clear(),
            || {
                picks
                    .iter()
                    .filter(|l| repo.graph_node_cached(&r25, l).unwrap().is_some())
                    .count()
            },
        );
        let by_key_warm = time_with(
            runs,
            || {},
            || {
                picks
                    .iter()
                    .filter(|l| repo.graph_node_cached(&r25, l).unwrap().is_some())
                    .count()
            },
        );
        let starts: Vec<String> = picks[..10].to_vec();
        let hops_plain = time_with(
            runs,
            || {},
            || {
                let mut seen: BTreeSet<String> = BTreeSet::new();
                let mut frontier: BTreeSet<String> = starts.iter().cloned().collect();
                for hop in 0..=2 {
                    let mut next = BTreeSet::new();
                    for n in repo.graph_nodes_in(&r25, &frontier).unwrap() {
                        if hop < 2 {
                            next.extend(n.edges.iter().filter_map(follow));
                        }
                        seen.insert(n.label);
                    }
                    frontier = next.into_iter().filter(|l| !seen.contains(l)).collect();
                }
                seen.len()
            },
        );
        let hop_n = repo.graph_hops(&r25, &starts, 2, follow).unwrap().len();
        let hops_cold = time_with(
            runs,
            || repo.graph_cache_clear(),
            || repo.graph_hops(&r25, &starts, 2, follow).unwrap().len(),
        );
        let hops_warm = time_with(
            runs,
            || {},
            || repo.graph_hops(&r25, &starts, 2, follow).unwrap().len(),
        );
        println!("\n| read of version 25 | µs |");
        println!("| --- | --- |");
        for (n, us) in [
            ("whole, plain (graph_nodes + read_content each)", plain_us),
            ("whole, E449 graph_read, no cache", nocache),
            ("whole, E449 graph_read, cache emptied", cold),
            ("whole, E449 graph_read, after reading v24", warm),
            ("whole, E449 graph_read_par, 3 threads, cache emptied", par),
            (
                "delta v24 to v25 (graph_diff, shared chunks skipped)",
                delta,
            ),
            ("100 nodes by key, plain (graph_node)", by_key_plain),
            ("100 nodes by key, E449 cached, cache emptied", by_key_cold),
            ("100 nodes by key, E449 cached, kept", by_key_warm),
            (
                &*format!("2-hop of 10 starts ({hop_n} nodes), plain (graph_nodes_in)"),
                hops_plain,
            ),
            ("2-hop, E449 graph_hops, cache emptied", hops_cold),
            ("2-hop, E449 graph_hops, kept", hops_warm),
        ] {
            println!("| {n} | {us:.0} |");
        }
        let fsck = repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
        println!("\n- fsck and fsck_deep problems: {fsck}\n");
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "profile".into());
    let arg = args.next();
    let sizes = || -> Vec<usize> {
        arg.as_ref()
            .map(|s| s.split(',').map(|x| x.parse().unwrap()).collect())
            .unwrap_or_else(|| vec![1000, 10000])
    };
    match mode.as_str() {
        "profile" => {
            println!("## where the time goes (median of at least 5 runs and 300 ms)");
            for n in sizes() {
                profile(n);
            }
        }
        "nets" => nets_mode(&sizes()),
        "write" => {
            // For a profiler: the chain written with delta writes.
            let nets = chain(sizes()[0]);
            let o = Opts {
                fast: true,
                cache: 1 << 16,
                threads: 0,
                delta: true,
            };
            std::hint::black_box(build(&nets, false, o, ikam_kernel::DEFAULT_READ_WEIGHT).first);
        }
        "cold" => {
            // For a profiler: cold fast reads of version 25, over and over.
            let nets = chain(sizes()[0]);
            let b = build(&nets, false, E446, ikam_kernel::DEFAULT_READ_WEIGHT);
            let mut repo = b.repo;
            repo.graph_cache = 1 << 16;
            let r = b.roots[nets.len() - 1];
            let hot = std::env::var("HOT").is_ok();
            for _ in 0..20 {
                if !hot {
                    repo.graph_cache_clear();
                }
                std::hint::black_box(repo.net_fragments_fast(&r).unwrap());
            }
        }
        "fire" => fire_mode(arg.map_or(300, |a| a.parse().unwrap())),
        "graphs" => graphs_mode(arg.map_or(1.0, |a| a.parse().unwrap())),
        m => panic!("unknown mode {m}"),
    }
}
