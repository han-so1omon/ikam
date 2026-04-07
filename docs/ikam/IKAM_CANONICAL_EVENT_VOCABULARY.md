# IKAM Canonical Event Vocabulary

This document defines the **canonical event vocabulary** used to support IKAM interactivity (agents, plans, progress, graph mutations) across the stack.

## Quick checklist (PR-style)

Before changing event shapes or adding new lifecycle events:

- Read [Graph Vertex & Edge Definitions](GRAPH_VERTEX_EDGE_DEFINITIONS.md) (what “edge mutation” vs “knowledge link” means)
- Read [Relational Knowledge Fragments: Phase-Scoped Changes](RELATIONAL_KNOWLEDGE_FRAGMENTS_PHASE_MAP.md) (how relations/overlays/commits map to the active epics)

Core rule: **event semantics are defined here (and in the referenced schemas), not by Kafka topic topology.** Kafka topics and SSE endpoints are transport adapters that carry these semantics.

## Scope

- What events exist today (schemas + runtime behavior)
- What we standardize as “canonical” for the IKAM interactivity plan
- What is still **planned** (and how it should be expressed without inventing new top-level transports)

## Event families (canonical)

### 1) Interaction events (chat + agent envelopes)

**Canonical type:** `InteractionEvent`

- **Primary use:** chat history + agent request/response envelopes
- **Transport:** Kafka `interactions.user` and `interactions.agent` (**JSON-canonical**)
- **Schema (Avro mirror):** [schemas/interaction-event.avsc](../../schemas/interaction-event.avsc)
- **SSE surfaces:**
  - Project-scoped: `GET /api/session/projects/{project_id}/interactions/stream`
  - Global: `GET /api/interactions/stream`

**Canonical fields (semantic):**
- `interaction_id` (monotonic per-project in DB; becomes SSE `cursor`)
- `project_id`, `session_id`
- `scope`: `user | agent | system`
- `type`: `user_message | assistant_response | agent_request | agent_response | system_event`
- `content` (human text)
- `metadata` (JSON object)
- `parent_id` (threading/correlation)
- `created_at` (ms)

**Important note (canonical policy):**
- `interactions.*` topics are **JSON-canonical** to support nested `metadata` objects/lists.
- Non-JSON payloads on `interactions.*` are treated as a configuration error and dropped (no fallback decoding).

### 2) Job events (legacy/worker progress)

**Canonical type:** `JobEvent`

- **Primary use:** background job progress (`job_worker`, long-running tasks)
- **Transport:** Kafka `jobs.events` (Avro; camelCase fields)
- **Schema:** [schemas/jobs-event.avsc](../../schemas/jobs-event.avsc)
- **SSE surface:** `GET /api/model/stream` (mapped to `job_*` SSE event names)

**Normalization (EventStreamer):**
- `jobId` → `job_id`
- `t` → `timestamp`
- `payload` is parsed as JSON when possible

### 3) Model stream events (the UI’s unified “operation feed”)

This is the **canonical UI stream** for IKAM interactivity.

**SSE surface:** `GET /api/model/stream`

It carries multiple *families* of events, normalized by the base-api `EventStreamer`:

#### 3a) Progress events (typed operation progress)

**Canonical type:** `ProgressEvent`

- **Primary use:** structured operation progress for a single logical execution
- **Transport:** Kafka `model.events` (Avro), and also persisted/replayed from `model_events`
- **Schema:** [schemas/progress-event.avsc](../../schemas/progress-event.avsc)

**Canonical correlation:**
- `operationId` is the primary correlation id.
- Canonical policy for IKAM interactivity: **use `instruction_id` as the operation id** end-to-end.
  - See [docs/architecture/EVENT_ID_STRATEGY.md](../architecture/EVENT_ID_STRATEGY.md).

#### 3b) Plan events (leases/claims/amendments)

**Canonical type:** `PlanEvent`

- **Primary use:** multi-agent coordination around a plan artifact, section leases/claims, amendments
- **Transport:** Kafka `plans.events` (Avro)
- **Schema:** [schemas/plan-event.avsc](../../schemas/plan-event.avsc)

**Canonical fields:**
- `planArtifactId`: the durable IKAM artifact id representing the plan
- `scopeId`: scope/thread/workspace boundary for the plan
- `fragmentId`: optional; identifies the plan section fragment being leased/updated
- `leaseOwner`, `leaseToken`, `leaseExpiresAt`: coordination handles

**Canonical kinds (today):**
- `plan_created`
- `plan_section_claimed` / `plan_section_progress` / `plan_section_blocked` / `plan_section_completed` / `plan_section_released`
- `plan_amendment_proposed` / `plan_amendment_accepted` / `plan_amendment_rejected`

#### 3c) Model lifecycle events (broad typed lifecycle)

**Canonical type:** `ModelLifecycleEvent` (emitted on Kafka topic `model.lifecycle`, normalized to `type: model_lifecycle`)

- **Primary use:** CRUD/lifecycle notifications for model artifacts (models, revisions, branches, subscriptions, proposals)
- **Transport:** Kafka `model.lifecycle` (Avro by default; JSON allowed)
- **Schema:** [schemas/model-event.avsc](../../schemas/model-event.avsc)

This record is intentionally generic (`eventType`, `entityType`, `payload`) and can be safely extended by introducing **new `eventType` values** without schema changes.

### 4) Graph edge mutation log (authoritative provenance/derivation edges)

**Canonical type:** `graph_edge_events` (Postgres append-only log)

- **Primary use:** provenance-safe graph mutations and deterministic projection rebuilds
- **Transport:** Postgres table (not Kafka)
- **Spec:** [docs/ikam/GRAPH_EDGE_EVENT_LOG.md](GRAPH_EDGE_EVENT_LOG.md)

For IKAM interactivity, this remains the **source of truth**. If/when we need real-time UI updates for edge mutations, we should **mirror** edge-event appends into the canonical model stream (see “Planned extensions”).

## Canonical naming and correlation

### Names

- Use `snake_case` in JSON event payloads consumed by the UI.
- SSE `event:` names are a transport concern; the payload’s `type` is the canonical discriminator.
- **Canonical lifecycle SSE event name:** `model_lifecycle` (underscore). Server emits this consistently.

### Correlation keys (canonical)

This section is the **single source of truth** for which correlation fields are required.

**InteractionEvent** (`interactions.*`, JSON-canonical)
- Required: `interaction_id`, `project_id`, `scope`, `type`, and one of `created_at` / `timestamp`
- Required for correlated (non-root) events: `parent_id` (the canonical `instruction_id` for the operation)
- Optional: `thread_id` (attached from metadata when available)

**ProgressEvent** (`type: progress` in the model stream)
- Required: `timestamp`
- Required: one of `instruction_id` / `operationId`
- Rule: if both are present, they must match (canonical policy: use `instruction_id` as the operation id)

**JobEvent** (legacy `jobs.events` normalization)
- Required: `job_id`, `timestamp`

**PlanEvent** (`type: plan_event`)
- Required: `project_id`, `plan_artifact_id`, `kind`
- Required for section-level kinds (`kind` starts with `plan_section_`): `fragment_id`, `lease_token`

**ModelLifecycleEvent** (`type: model_lifecycle`)
- Required: `project_id`, `eventType`, `entityType`, `entityId`, `correlationId`, `t`

**Graph edge events** (Postgres append-only log)
- Required: `project_id`, `idempotency_key`

### Runtime concepts → correlation fields

This mapping keeps correlation semantics consistent across transports (Kafka, SSE, Postgres).

- **Operation / run id** → `instruction_id` (preferred) / `operationId` (ProgressEvent), and `parent_id` (InteractionEvent threading)
- **Thread id** → `thread_id` (optional; extracted from interaction metadata when available)
- **Plan artifact** → `plan_artifact_id` (PlanEvent)
- **Plan section / step** → `fragment_id` (PlanEvent section kinds)
- **Lease / claim token** → `lease_token` (PlanEvent section kinds)
- **Lifecycle entity** → `entityType` + `entityId` (ModelLifecycleEvent)
- **Lifecycle correlation** → `correlationId` (ModelLifecycleEvent)
- **Edge mutation idempotency** → `idempotency_key` (Graph edge events)

### Idempotency

- Plan events: dedupe by `(kind, event_id)` (enforced by `model_events` unique index when event_id present)
- Graph edge events: dedupe by `(project_id, idempotency_key)`

## Planned extensions for IKAM interactivity (no new top-level transports)

These are needed for the IKAM interactivity plan but should be expressed using **existing event families**.

### A) Draft overlays + commits

Represent as `model_lifecycle` with:
- `entityType: "overlay"`
- `eventType` values:
  - `overlay.created`
  - `overlay.updated`
  - `overlay.committed`
  - `overlay.discarded`
- `payload` contains `overlay_id`, base artifact ids, and commit ids.

Rationale: avoids schema churn and keeps overlays visible to `/api/model/stream` subscribers.

### B) Merge/conflict resolution artifacts

Represent as `model_lifecycle` with:
- `entityType: "merge"` (or `"conflict"`)
- `eventType` values:
  - `merge.required`
  - `merge.resolved`
  - `merge.resolution_artifact_created`

Rationale: merge resolution is a lifecycle state transition; it doesn’t require new transport.

### C) Graph edge mutation mirroring (optional UI feature)

When an edge-event is appended to Postgres, mirror a lightweight notification into the model stream:

- `type: model_lifecycle`
- `entityType: "graph"`
- `eventType: "graph.edge_event_appended"`
- `payload`: `{ "graph_edge_event_id": <bigint>, "edge_label": "...", "out_id": "...", "in_id": "..." }`

This keeps Postgres as the source of truth while enabling UI/agent observers.

## References (implementation)

- Event normalization + durable replay: [services/base-api/src/narraciones_base_api/app/services/event_streamer.py](../../services/base-api/src/narraciones_base_api/app/services/event_streamer.py)
- Model SSE endpoint: [services/base-api/src/narraciones_base_api/app/api/model.py](../../services/base-api/src/narraciones_base_api/app/api/model.py)
- Interactions SSE endpoint (project-scoped): [services/base-api/src/narraciones_base_api/app/api/projects.py](../../services/base-api/src/narraciones_base_api/app/api/projects.py)
- Kafka-backed streaming overview: [docs/architecture/kafka-sse-streaming.md](../architecture/kafka-sse-streaming.md)
