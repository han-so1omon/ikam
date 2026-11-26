//! Reverse-mode differentiation as a graph-to-graph transform (as JAX's
//! `grad` turns a jaxpr into another jaxpr). The gradient is an ordinary
//! op graph: it is stored, versioned and memoized like the graph it came
//! from, and it can be differentiated again (Hessian-vector products).

use std::collections::BTreeMap;

use crate::ir::{Graph, Op, V};
use crate::{Error, Tensor};

/// A graph returning `[of, d of / d wrt[0], ...]`: `g`'s nodes, then the
/// adjoint nodes. `of` must be a scalar; `wrt` names inputs. Shapes are
/// needed to undo scalar broadcasts, so the input shapes are given.
pub fn grad(
    g: &Graph,
    shapes: &BTreeMap<String, Vec<usize>>,
    of: V,
    wrt: &[&str],
) -> Result<Graph, Error> {
    let shape = g.shapes(shapes)?;
    if !shape[of].is_empty() {
        return Err(Error::Shape(format!("grad of a non-scalar {:?}", shape[of])));
    }
    let mut h = g.clone();
    let mut adj: BTreeMap<V, V> = BTreeMap::new();
    let one = h.scalar(1.0);
    adj.insert(of, one);
    for i in (0..=of).rev() {
        let Some(&gy) = adj.get(&i) else { continue };
        let node = g.nodes[i].clone();
        for (k, contrib) in vjp(&mut h, &node.op, &node.args, i, gy, &shape)? {
            let a = node.args[k];
            let total = match adj.get(&a) {
                Some(&prev) => h.add(prev, contrib),
                None => contrib,
            };
            adj.insert(a, total);
        }
    }
    let mut outputs = vec![of];
    for name in wrt {
        let x = g.input_node(name).ok_or_else(|| Error::MissingInput(name.to_string()))?;
        let gx = match adj.get(&x) {
            Some(&v) => v,
            None => {
                let z = h.scalar(0.0);
                h.push(Op::Fill { shape: shape[x].clone() }, &[z])
            }
        };
        outputs.push(gx);
    }
    h.outputs = outputs;
    Ok(h)
}

/// Sum a contribution down to an argument's shape when that argument was
/// a broadcast scalar.
fn unbroadcast(h: &mut Graph, c: V, arg_shape: &[usize], out_shape: &[usize]) -> V {
    if arg_shape.is_empty() && !out_shape.is_empty() { h.sum(c) } else { c }
}

/// Contributions to each argument's adjoint, as (argument index, node).
fn vjp(
    h: &mut Graph,
    op: &Op,
    args: &[V],
    y: V,
    gy: V,
    shape: &[Vec<usize>],
) -> Result<Vec<(usize, V)>, Error> {
    let sa = |k: usize| shape[args[k]].clone();
    let sy = shape[y].clone();
    let ub = |h: &mut Graph, c: V, k: usize| unbroadcast(h, c, &shape[args[k]], &sy);
    Ok(match op {
        Op::Input(_) | Op::Const(_) | Op::Gt => vec![],
        Op::Add => {
            let (a, b) = (ub(h, gy, 0), ub(h, gy, 1));
            vec![(0, a), (1, b)]
        }
        Op::Sub => {
            let n = h.neg(gy);
            let (a, b) = (ub(h, gy, 0), ub(h, n, 1));
            vec![(0, a), (1, b)]
        }
        Op::Mul => {
            let ca = h.mul(gy, args[1]);
            let cb = h.mul(gy, args[0]);
            let (a, b) = (ub(h, ca, 0), ub(h, cb, 1));
            vec![(0, a), (1, b)]
        }
        Op::Div => {
            let ca = h.div(gy, args[1]);
            let t = h.mul(gy, y);
            let t = h.div(t, args[1]);
            let cb = h.neg(t);
            let (a, b) = (ub(h, ca, 0), ub(h, cb, 1));
            vec![(0, a), (1, b)]
        }
        Op::Max => {
            // The first argument takes ties, as eval's `x >= y` does.
            let lt = h.push(Op::Gt, &[args[1], args[0]]);
            let one = h.scalar(1.0);
            let ge = h.sub(one, lt);
            let ca = h.mul(gy, ge);
            let cb = h.mul(gy, lt);
            let (a, b) = (ub(h, ca, 0), ub(h, cb, 1));
            vec![(0, a), (1, b)]
        }
        Op::Neg => vec![(0, h.neg(gy))],
        Op::Exp => vec![(0, h.mul(gy, y))],
        Op::Ln => vec![(0, h.div(gy, args[0]))],
        Op::Sin => {
            let c = h.unary(Op::Cos, args[0]);
            vec![(0, h.mul(gy, c))]
        }
        Op::Cos => {
            let s = h.unary(Op::Sin, args[0]);
            let t = h.mul(gy, s);
            vec![(0, h.neg(t))]
        }
        Op::Tanh => {
            let yy = h.mul(y, y);
            let one = h.scalar(1.0);
            let d = h.sub(one, yy);
            vec![(0, h.mul(gy, d))]
        }
        Op::Sqrt => {
            let half = h.scalar(0.5);
            let t = h.mul(gy, half);
            vec![(0, h.div(t, y))]
        }
        Op::Sum => vec![(0, h.push(Op::Fill { shape: sa(0) }, &[gy]))],
        Op::Cumsum => vec![(0, h.push(Op::RevCumsum, &[gy]))],
        Op::RevCumsum => vec![(0, h.push(Op::Cumsum, &[gy]))],
        Op::Slice { start, .. } => vec![(0, h.push(Op::Pad { start: *start, total: sa(0)[0] }, &[gy]))],
        Op::Pad { start, .. } => vec![(0, h.push(Op::Slice { start: *start, len: sa(0)[0] }, &[gy]))],
        Op::Concat => {
            let mut at = 0;
            let mut out = Vec::new();
            for k in 0..args.len() {
                let len = sa(k)[0];
                out.push((k, h.push(Op::Slice { start: at, len }, &[gy])));
                at += len;
            }
            out
        }
        Op::Fill { .. } => vec![(0, h.sum(gy))],
        Op::MatMul => {
            let bt = h.push(Op::Transpose, &[args[1]]);
            let at = h.push(Op::Transpose, &[args[0]]);
            let ga = h.push(Op::MatMul, &[gy, bt]);
            let gb = h.push(Op::MatMul, &[at, gy]);
            vec![(0, ga), (1, gb)]
        }
        Op::Transpose => vec![(0, h.push(Op::Transpose, &[gy]))],
    })
}

/// A graph returning the Hessian of `of` (output 0 of `grad`'s inputs)
/// times direction inputs `v:<name>`: `[H v for each wrt]`. Built as the
/// gradient of `sum(grad . v)`, reverse over reverse.
pub fn hvp(
    g: &Graph,
    shapes: &BTreeMap<String, Vec<usize>>,
    of: V,
    wrt: &[&str],
) -> Result<Graph, Error> {
    let mut gg = grad(g, shapes, of, wrt)?;
    let mut shapes2 = shapes.clone();
    let mut terms = Vec::new();
    for (k, name) in wrt.iter().enumerate() {
        let vname = format!("v:{name}");
        shapes2.insert(vname.clone(), shapes[*name].clone());
        let v = gg.input(&vname);
        let gk = gg.outputs[k + 1];
        let p = gg.mul(gk, v);
        terms.push(gg.sum(p));
    }
    let mut s = terms[0];
    for t in &terms[1..] {
        s = gg.add(s, *t);
    }
    let mut h = grad(&gg, &shapes2, s, wrt)?;
    h.outputs.remove(0);
    Ok(h)
}

/// Gradient of `f` at `env` by central differences, for checking `grad`.
pub fn numeric_grad(
    g: &Graph,
    env: &crate::eval::Env,
    of: V,
    name: &str,
    step: f64,
) -> Result<Tensor, Error> {
    let x = env.get(name).ok_or_else(|| Error::MissingInput(name.into()))?;
    let mut out = Tensor::zeros(&x.shape);
    let f = |e: &crate::eval::Env| -> Result<f64, Error> { Ok(crate::eval::eval_all(g, e)?[of].item()) };
    for i in 0..x.data.len() {
        let mut e = env.clone();
        e.get_mut(name).unwrap().data[i] = x.data[i] + step;
        let fp = f(&e)?;
        e.get_mut(name).unwrap().data[i] = x.data[i] - step;
        let fm = f(&e)?;
        out.data[i] = (fp - fm) / (2.0 * step);
    }
    Ok(out)
}
