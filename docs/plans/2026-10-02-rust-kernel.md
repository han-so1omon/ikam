# IKAM Rust kernel: reconstructable store with an execution layer

Status: built in `packages/kernel` — L0 objects, L1 data-driven dedup, and versioning (commits, refs, gc). Not built yet — semantic dedup (L2), derivations (L3) and scheduling.

## Why restart

The Python stack (about 150k lines across `ikam`, `modelado`, `interacciones`, `mcp-ikam`, perf-report) has sound ideas but does not deliver its central guarantee:

- Verification compares the source with itself. `ikam/forja/debug_execution.py:2081-2085` substitutes the source bytes whenever reconstruction is empty or raises.
- Reconstruction only works because the whole file is kept as one fragment.
- Three hash functions (blake3, sha256, blake2b) are used with silent fallbacks.
- There are about eight overlapping graph and fragment stores and three execution engines.
- Six tables are written to but have no DDL.

The kernel keeps the ideas and makes each layer's guarantee a tested law.

## Layers and laws

| Layer | Contents | Law (enforced by code and tests) |
|---|---|---|
| L0 objects | `Blob`, `Rep` (slices of blobs), `Tree`, `Commit`. Id = `BLAKE3(encoding)`. | Strict canonical encoding: `encode(decode(b)) == b`. Every read is re-hashed against its id. |
| L1 storage dedup | A file's id is `Id::of_content(bytes)`, i.e. the id of its blob form. It is stored either as that blob or as a `Rep` slicing other blobs. A planner proposes the slices; the kernel verifies them. | `read_content(put_content(x).id) == x`. The id never depends on storage form or planner. A plan that is invalid, wrong or not smaller falls back to a blob. |
| Versioning | Trees (sorted, validated names), commits, refs. Refs are the only mutable state, updated by compare-and-swap under a lock file. `gc` deletes whatever no ref can reach. | History is immutable. A ref moves only from its expected old value. `gc` keeps everything a ref can reach, including rep sources. |
| L2 semantic dedup (planned) | An AI proposes that content is a transformation of other content (e.g. reformatted, reordered, templated). | Accepted only if reconstruction is verified byte-for-byte and saves bytes. An AI mistake costs space, never correctness. |
| L3 derivations (planned) | `(fn_id, input_ids, env_id) -> output_id`. Functions are objects (WASM, or pinned sandboxed code; never `exec`). Each op has an effect tag: `pure` / `nondet` / `io`. | `pure` results are memoized. `nondet` results (LLM, tools) are recorded and replayed, never silently regenerated. |
| Scheduling (planned) | Petri nets over derivations, for approvals, budgets/resources and retries. Markings are content-addressed. | A transition fires only when enabled; each firing records its before and after markings. |

Postgres, pgvector and any graph database are **projections** that can be rebuilt from objects and refs. They are never a source of truth. The seed index used for dedup is already one: it is rebuilt from blobs on demand.

## L0 encoding (normative)

u32 and u64 are big-endian.

```
blob   = "B" bytes
rep    = "R" count:u32 { src[32] start:u64 len:u64 }          stored under the id of the blob it reproduces
tree   = "T" count:u32 { name_len:u32 name kind("F"|"T") id[32] }   names strictly ascending, no "/", NUL, ".", ".."
commit = "C" tree[32] nparents:u32 { parent[32] } msg_len:u32 msg
```

- **Rep sources** must be stored as plain blobs, so reconstruction is at most one level deep. There are no delta chains to bound or rebase.
- **Structure and media type are not part of a file's identity.** Semantic structure (sections, entities, relations) belongs in a separate annotation layer over content ids. It never belongs in storage.

## Dedup boundaries are data-driven

Chunks are not cut at predefined positions: no fixed sizes, no headings or paragraphs. The current planner (`matcher.rs`) works as follows:

1. **Index seeds.** For every stored blob, index 32-byte seeds at content-defined anchors. An anchor is a gear-hash position, about 1 in 64, and depends only on the preceding 64 bytes. Seeds are only a lookup key.
2. **Find candidates.** At each anchor of new content, look up candidate matches in stored blobs.
3. **Extend byte by byte.** Grow each candidate backward and forward. A slice therefore ends exactly where the content stops agreeing. `tests/laws.rs::boundaries_are_byte_precise` checks this for insertions at arbitrary offsets.
4. **Keep only matches that pay.** A match is used only if it is at least 96 B, the cost of its slice entry plus the entry needed to resume the literal run. This is a cost rule, not a boundary.
5. **Store new bytes once.** All unmatched bytes become one new literal blob, which is indexed for future matches.

Known limits:
- Repeats inside a single new file are not yet matched against themselves.
- Matches shorter than about 96 B, or that contain no anchor, are missed.
- The index is rebuilt in memory per process. A persisted index would be a checkpointed projection.

## Measured (2026-10-02, release build, via the CLI)

Ratio = stored object bytes / input bytes. No compression is applied anywhere.

| Corpus | First L1 (predefined md/cdc chunks) | Data-driven L1 |
|---|---|---|
| 344 md fixtures, 180 KB | 1.70× (md), 1.00× (raw) | **0.97×** |
| 220 pdf+xlsx fixtures, 967 KB | 1.01× (cdc) | **0.41×** |
| 21 synthetic revisions of a 30 KB spec | 0.15× (md), 0.39× (cdc) | **0.086×** |
| 10 real snapshots of this repo's history (docs + 3 Python packages, 46 MB) | — | **0.104×** |

All files round-tripped byte-for-byte. A checkout of the latest snapshot matches the source tree exactly.

Reference points for the real history:
- **File-level dedup only:** 0.116×. This history mostly adds or removes whole files, so there is little in-file reuse to find.
- **git's packed store (zlib + deltas):** 1.23 MB, or 0.027×. git wins by about 4×, and most of the gap is compression.
- **Next fix:** compress blobs in storage (zstd). Identity is computed over uncompressed content, so this changes no ids.

## Salvage inventory from the Python packages

The Python packages are stripped to their ideas, not ported line-for-line.

| Source | Idea taken | Kernel home |
|---|---|---|
| `ikam/forja/boundary_planner*.py` | AI proposes, deterministic code verifies | `Repo::put_planned` accepts any planner's proposal, then verifies it |
| `ikam/forja/verifier.py` (`ByteIdentityVerifier`) | Byte identity as the only pass criterion | `put_planned`, `read_content` |
| `ikam/delta_chain.py` | Store versions as deltas | `Rep` slices; one level deep, so no chains |
| `ikam/fragments.py` relations + `relation_eval.py` | Operator identified by function hash plus slot bindings | L3 derivation key |
| `modelado/graph_edge_event_log.py`, `graph_edge_event_folding.py` | Idempotency keys; deterministic fold; checkpointed projections | Projections over objects/refs (seed index first) |
| `modelado/history/head_locators.py`, `ikam_fragment_objects` | Refs → commits → immutable manifests | Versioning (built) |
| `modelado/core/model_call_cache*` | Cache keyed by (model, prompt hash, seed) | L3 `nondet` record/replay |
| `modelado/authz/signing.py`, `core/execution_context.py` | Signed write envelopes; single write chokepoint | Ref updates |
| `modelado/plans/engine.py`, `interacciones/schemas/petri.py` | Petri enabling/firing; CAS-hashed markings; bipartite validation | Scheduling |
| `interacciones/.../scheduler.py` | Lease-based claiming; durable retry state | Scheduling runtime |
| `modelado/enrichment/policy.py` | Commit lanes require strict dedup plus provenance | L2 acceptance policy |
| docs: CONTRACT §6.3, §9-10; STAGING §3 | Identity excludes structure; freeze-on-publish; promote reachable closure | L0, L3, refs |

To be dropped when the Python packages are retired:

- Fisher-information code and claims (both copies)
- `sequencer/`, economic and story operation handlers, domain `models.py`
- Product tables in `db.py`
- Duplicate repositories, clients and traversal engines
- The unsandboxed `exec` path
- HugeGraph as a dependency

## Next steps

1. **Storage:** zstd-compressed blob form; a persisted seed index; self-matching within a file.
2. **Format awareness without fixed boundaries:** expand container formats (zip parts of xlsx/docx/pptx, PDF streams) into a stored *derivation* `unpack(x) -> parts`, so dedup sees uncompressed content. Reconstruction is verified like any plan.
3. **L3:** a derivation record plus a memo table; effect tags; WASM function objects (wasmtime).
4. **L2:** AI-proposed transformations, gated by verified reconstruction and measured savings.
5. **Scheduling:** a Petri net over derivations; port the enabling/firing semantics from `modelado/plans/engine.py`.
6. **Bindings:** PyO3 bindings so remaining Python code can call the kernel while it is retired.
