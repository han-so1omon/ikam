//! Execution and graph connectivity.
//!
//! `apply` evaluates a pure function and records a `Run` (apply -> output).
//! The run record is both the memo entry and the provenance edge. Because
//! every function is pure (builtins, or sandboxed WASM without imports or
//! floats), a recorded output stays valid forever.

use std::collections::HashMap;

use crate::{Apply, Error, Id, Object, Repo, Run, Store};

impl<S: Store> Repo<S> {
    /// Evaluate `apply`, reusing a recorded run when one exists. Returns the
    /// output's content id.
    pub fn apply(&mut self, apply: Apply) -> Result<Id, Error> {
        let key = Object::Apply(apply.clone()).id();
        self.ensure_runs()?;
        if let Some(out) = self.runs.as_ref().unwrap().get(&key)
            && self.store.has(out)
        {
            return Ok(*out);
        }
        let bytes = self.eval(&apply, 0)?;
        let output = self.put_content(&bytes)?.id;
        self.put(&Object::Run(Run { apply, output }))?;
        self.runs.as_mut().unwrap().insert(key, output);
        Ok(output)
    }

    fn ensure_runs(&mut self) -> Result<(), Error> {
        if self.runs.is_none() {
            let mut runs = HashMap::new();
            for id in self.store.ids()? {
                if let Ok(Object::Run(run)) = self.get(&id) {
                    runs.insert(Object::Apply(run.apply).id(), run.output);
                }
            }
            self.runs = Some(runs);
        }
        Ok(())
    }

    /// Outgoing edges of any stored object or content: slice sources,
    /// function and argument ids, tree entries, commit parents, run outputs.
    pub fn links(&self, id: &Id) -> Result<Vec<(String, Id)>, Error> {
        let encoded = self.store.read(id)?;
        match encoded.first() {
            Some(b'B' | b'Z') => Ok(vec![]),
            _ => Ok(Object::decode(&encoded)?.links()),
        }
    }

    /// Incoming edges: every stored object that links to `id`, with the
    /// edge label. A full scan; a persisted reverse index is a projection
    /// to add when this becomes hot.
    pub fn used_by(&self, id: &Id) -> Result<Vec<(Id, String)>, Error> {
        let mut out = Vec::new();
        for src in self.store.ids()? {
            for (label, target) in self.links(&src)? {
                if target == *id {
                    out.push((src, label));
                }
            }
        }
        Ok(out)
    }
}
