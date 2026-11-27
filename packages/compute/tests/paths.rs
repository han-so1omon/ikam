//! E439: equality constraints, arc-length continuation and explicit time
//! stepping, against closed forms.

use std::collections::BTreeMap;

use ikam_compute::Tensor;
use ikam_compute::dynamics::{Dynamics, Scheme};
use ikam_compute::eval::Env;
use ikam_compute::ir::{Graph, Op, V};
use ikam_compute::path::{Continuation, Linear, PathOptions, lu_solve};
use ikam_compute::solve::{Minimizer, Options};

/// A rod of `n` segments with a saturating section law `M(k) = tanh k + h k`
/// under an end moment `load`.
fn rod(n: usize, hard: f64) -> (Graph, V, BTreeMap<String, Vec<usize>>) {
    let h = 1.0 / n as f64;
    let mut g = Graph::new();
    let phi = g.input("phi");
    let load = g.input("load");
    let zero = g.konst(Tensor::vector(vec![0.0]));
    let prev = g.slice(phi, 0, n - 1);
    let sh = g.concat(&[zero, prev]);
    let d = g.sub(phi, sh);
    let mut lens = vec![h; n];
    lens[0] = h / 2.0;
    let lv = g.konst(Tensor::vector(lens));
    let k = g.div(d, lv);
    let ep = g.unary(Op::Exp, k);
    let nk = g.neg(k);
    let em = g.unary(Op::Exp, nk);
    let c = g.add(ep, em);
    let half = g.scalar(0.5);
    let c = g.mul(c, half);
    let lc = g.unary(Op::Ln, c);
    let kk = g.mul(k, k);
    let hh = g.scalar(0.5 * hard);
    let q = g.mul(kk, hh);
    let w = g.add(lc, q);
    let wl = g.mul(w, lv);
    let internal = g.sum(wl);
    let last = g.slice(phi, n - 1, 1);
    let tip = g.sum(last);
    let work = g.mul(load, tip);
    let e = g.sub(internal, work);
    (g, e, BTreeMap::from([("phi".into(), vec![n]), ("load".into(), vec![])]))
}

#[test]
fn lu_solves_and_signs_the_determinant() {
    let mut a = vec![0.0, 2.0, 1.0, 1.0, 1.0, 0.0, 3.0, 0.0, 1.0];
    let mut b = vec![5.0, 2.0, 6.0];
    let sign = lu_solve(&mut a, 3, &mut b).unwrap();
    // det = 0*(1-0) - 2*(1-0) + 1*(0-3) = -5.
    assert_eq!(sign, -1.0);
    let want = [1.0, 1.0, 3.0];
    for (g, w) in b.iter().zip(want) {
        assert!((g - w).abs() < 1e-12, "{b:?}");
    }
    assert!(lu_solve(&mut [1.0, 2.0, 2.0, 4.0], 2, &mut [1.0, 1.0]).is_err());
}

#[test]
fn a_prescribed_rotation_gives_the_reaction_moment() {
    let n = 12;
    let hard = 1e-3;
    let (g, e, shapes) = rod(n, hard);
    let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
    let env = Env::from([("load".into(), Tensor::scalar(0.0))]);
    let h = 1.0 / n as f64;
    let mut x = Tensor::zeros(&[n]);
    for k in [1.0, 3.0, 8.0, 15.0] {
        let con = Linear::new().fix(n - 1, k * (1.0 - h / 2.0));
        let (phi, c) = solver.minimize_constrained(&env, x, &con, &Options::default()).unwrap();
        let want = k.tanh() + hard * k;
        assert!((c.multipliers[0] - want).abs() < 1e-9, "k {k}: {} vs {want}", c.multipliers[0]);
        assert!(c.residual < 1e-12, "{c:?}");
        assert!(c.report.grad_norm < 1e-10);
        x = phi;
    }
}

#[test]
fn a_prescribed_combination_holds_and_its_multipliers_balance() {
    // E = sum (x_i - i)^2 / 2 with x0 + x1 = 1 and x2 - x3 = 0.
    let mut g = Graph::new();
    let x = g.input("x");
    let t = g.konst(Tensor::vector(vec![0.0, 1.0, 2.0, 3.0]));
    let d = g.sub(x, t);
    let dd = g.mul(d, d);
    let half = g.scalar(0.5);
    let w = g.mul(dd, half);
    let e = g.sum(w);
    let shapes = BTreeMap::from([("x".into(), vec![4])]);
    let solver = Minimizer::new(&g, e, "x", &shapes).unwrap();
    let con = Linear::new().combo(&[(0, 1.0), (1, 1.0)], 1.0).combo(&[(2, 1.0), (3, -1.0)], 0.0);
    let (x, c) = solver.minimize_constrained(&Env::new(), Tensor::zeros(&[4]), &con, &Options::default()).unwrap();
    let want = [0.0, 1.0, 2.5, 2.5];
    for (g, w) in x.data.iter().zip(want) {
        assert!((g - w).abs() < 1e-12, "{:?}", x.data);
    }
    // grad E = A^T lambda: (0, 0, 0.5, -0.5) = l1 (1, 1, 0, 0) + l2 (0, 0, 1, -1).
    assert!(c.multipliers[0].abs() < 1e-12 && (c.multipliers[1] - 0.5).abs() < 1e-12, "{c:?}");
    // Dependent rows are refused.
    let bad = Linear::new().fix(0, 1.0).fix(0, 2.0);
    assert!(solver.minimize_constrained(&Env::new(), Tensor::zeros(&[4]), &bad, &Options::default()).is_err());
}

#[test]
fn load_control_stalls_past_the_plateau_and_arc_length_does_not() {
    let n = 8;
    let (g, e, shapes) = rod(n, 0.0);
    let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
    let env = Env::from([("load".into(), Tensor::scalar(1.05))]);
    assert!(solver.minimize(&env, Tensor::zeros(&[n]), &Options::default()).is_err());
    let cont = Continuation::new(&g, e, "phi", "load", &shapes).unwrap();
    let opt = PathOptions { ds: 0.1, ds_max: 2.0, ..PathOptions::default() };
    let h = 1.0 / n as f64;
    let kap = |p: &[f64]| p[n - 1] / (1.0 - h / 2.0);
    let path = cont.follow(&Env::new(), &Tensor::zeros(&[n]), 0.0, &opt, |p| kap(&p.x) >= 10.0).unwrap();
    for p in &path {
        assert!((p.load - kap(&p.x).tanh()).abs() < 1e-9, "{} at k {}", p.load, kap(&p.x));
        assert!(p.residual < 1e-10);
    }
    assert!(kap(&path.last().unwrap().x) >= 10.0);
}

#[test]
fn arc_length_follows_a_truss_through_both_limit_points() {
    let (a, h0, ea) = (1.0f64, 0.1f64, 1000.0f64);
    let l0 = (a * a + h0 * h0).sqrt();
    let mut g = Graph::new();
    let x = g.input("x");
    let load = g.input("load");
    let w = g.slice(x, 1, 1);
    let w = g.sum(w);
    let u = g.slice(x, 0, 1);
    let u = g.sum(u);
    let hv = g.scalar(h0);
    let z = g.sub(hv, w);
    let zz = g.mul(z, z);
    let av = g.scalar(a);
    let mut terms = Vec::new();
    for s in [1.0, -1.0] {
        let sv = g.scalar(s);
        let su = g.mul(sv, u);
        let dx = g.add(av, su);
        let dxx = g.mul(dx, dx);
        let ll = g.add(dxx, zz);
        let l = g.unary(Op::Sqrt, ll);
        let lv = g.scalar(l0);
        let st = g.sub(l, lv);
        let st2 = g.mul(st, st);
        let kv = g.scalar(ea / (2.0 * l0));
        terms.push(g.mul(kv, st2));
    }
    let el = g.add(terms[0], terms[1]);
    let work = g.mul(load, w);
    let e = g.sub(el, work);
    let shapes = BTreeMap::from([("x".into(), vec![2]), ("load".into(), vec![])]);
    let p_of = |w: f64| {
        let z = h0 - w;
        let l = (a * a + z * z).sqrt();
        2.0 * ea * z * (l0 - l) / (l0 * l)
    };
    let ls = (l0 * a * a).cbrt();
    let zs = (ls * ls - a * a).sqrt();
    let pmax = 2.0 * ea * zs * (l0 - ls) / (l0 * ls);
    let cont = Continuation::new(&g, e, "x", "load", &shapes).unwrap();
    let opt = PathOptions { ds: 0.01, ds_max: 0.02, ..PathOptions::default() };
    let path = cont.follow(&Env::new(), &Tensor::zeros(&[2]), 0.0, &opt, |p| p.x[1] >= 2.2 * h0).unwrap();
    let turns: Vec<usize> = (1..path.len()).filter(|&i| path[i - 1].dload.signum() != path[i].dload.signum()).collect();
    assert_eq!(turns.len(), 2, "two limit points");
    for (i, want) in turns.iter().zip([pmax, -pmax]) {
        let lim = cont.locate_limit(&Env::new(), &path[i - 1], &path[*i], &opt, 50).unwrap();
        assert!((lim.load - want).abs() < 1e-10, "{} vs {want}", lim.load);
        assert!(lim.dload.abs() < 1e-6, "{}", lim.dload);
    }
    for p in &path {
        assert!((p.load - p_of(p.x[1])).abs() < 1e-8, "{} vs {}", p.load, p_of(p.x[1]));
        assert!(p.x[0].abs() < 1e-9);
    }
}

#[test]
fn velocity_verlet_keeps_a_pendulums_energy_bounded() {
    let mut g = Graph::new();
    let th = g.input("th");
    let c = g.unary(Op::Cos, th);
    let cs = g.sum(c);
    let one = g.scalar(1.0);
    let pe = g.sub(one, cs);
    let dynm = Dynamics::new(&g, pe, "th", &BTreeMap::from([("th".into(), vec![1])])).unwrap();
    let mass = [1.0];
    let drift = |scheme: Scheme| {
        let mut env = Env::new();
        let mut s = dynm.start(&env, vec![1.0], vec![0.0]).unwrap();
        let e0 = s.potential + s.kinetic(&mass);
        let mut worst = 0.0f64;
        for _ in 0..10_000 {
            dynm.step(&mut env, &mut s, &mass, 0.01, scheme).unwrap();
            worst = worst.max((s.potential + s.kinetic(&mass) - e0).abs() / e0);
        }
        worst
    };
    let verlet = drift(Scheme::VelocityVerlet);
    let euler = drift(Scheme::ExplicitEuler);
    assert!(verlet < 1e-4, "{verlet}");
    assert!(euler > 100.0 * verlet, "{euler} vs {verlet}");
}
