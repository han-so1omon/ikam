//! E439: constraints, continuation and time stepping on generic models.
//!
//! `cargo run --release --example e439_paths [-- <dir for CSV paths>]`
//!
//! 1. A rod with a yielding section, `M(k) = My tanh(k/ky) + h k` (energy
//!    `My ky ln cosh(k/ky) + h k^2/2`), loaded by an end moment: load
//!    control (Newton-CG today), rotation control (a prescribed tip angle,
//!    `minimize_constrained`) and arc-length continuation, against the
//!    closed form (uniform curvature, `M(k)`).
//! 2. A shallow two-bar truss under an apex load (snap-through, limit
//!    load in closed form): load control, displacement control and
//!    arc-length past both limit points.
//! 3. A pendulum and a nonlinear spring chain stepped 10^4 times:
//!    energy drift by scheme.

use std::collections::BTreeMap;
use std::io::Write;
use std::time::Instant;

use ikam_compute::Tensor;
use ikam_compute::dynamics::{Dynamics, Scheme};
use ikam_compute::eval::Env;
use ikam_compute::ir::{Graph, Op, V};
use ikam_compute::path::{Continuation, Linear, PathOptions, PathPoint};
use ikam_compute::solve::{Minimizer, Options};

const LEN: f64 = 1.0;

/// The rod: `n` segment angles `phi`, the clamp half a segment before the
/// first midpoint, each section with the yielding law, and an end moment
/// `load` (moment per unit My) doing work on the last angle.
fn rod(n: usize, hard: f64) -> (Graph, V, BTreeMap<String, Vec<usize>>) {
    let h = LEN / n as f64;
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
    // ln cosh k = ln((e^k + e^-k) / 2), fine for |k| < 700.
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
    let shapes = BTreeMap::from([("phi".into(), vec![n]), ("load".into(), vec![])]);
    (g, e, shapes)
}

fn moment(k: f64, hard: f64) -> f64 {
    k.tanh() + hard * k
}

/// The curvature carrying moment `m`, by bisection (a fine 1-D solve).
fn curvature(m: f64, hard: f64) -> Option<f64> {
    if hard == 0.0 && m >= 1.0 {
        return None;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    while moment(hi, hard) < m {
        hi *= 2.0;
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if moment(mid, hard) < m { lo = mid } else { hi = mid }
    }
    Some(0.5 * (lo + hi))
}

/// Section curvatures of a solution.
fn curvatures(phi: &[f64]) -> Vec<f64> {
    let n = phi.len();
    let h = LEN / n as f64;
    (0..n)
        .map(|i| {
            let d = phi[i] - if i == 0 { 0.0 } else { phi[i - 1] };
            d / if i == 0 { h / 2.0 } else { h }
        })
        .collect()
}

/// Mean curvature from the tip angle, and the largest spread from it.
fn uniform(phi: &[f64]) -> (f64, f64) {
    let n = phi.len();
    let h = LEN / n as f64;
    let k = phi[n - 1] / (LEN - h / 2.0);
    let spread = curvatures(phi).iter().fold(0.0f64, |m, c| m.max((c - k).abs()));
    (k, spread)
}

fn out(dir: &Option<String>, name: &str, header: &str, rows: &[String]) {
    if let Some(d) = dir {
        let mut f = std::fs::File::create(format!("{d}/{name}")).unwrap();
        writeln!(f, "{header}").unwrap();
        for r in rows {
            writeln!(f, "{r}").unwrap();
        }
    }
}

// A single section with softening and re-hardening:
// W(k) = ln cosh k - SA k^2 / 2 + SB k^4 / 4, M(k) = tanh k - SA k + SB k^3,
// a peak (a limit point) and a valley.
const SA: f64 = 0.5;
const SB: f64 = 0.02;

fn section() -> (Graph, V, BTreeMap<String, Vec<usize>>) {
    let mut g = Graph::new();
    let x = g.input("k");
    let load = g.input("load");
    let k = g.sum(x);
    let ep = g.unary(Op::Exp, k);
    let nk = g.neg(k);
    let em = g.unary(Op::Exp, nk);
    let c = g.add(ep, em);
    let half = g.scalar(0.5);
    let c = g.mul(c, half);
    let lc = g.unary(Op::Ln, c);
    let k2 = g.mul(k, k);
    let k4 = g.mul(k2, k2);
    let av = g.scalar(-0.5 * SA);
    let bv = g.scalar(0.25 * SB);
    let t2 = g.mul(k2, av);
    let t4 = g.mul(k4, bv);
    let w = g.add(lc, t2);
    let w = g.add(w, t4);
    let work = g.mul(load, k);
    let e = g.sub(w, work);
    (g, e, BTreeMap::from([("k".into(), vec![1]), ("load".into(), vec![])]))
}

fn soft_moment(k: f64) -> f64 {
    k.tanh() - SA * k + SB * k * k * k
}

/// Where M'(k) = sech^2 k - SA + 3 SB k^2 = 0 in [lo, hi], by bisection.
fn soft_turn(mut lo: f64, mut hi: f64) -> f64 {
    let d = |k: f64| 1.0 / k.cosh().powi(2) - SA + 3.0 * SB * k * k;
    let up = d(lo) > 0.0;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if (d(mid) > 0.0) == up { lo = mid } else { hi = mid }
    }
    0.5 * (lo + hi)
}

fn section_cases(dir: &Option<String>) {
    let (g, e, shapes) = section();
    let (kp, kv) = (soft_turn(0.1, 2.0), soft_turn(2.0, 5.0));
    let (mp, mv) = (soft_moment(kp), soft_moment(kv));
    println!("### Softening section alone (M = tanh k - {SA} k + {SB} k^3)\n");
    println!("Closed form: peak M {mp:.10} at k {kp:.10}; valley M {mv:.10} at k {kv:.10}.\n");
    let solver = Minimizer::new(&g, e, "k", &shapes).unwrap();
    println!("Load control (warm-started):\n");
    println!("| M / peak | Newton its | k | M(k) |");
    println!("|---|---|---|---|");
    let mut x = Tensor::zeros(&[1]);
    for f in [0.5, 0.9, 0.99, 1.01, 1.1] {
        let env = Env::from([("load".into(), Tensor::scalar(f * mp))]);
        match solver.minimize(&env, x.clone(), &Options::default()) {
            Ok((xs, rep)) => {
                println!("| {f} | {} | {:.6} | {:.6} |", rep.iterations, xs.data[0], soft_moment(xs.data[0]));
                x = xs;
            }
            Err(err) => println!("| {f} | {err} | - | - |"),
        }
    }
    let zero = Env::from([("load".into(), Tensor::scalar(0.0))]);
    let mut x = Tensor::zeros(&[1]);
    let (mut worst, mut tot) = (0.0f64, 0);
    for j in 1..=50 {
        let k = 0.1 * j as f64;
        let con = Linear::new().fix(0, k);
        let (xs, c) = solver.minimize_constrained(&zero, x.clone(), &con, &Options::default()).unwrap();
        worst = worst.max((c.multipliers[0] - soft_moment(k)).abs());
        tot += c.report.iterations;
        x = xs;
    }
    println!("\nCurvature control, 50 steps to k = 5: {tot} Newton iterations, worst |reaction - M(k)| {worst:.1e}.\n");
    let cont = Continuation::new(&g, e, "k", "load", &shapes).unwrap();
    let opt = PathOptions { ds: 0.05, ds_max: 0.2, ..PathOptions::default() };
    let path = cont.follow(&Env::new(), &Tensor::zeros(&[1]), 0.0, &opt, |p| p.x[0] >= 5.0).unwrap();
    let worst = path.iter().fold(0.0f64, |m, p| m.max((p.load - soft_moment(p.x[0])).abs()));
    println!("Arc-length (until k = 5):\n");
    summarize(&path, |p| {
        format!("{:.6} | {:.8} | {:.1e}", p.x[0], soft_moment(p.x[0]), (p.load - soft_moment(p.x[0])).abs())
    });
    let top = path.iter().map(|p| p.load).fold(f64::MIN, f64::max);
    println!(
        "\n{} points; worst |load - M(k)| {worst:.1e}; highest sampled load {top:.8} (peak {mp:.8}).\n",
        path.len()
    );
    limits(&cont, &path, &opt, |p| p.x[0], &[(kp, mp), (kv, mv)]);
    let rows: Vec<String> = path.iter().map(|p| row(p, p.x[0], soft_moment(p.x[0]))).collect();
    out(dir, "section_arc.csv", CSV_HEADER, &rows);
}

fn rod_cases(dir: &Option<String>) {
    let n = 20;
    println!("## 1. Rod with a yielding section (n = {n}, My = ky = L = 1)\n");
    for hard in [1e-3, 0.0] {
        let (g, e, shapes) = rod(n, hard);
        let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
        println!("### hardening h = {hard}\n");
        println!("Load control (Newton-CG, warm-started from the last step, max 100 iterations):\n");
        println!("| moment M/My | result | Newton its | CG steps | k | k exact | |dk| |");
        println!("|---|---|---|---|---|---|---|");
        let mut x = Tensor::zeros(&[n]);
        let mut rows = Vec::new();
        for m in [0.2, 0.4, 0.6, 0.8, 0.9, 0.95, 0.99, 0.999, 1.0, 1.005, 1.01, 1.02] {
            let env = Env::from([("load".into(), Tensor::scalar(m))]);
            let t0 = Instant::now();
            let r = solver.minimize(&env, x.clone(), &Options::default());
            let us = t0.elapsed().as_micros();
            let exact = curvature(m, hard);
            match r {
                Ok((phi, rep)) => {
                    let (k, _) = uniform(&phi.data);
                    match exact {
                        Some(ex) => println!(
                            "| {m} | converged | {} | {} | {k:.6} | {ex:.6} | {:.1e} |",
                            rep.iterations,
                            rep.cg_steps,
                            (k - ex).abs()
                        ),
                        // No equilibrium exists: the residual M - tanh k
                        // fell below the tolerance on the plateau.
                        None => println!(
                            "| {m} | **false convergence** (|g| {:.1e}) | {} | {} | {k:.6} | none | - |",
                            rep.grad_norm, rep.iterations, rep.cg_steps
                        ),
                    }
                    let ex = exact.map(|v| v.to_string()).unwrap_or("none".into());
                    rows.push(format!("{m},ok,{},{},{k},{ex},{us}", rep.iterations, rep.cg_steps));
                    x = phi;
                }
                Err(err) => {
                    let ex = exact.map(|v| format!("{v:.6}")).unwrap_or("none".into());
                    println!("| {m} | **{err}** | - | - | - | {ex} | - |");
                    rows.push(format!("{m},fail,,,,{ex},{us}"));
                }
            }
        }
        // The same solve from rest, as a single load step.
        let m = 1.02;
        let env = Env::from([("load".into(), Tensor::scalar(m))]);
        match solver.minimize(&env, Tensor::zeros(&[n]), &Options::default()) {
            Ok((_, rep)) => println!("\nOne step from rest to M = {m}: {} Newton iterations.\n", rep.iterations),
            Err(err) => println!("\nOne step from rest to M = {m}: {err}.\n"),
        }
        out(dir, &format!("rod_load_h{hard}.csv"), "moment,result,iterations,cg,k,k_exact,us", &rows);

        // Rotation control: the tip angle prescribed.
        println!("Rotation control (tip angle prescribed, `minimize_constrained`, warm-started):\n");
        println!("| k target | Newton its | CG steps | |Ax-b| | reaction M | M(k) exact | |dM| | curvature spread |");
        println!("|---|---|---|---|---|---|---|---|");
        let zero = Env::from([("load".into(), Tensor::scalar(0.0))]);
        let mut x = Tensor::zeros(&[n]);
        let mut rows = Vec::new();
        let (mut worst_m, mut worst_res, mut its) = (0.0f64, 0.0f64, Vec::new());
        let h = LEN / n as f64;
        for j in 1..=40 {
            let kt = 0.5 * j as f64;
            let theta = kt * (LEN - h / 2.0);
            let con = Linear::new().fix(n - 1, theta);
            let (phi, c) = solver.minimize_constrained(&zero, x.clone(), &con, &Options::default()).unwrap();
            let (_, spread) = uniform(&phi.data);
            let exact = moment(kt, hard);
            let dm = (c.multipliers[0] - exact).abs();
            worst_m = worst_m.max(dm);
            worst_res = worst_res.max(c.residual);
            its.push(c.report.iterations);
            if [1, 2, 4, 6, 8, 12, 20, 40].contains(&j) {
                println!(
                    "| {kt} | {} | {} | {:.1e} | {:.8} | {exact:.8} | {dm:.1e} | {spread:.1e} |",
                    c.report.iterations, c.report.cg_steps, c.residual, c.multipliers[0]
                );
            }
            rows.push(format!(
                "{kt},{},{},{},{},{exact}",
                c.report.iterations, c.report.cg_steps, c.residual, c.multipliers[0]
            ));
            x = phi;
        }
        println!(
            "\n40 steps to k = 20: Newton iterations per step {}..{} (total {}), worst |dM| {worst_m:.1e}, worst |Ax-b| {worst_res:.1e}.\n",
            its.iter().min().unwrap(),
            its.iter().max().unwrap(),
            its.iter().sum::<usize>()
        );
        out(dir, &format!("rod_rotation_h{hard}.csv"), "k,iterations,cg,residual,reaction,exact", &rows);

        // Arc-length.
        let cont = Continuation::new(&g, e, "phi", "load", &shapes).unwrap();
        let opt = PathOptions { ds: 0.1, ds_max: 4.0, ..PathOptions::default() };
        let t0 = Instant::now();
        let path = cont.follow(&Env::new(), &Tensor::zeros(&[n]), 0.0, &opt, |p| uniform(&p.x).0 >= 20.0).unwrap();
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        let mut worst = 0.0f64;
        let mut spread = 0.0f64;
        let mut rows = Vec::new();
        for p in &path {
            let (k, s) = uniform(&p.x);
            worst = worst.max((p.load - moment(k, hard)).abs());
            spread = spread.max(s);
            rows.push(row(p, k, moment(k, hard)));
        }
        println!("Arc-length continuation (from rest until k = 20):\n");
        summarize(&path, |p| {
            let k = uniform(&p.x).0;
            format!("{k:.4} | {:.8} | {:.1e}", moment(k, hard), (p.load - moment(k, hard)).abs())
        });
        println!(
            "\n{} points in {ms:.0} ms; worst |load - M(k)| {worst:.1e}, curvature spread {spread:.1e}.\n",
            path.len()
        );
        out(dir, &format!("rod_arc_h{hard}.csv"), CSV_HEADER, &rows);
    }
}

const CSV_HEADER: &str = "load,iterations,retries,residual,arc_residual,ds,det_sign,dload,evals,state,exact";

fn row(p: &PathPoint, state: f64, exact: f64) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{},{state},{exact}",
        p.load, p.iterations, p.retries, p.residual, p.arc_residual, p.ds, p.det_sign, p.dload, p.evals
    )
}

/// Print a sample of a path: about 12 rows and every point where the
/// load's direction or det K's sign changes.
fn summarize(path: &[PathPoint], extra: impl Fn(&PathPoint) -> String) {
    println!("| step | load | corrector its | retries | |R| | |N| | ds | det K | dl/ds | state | exact | error |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|");
    let every = (path.len() / 12).max(1);
    for (i, p) in path.iter().enumerate() {
        let turn = i > 0 && (path[i - 1].dload.signum() != p.dload.signum() || path[i - 1].det_sign != p.det_sign);
        if i % every == 0 || i + 1 == path.len() || turn {
            println!(
                "| {i}{} | {:.8} | {} | {} | {:.1e} | {:.1e} | {:.3} | {:+} | {:+.3} | {} |",
                if turn { " (turn)" } else { "" },
                p.load,
                p.iterations,
                p.retries,
                p.residual,
                p.arc_residual,
                p.ds,
                p.det_sign,
                p.dload,
                extra(p)
            );
        }
    }
}

// The truss: supports at (-A, 0) and (A, 0), apex at (u, H0 - w), axial
// stiffness EA, a downward apex load `load`.
const A: f64 = 1.0;
const H0: f64 = 0.1;
const EA: f64 = 1000.0;

fn truss() -> (Graph, V, BTreeMap<String, Vec<usize>>) {
    let l0 = (A * A + H0 * H0).sqrt();
    let mut g = Graph::new();
    let x = g.input("x");
    let load = g.input("load");
    let u = g.slice(x, 0, 1);
    let w = g.slice(x, 1, 1);
    let u = g.sum(u);
    let w = g.sum(w);
    let hv = g.scalar(H0);
    let z = g.sub(hv, w);
    let zz = g.mul(z, z);
    let av = g.scalar(A);
    let mut energy = None;
    for sign in [1.0, -1.0] {
        let s = g.scalar(sign);
        let su = g.mul(s, u);
        let dx = g.add(av, su);
        let dxx = g.mul(dx, dx);
        let ll = g.add(dxx, zz);
        let l = g.unary(Op::Sqrt, ll);
        let l0v = g.scalar(l0);
        let st = g.sub(l, l0v);
        let st2 = g.mul(st, st);
        let k = g.scalar(EA / (2.0 * l0));
        let e = g.mul(k, st2);
        energy = Some(match energy {
            None => e,
            Some(prev) => g.add(prev, e),
        });
    }
    let work = g.mul(load, w);
    let e = g.sub(energy.unwrap(), work);
    let shapes = BTreeMap::from([("x".into(), vec![2]), ("load".into(), vec![])]);
    (g, e, shapes)
}

/// The symmetric equilibrium load at apex drop `w`.
fn truss_load(w: f64) -> f64 {
    let l0 = (A * A + H0 * H0).sqrt();
    let z = H0 - w;
    let l = (A * A + z * z).sqrt();
    2.0 * EA * z * (l0 - l) / (l0 * l)
}

/// The limit points in closed form: l* = (L0 A^2)^(1/3).
fn truss_limit() -> (f64, f64) {
    let l0 = (A * A + H0 * H0).sqrt();
    let ls = (l0 * A * A).cbrt();
    let zs = (ls * ls - A * A).sqrt();
    (H0 - zs, 2.0 * EA * zs * (l0 - ls) / (l0 * ls))
}

/// Every limit point on `path` (where dl/ds changes sign): the nearest
/// point, and the point located by bisection on the arc length, against
/// the closed form `(state, load)`.
fn limits(
    cont: &Continuation,
    path: &[PathPoint],
    opt: &PathOptions,
    state: impl Fn(&PathPoint) -> f64,
    exact: &[(f64, f64)],
) {
    println!(
        "| limit point | between steps | nearest point's load | located: state | located: load | closed form state | closed form load | load error | state error |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    let mut k = 0;
    for i in 1..path.len() {
        if path[i - 1].dload.signum() == path[i].dload.signum() {
            continue;
        }
        let near = if path[i - 1].load.abs() > path[i].load.abs() { &path[i - 1] } else { &path[i] };
        let p = cont.locate_limit(&Env::new(), &path[i - 1], &path[i], opt, 50).unwrap();
        let (cs, cl) = exact.get(k).copied().unwrap_or((f64::NAN, f64::NAN));
        println!(
            "| {} | {}-{i} | {:.10} | {:.10} | {:.12} | {cs:.10} | {cl:.12} | {:.1e} | {:.1e} |",
            k + 1,
            i - 1,
            near.load,
            state(&p),
            p.load,
            (p.load - cl).abs(),
            (state(&p) - cs).abs()
        );
        k += 1;
    }
    println!();
}

fn truss_cases(dir: &Option<String>) {
    let (g, e, shapes) = truss();
    let (ws, pmax) = truss_limit();
    println!("## 2. Shallow two-bar truss (half span 1, rise 0.1, EA = 1000)\n");
    println!(
        "Closed form: limit load {pmax:.10} at apex drop {ws:.10}; second limit -{pmax:.10} at {:.10}.\n",
        2.0 * H0 - ws
    );
    let solver = Minimizer::new(&g, e, "x", &shapes).unwrap();
    println!("Load control (warm-started Newton-CG):\n");
    println!("| load / limit | Newton its | apex drop w | w / rise | P(w) closed form |");
    println!("|---|---|---|---|---|");
    let mut x = Tensor::zeros(&[2]);
    for f in [0.25, 0.5, 0.75, 0.9, 0.99, 1.01, 1.1] {
        let env = Env::from([("load".into(), Tensor::scalar(f * pmax))]);
        match solver.minimize(&env, x.clone(), &Options::default()) {
            Ok((xs, rep)) => {
                println!(
                    "| {f} | {} | {:.6} | {:.3} | {:.6} |",
                    rep.iterations,
                    xs.data[1],
                    xs.data[1] / H0,
                    truss_load(xs.data[1])
                );
                x = xs;
            }
            Err(err) => println!("| {f} | {err} | - | - | - |"),
        }
    }
    println!(
        "\nThe same loads through `minimize_constrained` with no constraints (its line search accepts a full step that halves the gradient):\n"
    );
    println!("| load / limit | Newton its | apex drop w | |g| |");
    println!("|---|---|---|---|");
    let mut x = Tensor::zeros(&[2]);
    for f in [0.25, 0.5, 0.75, 0.9, 0.99, 1.01, 1.1] {
        let env = Env::from([("load".into(), Tensor::scalar(f * pmax))]);
        match solver.minimize_constrained(&env, x.clone(), &Linear::new(), &Options::default()) {
            Ok((xs, c)) => {
                println!("| {f} | {} | {:.6} | {:.1e} |", c.report.iterations, xs.data[1], c.report.grad_norm);
                x = xs;
            }
            Err(err) => println!("| {f} | {err} | - | - |"),
        }
    }
    println!();

    println!("Displacement control (apex drop prescribed, `minimize_constrained`, each step from u = 1e-3):\n");
    println!("| w / rise | Newton its | |Ax-b| | reaction | P(w) | |dP| | u |");
    println!("|---|---|---|---|---|---|---|");
    let zero = Env::from([("load".into(), Tensor::scalar(0.0))]);
    let mut x = Tensor::zeros(&[2]);
    let (mut worst, mut tot) = (0.0f64, 0);
    for j in 1..=44 {
        let w = 0.05 * H0 * j as f64;
        let con = Linear::new().fix(1, w);
        // Start each step off the symmetric state, else u = 0 is already
        // optimal and no iteration is needed.
        x.data[0] = 1e-3;
        let (xs, c) = solver.minimize_constrained(&zero, x.clone(), &con, &Options::default()).unwrap();
        let dp = (c.multipliers[0] - truss_load(w)).abs();
        worst = worst.max(dp);
        tot += c.report.iterations;
        if j % 4 == 0 {
            println!(
                "| {:.2} | {} | {:.1e} | {:.10} | {:.10} | {dp:.1e} | {:.1e} |",
                w / H0,
                c.report.iterations,
                c.residual,
                c.multipliers[0],
                truss_load(w),
                xs.data[0]
            );
        }
        x = xs;
    }
    println!("\n44 steps to w = 2.2 rise: {tot} Newton iterations, worst |reaction - P(w)| {worst:.1e}.\n");

    let cont = Continuation::new(&g, e, "x", "load", &shapes).unwrap();
    let opt = PathOptions { ds: 0.01, ds_max: 0.05, ..PathOptions::default() };
    let t0 = Instant::now();
    let path = cont.follow(&Env::new(), &Tensor::zeros(&[2]), 0.0, &opt, |p| p.x[1] >= 2.2 * H0).unwrap();
    let retries: usize = path.iter().map(|p| p.retries).sum();
    let its: usize = path.iter().map(|p| p.iterations).sum();
    let ms = t0.elapsed().as_secs_f64() * 1e3;
    println!("Arc-length continuation (from rest until w = 2.2 rise):\n");
    summarize(&path, |p| {
        format!("{:.6} | {:.8} | {:.1e}", p.x[1], truss_load(p.x[1]), (p.load - truss_load(p.x[1])).abs())
    });
    let worst = path.iter().fold(0.0f64, |m, p| m.max((p.load - truss_load(p.x[1])).abs()));
    let umax = path.iter().fold(0.0f64, |m, p| m.max(p.x[0].abs()));
    println!(
        "\nCorrector iterations {its} in all ({:.1} per point), {retries} halved steps.",
        its as f64 / path.len() as f64
    );
    println!(
        "\n{} points in {ms:.1} ms; worst |load - P(w)| {worst:.1e} over the path; |u| <= {umax:.1e}.\n",
        path.len()
    );
    limits(&cont, &path, &opt, |p| p.x[1], &[(ws, pmax), (2.0 * H0 - ws, -pmax)]);
    println!();
    let rows: Vec<String> = path.iter().map(|p| row(p, p.x[1], truss_load(p.x[1]))).collect();
    out(dir, "truss_arc.csv", CSV_HEADER, &rows);
}

fn dynamics_cases(dir: &Option<String>) {
    println!("## 3. Explicit time stepping, 10^4 steps\n");
    // Pendulum: E = 1 - cos(theta), unit mass and length, theta0 = 1 rad.
    let mut g = Graph::new();
    let th = g.input("th");
    let c = g.unary(Op::Cos, th);
    let cs = g.sum(c);
    let one = g.scalar(1.0);
    let pe = g.sub(one, cs);
    let pend = Dynamics::new(&g, pe, "th", &BTreeMap::from([("th".into(), vec![1])])).unwrap();
    // FPU-beta chain: 8 masses between walls, s^2/2 + s^4/4 per spring.
    let m = 8;
    let mut g = Graph::new();
    let x = g.input("x");
    let z = g.konst(Tensor::vector(vec![0.0]));
    let ext = g.concat(&[z, x, z]);
    let a = g.slice(ext, 1, m + 1);
    let b = g.slice(ext, 0, m + 1);
    let s = g.sub(a, b);
    let s2 = g.mul(s, s);
    let s4 = g.mul(s2, s2);
    let hv = g.scalar(0.5);
    let qv = g.scalar(0.25);
    let t1 = g.mul(s2, hv);
    let t2 = g.mul(s4, qv);
    let t = g.add(t1, t2);
    let pe = g.sum(t);
    let chain = Dynamics::new(&g, pe, "x", &BTreeMap::from([("x".into(), vec![m])])).unwrap();
    let x0: Vec<f64> = (0..m).map(|i| 0.5 * (std::f64::consts::PI * (i + 1) as f64 / (m + 1) as f64).sin()).collect();

    println!("| model | scheme | dt | max |dE|/E0 | final |dE|/E0 | us per step |");
    println!("|---|---|---|---|---|---|");
    let mut rows = Vec::new();
    for (name, dyns, xs, dts) in [
        ("pendulum, 1 rad", &pend, vec![1.0], [0.01, 0.005]),
        ("FPU-beta chain, 8 masses", &chain, x0.clone(), [0.05, 0.025]),
    ] {
        let n = xs.len();
        let mass = vec![1.0; n];
        for scheme in [Scheme::ExplicitEuler, Scheme::SymplecticEuler, Scheme::VelocityVerlet] {
            for dt in dts {
                let mut env = Env::new();
                let mut st = dyns.start(&env, xs.clone(), vec![0.0; n]).unwrap();
                let e0 = st.potential + st.kinetic(&mass);
                let mut worst = 0.0f64;
                let t0 = Instant::now();
                for _ in 0..10_000 {
                    dyns.step(&mut env, &mut st, &mass, dt, scheme).unwrap();
                    let e = st.potential + st.kinetic(&mass);
                    worst = worst.max(((e - e0) / e0).abs());
                }
                let us = t0.elapsed().as_secs_f64() * 1e6 / 1e4;
                let fin = ((st.potential + st.kinetic(&mass) - e0) / e0).abs();
                println!("| {name} | {scheme:?} | {dt} | {worst:.2e} | {fin:.2e} | {us:.1} |");
                rows.push(format!("{name},{scheme:?},{dt},{worst},{fin},{us}"));
            }
        }
    }
    println!();
    out(dir, "dynamics.csv", "model,scheme,dt,max_rel_drift,final_rel_drift,us_per_step", &rows);
}

fn main() {
    let dir = std::env::args().nth(1);
    if let Some(d) = &dir {
        std::fs::create_dir_all(d).unwrap();
    }
    rod_cases(&dir);
    section_cases(&dir);
    truss_cases(&dir);
    dynamics_cases(&dir);
}
