//! E446: nets as connected fragments, chained by commits, with deltas
//! computed as views over the traversal.
//!
//! 1. Synthetic nets of `SIZES` transitions, each put through a chain of
//!    `VERSIONS` versions with mixed edits (rename a transition, change an
//!    arc weight, add a transition and a place, remove a place, a burst of
//!    edits in one region), in four forms: E422's text file, the old graph
//!    object (`put_net`), and fragments with a flat and with a tree layout.
//!    Per form and size: bytes the first version costs, bytes each later
//!    version adds, fragment reuse, write and read time, delta time and
//!    chunks visited against a full text diff, and that every delta
//!    applied to its earlier version gives the later one exactly.
//! 2. Firings: the night net of E422 edited mid-run, tracked by text and
//!    by fragments; both replay to the same marking.
//!
//!   cargo run --release --example e446_net_fragments [-- SIZES [CANDIDATES]]
//!
//! SIZES is a comma-separated list (default 10,1000,10000).

use std::collections::HashSet;
use std::time::Instant;

use ikam_kernel::net::{Arc, Kind, Layout, Net, Place, Transition};
use ikam_kernel::{Arg, Graph, Id, MemStore, Object, Repo, Store, Target};

const VERSIONS: usize = 25;
const PATH: &str = "nets/s.net";

fn size(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn arc(place: String, weight: u32) -> Arc {
    Arc { place, weight }
}

/// A synthetic net of `n` transitions over `max(4, n/2)` places.
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

/// The weight of a transition's first arc (an output if it has no input).
fn first_arc(t: &mut Transition) -> &mut u32 {
    match t.inputs.first_mut() {
        Some(a) => &mut a.weight,
        None => &mut t.outputs[0].weight,
    }
}

/// Version `k` from version `k - 1`: one of five edits, by `k`.
fn edit(net: &mut Net, k: usize) -> &'static str {
    net.version = k as u32;
    let pick = |len: usize| (k.wrapping_mul(2_654_435_761) >> 7) % len.max(1);
    match k % 5 {
        0 => {
            let j = pick(net.transitions.len());
            net.transitions[j].name = format!("{}r{k}", net.transitions[j].name);
            "rename a transition"
        }
        1 => {
            let j = pick(net.transitions.len());
            let w = first_arc(&mut net.transitions[j]);
            *w = *w % 5 + 1;
            "change an arc weight"
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
            "add a transition (and a place)"
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
            "remove a place"
        }
        _ => {
            let m = (net.transitions.len() / 2).clamp(1, 20);
            let j = pick(net.transitions.len() - m + 1);
            for t in &mut net.transitions[j..j + m] {
                let w = first_arc(t);
                *w = *w % 4 + 1;
            }
            "a burst of 20 edits in one region"
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    Text,
    Graph,
    Flat,
    Tree,
}

impl Form {
    fn name(self) -> &'static str {
        match self {
            Form::Text => "text file (E422)",
            Form::Graph => "graph object (put_net)",
            Form::Flat => "fragments, flat",
            Form::Tree => "fragments, tree",
        }
    }
}

/// The net's own objects of a version (not the trees holding it), with
/// each one's logical size: a content's length, an object's encoding.
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
                    if let Target::Content(a) = n.target {
                        let c = match a {
                            Arg::Whole(c) => c,
                            Arg::Range { id, .. } | Arg::Graph(id) => id,
                        };
                        out.push((c, repo.read_content(&c).unwrap().len()));
                    }
                }
            }
            _ => panic!("not a graph"),
        }
    }
    out
}

/// Fastest of repeated runs of `f`, in µs: at least 3 and 200 ms.
fn time<T>(mut f: impl FnMut() -> T) -> f64 {
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

struct Row {
    form: Form,
    n: usize,
    first: usize,
    added: Vec<usize>,
    reuse_objects: (usize, usize),
    reuse_bytes: (usize, usize),
    write_us: f64,
    read_first_us: f64,
    read_last_us: f64,
    delta_us: f64,
    delta_chunks: f64,
    delta_fragments: f64,
    unpruned_chunks: f64,
    first_last_us: f64,
    first_last_chunks: usize,
    applied: (usize, usize),
    repack: Option<(usize, f64)>,
    heavy: (usize, f64),
    fsck: usize,
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

fn line_diff(a: &str, b: &str) -> usize {
    similar::TextDiff::from_lines(a, b)
        .iter_all_changes()
        .filter(|c| c.tag() != similar::ChangeTag::Equal)
        .count()
}

/// The planner's weight under the bench's heavy workload (README, E069):
/// about 49 stored bytes per decoded byte, so slices of earlier content
/// rarely pay.
const HEAVY_READ_WEIGHT: f64 = 48.8;

/// Store `net` in `form` at `PATH` in `tree`; returns the net's own root.
fn put_version(repo: &mut Repo<MemStore>, form: Form, tree: &mut Option<Id>, net: &Net) -> Id {
    match form {
        Form::Text => {
            let (t, id) = repo.put_net_file(*tree, PATH, net).unwrap();
            *tree = Some(t);
            id
        }
        Form::Graph => {
            // Held in a commit's tree too, so repack keeps it.
            let id = repo.put_net(net).unwrap();
            let edit = ikam_kernel::Edit::PutGraph {
                path: PATH.into(),
                id,
            };
            *tree = Some(repo.edit_tree(*tree, &[edit]).unwrap());
            id
        }
        Form::Flat | Form::Tree => {
            let layout = if form == Form::Flat {
                Layout::Flat
            } else {
                Layout::Tree
            };
            let (t, id) = repo
                .put_net_fragment_file(*tree, PATH, net, layout)
                .unwrap();
            *tree = Some(t);
            id
        }
    }
}

/// Bytes the first version costs and the mean a later one adds, planned
/// at the heavy read weight.
fn heavy_bytes(form: Form, nets: &[Net]) -> (usize, f64) {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = HEAVY_READ_WEIGHT;
    let (mut tree, mut sizes) = (None, Vec::new());
    for net in nets {
        let before = size(&repo);
        put_version(&mut repo, form, &mut tree, net);
        sizes.push(size(&repo) - before);
        repo.commit("main", tree.unwrap(), "v").unwrap();
    }
    let later = &sizes[1..];
    (
        sizes[0],
        later.iter().sum::<usize>() as f64 / later.len() as f64,
    )
}

fn measure(form: Form, nets: &[Net], repack: bool) -> Row {
    let n = nets[0].transitions.len();
    let mut repo = Repo::new(MemStore::default());
    let (mut tree, mut trees, mut roots) = (None, Vec::new(), Vec::new());
    let (mut first, mut added, mut write_s) = (0, Vec::new(), 0.0);
    let mut seen: HashSet<Id> = HashSet::new();
    let (mut reuse_objects, mut reuse_bytes) = ((0, 0), (0, 0));
    for (k, net) in nets.iter().enumerate() {
        let before = size(&repo);
        let at = Instant::now();
        let root = put_version(&mut repo, form, &mut tree, net);
        write_s += at.elapsed().as_secs_f64();
        let grown = size(&repo) - before;
        if k == 0 {
            first = grown;
        } else {
            added.push(grown);
        }
        // Git-like: each version a commit on the last (not counted above).
        if let Some(t) = tree {
            trees.push(repo.commit("main", t, &format!("v{}", k + 1)).unwrap());
        }
        let objs = reach(&repo, form, &root);
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
        roots.push(root);
    }
    repo.prune_diff = true; // E249, for the graph form's diff
    let read = |repo: &Repo<MemStore>, k: usize| -> Net {
        match form {
            Form::Text => repo.net_file(&trees[k], PATH).unwrap(),
            Form::Graph => repo.net_at(&roots[k]).unwrap(),
            Form::Flat | Form::Tree => repo.net_fragment_file(&trees[k], PATH).unwrap(),
        }
    };
    let last = nets.len() - 1;
    // put_net keeps places in label order, not the net's (E269).
    let same = |a: &Net, b: &Net| {
        let sorted = |n: &Net| {
            let mut n = n.clone();
            if form == Form::Graph {
                n.places.sort_by(|x, y| x.name.cmp(&y.name));
            }
            n
        };
        sorted(a) == sorted(b)
    };
    for k in [0, last] {
        assert!(same(&read(&repo, k), &nets[k]), "{form:?} v{}", k + 1);
    }
    let read_first_us = time(|| read(&repo, 0));
    let read_last_us = time(|| read(&repo, last));
    // Deltas between consecutive versions, and from the first to the last.
    let (mut delta_us, mut chunks, mut frags, mut unpruned) = (0.0, 0, 0, 0);
    let mut applied = (0, 0);
    let delta = |repo: &Repo<MemStore>, a: usize, b: usize| -> (f64, usize, usize) {
        match form {
            Form::Text => {
                let us = time(|| {
                    let x = repo.read_content(&roots[a]).unwrap();
                    let y = repo.read_content(&roots[b]).unwrap();
                    line_diff(
                        std::str::from_utf8(&x).unwrap(),
                        std::str::from_utf8(&y).unwrap(),
                    )
                });
                let lines = nets[a].to_text().lines().count() + nets[b].to_text().lines().count();
                (us, lines, 0)
            }
            Form::Graph => {
                let us = time(|| repo.graph_diff(&roots[a], &roots[b]).unwrap());
                (us, 0, 0)
            }
            Form::Flat | Form::Tree => {
                let us = time(|| repo.net_delta(&roots[a], &roots[b]).unwrap());
                let d = repo.net_delta(&roots[a], &roots[b]).unwrap();
                (us, d.chunks_read, d.fragments_read)
            }
        }
    };
    for k in 1..nets.len() {
        let (us, c, f) = delta(&repo, k - 1, k);
        delta_us += us;
        chunks += c;
        frags += f;
        if matches!(form, Form::Flat | Form::Tree) {
            let d = repo.net_delta(&roots[k - 1], &roots[k]).unwrap();
            let full = repo
                .net_delta_with(&roots[k - 1], &roots[k], false)
                .unwrap();
            unpruned += full.chunks_read;
            assert_eq!(
                (&d.places, &d.transitions, &d.links),
                (&full.places, &full.transitions, &full.links)
            );
            applied.1 += 1;
            if d.apply(&nets[k - 1]).unwrap() == nets[k] {
                applied.0 += 1;
            }
        }
    }
    let (first_last_us, first_last_chunks, _) = delta(&repo, 0, last);
    if matches!(form, Form::Flat | Form::Tree) {
        let d = repo.net_delta(&roots[0], &roots[last]).unwrap();
        applied.1 += 1;
        if d.apply(&nets[0]).unwrap() == nets[last] {
            applied.0 += 1;
        }
    }
    let fsck = repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
    let repack = repack.then(|| {
        let at = Instant::now();
        repo.repack().unwrap();
        let s = at.elapsed().as_secs_f64();
        for k in [0, last] {
            assert!(same(&read(&repo, k), &nets[k]));
        }
        (size(&repo), s)
    });
    let e = (nets.len() - 1) as f64;
    Row {
        form,
        n,
        first,
        added,
        reuse_objects,
        reuse_bytes,
        write_us: write_s * 1e6 / nets.len() as f64,
        read_first_us,
        read_last_us,
        delta_us: delta_us / e,
        delta_chunks: chunks as f64 / e,
        delta_fragments: frags as f64 / e,
        unpruned_chunks: unpruned as f64 / e,
        first_last_us,
        first_last_chunks,
        applied,
        repack,
        heavy: heavy_bytes(form, nets),
        fsck,
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

fn night(k: u32) -> Net {
    let mut text = NIGHT.replace("net night 1", &format!("net night {k}"));
    if k > 1 {
        text += "place checked colored\n";
        text += &format!("transition check{k} pure: measured -> checked\n");
    }
    Net::parse(&text).unwrap()
}

/// E422's run on the night net, tracked by text or by fragments: firing
/// time, record bytes, the final marking and whether replay equals it.
fn firings(n: usize, layout: Option<Layout>) -> (f64, f64, String, bool, usize) {
    let mut repo = Repo::new(MemStore::default());
    let mut cur = night(1);
    let mut db = repo.db_empty().unwrap();
    db = repo.net_init(&db, &cur).unwrap();
    db = match layout {
        None => repo.net_track_versions(&db, &cur).unwrap(),
        Some(l) => repo.net_track_fragments(&db, &cur, l).unwrap(),
    };
    let every = (n / 10).max(1);
    let (mut fire_s, mut count) = (0.0, 0);
    for i in 0..n {
        if i > 0 && i % every == 0 {
            cur = night(cur.version + 1);
        }
        let c = format!("c{i}");
        let mut steps = vec![("propose", "p"), ("start", "s"), ("finish", "f")];
        let check = format!("check{}", cur.version);
        if cur.version > 1 {
            steps.push((check.as_str(), "k"));
        }
        for (t, k) in steps {
            let at = Instant::now();
            db = repo
                .net_fire(&db, &cur, t, Some(&c), &format!("{k}{i}"))
                .unwrap()
                .0;
            fire_s += at.elapsed().as_secs_f64();
            count += 1;
        }
    }
    let mut bytes = 0;
    for f in repo.net_firings(&db, "night").unwrap() {
        bytes += repo
            .db_get(&db, &format!("net/night/fire/{}", f.key))
            .unwrap()
            .unwrap()
            .len();
    }
    let m = repo.net_marking(&db, "night").unwrap();
    let ok = repo.net_replay_versions(&db, "night").unwrap() == m;
    let versions: HashSet<Id> = repo
        .net_firings(&db, "night")
        .unwrap()
        .iter()
        .map(|f| f.net.unwrap())
        .collect();
    (
        fire_s * 1e6 / count as f64,
        bytes as f64 / count as f64,
        format!("{m:?}"),
        ok,
        versions.len(),
    )
}

fn main() {
    let mut args = std::env::args().skip(1);
    let sizes: Vec<usize> = args
        .next()
        .map(|s| s.split(',').map(|x| x.parse().unwrap()).collect())
        .unwrap_or_else(|| vec![10, 1000, 10000]);
    let candidates: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(300);

    let mut rows = Vec::new();
    for &n in &sizes {
        let nets = chain(n);
        let text = nets[0].to_text();
        eprintln!(
            "size {n}: {} places, {} text bytes, {} versions",
            nets[0].places.len(),
            text.len(),
            nets.len()
        );
        for (k, net) in nets.iter().enumerate().skip(1).take(5) {
            let mut x = nets[k - 1].clone();
            eprintln!("  v{}: {}", k + 1, edit(&mut x, k + 1));
            let _ = net;
        }
        for form in [Form::Text, Form::Graph, Form::Flat, Form::Tree] {
            let at = Instant::now();
            let row = measure(form, &nets, n <= 1000);
            eprintln!(
                "  {} done in {:.1} s",
                form.name(),
                at.elapsed().as_secs_f64()
            );
            rows.push(row);
        }
    }

    println!("## 1. stored bytes and reuse, {VERSIONS} versions\n");
    println!(
        "| transitions | form | bytes v1 | bytes a version adds (mean / max) | objects reused | bytes reused | after repack | heavy weight: v1, a version adds |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for r in &rows {
        let mean = r.added.iter().sum::<usize>() as f64 / r.added.len() as f64;
        let max = r.added.iter().max().unwrap();
        println!(
            "| {} | {} | {} | {:.0} / {} | {:.1}% | {:.1}% | {} | {}, {:.0} |",
            r.n,
            r.form.name(),
            r.first,
            mean,
            max,
            100.0 * r.reuse_objects.0 as f64 / r.reuse_objects.1 as f64,
            100.0 * r.reuse_bytes.0 as f64 / r.reuse_bytes.1 as f64,
            match r.repack {
                Some((b, s)) => format!("{b} B ({s:.1} s)"),
                None => "not run".into(),
            },
            r.heavy.0,
            r.heavy.1
        );
    }
    println!("\n## 2. time\n");
    println!(
        "| transitions | form | write µs a version | read v1 µs | read v{VERSIONS} µs | delta µs | delta chunks (unpruned) | delta fragments | v1 to v{VERSIONS} delta µs | applied exactly | fsck problems |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for r in &rows {
        let frag = matches!(r.form, Form::Flat | Form::Tree);
        println!(
            "| {} | {} | {:.0} | {:.1} | {:.1} | {:.1} | {} | {} | {:.1} | {} | {} |",
            r.n,
            r.form.name(),
            r.write_us,
            r.read_first_us,
            r.read_last_us,
            r.delta_us,
            match r.form {
                Form::Text => format!("{:.0} lines", r.delta_chunks),
                Form::Graph => "–".into(),
                _ => format!(
                    "{:.1} ({:.1}); v1-v{VERSIONS}: {}",
                    r.delta_chunks, r.unpruned_chunks, r.first_last_chunks
                ),
            },
            if frag {
                format!("{:.1}", r.delta_fragments)
            } else {
                "–".into()
            },
            r.first_last_us,
            if frag {
                format!("{}/{}", r.applied.0, r.applied.1)
            } else {
                "–".into()
            },
            r.fsck
        );
    }

    println!("\n## 3. firings: the night net edited 9 times, {candidates} candidates\n");
    println!("| tracked by | fire µs each | record bytes each | versions named | replay |");
    println!("| --- | --- | --- | --- | --- |");
    let mut markings = Vec::new();
    for (name, layout) in [
        ("text (E422)", None),
        ("fragments, flat", Some(Layout::Flat)),
        ("fragments, tree", Some(Layout::Tree)),
    ] {
        let (us, bytes, m, ok, versions) = firings(candidates, layout);
        println!(
            "| {name} | {us:.1} | {bytes:.1} | {versions} | {} |",
            if ok { "equal marking" } else { "FAILS" }
        );
        markings.push(m);
    }
    println!(
        "\n- final markings equal across the three: {}",
        markings.windows(2).all(|w| w[0] == w[1])
    );
}
