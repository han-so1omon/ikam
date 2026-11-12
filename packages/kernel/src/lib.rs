//! IKAM kernel: a content-addressed store whose files are guaranteed to
//! reconstruct byte-for-byte, with data-driven dedup, git-like versioning,
//! and a ledger of verified derivations that is at once storage, provenance
//! and the object graph.
//! See docs/plans/2026-10-02-rust-kernel.md for the layer laws.

mod abbrev;
pub mod backends;
mod claims;
mod codec;
pub mod colblock;
mod container;
mod db;
mod dict;
mod exec;
mod fragment;
pub mod func;
mod grant;
mod graph;
mod group;
mod history;
mod id;
mod ingest;
pub mod ir;
mod library;
mod markup;
pub mod matcher;
mod object;
mod pdftext;
mod range;
mod repack;
mod repo;
mod search;
pub mod snapshot;
mod store;
mod template;
mod wasm;
pub mod worktree;

pub use container::expand;
pub use db::{Db, Event, Op};
pub use dict::decode_cost;
pub use fragment::{FragmentIndex, blocks};
pub use grant::{Denied, FileGrant, FragmentGrant, Grant, Sections, windows_around};
pub use graph::Keys;
pub use id::Id;
pub use object::{
    Arg, Claim, Commit, Derivation, Edge, Graph, Kind, Node, Object, Target, TreeEntry, node_key,
};
pub use range::ReadSession;
pub use repack::Repacked;
pub use repo::{DEFAULT_READ_WEIGHT, Form, Put, Repo};
pub use search::{SEARCH_REF, SearchIndex};
pub use store::{FsStore, MemStore, Store};
pub use worktree::Edit;

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
    #[error("idempotency key {0:?} is already logged with a different event")]
    IdempotencyConflict(String),
    #[error("ref {0} moved or is locked; retry")]
    RefConflict(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
