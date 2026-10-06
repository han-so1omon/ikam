# IKAM system architecture (proposed)

Status: proposal. Parts marked **built** exist on `feature/rust-kernel` (experiments E000–E020); parts marked **proposed** do not. Claims about effects are measured only where an experiment is cited.

## The idea in one paragraph

Every file is stored as verified, content-addressed bytes. **How** its bytes are stored (whole, sliced, templated, grouped, derived by a function) is a replaceable plan, chosen by measured size and read cost. The **meaning** of content (relations, entities, versions, render structure) is a separate, versioned graph layer. Semantic proposers, including LLMs, write into that layer. The planner turns the layer's relations into candidate derivations, and only what is verified byte-exactly and measured to pay is kept. Meaning can therefore guide storage but never corrupt it.

## Layers

```mermaid
flowchart TB
  subgraph Proposers["Proposers (pluggable, may be wrong)"]
    direction LR
    M["matcher seeds<br/>(built)"]
    T["templates / anti-unification<br/>(built)"]
    D["dictionary trainers<br/>COVER, raw, repeats (built)"]
    G["groupers: byte order, seeds,<br/>measured cost (built)"]
    E["embedding tool, offline<br/>(built, E019)"]
    L["LLM relation extractor<br/>(proposed)"]
  end
  subgraph Semantic["Semantic layer (versioned, cyclic)"]
    C["claims: measured relations<br/>gain_bits, author (built)"]
    GR["graph objects: prolly-tree G chunks<br/>slices, diffs, history (built)"]
  end
  subgraph Planner["Planner + evaluator (built)"]
    P["candidates -> verify -> score = bytes + w * read work<br/>ingest (greedy), repack (global, parallel plans)"]
  end
  subgraph Ledger["Ledger: how bytes are rebuilt (DAG, verified)"]
    DV["derivations: concat, fill, deflate-pack, WASM<br/>recorded only after reproducing exact bytes"]
  end
  subgraph Enc["Storage encodings (stored form only)"]
    EN["B/Z/Y plain, S group member, object forms,<br/>abbreviated ids, magicless frames, dictionary"]
  end
  ST[("CAS store: BLAKE3 ids<br/>MemStore / FsStore")]
  H["history: commits, trees, gc, fsck (built)"]

  Proposers -->|relations, orders, hints| Semantic
  Proposers -->|candidates| Planner
  Semantic -->|relations become candidate derivations| Planner
  Planner -->|verified derivations| Ledger
  Planner -->|chosen encodings| Enc
  Planner -->|measured gain| C
  Ledger --> ST
  Enc --> ST
  H --> ST
```

| layer | role | guarantee |
|---|---|---|
| CAS store | bytes by id | ids are BLAKE3; every read is checked against its id |
| Encodings | the smallest stored form of the same bytes | changing an encoding never changes an id |
| Ledger | `output = func(args)`, a DAG | recorded only if it rebuilds the exact bytes; cycles rejected |
| Planner and evaluator | chooses among verified candidates | score = bytes written + `read_weight` x decode work |
| Semantic layer | claims (measured, mutable) and graphs (versioned snapshots) | never on the rebuild path, so cycles are safe |
| Proposers | suggest relations, orders, candidates | may be wrong or nondeterministic; nothing they produce is trusted unverified |
| History | commits, trees, gc, fsck | gc keeps what live data needs: derivation inputs, dictionaries, groups, graph targets |

## LLM relations into compression (proposed)

An LLM is good at naming *why* two pieces of content are related. Byte matching cannot see that. Each relation type maps to the compression mechanism worth trying:

| relation (free-text predicate) | candidate the planner tries | mechanism |
|---|---|---|
| revision-of, copy-of, derived-from | delta: slices of the base, or `fill` with the base as template | ledger derivation (built) |
| same-form-as (invoice, report, slide) | induce one template over the set | templates (built) |
| same-topic-as, same-project-as | put in one group; train a family dictionary | groups (built), per-family dictionary (proposed) |
| contains, embeds, rendered-from | unpack the container; derive the rendering | `deflate-pack`, WASM functions (built) |
| translation-of, summary-of | none expected to pay | kept as knowledge only |

```mermaid
sequenceDiagram
  participant X as LLM extractor (offline, pinned model or recorded outputs)
  participant S as Semantic layer
  participant P as Planner
  participant K as Kernel (verify + ledger)
  X->>S: claim(subject, "revision-of", object), author = model id, gain unmeasured
  S->>P: relation becomes a candidate (delta / template / group)
  P->>K: put_derivation(bytes, func, args)
  K-->>P: verified? decode work?
  P->>P: score = bytes + w * work vs current plan
  alt saves bytes
    P->>K: record derivation, drop redundant bytes
    P->>S: claim gain_bits = measured saving
  else does not
    P->>S: claim kept with gain 0 (knowledge, not storage)
  end
```

Rules that keep this sound:
- **Reproducibility.** Model calls never run inside the benchmark or the kernel. An extractor writes its relations once (claims, or a proposal file as in E019) with the model id and revision recorded. Re-running the planner on the same relations gives the same store.
- **No trust in model output.** A wrong relation costs a failed verification or a rejected plan, never wrong bytes.
- **Learning from measurements.** Measured `gain_bits` per relation type is the training signal for which relations to propose. This is DeepSketch's lesson: learn from measured compression, not from semantic similarity. E019 measured embeddings as informative but dominated by measured cost.

## Reconstruction functions as a library in the graph (proposed, from the user's direction)

The LLM's main job is to **write reconstruction functions**, not only to name relations. Functions are stored like everything else, organised in the graph, and applied through the ledger. Reconstruction can then be any algorithm: arithmetic (a multiplication table), common algorithms (run-length, delta, transpose, reversal), templates, experimental ones, or recursive and fractal generators.

**What exists (built).** A derivation's function may be any WASM module stored as content (`func.rs`, `wasm.rs`, `tests/exec.rs`):
- deterministic: no imports, floats disabled, bounded fuel and memory;
- verified: recorded only after reproducing the exact bytes;
- used by repack: it drops stored bytes a function rebuilds when that is cheaper.

So an LLM-written function can never produce wrong bytes; at worst it fails verification or does not pay.

```mermaid
flowchart LR
  subgraph Lib["Function library (graph G: one node per function)"]
    F1["rle-decode"] ---|composes| F2["delta-decode"]
    F3["dsl-interpreter"] ---|runs| P1["program: 'table i*j'"]
    F4["transpose"]
  end
  LLM["LLM (offline): sees content + library + past failures"] -->|new function or small program| Lib
  Lib -->|candidate: output = f(args)| V{"verify: exact bytes?<br/>fuel and memory bounded"}
  V -->|yes| S{"pays? module counted once,<br/>amortised over every use,<br/>+ w * fuel"}
  V -->|no| X["discard; record the failure for the LLM"]
  S -->|yes| R["ledger derivation; stored bytes dropped;<br/>claim: measured gain per function"]
  S -->|no| K["keep as library entry only if reused elsewhere"]
```

Design points:
- **Library in the graph.** Each function is a graph node targeting its module (content). Edges record "applied-to" (its derivations), "composes" and "generalises". Claims carry the measured gain per function, which is the evidence for keeping it, generalising it, or dropping it (library learning, as in Stitch and babble).
- **Cost is description length.** A module is content, so it is stored once and shared by every derivation that uses it; its bytes are paid once. Each use pays only its small argument contents and its fuel (read work). This is the minimum-description-length rule DreamCoder, Stitch and Brevis use, and the kernel's evaluator already counts it this way.
- **Small programs over a shared interpreter.** An LLM-written WASM module is hundreds of bytes or more. A better shape is one stored interpreter module for a small deterministic language (integer arithmetic, loops, byte output, calls to library functions). Each LLM program is then a short argument, tens of bytes, so a program that generates a 100 KB table can cost less than its zstd output. *[inference]*
- **Composition and recursion.** Function outputs are content, so functions feed functions: delta of a transpose, a template filled by a generator, a fractal as an iterated function. The ledger's DAG is the composition; cycles stay impossible, as reconstruction requires.
- **Determinism limits.** No floating point: fractals and numeric generators use fixed-point integers or a deterministic soft-float compiled into the module.
- **Where it should pay (from the literature, not yet measured here).** Data that is the output of a procedure: computed spreadsheet columns, numeric sequences, logs, procedural images, generated code. For prose, the KoLMogorov Test (ICLR 2025) found frontier LLM programs failing 40–78% of the time and losing to gzip when correct. Templates, deltas and the dictionary remain the right tools there.
- **Reproducibility.** The LLM runs offline. Its functions and programs are stored as content, so once written they are ordinary data. The benchmark never calls a model.

## Partial reads: decompress only the segments needed (proposed)

**Yes, the render structure lets reads decode only what they need, with one codec-level limit.**

A file's render pipeline is already explicit in the ledger: `concat` of byte ranges, `fill(template, fillers)`, `deflate-pack(manifest, members)`, groups. Graph nodes can name semantic segments as `Arg::Range` targets ("slide 3", "section 2"). A range read can therefore walk the pipeline backwards:

```mermaid
flowchart LR
  Q["read(id, 4000..4200)"] --> R{"how is id stored?"}
  R -->|"concat(a[0..3000], b[100..5000])"| C1["read(b, 1100..1300) only"]
  R -->|"fill(template, fillers)"| C2["map the range onto template segments / filler parts"]
  R -->|"S member of a group"| C3["decode the group's frame only up to start+len,<br/>or one block of a seekable group"]
  R -->|"deflate-pack(zip)"| C4["the members overlapping the range"]
  R -->|"opaque WASM function"| C5["whole inputs (no range map)"]
```

- **concat and slices**: exact. A range of the output is a computable set of ranges of the inputs.
- **fill**: exact. Template segments and filler parts have known offsets.
- **Containers**: a range of a zip maps to the members that cover it, plus header bytes.
- **Opaque WASM functions**: no general range map, so whole inputs are needed, unless the function declares one (a proposed extension).
- **The codec limit.** A zstd frame decodes sequentially. Inside a group, reading a member must decode from the frame start to the member's end: on average half the group, not all of it as today. To read one segment in O(segment), a group is split into **independently decodable blocks** (zstd's "seekable format": frames of a few KiB plus an offset table, shipped in zstd's `contrib/`). Blocks lose some shared context, and the dictionary recovers part of it. The block size is then a measured trade-off.

With range reads, the evaluator can price **decode work actually needed** for a given access pattern (whole files, or segments named in the graph), instead of today's proxy (bytes produced by derivations, decompression unpriced).

## What exists and what is next

| step | status | measure |
|---|---|---|
| CAS, ledger, verify-before-record, history, gc, fsck | built | all laws and tests (55) |
| encodings, dictionary, groups, abbreviated ids | built | score 0.8896 -> 0.4762 (E000–E020, two benchmark changes) |
| graphs: prolly chunks, cycles, slices, diffs | built | laws 1–7; repo-graph corpus |
| proposer hook (orders) and offline embedding tool | built | E019 (neutral) |
| read benchmark: per-read decode work for whole-file and segment reads | proposed, next | new reported metric, not scored at first |
| range reads through concat, fill, containers and groups (stop the frame early) | proposed | decode work per read, same stored bytes |
| seekable groups (independent blocks) | proposed | bytes vs decode work per read, block size swept |
| evaluator prices decompression | proposed | score and read metric together |
| function library as a graph; hand-written baseline functions (rle, delta, transpose, reversal, arithmetic tables) | proposed | bytes on a new "generated" corpus |
| deterministic DSL interpreter module + programs as arguments | proposed | program bytes vs zstd of the output |
| generated corpus (computed tables, sequences, logs, procedural images) | proposed (benchmark change) | new reference score |
| LLM function writer and relation extractor (offline, outputs recorded) | proposed (needs an API key or a local model) | bytes saved per function and relation (measured `gain_bits`) |
| per-family dictionaries from relations | proposed | office/md bytes |
