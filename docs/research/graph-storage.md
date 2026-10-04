# Research note: storage for a versioned, sliceable, deduplicated graph layer (2026-10-04)

**[read]** means I read the text myself. **[inf]** means my own inference. Sources are cached in the session scratchpad (`papers/graph/`) and are not committed.

## Question

We want a graph layer in the kernel: labelled nodes (optionally pointing at byte ranges or other objects) and labelled edges, with cycles allowed. It must be (a) versioned (commits, diffs, time travel), (b) sliceable (extract and share subgraphs), (c) deduplicated across versions and across graphs, (d) compact on disk, and (e) able to support "semantic dedup", where relations let content be stored via other content. Should we use our own design, Apache HugeGraph, Red Planet Labs Rama, or something else? What mechanisms and numbers do these systems publish?

## Sources read

**1. Apache HugeGraph.** README, `hugegraph-server/README.md`, design-concept guide, config-option reference, and source files `RocksDBTables.java` and `BinarySerializer.java` (github.com/apache/hugegraph, hugegraph.apache.org). **Full text of these pages [read].**
- Backends: RocksDB (embedded, the default for standalone, "< 1TB") and HStore (Raft, distributed, "< 1000 TB"). HBase is deprecated and planned for removal in 2.0. MySQL, PostgreSQL, Cassandra, ScyllaDB and Palo are gone from mainline.
- Layout: separate RocksDB tables `g/vertex`, `g/edge_out` and `g/edge_in`, so each edge is stored twice, plus index tables.
  - EdgeId is `srcVertexId + edgeLabel + sortKey + tgtVertexId`; the current version also encodes direction. Re-inserting the same edge overwrites it, which is HugeGraph's only "dedup".
  - Vertex ids come from Snowflake, a primary key, or a custom id.
- Compression is RocksDB's: `compression_per_level` defaults to `[none, none, snappy×5]`, with 4 KB blocks and key prefix delta encoding (`block_restart_interval` 16).
- **Versioning:** none. The only temporal feature is per-element TTL (`expiredTime`). There are no commits, history or diff. "Snapshot" means only Raft log snapshots.
- No per-edge size numbers published. Maps to us: nothing beyond storing out- and in-adjacency separately.

**2. Red Planet Labs Rama.** Docs pages *Terminology*, *PStates*, *Depots*, *Why use Rama*, *All configs*, *Pricing*, *Downloads* (redplanetlabs.com/docs). **Full text [read].**
- Not a graph database: a JVM distributed application platform (Conductor, ZooKeeper Metastore, Supervisors; modules of depots, ETL topologies, PStates, query topologies).
- A *depot* is a "distributed, replicated, unindexed log". It is the event-sourced source of truth and keeps everything by default; trimming is opt-in (`depot.max.entries.per.partition`).
- A *PState* is a "distributed, durable, replicated datastore of arbitrary shape", made of nested maps, sets and lists.
  - "PStates with a top-level map in the schema use RocksDB as the underlying durable storage" (default: two-level index, 256 MB block cache).
- **No structural sharing:** "The same subindexed structure cannot be stored in multiple locations". This is enforced at runtime with a disk read.
- **No history:** PStates are mutable materialized views. History exists only as the depot log you replay. There is no as-of query on a PState.
- Licence: proprietary. Free for up to 2 nodes, $100/month for 3 nodes, then $1000 base + $100 per node.
- Maps to us: the depot → PState split equals our ledger → derived objects. Rama does not solve versioned storage of the derived state.

**3. van Otterdijk, Mendel-Gleason, Feeney, "Succinct Data Structures and Delta Encoding for Modern Databases", TerminusDB whitepaper, 14 Jan 2020.** Fetched from `raw.githubusercontent.com/terminusdb/terminusdb/v4.0.0/docs/whitepaper/terminusdb.pdf` (the asset URL now 404s). **Full text [read]**, plus terminusdb-store source `src/layer/internal/{base,child}.rs` [read].
- Data model: RDF triple sets, with ids from front-coded dictionaries (sorted strings that share prefixes; the position is the id). This is borrowed from HDT.
- Triples are stored as bit-sequence plus log-array adjacency (s→p, sp→o), with a wavelet tree for predicate-first access and an o→ps index.
- **Versioning:** a database is a chain of immutable *layers*, each named by a 20-byte id.
  - A child layer holds new dictionary entries, numbered from the parent's last id, plus *positive* and *negative* triple indexes (`pos_s_p_adjacency_list`, `neg_…` in `ChildLayer`).
  - A branch is a label file that points at a layer id. Many layers can share one parent.
- Deep chains slow queries, so layers can be *rolled up* into a delta-compressed or new base layer. Rolling up and deleting the old layers loses time travel.
- **The whitepaper gives no size-per-triple or benchmark numbers.**
- Licence and runtime: Apache-2.0. The store crate is Rust (`terminus-store`, `tdb-succinct`); the server is Prolog plus Rust.
- Maps to us: a layer is exactly "commit = delta object pointing at its parent". Positive and negative edge sets keep diff O(delta).

**4. HDT (Fernández, Martínez-Prieto, Gutiérrez, Polleres, Arias, J. Web Semantics 19, 2013), doi:10.1016/j.websem.2013.01.002.**
- **Not accessed:** ScienceDirect 403, ResearchGate 403, rdfhdt.org has an expired TLS certificate, and the Elsevier API was refused. I have only the title and metadata.
- Instead [read]: the W3C Member Submission (2011, w3.org/submissions/HDT) and Hernández-Illera et al. / HDTCat (arXiv:1809.06859, full text).
- Format: Header + Dictionary (shared subject-object section, so common S/O terms are stored once) + Triples. Triples can be "Compact" (two coordinated id streams) or "BitmapTriples" (bitmaps replace the subject column).
  - The spec default is 32-bit ids, with an option for log(n)-bit ids.
- HDTCat: "realistic to compress an RDF file from N-triples to HDT and gain a factor 10 in space."
  - HDTCat merges two HDT files *without* decompressing them (dictionary merge + id remap): LUBM 5.32 B triples in 84,633 s, peak 100.1 GB; the non-cat route topped out at 0.93 B triples on 128 GB.
- No bytes-per-triple figure from the original paper was available to me.
- Maps to us: merging immutable compressed graph slices by dictionary remapping is the operation behind "share part of a graph, combine later".

**5. Brisaboa, Ladra, Navarro, "k²-trees for Compact Web Graph Representation", SPIRE 2009.** users.dcc.uchile.cl/~gnavarro/ps/spire09.1.pdf. **Full text [read].**
- The adjacency matrix is split recursively into k² submatrices. One bit per submatrix says empty or non-empty. The levels are concatenated into bitmaps T (internal) and L (leaves).
  - Child i of x is at `rank(T,x)·k² + i`, with rank adding 5% extra space.
- The same structure answers direct *and reverse* neighbours.
- Results: **3.3–5.3 bits per edge** (UK 2002: 18.5 M pages, 298 M links, **4.22 bpe**), at 2–15 µs per neighbour.
  - EU graph, 2×2 tree: 5.21 bpe. 4×4: 7.22 bpe, faster.
- WebGraph and RePair win for forward-only navigation.

**6. Boldi & Vigna, "The WebGraph Framework I: Compression Techniques", WWW 2004.** vigna.di.unimi.it/ftp/papers/WebGraphI.pdf. **Full text [read].**
- **Gaps:** successor lists are sorted and stored as gaps with ζ codes. Gaps follow a power law because of locality.
- **Reference compression:** list S(x) is coded against a *reference list* S(x−r) chosen from the previous W lists. A copy-list (or run-length "copy blocks") says which reference successors to keep; the rest are "extra nodes". "Copying entirely a list costs one bit."
- **Intervals:** runs of consecutive ids of length ≥ L_min are stored as (left, length).
- **Chain bound R:** a maximum reference count caps decode depth. Lazy iterators cascade over references without expanding them.
- Numbers (Table 7, L_min = 3):

  | Graph | Setting | Bits/link | Avg chain |
  |---|---|---|---|
  | WebBase, 118 M nodes / 1 G links | W=7, R=∞ | **3.08** | 120 |
  | WebBase | R=3 | 3.74 | |
  | WebBase | R=1 | 4.17 | |
  | WebBase transpose | | 2.89 | |
  | .uk, 18.5 M nodes / 300 M links | W=7, R=∞ | 2.22 | |
  | .uk | R=3 | 3.00 | |

  (LINK database: 5.61 bits/link.)
- Maps to us: this is *semantic dedup of adjacency*. A node's edge list is stored as "the list of node y, minus some, plus a few". It is the graph form of our slice/template derivation.

**7. Prolly trees: Noms intro (`attic-labs/noms/doc/intro.md`) and Dolt docs "Prolly Tree" (docs.dolthub.com). Full text [read].**
- Content-defined chunking of a *sorted* key sequence. A boundary falls where the rolling hash matches (Noms: 12 high bits set, so an expected 4 KB chunk; 64-byte window). The tree recurses on the index of chunk addresses.
- **History independence:** the same set gives the same chunks and the same root hash, whatever the edit order. Diff walks only subtrees whose hashes differ, so cost is O(d), not O(n). Structural sharing is free.
- Noms: a boundary moves on ~1.6% of writes. Expected cost per write is "1.016 × treedepth" chunk writes. A 4-level tree holds 4096⁴ ≈ 281 TB, and one mutation costs about four 4 KB writes.
- Dolt's changes:
  - Noms' geometric chunk sizes gave many tiny and a few huge chunks, and buzhash did badly on low-entropy keys.
  - Dolt hashes **only keys** and uses a size-aware CDF split probability, so chunk sizes are roughly normal around 4 KB.
  - Fixed-size value edits do not re-chunk.
- Complexity table: random write `(1+k/w)·log_k n`, diff `d` (vs `n` for a B-tree).

**8. Datomic docs (*Introduction/Overview*, *Indexes*). docs.datomic.com. Full text [read].**
- An immutable, accumulate-only set of datoms `[e a v tx op]`. A retraction is a new datom.
- Four covering indexes: EAVT, AEVT, AVET (Cloud: all attributes; Pro: opted-in only) and VAET (reference attributes, i.e. reverse edges).
- Indexes are "shallow trees of segments, where each segment typically contains thousands of datoms". They are rebuilt "only occasionally, via background indexing jobs" and merged with an in-memory novelty set. A log gets O(1) storage writes per transaction.
- History: as-of / since / history database views. A cache miss is "1-2 segment fetches".
- JVM; "All editions … free, binaries licensed under Apache 2.0" (source not open). No bytes-per-datom figure found.

**9. XTDB README (github.com/xtdb/xtdb, MPL-2.0). README only [read].**
- An immutable, bitemporal (system time + valid time) SQL database. It is columnar on Apache Arrow, designed for object storage, with a log at the centre. The dev docs mention "hash trie" files.
- JVM. Architecture docs not read (404). No numbers.

**10. Jin et al., "KÙZU Graph Database Management System", CIDR 2023.** cidrdb.org/cidr2023/papers/p48-jin.pdf. **Full text of the storage section [read].**
- Node properties go in plain column files. Edges are **double-indexed CSR** (forward and backward). Neighbour labels are omitted when the DDL fixes them. Edge properties sit in "parallel" CSR columns. Pages are 4 KB with GClock.
- MVCC was "in the roadmap" and there is no history.
- No size numbers in the paper.
- **The repository is archived** (README: "We are archiving the KuzuDB project", last release 0.11.3).

**11. Neo4j.** Operations manual *Store formats* [read] and record-format source `NodeRecordFormat.java`, `RelationshipRecordFormat.java`, `PropertyRecordFormat.java` (neo4j/neo4j dev and 4.4 branches) [read].
- Standard/aligned format:

  | Record | Bytes | Contents |
  |---|---|---|
  | Node | **15** | in_use, next_rel, next_prop, 5 B inline labels, extra |
  | Relationship | **34** | first_node, second_node, type, and prev/next pointers for *both* endpoints' doubly-linked chains, next_prop |
  | Property | **41** | |

- Fixed-size records addressed by id × size; `block` is now the Enterprise default; no versioning. One edge = 272 bits before properties, vs 2–5 bits/edge for WebGraph or k²-trees.

## Comparison table

| System | Model | Versioning / history | Structural sharing across versions | On-disk compactness (source's numbers) | Slicing / subgraphs | Embeddable in a Rust kernel? | Licence / runtime |
|---|---|---|---|---|---|---|---|
| HugeGraph | Property graph, KV rows (RocksDB/HStore) | None (TTL only) | No | RocksDB snappy; no per-edge figure; each edge stored out + in | Query-level only | No (Java server) | Apache-2.0, JVM |
| Rama | Event log (depot) + nested-collection PStates; not a graph DB | Depot log only; PStates mutable | Explicitly disallowed for subindexed structures | RocksDB; no figure | Partitioned by key | No | Proprietary (free ≤2 nodes), JVM cluster |
| TerminusDB / terminus-store | RDF triples; HDT-style succinct layers | Immutable layer chain, branches as labels, rollup | Yes, at layer granularity (child = +/− delta on parent) | No numbers published in the whitepaper | Not natively; whole layers | Yes in principle (Rust crate) [inf] | Apache-2.0, Rust + Prolog |
| HDT | Static RDF file (dictionary + bitmap triples) | None (static) | No (HDTCat merges files) | ~10× vs N-Triples (HDTCat paper); original numbers not accessed | Spec supports splitting into chunks | Format only; hdt-rs exists [inf, unverified] | Spec; Java/C++ libraries |
| Prolly tree (Noms/Dolt) | Sorted map in CDC chunks, content-addressed | Commits of root hashes | Yes, chunk-level; diff O(d) | ~4 KB chunks; ~1.016×depth chunk writes per edit | Key-range scans; subtree by hash | Idea is easy to reimplement; Dolt is Go | Apache-2.0, Go |
| Datomic | Datoms (EAVT…), 4 covering indexes | Full: as-of/since/history | Segment trees, immutable [inf: shared] | "thousands of datoms" per segment; no byte figure | Index range scans | No | Free binaries (Apache-2.0), JVM |
| XTDB | Bitemporal documents/SQL, Arrow columnar | Full bitemporal | Unknown | No figure found | SQL | No | MPL-2.0, JVM |
| k²-tree | Static adjacency bitmap tree | None | No | **3.3–5.3 bpe**, with reverse neighbours | Submatrix = region query | Algorithm, small [inf] | Paper |
| WebGraph | Static compressed adjacency lists | None | No (but *reference lists* = intra-graph sharing) | **2.2–3.1 bits/link** (R=∞), 3.0–3.7 (R=3) | Per-node lazy iteration | Rust port `webgraph-rs` exists [inf, unverified] | LGPL/Apache (Java) [inf] |
| Kùzu | Property graph, columnar + double CSR | None (MVCC planned) | No | No figure | Cypher | C++ embeddable, but **archived** | MIT, C++ |
| Neo4j (standard) | Fixed records, linked rel chains | None | No | node 15 B, rel 34 B, prop 41 B | Cypher | No | GPLv3/commercial, JVM |

## Hypotheses

**G1. Graph = a content-addressed, sorted edge set chunked as a prolly tree on keys `(src, label, dst)`, plus a reverse set (sources: Noms/Dolt prolly trees; Kùzu and HugeGraph double indexing).**
- Change: a graph version is two root ids, forward and reverse. A commit points at them. The edge keys use our abbreviated ids (E006), and node records are a third prolly map.
- Expected effect: diff and slicing by key range cost O(changes). Identical subranges dedupe across versions and across graphs for free.
- Risk: E009 showed that many small objects pay framing plus full ids, so the chunk target must be large, about 4 KB as in Noms/Dolt. A key-only boundary hash (Dolt) keeps edits from re-chunking.
- Measure: bytes per edge per version on a synthetic edit history, with byte-exact reconstruction and a clean fsck.
- **Status:** implemented in E011 (capability): prolly chunks of 4–32 KB stored as content; 20 versions take 3.3x one compressed version (one content per version: 1.6x, without chunk-level lookup or diff).

**G2. Layer deltas instead of re-chunking: commit = parent graph id + positive edge set + negative edge set, with periodic rollup (source: TerminusDB layers).**
- Change: store a graph commit as small `+E`/`−E` objects over a parent, and roll up when the chain exceeds a bound. This mirrors git `pack.depth` and Mercurial's 2× rule from tree-metadata.md.
- Expected effect: the smallest per-commit bytes for small edits. Reads cost O(chain).
- This is the alternative to G1: run it on a separate branch and compare it on the same corpus.
- **Status:** `open` (not yet run).

**G3. WebGraph-style reference adjacency as a semantic-dedup derivation (source: Boldi–Vigna reference compression, copy blocks, chain bound R).**
- Change: in the ledger, allow an adjacency list (or edge chunk) to be derived as `copy-blocks(ref list) + extras`. The reference is chosen within a window, and the chain is capped at R ≈ 3.
- Expected effect: Boldi–Vigna report 3.08 vs 4.17 bits/link (R=∞ vs R=1) on WebBase. Our graphs have no URL-order locality, so the gain is unknown [inf]. Node ordering (e.g. by source byte offset) would matter.
- **Status:** `open` (not yet run).

**G4. Dictionary-coded node ids per immutable segment (sources: HDT/TerminusDB front-coded dictionaries; HDTCat merge by id remap).**
- Change: inside a stored edge chunk, replace 32 B node ids with ordinals into a per-chunk sorted id table. Gaps plus varints, as in WebGraph, keep the canonical (hashed) form full-width. This is analogous to E006.
- Expected effect: edge bytes fall from ~64+ B per edge to a few bytes plus the amortized id table. The size is not predicted until measured.
- **Status:** `open` (not yet run).

## Unknowns

- **HDT paper (JWS 2013):** not accessed (paywall, ResearchGate 403, rdfhdt.org TLS certificate expired). Its bytes-per-triple figures are not in this note. The only HDT figure is HDTCat's "factor 10" vs N-Triples.
- **TerminusDB whitepaper** has no size or benchmark numbers, despite the request. I found no published bytes-per-triple figure for terminus-store.
- **Datomic, XTDB, Dolt and HugeGraph** publish no bytes-per-edge or bytes-per-datom figures that I found. XTDB was README-level only.
- **Rama:** I did not see the on-disk format of the depot log, or whether PState values are compressed.
- Whether `webgraph-rs` and an HDT Rust crate are maintained and fit our licence: not checked [inf].
- k²-tree and WebGraph numbers are for web graphs with strong locality, and both structures are static. Our graphs are small and versioned, so per-object framing may dominate, as E009 found for trees [inf].
- Whether prolly-tree history independence survives our ledger derivations (objects stored via other objects must still hash canonically): believed yes, because ids are over canonical bytes, but not tested [inf].
