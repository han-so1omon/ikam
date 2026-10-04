//! Graph laws (docs/plans/2026-10-04-graph-objects.md).

use std::collections::BTreeSet;

use ikam_kernel::{
    Arg, Edge, Graph, Id, Kind, MemStore, Node, Object, Repo, Store, Target, TreeEntry, node_key,
};

fn rng(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

fn node(label: &str, edges: &[(&str, &str)]) -> Node {
    let mut n = Node {
        label: label.into(),
        target: Target::None,
        edges: edges
            .iter()
            .map(|(to, l)| Edge {
                to: node_key(to),
                label: (*l).into(),
            })
            .collect(),
    };
    n.edges.sort();
    n
}

/// `n` entities with ~3 random labelled edges each, then versions that add
/// 5 nodes and 10 edges and remove 2 edges each.
fn versions(n: usize, count: usize) -> Vec<Vec<Node>> {
    let mut s = 0x9e3779b97f4a7c15u64;
    let labels = ["cites", "mentions", "part-of", "derived-from"];
    let mut names: Vec<String> = (0..n)
        .map(|_| format!("entity:{:x}", rng(&mut s) % 1_000_000_007))
        .collect();
    let mut edges: BTreeSet<(String, String, String)> = BTreeSet::new();
    let add_edge = |s: &mut u64, names: &[String], edges: &mut BTreeSet<_>| {
        let a = names[(rng(s) % names.len() as u64) as usize].clone();
        let b = names[(rng(s) % names.len() as u64) as usize].clone();
        edges.insert((a, b, labels[(rng(s) % 4) as usize].to_string()));
    };
    for _ in 0..3 * n {
        add_edge(&mut s, &names, &mut edges);
    }
    let build = |names: &[String], edges: &BTreeSet<(String, String, String)>| -> Vec<Node> {
        let unique: BTreeSet<&String> = names.iter().collect();
        unique
            .into_iter()
            .map(|a| Node {
                label: a.clone(),
                target: Target::None,
                edges: edges
                    .range((a.clone(), String::new(), String::new())..)
                    .take_while(|e| &e.0 == a)
                    .map(|(_, b, l)| Edge {
                        to: node_key(b),
                        label: l.clone(),
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            })
            .collect()
    };
    let mut out = vec![build(&names, &edges)];
    for _ in 1..count {
        for _ in 0..5 {
            names.push(format!("entity:{:x}", rng(&mut s) % 1_000_000_007));
        }
        for _ in 0..10 {
            add_edge(&mut s, &names, &mut edges);
        }
        for _ in 0..2 {
            let e = edges
                .iter()
                .nth((rng(&mut s) % edges.len() as u64) as usize)
                .unwrap()
                .clone();
            edges.remove(&e);
        }
        out.push(build(&names, &edges));
    }
    out
}

fn stored(repo: &Repo<MemStore>) -> usize {
    let ids = repo.store().ids().unwrap();
    ids.iter()
        .map(|id| repo.store().read(id).unwrap().len())
        .sum()
}

fn commit_graphs(repo: &mut Repo<MemStore>, roots: &[Id]) {
    let entries = roots
        .iter()
        .enumerate()
        .map(|(i, id)| TreeEntry {
            name: format!("g{i:03}"),
            kind: Kind::Graph,
            id: *id,
        })
        .collect();
    let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
}

/// Law 1: canonical chunks round-trip, and decoding is strict.
#[test]
fn graph_chunks_decode_strictly() {
    let g = Object::Graph(Graph::Nodes(vec![
        node("a", &[("a", "self"), ("b", "next")]),
        node("b", &[("a", "back")]),
    ]));
    assert_eq!(Object::decode(&g.encode()).unwrap(), g);
    let unsorted = Object::Graph(Graph::Nodes(vec![node("b", &[]), node("a", &[])]));
    assert!(Object::decode(&unsorted.encode()).is_err());
    let mut edges = node("a", &[("a", "x"), ("b", "y")]);
    edges.edges.reverse();
    assert!(Object::decode(&Object::Graph(Graph::Nodes(vec![edges])).encode()).is_err());
    let mut trailing = g.encode();
    trailing.push(0);
    assert!(Object::decode(&trailing).is_err());
}

/// Law 2: the same labelled graph has the same id whatever the build order;
/// labels with the same key are rejected.
#[test]
fn graph_identity_ignores_insertion_order() {
    let nodes = versions(500, 1).remove(0);
    let mut repo = Repo::new(MemStore::default());
    let root = repo.put_graph(nodes.clone()).unwrap();
    let mut shuffled = nodes.clone();
    shuffled.reverse();
    shuffled.iter_mut().for_each(|n| n.edges.reverse());
    assert_eq!(repo.put_graph(shuffled).unwrap(), root);
    assert_eq!(repo.graph_nodes(&root).unwrap(), nodes);
    assert!(
        repo.put_graph(vec![node("x", &[]), node("x", &[])])
            .is_err()
    );
}

/// Laws 3–5: cycles (with a self-loop) survive put, get, gc, repack and
/// fsck; gc keeps node targets live and never follows edges.
#[test]
fn cyclic_graphs_survive_gc_and_repack() {
    let mut repo = Repo::new(MemStore::default());
    let doc = repo
        .put_content(b"Section 1 cites section 2, which cites section 1.")
        .unwrap()
        .id;
    let orphan = repo.put_content(b"content nothing points at").unwrap().id;
    let mut a = node(
        "section-1",
        &[("section-1", "self"), ("section-2", "cites")],
    );
    a.target = Target::Content(Arg::Range {
        id: doc,
        start: 0,
        len: 9,
    });
    let b = node("section-2", &[("section-1", "cites")]);
    let mut nodes = versions(300, 1).remove(0);
    nodes.extend([a, b]);
    let root = repo.put_graph(nodes.clone()).unwrap();
    commit_graphs(&mut repo, &[root]);
    repo.gc().unwrap();
    assert!(
        repo.read_content(&orphan).is_err(),
        "gc dropped unreferenced content"
    );
    repo.repack().unwrap();
    assert!(repo.fsck().unwrap().is_empty());
    nodes.sort_by(|x, y| x.label.cmp(&y.label));
    assert_eq!(repo.graph_nodes(&root).unwrap(), nodes);
    let s1 = repo.graph_node(&root, "section-1").unwrap().unwrap();
    assert_eq!(s1.edges[0].to.len(), 8);
    assert_eq!(repo.read_content(&doc).unwrap()[..9], *b"Section 1");
}

/// Law 6: a slice is a graph; an unchanged slice has the same id in two
/// versions, and diff reports exactly the changed nodes.
#[test]
fn slices_and_diffs_follow_versions() {
    let vs = versions(2000, 2);
    let mut repo = Repo::new(MemStore::default());
    let (r0, r1) = (
        repo.put_graph(vs[0].clone()).unwrap(),
        repo.put_graph(vs[1].clone()).unwrap(),
    );
    let changed: BTreeSet<String> = repo
        .graph_diff(&r0, &r1)
        .unwrap()
        .into_iter()
        .map(|d| d.0)
        .collect();
    assert!(
        !changed.is_empty() && changed.len() < 40,
        "{} changed",
        changed.len()
    );
    let stable: BTreeSet<String> = vs[0]
        .iter()
        .map(|n| n.label.clone())
        .filter(|l| !changed.contains(l))
        .take(50)
        .collect();
    let keys: BTreeSet<[u8; 8]> = stable.iter().map(|l| node_key(l)).collect();
    let touches = |v: &Vec<Node>| {
        v.iter()
            .filter(|n| stable.contains(&n.label))
            .flat_map(|n| &n.edges)
            .filter(|e| keys.contains(&e.to))
            .count()
    };
    if touches(&vs[0]) == touches(&vs[1]) {
        assert_eq!(
            repo.graph_slice(&r0, &stable).unwrap(),
            repo.graph_slice(&r1, &stable).unwrap()
        );
    }
}

/// Law 7: versions share chunks, so a history costs a few versions' worth.
/// Measured: 20 versions take ~3.3x one version at zstd -19. Storing each
/// version as one content takes ~1.6x but gives up chunk-level lookup, diff
/// and sharing; that trade-off is recorded in the design note.
#[test]
fn graph_versions_share_storage() {
    let vs = versions(2000, 20);
    let one = Object::Graph(Graph::Nodes(vs[0].clone())).encode();
    let one_zstd = zstd::bulk::compress(&one, 19).unwrap().len();
    let mut repo = Repo::new(MemStore::default());
    let roots: Vec<Id> = vs
        .iter()
        .map(|v| repo.put_graph(v.clone()).unwrap())
        .collect();
    commit_graphs(&mut repo, &roots);
    let ingest = stored(&repo);
    repo.repack().unwrap();
    let total = stored(&repo);
    println!(
        "one version {} B, zstd-19 {one_zstd} B; 20 versions: ingest {ingest} B, repack {total} B",
        one.len()
    );
    for (v, root) in vs.iter().zip(&roots) {
        assert_eq!(&repo.graph_nodes(root).unwrap(), v);
    }
    assert!(total < 4 * one_zstd, "{total} >= 4 x {one_zstd}");
}
