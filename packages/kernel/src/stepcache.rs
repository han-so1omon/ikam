//! A platform-keyed step cache (E428, an option; nothing calls it unless a
//! caller chooses it).
//!
//! Pure derivations already serve as a memo: `apply` finds a recorded
//! derivation and never runs it twice. Recorded and float steps (E441) had
//! none, so the same training step on the same machine ran from scratch
//! every time. Here, as in Bazel's action cache, a step's key is
//! `(function id, input ids, kind, platform tag)`; a hit returns the
//! recorded output without running anything. A run under another platform
//! tag records its own output beside the first, so where two platforms give
//! different bits is visible in the store, not lost by a cache that keeps
//! one answer.
//!
//! Layout: each result is a recorded step (a [`Run`], E441's design B)
//! whose `props` hold the kind and the platform, named by the ref
//! `steps/<group>/<platform>`, where `<group>` hashes the key without its
//! platform. Every platform's result of one step is under one prefix
//! ([`Repo::step_cache_entries`]). The ref keeps the record, its output and
//! its inputs live through gc and repack; fsck requires the output.

use crate::store::check_ref_name;
use crate::{Arg, Check, Error, Id, Object, Repo, Run, Store};

/// Where cache entries live among the refs.
pub const STEP_CACHE_REFS: &str = "steps/";

/// What runs a cached step (part of its key).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// Work done outside the kernel (a command, an agent): the caller runs
    /// it on a miss and hands back its bytes.
    Recorded,
    /// A WASM function the kernel runs with floats pinned (E441).
    Float,
}

impl StepKind {
    pub fn name(self) -> &'static str {
        match self {
            StepKind::Recorded => "recorded",
            StepKind::Float => "float",
        }
    }
}

/// A cached step's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepKey {
    pub func: Id,
    pub args: Vec<Arg>,
    pub kind: StepKind,
    /// The platform the step runs on, as the caller names it (letters,
    /// digits, `.`, `_` and `-`).
    pub platform: String,
}

/// What a cached call returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cached {
    pub output: Id,
    /// The recorded step (a `Run`) that made the output.
    pub record: Id,
    /// Whether the output came from the cache (nothing ran).
    pub hit: bool,
}

/// One platform's result of a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheEntry {
    pub platform: String,
    pub record: Id,
    pub run: Run,
}

impl StepKey {
    pub fn new(func: Id, args: Vec<Arg>, kind: StepKind, platform: &str) -> StepKey {
        StepKey {
            func,
            args,
            kind,
            platform: platform.into(),
        }
    }

    /// The key without its platform: every platform's result of the same
    /// step is filed under it.
    pub fn group(&self) -> Id {
        let mut b = b"ikam step key 1\n".to_vec();
        b.extend_from_slice(self.func.as_bytes());
        b.extend_from_slice(&(self.args.len() as u64).to_be_bytes());
        for a in &self.args {
            match a {
                Arg::Whole(id) => {
                    b.push(0);
                    b.extend_from_slice(id.as_bytes());
                }
                Arg::Range { id, start, len } => {
                    b.push(1);
                    b.extend_from_slice(id.as_bytes());
                    b.extend_from_slice(&start.to_be_bytes());
                    b.extend_from_slice(&len.to_be_bytes());
                }
            }
        }
        b.push(match self.kind {
            StepKind::Recorded => 0,
            StepKind::Float => 1,
        });
        Id::from_bytes(*blake3::hash(&b).as_bytes())
    }

    /// The ref that names this key's result.
    pub fn ref_name(&self) -> Result<String, Error> {
        if self.platform.contains('/') {
            return Err(Error::InvalidName(self.platform.clone()));
        }
        let name = format!("{STEP_CACHE_REFS}{}/{}", self.group(), self.platform);
        check_ref_name(&name)?;
        Ok(name)
    }

    /// What is stored with each result: the kind and the platform.
    fn props(&self) -> Vec<(String, String)> {
        vec![
            ("kind".into(), self.kind.name().into()),
            ("platform".into(), self.platform.clone()),
        ]
    }
}

impl<S: Store> Repo<S> {
    /// The cached result of `key`, if there is one: the record's id and the
    /// record. A record that is not the step its key names is refused.
    pub fn step_cache_get(&self, key: &StepKey) -> Result<Option<(Id, Run)>, Error> {
        let Some(record) = self.store.get_ref(&key.ref_name()?)? else {
            return Ok(None);
        };
        match self.get(&record)? {
            Object::Run(run) if run.func == key.func && run.args == key.args => {
                Ok(Some((record, run)))
            }
            _ => Err(Error::Corrupt(record)),
        }
    }

    /// Run `key`'s step through the cache. On a hit, nothing runs and the
    /// recorded output is returned. On a miss the step runs (`outside`, the
    /// caller's work, for [`StepKind::Recorded`]; the kernel, for
    /// [`StepKind::Float`]), its output is kept as a recorded step with the
    /// kind and platform stored beside it, and the key's ref names it. A
    /// failed run records nothing.
    pub fn step_cached(
        &mut self,
        key: &StepKey,
        check: Option<Check>,
        outside: Option<&mut dyn FnMut() -> Result<Vec<u8>, Error>>,
    ) -> Result<Cached, Error> {
        let name = key.ref_name()?;
        if let Some((record, run)) = self.step_cache_get(key)? {
            if !self.has_content(&run.output)? {
                return Err(Error::NotFound(run.output));
            }
            return Ok(Cached {
                output: run.output,
                record,
                hit: true,
            });
        }
        let bytes = match (key.kind, outside) {
            (StepKind::Recorded, Some(work)) => work()?,
            (StepKind::Float, None) => self.run_step(&key.func, &key.args)?,
            (kind, _) => {
                return Err(Error::Exec(format!(
                    "a {} step {} outside work",
                    kind.name(),
                    if kind == StepKind::Recorded {
                        "needs"
                    } else {
                        "takes no"
                    }
                )));
            }
        };
        let output = self.put_content(&bytes)?.id;
        let record = self.write_run(Run {
            output,
            func: key.func,
            args: key.args.clone(),
            check,
            props: key.props(),
        })?;
        self.store.set_ref(&name, None, record)?;
        Ok(Cached {
            output,
            record,
            hit: false,
        })
    }

    /// Every platform's cached result of `key`'s step (its platform aside),
    /// by platform name.
    pub fn step_cache_entries(&self, key: &StepKey) -> Result<Vec<CacheEntry>, Error> {
        let prefix = format!("{STEP_CACHE_REFS}{}/", key.group());
        let mut out = Vec::new();
        for (name, record) in self.store.refs()? {
            let Some(platform) = name.strip_prefix(&prefix) else {
                continue;
            };
            let Object::Run(run) = self.get(&record)? else {
                return Err(Error::Corrupt(record));
            };
            out.push(CacheEntry {
                platform: platform.to_string(),
                record,
                run,
            });
        }
        out.sort_by(|a, b| a.platform.cmp(&b.platform));
        Ok(out)
    }
}
