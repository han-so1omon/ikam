//! Bounds on nondeterminism (E442), optional: an [`Op::Within`] node in a
//! graph declares how far the value at that point may move between runs
//! with other float arithmetic. The bound is part of the graph, stored and
//! versioned with it like any other node; `memo::check_bounded` and
//! `memo::check_graph_bounded` evaluate it when they check a rerun. A
//! graph without bounds is checked exactly, as before.
//!
//! [`predict`] gives, without a rerun, a bound on each node's rounding
//! error from a running error analysis, and [`fill_from_prediction`]
//! writes Within tolerances from it.

use std::collections::BTreeMap;

use crate::eval::{Env, eval_all};
use crate::ir::{Graph, Op, OpNode, V};
use crate::{Error, Tensor};

/// Units in the last place between two floats (E409's measure): 0 for
/// equal bits, 1 between neighbours (and between -0 and +0); NaN against
/// NaN is 0, NaN against a number `u64::MAX`.
pub fn ulp_distance(a: f64, b: f64) -> u64 {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => 0,
        (true, false) | (false, true) => u64::MAX,
        _ => {
            let key = |x: f64| {
                let b = x.to_bits();
                if b >> 63 == 1 { !b } else { b | 1 << 63 }
            };
            key(a).abs_diff(key(b))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The same bits as recorded.
    Same,
    /// Other bits, every element within the tolerance.
    Within,
    /// Some element outside the tolerance.
    Outside,
}

/// How far a value moved from its record, element by element.
#[derive(Clone, Debug, PartialEq)]
pub struct Distance {
    pub max_ulp: u64,
    pub max_abs: f64,
    /// `|r - x| / |r|`, 0 where both are 0, infinite where only `r` is.
    pub max_rel: f64,
    /// Elements whose bits differ.
    pub differing: usize,
}

/// The largest distances between a recorded value and a new one. A shape
/// change, a NaN against a number or two different infinities count as
/// infinitely far.
pub fn distance(recorded: &Tensor, now: &Tensor) -> Distance {
    let far = Distance { max_ulp: u64::MAX, max_abs: f64::INFINITY, max_rel: f64::INFINITY, differing: 1 };
    if recorded.shape != now.shape || recorded.data.len() != now.data.len() {
        return far;
    }
    let mut d = Distance { max_ulp: 0, max_abs: 0.0, max_rel: 0.0, differing: 0 };
    for (r, x) in recorded.data.iter().zip(&now.data) {
        let (r, x) = (*r, *x);
        if r.to_bits() == x.to_bits() || (r.is_nan() && x.is_nan()) {
            continue;
        }
        d.differing += 1;
        d.max_ulp = d.max_ulp.max(ulp_distance(r, x));
        if !r.is_finite() || !x.is_finite() {
            // Only equal bits (above) match a special value; -0 against
            // +0 is finite.
            d.max_abs = f64::INFINITY;
            d.max_rel = f64::INFINITY;
            continue;
        }
        let e = (r - x).abs();
        d.max_abs = d.max_abs.max(e);
        let rel = if r == 0.0 { if e == 0.0 { 0.0 } else { f64::INFINITY } } else { e / r.abs() };
        d.max_rel = d.max_rel.max(rel);
    }
    d
}

/// Whether every element pair is within `Op::Within`'s tolerance.
pub fn within(recorded: &Tensor, now: &Tensor, ulp: u64, abs: f64, rel: f64) -> bool {
    if recorded.shape != now.shape || recorded.data.len() != now.data.len() {
        return false;
    }
    recorded.data.iter().zip(&now.data).all(|(r, x)| {
        let (r, x) = (*r, *x);
        if r.to_bits() == x.to_bits() || (r.is_nan() && x.is_nan()) {
            return true;
        }
        if r.is_nan() || x.is_nan() || r.is_infinite() || x.is_infinite() {
            return false;
        }
        let e = (r - x).abs();
        ulp_distance(r, x) <= ulp || e <= abs || e <= rel * r.abs()
    })
}

/// One bound's outcome in a check.
#[derive(Clone, Debug, PartialEq)]
pub struct Bound {
    /// The Within node.
    pub node: V,
    pub verdict: Verdict,
    pub distance: Distance,
}

/// What a bounded check found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// Every Within node the check reached, in graph order.
    pub bounds: Vec<Bound>,
    /// Nodes (Within nodes included) whose bits differ from the record.
    pub differing: usize,
    /// Nodes compared.
    pub compared: usize,
}

impl Report {
    /// No bound outside its tolerance.
    pub fn passed(&self) -> bool {
        self.bounds.iter().all(|b| b.verdict != Verdict::Outside)
    }

    pub fn outside(&self) -> usize {
        self.bounds.iter().filter(|b| b.verdict == Verdict::Outside).count()
    }

    pub fn same(&self) -> bool {
        self.differing == 0
    }

    pub(crate) fn judge(&mut self, node: V, op: &Op, recorded: &Tensor, now: &Tensor) {
        let Op::Within { ulp, abs, rel, .. } = op else {
            return;
        };
        let distance = distance(recorded, now);
        let verdict = if distance.differing == 0 {
            Verdict::Same
        } else if within(recorded, now, *ulp, *abs, *rel) {
            Verdict::Within
        } else {
            Verdict::Outside
        };
        self.bounds.push(Bound { node, verdict, distance });
    }
}

/// Which nodes a difference is allowed at: a Within node, or a node whose
/// every path to a graph output passes through a Within node. A graph
/// output that is not a Within node, and a node no output depends on,
/// are not covered.
pub fn coverage(g: &Graph) -> Vec<bool> {
    let n = g.nodes.len();
    let mut consumers = vec![0usize; n];
    let mut is_output = vec![false; n];
    for o in &g.outputs {
        is_output[*o] = true;
    }
    for node in &g.nodes {
        for a in &node.args {
            consumers[*a] += 1;
        }
    }
    let mut all_covered = vec![true; n];
    let mut covered = vec![false; n];
    for i in (0..n).rev() {
        covered[i] =
            matches!(g.nodes[i].op, Op::Within { .. }) || (!is_output[i] && consumers[i] > 0 && all_covered[i]);
        for a in &g.nodes[i].args {
            all_covered[*a] &= covered[i];
        }
    }
    covered
}

/// A copy of `g` with a Within node (`bound`, an `Op::Within`) placed
/// after each node in `at`; every use of such a node, and the outputs,
/// now go through its bound. Returns the graph and where each old node
/// went.
pub fn insert_within(g: &Graph, at: &[V], bound: &Op) -> Result<(Graph, Vec<V>), Error> {
    if !matches!(bound, Op::Within { .. }) {
        return Err(Error::Shape("insert_within takes an Op::Within".into()));
    }
    let mut h = Graph::new();
    let mut map: Vec<V> = Vec::with_capacity(g.nodes.len());
    let mut mark = vec![false; g.nodes.len()];
    for a in at {
        mark[*a] = true;
    }
    for (i, n) in g.nodes.iter().enumerate() {
        let args: Vec<V> = n.args.iter().map(|a| map[*a]).collect();
        let v = h.push(n.op.clone(), &args);
        map.push(if mark[i] { h.push(bound.clone(), &[v]) } else { v });
    }
    h.outputs = g.outputs.iter().map(|o| map[*o]).collect();
    Ok((h, map))
}

/// Half an ULP relative: one correctly rounded operation's error is at
/// most `U * |v|` (plus an underflow floor).
pub const U: f64 = f64::EPSILON / 2.0;
const TINY: f64 = 5e-324;

#[inline]
fn rnd(v: f64) -> f64 {
    U * v.abs() + TINY
}

/// One ULP relative, for the math library's functions (pinned `libm` and
/// a host library are each within one ULP).
#[inline]
fn rnd_lib(v: f64) -> f64 {
    2.0 * U * v.abs() + TINY
}

/// A running error analysis (Wilkinson's, first order): for every node, a
/// bound on each element's distance from the value exact arithmetic would
/// give, for any arithmetic that rounds each basic operation correctly
/// (the pinned rules and `Arith::Host` both). Each basic operation adds
/// at most half an ULP of its result to the errors it propagates (by the
/// absolute values of its partial derivatives), a math-library function
/// one ULP, and a sum or matrix product the larger of the sequential and
/// the 8-lane order's running bound. `input_err` bounds the inputs'
/// errors (absent: exact). A local Within node starts again from zero, as
/// a bounded check restarts there from the value of record; a carried one
/// passes its argument's bound on. Two runs differ by at most twice the
/// bound. A non-finite value is taken as reproduced when its arguments are
/// exact and as unbounded otherwise.
pub fn predict(g: &Graph, env: &Env, input_err: &BTreeMap<String, Tensor>) -> Result<Vec<Tensor>, Error> {
    let vals = eval_all(g, env)?;
    let mut errs: Vec<Tensor> = Vec::with_capacity(g.nodes.len());
    for (i, n) in g.nodes.iter().enumerate() {
        let v = &vals[i];
        let a = |k: usize| &vals[n.args[k]];
        let ea = |k: usize| &errs[n.args[k]];
        let zeros = || Tensor { shape: v.shape.clone(), data: vec![0.0; v.data.len()] };
        let mut e = match &n.op {
            Op::Input(name) => match input_err.get(name) {
                Some(t) if t.data.len() == v.data.len() => Tensor { shape: v.shape.clone(), data: t.data.clone() },
                Some(_) => return Err(Error::Shape(format!("input error for {name}"))),
                None => zeros(),
            },
            Op::Const(_) => zeros(),
            Op::Within { local: true, .. } => zeros(),
            Op::Within { local: false, .. } | Op::Reshape { .. } | Op::Neg => {
                Tensor { shape: v.shape.clone(), data: ea(0).data.clone() }
            }
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Max | Op::Gt => {
                let (x, y, ex, ey) = (a(0), a(1), ea(0), ea(1));
                let at = |t: &Tensor, j: usize| {
                    if t.data.len() == 1 { t.data[0] } else { t.data[j] }
                };
                let data = (0..v.data.len())
                    .map(|j| {
                        let (x, y, ex, ey, r) = (at(x, j), at(y, j), at(ex, j), at(ey, j), v.data[j]);
                        match &n.op {
                            Op::Add | Op::Sub => ex + ey + rnd(r),
                            Op::Mul => y.abs() * ex + x.abs() * ey + ex * ey + rnd(r),
                            Op::Div => {
                                if y.abs() > ey {
                                    (ex + r.abs() * ey) / (y.abs() - ey) + rnd(r)
                                } else {
                                    f64::INFINITY
                                }
                            }
                            Op::Max => ex.max(ey),
                            _ => {
                                if ex + ey > 0.0 && (x - y).abs() <= ex + ey {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                        }
                    })
                    .collect();
                Tensor { shape: v.shape.clone(), data }
            }
            Op::Exp | Op::Ln | Op::Sin | Op::Cos | Op::Tanh | Op::Sqrt => {
                let data = a(0)
                    .data
                    .iter()
                    .zip(&ea(0).data)
                    .zip(&v.data)
                    .map(|((x, ex), r)| {
                        let (x, ex, r) = (*x, *ex, *r);
                        match &n.op {
                            Op::Exp => r.abs() * ex.exp_m1() + rnd_lib(r),
                            Op::Ln => {
                                if x.abs() > ex {
                                    ex / (x.abs() - ex) + rnd_lib(r)
                                } else {
                                    f64::INFINITY
                                }
                            }
                            Op::Sin | Op::Cos | Op::Tanh => ex.min(2.0) + rnd_lib(r),
                            _ => {
                                let lo = (x - ex).max(0.0);
                                let prop = if lo > 0.0 { ex / (x.max(0.0).sqrt() + lo.sqrt()) } else { ex.sqrt() };
                                prop + rnd(r)
                            }
                        }
                    })
                    .collect();
                Tensor { shape: v.shape.clone(), data }
            }
            Op::Sum => Tensor::scalar(sum_bound(&a(0).data, &ea(0).data)),
            Op::Cumsum | Op::RevCumsum => {
                let (x, ex) = (&a(0).data, &ea(0).data);
                let mut data = vec![0.0; x.len()];
                let (mut s, mut es) = (0.0f64, 0.0f64);
                let order: Vec<usize> =
                    if matches!(n.op, Op::Cumsum) { (0..x.len()).collect() } else { (0..x.len()).rev().collect() };
                for j in order {
                    s += x[j];
                    es += ex[j] + rnd(s);
                    data[j] = es;
                }
                Tensor { shape: v.shape.clone(), data }
            }
            Op::Slice { start, len } => Tensor::vector(ea(0).data[*start..start + len].to_vec()),
            Op::Pad { start, total } => {
                let mut data = vec![0.0; *total];
                data[*start..start + ea(0).data.len()].copy_from_slice(&ea(0).data);
                Tensor::vector(data)
            }
            Op::Concat => Tensor::vector(n.args.iter().flat_map(|k| errs[*k].data.iter().copied()).collect()),
            Op::Fill { shape } => Tensor { shape: shape.clone(), data: vec![ea(0).data[0]; v.data.len()] },
            Op::Transpose => {
                let (r, c) = (a(0).shape[0], a(0).shape[1]);
                let mut data = vec![0.0; r * c];
                for p in 0..r {
                    for q in 0..c {
                        data[q * r + p] = ea(0).data[p * c + q];
                    }
                }
                Tensor { shape: vec![c, r], data }
            }
            Op::MatMul => {
                let (x, y, ex, ey) = (a(0), a(1), ea(0), ea(1));
                let (rows, k, m) = (x.shape[0], x.shape[1], y.shape[1]);
                let mut data = vec![0.0; rows * m];
                for p in 0..rows {
                    for q in 0..m {
                        let terms: Vec<(f64, f64, f64, f64)> = (0..k)
                            .map(|t| (x.data[p * k + t], y.data[t * m + q], ex.data[p * k + t], ey.data[t * m + q]))
                            .collect();
                        data[p * m + q] = dot_bound(&terms);
                    }
                }
                Tensor { shape: vec![rows, m], data }
            }
        };
        for ej in e.data.iter_mut() {
            if ej.is_nan() {
                *ej = f64::INFINITY;
            }
        }
        if v.data.iter().any(|x| !x.is_finite()) {
            let exact = n.args.iter().all(|k| errs[*k].data.iter().all(|e| *e == 0.0));
            for (ej, vj) in e.data.iter_mut().zip(&v.data) {
                if !vj.is_finite() {
                    *ej = if exact { 0.0 } else { f64::INFINITY };
                }
            }
        }
        errs.push(e);
    }
    Ok(errs)
}

/// The running bound of a sum, the larger of index order and 8 lanes.
fn sum_bound(x: &[f64], ex: &[f64]) -> f64 {
    let (mut s, mut es) = (0.0f64, 0.0f64);
    for (v, e) in x.iter().zip(ex) {
        s += v;
        es += e + rnd(s);
    }
    let mut l = [(0.0f64, 0.0f64); 8];
    for (j, (v, e)) in x.iter().zip(ex).enumerate() {
        let (ls, le) = &mut l[j % 8];
        *ls += v;
        *le += e + rnd(*ls);
    }
    es.max(pairwise(&l))
}

fn pairwise(l: &[(f64, f64); 8]) -> f64 {
    let add = |a: (f64, f64), b: (f64, f64)| {
        let s = a.0 + b.0;
        (s, a.1 + b.1 + rnd(s))
    };
    let p = add(add(add(l[0], l[1]), add(l[2], l[3])), add(add(l[4], l[5]), add(l[6], l[7])));
    p.1
}

/// The running bound of a dot product (x, y, their errors), the larger of
/// index order with rounded products and the fused orders `dot_host`
/// uses (one chain, or 8 lanes from 16 terms).
fn dot_bound(terms: &[(f64, f64, f64, f64)]) -> f64 {
    let prop = |(x, y, ex, ey): (f64, f64, f64, f64)| y.abs() * ex + x.abs() * ey + ex * ey;
    let (mut s, mut es) = (0.0f64, 0.0f64);
    for t in terms {
        let p = t.0 * t.1;
        s += p;
        es += prop(*t) + rnd(p) + rnd(s);
    }
    if terms.len() < 16 {
        let (mut f, mut ef) = (0.0f64, 0.0f64);
        for t in terms {
            f = t.0.mul_add(t.1, f);
            ef += prop(*t) + rnd(f);
        }
        return es.max(ef);
    }
    let mut l = [(0.0f64, 0.0f64); 8];
    for (j, t) in terms.iter().enumerate() {
        let (ls, le) = &mut l[j % 8];
        *ls = t.0.mul_add(t.1, *ls);
        *le += prop(*t) + rnd(*ls);
    }
    es.max(pairwise(&l))
}

/// The largest element of each node's predicted bound on the difference
/// between two runs (twice `predict`'s bound on each run's error).
pub fn predicted_difference(errs: &[Tensor]) -> Vec<f64> {
    errs.iter().map(|e| 2.0 * e.data.iter().fold(0.0f64, |m, v| m.max(*v))).collect()
}

/// A copy of `g` whose Within nodes take their `abs` tolerance from the
/// prediction: `factor` times the predicted difference between two runs
/// at their argument (`ulp`, `rel` and `local` kept). The numbers are
/// written into the graph, so the stored graph carries the bound it was
/// checked with.
pub fn fill_from_prediction(
    g: &Graph,
    env: &Env,
    input_err: &BTreeMap<String, Tensor>,
    factor: f64,
) -> Result<Graph, Error> {
    let diff = predicted_difference(&predict(g, env, input_err)?);
    let mut h = g.clone();
    for (i, n) in h.nodes.iter_mut().enumerate() {
        if let Op::Within { ulp, rel, local, .. } = n.op {
            let abs = factor * diff[g.nodes[i].args[0]];
            *n = OpNode { op: Op::Within { ulp, abs, rel, local }, args: n.args.clone() };
        }
    }
    Ok(h)
}
