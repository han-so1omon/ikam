//! Evaluation on the store: every node's value is content, and each
//! computation `value = op(argument ids)` is logged in an ikam database
//! under its key, so a rerun reuses what was computed (a memo, as the
//! kernel's ledger is for pure functions) and every value has provenance.
//!
//! The database refuses a key logged again with different bytes
//! (`IdempotencyConflict`), so a step that computed other bits on a rerun
//! is caught rather than silently replaced: this is the "recorded" step
//! kind (outputs kept, a rerun is a check). The kernel's own ledger cannot
//! hold these records yet: a derivation's function must be a builtin or a
//! WASM module it can re-run, and WASM floats are off (see the plan's
//! kernel changes).

use std::collections::HashMap;

use ikam_kernel::{Db, Event, Form, Id, Repo, Store};

use crate::bounds::{Report, coverage};
use crate::eval::{Arith, Env, apply, apply_with, eval_with};
use crate::ir::{Graph, Op};
use crate::{Error, Tensor};

pub const MEMO_PREFIX: &str = "memo/";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub hits: usize,
    pub misses: usize,
    /// Values that were content already (equal values from other keys).
    pub dedup: usize,
}

/// The memo key of `op` over argument contents.
pub fn key(op: &Op, args: &[Id]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"ikam-compute/eval/1\0");
    let enc = match op {
        // An input's value is its argument; its name does not change it.
        Op::Input(_) => b"input".to_vec(),
        op => op.encode(),
    };
    h.update(&(enc.len() as u64).to_le_bytes());
    h.update(&enc);
    for a in args {
        h.update(a.as_bytes());
    }
    format!("{MEMO_PREFIX}{}", h.finalize().to_hex())
}

/// Evaluate `g` with every value stored as content. Returns the outputs'
/// ids, the database with the new memo records, and counts. With
/// `check`, every node is recomputed and compared with its record.
pub fn eval_stored<S: Store>(
    repo: &mut Repo<S>,
    db: &Db,
    g: &Graph,
    env: &Env,
    check: bool,
) -> Result<(Vec<Id>, Db, Stats), Error> {
    let mut ids: Vec<Id> = Vec::with_capacity(g.nodes.len());
    let mut cache: HashMap<Id, Tensor> = HashMap::new();
    let mut events = Vec::new();
    let mut stats = Stats::default();
    for n in &g.nodes {
        let (args, k) = match &n.op {
            Op::Input(name) => {
                let t = env.get(name).ok_or_else(|| Error::MissingInput(name.clone()))?;
                let id = put(repo, t, &mut cache, &mut stats)?;
                (vec![id], key(&n.op, &[id]))
            }
            _ => {
                let args: Vec<Id> = n.args.iter().map(|a| ids[*a]).collect();
                let k = key(&n.op, &args);
                (args, k)
            }
        };
        if matches!(n.op, Op::Input(_)) {
            ids.push(args[0]);
            continue;
        }
        let recorded = match repo.db_get(db, &k)? {
            Some(b) if b.len() == 32 => Some(Id::from_bytes(b.try_into().unwrap())),
            _ => None,
        };
        if let Some(id) = recorded
            && !check
            && repo.has_content(&id)?
        {
            stats.hits += 1;
            ids.push(id);
            continue;
        }
        let vals: Vec<Tensor> = args.iter().map(|a| get(repo, a, &mut cache)).collect::<Result<_, _>>()?;
        let refs: Vec<&Tensor> = vals.iter().collect();
        let out = apply(&n.op, &refs, env)?;
        let id = put(repo, &out, &mut cache, &mut stats)?;
        match recorded {
            Some(r) if r != id => return Err(Error::Nondeterministic { key: k, recorded: r, now: id }),
            Some(_) => stats.hits += 1,
            None => {
                stats.misses += 1;
                events.push(Event::upsert(&k, id.as_bytes(), &k));
            }
        }
        ids.push(id);
    }
    let db = if events.is_empty() { *db } else { repo.db_append(db, &events)? };
    Ok((g.outputs.iter().map(|o| ids[*o]).collect(), db, stats))
}

fn put<S: Store>(
    repo: &mut Repo<S>,
    t: &Tensor,
    cache: &mut HashMap<Id, Tensor>,
    stats: &mut Stats,
) -> Result<Id, Error> {
    let put = repo.put_content(&t.encode())?;
    if matches!(put.form, Form::Existing) {
        stats.dedup += 1;
    }
    let id = put.id;
    cache.insert(id, t.clone());
    Ok(id)
}

pub fn get<S: Store>(repo: &Repo<S>, id: &Id, cache: &mut HashMap<Id, Tensor>) -> Result<Tensor, Error> {
    if let Some(t) = cache.get(id) {
        return Ok(t.clone());
    }
    let t = Tensor::decode(&repo.read_content(id)?)?;
    cache.insert(*id, t.clone());
    Ok(t)
}

/// The memo key of a whole stored graph over its inputs' contents.
pub fn graph_key(graph: &Id, inputs: &[(String, Id)]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"ikam-compute/graph/1\0");
    h.update(graph.as_bytes());
    for (name, id) in inputs {
        h.update(&(name.len() as u64).to_le_bytes());
        h.update(name.as_bytes());
        h.update(id.as_bytes());
    }
    format!("{MEMO_PREFIX}g/{}", h.finalize().to_hex())
}

/// Evaluate a stored graph (`graph` is its id from `Graph::put`) as one
/// recorded step: one record maps (graph, input contents) to the output
/// contents, so a rerun costs one lookup however many nodes the graph
/// has. Intermediate values are not kept. With `check`, the graph is run
/// again and compared with the record. Returns the output ids, the
/// database, and whether the record was reused.
pub fn eval_graph_stored<S: Store>(
    repo: &mut Repo<S>,
    db: &Db,
    g: &Graph,
    graph: &Id,
    env: &Env,
    check: bool,
) -> Result<(Vec<Id>, Db, bool), Error> {
    let mut cache = HashMap::new();
    let mut stats = Stats::default();
    let mut inputs = Vec::new();
    for name in g.input_names() {
        let t = env.get(&name).ok_or_else(|| Error::MissingInput(name.clone()))?;
        inputs.push((name, put(repo, t, &mut cache, &mut stats)?));
    }
    let k = graph_key(graph, &inputs);
    let recorded: Option<Vec<Id>> = repo.db_get(db, &k)?.and_then(|b| {
        (b.len() == 32 * g.outputs.len())
            .then(|| b.chunks(32).map(|c| Id::from_bytes(c.try_into().unwrap())).collect())
    });
    // A record can arrive without the content it names (a records share
    // carries record bytes only), so reuse it only when every output's
    // content is here; otherwise compute again.
    if let Some(ids) = &recorded
        && !check
        && ids.iter().map(|i| repo.has_content(i)).collect::<Result<Vec<_>, _>>()?.into_iter().all(|h| h)
    {
        return Ok((ids.clone(), *db, true));
    }
    let outs = crate::eval::eval(g, env)?;
    let mut ids = Vec::with_capacity(outs.len());
    for t in &outs {
        ids.push(put(repo, t, &mut cache, &mut stats)?);
    }
    match recorded {
        Some(r) if r != ids => {
            let i = r.iter().zip(&ids).position(|(a, b)| a != b).unwrap_or(0);
            Err(Error::Nondeterministic { key: k, recorded: r[i], now: ids[i] })
        }
        Some(_) => Ok((ids, *db, true)),
        None => {
            let bytes: Vec<u8> = ids.iter().flat_map(|i| i.as_bytes().to_vec()).collect();
            let db = repo.db_append(db, &[Event::upsert(&k, &bytes, &k)])?;
            Ok((ids, db, false))
        }
    }
}

fn recorded_id<S: Store>(repo: &Repo<S>, db: &Db, k: &str) -> Result<Id, Error> {
    match repo.db_get(db, k)? {
        Some(b) if b.len() == 32 => Ok(Id::from_bytes(b.try_into().unwrap())),
        _ => Err(Error::NotRecorded(k.to_string())),
    }
}

/// Check a rerun against the records `eval_stored` made, with bounds on
/// nondeterminism (E442). The graph is evaluated again in memory with
/// `arith` and every node is compared with its record, found under the
/// first run's key (the op over the recorded argument ids), so a value
/// that moved upstream does not make the lookups downstream miss. A node
/// whose bits differ must be covered by a Within node (`bounds::coverage`:
/// itself, or one on every path to an output), or the check fails with
/// `Error::Nondeterministic` as an exact check does. Each Within node is
/// reported as same, within or outside; past a local one the check
/// carries on from the value of record. Nothing is written: no memo
/// record and no content, so the first run stays the one of record. A
/// graph without Within nodes is checked exactly.
pub fn check_bounded<S: Store>(repo: &Repo<S>, db: &Db, g: &Graph, env: &Env, arith: Arith) -> Result<Report, Error> {
    let covered = coverage(g);
    let mut rec: Vec<Id> = Vec::with_capacity(g.nodes.len());
    let mut now: Vec<Tensor> = Vec::with_capacity(g.nodes.len());
    let mut cache: HashMap<Id, Tensor> = HashMap::new();
    let mut report = Report::default();
    for (i, n) in g.nodes.iter().enumerate() {
        if let Op::Input(name) = &n.op {
            let t = env.get(name).ok_or_else(|| Error::MissingInput(name.clone()))?;
            rec.push(Id::of_content(&t.encode()));
            now.push(t.clone());
            continue;
        }
        let args: Vec<Id> = n.args.iter().map(|a| rec[*a]).collect();
        let k = key(&n.op, &args);
        let r = recorded_id(repo, db, &k)?;
        let refs: Vec<&Tensor> = n.args.iter().map(|a| &now[*a]).collect();
        let v = apply_with(&n.op, &refs, env, arith)?;
        let vid = Id::of_content(&v.encode());
        report.compared += 1;
        let differs = vid != r;
        report.differing += differs as usize;
        let v = match &n.op {
            Op::Within { local, .. } => {
                let recorded = get(repo, &r, &mut cache)?;
                report.judge(i, &n.op, &recorded, &v);
                if *local { recorded } else { v }
            }
            _ if differs && !covered[i] => {
                return Err(Error::Nondeterministic { key: k, recorded: r, now: vid });
            }
            _ => v,
        };
        rec.push(r);
        now.push(v);
    }
    Ok(report)
}

/// Check a rerun against the whole-graph record `eval_graph_stored` made,
/// with bounds (E442). Only the outputs were recorded, so the bounds are
/// the Within nodes among the outputs: an output that differs must be
/// one, and each is reported; Within nodes inside the graph pass values
/// through (a local one cannot restart from a record that was not kept).
/// Nothing is written.
pub fn check_graph_bounded<S: Store>(
    repo: &Repo<S>,
    db: &Db,
    g: &Graph,
    graph: &Id,
    env: &Env,
    arith: Arith,
) -> Result<Report, Error> {
    let mut inputs = Vec::new();
    for name in g.input_names() {
        let t = env.get(&name).ok_or_else(|| Error::MissingInput(name.clone()))?;
        inputs.push((name, Id::of_content(&t.encode())));
    }
    let k = graph_key(graph, &inputs);
    let rec: Vec<Id> = match repo.db_get(db, &k)? {
        Some(b) if b.len() == 32 * g.outputs.len() => {
            b.chunks(32).map(|c| Id::from_bytes(c.try_into().unwrap())).collect()
        }
        _ => return Err(Error::NotRecorded(k)),
    };
    let outs = eval_with(g, env, arith)?;
    let mut cache = HashMap::new();
    let mut report = Report::default();
    for ((o, v), r) in g.outputs.iter().zip(&outs).zip(&rec) {
        let vid = Id::of_content(&v.encode());
        report.compared += 1;
        let differs = vid != *r;
        report.differing += differs as usize;
        let op = &g.nodes[*o].op;
        if matches!(op, Op::Within { .. }) {
            let recorded = get(repo, r, &mut cache)?;
            report.judge(*o, op, &recorded, v);
        } else if differs {
            return Err(Error::Nondeterministic { key: k, recorded: *r, now: vid });
        }
    }
    Ok(report)
}
