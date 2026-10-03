# IKAM Rust kernel: reconstructable store with an execution layer

**Goals.** Semantic dedup, graph connectivity, and guaranteed reconstruction.

**Built** in `packages/kernel`:
- L0 objects
- L1 data-driven dedup and compression
- Container unpacking
- L3 pure functions (builtin and WASM), memoized through the ledger
- A ledger of verified derivations with selectors (many per id), which is also the graph
- Versioning: commits, refs, gc, fsck, repack

**Not built:**
- template (anti-unification) and AI planners that propose derivations
- unverified semantic claims
- effectful/LLM runs
- scheduling

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
| L0 objects | `Blob`, `Tree`, `Commit`, `Derivation`. Id = `BLAKE3(encoding)`. | Strict canonical encoding: `encode(decode(b)) == b`. Every read is re-hashed against its id. |
| L1 content | A file's id is `Id::of_content(bytes)`. Its bytes may be stored (zstd-compressed when smaller), or it may exist only through ledger derivations. Those include references (`concat` of byte ranges of any content) and function applications. | `read_content(put_content(x).id) == x`. The id never depends on storage. Every derivation is verified by reconstructing the bytes before it is recorded. |
| L2 semantic dedup | `Repo::put_derivation(bytes, func, args)` records `bytes = func(args)` if evaluating it reproduces `bytes` exactly. This is the hook for template and AI planners: they propose, the kernel verifies. | An AI mistake costs nothing: a rejected proposal writes nothing. |
| L3 execution | Functions are builtins (`concat`, `deflate-pack`) or WASM modules stored as content. `Repo::apply` evaluates and records the derivation, which serves as memo, provenance and a storage option all at once. | Functions are pure: WASM may not import anything or use floats, and runs under fixed fuel and memory limits, so a recorded output is valid forever. |
| Graph | `links(id)` lists outgoing edges of any object or content id. Edge labels: `derivation`, `output`, `func`, `argN`, `argN[start..end]`, tree entry names, `tree`, `parent`. `used_by(id)` lists incoming edges. | Edges come from the same objects reconstruction uses, so the graph cannot disagree with the data. |
| Versioning | Trees, commits, compare-and-swap refs. `gc` keeps what refs reach, plus every derivation of kept content and its inputs. `repack` re-plans storage store-wide. `fsck` verifies every object and evaluates every derivation. | History is immutable; refs move only from their expected value; live content keeps its provenance. |
| Scheduling (planned) | Petri nets over derivations, for approvals, budgets/resources and retries. | A transition fires only when enabled; each firing records its before and after markings. |

Postgres, pgvector and any graph database are **projections** that can be rebuilt from objects and refs. Two projections exist today: the seed index and the ledger index (by output, and by computation for memoization). Both are rebuilt from objects on demand.

## L0 encoding (normative)

u32 and u64 are big-endian.

```
blob       = "B" bytes                      (storage variant: "Z" zstd(bytes); same id)
tree       = "T" count:u32 { name_len:u32 name kind("F"|"T") id[32] }   names strictly ascending
commit     = "C" tree[32] nparents:u32 { parent[32] } msg_len:u32 msg
derivation = "D" output[32] func[32] nargs:u32 { arg }
arg        = 0 id[32] | 1 id[32] start:u64 len:u64        (whole content | non-empty byte range)
```

- **Range selectors** may point into any content, including content that is itself only derived.
- **Reconstruction** follows derivations under a cycle guard and a depth bound (32).
- **Builtin function ids** are hashes of `"\0ikam/builtin/<name>"`. Every stored encoding starts with a letter tag, so a builtin id cannot collide with stored content.

## Dedup boundaries are data-driven

There are no fixed chunk sizes and no heading or paragraph cuts. The matcher (`matcher.rs`) works as follows:

1. **Index seeds.** Content-defined anchors (gear hash, about 1 in 64 positions) index 32-byte seeds of stored blobs. Seeds are lookup keys only.
2. **Extend matches.** Candidate matches are extended byte-by-byte, so slices end exactly where content stops agreeing.
3. **Keep matches that pay.** A match is used only if it is at least 96 B. That is a cost rule, not a boundary.

**Containers.** `container.rs` splits a zip (xlsx/docx/pptx) into a manifest (all non-stream bytes, verbatim) and the *uncompressed* members. Each member is then stored and deduped like any file. The zip itself becomes the derivation `deflate-pack(manifest, members...)`:
- A member is expanded only if re-deflating it reproduces its original stream exactly.
- The whole reconstruction is verified before the container is stored in this form.

**Corrupted inputs.** A property test feeds bit-flipped and truncated containers through ingest. They must round-trip, and they do.

## Reconstruction depends on pinned behaviour

A derivation reconstructs only if its function behaves identically in the future:
- **zlib** for `deflate-pack`. Bundled and statically linked, pinned with `=`. `container::tests::deflate_output_is_pinned` fails if any level's output changes.
- **The WASM interpreter** (wasmi). Pinned with `=`; `portable-dispatch` and `deterministic` features enabled; floats disabled. Fuel accounting is part of a function's semantics.

After upgrading either, run `ikam fsck`: it reconstructs and verifies every object. A failure is detected, never silently wrong data. But content whose only derivation uses a function that changed behaviour becomes unreadable until the old version is restored.

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

## Measured: ledger model (2026-10-03, release build, via the CLI)

Each corpus is committed as directories into one store, so tree and commit objects are included. That is why these ratios are higher than the per-file numbers above.

"Previous" is the commit before the ledger model (`b7a07cf`), run through the same script.

| Corpus | Previous | Ledger, at ingest | Ledger, after repack |
|---|---|---|---|
| 344 md fixtures (180 KB) | 136,201 | 136,434 | 136,434 |
| 44 pdf fixtures (80 KB) | 48,417 | 50,197 | 49,569 |
| 238 office fixtures (2.9 MB) | 505,500 | 529,075 | 528,919 |
| Synthetic 21 revisions, oldest first (650 KB) | 36,793 | 38,530 | **25,919** |
| Synthetic 21 revisions, newest first | — | 25,919 | 25,919 |
| This repo, 10 snapshots oldest first (46 MB) | 1,430,522 | 1,435,689 | **1,421,641** |
| This repo, newest first | — | 1,422,514 | 1,421,641 |

Every row passed `fsck` and an exact checkout of the last snapshot. git's packed store of the same snapshots is 1,232,071.

**Findings:**
- **Repack removes ingest-order dependence.** Both orders converge to the same size. On the synthetic history that is 30% below the previous version.
- **The ledger costs 0.2–4.6% at ingest on corpora with little reuse.** A derivation record names its output (32 B) and function (32 B), plus a tag byte per argument. The old single-form encoding stored under the output id needed neither. Two fixes if this matters:
  - store an id's *primary* derivation under the id itself, with the output implicit
  - give builtins short ids
- **Container members are stored whichever form wins.** They are content in their own right. A container candidate is therefore charged only its derivation record. Charging the members too made first-seen xlsx files lose to plain bytes even though the members were already written. Tests caught this.
- **Storage plans vs facts.** `concat` derivations are storage plans: repack replaces them, and the literal blobs they read are not kept for their own sake. All other derivations are facts: they are re-recorded with their inputs. Without this rule, repack treated old literal blobs as live files and grew the store by about 8 KB on the synthetic history.

## Salvage inventory from the Python packages

The Python packages are stripped to their ideas, not ported line-for-line. The old code is not trusted: e.g. its round-trip verifier, its delta chains (`compute_delta` is a whole-content replace; `delete` does not check old content) and its "Fisher information" metrics were all found to be broken or vacuous.

| Source | Idea taken | Kernel home |
|---|---|---|
| `ikam/forja/boundary_planner*.py` | AI proposes, deterministic code verifies | `put_derivation`, ingest candidates |
| `ikam/forja/verifier.py` (`ByteIdentityVerifier`) | Byte identity as the only pass criterion | Every storage form |
| `ikam/fragments.py` relations + `relation_eval.py` | Operator identified by function hash plus slot bindings | `Derivation{output, func, args}` |
| docs: fragment algebra "monadic" fragments | Values, references, function applications | L1 content forms |
| `ikam/delta_chain.py` | Store versions as deltas | `concat` of ranges; repack re-plans instead of rebasing chains |
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

## Ledger model (2026-10-03, built)

This replaces "one storage form per id" with a ledger of verified derivations. It follows the 2026-10-03 design discussion.

**The ledger.** A derivation record `D` says `output = func(args)`.
- Each argument is a *selector*: a whole content id, or a byte range of one.
- A record enters the ledger only after evaluating it reproduces the output's exact bytes.
- One id may have many derivations, which makes the ledger an e-graph: equivalent programs for the same bytes.

**Reconstruction.** Use the materialized bytes if present. Otherwise try the id's derivations, recursively, with a cycle guard and a depth bound, and verify the hash. A corrupted materialized copy falls through to a derivation, so the store can heal itself where the ledger allows.

**Storage is a policy over the ledger.** Which ids keep raw bytes is decided by cost. At ingest, candidates compete:
- plain bytes
- a slice plan (`concat` of selectors)
- container unpacking (`deflate-pack`)

The cheapest verified candidate wins. `repack` re-plans all live content together, largest first, and keeps the result only if it is smaller and fully verified. This addresses the ingest-order dependence measured on 2026-10-03: the same 21 revisions stored in 34.3 KB oldest-first and 21.9 KB newest-first.

**Unification.** The old forms collapse into one record kind:
- `Rep` becomes `concat(ranges)`.
- `Apply` becomes a derivation used for storage.
- `Run` becomes a derivation recorded by execution, which serves as the memo.

**Claims vs derivations.** Unverified semantic claims (mentions, paraphrase, section-of) are a separate record kind, not built yet. A claim can be promoted to a derivation once a function plus residual reproduces the bytes.

**Planned after this:** an anti-unification (template + fillers) planner, measured on the fixtures before any general transform search is built.

## Next steps

1. **Template planner:** anti-unification over similar segments; measure against slices + zstd.
2. **Claims:** unverified semantic relations over selectors, with edge weight = measured bits saved by conditioning.
3. **WASM floats:** allow them. The `deterministic` feature already canonicalizes NaNs; disable relaxed-SIMD instead.
4. **Effectful runs.** LLM and tool calls as recorded, replayed, never-regenerated runs. These are never storage derivations.
5. **Storage.**
   - A zstd dictionary trained from the store.
   - Persisted ledger, seed and reverse-link indexes as checkpointed projections. Today each CLI process rebuilds them by reading every object.
   - Cheaper derivation records (implicit output for the primary derivation; short builtin ids).
   - Let the matcher index derived content too. Today it only finds matches in stored bytes; ranges into derived content come only from proposals.
   - Self-matching within a file.
   - zlib-as-WASM.
   - PDF FlateDecode streams.
6. **Scheduling** (a Petri net over derivations) and **PyO3 bindings**.
