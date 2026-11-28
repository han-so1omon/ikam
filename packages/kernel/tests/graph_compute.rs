//! E450: computations over graphs stored as connected fragments, kept up to
//! date from the delta between versions. Every incremental result must equal
//! a full recompute (same root), the in-memory baseline must agree with
//! both, and the results must be versioned graphs that gc keeps through the
//! step records a commit holds.

use std::collections::{BTreeMap, HashMap};

use ikam_kernel::graph_compute::{Compute, Deg, Entry, GraphDelta, Key, MemGraph, key_of};
use ikam_kernel::{Arg, Edge, Edit, Id, MemStore, Node, Repo, Target, node_key};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn edge(to: &str, label: &str) -> Edge {
    Edge {
        to: node_key(to),
        label: label.into(),
    }
}

fn node(label: &str, target: Target, edges: Vec<Edge>) -> Node {
    let mut edges = edges;
    edges.sort();
    edges.dedup();
    Node {
        label: label.into(),
        target,
        edges,
    }
}

/// A random graph of `n` nodes: a few components, some edges to labels no
/// node has (dangling), some with two labels to one target, self loops.
fn random_graph(repo: &mut Repo<MemStore>, n: usize, rng: &mut Rng) -> BTreeMap<String, Node> {
    let mut out = BTreeMap::new();
    for i in 0..n {
        let label = format!("n{i}");
        let title = repo
            .put_content(format!("title {i}").as_bytes())
            .unwrap()
            .id;
        let mut edges = Vec::new();
        // Nodes in blocks of 200 link only within their block, so there
        // are several components.
        let block = i / 200 * 200;
        for _ in 0..rng.below(5) {
            let t = block + rng.below(200.min(n - block));
            edges.push(edge(
                &format!("n{t}"),
                if rng.below(9) == 0 { "b" } else { "" },
            ));
        }
        if rng.below(30) == 0 {
            edges.push(edge(&format!("gone{}", rng.below(5)), ""));
        }
        out.insert(
            label.clone(),
            node(&label, Target::Content(Arg::Whole(title)), edges),
        );
    }
    out
}

/// One version's edits: edges added and removed, a node removed, a node
/// added (sometimes one removed before, or a dangling target), a small
/// separate subgraph, a bridge between blocks, a title changed.
fn edits(
    repo: &mut Repo<MemStore>,
    g: &BTreeMap<String, Node>,
    v: usize,
    rng: &mut Rng,
) -> BTreeMap<String, Option<Node>> {
    let labels: Vec<&String> = g.keys().collect();
    let pick = |rng: &mut Rng| labels[rng.below(labels.len())].clone();
    let mut e: BTreeMap<String, Option<Node>> = BTreeMap::new();
    let get = |e: &BTreeMap<String, Option<Node>>, l: &str| -> Option<Node> {
        match e.get(l) {
            Some(x) => x.clone(),
            None => g.get(l).cloned(),
        }
    };
    for _ in 0..4 {
        let l = pick(rng);
        if let Some(mut n) = get(&e, &l)
            && !n.edges.is_empty()
        {
            let i = rng.below(n.edges.len());
            n.edges.remove(i);
            e.insert(l, Some(n));
        }
    }
    for _ in 0..4 {
        let (a, b) = (pick(rng), pick(rng));
        if let Some(mut n) = get(&e, &a) {
            n.edges.push(edge(&b, ""));
            n.edges.sort();
            n.edges.dedup();
            e.insert(a, Some(n));
        }
    }
    if v.is_multiple_of(3) {
        let l = pick(rng);
        e.insert(l, None);
    }
    if v % 4 == 1 {
        // A removed node comes back, or a dangling target becomes a node.
        let l = if v % 8 == 1 {
            format!("gone{}", rng.below(5))
        } else {
            format!("n{}", rng.below(labels.len()))
        };
        let t = pick(rng);
        e.insert(l.clone(), Some(node(&l, Target::None, vec![edge(&t, "")])));
    }
    if v % 5 == 2 {
        // A separate small subgraph, then later joined to a block.
        for j in 0..4 {
            let l = format!("s{v}-{j}");
            let next = format!("s{v}-{}", (j + 1) % 4);
            e.insert(
                l.clone(),
                Some(node(&l, Target::None, vec![edge(&next, "")])),
            );
        }
    }
    if v % 5 == 4 {
        let l = format!("s{}-0", v - 2);
        if let Some(mut n) = get(&e, &l) {
            n.edges.push(edge(&pick(rng), ""));
            e.insert(l, Some(node(&n.label.clone(), n.target, n.edges)));
        }
    }
    let l = pick(rng);
    if let Some(n) = get(&e, &l) {
        let t = repo
            .put_content(format!("title {l} v{v}").as_bytes())
            .unwrap()
            .id;
        e.insert(
            l.clone(),
            Some(node(&l, Target::Content(Arg::Whole(t)), n.edges)),
        );
    }
    e
}

fn computes(roots: Vec<String>) -> Vec<Compute> {
    vec![
        Compute::Degree,
        Compute::Hops {
            roots: roots.clone(),
            k: Some(2),
        },
        Compute::Hops { roots, k: None },
        Compute::Components,
        Compute::PageRank {
            iters: 10,
            quantum: 8,
        },
    ]
}

fn sorted<V: Clone + Ord>(m: &HashMap<Key, V>) -> Vec<(Key, V)> {
    let mut v: Vec<(Key, V)> = m.iter().map(|(k, v)| (*k, v.clone())).collect();
    v.sort();
    v
}

#[test]
fn every_incremental_result_equals_a_full_recompute() {
    for seed in [0x9e3779b97f4a7c15, 12345, 0xdead_beef] {
        incremental_equals_full(seed);
    }
}

fn incremental_equals_full(seed: u64) {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Rng(seed);
    let mut g = random_graph(&mut repo, 1500, &mut rng);
    let roots = vec!["n3".to_string(), "n401".to_string(), "n999".to_string()];
    let cs = computes(roots.clone());
    let mut root = repo.put_graph(g.values().cloned().collect()).unwrap();
    let (mut done, _) = repo.graph_compute(&root, &cs).unwrap();
    let mut mem = MemGraph::from_nodes(&g.values().cloned().collect::<Vec<_>>());
    let mut mi: HashMap<Key, Entry> = mem.values(&Compute::Index);
    let mut md: HashMap<Key, Deg> = mem.values(&cs[0]);
    let mut mh: HashMap<Key, u32> = mem.values(&cs[1]);
    let mut mr: HashMap<Key, u32> = mem.values(&cs[2]);
    let mut mc: HashMap<Key, u64> = mem.values(&cs[3]);
    for v in 2..=40 {
        let e = edits(&mut repo, &g, v, &mut rng);
        for (l, n) in &e {
            match n {
                Some(n) => g.insert(l.clone(), n.clone()),
                None => g.remove(l),
            };
        }
        let next = repo.graph_edit(&root, e).unwrap();
        assert_eq!(next, repo.put_graph(g.values().cloned().collect()).unwrap());
        let delta = repo.graph_delta(&root, &next).unwrap();
        let (inc, _) = repo
            .graph_compute_update(&done, &next, Some(&delta), &cs)
            .unwrap();
        let (full, _) = repo.graph_compute(&next, &cs).unwrap();
        assert_eq!(inc, full, "version {v}: incremental differs from full");
        // The in-memory baseline, updated the same way, agrees.
        mem.apply(&delta);
        mem.update_index(&mut mi, &delta);
        mem.update_degree(&mut md, &delta);
        mem.update_hops(&mut mh, &delta, &roots, Some(2));
        mem.update_hops(&mut mr, &delta, &roots, None);
        mem.update_components(&mut mc, &delta);
        let fresh = MemGraph::from_nodes(&g.values().cloned().collect::<Vec<_>>());
        assert_eq!(sorted(&mi), sorted(&fresh.values::<Entry>(&Compute::Index)));
        assert_eq!(sorted(&md), sorted(&fresh.values::<Deg>(&cs[0])));
        assert_eq!(sorted(&mh), sorted(&fresh.values::<u32>(&cs[1])), "v{v}");
        assert_eq!(sorted(&mr), sorted(&fresh.values::<u32>(&cs[2])), "v{v}");
        assert_eq!(sorted(&mc), sorted(&fresh.values::<u64>(&cs[3])), "v{v}");
        let stored: BTreeMap<Key, u64> = repo.graph_values(&inc.results[&cs[3].spec()]).unwrap();
        assert_eq!(stored.into_iter().collect::<Vec<_>>(), sorted(&mc));
        (root, done) = (next, inc);
    }
}

#[test]
fn an_update_reads_only_what_the_delta_reaches() {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Rng(7);
    let g = random_graph(&mut repo, 20_000, &mut rng);
    let root = repo.put_graph(g.values().cloned().collect()).unwrap();
    let cs = vec![
        Compute::Degree,
        Compute::Hops {
            roots: vec!["n5".into()],
            k: Some(2),
        },
        Compute::Components,
    ];
    let (done, full_costs) = repo.graph_compute(&root, &cs).unwrap();
    // One edge added, one removed.
    let mut a = g["n10"].clone();
    a.edges.push(edge("n11", ""));
    let mut b = g["n15000"].clone();
    b.edges.clear();
    let e = BTreeMap::from([
        ("n10".to_string(), Some(a)),
        ("n15000".to_string(), Some(b)),
    ]);
    let next = repo.graph_edit(&root, e).unwrap();
    let delta = repo.graph_delta(&root, &next).unwrap();
    let (inc, costs) = repo
        .graph_compute_update(&done, &next, Some(&delta), &cs)
        .unwrap();
    let (full, _) = repo.graph_compute(&next, &cs).unwrap();
    assert_eq!(inc, full);
    let whole = full_costs[0].chunks_read;
    assert!(whole > 50, "{whole}");
    assert!(delta.chunks_read * 5 < whole, "delta {}", delta.chunks_read);
    for c in &costs {
        // The components update walks the 200-node block that may have
        // split; the index is in key order, so that walk touches index
        // chunks all over (E450's locality finding). The others stay small.
        let bound = if c.spec == "components" { 2 } else { 5 };
        assert!(
            c.chunks_read * bound < whole,
            "{} read {} of {whole}",
            c.spec,
            c.chunks_read
        );
    }
}

#[test]
fn steps_record_inputs_and_outputs_and_gc_keeps_them() {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Rng(11);
    let mut g = random_graph(&mut repo, 600, &mut rng);
    let cs = computes(vec!["n1".into()]);
    let mut root = repo.put_graph(g.values().cloned().collect()).unwrap();
    let (mut done, _) = repo.graph_compute(&root, &cs).unwrap();
    let specs: Vec<String> = std::iter::once(Compute::Index.spec())
        .chain(cs.iter().map(Compute::spec))
        .collect();
    let mut steps: Vec<Option<Id>> = vec![None; specs.len()];
    let mut tree = None;
    let mut all = Vec::new();
    for v in 1..=8 {
        if v > 1 {
            let e = edits(&mut repo, &g, v, &mut rng);
            for (l, n) in &e {
                match n {
                    Some(n) => g.insert(l.clone(), n.clone()),
                    None => g.remove(l),
                };
            }
            let next = repo.graph_edit(&root, e).unwrap();
            (done, _) = repo.graph_compute_update(&done, &next, None, &cs).unwrap();
            root = next;
        }
        let mut tree_edits = vec![Edit::PutGraph {
            path: "graph".into(),
            id: root,
        }];
        for (i, spec) in specs.iter().enumerate() {
            let s = repo.put_graph_step(&done, spec, steps[i]).unwrap();
            steps[i] = Some(s);
            all.push(s);
            tree_edits.push(Edit::PutGraph {
                path: format!("steps/{i}"),
                id: s,
            });
        }
        tree = Some(repo.edit_tree(tree, &tree_edits).unwrap());
        repo.commit("main", tree.unwrap(), "v").unwrap();
    }
    // Junk that nothing holds goes; every step of every version stays.
    repo.put_graph(vec![node("junk", Target::None, vec![])])
        .unwrap();
    assert!(repo.gc().unwrap() > 0);
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.fsck_deep().unwrap().is_empty());
    for s in &all {
        let (c, input, output) = repo.graph_step(s).unwrap();
        assert!(specs.contains(&c.spec()));
        assert!(repo.graph_nodes(&input).is_ok() && repo.graph_nodes(&output).is_ok());
        assert!(repo.graph_step_check(s).unwrap(), "{}", c.spec());
    }
    // A step whose output was not made from its input fails the check.
    let mut wrong = done.clone();
    wrong.results.insert(cs[0].spec(), done.index);
    let bad = repo.put_graph_step(&wrong, &cs[0].spec(), None).unwrap();
    assert!(!repo.graph_step_check(&bad).unwrap());
}

#[test]
fn a_graph_delta_applies_exactly_and_names_its_edges() {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Rng(5);
    let g = random_graph(&mut repo, 3000, &mut rng);
    let root = repo.put_graph(g.values().cloned().collect()).unwrap();
    let mut a = g["n7"].clone();
    a.edges = vec![edge("n8", "x")];
    let e = BTreeMap::from([
        ("n7".to_string(), Some(a.clone())),
        ("n9".to_string(), None),
        (
            "new".to_string(),
            Some(node("new", Target::None, vec![edge("n7", "")])),
        ),
    ]);
    let next = repo.graph_edit(&root, e).unwrap();
    let d: GraphDelta = repo.graph_delta(&root, &next).unwrap();
    assert_eq!(d.added().count(), 1);
    assert_eq!(d.removed().count(), 1);
    assert_eq!(d.changed().count(), 1);
    let (added, removed) = d.edges();
    assert!(added.contains(&("n7".into(), edge("n8", "x"))));
    assert!(added.contains(&("new".into(), edge("n7", ""))));
    assert_eq!(
        removed.iter().filter(|(l, _)| l == "n7").count(),
        g["n7"].edges.len()
    );
    let applied = d.apply(repo.graph_nodes(&root).unwrap());
    assert_eq!(applied, repo.graph_nodes(&next).unwrap());
    let whole = repo.graph_nodes(&root).unwrap().len();
    assert!(d.chunks_read < 20, "{} of {whole}", d.chunks_read);
    assert_eq!(key_of("n7"), u64::from_be_bytes(node_key("n7")));
}
