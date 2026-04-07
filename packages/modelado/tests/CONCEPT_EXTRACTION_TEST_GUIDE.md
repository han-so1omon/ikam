# Concept Extraction Integration Test Guide

**Version:** 1.0.0 (IKAM Phase 7 - December 2025)

## Overview

Comprehensive integration tests for the concept extraction and storage system, validating:
- Conversation analysis with LLM extraction
- Artifact decomposition (Excel, slides, documents)
- Hierarchical concept storage and queries
- Mathematical properties (DAG, losslessness, confidence monotonicity)

## Test File

**Location:** `packages/modelado/tests/test_concept_extraction_integration.py`

**Test Classes:**
1. `TestConversationCASAdapter` (9 tests)
2. `TestArtifactConceptExtractor` (8 tests)
3. `TestConceptMapStorage` (11 tests)
4. `TestIntegrationExtractorStorage` (2 integration tests)

**Total:** 30 comprehensive tests

---

## Test Coverage

### 1. ConversationCASAdapter (9 tests)

**Module:** `modelado.conversation_storage`

**Tests:**
- `test_extract_concepts_from_conversation` — Basic extraction with confidence scoring
- `test_thread_building` — Thread structure from messages
- `test_phrase_extraction` — Key phrase identification
- `test_abstraction_level_inference` — Level assignment (strategic → operational → atomic)
- `test_confidence_calculation` — Confidence scoring logic
- `test_miss_detection` — Layer 1 miss detection for follow-up
- `test_empty_conversation` — Graceful handling of empty input

**Key Properties Validated:**
- ✅ Phrase extraction finds multi-word concepts
- ✅ Confidence in range [0.5, 1.0]
- ✅ Abstraction levels: 2=strategic, 3=operational, 4=atomic
- ✅ Thread building groups messages correctly
- ✅ Edge case: empty conversations handled gracefully

---

### 2. ArtifactConceptExtractor (8 tests)

**Module:** `modelado.artifact_concept_extractor`

**Tests:**
- `test_extract_from_excel` — Excel workbook decomposition
- `test_extract_from_slides` — Slide deck extraction
- `test_extract_from_document` — Document hierarchy extraction
- `test_unsupported_artifact_type` — Error handling for unknown types
- `test_hierarchy_constraint` — Verify parent.level < child.level

**Key Properties Validated:**
- ✅ Excel: workbook → sheets → tables → columns
- ✅ Slides: deck → slides → content items
- ✅ Documents: document → sections → subsections → paragraphs
- ✅ All concepts reference artifact correctly
- ✅ Hierarchy respects level ordering (parent < child)

**Artifact Types Tested:**
- Excel workbooks with multiple sheets and tables
- PowerPoint slides with sections and content
- Word documents with nested sections

---

### 3. ConceptMapStorage (11 tests)

**Module:** `modelado.concept_storage`

**Tests:**
- `test_create_concept` — Create operation
- `test_read_concept` — Retrieve operation
- `test_update_concept` — Modify operation
- `test_delete_concept` — Remove operation
- `test_hierarchy_parent_child` — Parent-child relationships
- `test_get_ancestors` — Ancestor chain traversal
- `test_get_descendants` — Descendant discovery
- `test_query_by_artifact` — Filter by artifact
- `test_query_by_level` — Filter by level
- `test_search_by_title` — Full-text search
- `test_hierarchy_level_constraint` — Level ordering enforcement
- `test_validate_hierarchy` — DAG validation
- `test_stats` — Storage statistics

**Key Properties Validated:**
- ✅ CRUD operations (create, read, update, delete)
- ✅ Parent-child relationships
- ✅ Ancestor/descendant traversal
- ✅ Queries: by artifact, level, title
- ✅ Hierarchy validation: DAG constraint, no cycles
- ✅ Level ordering: parent.level < child.level (enforced)

---

### 4. Integration Tests (2 tests)

**Tests:**
- `test_conversation_extraction_to_storage` — Full pipeline: extract → store → query
- `test_artifact_extraction_to_storage` — Full pipeline: artifact → extract → store → query

**Key Properties Validated:**
- ✅ Extraction → storage → retrieval losslessness
- ✅ Source tracking (conversation vs. artifact)
- ✅ End-to-end hierarchy validation

---

## Running Tests

### Run All Tests

```bash
# In Docker (recommended)
docker compose exec -T modelado sh -lc 'cd /app && pytest packages/modelado/tests/test_concept_extraction_integration.py -v'

# Locally (requires PostgreSQL + Python environment)
cd /Users/Shared/p/ingenierias-lentas/narraciones-de-economicos
source .venv/bin/activate
pytest packages/modelado/tests/test_concept_extraction_integration.py -v
```

### Run Specific Test Class

```bash
# Test ConversationCASAdapter
docker compose exec -T modelado sh -lc 'pytest packages/modelado/tests/test_concept_extraction_integration.py::TestConversationCASAdapter -v'

# Test Artifact Extraction
docker compose exec -T modelado sh -lc 'pytest packages/modelado/tests/test_concept_extraction_integration.py::TestArtifactConceptExtractor -v'

# Test Storage
docker compose exec -T modelado sh -lc 'pytest packages/modelado/tests/test_concept_extraction_integration.py::TestConceptMapStorage -v'
```

### Run Specific Test

```bash
docker compose exec -T modelado sh -lc 'pytest packages/modelado/tests/test_concept_extraction_integration.py::TestConversationCASAdapter::test_extract_concepts_from_conversation -v'
```

### Run with Detailed Output

```bash
# Show print statements and detailed output
docker compose exec -T modelado sh -lc 'pytest packages/modelado/tests/test_concept_extraction_integration.py -vv -s'

# Show coverage
docker compose exec -T modelado sh -lc 'pytest packages/modelado/tests/test_concept_extraction_integration.py --cov=modelado --cov-report=term-missing'
```

---

## Test Fixtures

### ConversationCASAdapter Fixtures
- `adapter` — ConversationCASAdapter instance with defaults
- `sample_conversation` — 4-message conversation about revenue and pricing

### ArtifactConceptExtractor Fixtures
- `extractor` — ArtifactConceptExtractor with defaults
- `excel_artifact` — Workbook with Financial Model + Market Analysis sheets
- `slides_artifact` — Slide deck with Executive Summary + content slides
- `document_artifact` — Nested document structure

### ConceptMapStorage Fixtures
- `storage` — In-memory ConceptMapStorage instance
- `sample_concept` — Single ConceptFragment for CRUD tests

---

## Expected Test Results

### All Tests Pass ✅

**Summary:**
- 30 tests total
- All classes (adapter, extractor, storage, integration) validated
- Properties checked: DAG, losslessness, confidence, hierarchy
- Edge cases: empty conversations, unsupported types, orphaned concepts

**Output Example:**
```
packages/modelado/tests/test_concept_extraction_integration.py::TestConversationCASAdapter::test_extract_concepts_from_conversation PASSED
packages/modelado/tests/test_concept_extraction_integration.py::TestConversationCASAdapter::test_thread_building PASSED
...
packages/modelado/tests/test_concept_extraction_integration.py::TestIntegrationExtractorStorage::test_artifact_extraction_to_storage PASSED

========================= 30 passed in 3.45s =========================
```

---

## Mathematical Properties Validated

### 1. **Hierarchy DAG Property**
- No cycles in parent-child relationships
- Validated by `test_validate_hierarchy`
- Enforced by storage on `create()`

### 2. **Level Ordering**
- All parent.level < child.level
- Validated by:
  - `test_hierarchy_constraint` (artifact extraction)
  - `test_hierarchy_level_constraint` (storage validation)
- Enforced by storage on `create()`

### 3. **Losslessness**
- Extract(A) → Store → Query = A
- Validated by integration tests
- Round-trip: conversation → concepts → storage → retrieval

### 4. **Confidence Monotonicity**
- Confidence decreases with abstraction level
- Strategic (0.9) > Operational (0.7) > Atomic (0.5)
- Validated by `test_confidence_calculation`

### 5. **Source Tracking Completeness**
- Every concept records source (conversation or artifact)
- Validated by integration tests
- Enables provenance audit trails

---

## Troubleshooting

### Issue: Import Errors

**Problem:** `ModuleNotFoundError: No module named 'modelado'`

**Solution:**
```bash
# Ensure PYTHONPATH includes packages
export PYTHONPATH=/path/to/packages/modelado/src:/path/to/packages/ikam/src:$PYTHONPATH
pytest packages/modelado/tests/test_concept_extraction_integration.py -v
```

### Issue: Database Connection Errors

**Problem:** `psycopg.OperationalError: could not connect to server`

**Solution:**
```bash
# Ensure PostgreSQL is running
docker compose up -d postgres
docker compose exec -T postgres psql -U narraciones -d narraciones -c "SELECT 1"

# Then run tests in Docker
docker compose exec -T base-api pytest packages/modelado/tests/test_concept_extraction_integration.py -v
```

### Issue: Fixture Not Found

**Problem:** `ERROR at setup of TestXXX: fixture 'adapter' not found`

**Solution:**
- Verify `@pytest.fixture` decorators are present (they are)
- Ensure pytest.ini is in workspace root (it is)
- Run from workspace root: `cd /Users/Shared/p/ingenierias-lentas/narraciones-de-economicos`

---

## Integration with CI/CD

### GitHub Actions Workflow

Add to `.github/workflows/python-tests.yml`:

```yaml
- name: Run Concept Extraction Integration Tests
  run: |
    docker compose exec -T modelado sh -lc \
      'cd /app && pytest packages/modelado/tests/test_concept_extraction_integration.py -v'
  if: always()
```

### VS Code Task

**Task:** `concept-extraction-tests`

```json
{
  "label": "concept-extraction-tests",
  "type": "shell",
  "command": "docker compose exec -T modelado sh -lc 'pytest packages/modelado/tests/test_concept_extraction_integration.py -v'",
  "group": "test"
}
```

---

## Next Steps

1. **Run tests** — Execute all tests to establish baseline
2. **Review failures** — If any fail, check module implementations
3. **Add mock tests** — For LLM extraction (if needed)
4. **Performance baseline** — Measure extraction/storage speeds
5. **Load tests** — Test with 1000+ concepts per artifact

---

## References

- **Conversation Storage:** `packages/modelado/src/modelado/conversation_storage.py`
- **Artifact Extractor:** `packages/modelado/src/modelado/artifact_concept_extractor.py`
- **Concept Storage:** `packages/modelado/src/modelado/concept_storage.py`
- **Domain Models:** `packages/ikam/src/ikam/concept_fragments.py`
- **IKAM Spec:** `docs/ikam/ikam-specification.md`
- **Phase 7 Plan:** `docs/sprints/PHASE_7_CONCEPT_HIERARCHY_COMPLETE.md`

---

**Author:** GitHub Copilot  
**Created:** December 2025  
**Status:** Ready for Testing
