//! Equality constraints and path following (E439), opt-in beside the
//! solvers in [`crate::solve`], which are unchanged.
//!
//! - [`Linear`]: prescribed values of chosen inputs or of linear
//!   combinations of them (`A x = b`), a displacement or curvature that a
//!   process imposes. [`Minimizer::minimize_constrained`] keeps Newton-CG
//!   in the null space of `A` (projected CG, Nocedal and Wright 16.3) and
//!   returns the Lagrange multipliers `lambda` with `grad E = A^T lambda`:
//!   the reactions (the moment or force the constraint has to apply).
//! - [`Continuation`]: an energy with a scalar load parameter followed
//!   along its equilibrium path by pseudo arc-length (Keller; Riks and
//!   Crisfield in mechanics), through limit points where the load
//!   reverses and Newton under load control stalls or jumps. Each step
//!   solves the bordered system `[K q; t^T] [dx; dl] = -[R; N]` densely
//!   (the stiffness `K` assembled from Hessian-vector products), which
//!   stays regular at a limit point where `K` is singular. Dense: for
//!   models up to a few hundred unknowns.

use std::collections::BTreeMap;

use crate::eval::{Env, eval};
use crate::grad::{grad, hvp};
use crate::ir::{Graph, V};
use crate::solve::{Minimizer, Options, Report};
use crate::{Error, Tensor};

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn max_abs(v: &[f64]) -> f64 {
    v.iter().fold(0.0f64, |m, x| m.max(x.abs()))
}

fn finite(f: f64, g: &[f64]) -> Result<(), Error> {
    if !f.is_finite() || g.iter().any(|v| !v.is_finite()) {
        return Err(Error::NoConvergence("the energy or its gradient is not finite here".into()));
    }
    Ok(())
}

/// Solve the dense `n` by `n` system `a y = b` (row-major) by LU with
/// partial pivoting, in place: `b` becomes `y`. Returns the sign of the
/// determinant, or an error when a pivot vanishes.
pub fn lu_solve(a: &mut [f64], n: usize, b: &mut [f64]) -> Result<f64, Error> {
    let mut sign = 1.0;
    let scale = max_abs(a).max(f64::MIN_POSITIVE);
    for k in 0..n {
        let mut p = k;
        for i in k + 1..n {
            if a[i * n + k].abs() > a[p * n + k].abs() {
                p = i;
            }
        }
        if a[p * n + k].abs() <= 1e-300 * scale || !a[p * n + k].is_finite() {
            return Err(Error::NoConvergence("singular system".into()));
        }
        if p != k {
            for j in 0..n {
                a.swap(k * n + j, p * n + j);
            }
            b.swap(k, p);
            sign = -sign;
        }
        let piv = a[k * n + k];
        if piv < 0.0 {
            sign = -sign;
        }
        for i in k + 1..n {
            let f = a[i * n + k] / piv;
            if f == 0.0 {
                continue;
            }
            for j in k..n {
                a[i * n + j] -= f * a[k * n + j];
            }
            b[i] -= f * b[k];
        }
    }
    for k in (0..n).rev() {
        let mut s = b[k];
        for j in k + 1..n {
            s -= a[k * n + j] * b[j];
        }
        b[k] = s / a[k * n + k];
    }
    Ok(sign)
}

/// Linear equality constraints `A x = b` over the flattened variable:
/// each row a few (index, coefficient) pairs and a value.
#[derive(Clone, Debug, Default)]
pub struct Linear {
    pub rows: Vec<Vec<(usize, f64)>>,
    pub values: Vec<f64>,
}

impl Linear {
    pub fn new() -> Linear {
        Linear::default()
    }

    /// Prescribe `x[i] = value`.
    pub fn fix(mut self, i: usize, value: f64) -> Linear {
        self.rows.push(vec![(i, 1.0)]);
        self.values.push(value);
        self
    }

    /// Prescribe `sum c x[i] = value`.
    pub fn combo(mut self, terms: &[(usize, f64)], value: f64) -> Linear {
        self.rows.push(terms.to_vec());
        self.values.push(value);
        self
    }

    /// `A x - b`.
    pub fn residual(&self, x: &[f64]) -> Vec<f64> {
        self.rows.iter().zip(&self.values).map(|(r, b)| r.iter().map(|(i, c)| c * x[*i]).sum::<f64>() - b).collect()
    }

    fn apply(&self, x: &[f64]) -> Vec<f64> {
        self.rows.iter().map(|r| r.iter().map(|(i, c)| c * x[*i]).sum()).collect()
    }

    fn apply_t(&self, y: &[f64], out: &mut [f64]) {
        for (r, yk) in self.rows.iter().zip(y) {
            for (i, c) in r {
                out[*i] += c * yk;
            }
        }
    }

    /// `A A^T`, dense.
    fn gram(&self, n: usize) -> Vec<f64> {
        let m = self.rows.len();
        let mut dense = vec![0.0; n];
        let mut g = vec![0.0; m * m];
        for (a, ra) in self.rows.iter().enumerate() {
            dense.fill(0.0);
            for (i, c) in ra {
                dense[*i] += c;
            }
            for (b, rb) in self.rows.iter().enumerate() {
                g[a * m + b] = rb.iter().map(|(i, c)| c * dense[*i]).sum();
            }
        }
        g
    }

    /// `y` with `A A^T y = A v`.
    fn coeffs(&self, gram: &[f64], v: &[f64]) -> Result<Vec<f64>, Error> {
        let m = self.rows.len();
        let mut y = self.apply(v);
        let mut g = gram.to_vec();
        lu_solve(&mut g, m, &mut y).map_err(|_| Error::Shape("constraint rows are dependent".into()))?;
        Ok(y)
    }

    /// `v - A^T (A A^T)^-1 A v`: `v` in the null space of `A`.
    fn project(&self, gram: &[f64], v: &mut [f64]) -> Result<(), Error> {
        let y = self.coeffs(gram, v)?;
        let mut at = vec![0.0; v.len()];
        self.apply_t(&y, &mut at);
        for (a, b) in v.iter_mut().zip(&at) {
            *a -= b;
        }
        Ok(())
    }

    /// The nearest point to `x` (least change) that meets the constraints.
    fn restore(&self, gram: &[f64], x: &mut [f64]) -> Result<(), Error> {
        let m = self.rows.len();
        let mut r = self.residual(x);
        let mut g = gram.to_vec();
        lu_solve(&mut g, m, &mut r).map_err(|_| Error::Shape("constraint rows are dependent".into()))?;
        let mut at = vec![0.0; x.len()];
        self.apply_t(&r, &mut at);
        for (a, b) in x.iter_mut().zip(&at) {
            *a -= b;
        }
        Ok(())
    }
}

/// A constrained solve's outcome.
#[derive(Clone, Debug, Default)]
pub struct Constrained {
    pub report: Report,
    /// One per constraint row: `grad E = A^T lambda` at the solution.
    pub multipliers: Vec<f64>,
    /// The largest `|A x - b|` at the solution.
    pub residual: f64,
}

impl Minimizer {
    /// Minimize subject to `con` (prescribed values or combinations).
    /// Starts from the point nearest `x0` that meets the constraints,
    /// then takes Newton-CG steps in the constraints' null space, so every
    /// iterate meets them; converged when the projected gradient is below
    /// `opt.tol`. `opt.budget` behaves as in `minimize`.
    pub fn minimize_constrained(
        &self,
        env: &Env,
        x0: Tensor,
        con: &Linear,
        opt: &Options,
    ) -> Result<(Tensor, Constrained), Error> {
        let n = x0.data.len();
        if let Some(i) = con.rows.iter().flatten().map(|(i, _)| *i).find(|i| *i >= n) {
            return Err(Error::Shape(format!("constraint on index {i} of {n}")));
        }
        let gram = con.gram(n);
        let mut env = env.clone();
        let mut x = x0;
        con.restore(&gram, &mut x.data)?;
        let mut rep = Report::default();
        let vname = format!("v:{}", self.var);
        for it in 0..=opt.max_iter {
            env.insert(self.var.clone(), x.clone());
            let out = eval(&self.gradient, &env)?;
            rep.evals += 1;
            let (f, g) = (out[0].item(), out[1].data.clone());
            finite(f, &g)?;
            let mut pg = g.clone();
            con.project(&gram, &mut pg)?;
            let gn = max_abs(&pg);
            rep.iterations = it;
            rep.energy = f;
            rep.grad_norm = gn;
            if gn < opt.tol || (opt.budget && it == opt.max_iter) {
                let multipliers = con.coeffs(&gram, &g)?;
                let residual = max_abs(&con.residual(&x.data));
                return Ok((x, Constrained { report: rep, multipliers, residual }));
            }
            if it == opt.max_iter {
                break;
            }
            // Projected CG on P H P p = -P g from p = 0.
            let mut p = vec![0.0; n];
            let mut r: Vec<f64> = pg.iter().map(|v| -v).collect();
            let mut d = r.clone();
            let mut rr = dot(&r, &r);
            let rn = rr.sqrt();
            let stop = rn.sqrt().min(0.5) * rn;
            for k in 0..opt.max_cg {
                env.insert(vname.clone(), Tensor { shape: x.shape.clone(), data: d.clone() });
                let mut hd = eval(&self.hessian_vec, &env)?.remove(0).data;
                con.project(&gram, &mut hd)?;
                rep.evals += 1;
                rep.cg_steps += 1;
                let curv = dot(&d, &hd);
                if curv <= 0.0 {
                    if k == 0 {
                        p = r.clone();
                    }
                    break;
                }
                let alpha = rr / curv;
                for ((pi, ri), (di, hi)) in p.iter_mut().zip(r.iter_mut()).zip(d.iter().zip(&hd)) {
                    *pi += alpha * di;
                    *ri -= alpha * hi;
                }
                let rr2 = dot(&r, &r);
                if rr2.sqrt() < stop {
                    break;
                }
                let beta = rr2 / rr;
                for (di, ri) in d.iter_mut().zip(&r) {
                    *di = ri + beta * *di;
                }
                rr = rr2;
            }
            env.remove(&vname);
            let slope = dot(&g, &p);
            let mut step = 1.0;
            loop {
                let mut trial: Vec<f64> = x.data.iter().zip(&p).map(|(a, b)| a + step * b).collect();
                con.restore(&gram, &mut trial)?;
                let trial = Tensor { shape: x.shape.clone(), data: trial };
                env.insert(self.var.clone(), trial.clone());
                let ft = eval(&self.energy, &env)?[0].item();
                rep.evals += 1;
                let slack = 16.0 * f64::EPSILON * (f.abs() + ft.abs());
                if ft <= f + 1e-4 * step * slope + slack || step < 1e-12 {
                    x = trial;
                    break;
                }
                // Near the solution the decrease can be below the energy's
                // own rounding (terms that cancel, as an axial strain
                // `l - l0`), which the slack above does not see: accept the
                // full step when it halves the projected gradient.
                if step == 1.0 {
                    let out = eval(&self.gradient, &env)?;
                    rep.evals += 1;
                    let mut tg = out[1].data.clone();
                    con.project(&gram, &mut tg)?;
                    if out[0].item().is_finite() && tg.iter().all(|v| v.is_finite()) && max_abs(&tg) <= 0.5 * gn {
                        x = trial;
                        break;
                    }
                }
                step *= 0.5;
            }
        }
        Err(Error::NoConvergence(format!("{} iterations, |Pg| {:.3e}", opt.max_iter, rep.grad_norm)))
    }
}

#[derive(Clone, Debug)]
pub struct PathOptions {
    /// First arc-length step, and its bounds.
    pub ds: f64,
    pub ds_min: f64,
    pub ds_max: f64,
    /// Weight of the load in the arc length: `|dx|^2 + psi^2 dl^2 = ds^2`.
    pub psi: f64,
    /// Equilibrium and arc-length residuals below this.
    pub tol: f64,
    /// Corrector iterations before the step is halved.
    pub max_iter: usize,
    /// Corrector iterations aimed for: fewer grows the step, more shrinks it.
    pub target_iter: usize,
    pub max_steps: usize,
    /// A corrected point farther than this many step lengths from the
    /// last one (in the arc-length norm) is a jump to another branch: the
    /// step is halved instead.
    pub max_jump: f64,
}

impl Default for PathOptions {
    fn default() -> Self {
        PathOptions {
            ds: 0.05,
            ds_min: 1e-8,
            ds_max: 1.0,
            psi: 1.0,
            tol: 1e-10,
            max_iter: 12,
            target_iter: 4,
            max_steps: 1000,
            max_jump: 1.5,
        }
    }
}

/// A converged point on an equilibrium path.
#[derive(Clone, Debug)]
pub struct PathPoint {
    pub x: Vec<f64>,
    pub load: f64,
    /// Corrector iterations at this point (0 for the start if it was one).
    pub iterations: usize,
    /// Steps tried and halved before this one converged.
    pub retries: usize,
    /// The largest equilibrium residual `|dE/dx|` here.
    pub residual: f64,
    /// `|N|`, the arc-length constraint's residual.
    pub arc_residual: f64,
    pub ds: f64,
    /// Sign of `det K`: it changes where the path crosses a limit or
    /// bifurcation point.
    pub det_sign: f64,
    /// The unit tangent's load component `dl/ds`: it changes sign at a
    /// limit point.
    pub dload: f64,
    /// The unit tangent's `x` part.
    pub tangent: Vec<f64>,
    pub evals: usize,
}

/// An energy `E(x, load)` whose equilibria `dE/dx = 0` are followed as
/// the scalar input `load` varies.
pub struct Continuation {
    pub gradient: Graph,
    /// Hessian-vector products in `(x, load)`: inputs `v:<var>` and
    /// `v:<load>`; output 0 is `K v_x + q v_l`, with `q = d(dE/dx)/dload`.
    pub second: Graph,
    pub var: String,
    pub load: String,
    shape: Vec<usize>,
}

impl Continuation {
    /// `energy` returns a scalar at `of`; `var` is varied, `load` is a
    /// scalar input.
    pub fn new(
        energy: &Graph,
        of: V,
        var: &str,
        load: &str,
        shapes: &BTreeMap<String, Vec<usize>>,
    ) -> Result<Continuation, Error> {
        if shapes.get(load).is_none_or(|s| !s.is_empty()) {
            return Err(Error::Shape(format!("{load} must be a scalar input")));
        }
        let shape = shapes.get(var).ok_or_else(|| Error::MissingInput(var.into()))?.clone();
        Ok(Continuation {
            gradient: grad(energy, shapes, of, &[var])?,
            second: hvp(energy, shapes, of, &[var, load])?,
            var: var.into(),
            load: load.into(),
            shape,
        })
    }

    fn bind(&self, env: &mut Env, x: &[f64], l: f64) {
        env.insert(self.var.clone(), Tensor { shape: self.shape.clone(), data: x.to_vec() });
        env.insert(self.load.clone(), Tensor::scalar(l));
    }

    /// `dE/dx` at `(x, load)`.
    pub fn residual(&self, env: &Env, x: &[f64], l: f64) -> Result<Vec<f64>, Error> {
        let mut env = env.clone();
        self.bind(&mut env, x, l);
        let out = eval(&self.gradient, &env)?;
        let g = out[1].data.clone();
        finite(out[0].item(), &g)?;
        Ok(g)
    }

    /// The stiffness `K` (dense, row-major, from `n` Hessian-vector
    /// products) and the load vector `q`.
    fn stiffness(&self, env: &Env, x: &[f64], l: f64, evals: &mut usize) -> Result<(Vec<f64>, Vec<f64>), Error> {
        let n = x.len();
        let mut env = env.clone();
        self.bind(&mut env, x, l);
        let vx = format!("v:{}", self.var);
        let vl = format!("v:{}", self.load);
        let mut k = vec![0.0; n * n];
        let mut e = vec![0.0; n];
        env.insert(vl.clone(), Tensor::scalar(0.0));
        for j in 0..n {
            e.fill(0.0);
            e[j] = 1.0;
            env.insert(vx.clone(), Tensor { shape: self.shape.clone(), data: e.clone() });
            let col = eval(&self.second, &env)?.remove(0).data;
            *evals += 1;
            for i in 0..n {
                k[i * n + j] = col[i];
            }
        }
        // Symmetrize the rounding.
        for i in 0..n {
            for j in i + 1..n {
                let s = 0.5 * (k[i * n + j] + k[j * n + i]);
                k[i * n + j] = s;
                k[j * n + i] = s;
            }
        }
        env.insert(vx, Tensor::zeros(&self.shape));
        env.insert(vl, Tensor::scalar(1.0));
        let q = eval(&self.second, &env)?.remove(0).data;
        *evals += 1;
        Ok((k, q))
    }

    /// Solve `[K q; c^T cl] [dx; dl] = [rx; rn]`.
    fn bordered(k: &[f64], q: &[f64], c: &[f64], cl: f64, rx: &[f64], rn: f64) -> Result<(Vec<f64>, f64), Error> {
        let n = q.len();
        let m = n + 1;
        let mut a = vec![0.0; m * m];
        for i in 0..n {
            a[i * m..i * m + n].copy_from_slice(&k[i * n..i * n + n]);
            a[i * m + n] = q[i];
            a[n * m + i] = c[i];
        }
        a[n * m + n] = cl;
        let mut b = rx.to_vec();
        b.push(rn);
        lu_solve(&mut a, m, &mut b)?;
        let dl = b.pop().unwrap();
        Ok((b, dl))
    }

    /// The unit tangent at a point with stiffness `k` and load vector `q`,
    /// oriented along the previous tangent `(tx, tl)`.
    fn tangent(k: &[f64], q: &[f64], tx: &[f64], tl: f64, psi2: f64) -> Result<(Vec<f64>, f64), Error> {
        let zeros = vec![0.0; q.len()];
        let (nx, nl) = Self::bordered(k, q, tx, psi2 * tl, &zeros, 1.0)?;
        Ok(normalize(nx, nl, psi2))
    }

    /// The limit point (where `dl/ds = 0`) between consecutive path
    /// points `a` and `b` whose `dload` differ in sign, by bisection on
    /// the arc length from `a`: `iters` corrector solves.
    pub fn locate_limit(
        &self,
        env: &Env,
        a: &PathPoint,
        b: &PathPoint,
        opt: &PathOptions,
        iters: usize,
    ) -> Result<PathPoint, Error> {
        if a.dload.signum() == b.dload.signum() {
            return Err(Error::NoConvergence("no change of load direction between these points".into()));
        }
        let psi2 = opt.psi * opt.psi;
        let n = a.x.len();
        let (mut lo, mut hi) = (0.0, b.ds);
        let mut best = None;
        for _ in 0..iters {
            let s = 0.5 * (lo + hi);
            let mut evals = 0;
            let (x, l, it, rn, an, (k, q)) =
                self.correct(env, &a.x, a.load, &a.tangent, a.dload, s, psi2, opt, &mut evals)?;
            let (tx, tl) = Self::tangent(&k, &q, &a.tangent, a.dload, psi2)?;
            if tl.signum() == a.dload.signum() {
                lo = s;
            } else {
                hi = s;
            }
            best = Some(PathPoint {
                x,
                load: l,
                iterations: it,
                retries: 0,
                residual: rn,
                arc_residual: an,
                ds: s,
                det_sign: Self::det_sign(&k, n),
                dload: tl,
                tangent: tx,
                evals,
            });
        }
        best.ok_or_else(|| Error::NoConvergence("no bisection steps".into()))
    }

    fn det_sign(k: &[f64], n: usize) -> f64 {
        let mut a = k.to_vec();
        let mut b = vec![0.0; n];
        lu_solve(&mut a, n, &mut b).unwrap_or(0.0)
    }

    /// Follow the path from `(x0, load0)`, which is first corrected to an
    /// equilibrium at fixed load, with the load increasing at the start.
    /// Stops after `opt.max_steps` points or when `stop` says so; a step
    /// that does not converge is halved down to `opt.ds_min`, then the
    /// path so far is returned with an error.
    pub fn follow(
        &self,
        env: &Env,
        x0: &Tensor,
        load0: f64,
        opt: &PathOptions,
        mut stop: impl FnMut(&PathPoint) -> bool,
    ) -> Result<Vec<PathPoint>, Error> {
        let n = x0.data.len();
        let psi2 = opt.psi * opt.psi;
        let mut evals = 0;
        // Correct the start at fixed load.
        let mut x = x0.data.clone();
        let l = load0;
        let mut iters = 0;
        loop {
            let r = self.residual(env, &x, l)?;
            evals += 1;
            if max_abs(&r) < opt.tol {
                break;
            }
            if iters == opt.max_iter * 4 {
                return Err(Error::NoConvergence("the start is not an equilibrium".into()));
            }
            let (k, _) = self.stiffness(env, &x, l, &mut evals)?;
            let mut a = k.clone();
            let mut b: Vec<f64> = r.iter().map(|v| -v).collect();
            lu_solve(&mut a, n, &mut b)?;
            for (xi, bi) in x.iter_mut().zip(&b) {
                *xi += bi;
            }
            iters += 1;
        }
        let (k, q) = self.stiffness(env, &x, l, &mut evals)?;
        // Tangent: [K q; 0 1] t = [0; 1], then normalized.
        let zeros = vec![0.0; n];
        let (tx, tl) = Self::bordered(&k, &q, &zeros, 1.0, &zeros, 1.0)?;
        let (mut tx, mut tl) = normalize(tx, tl, psi2);
        let r = self.residual(env, &x, l)?;
        let mut path = vec![PathPoint {
            x: x.clone(),
            load: l,
            iterations: iters,
            retries: 0,
            residual: max_abs(&r),
            arc_residual: 0.0,
            ds: 0.0,
            det_sign: Self::det_sign(&k, n),
            dload: tl,
            tangent: tx.clone(),
            evals,
        }];
        if stop(&path[0]) {
            return Ok(path);
        }
        let mut ds = opt.ds;
        let (mut x0, mut l0) = (x, l);
        while path.len() <= opt.max_steps {
            let mut retries = 0;
            let (x1, l1, it, rn, an, kq) = loop {
                evals = 0;
                match self.correct(env, &x0, l0, &tx, tl, ds, psi2, opt, &mut evals) {
                    Ok(v) => break v,
                    Err(e) => {
                        ds *= 0.5;
                        retries += 1;
                        if ds < opt.ds_min {
                            return Err(Error::NoConvergence(format!(
                                "arc-length step below {:.1e} after {} points: {e}",
                                opt.ds_min,
                                path.len()
                            )));
                        }
                    }
                }
            };
            let (k, q) = kq;
            (tx, tl) = Self::tangent(&k, &q, &tx, tl, psi2)?;
            let p = PathPoint {
                x: x1.clone(),
                load: l1,
                iterations: it,
                retries,
                residual: rn,
                arc_residual: an,
                ds,
                det_sign: Self::det_sign(&k, n),
                dload: tl,
                tangent: tx.clone(),
                evals,
            };
            let done = stop(&p);
            path.push(p);
            if done {
                break;
            }
            let grow = (opt.target_iter.max(1) as f64 / it.max(1) as f64).sqrt().clamp(0.5, 2.0);
            ds = (ds * grow).clamp(opt.ds_min, opt.ds_max);
            (x0, l0) = (x1, l1);
        }
        Ok(path)
    }

    /// One predictor-corrector step of length `ds` from `(x0, l0)` along
    /// `(tx, tl)`.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn correct(
        &self,
        env: &Env,
        x0: &[f64],
        l0: f64,
        tx: &[f64],
        tl: f64,
        ds: f64,
        psi2: f64,
        opt: &PathOptions,
        evals: &mut usize,
    ) -> Result<(Vec<f64>, f64, usize, f64, f64, (Vec<f64>, Vec<f64>)), Error> {
        let mut x: Vec<f64> = x0.iter().zip(tx).map(|(a, t)| a + ds * t).collect();
        let mut l = l0 + ds * tl;
        for it in 1..=opt.max_iter {
            let (k, q) = self.stiffness(env, &x, l, evals)?;
            let r = self.residual(env, &x, l)?;
            *evals += 1;
            let dxs: Vec<f64> = x.iter().zip(x0).map(|(a, b)| a - b).collect();
            let nres = dot(tx, &dxs) + psi2 * tl * (l - l0) - ds;
            let rx: Vec<f64> = r.iter().map(|v| -v).collect();
            let (dx, dl) = Self::bordered(&k, &q, tx, psi2 * tl, &rx, -nres)?;
            for (xi, d) in x.iter_mut().zip(&dx) {
                *xi += d;
            }
            l += dl;
            let r = self.residual(env, &x, l)?;
            *evals += 1;
            let dxs: Vec<f64> = x.iter().zip(x0).map(|(a, b)| a - b).collect();
            let nres = dot(tx, &dxs) + psi2 * tl * (l - l0) - ds;
            let rn = max_abs(&r);
            if rn < opt.tol && nres.abs() < opt.tol.max(1e-14 * ds) {
                let dist = (dot(&dxs, &dxs) + psi2 * (l - l0) * (l - l0)).sqrt();
                if dist > opt.max_jump * ds {
                    return Err(Error::NoConvergence(format!("jumped {dist:.3e} for a step of {ds:.3e}")));
                }
                let kq = self.stiffness(env, &x, l, evals)?;
                return Ok((x, l, it, rn, nres.abs(), kq));
            }
        }
        Err(Error::NoConvergence(format!("corrector: {} iterations", opt.max_iter)))
    }
}

fn normalize(tx: Vec<f64>, tl: f64, psi2: f64) -> (Vec<f64>, f64) {
    let norm = (dot(&tx, &tx) + psi2 * tl * tl).sqrt();
    (tx.into_iter().map(|v| v / norm).collect(), tl / norm)
}
