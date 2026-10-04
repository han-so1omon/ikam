# Graph objects: cyclic graphs in a content-addressed kernel

Status: design, not implemented. Decided with the user: option 2 of three (a graph as one object, sliceable into many). Claims stay as they are. Revised after a probe: edges reference nodes by key, not position.

## Why

Hash-linked objects cannot form cycles. An id hashes the ids an object points to, so two objects pointing at each other would each need the other's hash first. That is also what makes reconstruction guaranteed: the rebuild path (derivations) is a DAG, and `rebuild` already rejects cycles.

Semantic structure is often cyclic: mutual references, citations, entity relations, state machines, Petri nets. Today such cycles exist only in claims, which are separate records and not one versioned value. A graph object makes a whole cyclic graph one immutable, versioned, shareable value, without touching the rebuild path.

## The object

A new kind `G`, next to `T` (tree), `C`, `D` and `L`. Each node carries its own outgoing edges, and an edge names its target node by a **key**, not by position. Edges may form any cycle, including self-loops. The id is `BLAKE3(canonical)`, like every object.

```
graph  = "G" nnodes:u32 { node }
node   = label_len:u32 label target nedges:u32 { edge }
target = 0                     (no target: a pure node)
       | 1 arg                 (content: whole, or a byte range; the `arg` encoding of derivations)
       | 2 id[32]              (another object: a tree, a graph, a commit, a derivation record)
edge   = to:key[8] label_len:u32 label
key    = the first 8 bytes of BLAKE3(node label)
```

- **Keys, not positions (measured).** Positional edges would renumber on every node insertion, so successive versions would share almost nothing. In a probe (2,000 nodes, 6,000 edges, 20 versions of small edits, stored by the current kernel), positional edges stored 625 KB while keyed adjacency stored **77 KB**, about 1.6 versions' worth of zstd for the whole history. Keyed is larger for one version alone (47 KB vs 32 KB at zstd -19), so the choice is for history and sharing. Probe source: session scratchpad `graph_probe.rs`, not committed.
- **Canonical order (identity independent of insertion order).** Nodes are strictly ascending by label, so labels are unique within a graph, as tree names are. Each node's edges are strictly ascending by `(to, label)`. The same labelled graph therefore always has the same bytes and id, without general graph canonicalisation.
- **Key collisions.** Two node labels in one graph with the same 8-byte key are rejected at construction; the label must change or the graph must split. That takes ~2^32 labels by chance, or ~2^64 work deliberately.
- **Dangling keys are allowed.** An edge's key may name a node in another graph slice (see below). Resolution happens within a commit's set of graphs.
- **Labels are free text.** There are no enums, per AGENTS.md section 3. A predicate is an edge label.
- **Strict decoding**, as for every object: order strict, no trailing bytes.

## Semantic dedup: what the graph adds

The graph is the semantic layer *over* the existing dedup layers. It is not a replacement.

- **Nodes are semantic chunks.** A node can target a byte range of content (`1 arg`), so a graph marks where meaningful pieces start and end (a section, a clause, a cell) without fixing storage boundaries.
- **Edges propose dedup.** Edges such as "same-entity-as" or "revision-of" point the planner at pairs to try as slices, templates or deltas, which today come only from seed matching. Proposals still go through `put_derivation`: verified, then recorded only if they pay.
- **Claims measure.** Claims keep the measured weights (`gain_bits`), the mutable and many-author side. A graph is the curated, versioned side, and the two can be converted either way.
- **The graph itself dedups.** It is content to the kernel (slices, dictionary, abbreviated ids), so related graphs and versions share bytes (measured above).

## Slicing and version history

- **Slices are graphs.** The induced subgraph on a set of nodes is a `G` with those nodes and their edges (keys outside the set stay as dangling keys). It has its own id, so slices are shareable, and the same slice taken from two versions dedups to one object when unchanged.
- **Large graphs are a tree of slices.** A commit's graph can be split, like a directory tree, into slices by label prefix (or any partition) under a tree of `G` entries. A version then rewrites only the slices that changed, as nested trees do (E009). This is structural sharing without a new mechanism. Prolly trees (content-defined node boundaries, as in Dolt) are the alternative, pending the storage research.
- **History is commits.** Each version is a commit whose tree holds the graph slices. `log` gives history; time travel is reading an old commit. A diff is a merge-walk of two sorted node lists, skipping slices whose ids are equal.
- **Cycles across slices** are fine: they are dangling keys resolved within the commit, never hash links.

## How it fits the kernel

| concern | behaviour |
|---|---|
| reconstruction | unchanged. A graph is never a derivation input and never on the rebuild path, so its cycles cannot reach `rebuild`. |
| gc / reachability | node targets are links: gc keeps them live. A tree entry or commit may point at a graph (tree entry kind `G`). Edges are internal, so reachability never loops. |
| storage | like trees: stored as content when versions share it (E005), with abbreviated ids (E006/E010), compressed with the dictionary. A new version of a large graph is then mostly slices of the old one. |
| provenance | a node may target a derivation record (`2 id`), so a graph can describe how bytes were made ("ledger as a semantic graph") while the derivations remain the verified DAG that actually rebuilds them. |
| claims | stay separate: measured, mutable, many authors. A graph is a curated, versioned snapshot. One can be built from the other (e.g. `relate` output frozen into a graph). |
| exec | `links` and `used_by` list node targets; a graph's edges come from decoding it. |

## Laws (tests to write first)

1. `decode(encode(g)) == g`, and decoding rejects unsorted nodes or edges, duplicate labels or keys, and trailing bytes.
2. Building the same labelled graph from nodes and edges in any insertion order gives the same id.
3. A cycle (including a self-loop) round-trips through put, get, gc, repack and fsck.
4. gc keeps every node target of a reachable graph live; it never follows edges.
5. A graph in a commit is restored exactly after repack, whatever storage form it takes.
6. Slicing: the induced subgraph of an unchanged node set has the same id in two versions; a graph split into slices and reassembled is equal to the original.
7. Versions: storing 20 versions of a keyed graph costs less than 3 versions' worth of zstd (the probe's 1.6, with margin).

## Not in scope (open questions)

- **Functions over graphs.** May a graph be an argument of a derivation (e.g. a WASM function that renders it)? It would have to be passed as its canonical bytes. Deferred until there is a use.
- **Edge weights or multi-edges.** Weights can live in labels or claims for now. Parallel edges with the same label are excluded by the strict order.
- **Hyperedges and Petri nets.** A Petri net fits as a bipartite graph (place and transition nodes); markings are content targets. No special support is planned until a use exists.
- **Benchmark.** A graph corpus (for example the claims of `examples/relatedness.rs` frozen into graphs across versions) would measure the storage cost. Without one, this is a capability with laws, not a score change.

## Steps

1. `object.rs`: `Object::Graph`, encode/decode, `Object::graph(nodes, edges)` that sorts and checks keys; links. Laws 1 and 2.
2. Tree entry kind `G`; reachability and gc; `exec` links. Laws 3 and 4.
3. Storage: graphs use the same encodings as trees (content form, abbreviation). Law 5.
4. Slicing (induced subgraph, split by prefix, reassemble) and diff. Laws 6 and 7.
5. A graph corpus in the benchmark (versions of a real graph), as a benchmark-change log entry.

Storage engines compared in `docs/research/graph-storage.md`: none gives versioning, structural sharing and compact storage in an embeddable form, so the layer is our own, borrowing prolly-tree chunking (Noms/Dolt), add/remove layers (TerminusDB) and reference-compressed adjacency lists (WebGraph). Its hypotheses G1–G4 are open.
