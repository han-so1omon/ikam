//! Execution and graph connectivity.
//!
//! `apply` evaluates a pure function and records the derivation
//! `output = func(args)`. That ledger entry is the memo, the provenance edge,
//! and a storage option (`repack` may drop the output's bytes and rely on
//! it). Functions are pure (builtins, or sandboxed WASM without imports), so
//! a recorded derivation stays valid forever.

use crate::repo::{Cx, decode_plain};
use crate::{Arg, Derivation, Error, Id, Object, Repo, Store};

impl<S: Store> Repo<S> {
    /// Evaluate `func(args)`, reusing a recorded derivation when one exists.
    /// Returns the output's content id.
    pub fn apply(&mut self, func: Id, args: Vec<Arg>) -> Result<Id, Error> {
        if let Some(out) = self
            .ledger()?
            .by_key
            .get(&Derivation::key(&func, &args))
            .copied()
            && self.has_content(&out)?
        {
            return Ok(out);
        }
        let bytes = self.eval(&func, &args, &mut Cx::default())?;
        let output = self.put_content(&bytes)?.id;
        self.record(Derivation { output, func, args })?;
        Ok(output)
    }

    /// Outgoing edges of a stored object, or of a content id (its
    /// derivation records, labelled "derivation").
    pub fn links(&self, id: &Id) -> Result<Vec<(String, Id)>, Error> {
        let mut out = match self.store.read(id) {
            Ok(encoded) if decode_plain(id, &encoded).is_err() => Object::decode(&encoded)?.links(),
            Ok(_) | Err(Error::NotFound(_)) => vec![],
            Err(e) => return Err(e),
        };
        for d in self.derivations(id)? {
            out.push(("derivation".to_string(), Object::Derivation(d).id()));
        }
        if out.is_empty() && !self.has_content(id)? {
            return Err(Error::NotFound(*id));
        }
        Ok(out)
    }

    /// Incoming edges: every stored object that links to `id`, with the
    /// edge label. A full scan; a persisted reverse index is a projection
    /// to add when this becomes hot.
    pub fn used_by(&self, id: &Id) -> Result<Vec<(Id, String)>, Error> {
        let mut out = Vec::new();
        for src in self.store.ids()? {
            let encoded = self.store.read(&src)?;
            if decode_plain(&src, &encoded).is_ok() {
                continue;
            }
            for (label, target) in Object::decode(&encoded)?.links() {
                if target == *id {
                    out.push((src, label));
                }
            }
        }
        Ok(out)
    }
}
