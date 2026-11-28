//! Step cache key rules (E443): the timeout and a salt are part of the key,
//! a do-not-cache mark keeps a step out of the cache, and platform
//! properties match exactly; each result stores them and what ran it.

use std::cell::Cell;

use ikam_kernel::stepcache::{STEP_CACHE_REFS, StepKey, StepKind};
use ikam_kernel::{Arg, Error, Id, MemStore, Repo, Store};

struct Fixture {
    repo: Repo<MemStore>,
    func: Id,
    args: Vec<Arg>,
    runs: Cell<u32>,
}

impl Fixture {
    fn new() -> Fixture {
        let mut repo = Repo::new(MemStore::default());
        let func = repo.put_content(b"train").unwrap().id;
        let data = repo.put_content(b"data").unwrap().id;
        Fixture {
            repo,
            func,
            args: vec![Arg::Whole(data)],
            runs: Cell::new(0),
        }
    }

    fn key(&self, platform: &str) -> StepKey {
        StepKey::new(self.func, self.args.clone(), StepKind::Recorded, platform)
    }

    /// Run through the cache; each run that happens makes new bytes.
    fn run(&mut self, key: &StepKey, ran_by: Option<&str>) -> (bool, Id) {
        let runs = &self.runs;
        let mut work = || {
            runs.set(runs.get() + 1);
            Ok(format!("out {}", runs.get()).into_bytes())
        };
        let c = (self.repo)
            .step_cached_by(key, None, Some(&mut work), ran_by)
            .unwrap();
        (c.hit, c.output)
    }

    fn refs(&self) -> usize {
        let refs = self.repo.store().refs().unwrap();
        refs.iter()
            .filter(|(n, _)| n.starts_with(STEP_CACHE_REFS))
            .count()
    }
}

#[test]
fn keys_without_the_new_fields_are_e428_keys() {
    let f = Fixture::new();
    let k = f.key("p");
    // E428's layout: prefix, function, arg count, args, kind.
    let mut b = b"ikam step key 1\n".to_vec();
    b.extend_from_slice(f.func.as_bytes());
    b.extend_from_slice(&1u64.to_be_bytes());
    b.push(0);
    b.extend_from_slice(f.args[0].id().as_bytes());
    b.push(0);
    assert_eq!(k.group(), Id::from_bytes(*blake3::hash(&b).as_bytes()));
    assert_eq!(k.platform_name().unwrap(), "p");
    for other in [
        k.clone().with_timeout_ms(1),
        k.clone().with_salt("s"),
        k.clone().with_timeout_ms(1).with_salt("s"),
    ] {
        assert_ne!(other.group(), k.group());
    }
    assert_eq!(k.clone().not_cached().group(), k.group());
}

#[test]
fn a_shorter_timeout_never_hits_a_longer_ones_result() {
    let mut f = Fixture::new();
    let long = f.key("p").with_timeout_ms(60_000);
    let short = f.key("p").with_timeout_ms(10_000);
    assert!(!f.run(&long, None).0);
    assert!(f.run(&long, None).0);
    assert!(!f.run(&short, None).0, "10 s must not hit a 60 s result");
    assert!(f.run(&short, None).0);
    assert!(!f.run(&f.key("p"), None).0, "no timeout is another key");
    assert_eq!(f.runs.get(), 3);
    let (_, run) = f.repo.step_cache_get(&short).unwrap().unwrap();
    assert!(run.props.contains(&("timeout-ms".into(), "10000".into())));
    // A run that times out records nothing; the next call runs again.
    let tiny = f.key("p").with_timeout_ms(1);
    let mut late = || Err(Error::Exec("timed out".into()));
    assert!(f.repo.step_cached(&tiny, None, Some(&mut late)).is_err());
    assert!(f.repo.step_cache_get(&tiny).unwrap().is_none());
}

#[test]
fn a_salt_disowns_every_earlier_result_without_deleting_one() {
    let mut f = Fixture::new();
    let (_, a) = f.run(&f.key("a"), None);
    let (_, b) = f.run(&f.key("b"), None);
    let before = (f.refs(), f.repo.store().ids().unwrap().len());
    for p in ["a", "b"] {
        let salted = f.key(p).with_salt("fixed-1");
        assert!(!f.run(&salted, None).0, "{p}: disowned");
        assert!(f.run(&salted, None).0);
    }
    assert_eq!(f.runs.get(), 4);
    // The old results are all still there, and still answer the old key.
    assert_eq!(f.refs(), before.0 + 2);
    assert!(f.repo.store().ids().unwrap().len() > before.1);
    assert_eq!(f.run(&f.key("a"), None), (true, a));
    assert_eq!(f.run(&f.key("b"), None), (true, b));
    f.repo.gc().unwrap();
    assert_eq!(f.repo.read_content(&a).unwrap(), b"out 1");
    assert!(f.repo.fsck().unwrap().is_empty());
}

#[test]
fn a_do_not_cache_step_runs_every_time_and_is_kept_but_never_filed() {
    let mut f = Fixture::new();
    let k = f.key("p");
    let (_, cached) = f.run(&k, None);
    let marked = k.clone().not_cached();
    let (hit1, one) = f.run(&marked, None);
    let (hit2, two) = f.run(&marked, None);
    assert!(!hit1 && !hit2, "never looked up, even with a result filed");
    assert_ne!(one, two);
    assert_eq!(f.refs(), 1, "only the unmarked run is filed");
    for out in [one, two] {
        let records = f.repo.step_records(&out).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].1.props.contains(&("cache".into(), "no".into())));
    }
    assert_eq!(f.run(&k, None), (true, cached));
}

#[test]
fn platform_properties_match_exactly_and_store_what_ran_the_step() {
    let mut f = Fixture::new();
    let props = [("arch", "x86_64"), ("cpu", "x86-64-v3"), ("fma", "on")];
    let k = f.key("").with_properties(&props);
    let (hit, out) = f.run(&k, Some("build 1"));
    assert!(!hit);
    // Another order, another runner: the same platform, a hit.
    let reordered =
        f.key("")
            .with_properties(&[("fma", "on"), ("arch", "x86_64"), ("cpu", "x86-64-v3")]);
    assert_eq!(f.run(&reordered, Some("build 2")), (true, out));
    let (_, run) = f.repo.step_cache_get(&reordered).unwrap().unwrap();
    assert!(run.props.contains(&("ran-by".into(), "build 1".into())));
    assert!(run.props.contains(&("platform.fma".into(), "on".into())));
    // Fewer, more or different properties: misses.
    for other in [
        vec![("arch", "x86_64"), ("cpu", "x86-64-v3")],
        vec![("arch", "x86_64"), ("cpu", "x86-64-v3"), ("fma", "off")],
        vec![
            ("arch", "x86_64"),
            ("cpu", "x86-64-v3"),
            ("fma", "on"),
            ("os", "linux"),
        ],
    ] {
        let k = f.key("").with_properties(&other);
        assert!(!f.run(&k, None).0, "{other:?}");
    }
    assert_eq!(f.repo.step_cache_entries(&k).unwrap().len(), 4);
    let dup = f.key("").with_properties(&[("fma", "on"), ("fma", "off")]);
    assert!(matches!(dup.ref_name(), Err(Error::InvalidName(_))));
}
