//! E451: E449's read, write and firing paths over E447's inline form.
//! Every read path (full, fast, cached, threaded, by key, through
//! `net_version`) over every layout (Flat, Tree, Inline, InlineFlat) gives
//! the same nets; deltas are the same in every layout; delta writes give
//! the whole writes' roots; firings replay to the same marking; forged
//! inline nodes are refused by the fast read as by the full read; and the
//! graph layer's cached reads and hops treat inline payloads as contents.

use std::collections::{BTreeMap, BTreeSet};

use ikam_kernel::net::{Arc, Kind, Layout, Net, NetDelta, Place, Transition};
use ikam_kernel::{ChunkSizes, Edge, Id, MemStore, Node, Repo, Target, node_key};

const LAYOUTS: [Layout; 4] = [
    Layout::Flat,
    Layout::Tree,
    Layout::Inline,
    Layout::InlineFlat,
];

fn arc(place: &str, weight: u32) -> Arc {
    Arc {
        place: place.into(),
        weight,
    }
}

/// A synthetic net of `n` transitions, with the shapes a line can take:
/// colored places, initial tokens, weights, capabilities, a source and a
/// sink transition, names with `-`, `_` and `.`.
fn synth(n: usize) -> Net {
    let np = (n / 2).max(6);
    let kinds = [Kind::Pure, Kind::Cmd, Kind::Agent, Kind::Human];
    let name = |i: usize| match i % 3 {
        0 => format!("p{i}"),
        1 => format!("p-{i}."),
        _ => format!("p_{i}-"),
    };
    Net {
        name: "s-1.x".into(),
        version: 1,
        places: (0..np)
            .map(|i| Place {
                name: name(i),
                colored: i % 4 == 3,
                initial: if i % 4 == 3 { 0 } else { (i % 5) as u32 * 10 },
            })
            .collect(),
        transitions: (0..n)
            .map(|i| {
                let (a, b, c) = ((i * 7 + 1) % np, (i * 3 + 2) % np, (i * 5 + 4) % np);
                let kind = kinds[i % 4];
                let (inputs, outputs) = match i % 7 {
                    0 => (vec![], vec![arc(&name(b), 1)]),
                    1 => (vec![arc(&name(a), 3)], vec![]),
                    _ => (
                        vec![arc(&name(a), 1 + (i % 3) as u32), arc(&name(c), 1)],
                        vec![arc(&name(b), 1), arc(&name(c), 12)],
                    ),
                };
                Transition {
                    name: format!("t{i}"),
                    inputs,
                    outputs,
                    kind,
                    capability: (kind == Kind::Cmd).then(|| "run.x-1".to_string()),
                }
            })
            .collect(),
    }
}

/// Version `k`: rename, reweight, insert, remove a place, burst, move a
/// transition, change the net's name or nothing but the version.
fn edit(net: &mut Net, k: usize) {
    net.version = k as u32;
    let pick = |len: usize| (k * 7919) % len.max(1);
    match k % 7 {
        0 => {
            let j = pick(net.transitions.len());
            net.transitions[j].name += &format!("r{k}");
        }
        1 => {
            let j = pick(net.transitions.len());
            let t = &mut net.transitions[j];
            match t.outputs.first_mut() {
                Some(a) => a.weight += 1,
                None => t.inputs[0].weight += 1,
            }
        }
        2 => {
            let place = format!("q{k}");
            net.places.insert(
                pick(net.places.len() + 1),
                Place {
                    name: place.clone(),
                    colored: false,
                    initial: 0,
                },
            );
            let at = pick(net.transitions.len() + 1);
            net.transitions.insert(
                at,
                Transition {
                    name: format!("n{k}"),
                    inputs: vec![],
                    outputs: vec![arc(&place, 2)],
                    kind: Kind::Pure,
                    capability: None,
                },
            );
        }
        3 => {
            let gone = net.places.remove(1 + pick(net.places.len() - 1)).name;
            let keep = net.places[0].name.clone();
            for t in &mut net.transitions {
                for side in [&mut t.inputs, &mut t.outputs] {
                    for a in side.iter_mut().filter(|a| a.place == gone) {
                        a.place = keep.clone();
                    }
                }
            }
        }
        4 => {
            let m = (net.transitions.len() / 2).clamp(1, 20);
            let j = pick(net.transitions.len() - m + 1);
            for t in &mut net.transitions[j..j + m] {
                t.kind = match t.kind {
                    Kind::Human => Kind::Pure,
                    k => k,
                };
            }
            net.transitions[j].capability = None;
            net.transitions[j].kind = Kind::Agent;
        }
        5 => {
            // Move a transition (and the first place) to the end.
            let t = net.transitions.remove(pick(net.transitions.len()));
            net.transitions.push(t);
            let p = net.places.remove(0);
            net.places.push(p);
        }
        _ => {}
    }
    net.check().unwrap();
}

fn chain(n: usize, versions: usize) -> Vec<Net> {
    let mut nets = vec![synth(n)];
    for k in 2..=versions {
        let mut next = nets.last().unwrap().clone();
        edit(&mut next, k);
        nets.push(next);
    }
    nets
}

fn opts_repo() -> Repo<MemStore> {
    let mut repo = Repo::new(MemStore::default());
    repo.graph_cache = 1 << 12;
    repo.net_fast_read = true;
    repo.net_delta_write = true;
    repo.read_threads = 3;
    repo
}

#[test]
fn every_read_path_reads_every_layout() {
    let nets = chain(1500, 22);
    for layout in LAYOUTS {
        let mut plain = Repo::new(MemStore::default());
        let mut opts = opts_repo();
        for (k, net) in nets.iter().enumerate() {
            let root = plain.put_net_fragments(net, layout).unwrap();
            assert_eq!(opts.put_net_fragments(net, layout).unwrap(), root);
            if k == 0 && matches!(layout, Layout::Tree | Layout::Inline) {
                assert!(plain.graph_leaf_ids(&root).unwrap().len() > 2, "{layout:?}");
            }
            assert_eq!(&plain.net_fragments(&root).unwrap(), net, "{layout:?} {k}");
            assert_eq!(
                &plain
                    .net_fragments_fast(&root)
                    .unwrap_or_else(|e| panic!("{layout:?} {k} {e:?}")),
                net,
                "{layout:?} {k}"
            );
            assert_eq!(&opts.net_fragments(&root).unwrap(), net, "{layout:?} {k}");
            assert_eq!(&opts.net_fragments_full(&root).unwrap(), net);
            assert_eq!(&opts.net_fragments_par(&root).unwrap(), net);
            assert_eq!(&opts.net_version(&root).unwrap(), net);
            // Read by key: two transitions and their places.
            let names = [
                &net.transitions[3].name,
                &net.transitions[net.transitions.len() / 2].name,
            ];
            let names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            let part = opts.net_fragments_part(&root, &names).unwrap();
            assert_eq!(part.transitions.len(), 2);
            assert_eq!(part.transitions[0], *net.transition(names[0]).unwrap());
            for p in &part.places {
                assert_eq!(Some(p), net.place(&p.name), "{layout:?} {k}");
            }
            assert!(opts.net_fragments_part(&root, &["none-such"]).is_err());
            // Cold: the cache emptied.
            if k % 7 == 0 {
                opts.graph_cache_clear();
                assert_eq!(&opts.net_fragments(&root).unwrap(), net);
            }
        }
        assert!(opts.fsck().unwrap().is_empty() && opts.fsck_deep().unwrap().is_empty());
    }
}

/// A delta with what it counts left out: what changed, the same in every
/// layout.
fn content(mut d: NetDelta) -> NetDelta {
    d.chunks_read = 0;
    d.fragments_read = 0;
    d
}

#[test]
fn deltas_are_the_same_in_every_layout() {
    let nets = chain(800, 22);
    let mut repo = opts_repo();
    let roots: Vec<Vec<Id>> = LAYOUTS
        .iter()
        .map(|l| {
            nets.iter()
                .map(|n| repo.put_net_fragments(n, *l).unwrap())
                .collect()
        })
        .collect();
    for k in 1..nets.len() {
        let mut first = None;
        for (i, layout) in LAYOUTS.iter().enumerate() {
            let d = repo.net_delta(&roots[i][k - 1], &roots[i][k]).unwrap();
            assert_eq!(d.apply(&nets[k - 1]).unwrap(), nets[k], "{layout:?} {k}");
            let unpruned = repo
                .net_delta_with(&roots[i][k - 1], &roots[i][k], false)
                .unwrap();
            assert_eq!(content(unpruned), content(d.clone()));
            match &first {
                None => first = Some(content(d)),
                Some(f) => assert_eq!(&content(d), f, "{layout:?} {k}"),
            }
        }
    }
}

#[test]
fn delta_writes_give_the_whole_writes_roots_in_every_layout() {
    let nets = chain(1200, 26);
    for layout in LAYOUTS {
        let mut whole = Repo::new(MemStore::default());
        let mut delta = Repo::new(MemStore::default());
        delta.net_delta_write = true;
        for net in &nets {
            let a = whole.put_net_fragments(net, layout).unwrap();
            let b = delta.put_net_fragments(net, layout).unwrap();
            assert_eq!(a, b, "{layout:?} v{}", net.version);
            assert_eq!(&delta.net_fragments(&b).unwrap(), net);
            // A repeat writes nothing.
            let before = delta.bytes_written;
            assert_eq!(delta.put_net_fragments(net, layout).unwrap(), b);
            assert_eq!(delta.bytes_written, before);
        }
        assert_eq!(whole.bytes_written, delta.bytes_written, "{layout:?}");
        assert!(delta.fsck().unwrap().is_empty() && delta.fsck_deep().unwrap().is_empty());
        // An invalid net is refused, as by the whole write.
        let mut bad = nets.last().unwrap().clone();
        bad.transitions[0].inputs = vec![arc("nowhere", 1)];
        assert!(whole.put_net_fragments(&bad, layout).is_err());
        assert!(delta.put_net_fragments(&bad, layout).is_err());
    }
}

/// Found by running the existing tests with the options on: a delta
/// write kept the last root it wrote and reused or edited it after the
/// chunk bounds changed, so the root was not the whole write's under the
/// new bounds.
#[test]
fn a_delta_write_after_the_chunk_bounds_change_gives_the_whole_writes_root() {
    let nets = chain(1500, 4);
    for layout in [Layout::Tree, Layout::Inline] {
        let mut whole = Repo::new(MemStore::default());
        let mut delta = Repo::new(MemStore::default());
        delta.net_delta_write = true;
        for (i, net) in nets.iter().enumerate() {
            let bounds = if i % 2 == 0 {
                ChunkSizes::uniform(4096, 32768)
            } else {
                ChunkSizes::uniform(1024, 8192)
            };
            for _ in 0..2 {
                whole.graph_chunks = bounds;
                delta.graph_chunks = bounds;
                let a = whole.put_net_fragments(net, layout).unwrap();
                assert_eq!(
                    delta.put_net_fragments(net, layout).unwrap(),
                    a,
                    "{layout:?} v{i}"
                );
                // The same net again under the other bounds.
                let other = if bounds.leaf.0 == 4096 {
                    ChunkSizes::uniform(1024, 8192)
                } else {
                    ChunkSizes::uniform(4096, 32768)
                };
                whole.graph_chunks = other;
                delta.graph_chunks = other;
                let b = whole.put_net_fragments(net, layout).unwrap();
                assert_ne!(a, b);
                assert_eq!(
                    delta.put_net_fragments(net, layout).unwrap(),
                    b,
                    "{layout:?} v{i}"
                );
            }
        }
    }
}

const NIGHT: &str = "\
net night 1
place cpu 2
place queued colored
place running colored
place measured colored
transition propose agent: -> queued
transition start cmd: queued, cpu -> running
transition finish pure: running -> measured, cpu
";

/// The night net at version `k`, beside `big` synthetic transitions.
fn night(k: u32, big: usize) -> Net {
    let mut text = NIGHT.replace("net night 1", &format!("net night {k}"));
    if k > 1 {
        text += "place checked colored\n";
        text += &format!("transition check{k} pure: measured -> checked\n");
    }
    let mut net = Net::parse(&text).unwrap();
    let s = synth(big);
    if big > 0 {
        net.places.extend(s.places);
        net.transitions.extend(s.transitions);
    }
    net.check().unwrap();
    net
}

/// E422's run on a net tracked in `layout`, fired whole or by key, with
/// the options on or off: the firing records (version ids aside) and the
/// marking.
fn run(big: usize, layout: Layout, at: bool, opts: bool) -> (Vec<String>, String) {
    let mut repo = match opts {
        true => opts_repo(),
        false => Repo::new(MemStore::default()),
    };
    let mut cur = night(1, big);
    let mut db = repo.db_empty().unwrap();
    db = repo.net_init(&db, &cur).unwrap();
    db = repo.net_track_fragments(&db, &cur, layout).unwrap();
    let mut root = repo.put_net_fragments(&cur, layout).unwrap();
    let mut roots = vec![root];
    for i in 0..24 {
        if i > 0 && i % 6 == 0 {
            cur = night(cur.version + 1, big);
            root = repo.put_net_fragments(&cur, layout).unwrap();
            roots.push(root);
        }
        let c = format!("c{i}");
        let check = format!("check{}", cur.version);
        let mut steps = vec!["propose", "start", "finish"];
        if cur.version > 1 {
            steps.push(&check);
        }
        for (j, t) in steps.into_iter().enumerate() {
            let key = format!("k{i}-{j}");
            db = match at {
                true => repo.net_fire_at(&db, &root, t, Some(&c), &key),
                false => repo.net_fire(&db, &cur, t, Some(&c), &key),
            }
            .unwrap()
            .0;
        }
    }
    let m = repo.net_marking(&db, "night").unwrap();
    assert_eq!(repo.net_replay_versions(&db, "night").unwrap(), m);
    let firings = repo.net_firings(&db, "night").unwrap();
    // Every firing names a version this layout wrote.
    for f in &firings {
        assert!(roots.contains(&f.net.unwrap()), "{layout:?}");
    }
    let records = firings
        .iter()
        .map(|f| {
            let mut f = f.clone();
            f.net = None;
            format!("{f:?}")
        })
        .collect();
    (records, format!("{m:?}"))
}

#[test]
fn firings_replay_to_the_same_marking_in_every_layout() {
    for big in [0, 300] {
        let base = run(big, Layout::Tree, false, false);
        for layout in LAYOUTS {
            for (at, opts) in [(false, false), (false, true), (true, true), (true, false)] {
                let got = run(big, layout, at, opts);
                assert_eq!(got.1, base.1, "{layout:?} at={at} opts={opts} big={big}");
            }
        }
    }
}

/// An inline root whose nodes are `net`'s, changed by `f`.
fn forged(repo: &mut Repo<MemStore>, net: &Net, f: impl FnOnce(&mut Vec<Node>)) -> Id {
    let good = repo.put_net_fragments(net, Layout::Inline).unwrap();
    let mut nodes = repo.graph_nodes(&good).unwrap();
    f(&mut nodes);
    repo.put_graph(nodes).unwrap()
}

fn payload(nodes: &mut [Node], label: &str, bytes: &str) {
    let n = nodes.iter_mut().find(|n| n.label == label).unwrap();
    n.target = Target::Inline(bytes.as_bytes().to_vec());
}

#[test]
fn the_fast_read_refuses_forged_inline_nodes() {
    let net = Net::parse(
        "net n 1\nplace a 2\nplace b colored\nplace c\n\
         transition t cmd x: 2*a, b -> c\ntransition u pure: c ->\ntransition v human: -> a\n",
    )
    .unwrap();
    // The payloads as written, to forge from.
    let mut repo = opts_repo();
    let good = repo.put_net_fragments(&net, Layout::Inline).unwrap();
    let shown: BTreeMap<String, String> = repo
        .graph_nodes(&good)
        .unwrap()
        .into_iter()
        .map(|n| match n.target {
            Target::Inline(b) => (n.label, String::from_utf8(b).unwrap()),
            _ => panic!("not inline"),
        })
        .collect();
    assert_eq!(shown["t/t"], "c x:2*a,b>c\nu");
    assert_eq!(shown["p/a"], "2\nb");
    type Forge = Box<dyn FnOnce(&mut Vec<Node>)>;
    let cases: Vec<(&str, Forge)> = vec![
        (
            "a weight of 1 written",
            Box::new(|n| payload(n, "t/u", "p:1*c>\nv")),
        ),
        (
            "a weight of 0",
            Box::new(|n| payload(n, "t/u", "p:0*c>\nv")),
        ),
        (
            "a count of 0 written",
            Box::new(|n| payload(n, "p/c", "0\n")),
        ),
        ("a leading zero", Box::new(|n| payload(n, "p/a", "02\nb"))),
        (
            "a human with a capability",
            Box::new(|n| payload(n, "t/v", "h x:>a\n")),
        ),
        ("no arcs", Box::new(|n| payload(n, "t/v", "h:>\n"))),
        ("an unknown kind", Box::new(|n| payload(n, "t/v", "z:>a\n"))),
        (
            "an arc to no place",
            Box::new(|n| payload(n, "t/u", "p:zz>\nv")),
        ),
        (
            "an arc to a bad name",
            Box::new(|n| payload(n, "t/u", "p:c d>\nv")),
        ),
        ("a cycle", Box::new(|n| payload(n, "t/v", "h:>a\nt"))),
        (
            "a next to no node",
            Box::new(|n| payload(n, "t/v", "h:>a\nw")),
        ),
        (
            "one node in another codec",
            Box::new(|n| payload(n, "p/c", "place c\n")),
        ),
        (
            "one node with E446's edges",
            Box::new(|n| {
                payload(n, "p/a", "place a 2\n");
                let a = n.iter_mut().find(|n| n.label == "p/a").unwrap();
                a.edges = vec![Edge {
                    to: node_key("p/b"),
                    label: "next p/b".into(),
                }];
            }),
        ),
        (
            "a node no link reaches",
            Box::new(|n| {
                n.push(Node {
                    label: "p/d".into(),
                    target: Target::Inline(b"\n".to_vec()),
                    edges: vec![],
                })
            }),
        ),
        (
            "a bad place name",
            Box::new(|n| {
                n.iter_mut().find(|n| n.label == "p/c").unwrap().label = "p/c c".into();
                payload(n, "p/b", "c\nc c");
            }),
        ),
        (
            "a head with a leading zero",
            Box::new(|n| payload(n, "net", "@n 01\na\nt")),
        ),
        ("no head", Box::new(|n| n.retain(|n| n.label != "net"))),
    ];
    for (what, f) in cases {
        let mut repo = opts_repo();
        let root = forged(&mut repo, &net, f);
        assert!(
            repo.net_fragments_full(&root).is_err(),
            "full read took {what}"
        );
        assert!(
            repo.net_fragments_fast(&root).is_err(),
            "fast read took {what}"
        );
        repo.graph_cache_clear();
        assert!(
            repo.net_fragments_par(&root).is_err(),
            "par read took {what}"
        );
    }
    // A transition read by key refuses a forged node it reads, and the
    // canonical net is read.
    let mut repo = opts_repo();
    let root = forged(&mut repo, &net, |n| payload(n, "t/u", "p:1*c>\nv"));
    assert!(repo.net_fragments_part(&root, &["u"]).is_err());
    assert!(repo.net_fragments_part(&root, &["t"]).is_ok());
    let root = forged(&mut repo, &net, |n| payload(n, "p/c", "place c\n"));
    assert!(repo.net_fragments_part(&root, &["t"]).is_err());
    assert_eq!(repo.net_fragments(&good).unwrap(), net);
}

/// A small general graph: label to (payload, links), a payload a few
/// bytes to a few hundred.
fn general(n: usize) -> BTreeMap<String, (String, BTreeSet<String>)> {
    (0..n)
        .map(|i| {
            let to = [(i * 7 + 3) % n, (i * i + 1) % n, i / 2]
                .iter()
                .map(|j| format!("n{j:05}"))
                .collect();
            let p = format!("payload {i} {}", "x".repeat((i * 37) % 300));
            (format!("n{i:05}"), (p, to))
        })
        .collect()
}

#[test]
fn graph_reads_and_hops_treat_inline_payloads_as_contents() {
    let g = general(3000);
    let mut repo = opts_repo();
    let mut nodes = Vec::new();
    for (l, (p, to)) in &g {
        nodes.push(Node {
            label: l.clone(),
            target: repo.put_payload(p.as_bytes(), 120).unwrap(),
            edges: to
                .iter()
                .map(|t| Edge {
                    to: node_key(t),
                    label: format!("link {t}"),
                })
                .collect(),
        });
    }
    let root = repo.put_graph(nodes).unwrap();
    let plain = repo.graph_nodes(&root).unwrap();
    assert!(plain.iter().any(|n| matches!(n.target, Target::Inline(_))));
    assert!(plain.iter().any(|n| !matches!(n.target, Target::Inline(_))));
    for pass in 0..2 {
        let leaves = match pass {
            0 => repo.graph_read(&root).unwrap(),
            _ => repo.graph_read_par(&root).unwrap(),
        };
        let mut read = BTreeMap::new();
        for leaf in &leaves {
            for (n, bytes) in &leaf.nodes {
                read.insert(
                    n.label.clone(),
                    String::from_utf8(bytes.clone().unwrap()).unwrap(),
                );
            }
        }
        let want: BTreeMap<String, String> =
            g.iter().map(|(l, (p, _))| (l.clone(), p.clone())).collect();
        assert_eq!(read, want);
        repo.graph_cache_clear();
    }
    // Hops equal an in-memory walk.
    let start = vec!["n00042".to_string(), "n02999".to_string()];
    let hops = repo
        .graph_hops(&root, &start, 2, |e| {
            e.label.strip_prefix("link ").map(str::to_string)
        })
        .unwrap();
    let mut want = BTreeSet::new();
    let mut frontier: BTreeSet<String> = start.iter().cloned().collect();
    for hop in 0..=2 {
        let mut next = BTreeSet::new();
        for l in &frontier {
            want.insert(l.clone());
            if hop < 2 {
                next.extend(g[l].1.iter().cloned());
            }
        }
        frontier = next.into_iter().filter(|l| !want.contains(l)).collect();
    }
    assert_eq!(hops.keys().cloned().collect::<BTreeSet<_>>(), want);
    for (l, n) in &hops {
        assert_eq!(repo.graph_node_cached(&root, l).unwrap().as_ref(), Some(n));
        assert_eq!(plain.iter().find(|p| &p.label == l), Some(n));
    }
}

/// E448's frames and chunk bounds under E447's inline form and E449's
/// options: the same roots as the plain whole write, every read path
/// equal, every delta exact, fsck clean, and gc keeps what commits hold.
#[test]
fn frames_and_bounds_with_inline_and_the_e449_options() {
    let nets = chain(1200, 16);
    let bounds = ikam_kernel::ChunkSizes {
        leaf: (4096, 32768),
        parent: (16384, 131072),
    };
    for layout in [Layout::Tree, Layout::Inline, Layout::InlineFlat] {
        let mut whole = Repo::new(MemStore::default());
        whole.graph_chunks = bounds;
        let mut repo = opts_repo();
        repo.object_contexts = true;
        repo.graph_chunks = bounds;
        let (mut tree, mut commits, mut roots) = (None, vec![], vec![]);
        for net in &nets {
            let (t, root) = repo
                .put_net_fragment_file(tree, "nets/s.net", net, layout)
                .unwrap();
            assert_eq!(whole.put_net_fragments(net, layout).unwrap(), root);
            tree = Some(t);
            commits.push(repo.commit("main", t, "v").unwrap());
            roots.push(root);
        }
        for k in 1..nets.len() {
            let d = repo.net_delta(&roots[k - 1], &roots[k]).unwrap();
            assert_eq!(d.apply(&nets[k - 1]).unwrap(), nets[k], "{layout:?} {k}");
        }
        repo.gc().unwrap();
        repo.graph_cache_clear();
        for (k, net) in nets.iter().enumerate() {
            assert_eq!(
                &repo.net_fragment_file(&commits[k], "nets/s.net").unwrap(),
                net
            );
            assert_eq!(&repo.net_fragments_full(&roots[k]).unwrap(), net);
            assert_eq!(&repo.net_fragments_par(&roots[k]).unwrap(), net);
        }
        assert!(repo.fsck().unwrap().is_empty() && repo.fsck_deep().unwrap().is_empty());
    }
}

/// The two chunk-size knobs of E447 and E448 are one (E451): the default
/// is the fixed rule, and E447's `(min, max)` is the uniform form.
#[test]
fn one_chunk_size_knob() {
    let repo = Repo::new(MemStore::default());
    assert_eq!(repo.graph_chunks, ikam_kernel::ChunkSizes::default());
    assert_eq!(
        repo.graph_chunks,
        ikam_kernel::ChunkSizes::uniform(4096, 32768)
    );
}
