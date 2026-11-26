//! IKAM kernel: a content-addressed store whose files are guaranteed to
//! reconstruct byte-for-byte, with data-driven dedup, git-like versioning,
//! and a ledger of verified derivations that is at once storage, provenance
//! and the object graph.
//! See docs/plans/2026-10-02-rust-kernel.md for the layer laws.

mod abbrev;
pub mod backends;
pub mod bm25;
mod claims;
mod codec;
pub mod colblock;
mod coldpass;
mod container;
mod db;
mod dict;
pub mod drafts;
#[cfg(feature = "embed")]
pub mod embed;
pub mod embeds;
mod exec;
mod fragment;
pub mod func;
pub mod fusion;
mod grant;
mod graph;
mod group;
mod history;
pub mod hnsw;
pub mod hot;
mod id;
mod ingest;
pub mod integrate;
pub mod ir;
pub mod join;
mod library;
mod markup;
pub mod matcher;
pub mod merge;
pub mod net;
mod object;
mod pdftext;
mod range;
mod repack;
mod repo;
mod search;
pub mod share;
pub mod snapshot;
pub mod stem;
mod store;
pub mod temperature;
mod template;
pub mod tensors;
pub mod vectors;
pub mod views;
mod wasm;
pub mod worktree;

pub use coldpass::{COLD_PASS_REF, ColdPass, ColdPassOpts};
pub use container::expand;
pub use db::{Counts, Db, Event, Op, prefix_end};
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
pub use store::{FsStore, MemStore, SharedStore, Store};
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
