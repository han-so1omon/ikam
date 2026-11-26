//! Graph computation on the ikam kernel (E430).
//!
//! - [`Tensor`]s are content: f64 values in a fixed byte order.
//! - [`ir::Graph`]s are op graphs stored as ikam graph objects.
//! - [`eval`] evaluates in memory with pinned float arithmetic
//!   ([`pinned`]), so results have the same bits on every platform.
//! - [`memo`] evaluates on the store: each value is content and each
//!   computation is logged under its key, so reruns reuse it and a rerun
//!   that computes other bits is refused.
//! - [`grad`] differentiates a graph into another graph (reverse mode),
//!   and again for Hessian-vector products.
//! - [`solve`] minimizes with Newton-CG over those graphs, for nonlinear
//!   static problems; [`run`] records a sequence of states (load steps or
//!   time steps) as a stream; [`plan`] lowers a graph to a fixed program
//!   for loops that must keep a period.
//!
//! See docs/plans/2026-11-26-math-engine.md.

pub mod eval;
pub mod grad;
pub mod ir;
pub mod memo;
pub mod pinned;
pub mod plan;
pub mod run;
pub mod solve;
mod tensor;

pub use tensor::Tensor;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("shape: {0}")]
    Shape(String),
    #[error("missing input {0}")]
    MissingInput(String),
    #[error("decode: {0}")]
    Decode(&'static str),
    #[error("{key} was recorded as {recorded} and now computes {now}")]
    Nondeterministic { key: String, recorded: ikam_kernel::Id, now: ikam_kernel::Id },
    #[error("did not converge: {0}")]
    NoConvergence(String),
    #[error(transparent)]
    Kernel(#[from] ikam_kernel::Error),
}
