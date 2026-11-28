//! E452: graph derivations (a graph-root form of `Arg` and of an output),
//! the read interface graph functions get, results labelled by the input's
//! labels, and the link index that answers `used_by`. All options, off by
//! default: content records encode as before.

use std::collections::{BTreeMap, BTreeSet};

use ikam_kernel::graph_compute::{Compute, Deg, Layout};
use ikam_kernel::graph_derive::{GraphFn, GraphRead};
use ikam_kernel::{
    Arg, Check, Claim, Derivation, Edge, Edit, Error, GraphDerivation, Id, MemStore, Node, Object,
    Repo, Run, Store, Target, node_key,
};

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

fn id(n: u8) -> Id {
    Id::from_bytes([n; 32])
}

/// Content arguments encode exactly as before E452 (tags 0 and 1, built
/// here by hand), records that hold them keep their ids, and a graph root
/// is refused wherever content is meant.
#[test]
fn content_args_encode_as_before_and_graph_args_only_in_graph_derivations() {
    let (out, f, a, b) = (id(1), id(2), id(3), id(4));
    let args = vec![
        Arg::Whole(a),
        Arg::Range {
            id: b,
            start: 7,
            len: 9,
        },
    ];
    let mut want = vec![b'D'];
    want.extend_from_slice(out.as_bytes());
    want.extend_from_slice(f.as_bytes());
    want.extend_from_slice(&2u32.to_be_bytes());
    want.push(0);
    want.extend_from_slice(a.as_bytes());
    want.push(1);
    want.extend_from_slice(b.as_bytes());
    want.extend_from_slice(&7u64.to_be_bytes());
    want.extend_from_slice(&9u64.to_be_bytes());
    let d = Object::Derivation(Derivation {
        output: out,
        func: f,
        args: args.clone(),
    });
    assert_eq!(d.encode(), want);
    // The record's id, as main gives it.
    assert_eq!(
        d.id().to_string(),
        "40f1fd21e2a6ddedc85ef634a928a39312d64f684cffca3b47062817c2ce8121"
    );
    let r = Object::Run(Run {
        output: out,
        func: f,
        args: args.clone(),
        check: Some(Check {
            func: f,
            args: vec![Arg::Whole(a)],
        }),
    });
    let mut want_run = vec![b'E'];
    want_run.extend_from_slice(&want[1..]);
    want_run.push(1);
    want_run.extend_from_slice(f.as_bytes());
    want_run.extend_from_slice(&1u32.to_be_bytes());
    want_run.push(0);
    want_run.extend_from_slice(a.as_bytes());
    assert_eq!(r.encode(), want_run);
    for o in [&d, &r] {
        assert_eq!(&Object::decode(&o.encode()).unwrap(), o);
    }
    // A graph derivation: tag H, graph arguments tagged 2.
    let g = Object::GraphDerivation(GraphDerivation {
        output: out,
        func: f,
        args: vec![Arg::Graph(a), Arg::Whole(b)],
    });
    let e = g.encode();
    assert_eq!(e[0], b'H');
    assert_eq!(e[69], 2);
    assert_eq!(Object::decode(&e).unwrap(), g);
    // The same bytes under a content record's tag do not decode.
    let mut as_d = e.clone();
    as_d[0] = b'D';
    assert!(Object::decode(&as_d).is_err());
    let claim = Object::Claim(Claim {
        subject: Arg::Graph(a),
        predicate: "p".into(),
        object: Arg::Whole(b),
        gain_bits: 0,
        by: None,
    });
    assert!(Object::decode(&claim.encode()).is_err());
    // Content functions refuse a graph argument.
    let mut repo = Repo::new(MemStore::default());
    let root = repo
        .put_graph(vec![node("x", Target::None, vec![])])
        .unwrap();
    assert!(
        repo.apply(ikam_kernel::func::concat(), vec![Arg::Graph(root)])
            .is_err()
    );
}

/// With results labelled by the input's labels, every incremental result
/// still equals a full recompute, and its values equal the default
/// layout's.
#[test]
fn label_layout_incremental_equals_full() {
    let mut repo = Repo::new(MemStore::default());
    repo.graph_result_layout = Layout::Label;
    let mut hashed = Repo::new(MemStore::default());
    let mut rng = Rng(0x51ed_270b_f00d);
    let mut g = random_graph(&mut repo, 1200, &mut rng);
    let roots = vec!["n3".to_string(), "n401".to_string()];
    let cs = computes(roots);
    let mut root = repo.put_graph(g.values().cloned().collect()).unwrap();
    let (mut done, _) = repo.graph_compute(&root, &cs).unwrap();
    for v in 2..30 {
        let e = edits(&mut repo, &g, v, &mut rng);
        for (l, n) in &e {
            match n {
                Some(n) => g.insert(l.clone(), n.clone()),
                None => g.remove(l),
            };
        }
        let next = repo.graph_edit(&root, e).unwrap();
        let (inc, _) = repo.graph_compute_update(&done, &next, None, &cs).unwrap();
        let (full, _) = repo.graph_compute(&next, &cs).unwrap();
        assert_eq!(inc, full, "version {v}: incremental differs from full");
        // Ordered by the input's labels, not by key.
        let nodes = repo.graph_nodes(&inc.results["degree"]).unwrap();
        assert!(nodes.iter().any(|n| g.contains_key(&n.label)));
        let degree: BTreeMap<_, Deg> = repo.graph_values(&inc.results["degree"]).unwrap();
        let h = hashed.put_graph(g.values().cloned().collect()).unwrap();
        let (hd, _) = hashed.graph_compute_one(&h, &cs[0]).unwrap();
        let theirs: BTreeMap<_, Deg> = hashed.graph_values_as(&hd, Layout::Hash).unwrap();
        assert_eq!(
            degree, theirs,
            "version {v}: values differ from the hash layout"
        );
        root = next;
        done = inc;
    }
    let bad = repo
        .put_graph(vec![node("\0x", Target::None, vec![])])
        .unwrap();
    assert!(repo.graph_compute(&bad, &cs).is_err());
}

/// Computations as graph derivations: each version's result is recorded as
/// `(spec, input root) -> output`, reached incrementally from the version
/// before and equal to a full recompute; a rerun is a memo hit; a graph
/// root is never content; gc keeps a live output's whole input graph and
/// records; fsck_deep rebuilds them, and catches a wrong one.
#[test]
fn graph_derivations_memo_gc_and_fsck() {
    for layout in [Layout::Hash, Layout::Label] {
        derivations(layout);
    }
}

fn derivations(layout: Layout) {
    let mut repo = Repo::new(MemStore::default());
    repo.graph_result_layout = layout;
    let mut rng = Rng(0x1234_5678_9abc);
    let mut g = random_graph(&mut repo, 900, &mut rng);
    let cs = computes(vec!["n5".to_string(), "n700".to_string()]);
    let mut root = repo.put_graph(g.values().cloned().collect()).unwrap();
    for c in &cs {
        let d = repo.graph_derive(c, &root).unwrap();
        assert!(!d.hit);
        assert_eq!(d.output, repo.graph_compute_one(&root, c).unwrap().0);
    }
    let mut last = BTreeMap::new();
    for v in 2..12 {
        let e = edits(&mut repo, &g, v, &mut rng);
        for (l, n) in &e {
            match n {
                Some(n) => g.insert(l.clone(), n.clone()),
                None => g.remove(l),
            };
        }
        let next = repo.graph_edit(&root, e).unwrap();
        for c in &cs {
            let d = repo.graph_derive_from(c, &next, &root, None).unwrap();
            assert!(!d.hit, "version {v}: {} hit before it ran", c.spec());
            let (full, _) = repo.graph_compute_one(&next, c).unwrap();
            assert_eq!(
                d.output,
                full,
                "version {v}: {} differs from full",
                c.spec()
            );
            // A rerun with the input unchanged is a memo hit.
            let again = repo.graph_derive(c, &next).unwrap();
            assert!(again.hit);
            assert_eq!((again.output, again.record), (d.output, d.record));
            last.insert(c.spec(), d);
        }
        root = next;
    }
    let index = repo.graph_derive(&Compute::Index, &root).unwrap();
    assert!(index.hit);
    for d in last.values().chain([&index]) {
        // Not content: no content derivation names it, and it does not read
        // as bytes.
        assert!(repo.derivations(&d.output).unwrap().is_empty());
        assert!(repo.read_content(&d.output).is_err());
        let recs = repo.graph_derivations(&d.output).unwrap();
        assert!(recs.iter().any(|(r, _)| *r == d.record));
    }
    // Keep only the last version's outputs through a commit: gc keeps their
    // records, the whole input graph (and its payloads) and the index.
    let edits: Vec<Edit> = (last.values().chain([&index]).enumerate())
        .map(|(i, d)| Edit::PutGraph {
            path: format!("out/{i}"),
            id: d.output,
        })
        .collect();
    let tree = repo.edit_tree(None, &edits).unwrap();
    repo.commit("main", tree, "outputs").unwrap();
    let deleted = repo.gc().unwrap();
    assert!(deleted > 0);
    let nodes = repo.graph_nodes(&root).unwrap();
    assert_eq!(nodes.len(), g.len());
    for n in &nodes {
        if let Target::Content(a) = &n.target {
            repo.read_content(&a.id()).unwrap();
        }
    }
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.fsck_deep().unwrap().is_empty());
    for c in &cs {
        assert!(
            repo.graph_derive(c, &root).unwrap().hit,
            "{} lost by gc",
            c.spec()
        );
    }
    // A record whose output is another graph passes fsck, not fsck_deep.
    let func = repo
        .put_content(layout.func_text("degree").as_bytes())
        .unwrap()
        .id;
    let wrong = repo
        .record_graph_derivation(GraphDerivation {
            output: last["components"].output,
            func,
            args: vec![Arg::Graph(root)],
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    let _ = wrong;
    assert!(repo.fsck().unwrap().is_empty());
    let deep = repo.fsck_deep().unwrap();
    assert_eq!(deep.len(), 1, "{deep:?}");
    // A record naming a graph that is not stored is refused.
    let missing = repo.record_graph_derivation(GraphDerivation {
        output: Id::from_bytes([9; 32]),
        func,
        args: vec![Arg::Graph(root)],
    });
    assert!(matches!(missing, Err(Error::NotFound(_))));
}

/// In-edges counted through the read interface: a native graph function.
struct InCount;

impl GraphFn for InCount {
    fn name(&self) -> String {
        "in-count".into()
    }
    fn run(&self, g: &mut dyn GraphRead) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        for n in g.range("", None)? {
            let inn = g.in_edges(&n.label)?;
            let outs = g.out_neighbours(&n.label)?;
            out.push(node(
                &n.label,
                Target::None,
                vec![edge("", &format!("{} {}", inn.len(), outs.len()))],
            ));
        }
        Ok(out)
    }
}

#[test]
fn a_graph_function_reads_through_the_interface_and_is_memoized() {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Rng(77);
    let g = random_graph(&mut repo, 600, &mut rng);
    let root = repo.put_graph(g.values().cloned().collect()).unwrap();
    let first = repo.graph_derive_fn(&InCount, &root).unwrap();
    assert!(!first.hit && first.cost.chunks_read > 0);
    let again = repo.graph_derive_fn(&InCount, &root).unwrap();
    assert!(again.hit);
    assert_eq!(again.output, first.output);
    // Against the in-memory degrees.
    let (deg, _) = repo.graph_compute_one(&root, &Compute::Degree).unwrap();
    let deg: BTreeMap<_, Deg> = repo.graph_values(&deg).unwrap();
    for n in repo.graph_nodes(&first.output).unwrap() {
        let k = ikam_kernel::graph_compute::key_of(&n.label);
        let inn: u32 = n.edges[0].label.split(' ').next().unwrap().parse().unwrap();
        assert_eq!(inn, deg[&k].inn, "{}", n.label);
    }
    assert!(repo.fsck_deep().unwrap().is_empty());
}

/// A small history: trees of files under directories, commits on top.
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

#[test]
fn link_index_answers_used_by() {
    let mut src = Repo::new(MemStore::default());
    let mut ix_repo = Repo::new(MemStore::default());
    let mut rng = Rng(4242);
    let mut tree = None;
    history(&mut src, &mut rng, 40, &mut tree);
    let check = |src: &Repo<MemStore>, ix_repo: &Repo<MemStore>, ix| {
        let ids: BTreeSet<Id> = src.store().ids().unwrap().into_iter().collect();
        for id in ids {
            let mut scan = src.used_by(&id).unwrap();
            scan.sort();
            assert_eq!(ix_repo.used_by_indexed(ix, &id).unwrap(), scan, "{id}");
        }
    };
    let (ix, u) = ix_repo.link_index_update(&src, None).unwrap();
    assert!(u.added > 40);
    check(&src, &ix_repo, &ix);
    history(&mut src, &mut rng, 15, &mut tree);
    let (ix, u) = ix_repo.link_index_update(&src, Some(&ix)).unwrap();
    assert!(u.added > 15 && u.removed == 0);
    check(&src, &ix_repo, &ix);
    // Trees no ref holds: indexed now, deleted by gc, then dropped.
    let mut loose = tree;
    for i in 0..5 {
        let id = src.put_content(format!("loose {i}").as_bytes()).unwrap().id;
        let path = format!("loose/{i}");
        loose = Some(src.edit_tree(loose, &[Edit::Put { path, id }]).unwrap());
    }
    let (ix, _) = ix_repo.link_index_update(&src, Some(&ix)).unwrap();
    check(&src, &ix_repo, &ix);
    assert!(src.gc().unwrap() > 0);
    let (ix, u) = ix_repo.link_index_update(&src, Some(&ix)).unwrap();
    assert!(u.removed > 0);
    check(&src, &ix_repo, &ix);
}

/// A WASM graph function: it gets the label to look at as content and reads
/// the graph only through the read-only host import `in_edges`, returning
/// each source with its edge label.
const IN_EDGES_WAT: &str = r#"
(module
  (import "ikam_graph" "in_edges" (func $in (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 4)
  (global $heap (mut i32) (i32.const 4096))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (local.get $p))
  (func $be (param $p i32) (result i32)
    (i32.or
      (i32.or (i32.shl (i32.load8_u (local.get $p)) (i32.const 24))
              (i32.shl (i32.load8_u offset=1 (local.get $p)) (i32.const 16)))
      (i32.or (i32.shl (i32.load8_u offset=2 (local.get $p)) (i32.const 8))
              (i32.load8_u offset=3 (local.get $p)))))
  (func (export "run") (param $ptr i32) (param $len i32) (result i64)
    (local $n i32)
    (local.set $n (call $in
      (i32.add (local.get $ptr) (i32.const 8))
      (call $be (i32.add (local.get $ptr) (i32.const 4)))
      (i32.const 65536) (i32.const 131072)))
    (if (i32.lt_s (local.get $n) (i32.const 0)) (then (local.set $n (i32.const 0))))
    (i64.or (i64.shl (i64.const 65536) (i64.const 32)) (i64.extend_i32_u (local.get $n)))))
"#;

#[test]
fn a_wasm_graph_function_reads_through_host_imports() {
    let mut repo = Repo::new(MemStore::default());
    let mut rng = Rng(99);
    let g = random_graph(&mut repo, 500, &mut rng);
    let root = repo.put_graph(g.values().cloned().collect()).unwrap();
    let module = repo
        .put_content(&wat::parse_str(IN_EDGES_WAT).unwrap())
        .unwrap()
        .id;
    let target = "n7";
    let label = repo.put_content(target.as_bytes()).unwrap().id;
    let d = repo.graph_derive_wasm(&module, &root, &[label]).unwrap();
    assert!(!d.hit);
    let want: BTreeSet<String> = (g.values())
        .filter(|n| n.edges.iter().any(|e| e.to == node_key(target)))
        .map(|n| n.label.clone())
        .collect();
    let got: BTreeSet<String> = (repo.graph_nodes(&d.output).unwrap().into_iter())
        .map(|n| n.label)
        .collect();
    assert!(!want.is_empty());
    assert_eq!(got, want);
    let again = repo.graph_derive_wasm(&module, &root, &[label]).unwrap();
    assert!(again.hit && again.output == d.output);
    assert!(repo.fsck_deep().unwrap().is_empty());
    // A module that imports anything else is refused.
    let bad = wat::parse_str(
        r#"(module (import "env" "clock" (func)) (memory (export "memory") 1)
        (func (export "alloc") (param i32) (result i32) (i32.const 0))
        (func (export "run") (param i32 i32) (result i64) (i64.const 0)))"#,
    )
    .unwrap();
    let bad = repo.put_content(&bad).unwrap().id;
    assert!(repo.graph_derive_wasm(&bad, &root, &[label]).is_err());
    // A record whose output is not what the module gives fails fsck_deep.
    repo.record_graph_derivation(GraphDerivation {
        output: root,
        func: module,
        args: vec![Arg::Graph(root), Arg::Whole(label)],
    })
    .unwrap();
    assert_eq!(repo.fsck_deep().unwrap().len(), 1);
}
