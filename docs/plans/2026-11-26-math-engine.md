# ikam as a mathematical engine

The owner, Nov 26: a graph computation toolset in the spirit of JAX,
TensorFlow, torch or formualizer; simulation suited to nonlinearities;
real-time control; all built on ikam's foundations (artifacts, fragments,
render chains, applied functions, Petri nets, streams, views, permissions,
versions, immutability, dedup), changing ikam where needed rather than
adding new foundations, and also to improve ikam.

This note records what E430 to E433 built and measured in
`packages/compute` (the `ikam-compute` crate, opt-in, outside the kernel)
and what the kernel needs next. Downstream projects that use it (a
robotics project and a materials project) live in their own repositories;
nothing of theirs is in this one.

## How the pieces map onto ikam

| Engine piece | ikam foundation |
|---|---|
| Tensor values | content (`ikT1` encoding: shape, then little-endian f64 with one NaN), deduplicated like any bytes |
| Op graph | a graph object: one node per op, its value a content target, edges to its arguments; committed at a path in a tree (`Graph::commit_at`, `load_at`) |
| Gradient, Hessian-vector product | graph transforms (reverse mode, and reverse over reverse): a new graph, stored and shared like the first |
| Evaluation | an applied function over pinned float ops (libm, no fused multiply-add), so a value is a function of its inputs' bytes |
| Stored evaluation | records in a database keyed by the graph and input ids, with the event log's idempotency keys catching a different answer for the same key |
| A simulation run | a stream: one record per step in a database, committed on `runs/<name>` |
| A process | a Petri net over the steps (place = state of a piece, transition = a step or a person's approval) |
| Sharing a model | a graph share (the graph) and a records share (its tables), by scope, between separate projects |

## What works (numbers from `examples/engine_numbers.rs` and the downstream checks)

- **Gradients** match central differences on a graph mixing matmul,
  transpose, reshape, broadcast, max, cumsum, slice, concat, pad and the
  unary ops; Hessian-vector products match differences of the gradient.
- **Newton-CG** on an energy graph: a planar elastica converges at second
  order (tip error 2.4e-4 L at 25 segments to 9.4e-7 L at 400, one Newton
  step each for the end-moment bend); the classic end-load case
  (PL^2/EI = 1) gives tip deflection 0.30172 L and shortening 0.05643 L,
  the published values. A gravity and tip-load cantilever matches an
  independent shooting solve within 2e-5 of the length.
- **Adjoint sensitivities** at an equilibrium (one extra linear solve per
  objective) match re-solving with perturbed parameters.
- **Planned programs** (`Plan`, `PlannedMinimizer`): fixed buffers, no
  allocation per call, bit-identical to the interpreter. A 44-node gradient
  runs in 0.64 us (p50) against 3.1 us interpreted.
- **A 1 kHz loop** (PI with feedforward, two warm-started Newton steps per
  tick on a 24-segment rod): planned p50 107 us, p99 198 us, no missed
  deadlines in 3,000 ticks (7 in 10,000 on a shared 4-CPU container);
  interpreted misses 1,152 of 3,000. Telemetry over a channel to a writer
  thread: every step recorded, none dropped, and a second run records the
  same commit.
- **A section model shared across projects**: one project publishes a
  yielding-section energy graph and its moment-curvature tables; the other
  imports both by scoped shares (in-process with `import_share`, and
  through one `ikam host` with `/shares/issue`, `/from/.../records`,
  `/from/.../merge`), inlines the graph into its own rod energy
  (`Graph::inline`), and its rod reproduces the imported tables (210
  solves, worst curvature error 7.7e-5). Records outside the shared range
  stay behind. Merging the same shares again gives the same commits.

## What we learned about ikam

1. **The ledger can't hold float derivations.** WASM floats are off for
   determinism, so stored evaluation lives in a database, not in the
   derivation ledger. Pinned float ops in Rust make values reproducible on
   one build; the ledger needs a function kind for them (E435).
2. **Granularity.** One record per op node is far too slow for fine graphs
   (a 44-node gradient: 2.3 ms stored against 9 us plain). One record per
   whole-graph evaluation (a recorded step) costs 82 us the first time and
   4 us to reuse. Record steps, not nodes.
3. **A records share carries record bytes, not what they point to.** A
   record holding a content id arrives dangling. Shared tables keep values
   inline.
4. **Binary record values do not survive the host's JSON.** Reading a
   records share over HTTP returns non-UTF-8 values as replacement
   characters; merging (a store-to-store copy) is exact (E434).
5. **A graph share carries compute graphs as they are**, and a graph that
   fixes no sizes (inputs shaped `[n, 1]`, outer products by matmul with
   constants) can be inlined into a model of any size by the receiver.
   Elementwise broadcasting beyond scalars would remove the matmul trick.
6. **No public edit puts a graph at a path.** `ikam-compute` carries its own
   `put_entry`; the kernel's `Edit` should have one (E438).
7. **Moment loading past a limit point stalls Newton.** A yielding section's
   moment flattens, and a rod loaded by a moment there has almost no
   stiffness; real processes prescribe the shape (a form). The solver needs
   constraints and continuation (E439).

## Planned experiments (E434 to E439, opt-in, each its own PR)

- E434 records over HTTP: answer non-UTF-8 record values as base64 with a
  marker in `/from/.../records` and `db serve`, readable back by the same
  clients.
- E435 pinned-float applied functions: a function kind whose ops are the
  pinned set, so a recorded step is a derivation in the ledger, checked by
  re-running. E409 (measured on 11 builds, open as its own PR) gives the
  rules: native code in a fixed order matched everywhere but NaN bits,
  which `ikT1` already makes canonical; no fused multiply-add and no
  lane-split sums; a shipped `exp` (the pinned set uses `libm`), not the
  host's.
- E436 recorded steps under Petri nets: a transition whose firing is a
  recorded compute step, its output in the firing record, replayable.
- E437 records shares with content: an option to carry the content a
  shared record names, within the share's scope.
- E438 `Edit::PutGraph { path, id }` in the kernel's tree edits.
- E439 solver: equality constraints (prescribed displacements) and
  arc-length continuation; then explicit time stepping for dynamics, and
  a stencil op for cellular automata and lattice flow solvers.
