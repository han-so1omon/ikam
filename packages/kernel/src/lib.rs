//! IKAM kernel: a content-addressed store whose files are guaranteed to
//! reconstruct byte-for-byte, with data-driven dedup, git-like versioning,
//! and a ledger of verified derivations that is at once storage, provenance
//! and the object graph.
//! See docs/plans/2026-10-02-rust-kernel.md for the layer laws.

mod abbrev;
mod claims;
mod codec;
mod container;
mod dict;
mod exec;
pub mod func;
mod graph;
mod group;
mod history;
mod id;
mod ingest;
mod library;
pub mod matcher;
mod object;
mod range;
mod repack;
mod repo;
pub mod snapshot;
mod store;
mod template;
mod wasm;

pub use dict::decompressed_bytes;
pub use id::Id;
pub use object::{
    Arg, Claim, Commit, Derivation, Edge, Graph, Kind, Node, Object, Target, TreeEntry, node_key,
};
pub use repack::Repacked;
pub use repo::{DEFAULT_READ_WEIGHT, Form, Put, Repo};
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
    #[error("execution failed: {0}")]
    Exec(String),
    #[error("ref {0} moved or is locked; retry")]
    RefConflict(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
