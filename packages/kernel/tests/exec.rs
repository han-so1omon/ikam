//! Containers, functions, runs and graph links.

use std::fs;
use std::path::{Path, PathBuf};

use ikam_kernel::{Arg, Error, Form, Id, Kind, MemStore, Object, Repo, Store, TreeEntry, func};
use proptest::prelude::*;

fn fixtures(ext: &str) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/cases");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == ext) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn zip_containers_are_stored_as_verified_derivations() {
    let files = fixtures("xlsx");
    assert!(files.len() >= 2, "xlsx fixtures missing");
    let mut repo = Repo::new(MemStore::default());
    for path in &files[..2] {
        let bytes = std::fs::read(path).unwrap();
        let put = repo.put_content(&bytes).unwrap();
        assert_eq!(put.form, Form::Derived, "{}", path.display());
        assert_eq!(repo.read_content(&put.id).unwrap(), bytes);
        // The members are first-class content, reachable as graph edges.
        let d = &repo.derivations(&put.id).unwrap()[0];
        assert_eq!(d.func, func::deflate_pack());
        assert!(d.args.len() > 2);
        for member in &d.args {
            assert!(!repo.used_by(&member.id()).unwrap().is_empty());
        }
    }
}

#[test]
fn similar_containers_share_members() {
    let files = fixtures("xlsx");
    let mut repo = Repo::new(MemStore::default());
    let mut costs = Vec::new();
    let mut input = 0;
    for path in &files[..10.min(files.len())] {
        let bytes = std::fs::read(path).unwrap();
        input += bytes.len();
        let before = repo.bytes_written;
        repo.put_content(&bytes).unwrap();
        costs.push(repo.bytes_written - before);
    }
    let total: usize = costs.iter().sum();
    assert!(total < input / 2, "stored {total} of {input} bytes");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    /// Corrupted containers (bit flips, truncation) must still round-trip:
    /// the zip planner may fail, but never lose bytes.
    #[test]
    fn damaged_containers_roundtrip(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 0..6), cut in 0.5..1.0f64) {
        let mut bytes = std::fs::read(&fixtures("xlsx")[0]).unwrap();
        for (at, v) in flips {
            let i = at.index(bytes.len());
            bytes[i] ^= v;
        }
        bytes.truncate((bytes.len() as f64 * cut) as usize);
        let mut repo = Repo::new(MemStore::default());
        let put = repo.put_content(&bytes).unwrap();
        prop_assert_eq!(repo.read_content(&put.id).unwrap(), bytes);
    }
}

/// Uppercases its single argument in place. No imports, no floats.
const UPPER: &str = r#"
(module
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 1024))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (if (i32.gt_u (i32.add (local.get $p) (local.get $n)) (i32.mul (memory.size) (i32.const 65536)))
      (then (drop (memory.grow (i32.add (i32.shr_u (local.get $n) (i32.const 16)) (i32.const 1))))))
    (global.set $heap (i32.add (local.get $p) (local.get $n)))
    (local.get $p))
  (func (export "run") (param $p i32) (param $n i32) (result i64)
    (local $i i32) (local $c i32)
    (local.set $i (i32.add (local.get $p) (i32.const 8)))
    (block $done (loop $next
      (br_if $done (i32.ge_u (local.get $i) (i32.add (local.get $p) (local.get $n))))
      (local.set $c (i32.load8_u (local.get $i)))
      (if (i32.and (i32.ge_u (local.get $c) (i32.const 97)) (i32.le_u (local.get $c) (i32.const 122)))
        (then (i32.store8 (local.get $i) (i32.sub (local.get $c) (i32.const 32)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (i64.or
      (i64.shl (i64.extend_i32_u (i32.add (local.get $p) (i32.const 8))) (i64.const 32))
      (i64.extend_i32_u (i32.sub (local.get $n) (i32.const 8))))))
"#;

fn store_module(repo: &mut Repo<MemStore>, wat: &str) -> Id {
    repo.put_content(&wat::parse_str(wat).unwrap()).unwrap().id
}

fn text(n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| b"abcdefghij klmnopqrstuvwxyz"[(i * 7 + i / 13) % 27])
        .collect()
}

#[test]
fn semantic_dedup_records_a_verified_function_application() {
    let mut repo = Repo::new(MemStore::default());
    let upper = store_module(&mut repo, UPPER);
    let x = text(20_000);
    let x_id = repo.put_content(&x).unwrap().id;
    let y = x.to_ascii_uppercase();
    let before = repo.bytes_written;
    let put = repo
        .put_derivation(&y, upper, vec![Arg::Whole(x_id)])
        .unwrap()
        .unwrap();
    assert_eq!(put.form, Form::Derived);
    assert_eq!(put.id, Id::of_content(&y));
    assert_eq!(
        repo.bytes_written - before,
        1 + 32 + 32 + 4 + 33,
        "one derivation record"
    );
    assert_eq!(repo.read_content(&put.id).unwrap(), y);

    // A proposal that does not reproduce the bytes is rejected, nothing written.
    let mut wrong = y.clone();
    wrong[100] = b'!';
    let before = repo.bytes_written;
    assert!(
        repo.put_derivation(&wrong, upper, vec![Arg::Whole(x_id)])
            .unwrap()
            .is_none()
    );
    assert_eq!(repo.bytes_written, before);
}

#[test]
fn applies_are_memoized_provenance() {
    let mut repo = Repo::new(MemStore::default());
    let upper = store_module(&mut repo, UPPER);
    let x_id = repo.put_content(b"hello graph").unwrap().id;
    let out = repo.apply(upper, vec![Arg::Whole(x_id)]).unwrap();
    assert_eq!(repo.read_content(&out).unwrap(), b"HELLO GRAPH");
    let after_first = repo.bytes_written;
    assert_eq!(repo.apply(upper, vec![Arg::Whole(x_id)]).unwrap(), out);
    assert_eq!(
        repo.bytes_written, after_first,
        "second apply must hit the memo"
    );

    let used_by = repo.used_by(&out).unwrap();
    assert_eq!(
        used_by.len(),
        1,
        "only the derivation record links to the output"
    );
    assert_eq!(used_by[0].1, "output");
    let links = repo.links(&used_by[0].0).unwrap();
    assert!(links.contains(&("func".to_string(), upper)));
    assert!(links.contains(&("arg0".to_string(), x_id)));
    assert_eq!(
        repo.links(&out).unwrap(),
        [("derivation".to_string(), used_by[0].0)]
    );
}

#[test]
fn repack_drops_bytes_a_derivation_rebuilds() {
    let mut repo = Repo::new(MemStore::default());
    let upper = store_module(&mut repo, UPPER);
    let x_id = repo.put_content(&text(20_000)).unwrap().id;
    let y_id = repo.apply(upper, vec![Arg::Whole(x_id)]).unwrap();
    assert!(repo.store().has(&y_id), "apply stores its output as bytes");
    let tree = repo
        .put(
            &Object::tree(vec![TreeEntry {
                name: "y".into(),
                kind: Kind::File,
                id: y_id,
            }])
            .unwrap(),
        )
        .unwrap();
    repo.commit("main", tree, "").unwrap();
    let r = repo.repack().unwrap();
    assert!(r.applied && r.after < r.before, "{r:?}");
    assert!(!repo.store().has(&y_id), "y is now rebuilt from x");
    assert_eq!(
        repo.read_content(&y_id).unwrap(),
        text(20_000).to_ascii_uppercase()
    );
    assert!(repo.fsck().unwrap().is_empty());
}

#[test]
fn gc_keeps_provenance_of_live_outputs_only() {
    let mut repo = Repo::new(MemStore::default());
    let upper = store_module(&mut repo, UPPER);
    let kept_in = repo.put_content(b"kept").unwrap().id;
    let dropped_in = repo.put_content(b"dropped").unwrap().id;
    let kept = repo.apply(upper, vec![Arg::Whole(kept_in)]).unwrap();
    let dropped = repo.apply(upper, vec![Arg::Whole(dropped_in)]).unwrap();
    let tree = repo
        .put(
            &Object::tree(vec![TreeEntry {
                name: "k".into(),
                kind: Kind::File,
                id: kept,
            }])
            .unwrap(),
        )
        .unwrap();
    repo.commit("main", tree, "").unwrap();
    repo.gc().unwrap();

    let (record, _) = repo
        .used_by(&kept)
        .unwrap()
        .into_iter()
        .find(|(_, label)| label == "output")
        .unwrap();
    assert!(matches!(repo.get(&record), Ok(Object::Derivation(_))));
    assert_eq!(
        repo.read_content(&upper).unwrap(),
        wat::parse_str(UPPER).unwrap(),
        "function kept as provenance"
    );
    assert_eq!(repo.read_content(&kept_in).unwrap(), b"kept");
    assert!(matches!(
        repo.read_content(&dropped),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        repo.read_content(&dropped_in),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn sandbox_rejects_impure_or_unbounded_modules() {
    let mut repo = Repo::new(MemStore::default());
    let x = repo.put_content(b"x").unwrap().id;
    let abi = r#"(memory (export "memory") 1) (func (export "alloc") (param i32) (result i32) (i32.const 0))"#;
    let cases = [
        format!(
            r#"(module (import "env" "clock" (func)) {abi} (func (export "run") (param i32 i32) (result i64) (i64.const 0)))"#
        ),
        format!(
            r#"(module {abi} (func (export "run") (param i32 i32) (result i64) (drop (f32.const 1)) (i64.const 0)))"#
        ),
        format!(
            r#"(module {abi} (func (export "run") (param i32 i32) (result i64) (loop (br 0)) (i64.const 0)))"#
        ),
    ];
    for wat in &cases {
        let f = store_module(&mut repo, wat);
        assert!(
            matches!(repo.apply(f, vec![Arg::Whole(x)]), Err(Error::Exec(_))),
            "{wat}"
        );
    }
}

/// The shared interpreter (`functions/interp.wasm`) passes the sandbox and
/// runs a 50-byte program into a 49 KB multiplication table exactly; its
/// decode work counts the WASM fuel, not only the bytes produced.
#[test]
fn interpreter_programs_reconstruct_exactly_and_price_their_fuel() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("functions");
    let mut repo = Repo::new(MemStore::default());
    let interp = repo
        .put_content(&fs::read(dir.join("interp.wasm")).unwrap())
        .unwrap()
        .id;
    let program = fs::read(dir.join("programs/multiplication.bin")).unwrap();
    let program = repo.put_content(&program).unwrap().id;
    let table: String = (1..=99u32)
        .map(|a| {
            (1..=99u32)
                .map(|b| format!("{:>5}", a * b))
                .collect::<String>()
                + "\n"
        })
        .collect();
    let (out, work) = repo.evaluate(&interp, &[Arg::Whole(program)]).unwrap();
    assert_eq!(out, table.as_bytes());
    assert!(work > 2 * out.len(), "fuel is priced: work {work}");
    let put = repo.put_derivation(table.as_bytes(), interp, vec![Arg::Whole(program)]);
    assert!(put.unwrap().is_some(), "verified derivation");
}
