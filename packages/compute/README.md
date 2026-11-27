# ikam-compute

A graph computation engine on ikam (E430 to E433, opt-in, outside the
kernel): tensors as content, op graphs as ikam graph objects, reverse-mode
gradients and Hessian-vector products as graph transforms, pinned float
ops, stored evaluation in a database, Newton-CG with adjoint
sensitivities, planned programs for fixed-period loops, and runs committed
as streams. See `docs/plans/2026-11-26-math-engine.md`.

Optional bounds on nondeterminism (E442): an `Op::Within` node in a graph
declares how far a value may move between runs with other float
arithmetic; `memo::check_bounded` reports each bound as same, within or
outside, and `bounds::predict` predicts bounds from rounding. A graph
without them is checked exactly, as before.

```
cargo test
cargo run --release --example engine_numbers
cargo run --release --example float_bounds
```
