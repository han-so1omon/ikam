//! IKAM kernel: a content-addressed object store (L0) and codecs whose
//! decompositions are guaranteed to reconstruct their input (L1).
//! See docs/plans/2026-10-02-rust-kernel.md for the layer laws.

pub mod codec;
mod id;
mod object;
mod store;

pub use codec::{Codec, Ingested, Shape, ingest, render};
pub use id::Id;
pub use object::{Entry, Kind, Object};
pub use store::{FsStore, MemStore, Store};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("object {0} not found")]
    NotFound(Id),
    #[error("object {0} is corrupt or has the wrong kind")]
    Corrupt(Id),
    #[error("decode error: {0}")]
    Decode(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
