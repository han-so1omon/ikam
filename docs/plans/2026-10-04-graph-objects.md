# Graph objects: cyclic graphs in a content-addressed kernel

Status: design, not implemented. Decided with the user: option 2 of three (a graph as one object). Claims stay as they are.

## Why

Hash-linked objects cannot form cycles. An id hashes the ids an object points to, so two objects pointing at each other would each need the other's hash first. That is also what makes reconstruction guaranteed: the rebuild path (derivations) is a DAG, and `rebuild` already rejects cycles.

Semantic structure is often cyclic: mutual references, citations, entity relations, state machines, Petri nets. Today such cycles exist only in claims, which are separate records and not one versioned value. A graph object makes a whole cyclic graph one immutable, versioned, shareable value, without touching the rebuild path.

## The object

A new kind `G`, next to `T` (tree), `C`, `D` and `L`. Edges refer to nodes by position, so they may form any cycle, including self-loops. The id is `BLAKE3(canonical)` like every object.

```
graph = "G" nnodes:u32 { node } nedges:u32 { edge }
node  = label_len:u32 label target
target = 0                     (no target: a pure node)
       | 1 arg                 (content: whole, or a byte range; the `arg` encoding of derivations)
       | 2 id[32]              (another object: a tree, a graph, a commit, a derivation record)
edge  = from:u32 to:u32 label_len:u32 label
```

- **Canonical order (identity independent of insertion order).** Nodes are strictly ascending by `(label, target)`, so node keys are unique, as tree names are. Edges are strictly ascending by `(from, to, label)` and refer to positions in that order. With unique keys, two encodings of the same labelled graph are byte-identical, so equal graphs dedup. General graph canonicalisation (isomorphism) is not needed; uniqueness of keys is what makes it cheap.
- **Labels are free text.** There are no enums, per AGENTS.md section 3. A predicate is an edge label.
- **Strict decoding**, as for every object: indices in range, order strict, no trailing bytes.

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

1. `decode(encode(g)) == g`, and decoding rejects out-of-range indices, unsorted nodes or edges, duplicate node keys, and trailing bytes.
2. Building the same labelled graph from nodes and edges in any insertion order gives the same id.
3. A cycle (including a self-loop) round-trips through put, get, gc, repack and fsck.
4. gc keeps every node target of a reachable graph live; it never follows edges.
5. A graph in a commit is restored exactly after repack, whatever storage form it takes.

## Not in scope (open questions)

- **Functions over graphs.** May a graph be an argument of a derivation (e.g. a WASM function that renders it)? It would have to be passed as its canonical bytes. Deferred until there is a use.
- **Edge weights or multi-edges.** Weights can live in labels or claims for now. Parallel edges with the same label are excluded by the strict order.
- **Hyperedges and Petri nets.** A Petri net fits as a bipartite graph (place and transition nodes); markings are content targets. No special support is planned until a use exists.
- **Benchmark.** A graph corpus (for example the claims of `examples/relatedness.rs` frozen into graphs across versions) would measure the storage cost. Without one, this is a capability with laws, not a score change.

## Steps

1. `object.rs`: `Object::Graph`, encode/decode, `Object::graph(nodes, edges)` that sorts nodes and remaps edge indices; links. Laws 1 and 2.
2. Tree entry kind `G`; reachability and gc; `exec` links. Laws 3 and 4.
3. Storage: graphs use the same encodings as trees (content form, abbreviation). Law 5.
4. Optional: a graph corpus in the benchmark, as a benchmark-change log entry.
