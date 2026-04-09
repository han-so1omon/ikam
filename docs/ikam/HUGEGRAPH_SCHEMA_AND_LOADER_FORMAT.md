# HugeGraph Projection Schema + Loader Export Format

This repo uses HugeGraph as a **projection / traversal index** for provenance/derivation graphs.

- **Source of truth:** Postgres append-only edge log (`graph_edge_events`).
- **Projection target:** HugeGraph graph (default name: `hugegraph`).

## Projection Schema (Derivations)

The schema is created (idempotently) by `ensure_ikam_projection_schema()`:

- Implementation: [packages/modelado/src/modelado/hugegraph_projection.py](../../packages/modelado/src/modelado/hugegraph_projection.py)

### Property keys

- `id` (TEXT)
- `project_id` (TEXT)
- `edge_key` (TEXT)
- `edge_label` (TEXT)
- `t` (LONG)
- `derivation_id` (TEXT)
- `derivation_type` (TEXT)

### VertexLabel: `Artifact`

- `id_strategy`: `PRIMARY_KEY`
- `primary_keys`: `["id"]`
- `properties`: `id`, `project_id`

The vertex `id` is the stable artifact id (i.e. `artifacts.id`).

### EdgeLabel: `Derivation`

- `source_label`: `Artifact`
- `target_label`: `Artifact`
- `frequency`: `MULTIPLE`
- `properties`: `edge_key`, `project_id`, `edge_label`, `t`, `derivation_id`, `derivation_type`
- `sort_keys`: `["edge_key"]`

`edge_key` is used for idempotent replay and is derived from the Postgres event `idempotency_key` when present.

### EdgeLabel: `value-at`

- source/target labels are created by the projector for graph-native addressed
  content
- this is the first-class HugeGraph projection label for `graph:value_at`
- it is not projected through `Derivation`

The `value-at` family preserves addressed graph slot metadata so HugeGraph can
query graph-native content without conflating it with derivation/provenance edges.

### Indexes

HugeGraph rejects property filters without an index, so the projection creates minimal secondary indexes:

- `derivation_by_edge_key` on `Derivation(edge_key)`
- `derivation_by_project_id` on `Derivation(project_id)`

#### Index strategy rationale

- **Vertex lookup keys:** `Artifact.id` is the vertex primary key (`PRIMARY_KEY` strategy). This keeps vertex reads deterministic and avoids requiring a secondary index for id-based lookup.
- **Projection isolation:** traversals are project-scoped via an edge `project_id` property; the `derivation_by_project_id` secondary index supports filtering edges by project in traversal queries.
- **Idempotent replay:** `edge_key` is used to ensure the projector can upsert/delete deterministically; the `derivation_by_edge_key` secondary index supports efficient lookup when de-duplicating or reconciling edges.

#### Query tuning notes

- Prefer **bounded traversals** with resource caps (`max_nodes`, `max_edges`) and optional depth caps, rather than hard-coded depth limits.
- Keep traversal predicates limited to indexed properties (`project_id`, `edge_key`) and label filters (`edge_label`) to avoid HugeGraph rejecting the query or falling back to expensive scans.

#### PD/Store compatibility

This schema + index strategy is intended to be topology-agnostic:

- Index labels are schema objects and apply equally in dev standalone HugeGraph and distributed HugeGraph-PD + HugeGraph-Store deployments.
- All client wiring should remain configuration-driven (base URL, graph name, graphspace, auth/timeouts) so tests and tooling can target either deployment.

#### Benchmarks / regression guard

For Week 2 traversal benchmarks used to validate performance and check for regressions, see:

- docs/benchmarks/hugegraph_vs_postgres_traversal.md

## Loader Export Format (CSV)

If you want to bulk-load a projection (instead of replaying `graph_edge_events`), a minimal export can be represented as two CSVs.

### Vertices: `artifacts.csv`

Required columns:

- `id` (TEXT) — artifact id
- `project_id` (TEXT)

### Edges: `derivations.csv`

Recommended columns:

- `edge_key` (TEXT)
- `project_id` (TEXT)
- `edge_label` (TEXT)
- `t` (LONG)
- `derivation_id` (TEXT)
- `derivation_type` (TEXT)
- `source_id` (TEXT) — maps to `Artifact.id`
- `target_id` (TEXT) — maps to `Artifact.id`

> Exact loader configuration varies by HugeGraph Loader version; treat this as the canonical **data shape** the projection expects.

## Deterministic Export From Postgres

When rebuilding from Postgres, the export must be deterministic for a given project.

Recommended procedure (single `project_id`):

1. **Extract ordered events** from `graph_edge_events` for the project in ascending `id` order.
2. **Fold** events into an effective edge set by applying `upsert`/`delete` in order per stable `edge_key`.
3. **Vertices CSV**: emit all distinct artifact ids referenced by the effective edges (`out_id` and `in_id`).
4. **Edges CSV**: emit only effective edges, keyed by `edge_key`, with the latest associated properties.

This is the same logic as the replay projector, expressed as a bulk-loadable snapshot.

## Recommended Path

- Local dev/backfills: replay via [scripts/hugegraph/replay_graph_edge_events.py](../../scripts/hugegraph/replay_graph_edge_events.py)
- Always-on dev projection: `hugegraph-projection-worker` in `docker-compose.yml`

## Read-Time Head Resolution

The HugeGraph projection remains rebuildable from the Postgres log, but read-time
callers may resolve locator-style references before they query the projected graph:

- `artifact_head_ref` is a locator, not a raw persisted head id
- shorthand locators resolve only against the current `EnvironmentScope.ref`
- `head_object_id` is the canonical resolved artifact head target
- artifact locators may lower to the selected head fragment before traversal or
  inspection logic runs

This keeps ref/head semantics in the runtime layer while leaving the projection
replay model append-only and deterministic.
