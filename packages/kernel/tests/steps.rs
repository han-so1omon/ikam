//! Step kinds (E441): pinned-float steps are pure derivations that repack
//! may rebuild; recorded steps keep their outputs, and a rerun is a check
//! judged by a check step stored with them.

#[path = "common/float_wat.rs"]
mod float_wat;

use float_wat::{Arith, bytes, f64s, inputs, native, wat};
use ikam_kernel::step::Design;
use ikam_kernel::{Arg, Check, Error, Id, Kind, MemStore, Object, Repo, Store, TreeEntry, func};

fn module(repo: &mut Repo<MemStore>, f: &str) -> Id {
    repo.put_content(&wat::parse_str(wat(f)).unwrap())
        .unwrap()
        .id
}

fn args(repo: &mut Repo<MemStore>, f: &str, input: &str) -> Vec<Arg> {
    let n = if f == "matmul" { 64 } else { 256 };
    let (a, b) = inputs(input, n);
    let a = repo.put_content(&bytes(&a)).unwrap().id;
    let b = repo.put_content(&bytes(&b)).unwrap().id;
    match f {
        "dot" | "matmul" => vec![Arg::Whole(a), Arg::Whole(b)],
        _ => vec![Arg::Whole(a)],
    }
}

fn commit(repo: &mut Repo<MemStore>, ids: &[Id]) {
    let entries = (ids.iter().enumerate())
        .map(|(i, id)| TreeEntry {
            name: format!("f{i}"),
            kind: Kind::File,
            id: *id,
        })
        .collect();
    let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
}

#[test]
fn floats_stay_off_by_default() {
    let mut repo = Repo::new(MemStore::default());
    let m = module(&mut repo, "dot");
    let a = args(&mut repo, "dot", "plain");
    assert!(matches!(repo.apply(m, a), Err(Error::Exec(_))));
}

#[test]
fn pinned_float_steps_match_pinned_native_and_rebuild_after_repack() {
    let mut repo = Repo::new(MemStore::default());
    let mut outs = Vec::new();
    for f in float_wat::FUNCS {
        let m = module(&mut repo, f);
        for input in ["plain", "cancel", "edges"] {
            let a = args(&mut repo, f, input);
            let out = repo.apply_pinned(m, a.clone()).unwrap();
            let (x, y) = {
                let x = f64s(&repo.read_content(&a[0].id()).unwrap());
                let y = a.get(1).map(|b| f64s(&repo.read_content(&b.id()).unwrap()));
                (x, y.unwrap_or_default())
            };
            let want = bytes(&native(f, &x, &y, Arith::Pinned));
            assert_eq!(repo.read_content(&out).unwrap(), want, "{f} {input}");
            assert_eq!(repo.apply_pinned(m, a).unwrap(), out, "memoized");
            outs.push((out, want));
        }
    }
    commit(&mut repo, &outs.iter().map(|o| o.0).collect::<Vec<_>>());
    // Big outputs that a derivation rebuilds lose their bytes.
    repo.repack().unwrap();
    let dropped = outs.iter().filter(|(id, _)| !repo.store().has(id)).count();
    assert!(dropped > 0, "repack rebuilds pinned-float outputs");
    for (id, want) in &outs {
        assert_eq!(&repo.read_content(id).unwrap(), want);
    }
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn pinned_float_nan_is_canonical() {
    let mut repo = Repo::new(MemStore::default());
    let m = module(&mut repo, "softmax");
    let a = args(&mut repo, "softmax", "special");
    let id = repo.apply_pinned(m, a).unwrap();
    let out = f64s(&repo.read_content(&id).unwrap());
    let nans: Vec<u64> = out
        .iter()
        .filter(|x| x.is_nan())
        .map(|x| x.to_bits())
        .collect();
    assert!(!nans.is_empty());
    assert!(
        nans.iter().all(|b| *b == 0x7ff8_0000_0000_0000),
        "{nans:x?}"
    );
}

fn within_check(repo: &mut Repo<MemStore>, spec: &str) -> Check {
    let spec = repo.put_content(spec.as_bytes()).unwrap().id;
    Check {
        func: func::within_f64(),
        args: vec![Arg::Whole(spec)],
    }
}

#[test]
fn recorded_steps_keep_bytes_and_rerun_as_checks() {
    for design in [Design::Run, Design::Wrapped] {
        let mut repo = Repo::new(MemStore::default());
        let m = module(&mut repo, "softmax");
        let a = args(&mut repo, "softmax", "wide");
        let check = within_check(&mut repo, "ulp=4");
        let (out, rec) = repo
            .record_step(m, a.clone(), None, Some(check), design)
            .unwrap();
        let first = repo.read_content(&out).unwrap();
        commit(&mut repo, &[out]);
        repo.gc().unwrap();
        repo.repack().unwrap();
        assert!(
            repo.store().has(&out),
            "{design:?}: recorded output kept as bytes"
        );
        assert!(repo.fsck().unwrap().is_empty() && repo.fsck_deep().unwrap().is_empty());
        assert_eq!(repo.step_records(&out).unwrap().len(), 1, "{design:?}");
        // A rerun in the kernel gives the same bits.
        let r = repo.rerun(&rec, None).unwrap();
        assert!(r.same && r.verdict.is_none(), "{design:?}");
        // A rerun elsewhere (host arithmetic) differs, and the check judges it.
        let x = f64s(&repo.read_content(&a[0].id()).unwrap());
        let host = bytes(&native("softmax", &x, &[], Arith::Host));
        assert_ne!(host, first);
        let r = repo.rerun(&rec, Some(&host)).unwrap();
        assert!(!r.same);
        let verdict = repo.read_content(&r.verdict.unwrap()).unwrap();
        assert!(verdict.starts_with(b"within") || verdict.starts_with(b"outside"));
        assert_eq!(
            repo.read_content(&out).unwrap(),
            first,
            "the first output stays"
        );
    }
}

#[test]
fn a_random_step_reruns_as_different_and_reads_back_its_first_output() {
    let mut repo = Repo::new(MemStore::default());
    let cmd = repo.put_content(b"cmd: draw 8 values").unwrap().id;
    let (out, rec) = repo
        .record_step(cmd, vec![], Some(&bytes(&[0.1; 8])), None, Design::Run)
        .unwrap();
    let r = repo.rerun(&rec, Some(&bytes(&[0.2; 8]))).unwrap();
    assert!(!r.same && r.verdict.is_none());
    assert_eq!(repo.read_content(&out).unwrap(), bytes(&[0.1; 8]));
    // A command cannot be rerun in the kernel.
    assert!(repo.rerun(&rec, None).is_err());
}

#[test]
fn within_bounds_ulp_abs_and_rel() {
    let mut repo = Repo::new(MemStore::default());
    let x = 1.0f64;
    let y = f64::from_bits(x.to_bits() + 3);
    let first = repo.put_content(&bytes(&[x, 2.0])).unwrap().id;
    let now = repo.put_content(&bytes(&[y, 2.0])).unwrap().id;
    for (spec, word) in [
        ("ulp=3", "within"),
        ("ulp=2", "outside"),
        ("abs=1e-15", "within"),
        ("rel=1e-16", "outside"),
        ("", "outside"),
    ] {
        let s = repo.put_content(spec.as_bytes()).unwrap().id;
        let v = repo
            .apply(
                func::within_f64(),
                vec![Arg::Whole(first), Arg::Whole(now), Arg::Whole(s)],
            )
            .unwrap();
        let text = String::from_utf8(repo.read_content(&v).unwrap()).unwrap();
        assert!(text.starts_with(word), "{spec}: {text}");
        assert!(text.contains("ulp=3"), "{text}");
    }
}

#[test]
fn gc_drops_runs_of_dead_outputs_with_their_inputs() {
    let mut repo = Repo::new(MemStore::default());
    let m = module(&mut repo, "dot");
    let live = args(&mut repo, "dot", "plain");
    let dead = args(&mut repo, "dot", "cancel");
    let (lo, lr) = repo
        .record_step(m, live.clone(), None, None, Design::Run)
        .unwrap();
    let (_, dr) = repo
        .record_step(m, dead.clone(), None, None, Design::Run)
        .unwrap();
    commit(&mut repo, &[lo]);
    repo.gc().unwrap();
    assert!(repo.get(&lr).is_ok() && repo.read_content(&live[0].id()).is_ok());
    assert!(repo.get(&dr).is_err() && repo.read_content(&dead[0].id()).is_err());
    assert!(repo.rerun(&lr, None).unwrap().same);
}

const LOOP: &str = "\
net loop 1
place cpu 1
place queued colored
place running colored
place measured colored
place waiting colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
transition ask pure: measured -> waiting
transition merge human: waiting ->
";

#[test]
fn a_net_fires_through_steps_and_verifies_them() {
    use ikam_kernel::net::{Net, Step};
    let mut repo = Repo::new(MemStore::default());
    let net = Net::parse(LOOP).unwrap();
    let empty = repo.db_empty().unwrap();
    let mut db = repo.net_init(&empty, &net).unwrap();
    let agent = repo.put_content(b"agent: propose").unwrap().id;
    let bench = repo.put_content(b"cmd: bench").unwrap().id;
    let mean_var = module(&mut repo, "mean_var");
    for i in 0..3 {
        let tok = format!("e{i}");
        let idea = format!("idea {i}");
        let (d, _) = repo
            .net_fire_step(
                &db,
                &net,
                "propose",
                Some(&tok),
                &format!("p{i}"),
                Step::Recorded {
                    func: agent,
                    args: vec![],
                    output: Some(idea.as_bytes()),
                    check: None,
                },
            )
            .unwrap();
        let times = bytes(&[1.0 + i as f64, 1.5, 0.75 * i as f64]);
        let (d, _) = repo
            .net_fire_step(
                &d,
                &net,
                "start",
                Some(&tok),
                &format!("s{i}"),
                Step::Recorded {
                    func: bench,
                    args: vec![],
                    output: Some(&times),
                    check: None,
                },
            )
            .unwrap();
        let run = repo.put_content(&times).unwrap().id;
        let step = || Step::Pure {
            func: func::pinned_float(),
            args: vec![Arg::Whole(mean_var), Arg::Whole(run)],
        };
        let (d, f) = repo
            .net_fire_step(&d, &net, "finish", Some(&tok), &format!("f{i}"), step())
            .unwrap();
        // A retry with the same key and step changes nothing.
        let (d2, f2) = repo
            .net_fire_step(&d, &net, "finish", Some(&tok), &format!("f{i}"), step())
            .unwrap();
        assert_eq!((d, &f), (d2, &f2));
        assert!(matches!(
            repo.get(&f.step.unwrap()),
            Ok(Object::Derivation(_))
        ));
        // A human transition runs no step; a step of the wrong kind is refused.
        assert!(
            repo.net_fire_step(
                &d,
                &net,
                "ask",
                Some(&tok),
                &format!("a{i}"),
                Step::Recorded {
                    func: bench,
                    args: vec![],
                    output: Some(b"x"),
                    check: None,
                }
            )
            .is_err()
        );
        db = d;
    }
    assert_eq!(
        repo.net_replay(&db, &net).unwrap(),
        repo.net_marking(&db, "loop").unwrap()
    );
    let checks = repo.net_verify_steps(&db, "loop").unwrap();
    assert_eq!(checks.len(), 9);
    assert!(checks.iter().all(|(_, ok)| *ok), "{checks:?}");
    assert!(repo.fsck().unwrap().is_empty());
}
