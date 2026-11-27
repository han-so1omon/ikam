//! Minimization with Newton-CG over op graphs: the gradient and the
//! Hessian-vector product are graphs derived from the energy's graph
//! (`grad`, `hvp`), so a nonlinear static problem (a rod's equilibrium, a
//! section's strain state) is an energy written once as a graph.
//!
//! Each iteration solves `H p = -g` by conjugate gradients, stopping at
//! negative curvature (truncated Newton, Nocedal and Wright ch. 7), then
//! backtracks along `p` until the energy falls enough (Armijo).

use std::collections::BTreeMap;

use crate::eval::{Arith, Env, dot_host, eval, eval_with};
use crate::grad::{grad, hvp};
use crate::ir::{Graph, V};
use crate::plan::Plan;
use crate::{Error, Tensor};

#[derive(Clone, Debug)]
pub struct Options {
    pub tol: f64,
    pub max_iter: usize,
    pub max_cg: usize,
    /// Return the iterate after `max_iter` steps instead of an error: a
    /// step budget, as in a control tick that warm-starts from the last.
    pub budget: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { tol: 1e-10, max_iter: 100, max_cg: 1000, budget: false }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub iterations: usize,
    pub cg_steps: usize,
    pub evals: usize,
    pub energy: f64,
    pub grad_norm: f64,
}

/// The energy's derived graphs, built once and reused for every solve.
pub struct Minimizer {
    pub energy: Graph,
    pub gradient: Graph,
    pub hessian_vec: Graph,
    pub var: String,
}

/// The largest gradient component, or an error when the energy or any
/// component is not finite: `f64::max` skips a NaN, so a NaN gradient
/// would otherwise fold to 0 and read as converged.
fn grad_norm(f: f64, g: &[f64]) -> Result<f64, Error> {
    if !f.is_finite() || g.iter().any(|v| !v.is_finite()) {
        return Err(Error::NoConvergence("the energy or its gradient is not finite here".into()));
    }
    Ok(g.iter().fold(0.0f64, |m, v| m.max(v.abs())))
}

impl Minimizer {
    /// `energy` returns a scalar at node `of`; `var` is the input varied.
    pub fn new(energy: &Graph, of: V, var: &str, shapes: &BTreeMap<String, Vec<usize>>) -> Result<Minimizer, Error> {
        let mut e = energy.clone();
        e.outputs = vec![of];
        Ok(Minimizer {
            gradient: grad(energy, shapes, of, &[var])?,
            hessian_vec: hvp(energy, shapes, of, &[var])?,
            energy: e,
            var: var.to_string(),
        })
    }

    pub fn minimize(&self, env: &Env, x0: Tensor, opt: &Options) -> Result<(Tensor, Report), Error> {
        self.minimize_with(env, x0, opt, Arith::Pinned)
    }

    /// `minimize` with another arithmetic for the graphs and for the
    /// solver's own dot products and updates (E442: with `Arith::Host`
    /// they are fused and the dot products summed in 8 lanes, as another
    /// build would compute them).
    pub fn minimize_with(&self, env: &Env, x0: Tensor, opt: &Options, arith: Arith) -> Result<(Tensor, Report), Error> {
        let dot = |a: &[f64], b: &[f64]| dot_with(a, b, arith);
        let axpy = |x: &[f64], a: f64, y: &[f64]| axpy_with(x, a, y, arith);
        let mut env = env.clone();
        let mut x = x0;
        let mut rep = Report::default();
        for it in 0..=opt.max_iter {
            env.insert(self.var.clone(), x.clone());
            let out = eval_with(&self.gradient, &env, arith)?;
            rep.evals += 1;
            let (f, g) = (out[0].item(), out[1].clone());
            let gn = grad_norm(f, &g.data)?;
            rep.iterations = it;
            rep.energy = f;
            rep.grad_norm = gn;
            if gn < opt.tol || (opt.budget && it == opt.max_iter) {
                return Ok((x, rep));
            }
            if it == opt.max_iter {
                break;
            }
            let p = self.cg(&env, &g, opt, &mut rep, arith)?;
            let slope = dot(&g.data, &p);
            let mut step = 1.0;
            loop {
                let trial = Tensor { shape: x.shape.clone(), data: axpy(&x.data, step, &p) };
                env.insert(self.var.clone(), trial.clone());
                let ft = eval_with(&self.energy, &env, arith)?[0].item();
                rep.evals += 1;
                // Near a minimum the predicted decrease is below the
                // energy's rounding, so allow that much slack; otherwise a
                // good Newton step is halved away and the solve stalls.
                let slack = 16.0 * f64::EPSILON * (f.abs() + ft.abs());
                if ft <= f + 1e-4 * step * slope + slack || step < 1e-12 {
                    x = trial;
                    break;
                }
                step *= 0.5;
            }
        }
        Err(Error::NoConvergence(format!("{} iterations, |g| {:.3e}", opt.max_iter, rep.grad_norm)))
    }

    /// Solve H p = -g by conjugate gradients from p = 0.
    fn cg(&self, env: &Env, g: &Tensor, opt: &Options, rep: &mut Report, arith: Arith) -> Result<Vec<f64>, Error> {
        let dot = |a: &[f64], b: &[f64]| dot_with(a, b, arith);
        let axpy = |x: &[f64], a: f64, y: &[f64]| axpy_with(x, a, y, arith);
        let n = g.data.len();
        let mut p = vec![0.0; n];
        let mut r: Vec<f64> = g.data.iter().map(|v| -v).collect();
        let mut d = r.clone();
        let mut rr = dot(&r, &r);
        let gn = rr.sqrt();
        let stop = gn.sqrt().min(0.5) * gn;
        let mut env = env.clone();
        for k in 0..opt.max_cg {
            env.insert(format!("v:{}", self.var), Tensor { shape: g.shape.clone(), data: d.clone() });
            let hd = eval_with(&self.hessian_vec, &env, arith)?.remove(0).data;
            rep.evals += 1;
            rep.cg_steps += 1;
            let curv = dot(&d, &hd);
            if curv <= 0.0 {
                // Negative curvature: the steepest descent direction on the
                // first step, else what was found so far.
                return Ok(if k == 0 { r } else { p });
            }
            let alpha = rr / curv;
            p = axpy(&p, alpha, &d);
            r = axpy(&r, -alpha, &hd);
            let rr2 = dot(&r, &r);
            if rr2.sqrt() < stop {
                break;
            }
            d = axpy(&r, rr2 / rr, &d);
            rr = rr2;
        }
        Ok(p)
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn axpy(x: &[f64], a: f64, y: &[f64]) -> Vec<f64> {
    x.iter().zip(y).map(|(x, y)| x + a * y).collect()
}

fn dot_with(a: &[f64], b: &[f64], arith: Arith) -> f64 {
    match arith {
        Arith::Pinned => dot(a, b),
        Arith::Host => dot_host(a, b),
    }
}

fn axpy_with(x: &[f64], a: f64, y: &[f64], arith: Arith) -> Vec<f64> {
    match arith {
        Arith::Pinned => axpy(x, a, y),
        Arith::Host => x.iter().zip(y).map(|(x, y)| a.mul_add(*y, *x)).collect(),
    }
}

/// The same Newton-CG over planned programs, for solves inside a loop that
/// keeps a period (a quasi-static model stepped every control tick): the
/// energy, gradient and Hessian-vector graphs are lowered once, every work
/// vector is allocated once, and a solve allocates nothing. It takes the
/// same steps as `Minimizer::minimize` with the same bits.
pub struct PlannedMinimizer {
    energy: Plan,
    gradient: Plan,
    hessian_vec: Plan,
    var: String,
    vname: String,
    g: Vec<f64>,
    p: Vec<f64>,
    r: Vec<f64>,
    d: Vec<f64>,
    trial: Vec<f64>,
}

impl Minimizer {
    pub fn plan(&self, shapes: &BTreeMap<String, Vec<usize>>) -> Result<PlannedMinimizer, Error> {
        let n: usize = shapes.get(&self.var).ok_or_else(|| Error::MissingInput(self.var.clone()))?.iter().product();
        let vname = format!("v:{}", self.var);
        let mut s2 = shapes.clone();
        s2.insert(vname.clone(), shapes[&self.var].clone());
        Ok(PlannedMinimizer {
            energy: Plan::new(&self.energy, shapes)?,
            gradient: Plan::new(&self.gradient, shapes)?,
            hessian_vec: Plan::new(&self.hessian_vec, &s2)?,
            var: self.var.clone(),
            vname,
            g: vec![0.0; n],
            p: vec![0.0; n],
            r: vec![0.0; n],
            d: vec![0.0; n],
            trial: vec![0.0; n],
        })
    }
}

impl PlannedMinimizer {
    /// Set a parameter input (a load, a tension) in every program.
    pub fn set(&mut self, name: &str, values: &[f64]) -> Result<(), Error> {
        self.energy.set(name, values)?;
        self.gradient.set(name, values)?;
        self.hessian_vec.set(name, values)
    }

    /// Minimize from `x` in place, at most `opt.max_iter` Newton steps
    /// (with `opt.budget`, a tick takes one or two from the last tick's
    /// solution and returns).
    pub fn minimize(&mut self, x: &mut [f64], opt: &Options) -> Result<Report, Error> {
        let mut rep = Report::default();
        for it in 0..=opt.max_iter {
            self.gradient.set(&self.var, x)?;
            self.gradient.run();
            rep.evals += 1;
            let f = self.gradient.output(0)[0];
            self.g.copy_from_slice(self.gradient.output(1));
            let gn = grad_norm(f, &self.g)?;
            rep.iterations = it;
            rep.energy = f;
            rep.grad_norm = gn;
            if gn < opt.tol || (opt.budget && it == opt.max_iter) {
                return Ok(rep);
            }
            if it == opt.max_iter {
                break;
            }
            self.hessian_vec.set(&self.var, x)?;
            self.cg(opt, &mut rep)?;
            let slope = dot(&self.g, &self.p);
            let mut step = 1.0;
            loop {
                for ((t, xi), pi) in self.trial.iter_mut().zip(x.iter()).zip(&self.p) {
                    *t = xi + step * pi;
                }
                self.energy.set(&self.var, &self.trial)?;
                self.energy.run();
                rep.evals += 1;
                let ft = self.energy.output(0)[0];
                let slack = 16.0 * f64::EPSILON * (f.abs() + ft.abs());
                if ft <= f + 1e-4 * step * slope + slack || step < 1e-12 {
                    x.copy_from_slice(&self.trial);
                    break;
                }
                step *= 0.5;
            }
        }
        Err(Error::NoConvergence(format!("{} iterations, |g| {:.3e}", opt.max_iter, rep.grad_norm)))
    }

    fn cg(&mut self, opt: &Options, rep: &mut Report) -> Result<(), Error> {
        self.p.fill(0.0);
        for (r, g) in self.r.iter_mut().zip(&self.g) {
            *r = -g;
        }
        self.d.copy_from_slice(&self.r);
        let mut rr = dot(&self.r, &self.r);
        let gn = rr.sqrt();
        let stop = gn.sqrt().min(0.5) * gn;
        for k in 0..opt.max_cg {
            self.hessian_vec.set(&self.vname, &self.d)?;
            self.hessian_vec.run();
            rep.evals += 1;
            rep.cg_steps += 1;
            let hd = self.hessian_vec.output(0);
            let curv = dot(&self.d, hd);
            if curv <= 0.0 {
                if k == 0 {
                    self.p.copy_from_slice(&self.r);
                }
                return Ok(());
            }
            let alpha = rr / curv;
            for ((p, r), (d, h)) in self.p.iter_mut().zip(self.r.iter_mut()).zip(self.d.iter().zip(hd)) {
                *p += alpha * d;
                *r -= alpha * h;
            }
            let rr2 = dot(&self.r, &self.r);
            if rr2.sqrt() < stop {
                break;
            }
            let beta = rr2 / rr;
            for (d, r) in self.d.iter_mut().zip(&self.r) {
                *d = r + beta * *d;
            }
            rr = rr2;
        }
        Ok(())
    }
}

impl Minimizer {
    /// Derivatives of an objective at an equilibrium with respect to
    /// parameters, by the adjoint of the equilibrium (implicit
    /// differentiation): at a minimum x*(p) of E(x, p), for J(x, p),
    /// dJ/dp = dJ/dp - lambda . d(dE/dx)/dp with H lambda = dJ/dx. One
    /// Hessian solve gives the derivative for every parameter, which is
    /// what inverse kinematics, design optimization and identification
    /// need. `shapes` covers the energy's and the objective's inputs.
    pub fn sensitivity(
        &self,
        shapes: &BTreeMap<String, Vec<usize>>,
        env: &Env,
        x: &Tensor,
        objective: (&Graph, V),
        params: &[&str],
        opt: &Options,
    ) -> Result<(f64, Vec<Tensor>), Error> {
        let (objective, of) = objective;
        let mut env = env.clone();
        env.insert(self.var.clone(), x.clone());
        let zeros = |name: &str| Tensor::zeros(&shapes[name]);
        // dJ/dx and the direct dJ/dp.
        let present = objective.input_names();
        let wrt: Vec<&str> = std::iter::once(self.var.as_str())
            .chain(params.iter().copied())
            .filter(|p| present.iter().any(|n| n == p))
            .collect();
        let jg = grad(objective, shapes, of, &wrt)?;
        let jo = eval(&jg, &env)?;
        let value = jo[0].item();
        let by_name: BTreeMap<&str, &Tensor> = wrt.iter().copied().zip(jo[1..].iter()).collect();
        let jx = by_name.get(self.var.as_str()).map(|t| (*t).clone()).unwrap_or_else(|| zeros(&self.var));
        // H lambda = dJ/dx.
        let lam = self.solve_h(&env, &jx, opt)?;
        // d(lambda . dE/dx)/dp.
        let mut mg = self.gradient.clone();
        let lname = format!("lam:{}", self.var);
        let l = mg.input(&lname);
        let gx = mg.outputs[1];
        let prod = mg.mul(gx, l);
        let s = mg.sum(prod);
        let mut s2 = shapes.clone();
        s2.insert(lname.clone(), shapes[&self.var].clone());
        let epresent = mg.input_names();
        let mp: Vec<&str> = params.iter().copied().filter(|p| epresent.iter().any(|n| n == p)).collect();
        let mut mixed: BTreeMap<&str, Tensor> = BTreeMap::new();
        if !mp.is_empty() {
            let mgg = grad(&mg, &s2, s, &mp)?;
            env.insert(lname, lam);
            let out = eval(&mgg, &env)?;
            for (p, t) in mp.iter().zip(out.into_iter().skip(1)) {
                mixed.insert(p, t);
            }
        }
        let mut res = Vec::new();
        for p in params {
            let mut d = by_name.get(p).map(|t| (*t).clone()).unwrap_or_else(|| zeros(p));
            if let Some(m) = mixed.get(p) {
                for (a, b) in d.data.iter_mut().zip(&m.data) {
                    *a -= b;
                }
            }
            res.push(d);
        }
        Ok((value, res))
    }

    /// Solve H y = b by conjugate gradients to a relative residual of 1e-12.
    fn solve_h(&self, env: &Env, b: &Tensor, opt: &Options) -> Result<Tensor, Error> {
        let n = b.data.len();
        let mut env = env.clone();
        let mut y = vec![0.0; n];
        let mut r = b.data.clone();
        let mut d = r.clone();
        let mut rr = dot(&r, &r);
        let stop = 1e-12 * rr.sqrt();
        for _ in 0..opt.max_cg.max(2 * n) {
            if rr.sqrt() <= stop {
                break;
            }
            env.insert(format!("v:{}", self.var), Tensor { shape: b.shape.clone(), data: d.clone() });
            let hd = eval(&self.hessian_vec, &env)?.remove(0).data;
            let curv = dot(&d, &hd);
            if curv <= 0.0 {
                return Err(Error::NoConvergence("the Hessian is not positive definite here".into()));
            }
            let alpha = rr / curv;
            y = axpy(&y, alpha, &d);
            r = axpy(&r, -alpha, &hd);
            let rr2 = dot(&r, &r);
            d = axpy(&r, rr2 / rr, &d);
            rr = rr2;
        }
        Ok(Tensor { shape: b.shape.clone(), data: y })
    }
}
