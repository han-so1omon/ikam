//! E449: graph and net reads that pay for what changed. Decoded chunks kept
//! across versions, a fast read of fragment nets that gives exactly what
//! the full read gives and refuses what it refuses, writes against the last
//! version that give the same roots, and firings that read one transition
//! by key.

use std::collections::{BTreeMap, BTreeSet};

use ikam_kernel::net::{Arc, Kind, Layout, Net, Place, Transition};
use ikam_kernel::{Arg, Edge, Id, MemStore, Node, Repo, SharedStore, Store, Target, node_key};

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

fn fast_repo() -> Repo<MemStore> {
    let mut repo = Repo::new(MemStore::default());
    repo.graph_cache = 1 << 12;
    repo.net_fast_read = true;
    repo
}

#[test]
fn the_fast_read_gives_what_the_full_read_gives() {
    let nets = chain(600, 22);
    let mut repo = fast_repo();
    repo.read_threads = 3;
    let mut roots = Vec::new();
    for net in &nets {
        roots.push(repo.put_net_fragments(net, Layout::Tree).unwrap());
    }
    roots.push(repo.put_net_fragments(&synth(3), Layout::Flat).unwrap());
    let mut want = nets.clone();
    want.push(synth(3));
    for (root, net) in roots.iter().zip(&want) {
        assert_eq!(&repo.net_fragments_full(root).unwrap(), net);
        assert_eq!(&repo.net_fragments_fast(root).unwrap(), net);
        assert_eq!(&repo.net_fragments_par(root).unwrap(), net);
        assert_eq!(&repo.net_version(root).unwrap(), net);
    }
    // And with nothing kept.
    repo.graph_cache = 0;
    for (root, net) in roots.iter().zip(&want) {
        assert_eq!(&repo.net_fragments_fast(root).unwrap(), net);
        assert_eq!(&repo.net_fragments_par(root).unwrap(), net);
    }
}

#[test]
fn reading_the_next_version_decodes_only_the_chunks_it_wrote() {
    let nets = chain(3000, 8);
    let mut repo = fast_repo();
    let roots: Vec<Id> = nets
        .iter()
        .map(|n| repo.put_net_fragments(n, Layout::Tree).unwrap())
        .collect();
    let leaves = repo.graph_leaf_ids(&roots[0]).unwrap().len();
    assert!(leaves > 20, "{leaves} leaves");
    repo.graph_cache_clear();
    assert_eq!(repo.net_fragments(&roots[0]).unwrap(), nets[0]);
    for k in 1..nets.len() {
        let (_, before) = repo.graph_cache_stats();
        assert_eq!(repo.net_fragments(&roots[k]).unwrap(), nets[k]);
        let (_, after) = repo.graph_cache_stats();
        // A few leaves and the path above them, not the whole net.
        assert!(
            after - before <= 12,
            "v{}: {} decoded",
            k + 1,
            after - before
        );
    }
    // A kept chunk is never a stale one: a net read after its neighbour's
    // versions is still itself.
    for k in (0..nets.len()).rev() {
        assert_eq!(repo.net_fragments(&roots[k]).unwrap(), nets[k]);
    }
}

#[test]
fn delta_writes_give_the_whole_writes_roots() {
    let nets = chain(2000, 26);
    let mut whole = Repo::new(MemStore::default());
    let mut delta = fast_repo();
    delta.net_delta_write = true;
    let (mut tw, mut td) = (None, None);
    let mut commits = Vec::new();
    for net in &nets {
        let (t, a) = whole
            .put_net_fragment_file(tw, "nets/s.net", net, Layout::Tree)
            .unwrap();
        tw = Some(t);
        let (t, b) = delta
            .put_net_fragment_file(td, "nets/s.net", net, Layout::Tree)
            .unwrap();
        td = Some(t);
        assert_eq!(a, b, "v{}", net.version);
        commits.push(delta.commit("main", t, "v").unwrap());
        // Writing the same version again writes nothing.
        let n = delta.store().ids().unwrap().len();
        assert_eq!(delta.put_net_fragments(net, Layout::Tree).unwrap(), b);
        assert_eq!(delta.store().ids().unwrap().len(), n);
    }
    // The flat layout is written whole, the same as without the option.
    let flat = whole.put_net_fragments(&nets[3], Layout::Flat).unwrap();
    assert_eq!(
        delta.put_net_fragments(&nets[3], Layout::Flat).unwrap(),
        flat
    );
    // gc keeps what the commits hold, and drops the kept state with it.
    delta.gc().unwrap();
    assert!(delta.fsck().unwrap().is_empty());
    assert!(delta.fsck_deep().unwrap().is_empty());
    for (c, net) in commits.iter().zip(&nets) {
        assert_eq!(&delta.net_fragment_file(c, "nets/s.net").unwrap(), net);
    }
    // A write after gc starts from the store again, and still agrees.
    let mut next = nets.last().unwrap().clone();
    edit(&mut next, 27);
    assert_eq!(
        delta.put_net_fragments(&next, Layout::Tree).unwrap(),
        whole.put_net_fragments(&next, Layout::Tree).unwrap()
    );
    // An invalid net is refused, as by the whole write.
    let mut bad = next.clone();
    bad.transitions[0].inputs = vec![arc("nowhere", 1)];
    assert!(delta.put_net_fragments(&bad, Layout::Tree).is_err());
}

/// A root whose nodes are `net`'s, changed by `f`.
fn forged(
    repo: &mut Repo<MemStore>,
    net: &Net,
    f: impl FnOnce(&mut Repo<MemStore>, &mut Vec<Node>),
) -> Id {
    let good = repo.put_net_fragments(net, Layout::Tree).unwrap();
    let mut nodes = repo.graph_nodes(&good).unwrap();
    f(repo, &mut nodes);
    repo.put_graph(nodes).unwrap()
}

fn retarget(repo: &mut Repo<MemStore>, nodes: &mut [Node], label: &str, text: &str) {
    let id = repo.put_content(text.as_bytes()).unwrap().id;
    let n = nodes.iter_mut().find(|n| n.label == label).unwrap();
    n.target = Target::Content(Arg::Whole(id));
}

#[test]
fn the_fast_read_refuses_what_the_full_read_refuses() {
    let net = Net::parse(
        "net n 1\nplace a 2\nplace b colored\nplace c\n\
         transition t cmd x: 2*a, b -> c\ntransition u pure: c ->\ntransition v human: -> a\n",
    )
    .unwrap();
    type Forge = Box<dyn FnOnce(&mut Repo<MemStore>, &mut Vec<Node>)>;
    let cases: Vec<(&str, Forge)> = vec![
        (
            "a second space",
            Box::new(|r, n| retarget(r, n, "p/a", "place a  2\n")),
        ),
        (
            "a weight of 1 written",
            Box::new(|r, n| retarget(r, n, "t/u", "transition u pure: 1*c ->\n")),
        ),
        (
            "a leading zero",
            Box::new(|r, n| retarget(r, n, "p/a", "place a 02\n")),
        ),
        (
            "a count of 0",
            Box::new(|r, n| retarget(r, n, "p/c", "place c 0\n")),
        ),
        (
            "an arc to no place",
            Box::new(|r, n| retarget(r, n, "t/u", "transition u pure: zz ->\n")),
        ),
        (
            "a transition named like a place",
            Box::new(|r, n| {
                // t/u becomes t/a, `transition a pure: c ->`, linked from t/t.
                retarget(r, n, "t/u", "transition a pure: c ->\n");
                n.iter_mut().find(|n| n.label == "t/u").unwrap().label = "t/a".into();
                let t = n.iter_mut().find(|n| n.label == "t/t").unwrap();
                t.edges = vec![Edge {
                    to: node_key("t/a"),
                    label: "next t/a".into(),
                }];
            }),
        ),
        (
            "a human with a capability",
            Box::new(|r, n| retarget(r, n, "t/v", "transition v human x: -> a\n")),
        ),
        (
            "no arcs",
            Box::new(|r, n| retarget(r, n, "t/v", "transition v human: ->\n")),
        ),
        (
            "a wrong edge key",
            Box::new(|_, n| {
                let t = n.iter_mut().find(|n| n.label == "t/t").unwrap();
                t.edges[0].to = node_key("t/v");
            }),
        ),
        (
            "an extra edge",
            Box::new(|_, n| {
                let t = n.iter_mut().find(|n| n.label == "p/a").unwrap();
                t.edges.push(Edge {
                    to: node_key("p/c"),
                    label: "also p/c".into(),
                });
            }),
        ),
        (
            "a cycle",
            Box::new(|_, n| {
                let t = n.iter_mut().find(|n| n.label == "t/v").unwrap();
                t.edges = vec![Edge {
                    to: node_key("t/t"),
                    label: "next t/t".into(),
                }];
            }),
        ),
        (
            "a node no link reaches",
            Box::new(|r, n| {
                let id = r.put_content(b"place d\n").unwrap().id;
                n.push(Node {
                    label: "p/d".into(),
                    target: Target::Content(Arg::Whole(id)),
                    edges: vec![],
                });
            }),
        ),
        (
            "a place linked among transitions",
            Box::new(|_, n| {
                let t = n.iter_mut().find(|n| n.label == "t/v").unwrap();
                t.edges = vec![Edge {
                    to: node_key("p/a"),
                    label: "next p/a".into(),
                }];
            }),
        ),
        ("no head", Box::new(|_, n| n.retain(|n| n.label != "net"))),
    ];
    for (what, f) in cases {
        let mut repo = fast_repo();
        let root = forged(&mut repo, &net, f);
        assert!(
            repo.net_fragments_full(&root).is_err(),
            "full read took {what}"
        );
        assert!(
            repo.net_fragments_fast(&root).is_err(),
            "fast read took {what}"
        );
        assert!(
            repo.net_fragments_par(&root).is_err(),
            "par read took {what}"
        );
    }
    // The canonical net itself is read.
    let mut repo = fast_repo();
    let root = repo.put_net_fragments(&net, Layout::Tree).unwrap();
    assert_eq!(repo.net_fragments_fast(&root).unwrap(), net);
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

/// E422's run, by `net_fire` on the whole net or `net_fire_at` on the
/// version's root; the firing records and the marking.
fn run(big: usize, at: bool, opts: bool) -> (Vec<String>, String) {
    let mut repo = Repo::new(MemStore::default());
    if opts {
        repo.graph_cache = 1 << 12;
        repo.net_fast_read = true;
        repo.net_delta_write = true;
    }
    let mut cur = night(1, big);
    let mut db = repo.db_empty().unwrap();
    db = repo.net_init(&db, &cur).unwrap();
    db = repo.net_track_fragments(&db, &cur, Layout::Tree).unwrap();
    let mut root = repo.put_net_fragments(&cur, Layout::Tree).unwrap();
    for i in 0..30 {
        if i > 0 && i % 6 == 0 {
            cur = night(cur.version + 1, big);
            root = repo.put_net_fragments(&cur, Layout::Tree).unwrap();
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
            // A retry with the same key changes nothing.
            let again = match at {
                true => repo.net_fire_at(&db, &root, t, Some(&c), &key),
                false => repo.net_fire(&db, &cur, t, Some(&c), &key),
            };
            assert_eq!(again.unwrap().0, db);
        }
    }
    let m = repo.net_marking(&db, "night").unwrap();
    assert_eq!(repo.net_replay_versions(&db, "night").unwrap(), m);
    let records = repo
        .net_firings(&db, "night")
        .unwrap()
        .iter()
        .map(|f| format!("{f:?}"))
        .collect();
    (records, format!("{m:?}"))
}

#[test]
fn firings_by_key_record_and_replay_as_whole_net_firings_do() {
    let plain = run(0, false, false);
    assert_eq!(run(0, false, true), plain);
    assert_eq!(run(0, true, true), plain);
    assert_eq!(run(0, true, false), plain);
    let big = run(400, false, false);
    assert_eq!(run(400, true, true), big);
    assert_eq!(run(400, false, true), big);
}

#[test]
fn a_firing_by_key_needs_fragment_tracking() {
    let mut repo = Repo::new(MemStore::default());
    let net = night(1, 0);
    let root = repo.put_net_fragments(&net, Layout::Tree).unwrap();
    let mut db = repo.db_empty().unwrap();
    db = repo.net_init(&db, &net).unwrap();
    assert!(
        repo.net_fire_at(&db, &root, "propose", Some("c"), "k")
            .is_err()
    );
    db = repo.net_track_versions(&db, &net).unwrap();
    assert!(
        repo.net_fire_at(&db, &root, "propose", Some("c"), "k")
            .is_err()
    );
    let part = repo.net_fragments_part(&root, &["start"]).unwrap();
    assert_eq!(
        part.transitions,
        vec![net.transition("start").unwrap().clone()]
    );
    let names: Vec<&str> = part.places.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["queued", "cpu", "running"]);
    assert!(repo.net_fragments_part(&root, &["nothing"]).is_err());
}

/// A small general graph: label to (payload, links).
fn general(n: usize) -> BTreeMap<String, (String, BTreeSet<String>)> {
    (0..n)
        .map(|i| {
            let to = [(i * 7 + 3) % n, (i * i + 1) % n, i / 2]
                .iter()
                .map(|j| format!("n{j:05}"))
                .collect();
            (
                format!("n{i:05}"),
                (format!("payload of node {i} {}", "x".repeat(i % 50)), to),
            )
        })
        .collect()
}

fn put_general(repo: &mut Repo<MemStore>, g: &BTreeMap<String, (String, BTreeSet<String>)>) -> Id {
    let mut nodes = Vec::new();
    for (l, (p, to)) in g {
        let id = repo.put_content(p.as_bytes()).unwrap().id;
        nodes.push(Node {
            label: l.clone(),
            target: Target::Content(Arg::Whole(id)),
            edges: to
                .iter()
                .map(|t| Edge {
                    to: node_key(t),
                    label: format!("link {t}"),
                })
                .collect(),
        });
    }
    repo.put_graph(nodes).unwrap()
}

#[test]
fn cached_graph_reads_equal_the_plain_ones() {
    let mut g = general(4000);
    let mut repo = Repo::new(MemStore::default());
    repo.graph_cache = 1 << 12;
    repo.read_threads = 3;
    let a = put_general(&mut repo, &g);
    g.get_mut("n00010").unwrap().0 = "edited".into();
    g.remove("n03000");
    let b = put_general(&mut repo, &g);
    for root in [a, b] {
        let nodes = repo.graph_nodes(&root).unwrap();
        assert_eq!(repo.graph_nodes_cached(&root).unwrap(), nodes);
        for read in [
            repo.graph_read(&root).unwrap(),
            repo.graph_read_par(&root).unwrap(),
        ] {
            let got: Vec<&(Node, Option<Vec<u8>>)> =
                read.iter().flat_map(|l| l.nodes.iter()).collect();
            assert_eq!(got.len(), nodes.len());
            for ((n, p), want) in got.into_iter().zip(&nodes) {
                assert_eq!(n, want);
                let Target::Content(Arg::Whole(id)) = want.target else {
                    panic!()
                };
                assert_eq!(p.as_ref().unwrap(), &repo.read_content(&id).unwrap());
            }
        }
        let ids: Vec<Id> = nodes
            .iter()
            .map(|n| match n.target {
                Target::Content(Arg::Whole(id)) => id,
                _ => panic!(),
            })
            .collect();
        let contents = repo.read_contents(&ids).unwrap();
        for (id, c) in ids.iter().zip(&contents) {
            assert_eq!(c, &repo.read_content(id).unwrap());
        }
        for l in [
            "n00000", "n00010", "n02999", "n03000", "n03999", "a", "z", "n0001",
        ] {
            assert_eq!(
                repo.graph_node_cached(&root, l).unwrap(),
                repo.graph_node(&root, l).unwrap(),
                "{l}"
            );
        }
        let labels: BTreeSet<String> = ["n00001", "n01234", "n03000", "n03998", "nope"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            repo.graph_nodes_in_cached(&root, &labels).unwrap(),
            repo.graph_nodes_in(&root, &labels).unwrap()
        );
    }
    // Two hops by links, against a walk of the graph in memory.
    let follow = |e: &Edge| e.label.strip_prefix("link ").map(String::from);
    let hops = repo
        .graph_hops(&b, &["n00007".to_string()], 2, follow)
        .unwrap();
    let mut want = BTreeSet::from(["n00007".to_string()]);
    let mut frontier = want.clone();
    for _ in 0..2 {
        let next: BTreeSet<String> = frontier
            .iter()
            .filter_map(|l| g.get(l))
            .flat_map(|(_, to)| to.iter().cloned())
            .filter(|l| g.contains_key(l) && !want.contains(l))
            .collect();
        want.extend(next.iter().cloned());
        frontier = next;
    }
    assert_eq!(hops.keys().cloned().collect::<BTreeSet<_>>(), want);
}

#[test]
fn kept_chunks_are_shared_with_siblings_and_dropped_by_gc() {
    let mut repo = Repo::new(SharedStore::new(MemStore::default()));
    repo.graph_cache = 1 << 12;
    repo.net_fast_read = true;
    let net = synth(500);
    let root = repo.put_net_fragments(&net, Layout::Tree).unwrap();
    assert_eq!(repo.net_fragments(&root).unwrap(), net);
    let (_, decoded) = repo.graph_cache_stats();
    let sib = repo.sibling();
    assert_eq!(sib.net_fragments(&root).unwrap(), net);
    assert_eq!(
        sib.graph_cache_stats().1,
        decoded,
        "the sibling decoded nothing"
    );
    repo.gc().unwrap();
    assert_eq!(repo.graph_cache_stats(), (0, 0));
}
