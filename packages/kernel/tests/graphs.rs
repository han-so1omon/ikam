//! Graph laws (docs/plans/2026-10-04-graph-objects.md).

use std::collections::{BTreeMap, BTreeSet};

use ikam_kernel::{
    Arg, Edge, Graph, Id, Keys, Kind, MemStore, Node, Object, Repo, Store, Target, TreeEntry,
    node_key,
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

fn simple(label: String, s: &mut u64) -> Node {
    let to = format!("entity:{:x}", rng(s) % 1_000_000_007);
    Node {
        label,
        target: Target::None,
        edges: vec![Edge {
            to: node_key(&to),
            label: "cites".into(),
        }],
    }
}

/// Random edits: inserts (some before the first label or after the last),
/// updates, removes of present and absent labels.
fn random_edits(nodes: &[Node], n: usize, s: &mut u64) -> BTreeMap<String, Option<Node>> {
    let mut edits = BTreeMap::new();
    for _ in 0..n {
        let r = rng(s);
        let existing = (!nodes.is_empty())
            .then(|| nodes[(rng(s) % nodes.len() as u64) as usize].label.clone());
        match (r % 10, existing) {
            (0, _) => edits.insert(
                format!("!{:x}", rng(s) % 1000),
                Some(simple(String::new(), s)),
            ),
            (1, _) => edits.insert(
                format!("~{:x}", rng(s) % 1000),
                Some(simple(String::new(), s)),
            ),
            (2..=4, _) => edits.insert(
                format!("entity:{:x}", rng(s) % 1_000_000_007),
                Some(simple(String::new(), s)),
            ),
            (5..=6, Some(l)) => edits.insert(l, Some(simple(String::new(), s))),
            (7..=8, Some(l)) => edits.insert(l, None),
            _ => edits.insert(format!("absent:{:x}", rng(s)), None),
        };
    }
    for (label, node) in edits.iter_mut() {
        if let Some(n) = node {
            n.label = label.clone();
        }
    }
    edits
}

fn apply(nodes: &[Node], edits: &BTreeMap<String, Option<Node>>) -> Vec<Node> {
    let mut map: BTreeMap<String, Node> =
        nodes.iter().map(|n| (n.label.clone(), n.clone())).collect();
    for (label, edit) in edits {
        match edit {
            Some(n) => map.insert(label.clone(), n.clone()),
            None => map.remove(label),
        };
    }
    map.into_values().collect()
}

fn height(repo: &Repo<MemStore>, root: &Id) -> u8 {
    match repo.get(root).unwrap() {
        Object::Graph(Graph::Children(h, _)) => h,
        _ => 0,
    }
}

/// In-place edits give the root `put_graph` gives for the edited node set,
/// byte for byte, on graphs of every height (including growing and
/// shrinking ones), chained edit after edit.
#[test]
fn in_place_edits_equal_a_full_rebuild() {
    let mut s = 0x853c49e6748fea9bu64;
    let mut repo = Repo::new(MemStore::default());
    let mut heights = BTreeSet::new();
    for &(size, rounds, batch) in &[
        (0usize, 20, 5usize),
        (40, 30, 10),
        (3000, 40, 30),
        (150_000, 6, 200),
    ] {
        let mut nodes: Vec<Node> = (0..size)
            .map(|_| simple(format!("entity:{:x}", rng(&mut s) % 1_000_000_007), &mut s))
            .map(|n| (n.label.clone(), n))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect();
        let mut root = repo.put_graph(nodes.clone()).unwrap();
        heights.insert(height(&repo, &root));
        for _ in 0..rounds {
            let n = 1 + (rng(&mut s) % batch as u64) as usize;
            let edits = random_edits(&nodes, n, &mut s);
            nodes = apply(&nodes, &edits);
            root = repo.graph_edit(&root, edits).unwrap();
            assert_eq!(root, repo.put_graph(nodes.clone()).unwrap(), "size {size}");
            heights.insert(height(&repo, &root));
        }
        assert_eq!(repo.graph_nodes(&root).unwrap(), nodes);
        let all: BTreeMap<String, Option<Node>> =
            nodes.iter().map(|n| (n.label.clone(), None)).collect();
        let emptied = repo.graph_edit(&root, all).unwrap();
        assert_eq!(
            emptied,
            repo.put_graph(Vec::new()).unwrap(),
            "removing everything gives the empty graph"
        );
    }
    assert!(
        heights.contains(&0) && heights.contains(&1) && heights.iter().any(|h| *h >= 2),
        "{heights:?}"
    );
}

/// Growing a graph one node at a time from empty, at the end and at the
/// start, matches `put_graph` at every step and grows its height.
#[test]
fn in_place_appends_equal_a_full_rebuild() {
    let mut s = 0x9e3779b97f4a7c15u64;
    let mut repo = Repo::new(MemStore::default());
    let (mut root, mut nodes) = (repo.put_graph(Vec::new()).unwrap(), Vec::new());
    for i in 0..3000u32 {
        let label = if i % 2 == 0 {
            format!("{:08}", i)
        } else {
            format!("!{:08}", 10_000 - i)
        };
        let node = simple(label.clone(), &mut s);
        nodes.push(node.clone());
        root = repo
            .graph_edit(&root, BTreeMap::from([(label, Some(node))]))
            .unwrap();
        if i % 97 == 0 || i == 2999 {
            assert_eq!(root, repo.put_graph(nodes.clone()).unwrap(), "after {i}");
        }
    }
    assert!(height(&repo, &root) >= 1);
}

/// A label whose key clashes with a node in a chunk being rewritten is
/// refused; a label must match its node.
#[test]
fn in_place_edits_check_labels() {
    let mut repo = Repo::new(MemStore::default());
    let root = repo.put_graph(vec![node("a", &[])]).unwrap();
    assert!(
        repo.graph_edit(
            &root,
            BTreeMap::from([("b".to_string(), Some(node("c", &[])))])
        )
        .is_err()
    );
}

/// One edit in a large graph writes only the path to it: a few chunks per
/// level, not the graph.
#[test]
fn in_place_edit_writes_only_its_path() {
    let mut s = 0x2545f4914f6cdd1du64;
    let mut repo = Repo::new(MemStore::default());
    let nodes: Vec<Node> = (0..150_000)
        .map(|i| simple(format!("entity:{i:08}"), &mut s))
        .collect();
    let root = repo.put_graph(nodes.clone()).unwrap();
    let before = repo.store().ids().unwrap().len();
    let label = nodes[75_000].label.clone();
    let edit = simple(label.clone(), &mut s);
    let edited = repo
        .graph_edit(&root, BTreeMap::from([(label, Some(edit))]))
        .unwrap();
    let written = repo.store().ids().unwrap().len() - before;
    let levels = height(&repo, &edited) as usize + 1;
    assert!(
        written <= 4 * levels,
        "{written} objects for {levels} levels"
    );
}

/// A store that records which objects are read.
#[derive(Default)]
struct Reads {
    inner: MemStore,
    read: std::cell::RefCell<Vec<Id>>,
}

impl Store for Reads {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, ikam_kernel::Error> {
        self.inner.write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), ikam_kernel::Error> {
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, ikam_kernel::Error> {
        self.read.borrow_mut().push(*id);
        self.inner.read(id)
    }
    fn has(&self, id: &Id) -> bool {
        self.inner.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, ikam_kernel::Error> {
        self.inner.ids()
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, ikam_kernel::Error> {
        self.inner.ids_with_prefix(prefix)
    }
    fn delete(&mut self, id: &Id) -> Result<(), ikam_kernel::Error> {
        self.inner.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, ikam_kernel::Error> {
        self.inner.get_ref(name)
    }
    fn set_ref(
        &mut self,
        name: &str,
        expected: Option<Id>,
        new: Id,
    ) -> Result<(), ikam_kernel::Error> {
        self.inner.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, ikam_kernel::Error> {
        self.inner.refs()
    }
}

/// An edit reads the path from the root to the chunks it changes, not
/// every chunk above the leaves (which grows with the graph: about 130 at
/// a million nodes, 1,300 at ten million).
#[test]
fn in_place_edit_reads_only_its_path() {
    let mut s = 0x9e3779b97f4a7c15u64;
    let mut repo = Repo::new(Reads::default());
    let nodes: Vec<Node> = (0..150_000)
        .map(|i| simple(format!("entity:{i:08}"), &mut s))
        .collect();
    let root = repo.put_graph(nodes.clone()).unwrap();
    // Every chunk above the leaves.
    let (mut upper, mut level) = (BTreeSet::new(), vec![root]);
    let mut top = 0;
    while !level.is_empty() {
        let mut next = Vec::new();
        for id in &level {
            if let Object::Graph(Graph::Children(h, children)) = repo.get(id).unwrap() {
                top = top.max(h as usize);
                upper.insert(*id);
                next.extend(children.into_iter().map(|c| c.1));
            }
        }
        level = next;
    }
    assert!(top >= 2 && upper.len() > 10, "{} upper chunks", upper.len());
    for at in [0, 75_000, 149_999] {
        let label = nodes[at].label.clone();
        let edit = simple(label.clone(), &mut s);
        repo.store().read.borrow_mut().clear();
        let edited = repo
            .graph_edit(&root, BTreeMap::from([(label.clone(), Some(edit.clone()))]))
            .unwrap();
        let reads = repo.store().read.borrow().clone();
        let upper_reads = reads.iter().filter(|id| upper.contains(id)).count();
        assert!(
            upper_reads <= 2 * top,
            "{upper_reads} of {} upper chunks read for one edit at height {top}",
            upper.len()
        );
        let mut want = nodes.clone();
        want[at] = edit;
        assert_eq!(edited, repo.put_graph(want).unwrap());
    }
}

/// Two labels whose 8-byte keys are the same, found once by a collision
/// search over labels of this shape (about 2^32 hashes).
const CLASH: [&str; 2] = ["ke32051949435c933", "kb4a1e63c25ad68d4"];

/// A graph of `n` labels spread over the range the clashing labels fall
/// in, so the two land in different chunks.
fn spread(n: u64, s: &mut u64) -> Vec<Node> {
    (0..n)
        .map(|i| simple(format!("k{:016x}", i * (u64::MAX / n) + 1), s))
        .collect()
}

/// Labels whose keys clash: `put_graph` refuses them; with `Keys::Any`
/// both are kept, found by label, and in-place edits give the root a full
/// build gives, whether the two land in one chunk or in different ones.
#[test]
fn clashing_keys_are_kept_alike_with_any_keys() {
    assert_ne!(CLASH[0], CLASH[1]);
    assert_eq!(node_key(CLASH[0]), node_key(CLASH[1]));
    let mut s = 0x2545f4914f6cdd1du64;
    for n in [0, 20, 60_000] {
        let mut repo = Repo::new(MemStore::default());
        let nodes = spread(n, &mut s);
        let clash: Vec<Node> = CLASH
            .iter()
            .map(|l| simple(l.to_string(), &mut s))
            .collect();
        let all: Vec<Node> = nodes.iter().chain(&clash).cloned().collect();
        assert!(repo.put_graph(all.clone()).is_err(), "{n} nodes");
        let full = repo.put_graph_with(all.clone(), Keys::Any).unwrap();

        // Without a clash, either kind of check gives the same root.
        let base = repo.put_graph(nodes.clone()).unwrap();
        assert_eq!(base, repo.put_graph_with(nodes.clone(), Keys::Any).unwrap());

        // One at a time, then both at once.
        let one = repo
            .graph_edit_with(
                &base,
                BTreeMap::from([(CLASH[0].to_string(), Some(clash[0].clone()))]),
                Keys::Any,
            )
            .unwrap();
        let two = repo
            .graph_edit_with(
                &one,
                BTreeMap::from([(CLASH[1].to_string(), Some(clash[1].clone()))]),
                Keys::Any,
            )
            .unwrap();
        assert_eq!(two, full, "{n} nodes");
        let both = clash
            .iter()
            .map(|c| (c.label.clone(), Some(c.clone())))
            .collect();
        assert_eq!(repo.graph_edit_with(&base, both, Keys::Any).unwrap(), full);
        for c in &clash {
            assert_eq!(repo.graph_node(&full, &c.label).unwrap().as_ref(), Some(c));
        }

        // Dropping one leaves the other, as a full build of the rest does.
        let dropped = repo
            .graph_edit_with(
                &full,
                BTreeMap::from([(CLASH[0].to_string(), None)]),
                Keys::Any,
            )
            .unwrap();
        let rest: Vec<Node> = nodes.iter().chain(&clash[1..]).cloned().collect();
        assert_eq!(dropped, repo.put_graph(rest).unwrap());
    }
}

/// Time and reads of one edit to a kv-shaped graph of `IKAM_EDIT_KEYS`
/// nodes (default 1,000,000): `cargo test --release --test graphs
/// edit_cost -- --ignored --nocapture`.
#[test]
#[ignore]
fn edit_cost() {
    let n: u64 = std::env::var("IKAM_EDIT_KEYS").map_or(1_000_000, |v| v.parse().unwrap());
    let event = Id::of_content(b"event");
    let node = |i: u64| Node {
        label: format!("user/{:08}/profile", i * 7919 % 100_000_000),
        target: Target::Content(Arg::Range {
            id: event,
            start: 60,
            len: 140,
        }),
        edges: vec![],
    };
    let mut repo = Repo::new(Reads::default());
    let t = std::time::Instant::now();
    let root = repo.put_graph((0..n).map(node).collect()).unwrap();
    println!(
        "{n} keys built in {:?}, height {}",
        t.elapsed(),
        match repo.get(&root).unwrap() {
            Object::Graph(Graph::Children(h, _)) => h,
            _ => 0,
        }
    );
    for k in 0..3 {
        let edit = node(n + 1 + k);
        repo.store().read.borrow_mut().clear();
        let t = std::time::Instant::now();
        repo.graph_edit(&root, BTreeMap::from([(edit.label.clone(), Some(edit))]))
            .unwrap();
        println!(
            "one edit: {:?}, {} store reads",
            t.elapsed(),
            repo.store().read.borrow().len()
        );
    }
}
