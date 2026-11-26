//! A run: a sequence of states (load steps, time steps, control ticks)
//! recorded as a stream. Each state is content (so a state equal to an
//! earlier one is stored once), and each step is an event in an ikam
//! database: `run/<name>/<step>` holds the state's id, with the step's
//! parameters beside it, under an idempotency key, so a retried append
//! changes nothing and a step logged twice with different states is
//! refused. A run is committed on a ref like any database, so it is
//! versioned, merged and shared by records scope.

use ikam_kernel::{Db, Event, Id, Repo, Store};

use crate::{Error, Tensor};

pub struct Run {
    pub name: String,
    pub db: Db,
    pub steps: u64,
    pending: Vec<Event>,
}

pub fn step_key(name: &str, step: u64) -> String {
    format!("run/{name}/{step:010}")
}

impl Run {
    pub fn start<S: Store>(repo: &mut Repo<S>, name: &str) -> Result<Run, Error> {
        Ok(Run { name: name.into(), db: repo.db_empty()?, steps: 0, pending: Vec::new() })
    }

    /// Record the next state and its parameters (any bytes: a load, a
    /// time, a command). Appended on `flush`.
    pub fn record<S: Store>(&mut self, repo: &mut Repo<S>, state: &Tensor, params: &[u8]) -> Result<Id, Error> {
        let id = repo.put_content(&state.encode())?.id;
        let k = step_key(&self.name, self.steps);
        self.pending.push(Event::upsert(&format!("{k}/state"), id.as_bytes(), &format!("{k}/state")));
        self.pending.push(Event::upsert(&format!("{k}/params"), params, &format!("{k}/params")));
        self.steps += 1;
        Ok(id)
    }

    pub fn flush<S: Store>(&mut self, repo: &mut Repo<S>) -> Result<(), Error> {
        if !self.pending.is_empty() {
            self.db = repo.db_append(&self.db, &self.pending)?;
            self.pending.clear();
        }
        Ok(())
    }

    /// Flush and commit on ref `runs/<name>`.
    pub fn commit<S: Store>(&mut self, repo: &mut Repo<S>, message: &str) -> Result<Id, Error> {
        self.flush(repo)?;
        Ok(repo.db_commit(&format!("runs/{}", self.name), &self.db, message)?)
    }

    pub fn state<S: Store>(&self, repo: &Repo<S>, step: u64) -> Result<Option<Tensor>, Error> {
        let k = format!("{}/state", step_key(&self.name, step));
        match repo.db_get(&self.db, &k)? {
            Some(b) if b.len() == 32 => {
                let id = Id::from_bytes(b.try_into().unwrap());
                Ok(Some(Tensor::decode(&repo.read_content(&id)?)?))
            }
            _ => Ok(None),
        }
    }
}
