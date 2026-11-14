//! A background integrator for view graphs (E262, opt-in, library only).
//!
//! The hot path appends to the log and returns; the views (E242) catch up
//! on another thread. `Integrator::start` takes a view state and a handle
//! whose store its clones share (such as `SharedStore`); a worker thread
//! on a sibling handle (`Repo::sibling`) folds the log into the views with
//! `Repo::views_update` whenever the views lag the newest database it was
//! given by at least `Policy::min_events` events, folding everything that
//! has arrived in one pass.
//!
//! - `advance(db)` hands the worker the database after an append and
//!   returns at once, unless `Policy::max_lag` is set and the views are
//!   further behind than that: then it waits until a pass brings them
//!   within it (backpressure).
//! - `current()` is the newest state the worker has written, with the
//!   database position it reflects; `lag()` is how many events behind
//!   the newest database it is.
//! - `wait(db)` folds now whatever is pending, whatever `min_events`
//!   says, and returns the state at `db` (a read that must see its own
//!   write).
//! - `stop()` catches up, ends the worker and returns the last state.
//!
//! The swap law holds for every state the worker writes: it is what
//! `views_build` gives at the same database, because it is what
//! `views_update` gives. The state lives in memory: a process that stops
//! loses the views' position, and the caller starts again from the last
//! state it saved (or rebuilds). Nothing else may gc the store while the
//! worker runs: gc does not see the worker's state as live.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

use crate::views::{ViewGraph, ViewState};
use crate::{Db, Error, Repo, Store};

/// When the worker folds.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    /// Fold once the views lag by at least this many events (at least 1).
    pub min_events: u64,
    /// If set, `advance` waits while the views lag by more than this.
    pub max_lag: Option<u64>,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            min_events: 1,
            max_lag: None,
        }
    }
}

struct Shared {
    target: Db,
    state: ViewState,
    passes: u64,
    flush: bool,
    stop: bool,
    error: Option<String>,
}

impl Shared {
    fn lag(&self) -> u64 {
        self.target.kv_at.saturating_sub(self.state.db.kv_at)
    }

    fn failed(&self) -> Result<(), Error> {
        match &self.error {
            Some(e) => Err(Error::Exec(format!("integrator: {e}"))),
            None => Ok(()),
        }
    }
}

type Inner = Arc<(Mutex<Shared>, Condvar)>;

/// Views kept current on another thread. See the module docs.
pub struct Integrator {
    inner: Inner,
    policy: Policy,
    worker: Option<JoinHandle<()>>,
}

fn lock(inner: &Inner) -> MutexGuard<'_, Shared> {
    inner.0.lock().unwrap_or_else(|e| e.into_inner())
}

impl Integrator {
    /// Start a worker on a sibling of `repo` that keeps `state` current
    /// for `graph`. `state` is the views at some database, as
    /// `views_build` or `views_update` returned them.
    pub fn start<S>(
        repo: &Repo<S>,
        graph: Arc<ViewGraph>,
        state: ViewState,
        policy: Policy,
    ) -> Integrator
    where
        S: Store + Clone + Send + 'static,
    {
        let mut worker_repo = repo.sibling();
        // New view chunks look for repeats in the views' own chunks, not
        // the whole store (E133).
        worker_repo.set_seed_scope(Some(state.roots.values().copied().collect()));
        let inner: Inner = Arc::new((
            Mutex::new(Shared {
                target: state.db,
                state,
                passes: 0,
                flush: false,
                stop: false,
                error: None,
            }),
            Condvar::new(),
        ));
        let min = policy.min_events.max(1);
        let theirs = inner.clone();
        let worker = std::thread::spawn(move || {
            let cv = &theirs.1;
            loop {
                let (current, target) = {
                    let mut s = lock(&theirs);
                    loop {
                        let lag = s.lag();
                        if lag > 0 && (lag >= min || s.flush || s.stop) {
                            break;
                        }
                        if s.stop {
                            return;
                        }
                        s.flush = false;
                        s = cv.wait(s).unwrap_or_else(|e| e.into_inner());
                    }
                    (s.state.clone(), s.target)
                };
                let next = worker_repo.views_update(&graph, &current, &target);
                let mut s = lock(&theirs);
                match next {
                    Ok(next) => {
                        s.state = next;
                        s.passes += 1;
                        if s.lag() == 0 {
                            s.flush = false;
                        }
                    }
                    Err(e) => {
                        s.error = Some(e.to_string());
                        cv.notify_all();
                        return;
                    }
                }
                cv.notify_all();
            }
        });
        Integrator {
            inner,
            policy,
            worker: Some(worker),
        }
    }

    /// Hand the worker the database after an append. Waits only while
    /// the views lag by more than `Policy::max_lag`.
    pub fn advance(&self, db: &Db) -> Result<(), Error> {
        let cv = &self.inner.1;
        let mut s = lock(&self.inner);
        s.failed()?;
        if db.kv_at > s.target.kv_at {
            s.target = *db;
            cv.notify_all();
        }
        if let Some(max) = self.policy.max_lag {
            while s.lag() > max && s.error.is_none() {
                s = cv.wait(s).unwrap_or_else(|e| e.into_inner());
            }
        }
        s.failed()
    }

    /// The newest state the worker has written.
    pub fn current(&self) -> ViewState {
        lock(&self.inner).state.clone()
    }

    /// Events between the newest database given and the views.
    pub fn lag(&self) -> u64 {
        lock(&self.inner).lag()
    }

    /// Passes the worker has made.
    pub fn passes(&self) -> u64 {
        lock(&self.inner).passes
    }

    /// Fold whatever is pending now and return the views at `db` (or
    /// later, if a newer database was given meanwhile).
    pub fn wait(&self, db: &Db) -> Result<ViewState, Error> {
        let cv = &self.inner.1;
        let mut s = lock(&self.inner);
        if db.kv_at > s.target.kv_at {
            s.target = *db;
        }
        s.flush = true;
        cv.notify_all();
        while s.state.db.kv_at < db.kv_at && s.error.is_none() {
            s = cv.wait(s).unwrap_or_else(|e| e.into_inner());
        }
        s.failed()?;
        Ok(s.state.clone())
    }

    /// Catch up, end the worker and return the last state.
    pub fn stop(mut self) -> Result<ViewState, Error> {
        self.end();
        let s = lock(&self.inner);
        s.failed()?;
        Ok(s.state.clone())
    }

    fn end(&mut self) {
        {
            let mut s = lock(&self.inner);
            s.stop = true;
            self.inner.1.notify_all();
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

impl Drop for Integrator {
    fn drop(&mut self) {
        self.end();
    }
}
