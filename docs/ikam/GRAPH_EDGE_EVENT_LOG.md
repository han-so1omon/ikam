# Graph Edge Event Log (Postgres)

The `graph_edge_events` table is the **authoritative** record of provenance/derivation edge mutations.

HugeGraph is a **projection / traversal index** built by replaying this log in deterministic order. Postgres remains the source of truth.

## Contract

- **Append-only:** events are inserted; never updated in-place.
- **Deterministic ordering:** consumers must replay in ascending `id` order.
- **Project-scoped:** every row is scoped by `project_id`; consumers must *always* filter by `project_id` to avoid cross-project leakage.

### Org scoping

The event log is **physically** scoped by `project_id` (foreign key to `projects(id)`), and org ownership is derived by joining through the project record.

If we later need a denormalized `owner_org_id` column for operational convenience, it can be added without changing the append-only semantics.
- **Idempotent appends:** producers should supply an `idempotency_key` (or let the writer compute one) so retries do not create duplicate events.

## Operations

`op` is constrained by the database to:

- `upsert` — assert/update that an edge exists with the given properties.
- `delete` — retract an edge.

> Note: even though `upsert` can conceptually “update”, the log itself remains append-only. Projections apply events sequentially.

## Identifiers

- `out_id` and `in_id` are the stable graph vertex ids for the source and target vertices.
- Depending on the producer and edge family, these ids may refer to artifact vertices or fragment vertices.
- Consumers must interpret endpoint ids in the context of the edge family they are traversing.
- `edge_label` is the semantic edge label. For derivations we use the `derivation:` prefix (for example: `derivation:derived_from`).

## Read-Time Locator Resolution

The append-only event log remains authoritative, but some read-time consumers now
resolve locator-style references before traversing the log:

- artifact locators such as `artifact://<semantic_artifact_id>` or `ref://refs/heads/main/artifact/<semantic_artifact_id>`
  resolve through the selected ref head
- `head_object_id` is the canonical resolved artifact head target
- artifact-head resolution then lowers to `ikam_fragment_objects.root_fragment_id`
  when a consumer needs a concrete graph target
- fragment locators such as `fragment://<fragment_id>` or `ref://refs/heads/main/fragment/<fragment_id>`
  target that fragment directly

This read-time resolution model does not change the append-only contract of
`graph_edge_events`; it defines how callers select the effective graph element
they want to inspect.

The HugeGraph projection computes a stable `edge_key` from `idempotency_key` when present.

### Projection-specific properties

For derivation edges, the HugeGraph projection currently looks for these optional keys in `properties`:

- `derivationId`
- `derivationType`

## Schema

Defined in [scripts/database/migrations/029_create_graph_edge_event_log.sql](../../scripts/database/migrations/029_create_graph_edge_event_log.sql).

Columns:

- `id BIGSERIAL` — monotonically increasing event id (replay order).
- `project_id TEXT` — owning project id.
- `op TEXT` — `upsert` or `delete`.
- `edge_label TEXT` — semantic edge label.
- `out_id TEXT` — source vertex id.
- `in_id TEXT` — target vertex id.
- `properties JSONB` — edge metadata (projection-specific keys allowed).
- `t BIGINT` — event timestamp (milliseconds since epoch).
- `idempotency_key TEXT` — deterministic key for retries.

Indexes:

- `(project_id, id)` — deterministic ordered replay per project.
- `(project_id, edge_label, out_id, in_id)` — inspection/debug queries.
- Unique `(project_id, idempotency_key)` when `idempotency_key` is not null.

## Reference Implementation

- Base API derivation emitter: [services/base-api/src/narraciones_base_api/app/services/derivations.py](../../services/base-api/src/narraciones_base_api/app/services/derivations.py)
- Postgres contract helpers: [packages/modelado/src/modelado/graph_edge_event_log.py](../../packages/modelado/src/modelado/graph_edge_event_log.py)
- HugeGraph projection schema + replay: [packages/modelado/src/modelado/hugegraph_projection.py](../../packages/modelado/src/modelado/hugegraph_projection.py)
- Local replay script: [scripts/hugegraph/replay_graph_edge_events.py](../../scripts/hugegraph/replay_graph_edge_events.py)
