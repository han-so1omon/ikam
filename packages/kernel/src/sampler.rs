//! A neighborhood sampler over the fragment graph (E427, an option).
//!
//! The graph is the fragment DAG under a commit (`fragment.rs`): one node
//! per file the commit's tree names and per fragment those files render,
//! one undirected edge per `render` argument. It is read in two steps:
//!
//! - A *view* ([`View`], [`Repo::fragment_view`]): every node in id order
//!   with integer features (kind, file type, length) and the whole
//!   adjacency as offsets and ordinals, nothing truncated. The view is a
//!   function of the commit *and the ledger* (which render a file is read
//!   through is ledger state, and the ledger can gain derivations for the
//!   same file later), so it is not a pure derivation of the commit: it is
//!   kept as a recorded step, `fragment-view(pin)`, whose pin is a small
//!   text naming the commit and its tree. [`Repo::check_fragment_view`]
//!   rebuilds it from that commit and compares.
//! - *Batches* ([`Spec`], [`Sampler`]): fixed-fanout, layer-by-layer
//!   samples from seeded roots, integers only. Each layer's frontier is
//!   sorted and deduplicated before it is read, so a hub reached from many
//!   parents is read once per layer and sampled once. A batch is a pure
//!   function of the view's bytes and the spec's text: the builtin
//!   `sample(view, spec)` ([`crate::func::sample`]), recorded with `apply`
//!   like any derivation and rebuilt by repack or fsck anywhere.
//!
//! [`Sampler::stream`] hands batches to an outside consumer (a trainer)
//! over a bounded channel. Nothing in the default paths calls this module.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, sync_channel};

use crate::{Arg, Error, Id, Kind, Object, Repo, Store, func};

/// Leading bytes of a view's encoding.
pub const VIEW_MAGIC: &[u8] = b"ikam-fragment-view/1\n";
/// Leading bytes of a batch's encoding.
pub const BATCH_MAGIC: &[u8] = b"ikam-sample-batch/1\n";
/// Leading bytes of a view's pin (the commit it was read from).
pub const PIN_MAGIC: &[u8] = b"ikam-view-pin/1\n";

/// A node the commit's tree names as a file.
pub const FILE: u8 = 0;
/// A fragment that is not itself a render.
pub const FRAGMENT: u8 = 1;
/// A fragment that renders other fragments (a run, or a level of
/// `put_cut`'s tree).
pub const RENDER: u8 = 2;
/// The label of a node that is not a file.
pub const NO_LABEL: u16 = u16::MAX;

/// The fragment graph under one commit, integers only (see the module).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub commit: Id,
    pub tree: Id,
    /// File types (extensions, lower case), sorted; a file's label indexes it.
    pub labels: Vec<String>,
    /// Node ids, sorted: a node's ordinal is its position here.
    pub ids: Vec<Id>,
    pub kind: Vec<u8>,
    pub label: Vec<u16>,
    pub len: Vec<u64>,
    /// Adjacency offsets (`nodes + 1`) into `adj`.
    pub off: Vec<u32>,
    /// Neighbor ordinals, each node's sorted and without repeats.
    pub adj: Vec<u32>,
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], Error> {
        if self.0.len() < n {
            return Err(Error::Decode("sampler: truncated"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn id(&mut self) -> Result<Id, Error> {
        Ok(Id::from_bytes(self.take(32)?.try_into().unwrap()))
    }
}

impl View {
    pub fn nodes(&self) -> usize {
        self.ids.len()
    }

    pub fn neighbors(&self, v: u32) -> &[u32] {
        let v = v as usize;
        &self.adj[self.off[v] as usize..self.off[v + 1] as usize]
    }

    pub fn ordinal(&self, id: &Id) -> Option<u32> {
        self.ids.binary_search(id).ok().map(|k| k as u32)
    }

    /// Undirected edges stored (each counted once).
    pub fn edges(&self) -> usize {
        self.adj.len() / 2
    }

    pub fn encode(&self) -> Vec<u8> {
        let n = self.ids.len();
        let mut b = Vec::with_capacity(96 + n * 51 + self.adj.len() * 4);
        b.extend_from_slice(VIEW_MAGIC);
        b.extend_from_slice(self.commit.as_bytes());
        b.extend_from_slice(self.tree.as_bytes());
        for x in [n, self.adj.len(), self.labels.len()] {
            b.extend_from_slice(&(x as u32).to_le_bytes());
        }
        for l in &self.labels {
            b.extend_from_slice(&(l.len() as u16).to_le_bytes());
            b.extend_from_slice(l.as_bytes());
        }
        for id in &self.ids {
            b.extend_from_slice(id.as_bytes());
        }
        b.extend_from_slice(&self.kind);
        for l in &self.label {
            b.extend_from_slice(&l.to_le_bytes());
        }
        for l in &self.len {
            b.extend_from_slice(&l.to_le_bytes());
        }
        for o in self.off.iter().chain(&self.adj) {
            b.extend_from_slice(&o.to_le_bytes());
        }
        b
    }

    pub fn decode(b: &[u8]) -> Result<View, Error> {
        let mut r = Reader(b);
        if r.take(VIEW_MAGIC.len())? != VIEW_MAGIC {
            return Err(Error::Decode("fragment view: magic"));
        }
        let (commit, tree) = (r.id()?, r.id()?);
        let (n, m, nl) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        let mut labels = Vec::with_capacity(nl);
        for _ in 0..nl {
            let k = r.u16()? as usize;
            let s = std::str::from_utf8(r.take(k)?)
                .map_err(|_| Error::Decode("fragment view: label"))?;
            labels.push(s.to_string());
        }
        let ids = (0..n).map(|_| r.id()).collect::<Result<_, _>>()?;
        let kind = r.take(n)?.to_vec();
        let label = (0..n).map(|_| r.u16()).collect::<Result<_, _>>()?;
        let len = (0..n).map(|_| r.u64()).collect::<Result<_, _>>()?;
        let off: Vec<u32> = (0..=n).map(|_| r.u32()).collect::<Result<_, _>>()?;
        let adj: Vec<u32> = (0..m).map(|_| r.u32()).collect::<Result<_, _>>()?;
        let ok = r.0.is_empty()
            && off.first() == Some(&0)
            && off.last().map(|&o| o as usize) == Some(m)
            && off.windows(2).all(|w| w[0] <= w[1])
            && adj.iter().all(|&v| (v as usize) < n);
        if !ok {
            return Err(Error::Decode("fragment view: adjacency"));
        }
        Ok(View {
            commit,
            tree,
            labels,
            ids,
            kind,
            label,
            len,
            off,
            adj,
        })
    }
}

/// Which nodes a batch's roots are drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Targets {
    All,
    Files,
}

/// One batch to sample: roots `index` of the seeded order of targets
/// (`batch` a batch, a new order each epoch), `hops` layers of at most
/// `fanout` neighbors each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub seed: u64,
    pub fanout: u32,
    pub hops: u32,
    pub batch: u32,
    pub index: u32,
    pub targets: Targets,
}

impl Spec {
    /// Canonical text: the content a batch's derivation names.
    pub fn to_text(&self) -> String {
        let t = match self.targets {
            Targets::All => "all",
            Targets::Files => "files",
        };
        format!(
            "ikam-sample/1 seed={} fanout={} hops={} batch={} index={} targets={t}\n",
            self.seed, self.fanout, self.hops, self.batch, self.index
        )
    }

    pub fn parse(text: &str) -> Result<Spec, Error> {
        let bad = || Error::Exec(format!("sample: spec {text:?}"));
        let mut parts = text.split_whitespace();
        if parts.next() != Some("ikam-sample/1") {
            return Err(bad());
        }
        let kv: BTreeMap<&str, &str> = parts.filter_map(|p| p.split_once('=')).collect();
        let num = |k: &str| -> Result<u64, Error> {
            kv.get(k).ok_or_else(bad)?.parse().map_err(|_| bad())
        };
        let small = |k: &str| -> Result<u32, Error> { num(k)?.try_into().map_err(|_| bad()) };
        let spec = Spec {
            seed: num("seed")?,
            fanout: small("fanout")?,
            hops: small("hops")?,
            batch: small("batch")?,
            index: small("index")?,
            targets: match kv.get("targets") {
                Some(&"all") => Targets::All,
                Some(&"files") => Targets::Files,
                _ => return Err(bad()),
            },
        };
        if spec.to_text() != text || spec.batch == 0 || spec.hops > 8 || spec.fanout == 0 {
            return Err(bad());
        }
        Ok(spec)
    }
}

/// What sampling a batch read: adjacency lists read (one per distinct
/// frontier node a layer) against the parent-to-node occurrences that would
/// each read one without the sorted frontier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub reads: usize,
    pub occurrences: usize,
    pub edges: usize,
    pub nodes: usize,
}

/// A decoded batch: the roots and each later layer (node ordinals, sorted),
/// and each hop's sampled edges as positions in its two layers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub view: Id,
    pub seed: u64,
    pub fanout: u32,
    pub hops: u32,
    pub batch: u32,
    pub index: u32,
    pub epoch: u32,
    pub layers: Vec<Vec<u32>>,
    pub edges: Vec<Vec<(u32, u32)>>,
}

impl Batch {
    /// The node records a batch carries per layer entry are skipped here:
    /// they are the view's, and the view is at hand when this is called.
    pub fn decode(b: &[u8]) -> Result<Batch, Error> {
        let mut r = Reader(b);
        if r.take(BATCH_MAGIC.len())? != BATCH_MAGIC {
            return Err(Error::Decode("sample batch: magic"));
        }
        let view = r.id()?;
        let seed = r.u64()?;
        let (fanout, hops, batch, index, epoch) =
            (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let mut layers = Vec::new();
        for _ in 0..=hops {
            let n = r.u32()? as usize;
            let mut l = Vec::with_capacity(n);
            for _ in 0..n {
                l.push(r.u32()?);
                r.take(8)?;
            }
            layers.push(l);
        }
        let mut edges = Vec::new();
        for _ in 0..hops {
            let n = r.u32()? as usize;
            edges.push(
                (0..n)
                    .map(|_| Ok((r.u32()?, r.u32()?)))
                    .collect::<Result<_, Error>>()?,
            );
        }
        if !r.0.is_empty() {
            return Err(Error::Decode("sample batch: trailing bytes"));
        }
        Ok(Batch {
            view,
            seed,
            fanout,
            hops,
            batch,
            index,
            epoch,
            layers,
            edges,
        })
    }

    /// Each sampled edge as a pair of node ordinals (parent, neighbor).
    pub fn edge_ordinals(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.edges.iter().enumerate().flat_map(move |(h, es)| {
            es.iter()
                .map(move |&(s, d)| (self.layers[h][s as usize], self.layers[h + 1][d as usize]))
        })
    }
}

/// splitmix64's finalizer: the sampler's only source of randomness, keyed
/// by counters, so a choice never depends on the order work is done in.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn key(parts: &[u64]) -> u64 {
    parts.iter().fold(0x6b75_6d6f_7361_6d70, |h, &p| mix(h ^ p))
}

/// A value in `0..=hi` from `r`.
fn below(r: u64, hi: u64) -> u64 {
    ((u128::from(r) * (u128::from(hi) + 1)) >> 64) as u64
}

/// Position `i` of a seeded permutation of `0..n`: a four-round Feistel
/// network over the smallest even number of bits that holds `n`, walked
/// until it lands below `n` (fewer than four steps expected), so a batch's
/// roots cost `O(batch)`, not a shuffle of every target.
fn permute(i: u64, n: u64, seed: u64, epoch: u64) -> u64 {
    if n < 2 {
        return 0;
    }
    let bits = 64 - (n - 1).leading_zeros();
    let half = bits.div_ceil(2);
    let mask = (1u64 << half) - 1;
    let mut x = i;
    loop {
        let (mut l, mut r) = (x >> half, x & mask);
        for round in 0..4 {
            let f = key(&[seed, 1, epoch, round, r]) & mask;
            (l, r) = (r, l ^ f);
        }
        x = (l << half) | r;
        if x < n {
            return x;
        }
    }
}

/// The bits of a length (0 for 0): a size feature in one byte.
fn lenlog(len: u64) -> u8 {
    (64 - len.leading_zeros()) as u8
}

/// One streamed batch: its spec, its bytes and what sampling it read.
pub type Sampled = Result<(Spec, Vec<u8>, Stats), Error>;

/// Samples batches from one view (parsed once).
pub struct Sampler {
    view: View,
    view_id: Id,
    files: Vec<u32>,
}

impl Sampler {
    pub fn new(view_bytes: &[u8]) -> Result<Sampler, Error> {
        let view = View::decode(view_bytes)?;
        let files = (0..view.nodes() as u32)
            .filter(|&v| view.kind[v as usize] == FILE)
            .collect();
        Ok(Sampler {
            view,
            view_id: Id::of_content(view_bytes),
            files,
        })
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn view_id(&self) -> Id {
        self.view_id
    }

    fn target_count(&self, t: Targets) -> usize {
        match t {
            Targets::All => self.view.nodes(),
            Targets::Files => self.files.len(),
        }
    }

    /// Batches a pass over the targets takes.
    pub fn batches_per_epoch(&self, spec: &Spec) -> u32 {
        (self.target_count(spec.targets) as u32)
            .div_ceil(spec.batch)
            .max(1)
    }

    /// Sample one batch: its encoding and what it read.
    pub fn sample(&self, spec: &Spec) -> Result<(Vec<u8>, Stats), Error> {
        let v = &self.view;
        let per = self.batches_per_epoch(spec);
        let (epoch, pos) = (spec.index / per, spec.index % per);
        // The epoch's order of targets: a seeded permutation, evaluated
        // only at this batch's positions.
        let n = self.target_count(spec.targets) as u64;
        let start = (u64::from(pos) * u64::from(spec.batch)).min(n);
        let end = (start + u64::from(spec.batch)).min(n);
        let mut roots: Vec<u32> = (start..end)
            .map(|i| {
                let k = permute(i, n, spec.seed, u64::from(epoch)) as usize;
                match spec.targets {
                    Targets::All => k as u32,
                    Targets::Files => self.files[k],
                }
            })
            .collect();
        roots.sort_unstable();
        let mut layers = vec![roots];
        let mut edges: Vec<Vec<(u32, u32)>> = Vec::new();
        let mut stats = Stats::default();
        let fanout = spec.fanout as usize;
        let mut chosen = Vec::with_capacity(fanout);
        for h in 0..spec.hops {
            // The frontier is sorted and has no repeats: each node's
            // adjacency is read, and sampled, once a layer.
            let frontier = &layers[h as usize];
            let mut hop = Vec::new();
            for (p, &node) in frontier.iter().enumerate() {
                let nb = v.neighbors(node);
                stats.reads += 1;
                chosen.clear();
                if nb.len() <= fanout {
                    chosen.extend(0..nb.len() as u64);
                } else {
                    // Floyd's algorithm: `fanout` distinct positions.
                    let deg = nb.len() as u64;
                    for (k, j) in (deg - fanout as u64..deg).enumerate() {
                        let r = key(&[
                            spec.seed,
                            2,
                            u64::from(spec.index),
                            u64::from(h),
                            u64::from(node),
                            k as u64,
                        ]);
                        let t = below(r, j);
                        let pick = if chosen.contains(&t) { j } else { t };
                        chosen.push(pick);
                    }
                    chosen.sort_unstable();
                }
                for &c in &chosen {
                    hop.push((p as u32, nb[c as usize]));
                }
            }
            let mut next: Vec<u32> = hop.iter().map(|e| e.1).collect();
            next.sort_unstable();
            next.dedup();
            stats.occurrences += hop.len();
            let hop = hop
                .into_iter()
                .map(|(p, n)| (p, next.binary_search(&n).unwrap() as u32))
                .collect();
            edges.push(hop);
            layers.push(next);
        }
        stats.occurrences += layers[0].len();
        stats.edges = edges.iter().map(Vec::len).sum();
        stats.nodes = layers.iter().map(Vec::len).sum();
        // Encode: header, each layer's node records, each hop's edges.
        let mut b = Vec::with_capacity(64 + stats.nodes * 12 + stats.edges * 8);
        b.extend_from_slice(BATCH_MAGIC);
        b.extend_from_slice(self.view_id.as_bytes());
        b.extend_from_slice(&spec.seed.to_le_bytes());
        for x in [spec.fanout, spec.hops, spec.batch, spec.index, epoch] {
            b.extend_from_slice(&x.to_le_bytes());
        }
        for layer in &layers {
            b.extend_from_slice(&(layer.len() as u32).to_le_bytes());
            for &n in layer {
                let k = n as usize;
                b.extend_from_slice(&n.to_le_bytes());
                b.push(v.kind[k]);
                b.extend_from_slice(&v.label[k].to_le_bytes());
                b.push(lenlog(v.len[k]));
                b.extend_from_slice(&(v.off[k + 1] - v.off[k]).to_le_bytes());
            }
        }
        for hop in &edges {
            b.extend_from_slice(&(hop.len() as u32).to_le_bytes());
            for &(s, d) in hop {
                b.extend_from_slice(&s.to_le_bytes());
                b.extend_from_slice(&d.to_le_bytes());
            }
        }
        Ok((b, stats))
    }

    /// Sample `specs` on a thread of its own, handing each batch to the
    /// receiver over a channel that holds at most `capacity` batches, so a
    /// slow consumer holds back the sampler rather than memory growing.
    pub fn stream(self: Arc<Self>, specs: Vec<Spec>, capacity: usize) -> Receiver<Sampled> {
        let (tx, rx) = sync_channel(capacity);
        std::thread::spawn(move || {
            for spec in specs {
                let out = self.sample(&spec).map(|(b, s)| (spec, b, s));
                let stop = out.is_err();
                if tx.send(out).is_err() || stop {
                    return;
                }
            }
        });
        rx
    }
}

/// `sample(view, spec)`: see [`crate::func::sample`].
pub(crate) fn sample_builtin(args: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
    let [view, spec] = args else {
        return Err(Error::Exec(
            "sample: two arguments, a view and a spec".into(),
        ));
    };
    let spec = std::str::from_utf8(spec).map_err(|_| Error::Exec("sample: spec".into()))?;
    let spec = Spec::parse(spec)?;
    Ok(Sampler::new(view)?.sample(&spec)?.0)
}

/// The commit (and tree) a pin names, if `bytes` is a pin.
pub fn parse_pin(bytes: &[u8]) -> Option<(Id, Id)> {
    let text = std::str::from_utf8(bytes.strip_prefix(PIN_MAGIC)?).ok()?;
    let mut lines = text.lines();
    let commit = lines.next()?.strip_prefix("commit ")?.parse().ok()?;
    let tree = lines.next()?.strip_prefix("tree ")?.parse().ok()?;
    lines.next().is_none().then_some((commit, tree))
}

fn label_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

impl<S: Store> Repo<S> {
    /// The fragment graph under `commit` (a commit or a tree) as a view: see
    /// the module. Every render argument is an edge, nothing truncated.
    pub fn fragment_view(&self, commit: &Id) -> Result<View, Error> {
        let tree = crate::snapshot::tree_of(self, commit)?;
        // Files in path order; a content at several paths takes the label
        // of its first.
        let mut files: Vec<(String, Id)> = Vec::new();
        let mut stack = vec![(String::new(), tree)];
        while let Some((prefix, t)) = stack.pop() {
            let Object::Tree(entries) = self.get(&t)? else {
                return Err(Error::WrongKind(t));
            };
            for e in entries {
                let path = if prefix.is_empty() {
                    e.name.clone()
                } else {
                    format!("{prefix}/{}", e.name)
                };
                match e.kind {
                    Kind::Tree => stack.push((path, e.id)),
                    Kind::File => files.push((path, e.id)),
                    Kind::Graph => {}
                }
            }
        }
        files.sort();
        let labels: Vec<String> = files
            .iter()
            .map(|(p, _)| label_of(p))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut nodes: BTreeMap<Id, (u8, u16)> = BTreeMap::new();
        for (path, id) in &files {
            let l = labels.binary_search(&label_of(path)).unwrap() as u16;
            nodes.entry(*id).or_insert((FILE, l));
        }
        // Renders, followed down to their leaves.
        let mut pairs: Vec<(Id, Id)> = Vec::new();
        let mut stack: Vec<Id> = nodes.keys().copied().collect();
        let mut seen: std::collections::HashSet<Id> = stack.iter().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(frags) = self.fragments_of(&id)? else {
                continue;
            };
            let e = nodes.entry(id).or_insert((RENDER, NO_LABEL));
            if e.0 == FRAGMENT {
                e.0 = RENDER;
            }
            for f in frags {
                pairs.push((id, f));
                if seen.insert(f) {
                    nodes.entry(f).or_insert((FRAGMENT, NO_LABEL));
                    stack.push(f);
                }
            }
        }
        let ids: Vec<Id> = nodes.keys().copied().collect();
        let ord: HashMap<Id, u32> = ids
            .iter()
            .enumerate()
            .map(|(k, id)| (*id, k as u32))
            .collect();
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); ids.len()];
        for (a, b) in pairs {
            let (a, b) = (ord[&a], ord[&b]);
            if a != b {
                lists[a as usize].push(b);
                lists[b as usize].push(a);
            }
        }
        let (mut off, mut adj) = (Vec::with_capacity(ids.len() + 1), Vec::new());
        off.push(0u32);
        for mut l in lists {
            l.sort_unstable();
            l.dedup();
            adj.extend(l);
            off.push(adj.len() as u32);
        }
        let mut len = Vec::with_capacity(ids.len());
        for id in &ids {
            len.push(self.read_content(id)?.len() as u64);
        }
        let commit = match self.get(commit)? {
            Object::Commit(_) => *commit,
            _ => Id::from_bytes([0; 32]),
        };
        Ok(View {
            commit,
            tree,
            labels,
            kind: ids.iter().map(|id| nodes[id].0).collect(),
            label: ids.iter().map(|id| nodes[id].1).collect(),
            ids,
            len,
            off,
            adj,
        })
    }

    /// The pin a view's recorded step names: the commit and its tree, as
    /// content.
    pub fn view_pin(&mut self, commit: &Id) -> Result<Id, Error> {
        let tree = crate::snapshot::tree_of(self, commit)?;
        let mut text = PIN_MAGIC.to_vec();
        text.extend_from_slice(format!("commit {commit}\ntree {tree}\n").as_bytes());
        Ok(self.put_content(&text)?.id)
    }

    /// Build the view of `commit` and keep it as a recorded step
    /// `fragment-view(pin)`. Returns the view's id and the record's id.
    pub fn record_fragment_view(&mut self, commit: &Id) -> Result<(Id, Id), Error> {
        let bytes = self.fragment_view(commit)?.encode();
        let pin = self.view_pin(commit)?;
        self.record_step(
            func::fragment_view(),
            vec![Arg::Whole(pin)],
            Some(&bytes),
            None,
            crate::step::Design::Run,
        )
    }

    /// Whether the view `view` is still what its commit gives: rebuilt
    /// from the commit its bytes name and compared. False when the ledger
    /// has since changed which render a file is read through.
    pub fn check_fragment_view(&self, view: &Id) -> Result<bool, Error> {
        let v = View::decode(&self.read_content(view)?)?;
        let rev = if v.commit == Id::from_bytes([0; 32]) {
            v.tree
        } else {
            v.commit
        };
        Ok(Id::of_content(&self.fragment_view(&rev)?.encode()) == *view)
    }

    /// Sample one batch as a pure derivation, `sample(view, spec)`.
    pub fn apply_sample(&mut self, view: Id, spec: &Spec) -> Result<Id, Error> {
        let spec = self.put_content(spec.to_text().as_bytes())?.id;
        self.apply(func::sample(), vec![Arg::Whole(view), Arg::Whole(spec)])
    }
}
