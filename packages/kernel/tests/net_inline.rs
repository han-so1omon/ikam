//! E447: nets and graphs with their elements' bytes inline in the leaf
//! chunks (`Target::Inline`, `Layout::Inline`, `InlineFlat`), so no element
//! is an object of its own. They read back, chain, delta and replay as
//! E446's fragments do, and default stored bytes do not change.

use std::collections::BTreeMap;

use ikam_kernel::net::{Arc, InlineCodec, Kind, Layout, Net, Place, Transition};
use ikam_kernel::{Db, Edge, Edit, Graph, MemStore, Node, Object, Repo, Store, Target, node_key};

const CODECS: [InlineCodec; 3] = [
    InlineCodec::Lines,
    InlineCodec::LinesNext,
    InlineCodec::Compact,
];

fn arc(place: &str, weight: u32) -> Arc {
    Arc {
        place: place.into(),
        weight,
    }
}

/// A synthetic net of `n` transitions over `n / 2` places (at least 4).
fn synth(n: usize) -> Net {
    let np = (n / 2).max(4);
    let kinds = [Kind::Pure, Kind::Cmd, Kind::Agent, Kind::Human];
    Net {
        name: "s".into(),
        version: 1,
        places: (0..np)
            .map(|i| Place {
                name: format!("p{i}"),
                colored: false,
                initial: (i % 5 == 0) as u32 * 2,
            })
            .collect(),
        transitions: (0..n)
            .map(|i| {
                let (a, b) = ((i * 7 + 1) % np, (i * 3 + 2) % np);
                Transition {
                    name: format!("t{i}"),
                    inputs: vec![arc(&format!("p{a}"), 1 + (i % 3) as u32)],
                    outputs: vec![arc(&format!("p{b}"), 1)],
                    kind: kinds[i % 4],
                    capability: None,
                }
            })
            .collect(),
    }
}

/// Version `k` of a chain: rename, reweight, insert, remove a place, burst.
fn edit(net: &mut Net, k: usize) {
    net.version = k as u32;
    let pick = |len: usize| (k * 7919) % len.max(1);
    match k % 5 {
        0 => {
            let j = pick(net.transitions.len());
            net.transitions[j].name += &format!("r{k}");
        }
        1 => {
            let j = pick(net.transitions.len());
            net.transitions[j].outputs[0].weight += 1;
        }
        2 => {
            let at = pick(net.transitions.len() + 1);
            let place = format!("q{k}");
            net.places.insert(
                pick(net.places.len() + 1),
                Place {
                    name: place.clone(),
                    colored: false,
                    initial: 0,
                },
            );
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
        _ => {
            let m = (net.transitions.len() / 2).clamp(1, 20);
            let j = pick(net.transitions.len() - m + 1);
            for t in &mut net.transitions[j..j + m] {
                t.outputs[0].weight = t.outputs[0].weight % 4 + 1;
            }
        }
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

fn night(k: u32) -> Net {
    let mut text = NIGHT.replace("net night 1", &format!("net night {k}"));
    if k > 1 {
        text += "place checked colored\n";
        text += &format!("transition check{k} pure: measured -> checked\n");
    }
    Net::parse(&text).unwrap()
}

/// The night net run for `n` candidates, edited every few, tracked by text
/// (None) or by fragments.
fn run(n: usize, layout: Option<Layout>) -> (Repo<MemStore>, Db) {
    let mut repo = Repo::new(MemStore::default());
    let mut cur = night(1);
    let db = repo.db_empty().unwrap();
    let db = repo.net_init(&db, &cur).unwrap();
    let mut db = match layout {
        None => repo.net_track_versions(&db, &cur).unwrap(),
        Some(l) => repo.net_track_fragments(&db, &cur, l).unwrap(),
    };
    for i in 0..n {
        if i > 0 && i % 4 == 0 {
            cur = night(cur.version + 1);
        }
        let tok = format!("c{i}");
        let check = format!("check{}", cur.version);
        let mut steps = vec!["propose", "start", "finish"];
        if cur.version > 1 {
            steps.push(&check);
        }
        for (j, t) in steps.iter().enumerate() {
            db = repo
                .net_fire(&db, &cur, t, Some(&tok), &format!("{i}.{j}"))
                .unwrap()
                .0;
        }
    }
    (repo, db)
}

/// A net with every kind of line: capabilities, colored and counted
/// places, weights, empty sides.
fn odd() -> Net {
    Net::parse(
        "net odd 7\nplace a 3\nplace b colored\nplace c\n\
         transition x cmd bench: 2*a, c -> b\ntransition y human: -> c\n\
         transition z agent: b ->\ntransition w pure: a -> 3*c, b\n",
    )
    .unwrap()
}

#[test]
fn inline_nets_read_back_in_every_codec() {
    let mut repo = Repo::new(MemStore::default());
    for net in [odd(), synth(1), synth(10), synth(2000), night(3)] {
        for codec in CODECS {
            let root = repo.put_net_inline(&net, codec).unwrap();
            assert_eq!(repo.net_fragments(&root).unwrap(), net, "{codec:?}");
            assert_eq!(repo.net_version(&root).unwrap(), net);
            assert_eq!(repo.net_fragments(&root).unwrap().to_text(), net.to_text());
        }
        let tree = repo.put_net_fragments(&net, Layout::Inline).unwrap();
        assert_eq!(
            tree,
            repo.put_net_inline(&net, InlineCodec::Compact).unwrap()
        );
        let flat = repo.put_net_fragments(&net, Layout::InlineFlat).unwrap();
        assert_eq!(repo.net_fragments(&flat).unwrap(), net);
        // No element is an object: the store holds chunks, not lines.
        for n in repo.graph_nodes(&tree).unwrap() {
            assert!(matches!(n.target, Target::Inline(_)) && n.edges.is_empty());
        }
    }
    // The same net always has the same root.
    let net = synth(500);
    let a = repo.put_net_fragments(&net, Layout::Inline).unwrap();
    let mut fresh = Repo::new(MemStore::default());
    assert_eq!(fresh.put_net_fragments(&net, Layout::Inline).unwrap(), a);
    assert!(repo.fsck().unwrap().is_empty() && repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn inline_versions_chain_delta_exactly_and_survive_gc() {
    let nets = chain(600, 16);
    for layout in [Layout::Inline, Layout::InlineFlat] {
        let mut repo = Repo::new(MemStore::default());
        let (mut tree, mut commits, mut roots) = (None, Vec::new(), Vec::new());
        for net in &nets {
            let (t, id) = repo
                .put_net_fragment_file(tree, "n.net", net, layout)
                .unwrap();
            tree = Some(t);
            commits.push(repo.commit("main", t, "v").unwrap());
            roots.push(id);
        }
        for k in 1..nets.len() {
            let d = repo.net_delta(&roots[k - 1], &roots[k]).unwrap();
            assert!(!d.is_empty());
            assert_eq!(d.apply(&nets[k - 1]).unwrap(), nets[k], "{layout:?} v{k}");
            let full = repo
                .net_delta_with(&roots[k - 1], &roots[k], false)
                .unwrap();
            assert_eq!(
                (&d.places, &d.transitions, &d.links),
                (&full.places, &full.transitions, &full.links)
            );
        }
        let d = repo.net_delta(&roots[0], roots.last().unwrap()).unwrap();
        assert_eq!(d.apply(&nets[0]).unwrap(), *nets.last().unwrap());
        assert!(repo.net_delta(&roots[3], &roots[3]).unwrap().is_empty());
        repo.gc().unwrap();
        for (c, net) in commits.iter().zip(&nets) {
            assert_eq!(&repo.net_fragment_file(c, "n.net").unwrap(), net);
        }
        assert!(repo.fsck().unwrap().is_empty() && repo.fsck_deep().unwrap().is_empty());
    }
}

#[test]
fn an_inline_edit_rewrites_only_the_chunks_it_falls_in() {
    let mut repo = Repo::new(MemStore::default());
    let mut net = synth(4000);
    let a = repo.put_net_fragments(&net, Layout::Inline).unwrap();
    let leaves = |repo: &Repo<MemStore>, root| -> Vec<ikam_kernel::Id> {
        match repo.get(root).unwrap() {
            Object::Graph(Graph::Children(_, cs)) => cs.into_iter().map(|c| c.1).collect(),
            _ => panic!("a tree of one level"),
        }
    };
    net.transitions[1234].outputs[0].weight += 1;
    let b = repo.put_net_fragments(&net, Layout::Inline).unwrap();
    let (la, lb) = (leaves(&repo, &a), leaves(&repo, &b));
    assert!(la.len() > 10);
    let new = lb.iter().filter(|id| !la.contains(id)).count();
    assert!(
        new <= 2,
        "one weight: its leaf rewritten (and at most the next, should a cut move), every other kept: {new}"
    );
    let d = repo.net_delta(&a, &b).unwrap();
    assert_eq!(d.transitions.len(), 1);
    assert_eq!(d.arcs.len(), 1);
    assert!(d.chunks_read <= 6, "{}", d.chunks_read);
}

#[test]
fn an_inline_root_that_is_not_the_nets_own_nodes_is_refused() {
    let mut repo = Repo::new(MemStore::default());
    let net = synth(10);
    let good = repo.put_net_fragments(&net, Layout::Inline).unwrap();
    let nodes = repo.graph_nodes(&good).unwrap();
    let with = |f: &dyn Fn(&mut Vec<Node>)| {
        let mut n = nodes.clone();
        f(&mut n);
        n
    };
    let set = |label: &'static str, payload: &'static str| {
        move |n: &mut Vec<Node>| {
            n.iter_mut().find(|n| n.label == label).unwrap().target =
                Target::Inline(payload.as_bytes().to_vec())
        }
    };
    let bad = [
        // The order stops short of the last transition.
        with(&set("t/t4", "c:p1>p2\n")),
        // A link to no element.
        with(&set("t/t4", "c:p1>p2\nt99")),
        // A kind letter that is no kind.
        with(&set("t/t4", "x:p1>p2\nt5")),
        // The same transition, written as a line: not how the codec writes it.
        with(&set("t/t0", "transition t0 pure: 1*p1 -> p2\nt1")),
        // An edge on an inline node.
        with(&|n: &mut Vec<Node>| {
            n.iter_mut()
                .find(|n| n.label == "p/p1")
                .unwrap()
                .edges
                .push(Edge {
                    to: node_key("p/p2"),
                    label: "next p/p2".into(),
                })
        }),
    ];
    for nodes in bad {
        let root = repo.put_graph(nodes).unwrap();
        assert!(repo.net_fragments(&root).is_err());
    }
}

#[test]
fn firings_tracked_inline_replay_like_text_ones() {
    let (text_repo, text_db) = run(20, None);
    let want = text_repo.net_marking(&text_db, "night").unwrap();
    for layout in [Layout::Inline, Layout::InlineFlat] {
        let (mut repo, db) = run(20, Some(layout));
        let m = repo.net_marking(&db, "night").unwrap();
        assert_eq!(m, want);
        assert_eq!(repo.net_replay_versions(&db, "night").unwrap(), m);
        for f in repo.net_firings(&db, "night").unwrap() {
            let root = f.net.unwrap();
            let net = repo.net_version(&root).unwrap();
            assert_eq!(repo.put_net_fragments(&net, layout).unwrap(), root);
        }
        let before = repo.store().ids().unwrap().len();
        repo.put_net_fragments(&night(5), layout).unwrap();
        assert_eq!(repo.store().ids().unwrap().len(), before);
    }
}

fn inline_node(label: &str, payload: &[u8], to: &[&str]) -> Node {
    let mut edges: Vec<Edge> = to
        .iter()
        .map(|t| Edge {
            to: node_key(t),
            label: "link".into(),
        })
        .collect();
    edges.sort();
    Node {
        label: label.into(),
        target: Target::Inline(payload.to_vec()),
        edges,
    }
}

#[test]
fn inline_targets_encode_decode_look_up_and_edit_like_any_node() {
    let mut repo = Repo::new(MemStore::default());
    let mut nodes: Vec<Node> = (0..3000)
        .map(|i| {
            let label = format!("n{i:05}");
            let payload = format!("payload of {i} {}", "x".repeat(i % 50));
            let to = format!("n{:05}", (i * 7) % 3000);
            inline_node(&label, payload.as_bytes(), &[&to])
        })
        .collect();
    nodes.push(inline_node("empty", b"", &[]));
    nodes.push(inline_node("bytes", &[0, 255, 3, 10], &[]));
    let root = repo.put_graph(nodes.clone()).unwrap();
    let mut sorted = nodes.clone();
    sorted.sort_by(|a, b| a.label.cmp(&b.label));
    assert_eq!(repo.graph_nodes(&root).unwrap(), sorted);
    // A chunk's encoding decodes to itself and encodes back the same.
    for n in sorted.chunks(7) {
        let g = Object::Graph(Graph::Nodes(n.to_vec()));
        assert_eq!(Object::decode(&g.encode()).unwrap(), g);
    }
    // Lookups that scan a chunk build inline nodes too.
    assert_eq!(
        repo.graph_node(&root, "n01234").unwrap().as_ref(),
        sorted.iter().find(|n| n.label == "n01234")
    );
    let want: std::collections::BTreeSet<String> = ["bytes", "n00007", "n02999"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(repo.graph_nodes_in(&root, &want).unwrap().len(), 3);
    // An edit equals building the edited set whole.
    let mut edits = BTreeMap::new();
    edits.insert(
        "n00100".to_string(),
        Some(inline_node("n00100", b"changed", &["n00001"])),
    );
    edits.insert("n00200".to_string(), None);
    edits.insert("zz".to_string(), Some(inline_node("zz", b"new", &[])));
    let edited = repo.graph_edit(&root, edits.clone()).unwrap();
    let mut whole: BTreeMap<String, Node> =
        sorted.into_iter().map(|n| (n.label.clone(), n)).collect();
    for (l, e) in edits {
        match e {
            Some(n) => whole.insert(l, n),
            None => whole.remove(&l),
        };
    }
    assert_eq!(
        repo.put_graph(whole.into_values().collect()).unwrap(),
        edited
    );
    assert!(repo.fsck().unwrap().is_empty() && repo.fsck_deep().unwrap().is_empty());
    // gc keeps what a commit holds: every chunk, so every inline payload.
    let edit = Edit::PutGraph {
        path: "g".into(),
        id: edited,
    };
    let tree = repo.edit_tree(None, &[edit]).unwrap();
    repo.commit("main", tree, "graph").unwrap();
    assert!(repo.gc().unwrap() > 0, "the first version's chunks go");
    assert!(repo.graph_nodes(&root).is_err());
    assert_eq!(repo.graph_nodes(&edited).unwrap().len(), 3002);
    assert_eq!(
        repo.graph_node(&edited, "zz").unwrap().unwrap().target,
        Target::Inline(b"new".to_vec())
    );
}

#[test]
fn graphs_without_inline_nodes_keep_their_bytes() {
    // The encoding of a chunk with every older target kind, pinned: the
    // inline tag adds a kind and changes none.
    let id = ikam_kernel::Id::of_content(b"x");
    let g = Object::Graph(Graph::Nodes(vec![
        Node {
            label: "a".into(),
            target: Target::None,
            edges: vec![Edge {
                to: node_key("b"),
                label: "e".into(),
            }],
        },
        Node {
            label: "b".into(),
            target: Target::Content(ikam_kernel::Arg::Whole(id)),
            edges: vec![],
        },
    ]));
    let mut want = vec![b'G', 0x80, 2, 0, 1, b'a', 0, 1];
    want.extend_from_slice(&node_key("b"));
    want.extend_from_slice(&[0, 1, b'e', 0, 1, b'b', 1, 0]);
    want.extend_from_slice(id.as_bytes());
    want.push(0);
    assert_eq!(g.encode(), want);
    // The default chunk sizes are the ones every graph was cut with.
    let repo = Repo::new(MemStore::default());
    assert_eq!(repo.graph_chunk, (4096, 32768));
}

#[test]
fn other_chunk_sizes_give_valid_graphs_that_edit_consistently() {
    let mut repo = Repo::new(MemStore::default());
    let net = synth(3000);
    let small = repo.put_net_fragments(&net, Layout::Inline).unwrap();
    repo.graph_chunk = (16384, 131072);
    let big = repo.put_net_fragments(&net, Layout::Inline).unwrap();
    assert_ne!(small, big);
    assert_eq!(repo.net_fragments(&big).unwrap(), net);
    assert!(repo.graph_nodes(&big).unwrap() == repo.graph_nodes(&small).unwrap());
    // graph_edit with the same sizes gives put_graph's root.
    let nodes = repo.graph_nodes(&big).unwrap();
    let mut edits = BTreeMap::new();
    let mut n = nodes[1700].clone();
    n.target = Target::Inline(b"c:p1>p2\nt1".to_vec());
    edits.insert(n.label.clone(), Some(n.clone()));
    let edited = repo.graph_edit(&big, edits).unwrap();
    let mut all = nodes;
    all[1700] = n;
    assert_eq!(repo.put_graph(all).unwrap(), edited);
}

#[test]
fn an_inline_net_sits_in_a_tree_beside_files() {
    let mut repo = Repo::new(MemStore::default());
    let net = odd();
    let (tree, _) = repo
        .put_net_fragment_file(None, "nets/odd.net", &net, Layout::Inline)
        .unwrap();
    let id = repo.put_content(b"hi").unwrap().id;
    let tree = repo
        .edit_tree(
            Some(tree),
            &[Edit::Put {
                path: "README".into(),
                id,
            }],
        )
        .unwrap();
    let c = repo.commit("main", tree, "both").unwrap();
    assert_eq!(repo.net_fragment_file(&c, "nets/odd.net").unwrap(), net);
}
