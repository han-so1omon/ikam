//! Op graphs: a computation as a list of nodes in topological order, each
//! an op over earlier nodes (like a jaxpr or a TensorFlow graph). A graph
//! is stored as an ikam graph object, one node per op, so it is content
//! like any file: versioned, diffed, shared by scope and deduplicated.

use std::collections::BTreeMap;

use ikam_kernel::{Arg, Edge, Id, Node, Repo, Store, Target, node_key};

use crate::{Error, Tensor};

#[derive(Clone, Debug)]
pub enum Op {
    /// A named input, bound at evaluation.
    Input(String),
    Const(Tensor),
    // Elementwise; either side may be a scalar, which broadcasts.
    Add,
    Sub,
    Mul,
    Div,
    Max,
    /// 1.0 where a > b, else 0.0 (no gradient).
    Gt,
    Neg,
    Exp,
    Ln,
    Sin,
    Cos,
    Tanh,
    Sqrt,
    /// All values to a scalar, in index order.
    Sum,
    /// Rank 1: out[i] = x[0] + ... + x[i].
    Cumsum,
    /// Rank 1: out[i] = x[i] + ... + x[n-1].
    RevCumsum,
    /// Rank 1: x[start..start+len].
    Slice { start: usize, len: usize },
    /// Rank 1: x placed at `start` in zeros of length `total`.
    Pad { start: usize, total: usize },
    /// Rank 1: the arguments end to end.
    Concat,
    /// A scalar repeated to `shape`.
    Fill { shape: Vec<usize> },
    /// Rank 2 by rank 2.
    MatMul,
    /// Rank 2.
    Transpose,
}

impl Op {
    /// Bytes naming the op and its parameters (a constant by its value).
    pub fn encode(&self) -> Vec<u8> {
        let (name, params): (&str, Vec<u64>) = match self {
            Op::Input(n) => return [b"input:".as_slice(), n.as_bytes()].concat(),
            Op::Const(t) => return [b"const:".as_slice(), &t.encode()].concat(),
            Op::Add => ("add", vec![]),
            Op::Sub => ("sub", vec![]),
            Op::Mul => ("mul", vec![]),
            Op::Div => ("div", vec![]),
            Op::Max => ("max", vec![]),
            Op::Gt => ("gt", vec![]),
            Op::Neg => ("neg", vec![]),
            Op::Exp => ("exp", vec![]),
            Op::Ln => ("ln", vec![]),
            Op::Sin => ("sin", vec![]),
            Op::Cos => ("cos", vec![]),
            Op::Tanh => ("tanh", vec![]),
            Op::Sqrt => ("sqrt", vec![]),
            Op::Sum => ("sum", vec![]),
            Op::Cumsum => ("cumsum", vec![]),
            Op::RevCumsum => ("revcumsum", vec![]),
            Op::Slice { start, len } => ("slice", vec![*start as u64, *len as u64]),
            Op::Pad { start, total } => ("pad", vec![*start as u64, *total as u64]),
            Op::Concat => ("concat", vec![]),
            Op::Fill { shape } => ("fill", shape.iter().map(|d| *d as u64).collect()),
            Op::MatMul => ("matmul", vec![]),
            Op::Transpose => ("transpose", vec![]),
        };
        let mut out = name.as_bytes().to_vec();
        for p in params {
            out.push(b':');
            out.extend_from_slice(p.to_string().as_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Op, Error> {
        let bad = || Error::Decode("op");
        if let Some(n) = bytes.strip_prefix(b"input:") {
            return Ok(Op::Input(String::from_utf8(n.to_vec()).map_err(|_| bad())?));
        }
        if let Some(t) = bytes.strip_prefix(b"const:") {
            return Ok(Op::Const(Tensor::decode(t)?));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
        let mut parts = text.split(':');
        let name = parts.next().ok_or_else(bad)?;
        let p: Vec<usize> = parts.map(|s| s.parse().map_err(|_| bad())).collect::<Result<_, _>>()?;
        Ok(match (name, p.as_slice()) {
            ("add", []) => Op::Add,
            ("sub", []) => Op::Sub,
            ("mul", []) => Op::Mul,
            ("div", []) => Op::Div,
            ("max", []) => Op::Max,
            ("gt", []) => Op::Gt,
            ("neg", []) => Op::Neg,
            ("exp", []) => Op::Exp,
            ("ln", []) => Op::Ln,
            ("sin", []) => Op::Sin,
            ("cos", []) => Op::Cos,
            ("tanh", []) => Op::Tanh,
            ("sqrt", []) => Op::Sqrt,
            ("sum", []) => Op::Sum,
            ("cumsum", []) => Op::Cumsum,
            ("revcumsum", []) => Op::RevCumsum,
            ("slice", [s, l]) => Op::Slice { start: *s, len: *l },
            ("pad", [s, t]) => Op::Pad { start: *s, total: *t },
            ("concat", []) => Op::Concat,
            ("fill", shape) => Op::Fill { shape: shape.to_vec() },
            ("matmul", []) => Op::MatMul,
            ("transpose", []) => Op::Transpose,
            _ => return Err(bad()),
        })
    }
}

#[derive(Clone, Debug)]
pub struct OpNode {
    pub op: Op,
    pub args: Vec<usize>,
}

/// Nodes in topological order (every argument is an earlier node) and the
/// nodes whose values the graph returns.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<OpNode>,
    pub outputs: Vec<usize>,
}

/// The value of a node, as an index into its graph.
pub type V = usize;

impl Graph {
    pub fn new() -> Graph {
        Graph::default()
    }

    pub fn push(&mut self, op: Op, args: &[V]) -> V {
        assert!(args.iter().all(|a| *a < self.nodes.len()), "arguments must come first");
        self.nodes.push(OpNode { op, args: args.to_vec() });
        self.nodes.len() - 1
    }

    pub fn input(&mut self, name: &str) -> V {
        self.push(Op::Input(name.into()), &[])
    }

    pub fn konst(&mut self, t: Tensor) -> V {
        self.push(Op::Const(t), &[])
    }

    pub fn scalar(&mut self, v: f64) -> V {
        self.konst(Tensor::scalar(v))
    }

    pub fn add(&mut self, a: V, b: V) -> V {
        self.push(Op::Add, &[a, b])
    }
    pub fn sub(&mut self, a: V, b: V) -> V {
        self.push(Op::Sub, &[a, b])
    }
    pub fn mul(&mut self, a: V, b: V) -> V {
        self.push(Op::Mul, &[a, b])
    }
    pub fn div(&mut self, a: V, b: V) -> V {
        self.push(Op::Div, &[a, b])
    }
    pub fn max(&mut self, a: V, b: V) -> V {
        self.push(Op::Max, &[a, b])
    }
    pub fn neg(&mut self, a: V) -> V {
        self.push(Op::Neg, &[a])
    }
    pub fn sum(&mut self, a: V) -> V {
        self.push(Op::Sum, &[a])
    }
    pub fn unary(&mut self, op: Op, a: V) -> V {
        self.push(op, &[a])
    }
    pub fn slice(&mut self, a: V, start: usize, len: usize) -> V {
        self.push(Op::Slice { start, len }, &[a])
    }
    pub fn cumsum(&mut self, a: V) -> V {
        self.push(Op::Cumsum, &[a])
    }
    pub fn concat(&mut self, parts: &[V]) -> V {
        self.push(Op::Concat, parts)
    }

    pub fn input_names(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter_map(|n| match &n.op {
                Op::Input(name) => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn input_node(&self, name: &str) -> Option<V> {
        self.nodes.iter().position(|n| matches!(&n.op, Op::Input(m) if m == name))
    }

    /// Every node's shape, given the inputs' shapes.
    pub fn shapes(&self, inputs: &BTreeMap<String, Vec<usize>>) -> Result<Vec<Vec<usize>>, Error> {
        let mut out: Vec<Vec<usize>> = Vec::with_capacity(self.nodes.len());
        for (i, n) in self.nodes.iter().enumerate() {
            let s: Vec<&Vec<usize>> = n.args.iter().map(|a| &out[*a]).collect();
            let bad = |why: &str| Error::Shape(format!("node {i} {:?}: {why}", n.op));
            let shape = match &n.op {
                Op::Input(name) => inputs.get(name).cloned().ok_or(Error::MissingInput(name.clone()))?,
                Op::Const(t) => t.shape.clone(),
                Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Max | Op::Gt => {
                    match (s[0].is_empty(), s[1].is_empty()) {
                        (true, _) => s[1].clone(),
                        (_, true) => s[0].clone(),
                        _ if s[0] == s[1] => s[0].clone(),
                        _ => return Err(bad("shapes differ")),
                    }
                }
                Op::Neg | Op::Exp | Op::Ln | Op::Sin | Op::Cos | Op::Tanh | Op::Sqrt => s[0].clone(),
                Op::Sum => vec![],
                Op::Cumsum | Op::RevCumsum => {
                    if s[0].len() != 1 {
                        return Err(bad("rank 1 only"));
                    }
                    s[0].clone()
                }
                Op::Slice { start, len } => {
                    if s[0].len() != 1 || start + len > s[0][0] {
                        return Err(bad("slice outside"));
                    }
                    vec![*len]
                }
                Op::Pad { start, total } => {
                    if s[0].len() != 1 || start + s[0][0] > *total {
                        return Err(bad("pad outside"));
                    }
                    vec![*total]
                }
                Op::Concat => {
                    if s.iter().any(|x| x.len() != 1) {
                        return Err(bad("rank 1 only"));
                    }
                    vec![s.iter().map(|x| x[0]).sum()]
                }
                Op::Fill { shape } => {
                    if !s[0].is_empty() {
                        return Err(bad("fill takes a scalar"));
                    }
                    shape.clone()
                }
                Op::MatMul => {
                    if s[0].len() != 2 || s[1].len() != 2 || s[0][1] != s[1][0] {
                        return Err(bad("matmul shapes"));
                    }
                    vec![s[0][0], s[1][1]]
                }
                Op::Transpose => {
                    if s[0].len() != 2 {
                        return Err(bad("rank 2 only"));
                    }
                    vec![s[0][1], s[0][0]]
                }
            };
            out.push(shape);
        }
        Ok(out)
    }

    /// Store as an ikam graph object: node `n000042` targets its op's
    /// bytes, with edges `a0`, `a1`, ... to its arguments; node `outputs`
    /// lists the returned nodes.
    pub fn put<S: Store>(&self, repo: &mut Repo<S>) -> Result<Id, Error> {
        let mut nodes = Vec::with_capacity(self.nodes.len() + 1);
        for (i, n) in self.nodes.iter().enumerate() {
            let target = Target::Content(Arg::Whole(repo.put_content(&n.op.encode())?.id));
            let edges = n
                .args
                .iter()
                .enumerate()
                .map(|(k, a)| Edge { to: node_key(&label(*a)), label: format!("a{k}") })
                .collect();
            nodes.push(Node { label: label(i), target, edges });
        }
        let outs: Vec<u8> = self.outputs.iter().flat_map(|o| (*o as u64).to_le_bytes()).collect();
        let target = Target::Content(Arg::Whole(repo.put_content(&outs)?.id));
        nodes.push(Node { label: "outputs".into(), target, edges: vec![] });
        Ok(repo.put_graph(nodes)?)
    }

    pub fn load<S: Store>(repo: &Repo<S>, root: &Id) -> Result<Graph, Error> {
        let mut by_key = BTreeMap::new();
        let all = repo.graph_nodes(root)?;
        for n in &all {
            by_key.insert(node_key(&n.label), n.label.clone());
        }
        let mut g = Graph::new();
        for n in all.iter().filter(|n| n.label.starts_with('n')) {
            let Target::Content(Arg::Whole(id)) = &n.target else {
                return Err(Error::Decode("graph node"));
            };
            let op = Op::decode(&repo.read_content(id)?)?;
            let mut edges = n.edges.clone();
            edges.sort_by_key(|e| e.label[1..].parse::<usize>().unwrap_or(usize::MAX));
            let args = edges
                .iter()
                .map(|e| by_key.get(&e.to).and_then(|l| l[1..].parse().ok()).ok_or(Error::Decode("edge")))
                .collect::<Result<Vec<usize>, _>>()?;
            g.nodes.push(OpNode { op, args });
        }
        if let Some(n) = all.iter().find(|n| n.label == "outputs")
            && let Target::Content(Arg::Whole(id)) = &n.target
        {
            g.outputs = repo
                .read_content(id)?
                .chunks_exact(8)
                .map(|c| u64::from_le_bytes(c.try_into().unwrap()) as usize)
                .collect();
        }
        Ok(g)
    }
}

fn label(i: usize) -> String {
    format!("n{i:06}")
}
