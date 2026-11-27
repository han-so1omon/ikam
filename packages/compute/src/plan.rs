//! A graph lowered to a fixed program for loops that must keep a period
//! (control, real-time simulation): shapes are fixed when the plan is
//! made, every node gets its buffer then, and a run only writes into those
//! buffers. Nothing is allocated, hashed or stored per run. Kernels use
//! the same pinned arithmetic as `eval`, so a plan's outputs have the same
//! bits as `eval`'s on the same inputs.

use std::collections::BTreeMap;

use crate::ir::{Graph, Op};
use crate::pinned::{self, canon};
use crate::{Error, Tensor};

pub struct Plan {
    ops: Vec<(Op, Vec<usize>)>,
    shapes: Vec<Vec<usize>>,
    bufs: Vec<Vec<f64>>,
    inputs: Vec<(String, usize)>,
    outputs: Vec<usize>,
}

impl Plan {
    pub fn new(g: &Graph, shapes: &BTreeMap<String, Vec<usize>>) -> Result<Plan, Error> {
        let sh = g.shapes(shapes)?;
        let mut bufs: Vec<Vec<f64>> = sh.iter().map(|s| vec![0.0; s.iter().product()]).collect();
        let mut inputs = Vec::new();
        for (i, n) in g.nodes.iter().enumerate() {
            match &n.op {
                Op::Const(t) => bufs[i].copy_from_slice(&t.data),
                Op::Input(name) => inputs.push((name.clone(), i)),
                _ => {}
            }
        }
        Ok(Plan {
            ops: g.nodes.iter().map(|n| (n.op.clone(), n.args.clone())).collect(),
            shapes: sh,
            bufs,
            inputs,
            outputs: g.outputs.clone(),
        })
    }

    /// Copy an input's values in (its length is fixed by the plan).
    pub fn set(&mut self, name: &str, values: &[f64]) -> Result<(), Error> {
        // A graph may read one input from several nodes; all take the value.
        let mut found = false;
        for &(ref n, i) in &self.inputs {
            if n != name {
                continue;
            }
            if values.len() != self.bufs[i].len() {
                return Err(Error::Shape(format!("{name}: {} values for {:?}", values.len(), self.shapes[i])));
            }
            self.bufs[i].copy_from_slice(values);
            found = true;
        }
        if !found {
            return Err(Error::MissingInput(name.into()));
        }
        Ok(())
    }

    pub fn output(&self, k: usize) -> &[f64] {
        &self.bufs[self.outputs[k]]
    }

    pub fn output_tensor(&self, k: usize) -> Tensor {
        let i = self.outputs[k];
        Tensor { shape: self.shapes[i].clone(), data: self.bufs[i].clone() }
    }

    /// Run every node in order.
    pub fn run(&mut self) {
        for i in 0..self.ops.len() {
            let (lo, hi) = self.bufs.split_at_mut(i);
            let out = &mut hi[0];
            let (op, args) = &self.ops[i];
            let a = |k: usize| lo[args[k]].as_slice();
            match op {
                Op::Input(_) | Op::Const(_) => {}
                Op::Add => zip(out, a(0), a(1), |x, y| x + y),
                Op::Sub => zip(out, a(0), a(1), |x, y| x - y),
                Op::Mul => zip(out, a(0), a(1), |x, y| x * y),
                Op::Div => zip(out, a(0), a(1), |x, y| x / y),
                Op::Max => zip(out, a(0), a(1), |x, y| if x >= y { x } else { y }),
                Op::Gt => zip(out, a(0), a(1), |x, y| if x > y { 1.0 } else { 0.0 }),
                Op::Neg => map(out, a(0), |x| -x),
                Op::Exp => map(out, a(0), pinned::exp),
                Op::Ln => map(out, a(0), pinned::ln),
                Op::Sin => map(out, a(0), pinned::sin),
                Op::Cos => map(out, a(0), pinned::cos),
                Op::Tanh => map(out, a(0), pinned::tanh),
                Op::Sqrt => map(out, a(0), pinned::sqrt),
                Op::Sum => out[0] = pinned::sum(a(0)),
                Op::Cumsum => {
                    let mut s = 0.0;
                    for (o, x) in out.iter_mut().zip(a(0)) {
                        s += x;
                        *o = canon(s);
                    }
                }
                Op::RevCumsum => {
                    let mut s = 0.0;
                    for (o, x) in out.iter_mut().zip(a(0)).rev() {
                        s += x;
                        *o = canon(s);
                    }
                }
                Op::Slice { start, len } => out.copy_from_slice(&a(0)[*start..start + len]),
                Op::Pad { start, .. } => {
                    out.fill(0.0);
                    let x = a(0);
                    out[*start..start + x.len()].copy_from_slice(x);
                }
                Op::Concat => {
                    let mut at = 0;
                    for k in 0..args.len() {
                        let x = a(k);
                        out[at..at + x.len()].copy_from_slice(x);
                        at += x.len();
                    }
                }
                Op::Fill { .. } => out.fill(a(0)[0]),
                Op::Reshape { .. } | Op::Within { .. } => out.copy_from_slice(a(0)),
                Op::MatMul => {
                    let (n, k) = (self.shapes[args[0]][0], self.shapes[args[0]][1]);
                    let m = self.shapes[args[1]][1];
                    let (x, y) = (a(0), a(1));
                    for r in 0..n {
                        for c in 0..m {
                            let mut s = 0.0;
                            for t in 0..k {
                                s += x[r * k + t] * y[t * m + c];
                            }
                            out[r * m + c] = canon(s);
                        }
                    }
                }
                Op::Transpose => {
                    let (r, c) = (self.shapes[args[0]][0], self.shapes[args[0]][1]);
                    let x = a(0);
                    for i in 0..r {
                        for j in 0..c {
                            out[j * r + i] = x[i * c + j];
                        }
                    }
                }
            }
        }
    }
}

#[inline]
fn map(out: &mut [f64], x: &[f64], f: impl Fn(f64) -> f64) {
    for (o, v) in out.iter_mut().zip(x) {
        *o = canon(f(*v));
    }
}

#[inline]
fn zip(out: &mut [f64], x: &[f64], y: &[f64], f: impl Fn(f64, f64) -> f64) {
    match (x.len(), y.len()) {
        (1, _) if out.len() != 1 => {
            for (o, b) in out.iter_mut().zip(y) {
                *o = canon(f(x[0], *b));
            }
        }
        (_, 1) if out.len() != 1 => {
            for (o, a) in out.iter_mut().zip(x) {
                *o = canon(f(*a, y[0]));
            }
        }
        _ => {
            for ((o, a), b) in out.iter_mut().zip(x).zip(y) {
                *o = canon(f(*a, *b));
            }
        }
    }
}
