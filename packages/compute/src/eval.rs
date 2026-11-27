//! Evaluation in memory: each op over tensors with pinned arithmetic, so
//! the same graph and inputs give the same bits everywhere.
//!
//! [`Arith::Host`] (E442) is another arithmetic for the same graph, a
//! stand-in for another platform or build: the host's math library,
//! fused multiply-add and sums reordered into 8 lanes. It exists to
//! measure and check bounds on nondeterminism; `eval` and `eval_all`
//! stay pinned.

use std::collections::BTreeMap;

use crate::ir::{Graph, Op};
use crate::pinned::{self, canon};
use crate::{Error, Tensor};

/// Inputs by name.
pub type Env = BTreeMap<String, Tensor>;

/// Float arithmetic for an evaluation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Arith {
    /// The pinned rules (`pinned`): the same bits on every platform.
    #[default]
    Pinned,
    /// A stand-in for another platform or build (E409's modes at once):
    /// `exp`, `ln`, `sin`, `cos`, `tanh` and `sqrt` from the host's math
    /// library (`f64` methods, not `libm`), every multiply-add in a
    /// matrix product fused (`dot_host`), and sums kept in 8 lanes added
    /// pairwise at the end (`sum_lanes8`), as a vectorized build reorders
    /// them. NaN stays canonical.
    Host,
}

/// Every node's value.
pub fn eval_all(g: &Graph, env: &Env) -> Result<Vec<Tensor>, Error> {
    eval_all_with(g, env, Arith::Pinned)
}

/// Every node's value with the given arithmetic.
pub fn eval_all_with(g: &Graph, env: &Env, arith: Arith) -> Result<Vec<Tensor>, Error> {
    let mut vals: Vec<Tensor> = Vec::with_capacity(g.nodes.len());
    for (i, n) in g.nodes.iter().enumerate() {
        let args: Vec<&Tensor> = n.args.iter().map(|a| &vals[*a]).collect();
        let v = apply_with(&n.op, &args, env, arith).map_err(|e| match e {
            Error::Shape(m) => Error::Shape(format!("node {i}: {m}")),
            e => e,
        })?;
        vals.push(v);
    }
    Ok(vals)
}

/// The graph's outputs.
pub fn eval(g: &Graph, env: &Env) -> Result<Vec<Tensor>, Error> {
    eval_with(g, env, Arith::Pinned)
}

/// The graph's outputs with the given arithmetic.
pub fn eval_with(g: &Graph, env: &Env, arith: Arith) -> Result<Vec<Tensor>, Error> {
    let vals = eval_all_with(g, env, arith)?;
    Ok(g.outputs.iter().map(|o| vals[*o].clone()).collect())
}

/// Sum in 8 lanes (element i into lane i mod 8), the lanes added
/// pairwise: E409's `lanes8`.
pub fn sum_lanes8(xs: &[f64]) -> f64 {
    let mut l = [0.0f64; 8];
    for (i, x) in xs.iter().enumerate() {
        l[i % 8] += x;
    }
    canon(((l[0] + l[1]) + (l[2] + l[3])) + ((l[4] + l[5]) + (l[6] + l[7])))
}

/// Sum of products with every multiply-add fused: one chain for fewer
/// than 16 terms (as a scalar build with FMA), else 8 lanes of chains
/// added pairwise (a vectorized build with FMA).
pub fn dot_host(a: &[f64], b: &[f64]) -> f64 {
    if a.len() < 16 {
        let mut s = 0.0f64;
        for (x, y) in a.iter().zip(b) {
            s = x.mul_add(*y, s);
        }
        return canon(s);
    }
    let mut l = [0.0f64; 8];
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        l[i % 8] = x.mul_add(*y, l[i % 8]);
    }
    canon(((l[0] + l[1]) + (l[2] + l[3])) + ((l[4] + l[5]) + (l[6] + l[7])))
}

fn host_unary(op: &Op) -> Option<fn(f64) -> f64> {
    Some(match op {
        Op::Exp => f64::exp,
        Op::Ln => f64::ln,
        Op::Sin => f64::sin,
        Op::Cos => f64::cos,
        Op::Tanh => f64::tanh,
        Op::Sqrt => f64::sqrt,
        _ => return None,
    })
}

fn map(a: &Tensor, mut f: impl FnMut(f64) -> f64) -> Tensor {
    Tensor { shape: a.shape.clone(), data: a.data.iter().map(|x| canon(f(*x))).collect() }
}

fn zip(a: &Tensor, b: &Tensor, f: impl Fn(f64, f64) -> f64) -> Result<Tensor, Error> {
    let data: Vec<f64> = match (a.is_scalar(), b.is_scalar()) {
        (true, _) => b.data.iter().map(|y| canon(f(a.data[0], *y))).collect(),
        (_, true) => a.data.iter().map(|x| canon(f(*x, b.data[0]))).collect(),
        _ if a.shape == b.shape => a.data.iter().zip(&b.data).map(|(x, y)| canon(f(*x, *y))).collect(),
        _ => return Err(Error::Shape(format!("{:?} vs {:?}", a.shape, b.shape))),
    };
    let shape = if a.is_scalar() { b.shape.clone() } else { a.shape.clone() };
    Ok(Tensor { shape, data })
}

fn rank1(a: &Tensor) -> Result<(), Error> {
    if a.shape.len() == 1 { Ok(()) } else { Err(Error::Shape(format!("rank 1 expected, got {:?}", a.shape))) }
}

pub fn apply(op: &Op, a: &[&Tensor], env: &Env) -> Result<Tensor, Error> {
    apply_with(op, a, env, Arith::Pinned)
}

/// One op with the given arithmetic.
pub fn apply_with(op: &Op, a: &[&Tensor], env: &Env, arith: Arith) -> Result<Tensor, Error> {
    if arith == Arith::Host {
        if let Some(f) = host_unary(op) {
            return Ok(map(a[0], f));
        }
        match op {
            Op::Sum => return Ok(Tensor::scalar(sum_lanes8(&a[0].data))),
            Op::MatMul => return matmul_with(a[0], a[1], arith),
            _ => {}
        }
    }
    Ok(match op {
        Op::Input(name) => env.get(name).cloned().ok_or_else(|| Error::MissingInput(name.clone()))?,
        Op::Const(t) => t.clone(),
        Op::Add => zip(a[0], a[1], |x, y| x + y)?,
        Op::Sub => zip(a[0], a[1], |x, y| x - y)?,
        Op::Mul => zip(a[0], a[1], |x, y| x * y)?,
        Op::Div => zip(a[0], a[1], |x, y| x / y)?,
        Op::Max => zip(a[0], a[1], |x, y| if x >= y { x } else { y })?,
        Op::Gt => zip(a[0], a[1], |x, y| if x > y { 1.0 } else { 0.0 })?,
        Op::Neg => map(a[0], |x| -x),
        Op::Exp => map(a[0], pinned::exp),
        Op::Ln => map(a[0], pinned::ln),
        Op::Sin => map(a[0], pinned::sin),
        Op::Cos => map(a[0], pinned::cos),
        Op::Tanh => map(a[0], pinned::tanh),
        Op::Sqrt => map(a[0], pinned::sqrt),
        Op::Sum => Tensor::scalar(pinned::sum(&a[0].data)),
        Op::Cumsum => {
            rank1(a[0])?;
            let mut s = 0.0;
            map(a[0], |x| {
                s += x;
                s
            })
        }
        Op::RevCumsum => {
            rank1(a[0])?;
            let mut data = a[0].data.clone();
            let mut s = 0.0;
            for v in data.iter_mut().rev() {
                s += *v;
                *v = canon(s);
            }
            Tensor { shape: a[0].shape.clone(), data }
        }
        Op::Slice { start, len } => {
            rank1(a[0])?;
            let src = a[0].data.get(*start..start + len).ok_or(Error::Shape("slice outside".into()))?;
            Tensor::vector(src.to_vec())
        }
        Op::Pad { start, total } => {
            rank1(a[0])?;
            if start + a[0].data.len() > *total {
                return Err(Error::Shape("pad outside".into()));
            }
            let mut data = vec![0.0; *total];
            data[*start..start + a[0].data.len()].copy_from_slice(&a[0].data);
            Tensor::vector(data)
        }
        Op::Concat => {
            let mut data = Vec::new();
            for t in a {
                rank1(t)?;
                data.extend_from_slice(&t.data);
            }
            Tensor::vector(data)
        }
        Op::Fill { shape } => {
            if !a[0].is_scalar() {
                return Err(Error::Shape("fill takes a scalar".into()));
            }
            Tensor { shape: shape.clone(), data: vec![a[0].data[0]; shape.iter().product()] }
        }
        Op::Reshape { shape } => Tensor::new(shape.clone(), a[0].data.clone())?,
        Op::Within { .. } => a[0].clone(),
        Op::MatMul => matmul_with(a[0], a[1], Arith::Pinned)?,
        Op::Transpose => {
            let (r, c) = dims2(a[0])?;
            let mut data = vec![0.0; r * c];
            for i in 0..r {
                for j in 0..c {
                    data[j * r + i] = a[0].data[i * c + j];
                }
            }
            Tensor { shape: vec![c, r], data }
        }
    })
}

fn dims2(t: &Tensor) -> Result<(usize, usize), Error> {
    match t.shape.as_slice() {
        [r, c] => Ok((*r, *c)),
        s => Err(Error::Shape(format!("rank 2 expected, got {s:?}"))),
    }
}

/// Row by column, each dot product summed in index order (pinned) or as
/// `dot_host` (host).
fn matmul_with(a: &Tensor, b: &Tensor, arith: Arith) -> Result<Tensor, Error> {
    let (n, k) = dims2(a)?;
    let (k2, m) = dims2(b)?;
    if k != k2 {
        return Err(Error::Shape(format!("matmul {n}x{k} by {k2}x{m}")));
    }
    let mut data = vec![0.0; n * m];
    if arith == Arith::Host {
        let mut col = vec![0.0; k];
        for j in 0..m {
            for (t, c) in col.iter_mut().enumerate() {
                *c = b.data[t * m + j];
            }
            for i in 0..n {
                data[i * m + j] = dot_host(&a.data[i * k..(i + 1) * k], &col);
            }
        }
        return Ok(Tensor { shape: vec![n, m], data });
    }
    for i in 0..n {
        for j in 0..m {
            let mut s = 0.0;
            for t in 0..k {
                s += a.data[i * k + t] * b.data[t * m + j];
            }
            data[i * m + j] = canon(s);
        }
    }
    Ok(Tensor { shape: vec![n, m], data })
}
