# ikam-compute

A graph computation engine on ikam (E430 to E433, opt-in, outside the
kernel): tensors as content, op graphs as ikam graph objects, reverse-mode
gradients and Hessian-vector products as graph transforms, pinned float
ops, stored evaluation in a database, Newton-CG with adjoint
sensitivities, planned programs for fixed-period loops, and runs committed
as streams. See `docs/plans/2026-11-26-math-engine.md`.

```
cargo test
cargo run --release --example engine_numbers
```
