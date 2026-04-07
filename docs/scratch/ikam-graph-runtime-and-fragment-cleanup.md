# IKAM Graph Runtime And Fragment Cleanup Notes

Status: temporary scratch document

Purpose:
- capture the current architectural direction for IKAM graph runtime work
- record the open cleanup questions around `ikam.fragments` and `ikam.graph`
- provide a temporary reference before any formal RFC or implementation plan is written

## Current Direction

The working direction is to standardize around an `IKAMGraph` runtime abstraction inside the existing `packages/ikam` package rather than creating a parallel graph package.

Key points agreed so far:

- `IKAMGraph` should be the graph-facing runtime abstraction.
- `IKAMGraph` should also be the interface boundary to HugeGraph.
- The in-memory implementation may use `rustworkx` under the hood, but the public/runtime API should not expose raw `rustworkx` objects directly.
- There may be cases where the runtime operates directly against a HugeGraph-backed `IKAMGraph` implementation instead of a `rustworkx`-backed implementation, and the graph-facing API should remain the same in both cases.
- `interacciones` should act as the coupling layer between the IKAM graph runtime environment and the executor/operator environment.
- Operators should be allowed to consume and produce arbitrary local values.
- Operator input/output type specifications should be the authority for semantic graph-facing shapes such as `document_set`, `chunk_extraction_set`, and related aggregate structures.
- Those input/output type specifications should be grounded in fragment MIME typing and intermediate representations, not ad hoc payload keys.
- GraphSON is the leading candidate for the canonical serialized interchange/storage format.

## Runtime Shape Under Discussion

The intended architecture is:

- `IKAMGraph`
  - graph-facing runtime API
  - backend-neutral interface
  - capable of GraphSON import/export
  - capable of HugeGraph-backed and rustworkx-backed implementations

- `IKAMGraphSlice`
  - projected subgraph view used at executor/operator boundaries

- `interacciones` coupling layer
  - transforms an `IKAMGraph` slice into operator-local input values
  - transforms operator output values into a well-formed `IKAMGraph` slice

- operator input/output type specifications
  - declarative first
  - define graph-facing semantic shapes
  - use MIME + IR typing as the source of truth
  - optional custom hooks only as an escape hatch

## Fragment Position In The Graph Model

The current direction is that fragments should remain first-class graph vertices.

Important implications:

- atomic fragments are vertices
- aggregate semantic structures like `document_set`, `chunk_extraction_set`, `claim_set`, etc. should also be represented as first-class typed fragment vertices rather than ad hoc payload conventions
- edges express graph semantics like `contains`, `derives`, `references`, `anchors`, `emits`, and related relations

This keeps graph semantics in the graph instead of scattering them across bespoke operator payload shapes.

## Current Fragment Model Situation

The implemented boundary now has two fragment-layer types with distinct roles:

1. `ikam.graph.StoredFragment`
2. `ikam.fragments.Fragment`

Relevant code:

- `packages/ikam/src/ikam/graph.py`
  - `graph.StoredFragment` is explicitly documented as "STORAGE LAYER ONLY"
  - docs say it represents the minimal CAS storage layer

- `packages/ikam/src/ikam/fragments.py`
  - `fragments.Fragment` is described as the "single core type in V3"
  - docs describe it as the universal content container

This is now the implemented boundary: `ikam.fragments.Fragment` is the primary public/runtime type, and `ikam.graph.StoredFragment` is the explicit storage record.

## Cleanup Problem Statement

The fragment model split currently creates ambiguity about:

- which fragment type is the primary runtime/domain type
- which fragment type operators and graph logic should use
- whether the storage model should still appear as a top-level `Fragment`
- how adapters between storage and runtime are expected to work in practice

That naming ambiguity has been resolved for the fragment boundary, even though the broader `IKAMGraph` runtime work remains open.

## Boundary Decision

The fragment-boundary decision is now:

- evolve `packages/ikam` rather than creating a new graph package
- keep one primary fragment type for runtime/domain use
- demote or rename the storage-only fragment type so it is no longer a public peer to the runtime fragment type

The implemented direction is:

- `ikam.fragments.Fragment`
  - primary runtime/domain fragment type
  - used as the semantic payload type for graph vertices

- `ikam.graph`
  - home for `IKAMGraph`, graph slices, graph backends, graph codecs, and storage-oriented graph integration

- `ikam.graph.StoredFragment`
  - storage-layer CAS record only
  - explicit name to avoid competing with the runtime/domain `Fragment`

This is now the implemented boundary. Remaining graph-runtime questions should build on this naming rather than reopen it.

## Why Modify The Existing `ikam` Package

The current consensus is that the right move is to modify the existing `packages/ikam` package rather than create a separate package for the graph runtime.

Reasons:

- the package already contains the fragment and graph concepts
- the fragment cleanup and graph runtime work are tightly related
- creating a parallel package would likely make the architecture more confusing, not less
- the desired `IKAMGraph` abstraction is a natural extension of the existing `ikam` package responsibilities

## Relationship To HugeGraph And Rustworkx

The graph runtime should not assume one backend.

Planned model:

- `IKAMGraph` is the public graph abstraction
- `RustworkxIKAMGraphBackend` is one implementation
- `HugeGraphIKAMGraphBackend` is another implementation

The important design constraint is that graph-facing code should be able to work against either backend through the same API.

That means:

- backend-neutral graph operations
- backend-neutral slices
- backend-neutral GraphSON import/export
- no leakage of raw backend types into general runtime/operator logic

## Relationship To Operators And `interacciones`

The desired execution boundary is:

- core IKAM runtime owns `IKAMGraph`
- `interacciones` owns the transformation boundary between graph slices and operator-local values

This means:

- operators keep arbitrary local input/output shapes
- operator input/output specs declare expected semantic graph structure
- `interacciones` transforms:
  - `IKAMGraphSlice -> operator-local inputs`
  - `operator-local outputs -> IKAMGraphSlice`

This is explicitly a transformation boundary, not just naive serialization/deserialization.

## Open Questions

These questions remain open and should be resolved before implementation:

1. How should `IKAMGraph` relate to the established `ikam.fragments.Fragment` and `ikam.graph.StoredFragment` boundary?
2. Does `ikam.graph` need any additional storage abstractions beyond `StoredFragment`?
3. Should `InspectionSubgraph` become a view over `IKAMGraph`, or remain a parallel debug-only graph representation?
4. What is the exact declarative schema for operator input/output graph type specifications?
5. What should the first `IKAMGraph` backend-neutral API surface look like?
6. What subset of GraphSON should be treated as the canonical serialized form for IKAM graph slices?

## Immediate Next Discussion Topics

The next conversations should focus on:

1. determining how `ikam.graph` should be reshaped to house `IKAMGraph`
2. building graph runtime APIs around the established `StoredFragment` storage boundary
3. deciding whether `InspectionSubgraph` should become a view over `IKAMGraph`
4. defining the first backend-neutral GraphSON slice boundary

## Non-Goals For This Scratch Note

This note does not yet define:

- final GraphSON schema
- final `IKAMGraph` method signatures
- final operator input/output type spec schema
- final migration sequencing

Those should be handled in a formal design document or implementation plan that builds on the implemented fragment boundary.
