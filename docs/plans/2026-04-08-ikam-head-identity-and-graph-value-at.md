# IKAM Head Identity and Graph Value At Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Unify IKAM fragment/artifact identification and head resolution around append-only history, explicit semantic identity, ref-scoped head locators, and a first-class `graph:value_at` graph family that can be replayed into HugeGraph without conflating graph-native addressed content with derivation edges.

**Architecture:** Reuse the repo's existing immutable fragment/object layer, canonical `refs/heads/...` scope model, `head_object_id` artifact manifest heads, and in-memory commit/ref-head primitives. Add a resolver-first layer that makes `artifact_head_ref` and `subgraph_ref` true ref-like locators, then migrate read-time consumers to that resolver before aligning graph replay, `graph:value_at`, and HugeGraph projection. Avoid parallel truth systems: Postgres remains authoritative append-only history, HugeGraph remains a replayable graph index, and read-time head resolution chooses effective state.

**Tech Stack:** Python, Pydantic, psycopg, pytest, Postgres schema in `packages/modelado`, IKAM IR/runtime models in `packages/ikam`, HugeGraph projection in `packages/modelado`

---

### Task 1: Lock locator grammar and resolver contracts with failing tests

**Files:**
- Create: `packages/modelado/tests/test_head_locator_resolution.py`
- Modify: `packages/modelado/tests/test_transition_validation.py`
- Modify: `packages/modelado/tests/test_graph_slice_delta_integration.py`
- Test: `packages/modelado/tests/test_head_locator_resolution.py`
- Test: `packages/modelado/tests/test_transition_validation.py`
- Test: `packages/modelado/tests/test_graph_slice_delta_integration.py`

**Step 1: Write the failing test changes**

Add tests that assert the canonical locator grammar and resolution rules:
- explicit artifact locator form: `ref://refs/heads/main/artifact/<semantic_artifact_id>`
- shorthand artifact locator form: `artifact://<semantic_artifact_id>`
- explicit subgraph locator form: `ref://refs/heads/main/subgraph/<semantic_subgraph_id>`
- shorthand subgraph locator form: `subgraph://<semantic_subgraph_id>`
- shorthand resolution uses only `EnvironmentScope.ref`
- shorthand resolution does **not** consult `base_refs`
- explicit locator uses its embedded ref exactly
- unresolved shorthand in current ref fails closed

Concrete failing tests to add:
- `test_parse_explicit_artifact_locator`
- `test_parse_shorthand_artifact_locator`
- `test_parse_explicit_subgraph_locator`
- `test_shorthand_artifact_locator_uses_current_env_ref_only`
- `test_shorthand_artifact_locator_does_not_fallback_to_base_refs`
- `test_transition_validation_resolves_artifact_head_ref_via_shared_rules`

**Step 2: Run tests to verify they fail**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_head_locator_resolution.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_graph_slice_delta_integration.py -q`
Expected: FAIL because no shared locator parser/resolver exists yet and `artifact_head_ref` / `subgraph_ref` are still treated as loose payload strings.

**Step 3: Write minimal implementation**

Planned code change:
- add a shared locator parser/resolver module under `packages/modelado/src/modelado/history/` or `packages/modelado/src/modelado/ikam_*`
- define explicit and shorthand locator parsing
- define `resolve_artifact_head(...)` / `resolve_subgraph_head(...)` contracts
- keep implementation read-only/minimal at this stage; do not yet refactor all call sites

**Step 4: Run tests to verify they pass**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_head_locator_resolution.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_graph_slice_delta_integration.py -q`
Expected: PASS once canonical parsing and resolution rules exist.

**Step 5: Commit**

Run: `git add packages/modelado/tests/test_head_locator_resolution.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_graph_slice_delta_integration.py packages/modelado/src/modelado/history && git commit -m "feat: add head locator resolution contracts"`

### Task 2: Wire ref-scoped artifact head persistence and resolution on top of existing branch/workspace/commit tables

**Files:**
- Modify: `packages/modelado/src/modelado/ikam_artifact_store_pg.py`
- Modify: `packages/modelado/src/modelado/db.py`
- Modify: `packages/modelado/src/modelado/history/ref_head.py`
- Modify: `packages/modelado/src/modelado/history/commit_entry.py`
- Modify: `packages/modelado/src/modelado/history/head_locators.py`
- Modify: `packages/modelado/src/modelado/operators/commit.py`
- Test: `packages/modelado/tests/test_commit_history_dag.py`
- Create or Modify: `packages/modelado/tests/test_artifact_head_resolution.py`

**Step 1: Write the failing test changes**

Add tests that assert:
- explicit artifact locator uses embedded ref exactly
- shorthand artifact locator uses only current `EnvironmentScope.ref`
- moving a branch/workspace head changes the resolved `head_object_id`
- resolution fails closed when the target ref has no artifact head
- `root_fragment_id` is not consulted as current-head truth

Concrete failing tests:
- `test_resolve_artifact_head_uses_branch_head_commit_for_explicit_ref`
- `test_resolve_artifact_head_uses_current_env_ref_for_shorthand`
- `test_resolve_artifact_head_fails_when_ref_has_no_head`
- `test_commit_updates_ref_head_to_new_head_object_id`
- `test_resolve_artifact_head_does_not_use_root_fragment_id_as_head`

**Step 2: Run tests to verify they fail**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_artifact_head_resolution.py packages/modelado/tests/test_commit_history_dag.py -q`
Expected: FAIL until runtime persistence actually resolves branch/workspace heads to `head_object_id`.

**Step 3: Write minimal implementation**

Planned code change:
- keep `head_object_id` as the canonical resolved artifact head target
- wire the smallest persistence/runtime path that connects locator ref, artifact branch/workspace head, commit/result ref, and resolved `head_object_id`
- reuse the existing `ikam_artifact_branches`, `ikam_artifact_commits`, and `ikam_artifact_workspaces` tables
- do not introduce a new parallel artifact-head table
- leave `root_fragment_id` in place only if still needed as reconstruction metadata, not current-head truth

**Step 4: Run tests to verify they pass**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_artifact_head_resolution.py packages/modelado/tests/test_commit_history_dag.py -q`
Expected: PASS once ref-scoped artifact head resolution is real and yields `head_object_id` deterministically.

**Step 5: Commit**

Run: `git add packages/modelado/src/modelado/ikam_artifact_store_pg.py packages/modelado/src/modelado/db.py packages/modelado/src/modelado/history/ref_head.py packages/modelado/src/modelado/history/commit_entry.py packages/modelado/src/modelado/history/head_locators.py packages/modelado/src/modelado/operators/commit.py packages/modelado/tests/test_artifact_head_resolution.py packages/modelado/tests/test_commit_history_dag.py && git commit -m "feat: wire ref scoped artifact head resolution"`

### Task 3: Move `subgraph_ref` to canonical ref-like locator grammar

**Dependency note:** Runtime emitters that produce `artifact_head_ref` should not be considered complete until Task 2's ref-scoped artifact head resolution is wired through persistence/runtime.

**Files:**
- Modify: `packages/modelado/src/modelado/hot_subgraph_store.py`
- Modify: `packages/modelado/src/modelado/oraculo/persistent_graph_state.py`
- Modify: `packages/modelado/src/modelado/inspection_runtime.py`
- Modify: `packages/modelado/src/modelado/executors/transition_validation.py`
- Test: `packages/modelado/tests/test_persistent_inspection_runtime.py`
- Test: `packages/modelado/tests/test_transition_validation.py`
- Modify: `packages/modelado/tests/test_graph_slice_delta_integration.py`

**Step 1: Write the failing test changes**

Add tests that assert:
- explicit subgraph locators round-trip through runtime outputs
- shorthand subgraph locators resolve only in current `EnvironmentScope.ref`
- runtime payloads stop depending on ad hoc `hot://...` semantics as the canonical form
- legacy `hot://...` input is either normalized or rejected according to the compatibility policy chosen during implementation

Concrete failing tests:
- `test_resolve_explicit_subgraph_locator`
- `test_resolve_shorthand_subgraph_locator_uses_current_ref_only`
- `test_transition_validation_normalizes_subgraph_ref_to_locator`

**Step 2: Run tests to verify they fail**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_persistent_inspection_runtime.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_graph_slice_delta_integration.py -q`
Expected: FAIL until runtime code stops treating `subgraph_ref` as an unstructured string.

**Step 3: Write minimal implementation**

Planned code change:
- add canonical subgraph locator parse/normalize logic to the shared resolver layer
- update read/write runtime paths to emit the canonical locator form
- keep the change as narrow as possible; do not redesign hot-store internals beyond what is needed to resolve locators

**Step 4: Run tests to verify they pass**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_persistent_inspection_runtime.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_graph_slice_delta_integration.py -q`
Expected: PASS once subgraph references are normalized and resolved by shared rules.

**Step 5: Commit**

Run: `git add packages/modelado/src/modelado/hot_subgraph_store.py packages/modelado/src/modelado/oraculo/persistent_graph_state.py packages/modelado/src/modelado/inspection_runtime.py packages/modelado/src/modelado/executors/transition_validation.py packages/modelado/tests/test_persistent_inspection_runtime.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_graph_slice_delta_integration.py && git commit -m "refactor: normalize subgraph head locators"`

### Task 4: Rename `graph:contains` to `graph:value_at` and define graph slot identity

**Files:**
- Modify: `packages/ikam/src/ikam/ir/graph_native.py`
- Modify: `packages/modelado/src/modelado/graph/delta_lowering.py`
- Modify: `packages/modelado/src/modelado/graph_edge_event_log.py`
- Modify: `packages/modelado/src/modelado/graph_edge_event_folding.py`
- Modify: `packages/modelado/src/modelado/graph_edge_projection_replay.py`
- Modify: `packages/modelado/src/modelado/ikam_operator_surface.py`
- Test: `packages/modelado/tests/test_graph_delta_lowering.py`
- Test: `packages/modelado/tests/test_graph_edge_event_folding.py`
- Test: `packages/modelado/tests/test_graph_edge_projection_replay.py`
- Test: `packages/modelado/tests/test_graph_slice_delta_integration.py`

**Step 1: Write the failing test changes**

Update tests to assert:
- graph-native addressed content uses `graph:value_at`
- semantic graph slot identity is based on the addressed slot (`handle + path`, scoped as needed), not on event identity
- subtree remove semantics still work after the rename
- inline payloads are still allowed

Concrete failing tests:
- rename current `graph:contains` assertions to `graph:value_at`
- add `test_graph_value_at_slot_identity_stable_across_revisions`

**Step 2: Run tests to verify they fail**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_graph_delta_lowering.py packages/modelado/tests/test_graph_edge_event_folding.py packages/modelado/tests/test_graph_edge_projection_replay.py packages/modelado/tests/test_graph_slice_delta_integration.py -q`
Expected: FAIL on old label assumptions and slot identity semantics.

**Step 3: Write minimal implementation**

Planned code change:
- rename the graph-native edge family to `graph:value_at`
- keep lowering/folding/replay semantics minimal and deterministic
- make graph slot identity explicit through existing handle/path metadata rather than inventing a second competing address scheme

**Step 4: Run tests to verify they pass**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_graph_delta_lowering.py packages/modelado/tests/test_graph_edge_event_folding.py packages/modelado/tests/test_graph_edge_projection_replay.py packages/modelado/tests/test_graph_slice_delta_integration.py -q`
Expected: PASS once rename and slot identity behavior are consistent.

**Step 5: Commit**

Run: `git add packages/ikam/src/ikam/ir/graph_native.py packages/modelado/src/modelado/graph/delta_lowering.py packages/modelado/src/modelado/graph_edge_event_log.py packages/modelado/src/modelado/graph_edge_event_folding.py packages/modelado/src/modelado/graph_edge_projection_replay.py packages/modelado/src/modelado/ikam_operator_surface.py packages/modelado/tests/test_graph_delta_lowering.py packages/modelado/tests/test_graph_edge_event_folding.py packages/modelado/tests/test_graph_edge_projection_replay.py packages/modelado/tests/test_graph_slice_delta_integration.py && git commit -m "refactor: rename graph addressed content edge"`

### Task 5: Make HugeGraph project `graph:value_at` as a first-class graph family

**Files:**
- Modify: `packages/modelado/src/modelado/hugegraph_projection.py`
- Test: `packages/modelado/tests/test_hugegraph_projection.py`
- Modify: `docs/ikam/HUGEGRAPH_SCHEMA_AND_LOADER_FORMAT.md`
- Modify: `docs/ikam/GRAPH_EDGE_EVENT_LOG.md`

**Step 1: Write the failing test changes**

Add tests that assert:
- `graph:value_at` does not fall back to `Derivation`
- HugeGraph projection label/schema for addressed graph content is first-class
- artifact/subgraph locator metadata needed for queryability survives projection
- projector scope filtering still works as documented

Concrete failing tests:
- `test_replay_maps_graph_value_at_to_first_class_edge_label`
- `test_projection_respects_edge_label_prefix_for_graph_value_at`
- `test_projection_preserves_graph_slot_metadata`

**Step 2: Run tests to verify they fail**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_hugegraph_projection.py -q`
Expected: FAIL until HugeGraph gets a first-class graph-native edge family and correct replay scoping.

**Step 3: Write minimal implementation**

Planned code change:
- add a first-class HugeGraph edge label for graph-native addressed content
- stop mapping it to `Derivation`
- preserve enough metadata to identify slot and replay history
- keep Postgres as authoritative; HugeGraph remains a replayable graph index

**Step 4: Run tests to verify they pass**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_hugegraph_projection.py -q`
Expected: PASS once HugeGraph projects `graph:value_at` coherently.

**Step 5: Commit**

Run: `git add packages/modelado/src/modelado/hugegraph_projection.py packages/modelado/tests/test_hugegraph_projection.py docs/ikam/HUGEGRAPH_SCHEMA_AND_LOADER_FORMAT.md docs/ikam/GRAPH_EDGE_EVENT_LOG.md && git commit -m "feat: project graph value edges into hugegraph"`

### Task 6: Migrate read-time consumers to shared resolver semantics

**Files:**
- Modify: `packages/modelado/src/modelado/executors/transition_validation.py`
- Modify: `packages/modelado/src/modelado/inspection_runtime.py`
- Modify: `packages/modelado/src/modelado/ikam_inference.py`
- Modify: `packages/modelado/src/modelado/knowledge_base/lineage.py`
- Test: `packages/modelado/tests/test_transition_validation.py`
- Create or Modify: targeted tests for inference/lineage resolution behavior

**Step 1: Write the failing test changes**

Add tests that assert:
- consumers no longer invent their own conflicting "latest/current/effective" logic
- artifact heads and subgraph heads are resolved through the shared resolver
- graph slot/effective queries use one canonical resolution path

Concrete failing tests:
- `test_transition_validation_uses_shared_artifact_head_resolution`
- `test_lineage_uses_resolved_head_in_selected_ref`
- `test_inference_does_not_group_by_ad_hoc_latest_rules`

**Step 2: Run tests to verify they fail**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_hugegraph_projection.py packages/modelado/tests/test_graph_slice_delta_integration.py -q`
Expected: FAIL until consumers are switched to the resolver and old local heuristics are removed.

**Step 3: Write minimal implementation**

Planned code change:
- route read-time resolution through the shared resolver
- remove or narrow ad hoc latest/effective grouping logic where canonical resolution should apply
- do this only after Task 2 and Task 3 are complete so consumers are not normalized against partial artifact-head semantics

**Step 4: Run tests to verify they pass**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_hugegraph_projection.py packages/modelado/tests/test_graph_slice_delta_integration.py -q`
Expected: PASS once shared resolution rules are used across consumers.

**Step 5: Commit**

Run: `git add packages/modelado/src/modelado/executors/transition_validation.py packages/modelado/src/modelado/inspection_runtime.py packages/modelado/src/modelado/ikam_inference.py packages/modelado/src/modelado/knowledge_base/lineage.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests && git commit -m "refactor: unify read time head resolution"`

### Task 7: Run final targeted verification and remove/mark deprecated concepts

**Files:**
- Modify: docs and compatibility comments in files touched above
- Verify: targeted suites covering resolver, artifact heads, subgraph locators, graph value replay, HugeGraph projection, and read-time consumers

**Step 1: Write the failing test/documentation cleanup assertions**

Add or update tests/docs to assert the deprecated concepts are no longer primary:
- `graph:contains` no longer appears in active runtime contracts
- `artifact_head_ref` is documented as a locator
- `subgraph_ref` is documented as a locator
- `head_object_id` is documented as the canonical resolved artifact head target

**Step 2: Run verification to confirm remaining gaps**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_head_locator_resolution.py packages/modelado/tests/test_artifact_head_resolution.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_graph_delta_lowering.py packages/modelado/tests/test_graph_edge_event_folding.py packages/modelado/tests/test_graph_edge_projection_replay.py packages/modelado/tests/test_graph_slice_delta_integration.py packages/modelado/tests/test_hugegraph_projection.py packages/modelado/tests/test_commit_history_dag.py -q`
Expected: any remaining failures should point to unfinished compatibility or consumer cleanup.

**Step 3: Write minimal implementation / documentation cleanup**

Planned code/doc change:
- remove or clearly mark deprecated aliases/comments
- update docs to reflect the canonical model and compatibility boundaries

**Step 4: Run tests to verify they pass**

Run: `python3 -m pytest --noconftest packages/modelado/tests/test_head_locator_resolution.py packages/modelado/tests/test_artifact_head_resolution.py packages/modelado/tests/test_transition_validation.py packages/modelado/tests/test_graph_delta_lowering.py packages/modelado/tests/test_graph_edge_event_folding.py packages/modelado/tests/test_graph_edge_projection_replay.py packages/modelado/tests/test_graph_slice_delta_integration.py packages/modelado/tests/test_hugegraph_projection.py packages/modelado/tests/test_commit_history_dag.py -q`
Expected: PASS across the targeted identity/head/graph-value suite.

**Step 5: Commit**

Run: `git add docs/ikam/GRAPH_EDGE_EVENT_LOG.md docs/ikam/HUGEGRAPH_SCHEMA_AND_LOADER_FORMAT.md docs/plans/2026-04-08-ikam-head-identity-and-graph-value-at.md packages/modelado packages/ikam && git commit -m "docs: finalize ikam head and graph identity model"`

---

## Migration Notes

- Avoid creating a second branch/ref namespace; reuse `refs/heads/...` only.
- Avoid creating a second artifact-head persistence model; build on `head_object_id`.
- Treat `ikam_artifact_fragments` as a possible denormalized read model until proven removable.
- Keep Postgres append-only history authoritative; HugeGraph must remain rebuildable from that history.
- Do not let shorthand locators silently fall back to `base_refs`.
- Inline payloads in `graph:value_at` are allowed, but translator/lowering should prefer `ExpressionIR`, `ClaimIR`, and `StructuredDataIR` when semantically justified.

## Execution Order

Recommended order when implementing:
1. Task 1
2. Task 2
3. Task 3
4. Task 4
5. Task 5
6. Task 6
7. Task 7

This order minimizes semantic ambiguity before changing graph replay/projection behavior.
