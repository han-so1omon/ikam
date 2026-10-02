//! IKAM kernel: a content-addressed store whose files are guaranteed to
//! reconstruct byte-for-byte, with data-driven dedup and git-like versioning.
//! See docs/plans/2026-10-02-rust-kernel.md for the layer laws.

mod id;
pub mod matcher;
mod object;
mod repo;
pub mod snapshot;
mod store;

pub use id::Id;
pub use object::{Commit, Kind, Object, Slice, TreeEntry};
pub use repo::{Form, Put, Repo};
pub use store::{FsStore, MemStore, Store};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("object {0} not found")]
    NotFound(Id),
    #[error("object {0} does not match its id")]
    Corrupt(Id),
    #[error("object {0} has the wrong kind for this operation")]
    WrongKind(Id),
    #[error("decode error: {0}")]
    Decode(&'static str),
    #[error("invalid name: {0:?}")]
    InvalidName(String),
    #[error("ref {0} moved or is locked; retry")]
    RefConflict(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
