//! Graph functions as WASM modules that read a graph through read-only host
//! imports (E452, an option; nothing in the default paths calls this).
//!
//! A content function (`wasm.rs`) imports nothing and gets its arguments as
//! bytes. A graph function gets its graph argument as host imports over a
//! [`GraphRead`] instead, so it reads lazily what it needs. The imports only
//! read, and what they read is named by the graph's root (content
//! addressed), so a run is still a pure function of its arguments and
//! deterministic as `wasm.rs` requires: same module, same roots, same output.
//!
//! Imports, module `ikam_graph`, each writing UTF-8 lines into the module's
//! memory at `out` (at most `cap` bytes) and returning their length: -1 if
//! the node is absent, or the length needed (more than `cap`) for the module
//! to call again with room.
//! - `out_edges(label, label_len, out, cap)`: `target label TAB edge label`
//!   per edge to a present node.
//! - `in_edges(label, label_len, out, cap)`: `source label TAB edge label`
//!   per edge into the node.
//! - `range(lo, lo_len, hi, hi_len, out, cap)`: the labels with
//!   `lo <= label < hi` (`hi_len` 0: to the end).
//!
//! ABI otherwise as `wasm.rs`: `memory`, `alloc`, `run(ptr, len) -> i64`;
//! the input is the content arguments (`count:u32 { len:u32 bytes }`). The
//! output is lines `label TAB value`, which [`Repo::graph_derive_wasm`]
//! stores as a result graph (each value on an edge to the zero key, labels
//! sorted, the first line for a label kept).

use std::collections::BTreeMap;

use wasmi::{Caller, Config, Engine, Linker, Module, StoreLimits, StoreLimitsBuilder};

use super::compute::Compute;
use super::derive::{Derived, FragRead, GraphRead};
use crate::{Arg, Edge, Error, GraphDerivation, Id, MemStore, Node, Repo, Store, Target};

const FUEL: u64 = 1 << 30;
const MAX_MEMORY: usize = 256 << 20;

struct Host<'a> {
    g: &'a mut dyn GraphRead,
    limits: StoreLimits,
}

fn fail(e: &dyn std::fmt::Display) -> Error {
    Error::Exec(format!("wasm graph: {e}"))
}

fn read_str(caller: &Caller<'_, Host<'_>>, ptr: i32, len: i32) -> Result<String, wasmi::Error> {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmi::Error::new("no memory"))?;
    let mut buf = vec![0; len.max(0) as usize];
    memory
        .read(caller, ptr as u32 as usize, &mut buf)
        .map_err(|e| wasmi::Error::new(e.to_string()))?;
    String::from_utf8(buf).map_err(|_| wasmi::Error::new("label is not UTF-8"))
}

fn write_out(
    caller: &mut Caller<'_, Host<'_>>,
    text: Option<String>,
    out: i32,
    cap: i32,
) -> Result<i32, wasmi::Error> {
    let Some(text) = text else {
        return Ok(-1);
    };
    let n = i32::try_from(text.len()).map_err(|_| wasmi::Error::new("output too large"))?;
    if n <= cap {
        let memory = caller
            .get_export("memory")
            .and_then(|e| e.into_memory())
            .ok_or_else(|| wasmi::Error::new("no memory"))?;
        memory
            .write(caller, out as u32 as usize, text.as_bytes())
            .map_err(|e| wasmi::Error::new(e.to_string()))?;
    }
    Ok(n)
}

fn lines(pairs: Vec<(String, String)>) -> String {
    pairs
        .into_iter()
        .map(|(a, b)| format!("{a}\t{b}\n"))
        .collect()
}

fn host_err(e: Error) -> wasmi::Error {
    wasmi::Error::new(e.to_string())
}

/// Run a graph module over `g` with content arguments `args`: its output
/// and the fuel it spent.
pub fn run_graph_module(
    module: &[u8],
    g: &mut dyn GraphRead,
    args: &[Vec<u8>],
) -> Result<(Vec<u8>, u64), Error> {
    let mut config = Config::default();
    config.floats(false).consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, module).map_err(|e| fail(&e))?;
    for import in module.imports() {
        if import.module() != "ikam_graph" {
            return Err(fail(&"a graph module imports only ikam_graph"));
        }
    }
    let host = Host {
        g,
        limits: StoreLimitsBuilder::new().memory_size(MAX_MEMORY).build(),
    };
    let mut store = wasmi::Store::new(&engine, host);
    store.limiter(|h: &mut Host<'_>| &mut h.limits);
    store.set_fuel(FUEL).map_err(|e| fail(&e))?;
    let mut linker: Linker<Host<'_>> = Linker::new(&engine);
    linker
        .func_wrap(
            "ikam_graph",
            "out_edges",
            |mut c: Caller<'_, Host<'_>>, p: i32, n: i32, out: i32, cap: i32| {
                let label = read_str(&c, p, n)?;
                let g = &mut c.data_mut().g;
                let text = match g.node(&label).map_err(host_err)? {
                    None => None,
                    Some(node) => {
                        let mut pairs = Vec::new();
                        for e in node.edges {
                            let to = u64::from_be_bytes(e.to);
                            if let Some(t) = g.label_of(to).map_err(host_err)? {
                                pairs.push((t, e.label));
                            }
                        }
                        Some(lines(pairs))
                    }
                };
                write_out(&mut c, text, out, cap)
            },
        )
        .map_err(|e| fail(&e))?;
    linker
        .func_wrap(
            "ikam_graph",
            "in_edges",
            |mut c: Caller<'_, Host<'_>>, p: i32, n: i32, out: i32, cap: i32| {
                let label = read_str(&c, p, n)?;
                let g = &mut c.data_mut().g;
                let text = match g.node(&label).map_err(host_err)? {
                    None => None,
                    Some(_) => Some(lines(g.in_edges(&label).map_err(host_err)?)),
                };
                write_out(&mut c, text, out, cap)
            },
        )
        .map_err(|e| fail(&e))?;
    linker
        .func_wrap(
            "ikam_graph",
            "range",
            |mut c: Caller<'_, Host<'_>>,
             lo: i32,
             lo_n: i32,
             hi: i32,
             hi_n: i32,
             out: i32,
             cap: i32| {
                let lo = read_str(&c, lo, lo_n)?;
                let hi = match hi_n {
                    0 => None,
                    _ => Some(read_str(&c, hi, hi_n)?),
                };
                let g = &mut c.data_mut().g;
                let nodes = g.range(&lo, hi.as_deref()).map_err(host_err)?;
                let text: String = nodes.into_iter().map(|n| n.label + "\n").collect();
                write_out(&mut c, Some(text), out, cap)
            },
        )
        .map_err(|e| fail(&e))?;
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| fail(&e))?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or_else(|| fail(&"no exported memory"))?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&store, "alloc")
        .map_err(|e| fail(&e))?;
    let entry = instance
        .get_typed_func::<(i32, i32), i64>(&store, "run")
        .map_err(|e| fail(&e))?;
    let mut input = Vec::new();
    input.extend_from_slice(&(args.len() as u32).to_be_bytes());
    for a in args {
        input.extend_from_slice(&(a.len() as u32).to_be_bytes());
        input.extend_from_slice(a);
    }
    let len = i32::try_from(input.len()).map_err(|_| fail(&"input too large"))?;
    let ptr = alloc.call(&mut store, len).map_err(|e| fail(&e))?;
    memory
        .write(&mut store, ptr as u32 as usize, &input)
        .map_err(|e| fail(&e))?;
    let packed = entry.call(&mut store, (ptr, len)).map_err(|e| fail(&e))? as u64;
    let mut out = vec![0; (packed & 0xffff_ffff) as usize];
    memory
        .read(&store, (packed >> 32) as usize, &mut out)
        .map_err(|e| fail(&e))?;
    let spent = FUEL - store.get_fuel().map_err(|e| fail(&e))?;
    Ok((out, spent))
}

/// A graph module's output (lines `label TAB value`) as result nodes.
fn output_nodes(out: &[u8]) -> Result<Vec<Node>, Error> {
    let text = std::str::from_utf8(out).map_err(|_| fail(&"output is not UTF-8"))?;
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let (label, value) = line.split_once('\t').unwrap_or((line, ""));
        nodes.entry(label.to_string()).or_insert_with(|| Node {
            label: label.to_string(),
            target: Target::None,
            edges: vec![Edge {
                to: [0; 8],
                label: value.to_string(),
            }],
        });
    }
    Ok(nodes.into_values().collect())
}

/// Whether content is a WASM module.
fn is_wasm(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\0asm")
}

impl<S: Store> Repo<S> {
    /// A WASM graph module (stored as content `module`) over the graph at
    /// `input` with content arguments `args`, as a recorded graph
    /// derivation `module(Graph(input), args...)`: a memo hit when recorded,
    /// otherwise run through host imports over [`FragRead`] (with the
    /// input's index, itself derived), its output stored as a graph.
    pub fn graph_derive_wasm(
        &mut self,
        module: &Id,
        input: &Id,
        args: &[Id],
    ) -> Result<Derived, Error> {
        let at = std::time::Instant::now();
        let mut all = vec![Arg::Graph(*input)];
        all.extend(args.iter().map(|a| Arg::Whole(*a)));
        if let Some((record, output)) = self.graph_memo(module, &all)? {
            return Ok(Derived {
                output,
                record,
                hit: true,
                cost: super::compute::Cost {
                    spec: "wasm".into(),
                    seconds: at.elapsed().as_secs_f64(),
                    ..Default::default()
                },
            });
        }
        let code = self.read_content(module)?;
        if !is_wasm(&code) {
            return Err(fail(&"not a WASM module"));
        }
        let inputs: Vec<Vec<u8>> = (args.iter())
            .map(|a| self.read_content(a))
            .collect::<Result<_, _>>()?;
        let index = self.graph_derive(&Compute::Index, input)?.output;
        let (out, reads) = {
            let mut g = FragRead::new(self, *input, Some(index));
            let (out, _) = run_graph_module(&code, &mut g, &inputs)?;
            (out, g.chunks_read())
        };
        let output = self.put_graph(output_nodes(&out)?)?;
        let record = self.record_graph_derivation(GraphDerivation {
            output,
            func: *module,
            args: all,
        })?;
        Ok(Derived {
            output,
            record,
            hit: false,
            cost: super::compute::Cost {
                spec: "wasm".into(),
                chunks_read: reads,
                changed: 0,
                seconds: at.elapsed().as_secs_f64(),
            },
        })
    }

    /// fsck_deep for a WASM graph derivation: rerun the module over a copy
    /// of the input in a scratch store and compare roots. `None` when the
    /// function is not a WASM module.
    pub(crate) fn check_wasm_graph_derivation(
        &self,
        d: &GraphDerivation,
    ) -> Result<Option<bool>, Error> {
        let code = self.read_content(&d.func)?;
        if !is_wasm(&code) {
            return Ok(None);
        }
        let Some(Arg::Graph(input)) = d.args.first() else {
            return Ok(Some(false));
        };
        let inputs: Vec<Vec<u8>> = (d.args[1..].iter())
            .map(|a| self.read_content(&a.id()))
            .collect::<Result<_, _>>()?;
        let mut scratch = Repo::new(MemStore::default());
        scratch.plain_hot_writes = true;
        let copy = scratch.put_graph(self.graph_nodes(input)?)?;
        let (index, _) = scratch.graph_compute_one(&copy, &Compute::Index)?;
        let out = {
            let mut g = FragRead::new(&scratch, copy, Some(index));
            run_graph_module(&code, &mut g, &inputs)?.0
        };
        let root = scratch.put_graph(output_nodes(&out)?)?;
        Ok(Some(copy == *input && root == d.output))
    }
}
