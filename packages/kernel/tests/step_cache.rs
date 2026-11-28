//! The step cache (E428): a recorded or float step keyed by (function,
//! inputs, kind, platform) runs once per platform; a hit returns the
//! recorded output, and another platform's result sits beside it.

#[path = "common/float_wat.rs"]
mod float_wat;

use std::cell::Cell;

use float_wat::{bytes, inputs, wat};
use ikam_kernel::stepcache::{STEP_CACHE_REFS, StepKey, StepKind};
use ikam_kernel::{Arg, Error, Id, MemStore, Object, Repo, Run, Store};

fn key(func: Id, args: &[Arg], kind: StepKind, platform: &str) -> StepKey {
    StepKey::new(func, args.to_vec(), kind, platform)
}

fn cache_refs(repo: &Repo<MemStore>) -> usize {
    let refs = repo.store().refs().unwrap();
    refs.iter()
        .filter(|(n, _)| n.starts_with(STEP_CACHE_REFS))
        .count()
}

#[test]
fn a_second_run_on_one_platform_is_a_hit_and_another_platform_sits_beside_it() {
    let mut repo = Repo::new(MemStore::default());
    let func = repo.put_content(b"train: 400 steps of descent").unwrap().id;
    let data = repo.put_content(b"seeded data").unwrap().id;
    let args = [Arg::Whole(data)];
    let runs = Cell::new(0);
    let work = |out: &'static [u8]| {
        let runs = &runs;
        move || {
            runs.set(runs.get() + 1);
            Ok(out.to_vec())
        }
    };
    let a = key(func, &args, StepKind::Recorded, "x86-64-plain");
    let first = repo
        .step_cached(&a, None, Some(&mut work(b"weights a")))
        .unwrap();
    assert!(!first.hit);
    let again = repo
        .step_cached(&a, None, Some(&mut work(b"never run")))
        .unwrap();
    assert!(again.hit);
    assert_eq!((again.output, again.record), (first.output, first.record));
    assert_eq!(runs.get(), 1, "a hit runs nothing");

    let b = key(func, &args, StepKind::Recorded, "x86-64-v3-fma");
    let other = repo
        .step_cached(&b, None, Some(&mut work(b"weights b")))
        .unwrap();
    assert!(!other.hit);
    assert_eq!(runs.get(), 2);
    let entries = repo.step_cache_entries(&a).unwrap();
    let side: Vec<(&str, Id)> = (entries.iter())
        .map(|e| (e.platform.as_str(), e.run.output))
        .collect();
    assert_eq!(
        side,
        [
            ("x86-64-plain", first.output),
            ("x86-64-v3-fma", other.output)
        ]
    );
    assert_eq!(
        entries[1].run.props,
        [
            ("kind".to_string(), "recorded".to_string()),
            ("platform".to_string(), "x86-64-v3-fma".to_string())
        ]
    );
    assert_eq!(repo.step_records(&other.output).unwrap().len(), 1);

    // Nothing else holds the outputs: the cache's refs keep them, and their
    // inputs, through gc and repack.
    repo.gc().unwrap();
    repo.repack().unwrap();
    assert_eq!(repo.read_content(&first.output).unwrap(), b"weights a");
    assert_eq!(repo.read_content(&other.output).unwrap(), b"weights b");
    assert_eq!(repo.read_content(&data).unwrap(), b"seeded data");
    assert!(repo.fsck().unwrap().is_empty());
    assert!(
        repo.step_cached(&a, None, Some(&mut work(b"x")))
            .unwrap()
            .hit
    );
    assert_eq!(runs.get(), 2);
}

#[test]
fn the_kind_is_in_the_key_and_float_steps_run_in_the_kernel() {
    let mut repo = Repo::new(MemStore::default());
    let module = repo
        .put_content(&wat::parse_str(wat("dot")).unwrap())
        .unwrap()
        .id;
    let (x, y) = inputs("plain", 256);
    let x = repo.put_content(&bytes(&x)).unwrap().id;
    let y = repo.put_content(&bytes(&y)).unwrap().id;
    let args = [Arg::Whole(x), Arg::Whole(y)];
    let f = key(module, &args, StepKind::Float, "any");
    let first = repo.step_cached(&f, None, None).unwrap();
    assert!(!first.hit);
    let want = repo.apply_pinned(module, args.to_vec()).unwrap();
    assert_eq!(first.output, want, "the kernel ran it with floats pinned");
    assert!(repo.step_cached(&f, None, None).unwrap().hit);
    // The same function and inputs as work done elsewhere: another key.
    let r = key(module, &args, StepKind::Recorded, "any");
    assert_ne!(r.group(), f.group());
    let elsewhere = repo
        .step_cached(&r, None, Some(&mut || Ok(b"host".to_vec())))
        .unwrap();
    assert!(!elsewhere.hit);
    assert_eq!(cache_refs(&repo), 2);
    // A float step takes no outside work; a recorded one needs it.
    let other = key(module, &args, StepKind::Float, "other");
    assert!(matches!(
        repo.step_cached(&other, None, Some(&mut || Ok(vec![]))),
        Err(Error::Exec(_))
    ));
    let other = key(module, &args, StepKind::Recorded, "other");
    assert!(matches!(
        repo.step_cached(&other, None, None),
        Err(Error::Exec(_))
    ));
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn off_unless_asked_and_records_without_props_keep_their_bytes() {
    let mut repo = Repo::new(MemStore::default());
    let func = repo.put_content(b"cmd").unwrap().id;
    let arg = repo.put_content(b"in").unwrap().id;
    repo.record_step(func, vec![Arg::Whole(arg)], Some(b"out"), None)
        .unwrap();
    assert_eq!(cache_refs(&repo), 0, "recording a step caches nothing");

    let out = Id::of_content(b"out");
    let run = Run {
        output: out,
        func,
        args: vec![Arg::Whole(arg)],
        check: None,
        props: vec![],
    };
    // E441's layout: tag, output, function, args, then the check byte last.
    let enc = Object::Run(run.clone()).encode();
    assert_eq!(enc.len(), 1 + 32 + 32 + 4 + 33 + 1);
    assert_eq!(enc.last(), Some(&0));
    let with = Run {
        props: vec![("platform".into(), "p".into())],
        ..run
    };
    let enc = Object::Run(with.clone()).encode();
    assert_eq!(Object::decode(&enc).unwrap(), Object::Run(with));
    // An empty list would be a second encoding of no props.
    let mut empty = Object::Run(Run {
        output: out,
        func,
        args: vec![],
        check: None,
        props: vec![],
    })
    .encode();
    empty.extend_from_slice(&0u32.to_be_bytes());
    assert!(Object::decode(&empty).is_err());
}

#[test]
fn a_failed_run_records_nothing_and_bad_platform_names_are_refused() {
    let mut repo = Repo::new(MemStore::default());
    let func = repo.put_content(b"cmd").unwrap().id;
    let k = key(func, &[], StepKind::Recorded, "p");
    let failed = repo.step_cached(&k, None, Some(&mut || Err(Error::Exec("timed out".into()))));
    assert!(matches!(failed, Err(Error::Exec(_))));
    assert_eq!(cache_refs(&repo), 0);
    assert!(repo.step_cache_get(&k).unwrap().is_none());
    for bad in ["", "a/b", ".hidden", "sp ace"] {
        let k = key(func, &[], StepKind::Recorded, bad);
        assert!(matches!(k.ref_name(), Err(Error::InvalidName(_))), "{bad}");
    }
}
