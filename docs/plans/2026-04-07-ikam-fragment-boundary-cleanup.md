# IKAM Fragment Boundary Cleanup Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Clarify IKAM's fragment architecture by making `ikam.fragments.Fragment` the primary runtime/domain fragment type, renaming the storage-layer `ikam.graph.Fragment`, and consolidating all domain/storage conversion behind a single canonical adapter boundary.

**Architecture:** Keep the two-layer architecture because the responsibilities are genuinely different: domain fragments represent semantic/runtime values and relation payloads, while storage fragments represent raw immutable CAS records. Reduce confusion by giving only one type the public name `Fragment`, renaming the storage record to a clearly storage-oriented type, and updating repositories/helpers/tests to use `ikam.adapters` as the sole conversion layer.

**Tech Stack:** Python, Pydantic, psycopg, pytest, BLAKE3, IKAM package structure under `packages/ikam` and `packages/modelado`

---

### Task 1: Lock the public API decision with failing contract tests

**Files:**
- Modify: `packages/ikam/tests/test_v3_api_contract.py`
- Modify: `packages/ikam/tests/test_graph_model_cleanup.py`
- Modify: `packages/ikam/tests/test_graph_models.py`
- Test: `packages/ikam/tests/test_v3_api_contract.py`
- Test: `packages/ikam/tests/test_graph_model_cleanup.py`
- Test: `packages/ikam/tests/test_graph_models.py`

**Step 1: Write the failing test changes**

Update contract expectations so they assert:
- `ikam.fragments.Fragment` remains the primary fragment type
- `ikam.graph` exports the renamed storage record type, recommended as `StoredFragment`
- `ikam.graph.Fragment` is absent in clean-break mode
- storage-model behavior tests target `StoredFragment.from_bytes(...)`

Concrete test targets to update:
- `test_storage_fragment_api_importable`
- `test_fragment_still_exists`
- `test_fragment_cas_and_roundtrip`

**Step 2: Run tests to verify they fail**

Run: `pytest packages/ikam/tests/test_v3_api_contract.py packages/ikam/tests/test_graph_model_cleanup.py packages/ikam/tests/test_graph_models.py -v`
Expected: FAIL on imports still expecting `ikam.graph.Fragment` and FAIL on missing `StoredFragment` until implementation lands.

**Step 3: Write minimal implementation**

Planned code change:
- rename `packages/ikam/src/ikam/graph.py:23-52` storage class from `Fragment` to `StoredFragment`
- update `__all__` accordingly
- do not keep a compatibility alias in clean-break mode

**Step 4: Run tests to verify they pass**

Run: `pytest packages/ikam/tests/test_v3_api_contract.py packages/ikam/tests/test_graph_model_cleanup.py packages/ikam/tests/test_graph_models.py -v`
Expected: PASS once storage record naming and exports are aligned.

**Step 5: Commit**

Run: `git add packages/ikam/tests/test_v3_api_contract.py packages/ikam/tests/test_graph_model_cleanup.py packages/ikam/tests/test_graph_models.py packages/ikam/src/ikam/graph.py && git commit -m "refactor: clarify storage fragment api"`

### Task 2: Make `ikam.adapters` the only canonical conversion layer

**Files:**
- Modify: `packages/ikam/src/ikam/adapters.py`
- Modify: `packages/ikam/src/ikam/forja/cas.py`
- Test: `packages/modelado/tests/test_ikam_v3_adapter_roundtrip.py`
- Test: `packages/ikam/tests/test_cas_fragment.py`

**Step 1: Write the failing test changes**

Add or update tests to assert:
- `v3_to_storage(...)` returns `StoredFragment`
- `cas_fragment(...)` produces the same `cas_id` and canonical bytes as `v3_fragment_to_cas_bytes(...)`
- no independent canonicalization logic is needed outside `ikam.adapters`

Likely test edits:
- `packages/modelado/tests/test_ikam_v3_adapter_roundtrip.py:148-205`
- `packages/ikam/tests/test_cas_fragment.py`

**Step 2: Run tests to verify they fail**

Run: `pytest packages/modelado/tests/test_ikam_v3_adapter_roundtrip.py packages/ikam/tests/test_cas_fragment.py -v`
Expected: FAIL until renamed storage type and unified helper usage are wired through.

**Step 3: Write minimal implementation**

Planned code changes:
- in `packages/ikam/src/ikam/adapters.py`
  - import `StoredFragment` instead of `Fragment as StorageFragment`
  - return `StoredFragment` from `v3_to_storage(...)`
- in `packages/ikam/src/ikam/forja/cas.py`
  - replace duplicated canonicalization/hash logic with a call path through `ikam.adapters`
  - keep `cas_fragment(...)` as a convenience wrapper only

**Step 4: Run tests to verify they pass**

Run: `pytest packages/modelado/tests/test_ikam_v3_adapter_roundtrip.py packages/ikam/tests/test_cas_fragment.py -v`
Expected: PASS, with one canonical serialization path.

**Step 5: Commit**

Run: `git add packages/ikam/src/ikam/adapters.py packages/ikam/src/ikam/forja/cas.py packages/modelado/tests/test_ikam_v3_adapter_roundtrip.py packages/ikam/tests/test_cas_fragment.py && git commit -m "refactor: centralize fragment storage adapters"`

### Task 3: Remove repository-private CAS/domain conversion helpers

**Files:**
- Modify: `packages/modelado/src/modelado/ikam_graph_repository.py`
- Modify: `packages/modelado/src/modelado/ikam_graph_repository_async.py`
- Test: `packages/modelado/tests/test_repository_integration.py`
- Test: `packages/modelado/tests/test_ikam_write_scope_guards.py`

**Step 1: Write the failing test changes**

Add/update tests so repositories are validated through the public adapter path rather than internal helper assumptions:
- raw CAS insert path uses `StoredFragment`
- domain fragment persistence uses `ikam.adapters.v3_fragment_to_cas_bytes` / `v3_to_storage`
- async repository no longer imports private sync helper functions

Concrete targets:
- `packages/modelado/tests/test_repository_integration.py:86-115`
- `packages/modelado/tests/test_ikam_write_scope_guards.py:1-48`

**Step 2: Run tests to verify they fail**

Run: `pytest packages/modelado/tests/test_repository_integration.py packages/modelado/tests/test_ikam_write_scope_guards.py -v`
Expected: FAIL while imports and helper usage still depend on `ikam.graph.Fragment` and repo-private conversion logic.

**Step 3: Write minimal implementation**

Planned code changes:
- remove or shrink these helpers in `packages/modelado/src/modelado/ikam_graph_repository.py`
  - `_generate_cas_bytes`
  - `_cas_id_for_domain_fragment`
  - `_build_domain_id_to_cas_id_map`
  - `_domain_fragment_from_cas_bytes`
- replace their call sites with `ikam.adapters` functions
- in `packages/modelado/src/modelado/ikam_graph_repository_async.py`
  - stop importing private helpers from sync repo
  - use adapter functions directly

**Step 4: Run tests to verify they pass**

Run: `pytest packages/modelado/tests/test_repository_integration.py packages/modelado/tests/test_ikam_write_scope_guards.py -v`
Expected: PASS with repositories using a single canonical boundary.

**Step 5: Commit**

Run: `git add packages/modelado/src/modelado/ikam_graph_repository.py packages/modelado/src/modelado/ikam_graph_repository_async.py packages/modelado/tests/test_repository_integration.py packages/modelado/tests/test_ikam_write_scope_guards.py && git commit -m "refactor: remove duplicate repository fragment codecs"`

### Task 4: Fix stale artifact-store boundary usage

**Files:**
- Modify: `packages/modelado/src/modelado/ikam_artifact_store_pg.py`
- Grep reference: `packages/ikam/src/ikam/adapters.py`
- Test: relevant artifact store tests if present, otherwise add one in `packages/modelado/tests/`

**Step 1: Write the failing test**

Add or update a test that exercises:
- `PostgresArtifactStore._insert_fragment_object(...)`
- `build_fragment_object_manifest(...)` using only currently supported parameters:
  - `artifact_id`
  - `kind`
  - `fragment_ids`

This should explicitly fail if stale params like `fragments` or `domain_id_to_cas_id` are still passed through.

**Step 2: Run test to verify it fails**

Run: `pytest packages/modelado/tests -k "artifact_store or fragment_object_manifest" -v`
Expected: FAIL on argument mismatch or stale call path.

**Step 3: Write minimal implementation**

Planned code change:
- update `packages/modelado/src/modelado/ikam_artifact_store_pg.py:133-140`
- remove unsupported args from `build_fragment_object_manifest(...)` call
- if extra data is still needed, make it explicit in manifest-building API rather than passing dead parameters

**Step 4: Run test to verify it passes**

Run: `pytest packages/modelado/tests -k "artifact_store or fragment_object_manifest" -v`
Expected: PASS with current adapter signature.

**Step 5: Commit**

Run: `git add packages/modelado/src/modelado/ikam_artifact_store_pg.py packages/modelado/tests && git commit -m "fix: align artifact store with manifest adapter api"`

### Task 5: Quarantine or remove legacy relation-query path

**Files:**
- Modify or delete: `packages/modelado/src/modelado/fragment_relation_queries.py`
- Test: any existing tests for this module, or add targeted compatibility/deprecation coverage

**Step 1: Write the failing test**

Decide one of two minimal test directions:
- if removing:
  - add guard coverage that no active V3 path depends on `fragment_relation_queries`
- if quarantining:
  - add a test asserting it is explicitly marked legacy and isolated from V3 claims

**Step 2: Run test to verify it fails**

Run: `pytest packages/modelado/tests -k "relation_queries or legacy" -v`
Expected: FAIL until the module's status is explicit.

**Step 3: Write minimal implementation**

Planned code change:
- preferred: isolate as legacy compatibility only with explicit module docstring and no new callers
- stronger option: delete if no active callers remain

Reason:
- current implementation depends on old `ikam_fragment_meta` / `ikam_fragment_content` schema and pre-V3 relation shape

**Step 4: Run tests to verify they pass**

Run: `pytest packages/modelado/tests -k "relation_queries or legacy" -v`
Expected: PASS with explicit architectural status.

**Step 5: Commit**

Run: `git add packages/modelado/src/modelado/fragment_relation_queries.py packages/modelado/tests && git commit -m "refactor: isolate legacy relation query path"`

### Task 6: Bring rendering and other runtime consumers back in sync with actual V3 shape

**Files:**
- Modify: `packages/modelado/src/modelado/ikam_rendering.py`
- Grep references: `packages/ikam/src/ikam/fragments.py`
- Test: rendering-related tests under `packages/modelado/tests/` or `packages/ikam/tests/`

**Step 1: Write the failing test**

Add/update tests that assert rendering code does not assume removed legacy fields such as:
- `artifact_id`
- `level`
- `content`

Instead, tests should verify behavior against the current minimal V3 domain fragment shape:
- `cas_id`
- `value`
- `mime_type`
- optional `fragment_id`

**Step 2: Run tests to verify they fail**

Run: `pytest packages/modelado/tests -k "render" -v`
Expected: FAIL if rendering still depends on stale fragment structure.

**Step 3: Write minimal implementation**

Planned code change:
- update `packages/modelado/src/modelado/ikam_rendering.py:91-159`
- remove stale doc/comments and code paths assuming rich legacy fragment fields
- use actual V3 fragment semantics or a separate projection if richer rendering metadata is required

**Step 4: Run tests to verify they pass**

Run: `pytest packages/modelado/tests -k "render" -v`
Expected: PASS for current V3 assumptions.

**Step 5: Commit**

Run: `git add packages/modelado/src/modelado/ikam_rendering.py packages/modelado/tests && git commit -m "refactor: align rendering with v3 fragment model"`

### Task 7: Align docs and compatibility tests with the final boundary

**Files:**
- Modify: `docs/ikam/IKAM_FRAGMENT_ALGEBRA_V3.md`
- Modify: `docs/scratch/ikam-graph-runtime-and-fragment-cleanup.md`
- Modify: `packages/ikam/tests/test_fragment_v3_minimal_schema.py`
- Modify: `packages/modelado/tests/test_legacy_adapter_guard.py`

**Step 1: Write the failing test/document checks**

Update test expectations to reflect final decisions:
- one primary public `Fragment`
- one storage-record type with explicit name
- fragment field set consistent with actual runtime model
- legacy adapter guard still enforces deletion of old adapter names

Concrete issue to resolve:
- `docs/ikam/IKAM_FRAGMENT_ALGEBRA_V3.md` still shows only `cas_id`, `value`, `mime_type`
- `packages/ikam/src/ikam/fragments.py` currently also includes `fragment_id`

**Step 2: Run tests to verify they fail**

Run: `pytest packages/ikam/tests/test_fragment_v3_minimal_schema.py packages/modelado/tests/test_legacy_adapter_guard.py -v`
Expected: FAIL if schema docs/tests remain out of sync with implementation.

**Step 3: Write minimal implementation**

Planned changes:
- update V3 doc to match actual model and public naming
- keep scratch note accurate or explicitly mark superseded sections
- update schema tests to assert the final intended model, not the stale interim one

**Step 4: Run tests to verify they pass**

Run: `pytest packages/ikam/tests/test_fragment_v3_minimal_schema.py packages/modelado/tests/test_legacy_adapter_guard.py -v`
Expected: PASS with docs/tests matching actual architecture.

**Step 5: Commit**

Run: `git add docs/ikam/IKAM_FRAGMENT_ALGEBRA_V3.md docs/scratch/ikam-graph-runtime-and-fragment-cleanup.md packages/ikam/tests/test_fragment_v3_minimal_schema.py packages/modelado/tests/test_legacy_adapter_guard.py && git commit -m "docs: align fragment boundary contracts"`

### Task 8: Run focused regression verification

**Files:**
- No code changes
- Test: cross-package targeted suite

**Step 1: Run focused package verification**

Run: `pytest packages/ikam/tests/test_v3_api_contract.py packages/ikam/tests/test_graph_model_cleanup.py packages/ikam/tests/test_graph_models.py packages/ikam/tests/test_cas_fragment.py packages/ikam/tests/test_fragment_v3_minimal_schema.py packages/modelado/tests/test_ikam_v3_adapter_roundtrip.py packages/modelado/tests/test_repository_integration.py packages/modelado/tests/test_ikam_write_scope_guards.py packages/modelado/tests/test_legacy_adapter_guard.py -v`
Expected: PASS for the renamed storage record, canonical adapter path, and cleaned-up boundary.

**Step 2: Run broader package verification if time allows**

Run: `pytest packages/ikam/tests packages/modelado/tests -v`
Expected: PASS, or a reduced list of unrelated failures to triage separately.

**Step 3: Commit final verification-only checkpoint if needed**

Run: `git status`
Expected: clean working tree if all planned commits were made.

### Execution Notes

- Execution style selected: `Subagent-Driven`
- Compatibility mode selected: `Clean break`
- In clean-break mode, remove `ikam.graph.Fragment` immediately instead of preserving an alias
- Do not begin implementation until explicitly instructed to execute this plan
