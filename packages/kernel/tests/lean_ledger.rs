//! E391: a lean ledger (record ids only) reads, rebuilds, repacks and
//! refreshes every derived content the same as the full ledger.

use ikam_kernel::{Edit, Form, FsStore, Id, Repo};

fn blob(seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..4096)
        .map(|_| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
        })
        .collect()
}

fn edit(base: u64, k: usize) -> Vec<u8> {
    let mut b = blob(base);
    b[64 + k] ^= 0x5A;
    b[2048 + 7 * k] ^= 0xA5;
    b.extend_from_slice(format!(" edit {base} {k}").as_bytes());
    b
}

fn open(dir: &std::path::Path, lean: bool) -> Repo<FsStore> {
    let mut r = Repo::new(FsStore::open(dir).unwrap());
    r.lean_ledger = lean;
    r
}

/// Puts 6 bases and 40 near-copies; returns every content's bytes and id.
fn fill(r: &mut Repo<FsStore>) -> Vec<(Vec<u8>, Id)> {
    let mut out = Vec::new();
    for i in 0..6 {
        let b = blob(i);
        out.push((b.clone(), r.put_content(&b).unwrap().id));
    }
    let mut derived = 0;
    for k in 0..40 {
        let b = edit(k as u64 % 6, k);
        let put = r.put_content(&b).unwrap();
        derived += (put.form == Form::Derived) as usize;
        out.push((b, put.id));
    }
    assert!(derived > 20, "only {derived} near-copies were derived");
    out
}

#[test]
fn lean_reads_and_derivations_match_full() {
    let dir = tempfile::tempdir().unwrap();
    let all = fill(&mut open(dir.path(), true));
    let lean = open(dir.path(), true);
    let full = open(dir.path(), false);
    for (bytes, id) in &all {
        assert_eq!(&lean.read_content(id).unwrap(), bytes);
        assert!(lean.has_content(id).unwrap());
        assert_eq!(lean.derivations(id).unwrap(), full.derivations(id).unwrap());
    }
    let missing = Id::of_content(b"never stored");
    assert!(!lean.has_content(&missing).unwrap());
    assert!(lean.fsck().unwrap().is_empty());
}

#[test]
fn lean_survives_repack_and_gc() {
    repack_case(true);
}

#[test]
fn full_survives_repack() {
    repack_case(false);
}

fn repack_case(lean: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut r = open(dir.path(), lean);
    let all = fill(&mut r);
    // Live content: a commit of every content, so repack keeps it.
    let edits: Vec<Edit> = all
        .iter()
        .enumerate()
        .map(|(i, (_, id))| Edit::Put {
            path: format!("f{i:02}"),
            id: *id,
        })
        .collect();
    let tree = r.edit_tree(None, &edits).unwrap();
    r.commit("main", tree, "all").unwrap();
    r.repack().unwrap();
    r.gc().unwrap();
    for (bytes, id) in &all {
        assert_eq!(&r.read_content(id).unwrap(), bytes);
    }
    let fresh = open(dir.path(), lean);
    for (bytes, id) in &all {
        assert_eq!(&fresh.read_content(id).unwrap(), bytes);
    }
    assert!(fresh.fsck().unwrap().is_empty());
}

#[test]
fn lean_refresh_reads_another_handles_derivation() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = open(dir.path(), true);
    r.refresh_ledger = true;
    let base = r.put_content(&blob(1)).unwrap().id;
    assert_eq!(r.read_content(&base).unwrap(), blob(1));
    let mut other = Repo::new(FsStore::open(dir.path()).unwrap());
    let bytes = edit(1, 3);
    let put = other.put_content(&bytes).unwrap();
    assert_eq!(put.form, Form::Derived);
    assert_eq!(r.read_content(&put.id).unwrap(), bytes);
    assert!(r.has_content(&put.id).unwrap());
}
