//! Chunk bounds picked for a net from its size and the read weight (E453,
//! opt-in; off by default).
//!
//! E448 made the prolly tree's chunk bounds settable (`Repo::graph_chunks`),
//! and E451 found that no one setting suits every net: a net of 1,000
//! transitions pays 1.6x the text file's bytes in its first version under
//! the default 4 KiB to 32 KiB bounds (each chunk compresses alone), while
//! larger leaves slow every later write (an edit rewrites a larger leaf),
//! and small leaves win only at the heavy read weight. With
//! [`Repo::net_chunk_policy`] set, the bounds of a net in the `Tree` and
//! `Inline` layouts are picked by a [`NetChunkPolicy`] from the encoded
//! bytes of its nodes and the read weight when the net is first written,
//! and kept for every later version of it: a graph must be edited with the
//! bounds it was cut with, and a version cut with other bounds shares no
//! chunk with the one before. Nothing about a chunk's stored form changes;
//! only which valid tree of the same nodes is written.
//!
//! The bounds are kept per net name in the handle. A handle that writes a
//! new version into a tree that already holds the net
//! ([`Repo::put_net_fragment_file`]) finds the bounds the stored version was
//! cut with among the policy's (and `graph_chunks`) by recomputing its
//! root under each, so the history keeps its bounds across handles too.

use super::{Layout, Net};
use crate::graph::{ChunkSizes, root_under};
use crate::{Error, Id, Repo, Store};

/// How [`Repo::net_chunk_policy`] picks a net's chunk bounds (E453).
#[derive(Clone, Debug, PartialEq)]
pub struct NetChunkPolicy {
    /// At a read weight under `heavy_from`: `(most bytes, bounds)` steps in
    /// ascending order of bytes. A net whose nodes encode to at most a
    /// step's bytes gets the first such step's bounds; a net larger than
    /// every step gets `graph_chunks`.
    pub steps: Vec<(usize, ChunkSizes)>,
    /// The read weight from which `heavy` is used in place of `steps`
    /// (`f64::INFINITY`: never).
    pub heavy_from: f64,
    /// Steps as `steps`, at a read weight of `heavy_from` or more.
    pub heavy: Vec<(usize, ChunkSizes)>,
}

impl NetChunkPolicy {
    /// The size a policy picks by: the encoded bytes of `net`'s nodes in
    /// `layout`, as chunk boundaries count them.
    pub fn net_bytes(net: &Net, layout: Layout) -> usize {
        super::fragments::node_bytes(net, layout)
    }

    /// E453's measured best ("C"): leaves scaled to the net, about six of
    /// them. A net of up to 1 MiB of nodes gets leaves whose least size is
    /// a sixth of its bytes, from 4 to 8 KiB in whole KiB, and whose most
    /// is eight times that, with parents of 16 to 128 KiB; a larger net
    /// gets `graph_chunks`. The same at every read weight.
    pub fn by_size() -> NetChunkPolicy {
        let mut steps: Vec<(usize, ChunkSizes)> = Vec::new();
        for k in 1..=1024usize {
            let min = ((k << 10) / 6).clamp(4 << 10, 8 << 10) >> 10 << 10;
            let b = ChunkSizes {
                leaf: (min, min * 8),
                parent: (16 << 10, 128 << 10),
            };
            match steps.last_mut() {
                Some(s) if s.1 == b => s.0 = k << 10,
                _ => steps.push((k << 10, b)),
            }
        }
        NetChunkPolicy {
            steps,
            heavy_from: f64::INFINITY,
            heavy: Vec::new(),
        }
    }

    /// One step for every size: `bounds` at every weight.
    pub fn fixed(bounds: ChunkSizes) -> NetChunkPolicy {
        NetChunkPolicy {
            steps: vec![(usize::MAX, bounds)],
            heavy_from: f64::INFINITY,
            heavy: Vec::new(),
        }
    }

    /// The bounds for a net whose nodes encode to `bytes`, at `weight`;
    /// `default` (the handle's `graph_chunks`) above every step.
    pub fn pick(&self, bytes: usize, weight: f64, default: ChunkSizes) -> ChunkSizes {
        let steps = match weight >= self.heavy_from {
            true => &self.heavy,
            false => &self.steps,
        };
        (steps.iter())
            .find(|(most, _)| bytes <= *most)
            .map_or(default, |s| s.1)
    }

    /// Every bounds the policy can pick, `default` last.
    pub fn candidates(&self, default: ChunkSizes) -> Vec<ChunkSizes> {
        let mut out: Vec<ChunkSizes> = Vec::new();
        for (_, b) in self.steps.iter().chain(&self.heavy) {
            if !out.contains(b) {
                out.push(*b);
            }
        }
        if !out.contains(&default) {
            out.push(default);
        }
        out
    }
}

/// Whether `layout` is cut by the prolly tree (the flat layouts are one
/// chunk whatever the bounds).
fn cut_layout(layout: Layout) -> bool {
    matches!(layout, Layout::Tree | Layout::Inline)
}

impl<S: Store> Repo<S> {
    /// The bounds the net named `name` was first written with under
    /// [`net_chunk_policy`](Repo::net_chunk_policy) in this handle (or found
    /// for it by [`put_net_fragment_file`](Repo::put_net_fragment_file)),
    /// which every later version of it is cut with.
    pub fn net_chunk_bounds(&self, name: &str) -> Option<ChunkSizes> {
        self.net_bounds.get(name).copied()
    }

    /// Write `net` with `write` under the bounds its policy gives: the
    /// kept bounds of its name, else the policy's pick for its size, kept
    /// once the write succeeds. Without a policy, or in a flat layout,
    /// `write` runs as it is.
    pub(super) fn with_net_bounds(
        &mut self,
        net: &Net,
        layout: Layout,
        write: impl FnOnce(&mut Self) -> Result<Id, Error>,
    ) -> Result<Id, Error> {
        let Some(policy) = self.net_chunk_policy.clone() else {
            return write(self);
        };
        if !cut_layout(layout) {
            return write(self);
        }
        // Kept bounds; else those of a version this handle wrote before the
        // policy was set, kept for delta writes; else the policy's pick.
        let known = (self.net_bounds.get(&net.name).copied())
            .or_else(|| self.last_net_chunks(&net.name, layout));
        let bounds = match known {
            Some(b) => b,
            None => {
                net.check()?;
                let bytes = NetChunkPolicy::net_bytes(net, layout);
                policy.pick(bytes, self.read_weight, self.graph_chunks)
            }
        };
        let saved = std::mem::replace(&mut self.graph_chunks, bounds);
        let out = write(self);
        self.graph_chunks = saved;
        if out.is_ok() {
            self.net_bounds.insert(net.name.clone(), bounds);
        }
        out
    }

    /// With a policy and no bounds kept for `net`'s name: when `previous`
    /// (the version a tree holds at the net's path) was cut with one of the
    /// bounds the policy can pick, or with `graph_chunks`, keep those, so
    /// the new version is cut as the history was. The policy's own pick for
    /// the new net is tried first, so a graph that several bounds give
    /// (one chunk) keeps the pick.
    pub(super) fn find_net_bounds(
        &mut self,
        net: &Net,
        layout: Layout,
        previous: &Id,
    ) -> Result<(), Error> {
        let Some(policy) = self.net_chunk_policy.clone() else {
            return Ok(());
        };
        if !cut_layout(layout) || self.net_bounds.contains_key(&net.name) {
            return Ok(());
        }
        let nodes = self.graph_nodes(previous)?;
        net.check()?;
        let pick = policy.pick(
            NetChunkPolicy::net_bytes(net, layout),
            self.read_weight,
            self.graph_chunks,
        );
        let mut tries = vec![pick];
        tries.extend(policy.candidates(self.graph_chunks));
        if let Some(b) = tries
            .into_iter()
            .find(|b| root_under(&nodes, *b) == *previous)
        {
            self.net_bounds.insert(net.name.clone(), b);
        }
        Ok(())
    }
}
