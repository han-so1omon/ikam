# Fragment-Target Inference and Lineage Typing Fix Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the recent locator-aware behavior internally consistent by aligning inference with mixed artifact/fragment `graph_edge_events` endpoints and emitting real fragment node types for fragment-rooted lineage traversals.

**Architecture:** Keep the existing shared locator helpers. Fix the two review findings by tightening contracts around read-time graph targets instead of adding new abstractions. For inference, prove fragment-target ancestry against mixed endpoint `graph_edge_events` and update docs/tests accordingly. For lineage, preserve current traversal shape but emit `type: "fragment"` when the resolved root target is fragment-backed.

**Tech Stack:** Python, pytest, Postgres-backed query helpers, shared locator utilities in `modelado.history.head_locators`

---

### Task 1: Lock mixed-endpoint inference expectations with failing tests

**Files:**
- Modify: `packages/modelado/tests/test_ikam_inference_locator_resolution.py`
- Read for context: `packages/modelado/src/modelado/ikam_inference.py`
- Read for context: `docs/ikam/GRAPH_EDGE_EVENT_LOG.md`

**Step 1: Write the failing test**

Add focused tests that prove fragment-target inference is allowed to traverse `graph_edge_events` using fragment ids.

Cover at least these cases:
- fragment-target completion accepts fragment ids in derivation edges
- fragment-target variation remains valid
- returned support/reasoning remains consistent with fragment-target ancestry

**Step 2: Run test to verify it fails**

Run:
```bash
PYTHONPATH="packages/modelado/src:packages/ikam/src:packages/interacciones/schemas/src" python3 -m pytest --noconftest packages/modelado/tests/test_ikam_inference_locator_resolution.py -q
```

Expected: FAIL because current tests/docs do not yet prove the mixed-endpoint contract clearly enough, or inference wording/behavior still assumes artifact-only ancestry.

**Step 3: Write minimal implementation**

If needed, modify only `packages/modelado/src/modelado/ikam_inference.py`.

Constraints:
- do not redesign `_fetch_effective_derivation_edges(...)`
- do not add new target kinds
- keep the change local to fragment-target inference consistency

**Step 4: Run test to verify it passes**

Run the same command again.

Expected: PASS

### Task 2: Fix lineage node typing for fragment-rooted traversals

**Files:**
- Modify: `packages/modelado/src/modelado/knowledge_base/lineage.py`
- Modify: `packages/modelado/tests/test_lineage_locator_resolution.py`

**Step 1: Write the failing test**

Add tests that assert:
- `artifact://...` resolving to a head fragment produces `rootId == <fragment_id>`
- the root node has `type == "fragment"`
- direct `fragment://...` roots also produce `type == "fragment"`
- raw artifact-id roots still produce `type == "artifact"`

**Step 2: Run test to verify it fails**

Run:
```bash
PYTHONPATH="packages/modelado/src:packages/ikam/src:packages/interacciones/schemas/src" python3 -m pytest --noconftest packages/modelado/tests/test_lineage_locator_resolution.py -q
```

Expected: FAIL because `ensure_node()` currently hardcodes `type: "artifact"`.

**Step 3: Write minimal implementation**

Modify `packages/modelado/src/modelado/knowledge_base/lineage.py` to:
- resolve the root target once
- carry enough metadata to type the root node truthfully
- emit `fragment` for fragment-rooted nodes
- keep downstream traversal behavior unchanged unless tests force broader typing

**Step 4: Run test to verify it passes**

Run the same command again.

Expected: PASS

### Task 3: Align docs with the intended mixed endpoint contract

**Files:**
- Modify: `docs/ikam/GRAPH_EDGE_EVENT_LOG.md`
- Optional: `docs/ikam/HUGEGRAPH_SCHEMA_AND_LOADER_FORMAT.md`

**Step 1: Write the documentation assertions**

Confirm the updated docs will state:
- `graph_edge_events.out_id/in_id` are not documented as artifact-only
- mixed artifact/fragment endpoints are allowed for evolving producers
- read-time locator lowering language remains accurate
- docs do not falsely claim all producers already emit mixed endpoints

**Step 2: Write minimal documentation update**

Update wording so the contract says:
- `out_id` and `in_id` are stable graph vertex ids
- they may be artifact ids or fragment ids depending on producer/edge family
- derivation consumers must interpret them in context

**Step 3: Verify the docs manually**

Re-read the modified sections and check they no longer contradict the intended inference behavior.

### Task 4: Run targeted regression suite

**Files / tests:**
- `packages/modelado/tests/test_head_locator_resolution.py`
- `packages/modelado/tests/test_transition_validation.py`
- `packages/modelado/tests/test_inspection_runtime.py`
- `packages/modelado/tests/test_lineage_locator_resolution.py`
- `packages/modelado/tests/test_ikam_inference_locator_resolution.py`

**Step 1: Run targeted suite**

Run:
```bash
PYTHONPATH="packages/modelado/src:packages/ikam/src:packages/interacciones/schemas/src" python3 -m pytest --noconftest packages/modelado/tests/test_head_locator_resolution.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_inspection_runtime.py packages/modelado/tests/test_lineage_locator_resolution.py packages/modelado/tests/test_ikam_inference_locator_resolution.py -q
```

Expected: PASS

### Task 5: Run expanded verification suite

**Step 1: Run expanded suite**

Run:
```bash
PYTHONPATH="packages/modelado/src:packages/ikam/src:packages/interacciones/schemas/src" python3 -m pytest --noconftest packages/modelado/tests/test_head_locator_resolution.py packages/modelado/tests/test_artifact_head_resolution.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_inspection_runtime.py packages/modelado/tests/test_lineage_locator_resolution.py packages/modelado/tests/test_ikam_inference_locator_resolution.py packages/modelado/tests/test_graph_delta_lowering.py packages/modelado/tests/test_graph_edge_event_folding.py packages/modelado/tests/test_graph_edge_projection_replay.py packages/modelado/tests/test_graph_slice_delta_integration.py packages/modelado/tests/test_hugegraph_projection.py packages/modelado/tests/test_commit_history_dag.py -q
```

Expected: PASS, aside from the existing known warning unless intentionally addressed.

### Task 6: Review residual compatibility risks before commit

**Files:**
- Review only

**Step 1: Review checklist**

Confirm:
- `lineage.py` root node type is truthful for fragment targets
- fragment-target inference is backed by a real mixed-endpoint test, not only monkeypatched assumptions
- docs say mixed endpoints are allowed/intended without overclaiming universal adoption
- no new local locator wrappers were introduced

**Step 2: Optional commit later**

Only if requested later:
```bash
git add docs/ikam/GRAPH_EDGE_EVENT_LOG.md docs/ikam/HUGEGRAPH_SCHEMA_AND_LOADER_FORMAT.md packages/modelado/src/modelado/knowledge_base/lineage.py packages/modelado/src/modelado/ikam_inference.py packages/modelado/tests/test_lineage_locator_resolution.py packages/modelado/tests/test_ikam_inference_locator_resolution.py
git commit -m "fix: align fragment target inference and lineage typing"
```

---

## Minimal Approach

- Do not redesign event replay.
- Do not add new target kinds.
- Do not broaden lineage typing beyond what tests require.
- Prefer root-node typing correctness plus contract/doc alignment.

## Risks / Unknowns

- The main unknown is whether downstream nodes in fragment-rooted lineage traversals also need mixed typing now. This plan keeps the change to root-node correctness unless tests prove more is needed.
- Docs should say mixed endpoints are an allowed evolving contract, not that every producer already emits them today.
