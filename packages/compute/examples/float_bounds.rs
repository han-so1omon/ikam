//! E442: bounds on nondeterminism in graph computation, measured.
//!
//! Every graph is run with the pinned arithmetic (the run of record) and
//! with `Arith::Host` (the host's math library, fused multiply-add and
//! 8-lane sums: a stand-in for another platform or build), and the two
//! are compared; `bounds::predict` gives the bound expected from rounding
//! alone. Sections: E409's five functions on its six inputs; the
//! elastica Newton solve; fixed-period loops of 10,000 steps (a damped
//! forced chain, an undamped chain, a chaotic system, and a rod solved
//! by two warm-started Newton steps a tick); the cost of a bounded check
//! on the store against the exact check.
//!
//! `cargo run --release --example float_bounds`

use std::collections::BTreeMap;
use std::time::Instant;

use ikam_compute::Tensor;
use ikam_compute::bounds::{Distance, distance, insert_within, predict, predicted_difference, within};
use ikam_compute::eval::{Arith, Env, eval, eval_with};
use ikam_compute::ir::{Graph, Op, V};
use ikam_compute::memo::{check_bounded, check_graph_bounded, eval_graph_stored, eval_stored};
use ikam_compute::solve::{Minimizer, Options};
use ikam_kernel::{MemStore, Repo};

// ---- E409's inputs (its generator, integer-only) ----

const N: usize = 1024;
const SOFT: usize = 64;
const DIM: usize = 8;

struct Lcg(u64);
impl Lcg {
    fn unit(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 52) as f64) - 1.0
    }
}

fn inputs(name: &str) -> (Vec<f64>, Vec<f64>) {
    let mut r = Lcg(409);
    let mut a: Vec<f64> = (0..N).map(|_| 10.0 * r.unit()).collect();
    let b: Vec<f64> = (0..N).map(|_| 10.0 * r.unit()).collect();
    match name {
        "plain" => {}
        "cancel" => {
            for (i, x) in a.iter_mut().enumerate() {
                *x = match i % 4 {
                    0 => 1e16 * (1.0 + *x * 1e-4),
                    2 => -1e16 * (1.0 + *x * 1e-4),
                    _ => *x,
                };
            }
        }
        "subnormal" => a.iter_mut().for_each(|x| *x *= 1e-310),
        "wide" => a.iter_mut().for_each(|x| *x *= 80.0),
        "edges" => {
            for (i, x) in a.iter_mut().take(SOFT).enumerate() {
                *x = match i {
                    0..40 => -745.2 + 36.9 * i as f64 / 39.0,
                    40..48 => 709.5 + 0.5 * (i - 40) as f64 / 7.0,
                    _ => *x * 1e-3,
                };
            }
        }
        _ => {
            a[3] = f64::from_bits(0x7ff8_0000_0000_1234);
            a[10] = f64::INFINITY;
            a[20] = f64::NEG_INFINITY;
            a[30] = f64::from_bits(1);
            a[40] = -0.0;
            a[50] = 1.7e308;
            a[60] = -1.7e308;
        }
    }
    (a, b)
}

/// E409's functions as graphs over inputs `a` and `b`.
fn function(name: &str) -> Graph {
    let mut g = Graph::new();
    let a = g.input("a");
    let b = g.input("b");
    let out = match name {
        "dot" => {
            let p = g.mul(a, b);
            g.sum(p)
        }
        "mean_var" => {
            let n = g.scalar(N as f64);
            let s = g.sum(a);
            let mean = g.div(s, n);
            let d = g.sub(a, mean);
            let dd = g.mul(d, d);
            let v = g.sum(dd);
            let var = g.div(v, n);
            let m1 = g.push(Op::Reshape { shape: vec![1] }, &[mean]);
            let v1 = g.push(Op::Reshape { shape: vec![1] }, &[var]);
            g.concat(&[m1, v1])
        }
        "softmax" | "exp" => {
            let x = g.slice(a, 0, SOFT);
            if name == "exp" {
                g.unary(Op::Exp, x)
            } else {
                // The largest by comparison, a NaN passed over unless first
                // (E409's rule): Max keeps its first argument on ties.
                let mut mx = g.slice(x, 0, 1);
                for i in 1..SOFT {
                    let xi = g.slice(x, i, 1);
                    mx = g.max(xi, mx);
                }
                let m = g.sum(mx);
                let d = g.sub(x, m);
                let e = g.unary(Op::Exp, d);
                let s = g.sum(e);
                g.div(e, s)
            }
        }
        _ => {
            let x = g.slice(a, 0, DIM * DIM);
            let y = g.slice(b, 0, DIM * DIM);
            let x = g.push(Op::Reshape { shape: vec![DIM, DIM] }, &[x]);
            let y = g.push(Op::Reshape { shape: vec![DIM, DIM] }, &[y]);
            g.push(Op::MatMul, &[x, y])
        }
    };
    g.outputs = vec![out];
    g
}

/// The tolerances tried, tightest first.
const TOLS: [(&str, u64, f64, f64); 7] = [
    ("ulp 1", 1, 0.0, 0.0),
    ("ulp 4", 4, 0.0, 0.0),
    ("ulp 64", 64, 0.0, 0.0),
    ("rel 1e-15", 0, 0.0, 1e-15),
    ("rel 1e-12", 0, 0.0, 1e-12),
    ("rel 1e-9", 0, 0.0, 1e-9),
    ("rel 1e-9 or abs 1e-300", 0, 1e-300, 1e-9),
];

fn fmt_ulp(u: u64) -> String {
    if u == u64::MAX { "inf".into() } else { u.to_string() }
}

fn passes(r: &Tensor, x: &Tensor) -> String {
    if r.bits_eq(x) {
        return "same bits".into();
    }
    TOLS.iter().filter(|(_, u, a, rel)| within(r, x, *u, *a, *rel)).map(|t| t.0).next().unwrap_or("none").to_string()
}

fn e409() {
    println!("## E409's functions, pinned (record) against host (rerun)\n");
    println!(
        "| function | input | elements differing | max ULP | max abs | max rel | predicted abs | tightest passing |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    let mut summary: BTreeMap<&str, usize> = BTreeMap::new();
    let mut total = 0;
    for input in ["plain", "cancel", "subnormal", "wide", "edges", "special"] {
        let (a, b) = inputs(input);
        let env = Env::from([("a".into(), Tensor::vector(a)), ("b".into(), Tensor::vector(b))]);
        for func in ["dot", "mean_var", "softmax", "exp", "matmul"] {
            let g = function(func);
            let p = eval(&g, &env).unwrap().remove(0);
            let h = eval_with(&g, &env, Arith::Host).unwrap().remove(0);
            let d = distance(&p, &h);
            let pred = predicted_difference(&predict(&g, &env, &BTreeMap::new()).unwrap())[g.outputs[0]];
            let pass = passes(&p, &h);
            total += 1;
            for (name, u, ab, rel) in TOLS {
                if within(&p, &h, u, ab, rel) {
                    *summary.entry(name).or_default() += 1;
                }
            }
            println!(
                "| {func} | {input} | {} of {} | {} | {:.2e} | {:.2e} | {:.2e} | {pass} |",
                d.differing,
                p.data.len(),
                fmt_ulp(d.max_ulp),
                d.max_abs,
                d.max_rel,
                pred
            );
        }
    }
    println!("\nPassing, of {total} function-input pairs:");
    for (name, ..) in TOLS {
        println!("- {name}: {}", summary.get(name).copied().unwrap_or(0));
    }

    // The same through the store: a bound on each output, checked.
    let mut repo = Repo::new(MemStore::default());
    let (mut ok, mut out, mut same) = (0, 0, 0);
    for input in ["plain", "cancel", "subnormal", "wide", "edges", "special"] {
        let (a, b) = inputs(input);
        let env = Env::from([("a".into(), Tensor::vector(a)), ("b".into(), Tensor::vector(b))]);
        for func in ["dot", "mean_var", "softmax", "exp", "matmul"] {
            let g = function(func);
            let (g, _) =
                insert_within(&g, &[g.outputs[0]], &Op::Within { ulp: 4, abs: 0.0, rel: 1e-12, local: false }).unwrap();
            let db = repo.db_empty().unwrap();
            let (_, db, _) = eval_stored(&mut repo, &db, &g, &env, false).unwrap();
            match check_bounded(&repo, &db, &g, &env, Arith::Host) {
                Ok(r) if r.same() => same += 1,
                Ok(r) if r.passed() => ok += 1,
                Ok(_) => out += 1,
                Err(e) => println!("check failed: {e}"),
            }
        }
    }
    println!(
        "\nStored and checked with `Within {{ ulp: 4, rel: 1e-12 }}` on each output: {same} same, {ok} within, {out} outside.\n"
    );
}

// ---- the elastica (engine_numbers' energy, the moment an input) ----

fn elastica(n: usize, len: f64, ei: f64, p: f64) -> (Graph, V) {
    let h = len / n as f64;
    let mut g = Graph::new();
    let phi = g.input("phi");
    let m = g.input("m");
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
    let ta = g.sum(last);
    let wm = g.mul(m, ta);
    let s = g.unary(Op::Sin, phi);
    let ys = g.sum(s);
    let ph = g.scalar(p * h);
    let wp = g.mul(ph, ys);
    let e = g.sub(bend, wm);
    let out = g.sub(e, wp);
    (g, out)
}

fn tip(phi: &[f64], len: f64) -> (f64, f64) {
    let h = len / phi.len() as f64;
    let (mut x, mut y) = (0.0, 0.0);
    for a in phi {
        x += h * libm::cos(*a);
        y += h * libm::sin(*a);
    }
    (x, y)
}

fn maxabs(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).fold(0.0f64, |m, (x, y)| m.max((x - y).abs()))
}

fn newton() {
    println!("## Elastica Newton-CG, pinned against host\n");
    println!("| case | segments | mode | Newton steps | CG steps | tip x / L | tip y / L | tip error vs exact arc |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    let (len, ei) = (1.0, 2.0);
    let theta = 2.0 * std::f64::consts::PI / 3.0;
    let mut diffs = Vec::new();
    for (case, n, m, p) in
        [("end moment 120 deg", 100usize, theta * ei / len, 0.0), ("end load PL^2/EI = 1", 200, 0.0, 2.0)]
    {
        let (g, e) = elastica(n, len, ei, p);
        let shapes = BTreeMap::from([("phi".into(), vec![n]), ("m".into(), vec![])]);
        let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
        let env = Env::from([("m".into(), Tensor::scalar(m))]);
        let mut sols = Vec::new();
        for arith in [Arith::Pinned, Arith::Host] {
            let (phi, r) = solver.minimize_with(&env, Tensor::zeros(&[n]), &Options::default(), arith).unwrap();
            let (x, y) = tip(&phi.data, len);
            let err = if p == 0.0 {
                let k = m / ei;
                format!("{:.3e}", ((x - theta.sin() / k).powi(2) + (y - (1.0 - theta.cos()) / k).powi(2)).sqrt())
            } else {
                "(no closed form)".into()
            };
            println!("| {case} | {n} | {arith:?} | {} | {} | {x:.12} | {y:.12} | {err} |", r.iterations, r.cg_steps);
            sols.push((phi, r, x, y));
        }
        let (a, b) = (&sols[0], &sols[1]);
        let mut at = env.clone();
        at.insert("phi".into(), Tensor::vector(a.0.data.iter().map(|v| v * 1.1 + 0.01).collect()));
        let gd = distance(
            &eval(&solver.gradient, &at).unwrap()[1],
            &eval_with(&solver.gradient, &at, Arith::Host).unwrap()[1],
        );
        println!(
            "|  |  | gradient graph at a nearby point: {} of {n} elements differ, max {:.2e} |  |  |  |  |  |",
            gd.differing, gd.max_abs
        );
        let tipd = ((a.2 - b.2).powi(2) + (a.3 - b.3).powi(2)).sqrt();
        let d = distance(&a.0, &b.0);
        diffs.push((case, n, tipd, d, a.1.grad_norm, b.1.grad_norm));
        // Error growth along the Newton steps: the iterate after k steps.
        let mut rows = Vec::new();
        for k in 0..=a.1.iterations.max(b.1.iterations) {
            let opt = Options { max_iter: k, budget: true, ..Options::default() };
            let (pa, ra) = solver.minimize_with(&env, Tensor::zeros(&[n]), &opt, Arith::Pinned).unwrap();
            let (pb, _) = solver.minimize_with(&env, Tensor::zeros(&[n]), &opt, Arith::Host).unwrap();
            let dk = distance(&pa, &pb);
            rows.push(format!(
                "{k}: max |dphi| {:.2e} ({} ULP), |g| {:.1e}",
                dk.max_abs,
                fmt_ulp(dk.max_ulp),
                ra.grad_norm
            ));
        }
        println!("|  |  | steps |  |  | {} |  |  |", rows.join("; "));
    }
    println!(
        "\n| case | tip difference pinned vs host / L | max angle difference | max ULP | final \\|g\\| pinned, host |"
    );
    println!("| --- | --- | --- | --- | --- |");
    for (case, n, tipd, d, ga, gb) in diffs {
        println!("| {case} ({n}) | {tipd:.2e} | {:.2e} | {} | {ga:.1e}, {gb:.1e} |", d.max_abs, fmt_ulp(d.max_ulp));
    }
    println!();
}

// ---- fixed-period loops ----

const DT: f64 = 1e-3;

/// A chain of 8 masses on springs (fixed ends) with a tanh softening, a
/// damping `c`, and a force `u` on the first: semi-implicit Euler at 1 kHz.
/// State `s` = [q (8), p (8)]. The stiffness is a matrix product.
fn chain_step(c: f64) -> Graph {
    let n = 8;
    let k = 1e4;
    let mut kd = vec![0.0; n * n];
    for i in 0..n {
        kd[i * n + i] = 2.0 * k;
        if i > 0 {
            kd[i * n + i - 1] = -k;
        }
        if i + 1 < n {
            kd[i * n + i + 1] = -k;
        }
    }
    let mut g = Graph::new();
    let s = g.input("s");
    let u = g.input("u");
    let q = g.slice(s, 0, n);
    let p = g.slice(s, n, n);
    let kmat = g.konst(Tensor::new(vec![n, n], kd).unwrap());
    let qc = g.push(Op::Reshape { shape: vec![n, 1] }, &[q]);
    let kq = g.push(Op::MatMul, &[kmat, qc]);
    let kq = g.push(Op::Reshape { shape: vec![n] }, &[kq]);
    let th = g.unary(Op::Tanh, q);
    let beta = g.scalar(50.0);
    let nl = g.mul(beta, th);
    let cc = g.scalar(c);
    let damp = g.mul(cc, p);
    let f = g.add(kq, nl);
    let f = g.add(f, damp);
    let f = g.neg(f);
    let mut e0 = vec![0.0; n];
    e0[0] = 1.0;
    let e0 = g.konst(Tensor::vector(e0));
    let fu = g.mul(u, e0);
    let f = g.add(f, fu);
    let dt = g.scalar(DT);
    let dp = g.mul(dt, f);
    let p2 = g.add(p, dp);
    let dq = g.mul(dt, p2);
    let q2 = g.add(q, dq);
    let out = g.concat(&[q2, p2]);
    g.outputs = vec![out];
    g
}

/// The Lorenz system (10, 28, 8/3), explicit Euler, 0.005 time units a
/// tick: chaotic. The linear part is a matrix product.
fn lorenz_step() -> Graph {
    let (sigma, rho, beta) = (10.0, 28.0, 8.0 / 3.0);
    let mut g = Graph::new();
    let s = g.input("s");
    let _u = g.input("u");
    let a = g.konst(Tensor::new(vec![3, 3], vec![-sigma, sigma, 0.0, rho, -1.0, 0.0, 0.0, 0.0, -beta]).unwrap());
    let sc = g.push(Op::Reshape { shape: vec![3, 1] }, &[s]);
    let lin = g.push(Op::MatMul, &[a, sc]);
    let lin = g.push(Op::Reshape { shape: vec![3] }, &[lin]);
    let x = g.slice(s, 0, 1);
    let y = g.slice(s, 1, 1);
    let z = g.slice(s, 2, 1);
    let xz = g.mul(x, z);
    let nxz = g.neg(xz);
    let xy = g.mul(x, y);
    let zero = g.konst(Tensor::vector(vec![0.0]));
    let nl = g.concat(&[zero, nxz, xy]);
    let ds = g.add(lin, nl);
    let dt = g.scalar(0.005);
    let st = g.mul(dt, ds);
    let out = g.add(s, st);
    g.outputs = vec![out];
    g
}

struct LoopRow {
    step: usize,
    chained: Distance,
    chained_max: f64,
    local_max_abs: f64,
    local_max_ulp: u64,
    pred_chained: f64,
    pred_local_max: f64,
    scale: f64,
}

fn run_loop(g: &Graph, s0: Vec<f64>, u: impl Fn(usize) -> f64, steps: usize, marks: &[usize]) -> Vec<LoopRow> {
    let mut sp = Tensor::vector(s0.clone());
    let mut sh = Tensor::vector(s0);
    let mut perr = Tensor::zeros(&sp.shape);
    let (mut lmax, mut lulp, mut plmax, mut cmax) = (0.0f64, 0u64, 0.0f64, 0.0f64);
    let mut rows = Vec::new();
    for k in 1..=steps {
        let uk = Tensor::scalar(u(k));
        let envp = Env::from([("s".into(), sp.clone()), ("u".into(), uk.clone())]);
        let envh = Env::from([("s".into(), sh.clone()), ("u".into(), uk.clone())]);
        let np = eval(g, &envp).unwrap().remove(0);
        let nh = eval_with(g, &envh, Arith::Host).unwrap().remove(0);
        // One host step from the recorded (pinned) state: the step's own
        // difference, what a local bound sees.
        let nl = eval_with(g, &envp, Arith::Host).unwrap().remove(0);
        let dl = distance(&np, &nl);
        lmax = lmax.max(dl.max_abs);
        lulp = lulp.max(dl.max_ulp);
        // Predicted: chained (the last bound carried in) and per step.
        let chained = predict(g, &envp, &BTreeMap::from([("s".into(), perr.clone())])).unwrap();
        let local = predict(g, &envp, &BTreeMap::new()).unwrap();
        perr = chained[g.outputs[0]].clone();
        plmax = plmax.max(predicted_difference(&local)[g.outputs[0]]);
        let dc = distance(&np, &nh);
        cmax = cmax.max(dc.max_abs);
        if marks.contains(&k) {
            rows.push(LoopRow {
                step: k,
                chained_max: cmax,
                chained: dc,
                local_max_abs: lmax,
                local_max_ulp: lulp,
                pred_chained: 2.0 * perr.data.iter().fold(0.0f64, |m, v| m.max(*v)),
                pred_local_max: plmax,
                scale: np.data.iter().fold(0.0f64, |m, v| m.max(v.abs())),
            });
        }
        sp = np;
        sh = nh;
    }
    rows
}

fn print_loop(name: &str, rows: &[LoopRow]) {
    println!("\n{name}\n");
    println!(
        "| step | chained max abs | chained, largest so far | chained max ULP | state scale | predicted chained | per-step max abs (so far) | per-step max ULP | predicted per step |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for r in rows {
        println!(
            "| {} | {:.2e} | {:.2e} | {} | {:.2e} | {:.2e} | {:.2e} | {} | {:.2e} |",
            r.step,
            r.chained.max_abs,
            r.chained_max,
            fmt_ulp(r.chained.max_ulp),
            r.scale,
            r.pred_chained,
            r.local_max_abs,
            r.local_max_ulp,
            r.pred_local_max
        );
    }
}

fn loops() {
    println!("## Fixed-period loops, 10,000 steps, pinned against host\n");
    println!("Chained: each run carries its own state. Per step: one host step from the pinned state.");
    let marks = [1, 10, 100, 1000, 2000, 5000, 10000];
    let steps = 10000;
    let mut s0 = vec![0.0; 16];
    for (i, v) in s0.iter_mut().take(8).enumerate() {
        *v = 0.01 * (i as f64 + 1.0) * if i % 2 == 0 { 1.0 } else { -0.7 };
    }
    let t0 = Instant::now();
    let rows = run_loop(
        &chain_step(5.0),
        s0.clone(),
        |k| 50.0 * libm::sin(2.0 * std::f64::consts::PI * 3.0 * k as f64 * DT),
        steps,
        &marks,
    );
    print_loop("Damped chain (c = 5 /s), forced at 3 Hz: contracting", &rows);
    let rows = run_loop(&chain_step(0.0), s0, |_| 0.0, steps, &marks);
    print_loop("Undamped chain, free: neutral", &rows);
    let rows = run_loop(&lorenz_step(), vec![1.0, 1.0, 1.0], |_| 0.0, steps, &marks);
    print_loop("Lorenz, 0.005 a tick: chaotic", &rows);
    eprintln!("loops: {:.1} s", t0.elapsed().as_secs_f64());

    // A rod solved every tick by two warm-started Newton steps (E432's
    // pattern): the moment varies, the end load stays.
    let n = 24;
    let ticks = 2000;
    let (g, e) = elastica(n, 1.0, 2.0, 2.0);
    let shapes = BTreeMap::from([("phi".into(), vec![n]), ("m".into(), vec![])]);
    let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
    let opt = Options { max_iter: 2, budget: true, ..Options::default() };
    let full = Options::default();
    let (mut xp, mut xh) = (Tensor::zeros(&[n]), Tensor::zeros(&[n]));
    let (mut lmax, mut lulp, mut cmax) = (0.0f64, 0u64, 0.0f64);
    let mut rows = Vec::new();
    for k in 1..=ticks {
        let m = 2.0 * (1.0 + 0.5 * libm::sin(2.0 * std::f64::consts::PI * k as f64 * DT));
        let env = Env::from([("m".into(), Tensor::scalar(m))]);
        let (np, rp) = solver.minimize_with(&env, xp.clone(), &opt, Arith::Pinned).unwrap();
        let (nh, _) = solver.minimize_with(&env, xh, &opt, Arith::Host).unwrap();
        let (nl, _) = solver.minimize_with(&env, xp.clone(), &opt, Arith::Host).unwrap();
        let dl = distance(&np, &nl);
        lmax = lmax.max(dl.max_abs);
        lulp = lulp.max(dl.max_ulp);
        let dc = distance(&np, &nh);
        cmax = cmax.max(dc.max_abs);
        if [1, 10, 100, 1000, 2000].contains(&k) {
            // The tick's iterate against a solve from zero to the default
            // tolerance: what two warm-started steps leave unconverged.
            let (conv, _) = solver.minimize_with(&env, Tensor::zeros(&[n]), &full, Arith::Pinned).unwrap();
            rows.push((k, dc, cmax, lmax, lulp, maxabs(&np.data, &conv.data), rp.grad_norm));
        }
        xp = np;
        xh = nh;
    }
    println!("\nRod, 24 segments, two warm-started Newton steps a tick, moment varying at 1 Hz\n");
    println!(
        "| tick | chained max abs | chained, largest so far | chained max ULP | per-step max abs (so far) | per-step max ULP | iterate vs a full solve | \\|g\\| after the tick |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for (k, d, cm, l, lu, c, gn) in rows {
        println!(
            "| {k} | {:.2e} | {cm:.2e} | {} | {l:.2e} | {lu} | {c:.2e} | {gn:.1e} |",
            d.max_abs,
            fmt_ulp(d.max_ulp)
        );
    }
    println!();
}

// ---- the check on the store ----

/// `steps` steps of a step graph unrolled into one graph, with a local
/// bound on each step's state (`per_step`) or a carried one on the final
/// state only.
fn unrolled(step: &Graph, steps: usize, u: impl Fn(usize) -> f64, per_step: bool) -> Graph {
    let mut g = Graph::new();
    let mut s = g.input("s");
    let b = Op::Within { ulp: 0, abs: 0.0, rel: 0.0, local: per_step };
    for k in 1..=steps {
        let uk = g.scalar(u(k));
        let out = g.inline(step, &BTreeMap::from([("s".into(), s), ("u".into(), uk)])).unwrap()[0];
        s = if per_step || k == steps { g.push(b.clone(), &[out]) } else { out };
    }
    g.outputs = vec![s];
    g
}

fn time<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let t0 = Instant::now();
    let v = f();
    (v, t0.elapsed().as_secs_f64() * 1e3)
}

/// A loop's input at each step.
type Forcing = Box<dyn Fn(usize) -> f64>;

fn store_checks() {
    println!("## Checks on the store, unrolled loops (1,000 steps in one graph)\n");
    println!(
        "| system | bounds | largest abs tolerance | nodes | plain eval ms | record ms | exact check ms | bounded check (pinned) ms | bounded check (host) ms | differing nodes | verdicts (same / within / outside) |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    let steps = 1000;
    let force = |k: usize| 50.0 * libm::sin(2.0 * std::f64::consts::PI * 3.0 * k as f64 * DT);
    let mut s0 = vec![0.0; 16];
    for (i, v) in s0.iter_mut().take(8).enumerate() {
        *v = 0.01 * (i as f64 + 1.0);
    }
    let systems: Vec<(&str, Graph, Vec<f64>, Forcing)> = vec![
        ("damped chain", chain_step(5.0), s0, Box::new(force)),
        ("Lorenz", lorenz_step(), vec![1.0, 1.0, 1.0], Box::new(|_| 0.0)),
    ];
    for (name, step, s0, u) in systems {
        let env = Env::from([("s".into(), Tensor::vector(s0))]);
        for (label, per_step, factor) in [
            ("none", false, -1.0),
            ("per step, predicted x 1", true, 1.0),
            ("final only, predicted x 1", false, 1.0),
            ("final only, abs 1e-12", false, 0.0),
        ] {
            let g = unrolled(&step, steps, &u, per_step);
            let g = if factor > 0.0 {
                ikam_compute::bounds::fill_from_prediction(&g, &env, &BTreeMap::new(), factor).unwrap()
            } else {
                let mut h = g.clone();
                let o = h.outputs[0];
                if factor == 0.0 {
                    h.nodes[o].op = Op::Within { ulp: 0, abs: 1e-12, rel: 0.0, local: false };
                } else {
                    h.nodes.pop();
                    h.outputs = vec![o - 1];
                }
                h
            };
            let tol = g
                .nodes
                .iter()
                .filter_map(|n| match n.op {
                    Op::Within { abs, .. } => Some(abs),
                    _ => None,
                })
                .fold(f64::NAN, f64::max);
            let (_, plain) = time(|| eval(&g, &env).unwrap());
            let mut repo = Repo::new(MemStore::default());
            let db0 = repo.db_empty().unwrap();
            let ((_, db, _), rec) = time(|| eval_stored(&mut repo, &db0, &g, &env, false).unwrap());
            let (_, exact) = time(|| eval_stored(&mut repo, &db, &g, &env, true).unwrap());
            let (rp, bp) = time(|| check_bounded(&repo, &db, &g, &env, Arith::Pinned).unwrap());
            assert!(rp.same());
            let (rh, bh) = time(|| check_bounded(&repo, &db, &g, &env, Arith::Host));
            let (diff, verdicts) = match rh {
                Ok(r) => {
                    let c = |v| r.bounds.iter().filter(|b| b.verdict == v).count();
                    use ikam_compute::bounds::Verdict::*;
                    let worst = r.bounds.iter().map(|b| b.distance.max_abs).fold(0.0f64, f64::max);
                    (
                        r.differing.to_string(),
                        format!("{} / {} / {} (largest move {worst:.2e})", c(Same), c(Within), c(Outside)),
                    )
                }
                Err(ikam_compute::Error::Nondeterministic { .. }) => {
                    ("-".into(), "refused: Nondeterministic (a node without a bound differs)".into())
                }
                Err(e) => ("-".into(), format!("error: {e}")),
            };
            let tol = if tol.is_nan() { "-".to_string() } else { format!("{tol:.2e}") };
            println!(
                "| {name} | {label} | {tol} | {} | {plain:.0} | {rec:.0} | {exact:.0} | {bp:.0} | {bh:.0} | {diff} | {verdicts} |",
                g.nodes.len()
            );
        }
    }
    // Whole-graph records: one record, the bound on the output.
    let g = unrolled(&chain_step(5.0), steps, force, false);
    let mut h = g.clone();
    let o = h.outputs[0];
    h.nodes[o].op = Op::Within { ulp: 0, abs: 1e-9, rel: 0.0, local: false };
    let env = Env::from([("s".into(), Tensor::vector(vec![0.01; 16]))]);
    let mut repo = Repo::new(MemStore::default());
    let gid = h.put(&mut repo).unwrap();
    let db0 = repo.db_empty().unwrap();
    let ((_, db, _), rec) = time(|| eval_graph_stored(&mut repo, &db0, &h, &gid, &env, false).unwrap());
    let (_, exact) = time(|| eval_graph_stored(&mut repo, &db, &h, &gid, &env, true).unwrap());
    let (r, bounded) = time(|| check_graph_bounded(&repo, &db, &h, &gid, &env, Arith::Host).unwrap());
    println!(
        "\nWhole-graph record of the damped chain (one record, `abs 1e-9` on the output): record {rec:.1} ms, exact check {exact:.1} ms, bounded host check {bounded:.1} ms, verdict {:?} (moved {:.2e}).\n",
        r.bounds[0].verdict, r.bounds[0].distance.max_abs
    );
}

fn main() {
    e409();
    newton();
    loops();
    store_checks();
}
