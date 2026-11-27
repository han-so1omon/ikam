//! Checks of the compute layer: gradients and Hessian-vector products
//! against finite differences, graphs stored and loaded as ikam graph
//! objects, stored evaluation (memo hits, partial reuse, a tampered
//! record caught), the planned program against the interpreter bit for
//! bit, Newton-CG on an elastica against an independent shooting solve,
//! and runs recorded as a stream.

use std::collections::BTreeMap;

use ikam_compute::eval::{Env, eval, eval_all};
use ikam_compute::grad::{grad, hvp, numeric_grad};
use ikam_compute::ir::{Graph, Op, V};
use ikam_compute::memo::{self, eval_graph_stored, eval_stored};
use ikam_compute::plan::Plan;
use ikam_compute::run::Run;
use ikam_compute::solve::{Minimizer, Options};
use ikam_compute::{Error, Tensor};
use ikam_kernel::{Event, Id, MemStore, Repo, Store};

fn t(shape: &[usize], data: &[f64]) -> Tensor {
    Tensor::new(shape.to_vec(), data.to_vec()).unwrap()
}

/// A scalar that uses every differentiable op: matmul, transpose,
/// broadcasting, max, cumsum, slice, concat, pad and the unary ops.
fn mixed() -> (Graph, V, BTreeMap<String, Vec<usize>>, Env) {
    let mut g = Graph::new();
    let w = g.input("w");
    let x = g.input("x");
    let u = g.input("u");
    let k = g.input("k");
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
    let pp = g.mul(p, p);
    let one = g.scalar(1.0);
    let q = g.add(pp, one);
    let l = g.unary(Op::Ln, q);
    let r = g.unary(Op::Sqrt, q);
    let th = g.unary(Op::Tanh, p);
    let co = g.unary(Op::Cos, p);
    let z = g.add(l, r);
    let z = g.add(z, th);
    let z = g.sub(z, co);
    let z = g.div(z, k);
    let pad = g.push(Op::Pad { start: 2, total: 10 }, &[z]);
    let neg = g.neg(pad);
    let s2 = g.sum(neg);
    let col = g.push(Op::Reshape { shape: vec![5, 1] }, &[u]);
    let row = g.push(Op::Transpose, &[col]);
    let outer = g.push(Op::MatMul, &[col, row]);
    let o2 = g.mul(outer, outer);
    let s3 = g.sum(o2);
    let f = g.add(s1, s2);
    let f = g.add(f, s3);
    let shapes = BTreeMap::from([
        ("w".into(), vec![2, 3]),
        ("x".into(), vec![3, 1]),
        ("u".into(), vec![5]),
        ("k".into(), vec![]),
    ]);
    let env = Env::from([
        ("w".into(), t(&[2, 3], &[0.3, -0.2, 0.5, 0.1, 0.4, -0.6])),
        ("x".into(), t(&[3, 1], &[1.0, 0.5, -0.25])),
        ("u".into(), t(&[5], &[0.2, -0.7, 1.1, 0.05, -0.3])),
        ("k".into(), Tensor::scalar(1.7)),
    ]);
    (g, f, shapes, env)
}

fn max_diff(a: &Tensor, b: &Tensor) -> f64 {
    a.data.iter().zip(&b.data).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max)
}

#[test]
fn gradients_match_central_differences() {
    let (g, f, shapes, env) = mixed();
    let names = ["w", "x", "u", "k"];
    let gg = grad(&g, &shapes, f, &names).unwrap();
    let out = eval(&gg, &env).unwrap();
    assert!(out[0].bits_eq(&eval_all(&g, &env).unwrap()[f]));
    for (i, name) in names.iter().enumerate() {
        let num = numeric_grad(&g, &env, f, name, 1e-6).unwrap();
        let d = max_diff(&out[i + 1], &num);
        assert_eq!(out[i + 1].shape, env[*name].shape, "{name}");
        assert!(d < 1e-7, "{name}: {d}");
    }
}

#[test]
fn hessian_vector_products_match_differences_of_the_gradient() {
    let (g, f, shapes, env) = mixed();
    let names = ["w", "x", "u", "k"];
    let gg = grad(&g, &shapes, f, &names).unwrap();
    let hg = hvp(&g, &shapes, f, &names).unwrap();
    let mut e = env.clone();
    for name in names {
        let x = &env[name];
        let v: Vec<f64> = (0..x.data.len()).map(|i| ((i * 7 + 3) % 5) as f64 * 0.2 - 0.4).collect();
        e.insert(format!("v:{name}"), Tensor { shape: x.shape.clone(), data: v });
    }
    let hv = eval(&hg, &e).unwrap();
    let step = 1e-5;
    let shifted = |sign: f64| {
        let mut s = env.clone();
        for name in names {
            let v = &e[&format!("v:{name}")];
            let x = s.get_mut(name).unwrap();
            for (xi, vi) in x.data.iter_mut().zip(&v.data) {
                *xi += sign * step * vi;
            }
        }
        eval(&gg, &s).unwrap()
    };
    let (plus, minus) = (shifted(1.0), shifted(-1.0));
    for (i, name) in names.iter().enumerate() {
        let fd: Vec<f64> = plus[i + 1].data.iter().zip(&minus[i + 1].data).map(|(a, b)| (a - b) / (2.0 * step)).collect();
        let d = max_diff(&hv[i], &Tensor { shape: hv[i].shape.clone(), data: fd });
        assert!(d < 1e-6, "{name}: {d}");
    }
}

#[test]
fn graphs_round_trip_through_the_store() {
    let (g, f, shapes, env) = mixed();
    let gg = grad(&g, &shapes, f, &["w", "u"]).unwrap();
    let mut repo = Repo::new(MemStore::default());
    let id = gg.put(&mut repo).unwrap();
    let back = Graph::load(&repo, &id).unwrap();
    assert_eq!(back.nodes.len(), gg.nodes.len());
    let (a, b) = (eval(&gg, &env).unwrap(), eval(&back, &env).unwrap());
    assert!(a.iter().zip(&b).all(|(x, y)| x.bits_eq(y)));
    // The same graph stored again is the same object, and the gradient
    // graph shares the forward graph's nodes as content.
    assert_eq!(gg.put(&mut repo).unwrap(), id);
    let before = repo.store().ids().unwrap().len();
    g.put(&mut repo).unwrap();
    let added = repo.store().ids().unwrap().len() - before;
    assert!(added < g.nodes.len(), "forward graph added {added} objects for {} nodes", g.nodes.len());
}

#[test]
fn stored_evaluation_reuses_and_checks_records() {
    let (g, f, shapes, env) = mixed();
    let gg = grad(&g, &shapes, f, &["w", "x", "u", "k"]).unwrap();
    let mut repo = Repo::new(MemStore::default());
    let db = repo.db_empty().unwrap();
    let (ids, db, s1) = eval_stored(&mut repo, &db, &gg, &env, false).unwrap();
    assert_eq!(s1.hits, 0);
    let computed = gg.nodes.iter().filter(|n| !matches!(n.op, Op::Input(_))).count();
    assert_eq!(s1.misses, computed);
    let want = eval(&gg, &env).unwrap();
    for (id, w) in ids.iter().zip(&want) {
        assert!(memo::get(&repo, id, &mut Default::default()).unwrap().bits_eq(w));
    }

    // A rerun computes nothing and appends nothing.
    let (ids2, db2, s2) = eval_stored(&mut repo, &db, &gg, &env, false).unwrap();
    assert_eq!((s2.misses, ids2.clone()), (0, ids.clone()));
    assert_eq!(db2, db);

    // A check recomputes every node and agrees with every record.
    let (_, _, s3) = eval_stored(&mut repo, &db, &gg, &env, true).unwrap();
    assert_eq!(s3.misses, 0);

    // Changing one input recomputes only what depends on it.
    let mut e2 = env.clone();
    e2.insert("k".into(), Tensor::scalar(2.5));
    let (_, db4, s4) = eval_stored(&mut repo, &db, &gg, &e2, false).unwrap();
    assert!(s4.hits > 0 && s4.misses > 0 && s4.misses < s1.misses, "{s4:?} after {s1:?}");

    // A record replaced with other bits is caught by a check.
    let n = gg.nodes.iter().position(|n| matches!(n.op, Op::MatMul)).unwrap();
    let args: Vec<Id> = {
        let mut all = Vec::new();
        let (vals, _, _) = eval_stored(&mut repo, &db4, &gg_upto(&gg, n), &env, false).unwrap();
        all.extend(vals);
        all
    };
    let k = memo::key(&gg.nodes[n].op, &args);
    let wrong = repo.put_content(&Tensor::scalar(42.0).encode()).unwrap().id;
    let tampered = repo.db_append(&db4, &[Event::upsert(&k, wrong.as_bytes(), "tamper")]).unwrap();
    match eval_stored(&mut repo, &tampered, &gg, &env, true) {
        Err(Error::Nondeterministic { recorded, .. }) => assert_eq!(recorded, wrong),
        other => panic!("expected a nondeterminism error, got {other:?}"),
    }

    // Logging a key again with other bits under its own idempotency key
    // is refused by the database itself.
    let r = repo.db_append(&db4, &[Event::upsert(&k, wrong.as_bytes(), &k)]);
    assert!(r.is_err());
}

#[test]
fn whole_graph_records_reuse_and_check() {
    let (g, f, shapes, env) = mixed();
    let gg = grad(&g, &shapes, f, &["w", "u"]).unwrap();
    let mut repo = Repo::new(MemStore::default());
    let gid = gg.put(&mut repo).unwrap();
    let db = repo.db_empty().unwrap();
    let (ids, db, hit) = eval_graph_stored(&mut repo, &db, &gg, &gid, &env, false).unwrap();
    assert!(!hit);
    let (ids2, db2, hit2) = eval_graph_stored(&mut repo, &db, &gg, &gid, &env, false).unwrap();
    assert!(hit2 && ids2 == ids && db2 == db);
    let (_, _, hit3) = eval_graph_stored(&mut repo, &db, &gg, &gid, &env, true).unwrap();
    assert!(hit3);
    // The same outputs as node-by-node stored evaluation.
    let empty = repo.db_empty().unwrap();
    let (per_node, _, _) = eval_stored(&mut repo, &empty, &gg, &env, false).unwrap();
    assert_eq!(per_node, ids);
    // Another input value is another record.
    let mut e2 = env.clone();
    e2.insert("k".into(), Tensor::scalar(3.0));
    let (_, _, hit4) = eval_graph_stored(&mut repo, &db, &gg, &gid, &e2, false).unwrap();
    assert!(!hit4);
}

/// The arguments of node `n`, as a graph whose outputs they are.
fn gg_upto(g: &Graph, n: usize) -> Graph {
    let mut h = g.clone();
    h.outputs = g.nodes[n].args.clone();
    h
}

#[test]
fn planned_programs_match_the_interpreter_bit_for_bit() {
    let (g, f, shapes, env) = mixed();
    let gg = grad(&g, &shapes, f, &["w", "x", "u", "k"]).unwrap();
    let mut plan = Plan::new(&gg, &shapes).unwrap();
    for (name, v) in &env {
        plan.set(name, &v.data).unwrap();
    }
    for _ in 0..3 {
        plan.run();
    }
    let want = eval(&gg, &env).unwrap();
    for (k, w) in want.iter().enumerate() {
        assert!(plan.output_tensor(k).bits_eq(w), "output {k}");
    }
    assert!(plan.set("u", &[1.0]).is_err());
}

/// A planar rod of length `len` clamped at the base, `n` segments with
/// angles `phi`, bending stiffness `ei`, an end moment `m` and a vertical
/// end load `p`. The clamp sits half a segment before the first segment's
/// midpoint, so a uniform curvature is an exact equilibrium.
fn elastica(n: usize, len: f64, ei: f64, m: f64, p: f64) -> (Graph, V) {
    let h = len / n as f64;
    let mut g = Graph::new();
    let phi = g.input("phi");
    let zero = g.konst(Tensor::vector(vec![0.0]));
    let prev = g.slice(phi, 0, n - 1);
    let shifted = g.concat(&[zero, prev]);
    let d = g.sub(phi, shifted);
    let mut w = vec![ei / (2.0 * h); n];
    w[0] = ei / h;
    let w = g.konst(Tensor::vector(w));
    let dd = g.mul(d, d);
    let wd = g.mul(w, dd);
    let bend = g.sum(wd);
    let last = g.slice(phi, n - 1, 1);
    let tip_angle = g.sum(last);
    let mv = g.scalar(m);
    let work_m = g.mul(mv, tip_angle);
    let s = g.unary(Op::Sin, phi);
    let ys = g.sum(s);
    let ph = g.scalar(p * h);
    let work_p = g.mul(ph, ys);
    let e = g.sub(bend, work_m);
    let e = g.sub(e, work_p);
    (g, e)
}

fn tip(phi: &Tensor, len: f64) -> (f64, f64) {
    let h = len / phi.data.len() as f64;
    (phi.data.iter().map(|a| h * a.cos()).sum(), phi.data.iter().map(|a| h * a.sin()).sum())
}

/// The cantilever with a vertical end load by shooting: theta'' =
/// -(P/EI) cos theta, theta(0) = 0, theta'(L) = 0, RK4 and bisection on
/// theta'(0). Independent of the graph code.
fn shooting(len: f64, ei: f64, p: f64) -> (f64, f64) {
    let steps = 20000;
    let h = len / steps as f64;
    let integrate = |k0: f64| {
        let (mut th, mut k, mut x, mut y) = (0.0f64, k0, 0.0f64, 0.0f64);
        let f = |th: f64, k: f64| (k, -(p / ei) * th.cos());
        for _ in 0..steps {
            let (a1, b1) = f(th, k);
            let (a2, b2) = f(th + 0.5 * h * a1, k + 0.5 * h * b1);
            let (a3, b3) = f(th + 0.5 * h * a2, k + 0.5 * h * b2);
            let (a4, b4) = f(th + h * a3, k + h * b3);
            let th2 = th + h / 6.0 * (a1 + 2.0 * a2 + 2.0 * a3 + a4);
            // Position by Simpson on the step.
            let mid = th + 0.5 * h * a1 + 0.125 * h * h * b1;
            x += h / 6.0 * (th.cos() + 4.0 * mid.cos() + th2.cos());
            y += h / 6.0 * (th.sin() + 4.0 * mid.sin() + th2.sin());
            th = th2;
            k += h / 6.0 * (b1 + 2.0 * b2 + 2.0 * b3 + b4);
        }
        (k, x, y)
    };
    let (mut lo, mut hi) = (0.0, 2.0 * p * len / ei);
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if integrate(mid).0 > 0.0 { hi = mid } else { lo = mid }
    }
    let (_, x, y) = integrate(0.5 * (lo + hi));
    (x, y)
}

#[test]
fn newton_cg_bends_an_elastica_to_the_exact_arc() {
    let (len, ei, n) = (1.0, 2.0, 200);
    // An end moment giving a 120 degree bend.
    let theta = 2.0 * std::f64::consts::PI / 3.0;
    let m = theta * ei / len;
    let (g, e) = elastica(n, len, ei, m, 0.0);
    let shapes = BTreeMap::from([("phi".into(), vec![n])]);
    let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
    let (phi, report) = solver.minimize(&Env::new(), Tensor::zeros(&[n]), &Options::default()).unwrap();
    let kappa = m / ei;
    let exact = (theta.sin() / kappa, (1.0 - theta.cos()) / kappa);
    let got = tip(&phi, len);
    let err = ((got.0 - exact.0).powi(2) + (got.1 - exact.1).powi(2)).sqrt();
    assert!(err < 1e-4 * len, "tip {got:?} vs {exact:?}: {err}");
    assert!(report.grad_norm < 1e-9, "{report:?}");
}

#[test]
fn newton_cg_matches_a_shooting_solve_for_an_end_load() {
    let (len, ei, n) = (1.0, 1.0, 200);
    let p = 1.0 * ei / (len * len);
    let (g, e) = elastica(n, len, ei, 0.0, p);
    let shapes = BTreeMap::from([("phi".into(), vec![n])]);
    let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
    let (phi, report) = solver.minimize(&Env::new(), Tensor::zeros(&[n]), &Options::default()).unwrap();
    let got = tip(&phi, len);
    let want = shooting(len, ei, p);
    assert!((got.1 - want.1).abs() < 1e-4, "deflection {got:?} vs {want:?}");
    assert!((got.0 - want.0).abs() < 1e-4, "{got:?} vs {want:?}");
    // Linear theory says PL^3/3EI = 0.333; the large-deflection answer is
    // about 0.30 (Bisshopp and Drucker).
    assert!((want.1 - 0.30).abs() < 0.01, "{want:?}");
    assert!(report.iterations < 30, "{report:?}");
}

#[test]
fn runs_are_recorded_versioned_and_deduplicated() {
    let mut repo = Repo::new(MemStore::default());
    let mut run = Run::start(&mut repo, "ramp").unwrap();
    let a = Tensor::vector(vec![0.0, 1.0]);
    let b = Tensor::vector(vec![0.5, 1.5]);
    let ia = run.record(&mut repo, &a, b"load=0").unwrap();
    run.record(&mut repo, &b, b"load=1").unwrap();
    let ia2 = run.record(&mut repo, &a, b"load=0 again").unwrap();
    assert_eq!(ia, ia2, "an equal state is stored once");
    let c = run.commit(&mut repo, "three steps").unwrap();
    assert_eq!(repo.resolve("runs/ramp").unwrap(), c);
    assert!(run.state(&repo, 1).unwrap().unwrap().bits_eq(&b));
    assert!(run.state(&repo, 3).unwrap().is_none());
    let db = repo.db_at(&c).unwrap();
    let k = format!("{}/state", ikam_compute::run::step_key("ramp", 0));
    assert_eq!(repo.db_get(&db, &k).unwrap().unwrap(), ia.as_bytes().to_vec());
}

#[test]
fn planned_newton_cg_takes_the_same_steps() {
    let (len, ei, n) = (1.0, 1.0, 64);
    let mut g = Graph::new();
    // The end load as an input, so one planned solver serves every load.
    let phi = g.input("phi");
    let load = g.input("load");
    let h = len / n as f64;
    let zero = g.konst(Tensor::vector(vec![0.0]));
    let prev = g.slice(phi, 0, n - 1);
    let sh = g.concat(&[zero, prev]);
    let d = g.sub(phi, sh);
    let mut w = vec![ei / (2.0 * h); n];
    w[0] = ei / h;
    let w = g.konst(Tensor::vector(w));
    let dd = g.mul(d, d);
    let wd = g.mul(w, dd);
    let bend = g.sum(wd);
    let s = g.unary(Op::Sin, phi);
    let ys = g.sum(s);
    let hh = g.scalar(h);
    let lh = g.mul(load, hh);
    let wp = g.mul(lh, ys);
    let e = g.sub(bend, wp);
    let shapes = BTreeMap::from([("phi".into(), vec![n]), ("load".into(), vec![])]);
    let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
    let mut planned = solver.plan(&shapes).unwrap();
    let opt = Options::default();
    let mut x = vec![0.0; n];
    for p in [0.5, 1.0, 2.0] {
        let env = Env::from([("load".into(), Tensor::scalar(p))]);
        let (want, wr) = solver.minimize(&env, Tensor::vector(x.clone()), &opt).unwrap();
        planned.set("load", &[p]).unwrap();
        let r = planned.minimize(&mut x, &opt).unwrap();
        assert_eq!((r.iterations, r.cg_steps), (wr.iterations, wr.cg_steps));
        assert!(Tensor::vector(x.clone()).bits_eq(&want), "load {p}");
    }
}

#[test]
fn adjoint_sensitivities_match_resolving() {
    // Tip height of a rod under an end load and an end moment, and its
    // derivative with respect to both, against finite differences of
    // full solves.
    let (len, ei, n) = (1.0, 1.0, 40);
    let h = len / n as f64;
    let mut g = Graph::new();
    let phi = g.input("phi");
    let load = g.input("load");
    let moment = g.input("moment");
    let zero = g.konst(Tensor::vector(vec![0.0]));
    let prev = g.slice(phi, 0, n - 1);
    let sh = g.concat(&[zero, prev]);
    let d = g.sub(phi, sh);
    let mut w = vec![ei / (2.0 * h); n];
    w[0] = ei / h;
    let w = g.konst(Tensor::vector(w));
    let dd = g.mul(d, d);
    let wd = g.mul(w, dd);
    let bend = g.sum(wd);
    let s = g.unary(Op::Sin, phi);
    let ys = g.sum(s);
    let hh = g.scalar(h);
    let tip_y = g.mul(hh, ys);
    let wp = g.mul(load, tip_y);
    let last = g.slice(phi, n - 1, 1);
    let ta = g.sum(last);
    let wm = g.mul(moment, ta);
    let e = g.sub(bend, wp);
    let e = g.sub(e, wm);
    let mut obj = Graph::new();
    let p2 = obj.input("phi");
    let s2 = obj.unary(Op::Sin, p2);
    let y2 = obj.sum(s2);
    let hh2 = obj.scalar(h);
    let tip = obj.mul(hh2, y2);
    obj.outputs = vec![tip];
    let shapes = BTreeMap::from([("phi".into(), vec![n]), ("load".into(), vec![]), ("moment".into(), vec![])]);
    let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
    let opt = Options { tol: 1e-12, ..Options::default() };
    let solve = |p: f64, m: f64| {
        let env = Env::from([("load".into(), Tensor::scalar(p)), ("moment".into(), Tensor::scalar(m))]);
        let (x, _) = solver.minimize(&env, Tensor::zeros(&[n]), &opt).unwrap();
        (env, x)
    };
    let tip_of = |x: &Tensor| eval(&obj, &Env::from([("phi".into(), x.clone())])).unwrap()[0].item();
    let (p0, m0) = (1.2, 0.4);
    let (env, x) = solve(p0, m0);
    let (value, ds) = solver.sensitivity(&shapes, &env, &x, (&obj, tip), &["load", "moment"], &opt).unwrap();
    assert!((value - tip_of(&x)).abs() < 1e-15);
    let step = 1e-5;
    let fd_p = (tip_of(&solve(p0 + step, m0).1) - tip_of(&solve(p0 - step, m0).1)) / (2.0 * step);
    let fd_m = (tip_of(&solve(p0, m0 + step).1) - tip_of(&solve(p0, m0 - step).1)) / (2.0 * step);
    assert!((ds[0].item() - fd_p).abs() < 1e-7, "{} vs {fd_p}", ds[0].item());
    assert!((ds[1].item() - fd_m).abs() < 1e-7, "{} vs {fd_m}", ds[1].item());
}

#[test]
fn inlined_graphs_compute_the_same() {
    let (g, f, _, env) = mixed();
    let mut h = Graph::new();
    let mut bind = BTreeMap::new();
    for name in ["w", "x", "u", "k"] {
        bind.insert(name.to_string(), h.input(&format!("outer:{name}")));
    }
    let mut g1 = g.clone();
    g1.outputs = vec![f];
    let out = h.inline(&g1, &bind).unwrap();
    let two = h.scalar(2.0);
    let y = h.mul(two, out[0]);
    h.outputs = vec![y];
    let outer: Env = env.iter().map(|(k, v)| (format!("outer:{k}"), v.clone())).collect();
    let want = eval_all(&g, &env).unwrap()[f].item() * 2.0;
    assert_eq!(eval(&h, &outer).unwrap()[0].item().to_bits(), want.to_bits());
}

#[test]
fn a_committed_graph_crosses_projects_by_a_graph_share() {
    use ikam_kernel::share::{Right, Scope, Share, import_share};
    let (g, f, shapes, env) = mixed();
    let gg = grad(&g, &shapes, f, &["u"]).unwrap();
    let mut src = Repo::new(MemStore::default());
    let (_, commit) = gg.commit_at(&mut src, "main", "models/mixed-grad", "the model").unwrap();
    // A database on the same ref keeps the graph entry.
    let db = src.db_empty().unwrap();
    let db = src.db_append(&db, &[Event::upsert("note", b"x", "note")]).unwrap();
    let c2 = src.db_commit("main", &db, "a record").unwrap();
    assert!(Graph::load_at(&src, &c2.to_hex(), "models/mixed-grad").is_ok());
    let share = Share {
        name: "model".into(),
        from: "source".into(),
        to: "receiver".into(),
        commit,
        scope: Scope::Graph { path: "models/mixed-grad".into(), prefix: String::new() },
        right: Right::Merge,
        expires: u64::MAX,
    };
    let key = [7u8; 32];
    let tag = src.share_issue(&share, &key).unwrap();
    src.share_check(&share, &tag, &key, "receiver", Right::Merge, 0).unwrap();
    let mut dst = Repo::new(MemStore::default());
    import_share(&mut src, &share, &mut dst, "main", "import the model").unwrap();
    let got = Graph::load_at(&dst, "main", "models/mixed-grad").unwrap();
    let (a, b) = (eval(&gg, &env).unwrap(), eval(&got, &env).unwrap());
    assert!(a.iter().zip(&b).all(|(x, y)| x.bits_eq(y)));
}
