//! E447: space per object. Nets stored as fragments with each element's
//! bytes inline in the graph's leaf chunks (`Target::Inline`), measured
//! against E422's text and E446's fragment tree on E446's chain.
//!
//! 1. Where E446's bytes go: the first version at each size broken down
//!    by what holds them (leaf chunks, their pointer objects, parent
//!    chunks, element contents, trees), and a leaf chunk's raw bytes by
//!    field (labels, targets, edge keys, edge labels, counts).
//! 2. E446's chain (same nets, same 25 versions) in five forms: text,
//!    E446's tree, and three inline codecs (each step alone: the line
//!    inline with E446's edges; the next name in the payload instead of
//!    edges; the compact codec). Bytes v1, bytes a version adds at the
//!    default and the heavy read weight, reuse by element (in a chunk
//!    stored before, or in bytes a slice of earlier content covers), write,
//!    read and delta time (median of runs), deltas applied, fsck.
//! 3. Firings tracked by the inline layout replay to the same marking.
//!
//!   cargo run --release --example e447_net_inline [-- SIZES [CANDIDATES]]
//!
//! The net generator and edit chain are E446's (examples/e446_net_fragments.rs),
//! copied so the two examples build apart.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use ikam_kernel::net::{Arc, InlineCodec, Kind, Layout, Net, Place, Transition};
use ikam_kernel::{Arg, Graph, Id, MemStore, Node, Object, Repo, Store, Target};

const VERSIONS: usize = 25;
const PATH: &str = "nets/s.net";
const HEAVY_READ_WEIGHT: f64 = 48.8;

// ---- E446's generator and chain, unchanged ----

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

/// Median of repeated runs of `f`, in µs: at least 5 runs and 200 ms.
fn time<T>(mut f: impl FnMut() -> T) -> f64 {
    let (mut runs, mut total) = (Vec::new(), 0.0);
    while runs.len() < 5 || total < 0.2 {
        let at = Instant::now();
        std::hint::black_box(f());
        let t = at.elapsed().as_secs_f64();
        runs.push(t);
        total += t;
    }
    runs.sort_by(f64::total_cmp);
    runs[runs.len() / 2] * 1e6
}

// ---- forms ----

/// Chunk sizes for the inline forms (E447_CHUNK=min,max; default the
/// kernel's), and short derivation records (E447_SHORT=1, every form).
fn new_repo(form: Form) -> Repo<MemStore> {
    let mut repo = Repo::new(MemStore::default());
    if let Ok(v) = std::env::var("E447_CHUNK")
        && matches!(
            form,
            Form::Lines | Form::LinesNext | Form::Compact | Form::Flat
        )
    {
        let (a, b) = v.split_once(',').unwrap();
        repo.graph_chunk = (a.parse().unwrap(), b.parse().unwrap());
    }
    repo.short_records = std::env::var("E447_SHORT").is_ok();
    if std::env::var("E447_NOTPL").is_ok() {
        repo.object_templates = false;
    }
    repo
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    Text,
    Tree,
    Lines,
    LinesNext,
    Compact,
    Flat,
}

impl Form {
    fn name(self) -> &'static str {
        match self {
            Form::Text => "text (E422)",
            Form::Tree => "fragments, tree (E446)",
            Form::Lines => "inline: line + E446 edges",
            Form::LinesNext => "inline: line + next, no edges",
            Form::Compact => "inline: compact (Layout::Inline)",
            Form::Flat => "inline: compact, one chunk (InlineFlat)",
        }
    }
    fn fragments(self) -> bool {
        self != Form::Text
    }
}

fn put_version(repo: &mut Repo<MemStore>, form: Form, tree: &mut Option<Id>, net: &Net) -> Id {
    let (t, id) = match form {
        Form::Text => repo.put_net_file(*tree, PATH, net).unwrap(),
        Form::Tree => repo
            .put_net_fragment_file(*tree, PATH, net, Layout::Tree)
            .unwrap(),
        Form::Compact => repo
            .put_net_fragment_file(*tree, PATH, net, Layout::Inline)
            .unwrap(),
        Form::Flat => repo
            .put_net_fragment_file(*tree, PATH, net, Layout::InlineFlat)
            .unwrap(),
        Form::Lines | Form::LinesNext => {
            let codec = match form {
                Form::Lines => InlineCodec::Lines,
                _ => InlineCodec::LinesNext,
            };
            let id = repo.put_net_inline(net, codec).unwrap();
            let edit = ikam_kernel::Edit::PutGraph {
                path: PATH.into(),
                id,
            };
            (repo.edit_tree(*tree, &[edit]).unwrap(), id)
        }
    };
    *tree = Some(t);
    id
}

/// The content holding a graph chunk's encoding (`g 2 content[32]`).
fn chunk_content(repo: &Repo<MemStore>, id: &Id) -> Option<Id> {
    match repo.store().read(id).ok()?.as_slice() {
        [b'g', 2, c @ ..] => Some(Id::from_bytes(c.try_into().ok()?)),
        _ => None,
    }
}

/// Leaf chunk ids and parent chunk ids under `root`.
fn chunks(repo: &Repo<MemStore>, root: &Id) -> (Vec<Id>, Vec<Id>) {
    let (mut leaves, mut parents, mut stack) = (Vec::new(), Vec::new(), vec![*root]);
    while let Some(id) = stack.pop() {
        match repo.get(&id).unwrap() {
            Object::Graph(Graph::Children(_, cs)) => {
                parents.push(id);
                stack.extend(cs.into_iter().map(|c| c.1));
            }
            Object::Graph(Graph::Nodes(_)) => leaves.push(id),
            _ => panic!("not a graph"),
        }
    }
    (leaves, parents)
}

fn leaf_nodes(repo: &Repo<MemStore>, id: &Id) -> Vec<Node> {
    match repo.get(id).unwrap() {
        Object::Graph(Graph::Nodes(n)) => n,
        _ => unreachable!(),
    }
}

// ---- 1. byte breakdown ----

fn varint_len(mut n: usize) -> usize {
    let mut k = 1;
    while n >= 0x80 {
        n >>= 7;
        k += 1;
    }
    k
}

fn front_len(prev: &str, s: &str) -> (usize, usize) {
    let shared = prev
        .bytes()
        .zip(s.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    (
        varint_len(shared) + varint_len(s.len() - shared),
        s.len() - shared,
    )
}

#[derive(Default)]
struct Raw {
    labels: usize,
    target_tags: usize,
    target_ids: usize,
    payload: usize,
    edge_keys: usize,
    edge_labels: usize,
    counts: usize,
}

fn raw_fields(nodes: &[Node], raw: &mut Raw) {
    raw.counts += 2 + varint_len(nodes.len());
    let (mut prev, mut prev_edge) = ("", "");
    for n in nodes {
        let (c, b) = front_len(prev, &n.label);
        raw.counts += c;
        raw.labels += b;
        prev = &n.label;
        raw.target_tags += 1;
        match &n.target {
            Target::Content(Arg::Whole(_)) => raw.target_ids += 33,
            Target::Content(Arg::Range { .. }) => raw.target_ids += 49,
            Target::Object(_) => raw.target_ids += 32,
            Target::Inline(b) => {
                raw.counts += varint_len(b.len());
                raw.payload += b.len();
            }
            Target::None => {}
        }
        raw.counts += varint_len(n.edges.len());
        for e in &n.edges {
            raw.edge_keys += 8;
            let (c, b) = front_len(prev_edge, &e.label);
            raw.counts += c;
            raw.edge_labels += b;
            prev_edge = &e.label;
        }
    }
}

fn breakdown(form: Form, net: &Net) -> Vec<(&'static str, usize)> {
    let mut repo = new_repo(form);
    let mut tree = None;
    let root = put_version(&mut repo, form, &mut tree, net);
    let total = size(&repo);
    let stored = |id: &Id| repo.store().read(id).map_or(0, |b| b.len());
    // A content's stored bytes: its plain encoding, or its derivation
    // records and the contents they name that nothing else holds.
    let (leaves, parents) = chunks(&repo, &root);
    let (mut leaf_ptr, mut leaf_body, mut par) = (0, 0, 0);
    let mut elements = 0;
    let mut raw = Raw::default();
    let mut nelem = 0;
    for l in &leaves {
        leaf_ptr += stored(l);
        leaf_body += chunk_content(&repo, l).map_or(0, |c| stored(&c));
        let nodes = leaf_nodes(&repo, l);
        raw_fields(&nodes, &mut raw);
        for n in &nodes {
            nelem += 1;
            if let Target::Content(a) = &n.target {
                elements += stored(&a.id());
            }
        }
    }
    for p in &parents {
        par += stored(p) + chunk_content(&repo, p).map_or(0, |c| stored(&c));
    }
    let leaf_raw: usize = leaves
        .iter()
        .map(|l| repo.get(l).unwrap().encode().len())
        .sum();
    let rest = total - leaf_ptr - leaf_body - par - elements;
    vec![
        ("stored bytes, v1", total),
        ("elements (nodes)", nelem),
        ("leaf chunks", leaves.len()),
        ("parent chunks", parents.len()),
        ("element contents (an object each)", elements),
        ("leaf chunk bodies (stored compressed)", leaf_body),
        ("leaf chunk pointer objects (g 2 id)", leaf_ptr),
        ("parent chunks, pointer and body", par),
        ("trees and the rest", rest),
        ("leaf chunks raw, before compression", leaf_raw),
        ("  raw: labels", raw.labels),
        ("  raw: target tags", raw.target_tags),
        ("  raw: target ids (arg tag + 32)", raw.target_ids),
        ("  raw: inline payloads", raw.payload),
        ("  raw: edge keys (8 bytes)", raw.edge_keys),
        ("  raw: edge labels", raw.edge_labels),
        ("  raw: counts and lengths", raw.counts),
    ]
}

/// What the later versions add, by stored encoding's first byte: mean
/// bytes a version, over the chain at the default weight.
fn adds_by_kind(form: Form, nets: &[Net]) -> Vec<(String, f64)> {
    let mut repo = new_repo(form);
    let mut tree = None;
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    for (k, net) in nets.iter().enumerate() {
        let before: HashSet<Id> = repo.store().ids().unwrap().into_iter().collect();
        let root = put_version(&mut repo, form, &mut tree, net);
        if std::env::var("E447_DEBUG").is_ok() && form != Form::Text {
            let (leaves, parents) = chunks(&repo, &root);
            for l in leaves.iter().chain(&parents) {
                if before.contains(l) {
                    continue;
                }
                let c = chunk_content(&repo, l).unwrap();
                let first = match repo.get(l).unwrap() {
                    Object::Graph(Graph::Nodes(n)) => n[0].label.clone(),
                    _ => "(parent)".into(),
                };
                let stored = repo.store().read(&c).map(|e| (e[0] as char, e.len()));
                eprintln!(
                    "v{} {form:?} new chunk from {first}: raw {} stored {stored:?} derived {}",
                    k + 1,
                    repo.read_content(&c).unwrap().len(),
                    repo.derivations(&c).unwrap().len()
                );
                for d in repo.derivations(&c).unwrap() {
                    eprintln!("    func {} args {}", d.func, d.args.len());
                    for a in &d.args {
                        if let Arg::Whole(id) = a {
                            let st = repo.store().read(id).map(|e| (e[0] as char, e.len()));
                            eprintln!("    whole {id} stored {st:?}");
                        }
                        if let Arg::Range { id, start, len } = a {
                            let st = repo.store().read(id).map(|e| (e[0] as char, e.len()));
                            eprintln!("    arg {id} {start}+{len} stored {st:?}");
                        }
                    }
                }
            }
        }
        if k > 0 {
            for id in repo.store().ids().unwrap() {
                if before.contains(&id) {
                    continue;
                }
                let e = repo.store().read(&id).unwrap();
                let kind = match e[0] {
                    b'g' if e[1] == 2 => "graph chunk pointer (g 2)",
                    b't' if e[1] == 2 => "tree pointer (t 2)",
                    b'd' | b'D' => "derivation records",
                    b'B' | b'Z' | b'Y' | b'W' => "plain contents (bodies, literals)",
                    _ => "other",
                };
                *by.entry(kind.to_string()).or_default() += e.len();
                if std::env::var("E447_DEBUG").is_ok() {
                    eprintln!("v{} {form:?} {kind} {}", k + 1, e.len());
                }
            }
        }
        repo.commit("main", tree.unwrap(), "v").unwrap();
    }
    let e = (nets.len() - 1) as f64;
    by.into_iter().map(|(k, v)| (k, v as f64 / e)).collect()
}

// ---- 2. the chain ----

/// Byte spans of each node in a compact leaf chunk's encoding.
fn node_spans(encoded: &[u8]) -> Vec<(usize, usize)> {
    fn varint(b: &[u8], at: &mut usize) -> usize {
        let (mut v, mut shift) = (0usize, 0);
        loop {
            let x = b[*at];
            *at += 1;
            v |= ((x & 0x7f) as usize) << shift;
            if x < 0x80 {
                return v;
            }
            shift += 7;
        }
    }
    let mut at = 2;
    let n = varint(encoded, &mut at);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let start = at;
        varint(encoded, &mut at);
        at += varint(encoded, &mut at);
        let tag = encoded[at];
        at += 1;
        match tag {
            0 => {}
            1 => at += if encoded[at] == 0 { 33 } else { 49 },
            2 => at += 32,
            3 => at += varint(encoded, &mut at),
            _ => panic!("target tag"),
        }
        let m = varint(encoded, &mut at);
        for _ in 0..m {
            at += 8;
            varint(encoded, &mut at);
            at += varint(encoded, &mut at);
        }
        out.push((start, at));
    }
    out
}

/// Byte ranges of `content` its derivation takes from contents stored
/// before (`before`), if it is stored as one.
fn reused_ranges(repo: &Repo<MemStore>, content: &Id, before: &HashSet<Id>) -> Vec<(usize, usize)> {
    if repo.store().has(content) {
        return vec![];
    }
    let Some(d) = repo.derivations(content).unwrap().into_iter().next() else {
        return vec![];
    };
    let (mut out, mut at) = (Vec::new(), 0usize);
    for a in &d.args {
        let (id, len) = match a {
            Arg::Whole(id) => (*id, repo.read_content(id).unwrap().len()),
            Arg::Range { id, len, .. } => (*id, *len as usize),
        };
        if before.contains(&id) {
            out.push((at, at + len));
        }
        at += len;
    }
    out
}

struct Row {
    repack: Option<usize>,
    form: Form,
    n: usize,
    first: usize,
    added: Vec<usize>,
    heavy: (usize, f64),
    objects_reused: (usize, usize),
    elems_in_kept_chunks: (usize, usize),
    elems_physical: (usize, usize),
    write_us: f64,
    read_last_us: f64,
    delta_us: f64,
    applied: (usize, usize),
    fsck: usize,
}

fn heavy_bytes(form: Form, nets: &[Net]) -> (usize, f64) {
    let mut repo = new_repo(form);
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

fn measure(form: Form, nets: &[Net]) -> Row {
    let n = nets[0].transitions.len();
    let mut repo = new_repo(form);
    let (mut tree, mut commits, mut roots) = (None, Vec::new(), Vec::new());
    let (mut first, mut added, mut write_s) = (0, Vec::new(), 0.0);
    let mut seen_objects: HashSet<Id> = HashSet::new();
    let mut objects_reused = (0, 0);
    let (mut kept, mut physical) = ((0, 0), (0, 0));
    for (k, net) in nets.iter().enumerate() {
        let before_ids: HashSet<Id> = repo.store().ids().unwrap().into_iter().collect();
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
        commits.push(
            repo.commit("main", tree.unwrap(), &format!("v{}", k + 1))
                .unwrap(),
        );
        if form.fragments() {
            let (leaves, parents) = chunks(&repo, &root);
            let mut objs: Vec<Id> = leaves.iter().chain(&parents).copied().collect();
            for l in &leaves {
                let nodes = leaf_nodes(&repo, l);
                let reused_chunk = before_ids.contains(l);
                let spans = match (reused_chunk, chunk_content(&repo, l)) {
                    (false, Some(c)) => {
                        let ranges = reused_ranges(&repo, &c, &before_ids);
                        let enc = repo.get(l).unwrap().encode();
                        node_spans(&enc)
                            .into_iter()
                            .map(|(s, e)| ranges.iter().any(|&(a, b)| a <= s && e <= b))
                            .collect()
                    }
                    _ => vec![reused_chunk; nodes.len()],
                };
                for (node, covered) in nodes.iter().zip(spans) {
                    if let Target::Content(a) = &node.target {
                        objs.push(a.id());
                    }
                    if k > 0 {
                        // An element is reused when its bytes are: its own
                        // content (E446) or its chunk stored before, or
                        // a slice of earlier content covering it.
                        let own = match &node.target {
                            Target::Content(a) => before_ids.contains(&a.id()),
                            _ => false,
                        };
                        kept.1 += 1;
                        physical.1 += 1;
                        kept.0 += (reused_chunk || own) as usize;
                        physical.0 += (reused_chunk || own || covered) as usize;
                    }
                }
            }
            if k > 0 {
                for id in &objs {
                    objects_reused.1 += 1;
                    objects_reused.0 += seen_objects.contains(id) as usize;
                }
            }
            seen_objects.extend(objs);
        }
        roots.push(root);
    }
    let read = |repo: &Repo<MemStore>, k: usize| -> Net {
        match form {
            Form::Text => repo.net_file(&commits[k], PATH).unwrap(),
            _ => repo.net_fragment_file(&commits[k], PATH).unwrap(),
        }
    };
    let last = nets.len() - 1;
    for (k, net) in nets.iter().enumerate() {
        assert_eq!(&read(&repo, k), net, "{form:?} v{}", k + 1);
    }
    let read_last_us = time(|| read(&repo, last));
    let (mut delta_us, mut applied) = (0.0, (0, 0));
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
            _ => {
                delta_us += time(|| repo.net_delta(&roots[k - 1], &roots[k]).unwrap());
                let d = repo.net_delta(&roots[k - 1], &roots[k]).unwrap();
                applied.1 += 1;
                applied.0 += (d.apply(&nets[k - 1]).unwrap() == nets[k]) as usize;
            }
        }
    }
    if form.fragments() {
        let d = repo.net_delta(&roots[0], &roots[last]).unwrap();
        applied.1 += 1;
        applied.0 += (d.apply(&nets[0]).unwrap() == nets[last]) as usize;
    }
    let mut fsck = repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
    repo.gc().unwrap();
    for (k, net) in nets.iter().enumerate() {
        assert_eq!(&read(&repo, k), net, "after gc: {form:?} v{}", k + 1);
    }
    fsck += repo.fsck().unwrap().len() + repo.fsck_deep().unwrap().len();
    // Repack (level 19, groups, contexts) at the smaller sizes, as E446.
    let repack = (n <= 1000).then(|| {
        repo.repack().unwrap();
        for k in [0, last] {
            assert_eq!(read(&repo, k), nets[k]);
        }
        size(&repo)
    });
    Row {
        repack,
        form,
        n,
        first,
        added,
        heavy: heavy_bytes(form, nets),
        objects_reused,
        elems_in_kept_chunks: kept,
        elems_physical: physical,
        write_us: write_s * 1e6 / nets.len() as f64,
        read_last_us,
        delta_us: delta_us / (nets.len() - 1) as f64,
        applied,
        fsck,
    }
}

// ---- 3. firings ----

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

fn firings(n: usize, layout: Option<Layout>) -> (f64, String, bool, usize, usize) {
    let mut repo = new_repo(match layout {
        Some(Layout::Inline) => Form::Compact,
        _ => Form::Tree,
    });
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
        format!("{m:?}"),
        ok,
        versions.len(),
        size(&repo),
    )
}

fn main() {
    let mut args = std::env::args().skip(1);
    let sizes: Vec<usize> = args
        .next()
        .map(|s| s.split(',').map(|x| x.parse().unwrap()).collect())
        .unwrap_or_else(|| vec![1000, 10000]);
    let candidates: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(300);
    let forms = [
        Form::Text,
        Form::Tree,
        Form::Lines,
        Form::LinesNext,
        Form::Compact,
        Form::Flat,
    ];

    println!("## 1. where the first version's bytes go\n");
    for &n in &sizes {
        let net = synth(n);
        let text = net.to_text().len();
        println!("### {n} transitions (canonical text {text} bytes)\n");
        let cols: Vec<_> = [
            Form::Tree,
            Form::Lines,
            Form::LinesNext,
            Form::Compact,
            Form::Flat,
        ]
        .iter()
        .map(|f| (f.name(), breakdown(*f, &net)))
        .collect();
        print!("| part |");
        cols.iter().for_each(|c| print!(" {} |", c.0));
        print!("\n| --- |");
        cols.iter().for_each(|_| print!(" --- |"));
        println!();
        for i in 0..cols[0].1.len() {
            print!("| {} |", cols[0].1[i].0);
            cols.iter().for_each(|c| print!(" {} |", c.1[i].1));
            println!();
        }
        println!();
    }

    println!("### what a later version adds, mean bytes (default weight)\n");
    for &n in &sizes {
        let nets = chain(n);
        for form in [Form::Text, Form::Tree, Form::Compact, Form::Flat] {
            let parts: Vec<String> = adds_by_kind(form, &nets)
                .into_iter()
                .map(|(k, v)| format!("{k} {v:.0}"))
                .collect();
            println!("- {n}, {}: {}", form.name(), parts.join("; "));
        }
    }
    println!();

    let mut rows = Vec::new();
    let only_parts = std::env::var("E447_PARTS").is_ok();
    for &n in &sizes {
        if only_parts {
            break;
        }
        let nets = chain(n);
        for form in forms {
            let at = Instant::now();
            rows.push(measure(form, &nets));
            eprintln!(
                "{n}: {} done in {:.1} s",
                form.name(),
                at.elapsed().as_secs_f64()
            );
        }
    }
    println!("## 2. E446's chain, {VERSIONS} versions\n");
    println!(
        "| transitions | form | bytes v1 | x text | a version adds (mean / max) | heavy: v1, adds | objects reused | elements in chunks kept | elements reused (slices too) | 25 versions after repack |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    let text_of: HashMap<usize, usize> = rows
        .iter()
        .filter(|r| r.form == Form::Text)
        .map(|r| (r.n, r.first))
        .collect();
    let pct = |p: (usize, usize)| match p.1 {
        0 => "–".to_string(),
        _ => format!("{:.1}%", 100.0 * p.0 as f64 / p.1 as f64),
    };
    for r in &rows {
        let mean = r.added.iter().sum::<usize>() as f64 / r.added.len() as f64;
        println!(
            "| {} | {} | {} | {:.2} | {:.0} / {} | {}, {:.0} | {} | {} | {} | {} |",
            r.n,
            r.form.name(),
            r.first,
            r.first as f64 / text_of[&r.n] as f64,
            mean,
            r.added.iter().max().unwrap(),
            r.heavy.0,
            r.heavy.1,
            pct(r.objects_reused),
            pct(r.elems_in_kept_chunks),
            pct(r.elems_physical),
            r.repack.map_or("not run".into(), |b| b.to_string()),
        );
    }
    println!(
        "\n| transitions | form | write µs a version | read v{VERSIONS} µs | delta µs | applied exactly | fsck problems (before, after gc) |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for r in &rows {
        println!(
            "| {} | {} | {:.0} | {:.0} | {:.0} | {} | {} |",
            r.n,
            r.form.name(),
            r.write_us,
            r.read_last_us,
            r.delta_us,
            match r.form.fragments() {
                true => format!("{}/{}", r.applied.0, r.applied.1),
                false => "–".into(),
            },
            r.fsck
        );
    }

    println!("\n## 3. firings: the night net edited 9 times, {candidates} candidates\n");
    println!("| tracked by | fire µs each | versions named | store bytes | replay |");
    println!("| --- | --- | --- | --- | --- |");
    let mut markings = Vec::new();
    for (name, layout) in [
        ("text (E422)", None),
        ("fragments, tree (E446)", Some(Layout::Tree)),
        ("inline (E447)", Some(Layout::Inline)),
    ] {
        let (us, m, ok, versions, bytes) = firings(candidates, layout);
        println!(
            "| {name} | {us:.1} | {versions} | {bytes} | {} |",
            if ok { "equal marking" } else { "FAILS" }
        );
        markings.push(m);
    }
    println!(
        "\n- final markings equal across the three: {}",
        markings.windows(2).all(|w| w[0] == w[1])
    );
}
