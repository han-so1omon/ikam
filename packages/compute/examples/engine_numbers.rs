//! Measured numbers for the compute layer: elastica accuracy against the
//! exact arc as the rod is refined, Newton-CG work, stored evaluation
//! reuse, and the planned program's per-tick time against the interpreter.
//! `cargo run --release --example engine_numbers`

use std::collections::BTreeMap;
use std::time::Instant;

use ikam_compute::Tensor;
use ikam_compute::eval::{Env, eval};
use ikam_compute::grad::grad;
use ikam_compute::ir::{Graph, Op, V};
use ikam_compute::memo::{eval_graph_stored, eval_stored};
use ikam_compute::plan::Plan;
use ikam_compute::solve::{Minimizer, Options};
use ikam_kernel::{MemStore, Repo};

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
    let ta = g.sum(last);
    let mv = g.scalar(m);
    let wm = g.mul(mv, ta);
    let s = g.unary(Op::Sin, phi);
    let ys = g.sum(s);
    let ph = g.scalar(p * h);
    let wp = g.mul(ph, ys);
    let e = g.sub(bend, wm);
    let out = g.sub(e, wp);
    (g, out)
}

fn pct(v: &mut [f64], q: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * q).round() as usize]
}

fn main() {
    let (len, ei) = (1.0, 2.0);
    let theta = 2.0 * std::f64::consts::PI / 3.0;
    let m = theta * ei / len;
    println!("elastica, 120 degree end-moment bend, tip error against the exact arc");
    println!("segments  tip_error/L  newton  cg_steps  ms");
    for n in [25usize, 50, 100, 200, 400] {
        let (g, e) = elastica(n, len, ei, m, 0.0);
        let shapes = BTreeMap::from([("phi".into(), vec![n])]);
        let t0 = Instant::now();
        let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
        let (phi, r) = solver.minimize(&Env::new(), Tensor::zeros(&[n]), &Options::default()).unwrap();
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        let h = len / n as f64;
        let (x, y): (f64, f64) = (phi.data.iter().map(|a| h * a.cos()).sum(), phi.data.iter().map(|a| h * a.sin()).sum());
        let k = m / ei;
        let err = ((x - theta.sin() / k).powi(2) + (y - (1.0 - theta.cos()) / k).powi(2)).sqrt();
        println!("{n:8}  {err:11.3e}  {:6}  {:8}  {ms:.1}", r.iterations, r.cg_steps);
    }

    let n = 200;
    let (g, e) = elastica(n, 1.0, 1.0, 0.0, 1.0);
    let shapes = BTreeMap::from([("phi".into(), vec![n])]);
    let solver = Minimizer::new(&g, e, "phi", &shapes).unwrap();
    let (phi, r) = solver.minimize(&Env::new(), Tensor::zeros(&[n]), &Options::default()).unwrap();
    let h = 1.0 / n as f64;
    let y: f64 = phi.data.iter().map(|a| h * a.sin()).sum();
    let x: f64 = phi.data.iter().map(|a| h * a.cos()).sum();
    println!("\nend load PL^2/EI = 1, 200 segments: tip deflection {y:.5} L, shortening {:.5} L, {} Newton steps", 1.0 - x, r.iterations);

    // Stored evaluation of the gradient graph.
    let (g, e) = elastica(100, len, ei, m, 0.0);
    let shapes = BTreeMap::from([("phi".into(), vec![100])]);
    let gg = grad(&g, &shapes, e, &["phi"]).unwrap();
    let env = Env::from([("phi".into(), Tensor::vector((0..100).map(|i| 0.01 * i as f64).collect()))]);
    let mut repo = Repo::new(MemStore::default());
    let db = repo.db_empty().unwrap();
    let t0 = Instant::now();
    let (_, db, s1) = eval_stored(&mut repo, &db, &gg, &env, false).unwrap();
    let first = t0.elapsed().as_secs_f64() * 1e6;
    let t0 = Instant::now();
    let (_, _, s2) = eval_stored(&mut repo, &db, &gg, &env, false).unwrap();
    let again = t0.elapsed().as_secs_f64() * 1e6;
    let t0 = Instant::now();
    let _ = eval(&gg, &env).unwrap();
    let plain = t0.elapsed().as_secs_f64() * 1e6;
    println!(
        "\nstored gradient eval, {} nodes: first {first:.0} us ({} computed, {} values already stored), rerun {again:.0} us ({} reused), plain eval {plain:.0} us",
        gg.nodes.len(),
        s1.misses,
        s1.dedup,
        s2.hits
    );

    let gid = gg.put(&mut repo).unwrap();
    let db0 = repo.db_empty().unwrap();
    let t0 = Instant::now();
    let (_, db1, _) = eval_graph_stored(&mut repo, &db0, &gg, &gid, &env, false).unwrap();
    let gfirst = t0.elapsed().as_secs_f64() * 1e6;
    let t0 = Instant::now();
    let (_, _, hit) = eval_graph_stored(&mut repo, &db1, &gg, &gid, &env, false).unwrap();
    let gagain = t0.elapsed().as_secs_f64() * 1e6;
    println!("as one recorded step: first {gfirst:.0} us, rerun {gagain:.0} us (reused {hit})");

    // Per-tick time: gradient of a 50-segment rod, interpreted vs planned.
    let n = 50;
    let (g, e) = elastica(n, len, ei, m, 0.0);
    let shapes = BTreeMap::from([("phi".into(), vec![n])]);
    let gg = grad(&g, &shapes, e, &["phi"]).unwrap();
    let mut plan = Plan::new(&gg, &shapes).unwrap();
    let mut ti = Vec::new();
    let mut tp = Vec::new();
    let mut x = vec![0.0; n];
    for tick in 0..20000 {
        for (i, v) in x.iter_mut().enumerate() {
            *v = 0.001 * ((tick + i) % 97) as f64;
        }
        if tick % 10 == 0 {
            let env = Env::from([("phi".into(), Tensor::vector(x.clone()))]);
            let t0 = Instant::now();
            let out = eval(&gg, &env).unwrap();
            ti.push(t0.elapsed().as_secs_f64() * 1e6);
            std::hint::black_box(out);
        }
        let t0 = Instant::now();
        plan.set("phi", &x).unwrap();
        plan.run();
        tp.push(t0.elapsed().as_secs_f64() * 1e6);
        std::hint::black_box(plan.output(1)[0]);
    }
    println!(
        "\nper-tick gradient of a 50-segment rod ({} nodes): interpreted p50 {:.2} us p99 {:.2} us; planned p50 {:.2} us p99 {:.2} us max {:.2} us",
        gg.nodes.len(),
        pct(&mut ti, 0.5),
        pct(&mut ti, 0.99),
        pct(&mut tp, 0.5),
        pct(&mut tp, 0.99),
        pct(&mut tp, 1.0)
    );
}
