//! E454: options that make stored graph updates faster (an overlay of the
//! delta on the earlier index, labels shared between a traversal and its
//! result, small results read whole, a link index in segments by commit
//! order). Every option leaves each result's root as a full recompute's.

use std::collections::{BTreeMap, BTreeSet};

use ikam_kernel::graph_compute::{Compute, Layout, UpdateOptions};
use ikam_kernel::graph_derive::LinkSegments;
use ikam_kernel::{Arg, ChunkSizes, Edge, Edit, Id, MemStore, Node, Repo, Store, Target, node_key};

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

fn option_sets() -> Vec<(&'static str, UpdateOptions)> {
    let o = |overlay, shared_labels, whole_result_leaves, lean_reads| UpdateOptions {
        overlay,
        shared_labels,
        whole_result_leaves,
        lean_reads,
        pagerank_kept: overlay,
        sparse_lookups: false,
    };
    let sparse = |o: UpdateOptions| UpdateOptions {
        sparse_lookups: true,
        ..o
    };
    vec![
        ("overlay", o(true, false, 0, false)),
        ("shared", o(false, true, 0, false)),
        ("whole", o(false, false, 3, false)),
        ("lean", o(false, false, 0, true)),
        ("overlay+lean", o(true, false, 0, true)),
        ("shared+whole", o(false, true, 3, false)),
        ("sparse", sparse(UpdateOptions::default())),
        ("all", o(true, true, 3, true)),
        ("all + sparse", sparse(o(true, true, 3, true))),
        (
            "all + sparse, whole always",
            sparse(o(true, true, 1 << 20, true)),
        ),
    ]
}

/// Every option set, in both layouts, with and without E451's cache and
/// small chunks: each incremental result equals a full recompute in a plain
/// store, and the update with no option, in every version.
#[test]
fn every_option_gives_the_full_recomputes_roots() {
    for layout in [Layout::Hash, Layout::Label] {
        for cached in [false, true] {
            for (name, opts) in option_sets() {
                versions_equal(layout, cached, name, opts, 0x454 ^ cached as u64);
            }
        }
    }
}

fn versions_equal(layout: Layout, cached: bool, name: &str, opts: UpdateOptions, seed: u64) {
    let setup = |repo: &mut Repo<MemStore>| {
        repo.graph_result_layout = layout;
        if cached {
            repo.graph_cache = 1 << 12;
            repo.graph_chunks = ChunkSizes::uniform(256, 2048);
        }
    };
    let mut repo = Repo::new(MemStore::default());
    setup(&mut repo);
    repo.graph_update = opts;
    let mut plain = Repo::new(MemStore::default());
    setup(&mut plain);
    let mut rng = Rng(seed);
    let mut g = random_graph(&mut repo, 900, &mut rng);
    random_graph(&mut plain, 900, &mut Rng(seed));
    let cs = computes(vec![
        "n3".to_string(),
        "n401".to_string(),
        "n650".to_string(),
    ]);
    let mut root = repo.put_graph(g.values().cloned().collect()).unwrap();
    let (mut done, _) = repo.graph_compute(&root, &cs).unwrap();
    let mut proot = plain.put_graph(g.values().cloned().collect()).unwrap();
    let (mut pdone, _) = plain.graph_compute(&proot, &cs).unwrap();
    assert_eq!(done, pdone);
    for v in 2..26 {
        let mut prng = Rng(rng.0);
        let e = edits(&mut repo, &g, v, &mut rng);
        let pe = edits(&mut plain, &g, v, &mut prng);
        assert_eq!(e, pe);
        for (l, n) in &e {
            match n {
                Some(n) => g.insert(l.clone(), n.clone()),
                None => g.remove(l),
            };
        }
        let next = repo.graph_edit(&root, e).unwrap();
        let pnext = plain.graph_edit(&proot, pe).unwrap();
        assert_eq!(next, pnext);
        let (inc, _) = repo.graph_compute_update(&done, &next, None, &cs).unwrap();
        let (pinc, _) = plain
            .graph_compute_update(&pdone, &pnext, None, &cs)
            .unwrap();
        let (full, _) = plain.graph_compute(&pnext, &cs).unwrap();
        assert_eq!(
            inc, full,
            "{layout:?} cached={cached} {name}, version {v}: incremental differs from full"
        );
        assert_eq!(inc, pinc);
        root = next;
        done = inc;
        proot = pnext;
        pdone = pinc;
    }
}

/// The delta made from the edits equals the diff of the two versions,
/// edits that change nothing, delete an absent label or repeat an edge
/// included.
#[test]
fn the_delta_of_the_edits_equals_the_diff() {
    for cached in [false, true] {
        let mut repo = Repo::new(MemStore::default());
        if cached {
            repo.graph_cache = 1 << 12;
            repo.graph_chunks = ChunkSizes::uniform(256, 2048);
            repo.graph_update.sparse_lookups = true;
        }
        let mut rng = Rng(0xde17a);
        let mut g = random_graph(&mut repo, 700, &mut rng);
        let mut root = repo.put_graph(g.values().cloned().collect()).unwrap();
        for v in 2..30 {
            let mut e = edits(&mut repo, &g, v, &mut rng);
            // An edit to the same node, an absent label deleted, an edge
            // given twice.
            let same = g.keys().nth(rng.below(g.len())).unwrap().clone();
            e.entry(same.clone())
                .or_insert_with(|| g.get(&same).cloned());
            e.insert(format!("absent{v}"), None);
            let twice = g.keys().nth(rng.below(g.len())).unwrap().clone();
            let base = e
                .get(&twice)
                .cloned()
                .unwrap_or_else(|| g.get(&twice).cloned());
            if let Some(mut n) = base {
                n.edges.push(edge("n1", "x"));
                n.edges.push(edge("n1", "x"));
                n.edges.reverse();
                e.insert(twice.clone(), Some(n));
            }
            let d = repo.graph_delta_of_edits(&root, &e).unwrap();
            for (l, n) in &e {
                match n {
                    Some(n) => {
                        let mut n = n.clone();
                        n.edges.sort();
                        n.edges.dedup();
                        g.insert(l.clone(), n)
                    }
                    None => g.remove(l),
                };
            }
            let (next, d2) = repo.graph_edit_delta(&root, e).unwrap();
            assert_eq!(d.nodes, d2.nodes);
            let diff = repo.graph_delta(&root, &next).unwrap();
            assert_eq!(d.nodes, diff.nodes, "version {v}");
            root = next;
        }
    }
}

fn history(repo: &mut Repo<MemStore>, rng: &mut Rng, commits: usize, tree: &mut Option<Id>) {
    for c in 0..commits {
        let mut edits = Vec::new();
        for _ in 0..3 {
            let path = format!("d{}/f{}", rng.below(20), rng.below(30));
            let id = repo
                .put_content(format!("{path} {c} {}", rng.next()).as_bytes())
                .unwrap()
                .id;
            edits.push(Edit::Put { path, id });
        }
        *tree = Some(repo.edit_tree(*tree, &edits).unwrap());
        repo.commit("main", tree.unwrap(), &format!("c{c}"))
            .unwrap();
    }
}

fn check(src: &Repo<MemStore>, ix_repo: &Repo<MemStore>, ix: &LinkSegments) {
    // Every third id, the scan being linear in the store.
    let ids: BTreeSet<Id> = src.store().ids().unwrap().into_iter().collect();
    for id in ids.into_iter().step_by(3) {
        let mut scan = src.used_by(&id).unwrap();
        scan.sort();
        assert_eq!(ix_repo.used_by_segments(ix, &id).unwrap(), scan, "{id}");
    }
}

/// The segmented link index answers `used_by` as the scan does, through
/// updates, merges, objects lost to gc and objects put back.
#[test]
fn link_segments_answer_used_by() {
    let mut src = Repo::new(MemStore::default());
    let mut ix_repo = Repo::new(MemStore::default());
    let mut rng = Rng(4545);
    let mut tree = None;
    history(&mut src, &mut rng, 30, &mut tree);
    let (mut ix, u) = ix_repo.link_segments_update(&src, None).unwrap();
    assert!(u.added > 30);
    check(&src, &ix_repo, &ix);
    let mut merged = 0;
    for round in 0..8 {
        history(&mut src, &mut rng, 3 + round, &mut tree);
        let (next, u) = ix_repo.link_segments_update(&src, Some(&ix)).unwrap();
        merged += u.merged;
        ix = next;
        check(&src, &ix_repo, &ix);
    }
    assert!(merged > 0 && ix.segments.len() < 9, "{:?}", ix.segments);
    // Trees no ref holds: indexed, deleted by gc, then one put back.
    let mut loose = tree;
    let mut held = Vec::new();
    for i in 0..5 {
        let id = src.put_content(format!("loose {i}").as_bytes()).unwrap().id;
        let path = format!("loose/{i}");
        loose = Some(src.edit_tree(loose, &[Edit::Put { path, id }]).unwrap());
        held.push((id, loose.unwrap()));
    }
    let (next, _) = ix_repo.link_segments_update(&src, Some(&ix)).unwrap();
    ix = next;
    check(&src, &ix_repo, &ix);
    assert!(src.gc().unwrap() > 0);
    let (next, u) = ix_repo.link_segments_update(&src, Some(&ix)).unwrap();
    assert!(u.removed > 0);
    ix = next;
    check(&src, &ix_repo, &ix);
    // Put the first loose tree back, as it was (the same id).
    let (blob, _) = held[0];
    src.put_content(b"loose 0").unwrap();
    let back = src
        .edit_tree(
            tree,
            &[Edit::Put {
                path: "loose/0".into(),
                id: blob,
            }],
        )
        .unwrap();
    let _ = back;
    let (next, _) = ix_repo.link_segments_update(&src, Some(&ix)).unwrap();
    ix = next;
    check(&src, &ix_repo, &ix);
    // Merging everything keeps the answers.
    for _ in 0..4 {
        history(&mut src, &mut rng, 20, &mut tree);
        let (next, _) = ix_repo.link_segments_update(&src, Some(&ix)).unwrap();
        ix = next;
    }
    check(&src, &ix_repo, &ix);
}
