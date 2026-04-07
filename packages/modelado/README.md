# Modelado [![Docs](https://img.shields.io/badge/docs-Read%20the%20Docs-blue)](https://narraciones.readthedocs.io/en/latest/modelado/)

**General-purpose modeling framework** providing reusable components for mathematics, documents, temporal analysis, and entity management.

## Core Principles

**⚠️ Semantic Evaluation is Always Available:**
When using persona/artifact evaluation features (test sequencing, evaluation hooks), semantic interpretation via embeddings, LLM classification, and content inspection is mandatory. There are no fallbacks to hardcoded persona types, artifact kind switches, or "generic" evaluators. Missing semantic infrastructure is a fatal error. See root `AGENTS.md` and `docs/testing/SEQUENCING_FRAMEWORK_PERSONAS_SPEC.md` for architectural requirements.

## Purpose

`modelado` is a **framework package** (not Narraciones-specific) that can be used in any project requiring:
- **Mathematics:** Natural language ↔ symbolic formula translation (SymPy integration)
- **Documents:** Chunking, embedding, retrieval (RAG pipelines)
- **Temporal:** Forecasting, aggregation, trend detection
- **Entities:** NER, graph operations, entity types

## Architecture

Part of the three-layer architecture:
- **Layer 0 (Standalone):** `ikam` — Domain models with zero dependencies
- **Layer 1 (Frameworks):** `modelado` + `interacciones` — Reusable orchestration (depend only on ikam)
- **Layer 2 (Application):** `narraciones` — Business-specific implementations

Infrastructure utilities (DB, Kafka, config) are embedded in `modelado` for framework-level operations.

## Installation

```bash
# Base package
pip install -e packages/modelado

# With optional dependencies
pip install -e "packages/modelado[mathematics]"
pip install -e "packages/modelado[documents]"
pip install -e "packages/modelado[temporal]"
pip install -e "packages/modelado[entities]"
pip install -e "packages/modelado[all]"
```

### Consuming Project Options

- Editable install (dev): `pip install -e /path/to/narraciones/packages/modelado`
- Wheels (release): `pip install /path/to/narraciones/packages/modelado/dist/*.whl`
- Poetry path deps:
	```toml
	[tool.poetry.dependencies]
	modelado = { path = "../narraciones/packages/modelado", develop = true }
	```

## Modules

- `modelado.mathematics` — NL ↔ formula translation, SymPy integration
- `modelado.temporal` — Forecasting, aggregation, trend detection
- `modelado.entities` — NER, graph operations, entity types
- `modelado.adapters` — IKAM storage ↔ domain model transformations
- `modelado.core` — Base models, pipelines, MCP commands, validation

**Note:** Document chunking functionality has been consolidated into `ikam.almacen.chunking` as part of IKAM v2 architecture.

## Usage Example

```python
from modelado.mathematics import MathTranslator

translator = MathTranslator()

## Documentation

- API and guides: https://narraciones.readthedocs.io/en/latest/modelado/
result = translator.translate("revenue minus costs")
# result.sympy_expr: Revenue - Costs
# result.latex: "\\text{Revenue} - \\text{Costs}"
```

## Dependencies

- **Core:** `ikam` (Layer 0 domain models)
- **Infrastructure:** PostgreSQL, Kafka (embedded adapters)
- **Optional:** SymPy (mathematics), sentence-transformers (documents)
- Zero business-logic dependencies — can be used in any project
- Integrates with `interacciones` for orchestration

## Releasing

Modelado publishes independently via tag-driven CI.

1. Bump `version` in `packages/modelado/pyproject.toml`.
2. Commit and push.
3. Tag and push:
	```bash
	git tag -a modelado-v0.1.1 -m "modelado v0.1.1"
	git push origin modelado-v0.1.1
	```
4. CI builds and publishes to GitHub Packages.
5. Consumers install via:
	```bash
	pip install narraciones-modelado --extra-index-url https://pypi.pkg.github.com/<owner>/
	```

### Release Checklist

- Update `packages/modelado/pyproject.toml` `version`
- Add entry to `packages/modelado/CHANGELOG.md`
- Commit changes
- Create and push tag `modelado-vX.Y.Z`
