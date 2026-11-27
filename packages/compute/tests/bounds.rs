//! Checks of bounds on nondeterminism (E442): `Op::Within` encodes and
//! decodes, graphs without it keep the ids they had before it existed,
//! the exact check is unchanged, a bounded check reports same, within
//! and outside, an uncovered difference is still an error, local bounds
//! restart from the record, a check writes nothing, and the predicted
//! bound covers the measured difference.

use std::collections::BTreeMap;

use ikam_compute::bounds::{self, Verdict, coverage, distance, insert_within, predict, ulp_distance};
use ikam_compute::eval::{Arith, Env, eval, eval_all, eval_all_with, eval_with};
use ikam_compute::grad::grad;
use ikam_compute::ir::{Graph, Op, V};
use ikam_compute::memo::{check_bounded, check_graph_bounded, eval_graph_stored, eval_stored};
use ikam_compute::plan::Plan;
use ikam_compute::{Error, Tensor};
use ikam_kernel::{MemStore, Repo, Store};

/// The graph whose ids were taken on the parent commit (before `Within`).
fn fixed() -> (Graph, V, BTreeMap<String, Vec<usize>>, Env) {
    let mut g = Graph::new();
    let w = g.input("w");
    let x = g.input("x");
    let u = g.input("u");
    let y = g.push(Op::MatMul, &[w, x]);
    let yt = g.push(Op::Transpose, &[y]);
    let ys = g.unary(Op::Sin, yt);
    let ye = g.unary(Op::Exp, ys);
    let a = g.mul(ye, yt);
    let fl = g.scalar(0.1);
    let m = g.max(yt, fl);
    let a = g.add(a, m);
    let s1 = g.sum(a);
    let c = g.cumsum(u);
    let sl = g.slice(c, 1, 3);
    let p = g.concat(&[sl, u]);
    let q = g.unary(Op::Ln, p);
    let r = g.unary(Op::Sqrt, q);
    let th = g.unary(Op::Tanh, r);
    let co = g.unary(Op::Cos, th);
    let z = g.div(co, s1);
    let pad = g.push(Op::Pad { start: 2, total: 10 }, &[z]);
    let neg = g.neg(pad);
    let s2 = g.sum(neg);
    let col = g.push(Op::Reshape { shape: vec![5, 1] }, &[u]);
    let gt = g.push(Op::Gt, &[s2, s1]);
    let f = g.sub(s2, gt);
    let fill = g.push(Op::Fill { shape: vec![2] }, &[f]);
    let rc = g.push(Op::RevCumsum, &[fill]);
    let k = g.konst(Tensor::vector(vec![1.5, -0.0]));
    let o = g.add(rc, k);
    let o = g.sum(o);
    g.outputs = vec![o, col];
    let shapes = BTreeMap::from([("w".into(), vec![2, 3]), ("x".into(), vec![3, 1]), ("u".into(), vec![5])]);
    let env = Env::from([
        ("w".into(), Tensor::new(vec![2, 3], vec![0.3, -0.2, 0.5, 0.1, 0.4, -0.6]).unwrap()),
        ("x".into(), Tensor::new(vec![3, 1], vec![1.0, 0.5, -0.25]).unwrap()),
        ("u".into(), Tensor::vector(vec![2.2, 1.7, 3.1, 1.05, 1.3])),
    ]);
    (g, o, shapes, env)
}

/// A sum whose order decides its bits: huge terms that cancel with small
/// ones between (E409's "cancel"), so `Arith::Host`'s 8 lanes differ.
fn cancelling(n: usize) -> Tensor {
    let mut s = 409u64;
    let mut unit = || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((s >> 11) as f64) / ((1u64 << 52) as f64) - 1.0
    };
    Tensor::vector(
        (0..n)
            .map(|i| {
                let x = 10.0 * unit();
                match i % 4 {
                    0 => 1e10 * (1.0 + x * 1e-4),
                    2 => -1e10 * (1.0 + x * 1e-4),
                    _ => x,
                }
            })
            .collect(),
    )
}

#[test]
fn within_round_trips_through_its_encoding() {
    for op in [
        Op::Within { ulp: 0, abs: 0.0, rel: 0.0, local: false },
        Op::Within { ulp: 4, abs: 1e-12, rel: 2.5e-15, local: true },
        Op::Within { ulp: u64::MAX, abs: f64::INFINITY, rel: 1e300, local: false },
    ] {
        let back = Op::decode(&op.encode()).unwrap();
        assert_eq!(back.encode(), op.encode());
        match (&op, &back) {
            (Op::Within { ulp, abs, rel, local }, Op::Within { ulp: u2, abs: a2, rel: r2, local: l2 }) => {
                assert_eq!((ulp, abs.to_bits(), rel.to_bits(), local), (u2, a2.to_bits(), r2.to_bits(), l2));
            }
            _ => panic!("decoded {back:?}"),
        }
    }
    assert!(Op::decode(b"within:1:2:3").is_err());
    assert!(Op::decode(b"within:1:2:3:2").is_err());
    // Within is a node like any other: stored, loaded, the same graph.
    let mut g = Graph::new();
    let x = g.input("x");
    let b = g.within(x, 3, 1e-9, 0.0, true);
    g.outputs = vec![b];
    let mut repo = Repo::new(MemStore::default());
    let id = g.put(&mut repo).unwrap();
    let back = Graph::load(&repo, &id).unwrap();
    assert_eq!(back.put(&mut repo).unwrap(), id);
    assert_eq!(back.nodes[1].op.encode(), g.nodes[1].op.encode());
}

#[test]
fn graphs_without_bounds_keep_their_ids() {
    // Ids printed by the same graphs on the parent commit, before Within.
    let (g, o, shapes, _) = fixed();
    let mut repo = Repo::new(MemStore::default());
    assert_eq!(g.put(&mut repo).unwrap().to_hex(), "67f334e344d2fb8e1d8bc1d890dfb77ce29723e2b78278d148d2ad8108a58211");
    let gg = grad(&g, &shapes, o, &["w", "x", "u"]).unwrap();
    assert_eq!(gg.put(&mut repo).unwrap().to_hex(), "19a5c630182cc30ee7d3b4794cd1e37011bbf0b81b426ea96ec71aab111beb64");
    for (op, bytes) in [
        (Op::Add, "add"),
        (Op::Slice { start: 1, len: 3 }, "slice:1:3"),
        (Op::Fill { shape: vec![2, 3] }, "fill:2:3"),
        (Op::MatMul, "matmul"),
    ] {
        assert_eq!(op.encode(), bytes.as_bytes());
    }
}

#[test]
fn a_bound_is_the_identity_in_value_gradient_and_plan() {
    let (g, o, shapes, env) = fixed();
    let (h, map) = insert_within(&g, &[o, 4, 13], &Op::Within { ulp: 2, abs: 0.0, rel: 0.0, local: true }).unwrap();
    assert_eq!(h.nodes.len(), g.nodes.len() + 3);
    let (a, b) = (eval(&g, &env).unwrap(), eval(&h, &env).unwrap());
    for (x, y) in a.iter().zip(&b) {
        assert!(x.bits_eq(y));
    }
    let (ga, gb) = (grad(&g, &shapes, o, &["w", "u"]).unwrap(), grad(&h, &shapes, h.outputs[0], &["w", "u"]).unwrap());
    for (x, y) in eval(&ga, &env).unwrap().iter().zip(&eval(&gb, &env).unwrap()) {
        assert!(x.bits_eq(y));
    }
    let mut plan = Plan::new(&h, &shapes).unwrap();
    for (name, v) in &env {
        plan.set(name, &v.data).unwrap();
    }
    plan.run();
    assert!(plan.output_tensor(0).bits_eq(&a[0]));
    assert!(matches!(h.nodes[map[o]].op, Op::Within { .. }));
    // Pinned is eval_all.
    for (x, y) in eval_all(&g, &env).unwrap().iter().zip(&eval_all_with(&g, &env, Arith::Pinned).unwrap()) {
        assert!(x.bits_eq(y));
    }
}

#[test]
fn coverage_needs_a_bound_on_every_path() {
    let mut g = Graph::new();
    let x = g.input("x");
    let a = g.sum(x); // 1: one path, through the bound
    let b = g.within(a, 0, 0.0, 0.0, false); // 2
    let c = g.unary(Op::Exp, x); // 3: to the bound and straight to an output
    let d = g.sum(c); // 4
    let e = g.add(b, d); // 5: an output, not a bound
    let f = g.within(e, 0, 0.0, 0.0, false); // 6
    g.outputs = vec![f, c];
    let cov = coverage(&g);
    // The input reaches an output through `c`, so it is not covered either
    // (inputs never differ: they are given).
    assert_eq!(cov, vec![false, true, true, false, true, true, true]);
}

#[test]
fn the_exact_check_is_unchanged_and_an_uncovered_difference_is_refused() {
    let x = cancelling(1000);
    let env = Env::from([("x".into(), x)]);
    let mut g = Graph::new();
    let xi = g.input("x");
    let s = g.sum(xi);
    g.outputs = vec![s];
    assert!(!eval_with(&g, &env, Arith::Host).unwrap()[0].bits_eq(&eval(&g, &env).unwrap()[0]), "host must differ");
    let mut repo = Repo::new(MemStore::default());
    let db = repo.db_empty().unwrap();
    let (ids, db, _) = eval_stored(&mut repo, &db, &g, &env, false).unwrap();
    // The exact check and a pinned bounded check agree with the record.
    let (ids2, db2, s) = eval_stored(&mut repo, &db, &g, &env, true).unwrap();
    assert_eq!((ids2, db2, s.misses), (ids.clone(), db, 0));
    let rep = check_bounded(&repo, &db, &g, &env, Arith::Pinned).unwrap();
    assert!(rep.same() && rep.bounds.is_empty() && rep.passed());
    // Without a bound, a rerun with other arithmetic is refused as before.
    match check_bounded(&repo, &db, &g, &env, Arith::Host) {
        Err(Error::Nondeterministic { recorded, .. }) => assert_eq!(recorded, ids[0]),
        other => panic!("expected nondeterminism, got {other:?}"),
    }
    // A graph never recorded has nothing to check against.
    let empty = repo.db_empty().unwrap();
    assert!(matches!(check_bounded(&repo, &empty, &g, &env, Arith::Pinned), Err(Error::NotRecorded(_))));
}

#[test]
fn a_bounded_check_reports_within_and_outside_and_writes_nothing() {
    let env = Env::from([("x".into(), cancelling(1000))]);
    let (pinned, host) = {
        let mut g = Graph::new();
        let xi = g.input("x");
        let s = g.sum(xi);
        g.outputs = vec![s];
        (eval(&g, &env).unwrap()[0].item(), eval_with(&g, &env, Arith::Host).unwrap()[0].item())
    };
    let moved = (pinned - host).abs();
    let ulps = ulp_distance(pinned, host);
    assert!(moved > 0.0 && ulps > 0);
    for (tol, want) in [
        ((ulps, 0.0, 0.0), Verdict::Within),
        ((ulps - 1, 0.0, 0.0), Verdict::Outside),
        ((0, moved, 0.0), Verdict::Within),
        ((0, moved * 0.5, 0.0), Verdict::Outside),
        ((0, 0.0, 1.01 * moved / pinned.abs()), Verdict::Within),
        ((0, 0.0, 0.0), Verdict::Outside),
    ] {
        let mut g = Graph::new();
        let xi = g.input("x");
        let s = g.sum(xi);
        let b = g.within(s, tol.0, tol.1, tol.2, false);
        let e = g.unary(Op::Exp, b);
        let b2 = g.within(e, 0, f64::INFINITY, 0.0, false);
        g.outputs = vec![b2];
        let mut repo = Repo::new(MemStore::default());
        let db = repo.db_empty().unwrap();
        let (_, db, _) = eval_stored(&mut repo, &db, &g, &env, false).unwrap();
        let before = repo.store().ids().unwrap();
        let rep = check_bounded(&repo, &db, &g, &env, Arith::Host).unwrap();
        assert_eq!(rep.bounds[0].verdict, want, "{tol:?}");
        assert_eq!(rep.bounds[0].distance.max_ulp, ulps);
        assert_eq!(rep.bounds.len(), 2);
        assert_eq!(rep.passed(), want == Verdict::Within);
        // Nothing written (the check takes the repository shared): the
        // store holds the same objects, and a plain rerun reuses every
        // record of the first run.
        assert_eq!(repo.store().ids().unwrap(), before);
        let (_, db2, s) = eval_stored(&mut repo, &db, &g, &env, false).unwrap();
        assert_eq!((db2, s.misses), (db, 0));
        // The same through the whole-graph record.
        let gid = g.put(&mut repo).unwrap();
        let gdb = repo.db_empty().unwrap();
        let (_, gdb, _) = eval_graph_stored(&mut repo, &gdb, &g, &gid, &env, false).unwrap();
        let rep = check_graph_bounded(&repo, &gdb, &g, &gid, &env, Arith::Host).unwrap();
        assert_eq!(rep.bounds.len(), 1);
        assert_eq!(rep.bounds[0].verdict, if rep.differing == 0 { Verdict::Same } else { Verdict::Within });
    }
    // A whole-graph record whose differing output is not a bound is refused.
    let mut g = Graph::new();
    let xi = g.input("x");
    let s = g.sum(xi);
    let b = g.within(s, u64::MAX, 0.0, 0.0, false);
    g.outputs = vec![s, b];
    let mut repo = Repo::new(MemStore::default());
    let gid = g.put(&mut repo).unwrap();
    let db = repo.db_empty().unwrap();
    let (_, db, _) = eval_graph_stored(&mut repo, &db, &g, &gid, &env, false).unwrap();
    assert!(matches!(
        check_graph_bounded(&repo, &db, &g, &gid, &env, Arith::Host),
        Err(Error::Nondeterministic { .. })
    ));
}

/// A chain of steps x <- x + h * sum(x) * c, each step bounded.
fn chain(steps: usize, local: bool, tol: (u64, f64, f64)) -> Graph {
    let mut g = Graph::new();
    let mut x = g.input("x");
    let h = g.scalar(1e-3);
    for _ in 0..steps {
        let s = g.sum(x);
        let t = g.mul(h, s);
        let y = g.unary(Op::Sin, x);
        let z = g.mul(t, y);
        let n = g.add(x, z);
        x = g.within(n, tol.0, tol.1, tol.2, local);
    }
    g.outputs = vec![x];
    g
}

#[test]
fn a_local_bound_restarts_from_the_record() {
    let env = Env::from([("x".into(), cancelling(64))]);
    let steps = 6;
    let mut repo = Repo::new(MemStore::default());
    let db0 = repo.db_empty().unwrap();
    let carried = chain(steps, false, (u64::MAX, 0.0, 0.0));
    let (_, db, _) = eval_stored(&mut repo, &db0, &carried, &env, false).unwrap();
    let rc = check_bounded(&repo, &db, &carried, &env, Arith::Host).unwrap();
    let local = chain(steps, true, (u64::MAX, 0.0, 0.0));
    let (_, db, _) = eval_stored(&mut repo, &db0, &local, &env, false).unwrap();
    let rl = check_bounded(&repo, &db, &local, &env, Arith::Host).unwrap();
    assert_eq!((rc.bounds.len(), rl.bounds.len()), (steps, steps));
    // The first step is the same either way; after it a local check sees
    // only each step's own difference, a carried one the sum of them.
    assert_eq!(rc.bounds[0].distance, rl.bounds[0].distance);
    // Each local step, recomputed from the recorded state, is what one
    // host step from the pinned state gives.
    let pinned = eval_all(&chain(steps, true, (0, 0.0, 0.0)), &env).unwrap();
    for (k, b) in rl.bounds.iter().enumerate() {
        let before = if k == 0 { env["x"].clone() } else { pinned[rl.bounds[k - 1].node].clone() };
        let one = chain(1, true, (0, 0.0, 0.0));
        let e1 = Env::from([("x".into(), before)]);
        let host = eval_with(&one, &e1, Arith::Host).unwrap().remove(0);
        assert_eq!(b.distance, distance(&pinned[b.node], &host), "step {k}");
    }
    let last = |r: &bounds::Report| r.bounds.last().unwrap().distance.max_abs;
    assert!(last(&rc) >= last(&rl), "{} < {}", last(&rc), last(&rl));
}

#[test]
fn the_prediction_covers_the_measured_difference() {
    let (g, _, _, env) = fixed();
    let errs = predict(&g, &env, &BTreeMap::new()).unwrap();
    let diff = bounds::predicted_difference(&errs);
    let (p, h) = (eval_all(&g, &env).unwrap(), eval_all_with(&g, &env, Arith::Host).unwrap());
    for (i, (a, b)) in p.iter().zip(&h).enumerate() {
        assert!(distance(a, b).max_abs <= diff[i], "node {i}: {} > {}", distance(a, b).max_abs, diff[i]);
    }
    let env = Env::from([("x".into(), cancelling(1000))]);
    let c = chain(20, false, (0, 0.0, 0.0));
    let errs = predict(&c, &env, &BTreeMap::new()).unwrap();
    let diff = bounds::predicted_difference(&errs);
    let (p, h) = (eval_all(&c, &env).unwrap(), eval_all_with(&c, &env, Arith::Host).unwrap());
    for (i, (a, b)) in p.iter().zip(&h).enumerate() {
        assert!(distance(a, b).max_abs <= diff[i], "node {i}: {} > {}", distance(a, b).max_abs, diff[i]);
    }
    // Filling a bound from the prediction passes the host rerun.
    let filled = bounds::fill_from_prediction(&c, &env, &BTreeMap::new(), 1.0).unwrap();
    let mut repo = Repo::new(MemStore::default());
    let db = repo.db_empty().unwrap();
    let (_, db, _) = eval_stored(&mut repo, &db, &filled, &env, false).unwrap();
    let rep = check_bounded(&repo, &db, &filled, &env, Arith::Host).unwrap();
    assert!(rep.passed() && rep.differing > 0, "{rep:?}");
}
