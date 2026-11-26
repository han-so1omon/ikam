//! Evaluation in memory: each op over tensors with pinned arithmetic, so
//! the same graph and inputs give the same bits everywhere.

use std::collections::BTreeMap;

use crate::ir::{Graph, Op};
use crate::pinned::{self, canon};
use crate::{Error, Tensor};

/// Inputs by name.
pub type Env = BTreeMap<String, Tensor>;

/// Every node's value.
pub fn eval_all(g: &Graph, env: &Env) -> Result<Vec<Tensor>, Error> {
    let mut vals: Vec<Tensor> = Vec::with_capacity(g.nodes.len());
    for (i, n) in g.nodes.iter().enumerate() {
        let args: Vec<&Tensor> = n.args.iter().map(|a| &vals[*a]).collect();
        let v = apply(&n.op, &args, env).map_err(|e| match e {
            Error::Shape(m) => Error::Shape(format!("node {i}: {m}")),
            e => e,
        })?;
        vals.push(v);
    }
    Ok(vals)
}

/// The graph's outputs.
pub fn eval(g: &Graph, env: &Env) -> Result<Vec<Tensor>, Error> {
    let vals = eval_all(g, env)?;
    Ok(g.outputs.iter().map(|o| vals[*o].clone()).collect())
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
        Op::MatMul => matmul(a[0], a[1])?,
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

/// Row by column, each dot product summed in index order.
fn matmul(a: &Tensor, b: &Tensor) -> Result<Tensor, Error> {
    let (n, k) = dims2(a)?;
    let (k2, m) = dims2(b)?;
    if k != k2 {
        return Err(Error::Shape(format!("matmul {n}x{k} by {k2}x{m}")));
    }
    let mut data = vec![0.0; n * m];
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
