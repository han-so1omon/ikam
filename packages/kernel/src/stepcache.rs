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
//!
//! Key rules (E443, from Bazel's `Action` and DVC's run cache), each unset
//! by default, so a key without them is E428's and files where E428 filed it:
//! - the timeout is part of the key: a step allowed 10 s never hits a result
//!   made with 60 s to run (it might not have finished in 10);
//! - a salt is part of the key: changing it disowns every result made
//!   before, on every platform, without deleting one;
//! - a do-not-cache mark (DVC refuses to cache stages marked
//!   `always_changed`): such a step is never looked up and its result is
//!   recorded but never filed for a hit;
//! - the platform may be a set of properties (`cpu=x86-64-v3`, `fma=on`),
//!   matched exactly: a result made under other properties, fewer or more,
//!   is a miss. The properties, the timeout, the salt and what ran the
//!   step (`ran-by`, which is not part of the key, as Bazel's worker is not)
//!   are stored with each result.

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
    /// The platform's properties, matched exactly (E443; empty: the tag
    /// alone). Names are letters, digits, `.`, `_` and `-`; values are any
    /// text on one line. Order does not matter; a name given twice is
    /// refused.
    pub properties: Vec<(String, String)>,
    /// The time the step is allowed, in milliseconds (E443). The caller's
    /// runner enforces it; the cache keys by it.
    pub timeout_ms: Option<u64>,
    /// A salt (E443): change it to disown every earlier result.
    pub salt: String,
    /// Never look the step up and never file its result (E443).
    pub do_not_cache: bool,
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
            properties: Vec::new(),
            timeout_ms: None,
            salt: String::new(),
            do_not_cache: false,
        }
    }

    /// With exact-match platform properties (E443).
    pub fn with_properties(mut self, properties: &[(&str, &str)]) -> StepKey {
        self.properties = (properties.iter())
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self
    }

    /// With a timeout in the key (E443).
    pub fn with_timeout_ms(mut self, ms: u64) -> StepKey {
        self.timeout_ms = Some(ms);
        self
    }

    /// With a salt in the key (E443).
    pub fn with_salt(mut self, salt: &str) -> StepKey {
        self.salt = salt.into();
        self
    }

    /// Marked always different: never cached (E443).
    pub fn not_cached(mut self) -> StepKey {
        self.do_not_cache = true;
        self
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
        // E443's fields, each only when set, so E428's keys stay as they were.
        if let Some(ms) = self.timeout_ms {
            b.push(b'T');
            b.extend_from_slice(&ms.to_be_bytes());
        }
        if !self.salt.is_empty() {
            b.push(b'S');
            b.extend_from_slice(&(self.salt.len() as u64).to_be_bytes());
            b.extend_from_slice(self.salt.as_bytes());
        }
        Id::from_bytes(*blake3::hash(&b).as_bytes())
    }

    /// The properties, sorted by name; a repeated name or a bad name or
    /// value is refused.
    pub fn sorted_properties(&self) -> Result<Vec<(String, String)>, Error> {
        let mut p = self.properties.clone();
        p.sort();
        for w in p.windows(2) {
            if w[0].0 == w[1].0 {
                return Err(Error::InvalidName(w[0].0.clone()));
            }
        }
        for (k, v) in &p {
            let name_ok = !k.is_empty()
                && (k.bytes()).all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
            if !name_ok || v.contains('\n') {
                return Err(Error::InvalidName(format!("{k}={v}")));
            }
        }
        Ok(p)
    }

    /// The platform's place under the key's group: the tag alone (E428),
    /// or, with properties, the tag (`p` if none) and a hash of the tag
    /// and the sorted properties, so any difference in them is a miss.
    pub fn platform_name(&self) -> Result<String, Error> {
        if self.platform.contains('/') {
            return Err(Error::InvalidName(self.platform.clone()));
        }
        let p = self.sorted_properties()?;
        if p.is_empty() {
            return Ok(self.platform.clone());
        }
        let mut b = b"ikam step platform 1\n".to_vec();
        b.extend_from_slice(self.platform.as_bytes());
        b.push(b'\n');
        for (k, v) in &p {
            b.extend_from_slice(format!("{k}={v}\n").as_bytes());
        }
        let hash = blake3::hash(&b).to_hex();
        let tag = if self.platform.is_empty() {
            "p"
        } else {
            &self.platform
        };
        Ok(format!("{tag}-{}", &hash[..32]))
    }

    /// The ref that names this key's result.
    pub fn ref_name(&self) -> Result<String, Error> {
        let name = format!(
            "{STEP_CACHE_REFS}{}/{}",
            self.group(),
            self.platform_name()?
        );
        check_ref_name(&name)?;
        Ok(name)
    }

    /// What is stored with each result: the kind and the platform, and
    /// (E443) each property, the timeout, the salt, what ran it and the
    /// do-not-cache mark, sorted by name.
    fn props(&self, ran_by: Option<&str>) -> Result<Vec<(String, String)>, Error> {
        let mut p = vec![("kind".to_string(), self.kind.name().to_string())];
        if !self.platform.is_empty() {
            p.push(("platform".into(), self.platform.clone()));
        }
        for (k, v) in self.sorted_properties()? {
            p.push((format!("platform.{k}"), v));
        }
        if let Some(ms) = self.timeout_ms {
            p.push(("timeout-ms".into(), ms.to_string()));
        }
        if !self.salt.is_empty() {
            p.push(("salt".into(), self.salt.clone()));
        }
        if let Some(r) = ran_by {
            p.push(("ran-by".into(), r.into()));
        }
        if self.do_not_cache {
            p.push(("cache".into(), "no".into()));
        }
        p.sort();
        Ok(p)
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
            // The record's own platform properties must be the key's, exactly.
            Object::Run(run)
                if run.func == key.func
                    && run.args == key.args
                    && platform_props(&run.props) == key.sorted_properties()? =>
            {
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
        self.step_cached_by(key, check, outside, None)
    }

    /// `step_cached`, storing `ran_by` (what ran the step: a build, a
    /// worker) with a new result (E443). It is not part of the key. A step
    /// marked do-not-cache is never looked up: it runs, its result is kept
    /// as a recorded step, and no ref files it.
    pub fn step_cached_by(
        &mut self,
        key: &StepKey,
        check: Option<Check>,
        outside: Option<&mut dyn FnMut() -> Result<Vec<u8>, Error>>,
        ran_by: Option<&str>,
    ) -> Result<Cached, Error> {
        let name = key.ref_name()?;
        let props = key.props(ran_by)?;
        if !key.do_not_cache
            && let Some((record, run)) = self.step_cache_get(key)?
        {
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
            props,
        })?;
        if !key.do_not_cache {
            self.store.set_ref(&name, None, record)?;
        }
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

/// The `platform.*` properties of a record, by name without the prefix.
fn platform_props(props: &[(String, String)]) -> Vec<(String, String)> {
    let mut p: Vec<(String, String)> = (props.iter())
        .filter_map(|(k, v)| Some((k.strip_prefix("platform.")?.to_string(), v.clone())))
        .collect();
    p.sort();
    p
}
