//! Template (anti-unification) dedup, end to end.

use ikam_kernel::{Form, Id, Kind, MemStore, Object, Repo, Store, TreeEntry};

/// An invoice from one generator: long shared boilerplate, short fields.
fn invoice(n: u32) -> Vec<u8> {
    let field = |k: u32| {
        format!(
            "{:x}",
            (n.wrapping_mul(2654435761) ^ k.wrapping_mul(40503)) % 100_000
        )
    };
    let mut out = String::new();
    for k in 0..20 {
        out.push_str(&format!(
            "<section id=\"{k}\"><label>Line item {k}: standard terms and conditions apply; see master agreement clause {k}.</label><value>{}</value></section>\n",
            field(k)
        ));
    }
    out.into_bytes()
}

fn stored_bytes(repo: &Repo<MemStore>) -> usize {
    repo.store()
        .ids()
        .unwrap()
        .iter()
        .map(|id| repo.store().read(id).unwrap().len())
        .sum()
}

fn commit_all(repo: &mut Repo<MemStore>, docs: &[Vec<u8>]) -> Vec<Id> {
    let ids: Vec<Id> = docs
        .iter()
        .map(|d| repo.put_content(d).unwrap().id)
        .collect();
    let entries = ids
        .iter()
        .enumerate()
        .map(|(i, id)| TreeEntry {
            name: format!("inv{i:03}"),
            kind: Kind::File,
            id: *id,
        })
        .collect();
    let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
    ids
}

/// Repack improves on greedy ingest for similar documents, exactly. (That
/// induction yields one template for the whole cluster is a unit test in
/// `repack.rs`; whether repack keeps templates or a dictionary is the
/// evaluator's call.)
#[test]
fn similar_documents_shrink_after_repack() {
    let mut repo = Repo::new(MemStore::default());
    let docs: Vec<Vec<u8>> = (0..30).map(invoice).collect();
    let ids = commit_all(&mut repo, &docs);
    let ingested = stored_bytes(&repo);
    let r = repo.repack().unwrap();
    assert!(r.applied, "{r:?}");
    for (id, d) in ids.iter().zip(&docs) {
        assert_eq!(&repo.read_content(id).unwrap(), d);
    }
    assert!(repo.fsck().unwrap().is_empty());

    let input: usize = docs.iter().map(Vec::len).sum();
    let stored = stored_bytes(&repo);
    println!("input={input} after_ingest={ingested} after_repack={stored}");
    assert!(stored < ingested, "repack must improve on greedy ingest");
}

#[test]
fn templates_survive_gc_and_repack() {
    let mut repo = Repo::new(MemStore::default());
    let docs: Vec<Vec<u8>> = (0..10).map(invoice).collect();
    commit_all(&mut repo, &docs);
    repo.gc().unwrap();
    repo.repack().unwrap();
    repo.repack().unwrap();
    for d in &docs {
        assert_eq!(&repo.read_content(&Id::of_content(d)).unwrap(), d);
    }
    assert!(repo.fsck().unwrap().is_empty());
    assert_eq!(repo.put_content(&docs[3]).unwrap().form, Form::Existing);
    // A new document from the same generator is cheap to add, whichever plan
    // wins (a fill of the template, or bytes against the trained dictionary).
    let before = repo.bytes_written;
    let fresh = invoice(99);
    let put = repo.put_content(&fresh).unwrap();
    assert_eq!(repo.read_content(&put.id).unwrap(), fresh);
    let cost = repo.bytes_written - before;
    assert!(
        cost * 5 < fresh.len(),
        "new document cost {cost} of {} bytes",
        fresh.len()
    );
}

/// 21 revisions of a spec (as in examples/bench.rs): one appended phrase per
/// revision, a new section every fifth.
fn revisions() -> Vec<Vec<u8>> {
    let spec = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/ikam/ikam-sheet-specification.md");
    let mut versions = vec![std::fs::read_to_string(spec).unwrap()];
    for i in 1..=20usize {
        let mut lines: Vec<String> = versions.last().unwrap().lines().map(String::from).collect();
        let k = (i * 37) % lines.len();
        lines[k].push_str(&format!(" (edit {i})"));
        let mut v = lines.join("\n") + "\n";
        if i % 5 == 0 {
            v.push_str(&format!(
                "\n## Added section {i}\nNew text for revision {i}.\n"
            ));
        }
        versions.push(v);
    }
    versions.into_iter().map(String::into_bytes).collect()
}

/// A revision history repacks into little more than one revision. (That
/// induction layers templates without a level limit is a unit test in
/// `repack.rs`; which plan repack keeps is the evaluator's call.)
#[test]
fn revisions_repack_into_little_more_than_one() {
    let docs = revisions();
    let mut repo = Repo::new(MemStore::default());
    let ids = commit_all(&mut repo, &docs);
    let r = repo.repack().unwrap();
    assert!(r.applied, "{r:?}");
    for (id, d) in ids.iter().zip(&docs) {
        assert_eq!(&repo.read_content(id).unwrap(), d);
    }
    assert!(repo.fsck().unwrap().is_empty());

    // Whichever plan wins (templates, or a group compressing revisions
    // against each other), the history costs under two revisions.
    let one = zstd::bulk::compress(&docs[0], 19).unwrap().len();
    let stored = stored_bytes(&repo);
    assert!(
        stored < 2 * one,
        "21 revisions stored in {stored} B; one is {one} B at zstd -19"
    );
}
