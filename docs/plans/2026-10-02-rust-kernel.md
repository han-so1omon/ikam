# IKAM Rust kernel: reconstructable store with an execution layer

**Goals.** Semantic dedup, graph connectivity, and guaranteed reconstruction.

**Built** in `packages/kernel`:
- L0 objects
- L1 data-driven dedup and compression
- Container unpacking
- L3 pure functions (builtin and WASM) with memoized, provenance-recording runs
- Graph links
- Versioning: commits, refs, gc, fsck

**Not built:** AI planners that propose semantic dedup (L2), effectful/LLM runs, scheduling.

## Why restart

The Python stack (about 150k lines across `ikam`, `modelado`, `interacciones`, `mcp-ikam`, perf-report) has sound ideas but does not deliver its central guarantee:

- Verification compares the source with itself. `ikam/forja/debug_execution.py:2081-2085` substitutes the source bytes whenever reconstruction is empty or raises.
- Reconstruction only works because the whole file is kept as one fragment.
- Three hash functions (blake3, sha256, blake2b) are used with silent fallbacks.
- There are about eight overlapping graph and fragment stores and three execution engines.
- Six tables are written to but have no DDL.

The design docs' "monadic" fragment idea is kept: fragments may be values, references or function applications. Here it is a storage law, not a metaphor (see L1).

## Layers and laws

| Layer | Contents | Law (enforced by code and tests) |
|---|---|---|
| L0 objects | `Blob`, `Rep`, `Apply`, `Tree`, `Commit`, `Run`. Id = `BLAKE3(encoding)`. | Strict canonical encoding: `encode(decode(b)) == b`. Every read is re-hashed against its id. |
| L1 content | A file's id is `Id::of_content(bytes)`. It is stored in one of three forms: a **value** (blob, zstd-compressed when smaller); a **reference** (`Rep`: slices of stored blobs); or a **function application** (`Apply`: a pure function of other stored content). | `read_content(put_content(x).id) == x`. The id never depends on the form. Every proposed form is verified by reconstructing the bytes before it is written. |
| L2 semantic dedup | `Repo::put_apply(bytes, Apply{func, args})` stores `bytes` as `func(args)` if evaluating it reproduces `bytes` exactly. This is the hook for AI planners: they propose, the kernel verifies. | An AI mistake costs nothing: a rejected proposal writes nothing. |
| L3 execution | Functions are builtins or WASM modules stored as content. `Repo::apply` evaluates and records a `Run` (apply → output), which is both memo and provenance. | Functions are pure: WASM may not import anything or use floats, and runs under fixed fuel and memory limits, so a recorded output is valid forever. |
| Graph | `links(id)` lists outgoing edges of any object. Edge labels: `slice`, `func`, `argN`, `output`, tree entry names, `tree`, `parent`. `used_by(id)` lists incoming edges. | Edges come from the same objects reconstruction uses, so the graph cannot disagree with the data. |
| Versioning | Trees, commits, compare-and-swap refs. `gc` keeps what refs reach, plus `Run` records (and their inputs) for every kept output. `fsck` reconstructs and verifies everything. | History is immutable; refs move only from their expected value; live content keeps its provenance. |
| Scheduling (planned) | Petri nets over runs, for approvals, budgets/resources and retries. | A transition fires only when enabled; each firing records its before and after markings. |

Postgres, pgvector and any graph database are **projections** that can be rebuilt from objects and refs. Two projections exist today: the seed index and the run memo. Both are rebuilt from objects on demand.

## L0 encoding (normative)

u32 and u64 are big-endian.

```
blob   = "B" bytes                      (storage variant: "Z" zstd(bytes); same id)
rep    = "R" count:u32 { src[32] start:u64 len:u64 }      stored under the id of the bytes it reproduces
apply  = "A" func[32] nargs:u32 { arg[32] }               stored under the id of the bytes it reproduces
tree   = "T" count:u32 { name_len:u32 name kind("F"|"T") id[32] }   names strictly ascending
commit = "C" tree[32] nparents:u32 { parent[32] } msg_len:u32 msg
run    = "X" func[32] nargs:u32 { arg[32] } output[32]
```

- **Rep sources** must be plain blobs, so slices never chain. **Apply** nesting is bounded (depth 16).
- **Builtin function ids** are hashes of `"\0ikam/builtin/<name>"`. Every stored encoding starts with a letter tag, so a builtin id cannot collide with stored content.

## Dedup boundaries are data-driven

There are no fixed chunk sizes and no heading or paragraph cuts. The matcher (`matcher.rs`) works as follows:

1. **Index seeds.** Content-defined anchors (gear hash, about 1 in 64 positions) index 32-byte seeds of stored blobs. Seeds are lookup keys only.
2. **Extend matches.** Candidate matches are extended byte-by-byte, so slices end exactly where content stops agreeing.
3. **Keep matches that pay.** A match is used only if it is at least 96 B. That is a cost rule, not a boundary.

**Containers.** `container.rs` splits a zip (xlsx/docx/pptx) into a manifest (all non-stream bytes, verbatim) and the *uncompressed* members. Each member is then stored and deduped like any file. The zip itself becomes `Apply{deflate-pack, [manifest, members...]}`:
- A member is expanded only if re-deflating it reproduces its original stream exactly.
- The whole reconstruction is verified before the container is stored in this form.

**Corrupted inputs.** A property test feeds bit-flipped and truncated containers through ingest. They must round-trip, and they do.

## Reconstruction depends on pinned behaviour

An `Apply` reconstructs only if its function behaves identically in the future:
- **zlib** for `deflate-pack`. Bundled and statically linked, pinned with `=`. `container::tests::deflate_output_is_pinned` fails if any level's output changes.
- **The WASM interpreter** (wasmi). Pinned with `=`; `portable-dispatch` and `deterministic` features enabled; floats disabled. Fuel accounting is part of a function's semantics.

After upgrading either, run `ikam fsck`: it reconstructs and verifies every object. A failure is detected, never silently wrong data. But a stored `Apply` whose function changed behaviour becomes unreadable until the old version is restored.

The durable fix is to make functions content: compile zlib to WASM and store it, so the container format no longer depends on a Rust dependency. Builtins are versioned (`deflate-pack/1`) so a successor can coexist.

## Measured (2026-10-02, release build, via the CLI)

Ratio = stored object bytes / input bytes. All files round-tripped byte-for-byte. Checkout and `fsck` of the real-history store pass.

| Corpus | Predefined chunks | Data-driven, no compression | **Current** | Plain zlib-6 per file |
|---|---|---|---|---|
| 344 md fixtures, 180 KB | 1.70× | 0.97× | **0.62×** | 0.60× |
| 44 pdf fixtures, 80 KB | — | — | **0.55×** | 0.57× |
| 238 xlsx/docx/pptx fixtures, 2.9 MB | — | — | **0.16×** | 0.87× |
| 21 synthetic revisions of a 30 KB spec | 0.15× | 0.086× | **0.053×** | — |
| 10 real snapshots of this repo (46 MB) | — | 0.104× | **0.031×** | git packed: 0.027× |

Notes:
- **Office files** gain the most because members are deduped uncompressed: shared styles, themes and boilerplate XML across workbooks are stored once.
- **Small markdown files** are slightly worse than zlib per file. Each file is compressed alone, and zstd's per-frame overhead dominates at about 520 B per file. A zstd dictionary trained from the store is the obvious fix; it is a learned, data-driven form of dedup.
- **The fixtures were written by Python's zlib**, which is why every member reproduces. Files saved by Microsoft Office use a different deflate implementation. Their members will mostly not reproduce, and those streams stay compressed in the manifest. Handling them needs a preflate-style reconstruction diff.

## Salvage inventory from the Python packages

The Python packages are stripped to their ideas, not ported line-for-line. The old code is not trusted: e.g. its round-trip verifier, its delta chains (`compute_delta` is a whole-content replace; `delete` does not check old content) and its "Fisher information" metrics were all found to be broken or vacuous.

| Source | Idea taken | Kernel home |
|---|---|---|
| `ikam/forja/boundary_planner*.py` | AI proposes, deterministic code verifies | `put_planned`, `put_apply` |
| `ikam/forja/verifier.py` (`ByteIdentityVerifier`) | Byte identity as the only pass criterion | Every storage form |
| `ikam/fragments.py` relations + `relation_eval.py` | Operator identified by function hash plus slot bindings | `Apply{func, args}` |
| docs: fragment algebra "monadic" fragments | Values, references, function applications | L1 content forms |
| `ikam/delta_chain.py` | Store versions as deltas | `Rep` slices; one level deep, so no chains |
| `modelado/graph_edge_event_log.py`, `graph_edge_event_folding.py` | Idempotency keys; deterministic fold; checkpointed projections | Projections (seed index, run memo) |
| `modelado/history/head_locators.py`, `ikam_fragment_objects` | Refs → commits → immutable manifests | Versioning |
| `modelado/core/model_call_cache*` | Cache keyed by (model, prompt hash, seed) | Future effectful runs (record/replay) |
| `modelado/authz/signing.py`, `core/execution_context.py` | Signed write envelopes; single write chokepoint | Ref updates |
| `modelado/plans/engine.py`, `interacciones/schemas/petri.py` | Petri enabling/firing; CAS-hashed markings | Scheduling |
| `interacciones/.../scheduler.py` | Lease-based claiming; durable retry state | Scheduling runtime |
| `modelado/enrichment/policy.py` | Commit lanes require strict dedup plus provenance | L2 acceptance policy |
| docs: CONTRACT §6.3, §9-10; STAGING §3 | Identity excludes structure; freeze-on-publish; promote reachable closure | L0, runs, refs |

To be dropped when the Python packages are retired:

- Fisher-information code and claims (both copies)
- `sequencer/`, economic and story operation handlers, domain `models.py`
- Product tables in `db.py`
- Duplicate repositories, clients and traversal engines
- The unsandboxed `exec` path
- HugeGraph as a dependency

## Next steps

1. **Semantic layer.** Annotation objects: typed relations between content ids (entity mentions, "section of", "derived from"). These make semantic graph edges separate from storage. AI extractors produce them as recorded runs, so they are replayable and attributed.
2. **AI dedup planner.** Propose `Apply` forms for near-duplicates: candidate source selection via embeddings, plus a library of WASM transforms (patch, reformat, template fill). Accept only what `put_apply` verifies and only when it saves bytes.
3. **Effectful runs.** LLM and tool calls as `nondet` runs: record outputs, replay them, never regenerate silently. These are never usable as storage forms.
4. **Storage.**
   - A zstd dictionary trained from the store.
   - Persisted seed, memo and reverse-link indexes as checkpointed projections.
   - Self-matching within a file.
   - zlib-as-WASM for `deflate-pack`.
   - PDF FlateDecode streams.
5. **Scheduling.** A Petri net over runs; port the enabling/firing semantics from `modelado/plans/engine.py` after reviewing them.
6. **Bindings.** PyO3 bindings so remaining Python code can call the kernel while it is retired.
