# IKAM Rust kernel: reconstructable store with an execution layer

Status: L0 and L1 implemented in `packages/kernel`. L2, L3 and versioning are planned, not built.

## Why restart

The Python stack (about 150k lines across `ikam`, `modelado`, `interacciones`, `mcp-ikam`, perf-report) has sound ideas but does not deliver its central guarantee:

- Verification compares the source with itself. `ikam/forja/debug_execution.py:2081-2085` substitutes the source bytes whenever reconstruction is empty or raises.
- Reconstruction only works because the whole file is kept as one fragment.
- Three hash functions (blake3, sha256, blake2b) are used with silent fallbacks.
- There are about eight overlapping graph and fragment stores and three execution engines.
- Six tables are written to but have no DDL.

The kernel keeps the ideas and makes each layer's guarantee a tested law.

## Layers and laws

| Layer | Contents | Law (enforced, not documented) |
|---|---|---|
| L0 objects | `Blob(bytes)`, `Node([(label, kind, id)])`; `id = BLAKE3(encode(obj))` | `decode(encode(o)) == o`; `encode(decode(b)) == b`; `get(put(o)) == o`; `get` re-hashes on read |
| L1 codecs | A codec proposes a `Shape`: a labelled tree of byte ranges. Rendering is fixed by the kernel (concatenate leaves in order). | `render(ingest(c, x)) == x` for every codec `c`. A shape that does not tile `x` exactly, or a failed byte comparison, falls back to storing `x` as one blob. |
| L2 dedup (planned) | Exact = CAS. Structural = codecs. Semantic = an AI proposes a canonical form `C` plus a residual patch per source. | Accepted only if `apply(C, patch_i) == x_i` is verified and the merge saves bytes. An AI mistake costs space, never correctness. |
| L3 derivations (planned) | `(fn_id, input_ids, env_id) -> output_id`. Functions are objects (WASM, or pinned sandboxed code; never `exec`). Each op carries an effect tag: `pure` / `nondet` / `io`. | `pure` results are memoized by key. `nondet` results (LLM, tools) are recorded as objects and replayed, never silently regenerated. |
| Versioning (planned) | Commit objects (root, parents, metadata); refs are the only mutable state, updated by compare-and-swap. | History is immutable. A ref moves only from its expected old value. |
| Scheduling (planned) | Petri nets over derivations, for approvals, budgets/resources and retries. Markings are content-addressed. | A transition fires only when enabled; every firing records `marking_before` and `marking_after`. |

Postgres, pgvector and any graph database are **projections** that can be rebuilt from objects and refs. They are never a source of truth.

## L0 encoding (normative)

All lengths are u32 big-endian.

```
blob = "B" bytes
node = "N" count { label_len label(utf-8) kind("B"|"N") id[32] }
```

- The kind tag inside the hashed bytes prevents a blob from colliding with a node.
- Labels are metadata and part of node identity, but never of leaf identity, so identical paragraphs under different headings still dedup.
- Media type is not part of identity.

## Design decisions

- **Rendering is not pluggable.** Codecs only split. That lets the guarantee hold for heuristic and AI codecs alike: the LLM boundary planner from `forja/boundary_planner_llm.py` becomes a codec whose output is checked like any other. Transforming renderings (normalization, canonical-plus-patch) belong to L2/L3 as derivations with their own verified law.
- **The round trip is checked at ingest** even though tiling makes it hold by construction, so the guarantee rests on a byte comparison.
- **Objects written by a rejected shape** are left for a future GC (mark from refs).

## Measured (2026-10-02, `packages/kernel`, release build)

- **Fixture corpus** (`tests/fixtures/cases`, 344 md files, 180 KB): 0 round-trip failures, 0 fallbacks. The md codec stores 1.70× the input, versus 1.00× raw.
  - Files average about 520 B and rarely share text, so the ~38 B-per-entry overhead (4 B label length, 1 B kind, 32 B id, 1 B blob tag) dominates.
  - Removing heading labels only gets to 1.55×. The fix is a minimum leaf size: merge small paragraphs, as CDC does.
- **pdf+xlsx** (220 files, 967 KB): cdc 1.01×, raw 1.00×. These are compressed containers, so byte-level dedup cannot help. They need format codecs (unzip xlsx parts).
- **Edit history** (21 synthetic revisions of a 30 KB markdown spec, one-line edits plus appended sections): raw 1.00×, cdc 0.39×, md **0.15×**.

These are the only measurements so far. The edit history is synthetic.

## Salvage inventory from the Python packages

The Python packages are stripped to their ideas, not ported line-for-line.

| Source | Idea taken | Kernel home |
|---|---|---|
| `ikam/forja/boundary_planner*.py` | AI proposes spans, deterministic validator checks full coverage | L1 codec + `tiles()` |
| `ikam/forja/verifier.py` (`ByteIdentityVerifier`) | Byte identity as the only pass criterion | `ingest` |
| `ikam/delta_chain.py` | Bounded delta chains with rebase | L2 residual patches |
| `ikam/fragments.py` relations + `relation_eval.py` | Operator identified by function hash plus slot bindings | L3 derivation key |
| `modelado/graph_edge_event_log.py`, `graph_edge_event_folding.py` | Idempotency keys; deterministic fold; projections with checkpoints | Projections over refs/objects |
| `modelado/history/head_locators.py`, `ikam_fragment_objects` | Refs → commits → immutable manifests | Versioning |
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

1. **L1:** set a minimum leaf size in the md codec; add a JSON codec; add an xlsx/docx/pptx codec that splits zip parts. Re-measure on the fixtures.
2. **Versioning:** a `Commit` object kind, a refs file with compare-and-swap, `ikam log`, and a GC that marks from refs.
3. **L3:** a derivation record plus a memo table; effect tags; WASM function objects (wasmtime).
4. **L2:** residual patches (`Patch` derivation), then AI-proposed canonical forms gated by the patch law.
5. **Scheduling:** a Petri net over derivations; port the enabling/firing semantics from `modelado/plans/engine.py`.
6. **Bindings:** PyO3 bindings so remaining Python code can call the kernel while it is retired.
