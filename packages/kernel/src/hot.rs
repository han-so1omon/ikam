//! Hot state kept per ref (E264, opt-in, library only).
//!
//! A branch is a ref (`Repo::branch`), and the database at a ref is read
//! from its commit (`Repo::db_at`). What a ref also needs while edits
//! arrive is its hot state: the newest database its writer appended to,
//! and its derived views (E242) kept current by a background integrator
//! (E262). `HotRefs` keeps both for each ref it has open:
//!
//! - `open(name)` takes the ref's database from its commit and starts an
//!   integrator from the views given (or a build of them).
//! - `fork(from, to)` creates the ref `to` at `from`'s head and starts its
//!   integrator from `from`'s newest view state, however far that lags:
//!   nothing is copied or rebuilt, so a branch costs one ref and a thread.
//! - `append(name, events)` appends, commits to that ref and hands the
//!   ref's integrator the new database.
//! - `merge(into, from)` merges the commits (`Repo::merge`, E265) without
//!   waiting for either side's views ("variant b"): the merge reads only
//!   logs and trees, and `into`'s integrator is handed the merged
//!   database, so the views catch up like after any append. To merge with
//!   current views ("variant a"), `wait` on both sides first; both give
//!   the same database, and views with the same rows.
//! - `wait(name)` returns the views at the ref's newest database;
//!   `close(name)` catches up and stops its integrator, keeping the ref.
//!
//! The cold pass (`Repo::repack_incremental`, E268) is shared: it walks
//! every ref's roots at once and skips what an earlier pass planned, so a
//! branch adds only its own new objects to the next pass. Run it, and gc,
//! only while no integrator runs (see `integrate`). View states live in
//! memory, as in E262: a process that stops starts again from `open`.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::integrate::{Integrator, Policy};
use crate::merge::Merged;
use crate::views::{ViewGraph, ViewState};
use crate::{Db, Error, Event, Repo, Store};

struct HotRef {
    db: Db,
    views: Integrator,
}

/// Hot state for each open ref. See the module docs.
pub struct HotRefs {
    graph: Arc<ViewGraph>,
    policy: Policy,
    refs: BTreeMap<String, HotRef>,
}

impl HotRefs {
    pub fn new(graph: Arc<ViewGraph>, policy: Policy) -> HotRefs {
        HotRefs {
            graph,
            policy,
            refs: BTreeMap::new(),
        }
    }

    fn get(&self, name: &str) -> Result<&HotRef, Error> {
        self.refs
            .get(name)
            .ok_or_else(|| Error::Exec(format!("hot ref {name} is not open")))
    }

    /// Open the ref `name`: its database from its commit, and views from
    /// `views` (which must be at that database or behind it) or built.
    pub fn open<S>(
        &mut self,
        repo: &mut Repo<S>,
        name: &str,
        views: Option<ViewState>,
    ) -> Result<Db, Error>
    where
        S: Store + Clone + Send + 'static,
    {
        if self.refs.contains_key(name) {
            return Err(Error::Exec(format!("hot ref {name} is open")));
        }
        let head = repo.resolve(name)?;
        let db = repo.db_at(&head)?;
        let state = match views {
            Some(s) => s,
            None => repo.views_build(&self.graph, &db)?,
        };
        let views = Integrator::start(repo, self.graph.clone(), state, self.policy);
        views.advance(&db)?;
        self.refs.insert(name.into(), HotRef { db, views });
        Ok(db)
    }

    /// Create the ref `to` at `from`'s head, with `from`'s hot state.
    pub fn fork<S>(&mut self, repo: &mut Repo<S>, from: &str, to: &str) -> Result<(), Error>
    where
        S: Store + Clone + Send + 'static,
    {
        if self.refs.contains_key(to) {
            return Err(Error::Exec(format!("hot ref {to} is open")));
        }
        let src = self.get(from)?;
        let (db, state) = (src.db, src.views.current());
        repo.branch(to, repo.resolve(from)?)?;
        let views = Integrator::start(repo, self.graph.clone(), state, self.policy);
        views.advance(&db)?;
        self.refs.insert(to.into(), HotRef { db, views });
        Ok(())
    }

    /// The ref's newest database.
    pub fn db(&self, name: &str) -> Result<Db, Error> {
        Ok(self.get(name)?.db)
    }

    /// Events between the ref's newest database and its views.
    pub fn lag(&self, name: &str) -> Result<u64, Error> {
        Ok(self.get(name)?.views.lag())
    }

    /// The ref's newest view state, which may lag its database.
    pub fn views(&self, name: &str) -> Result<ViewState, Error> {
        Ok(self.get(name)?.views.current())
    }

    /// Append `events` on the ref, commit them to it, and hand the new
    /// database to its integrator.
    pub fn append<S: Store>(
        &mut self,
        repo: &mut Repo<S>,
        name: &str,
        events: &[Event],
        message: &str,
    ) -> Result<Db, Error> {
        let h = self
            .refs
            .get_mut(name)
            .ok_or_else(|| Error::Exec(format!("hot ref {name} is not open")))?;
        let db = repo.db_append(&h.db, events)?;
        repo.db_commit(name, &db, message)?;
        h.db = db;
        h.views.advance(&db)?;
        Ok(db)
    }

    /// Merge `from` into `into` without waiting for views. On a merge
    /// commit or a fast-forward, `into`'s integrator gets the merged
    /// database. `from` stays open.
    pub fn merge<S: Store>(
        &mut self,
        repo: &mut Repo<S>,
        into: &str,
        from: &str,
        message: &str,
    ) -> Result<Merged, Error> {
        self.get(into)?;
        let merged = repo.merge(into, from, message)?;
        if let Merged::Commit(id) | Merged::FastForward(id) = merged {
            let db = repo.db_at(&id)?;
            let h = self.refs.get_mut(into).expect("checked above");
            h.db = db;
            h.views.advance(&db)?;
        }
        Ok(merged)
    }

    /// Fold what is pending and return the ref's views at its newest
    /// database.
    pub fn wait(&self, name: &str) -> Result<ViewState, Error> {
        let h = self.get(name)?;
        let state = h.views.wait(&h.db)?;
        if state.db != h.db {
            return Err(Error::Exec(format!("hot ref {name}: views at another db")));
        }
        Ok(state)
    }

    /// Catch up and stop the ref's integrator; the ref stays.
    pub fn close(&mut self, name: &str) -> Result<ViewState, Error> {
        let h = self
            .refs
            .remove(name)
            .ok_or_else(|| Error::Exec(format!("hot ref {name} is not open")))?;
        h.views.wait(&h.db)?;
        h.views.stop()
    }

    /// The open refs, in order.
    pub fn names(&self) -> Vec<String> {
        self.refs.keys().cloned().collect()
    }
}
