//! Template (anti-unification) dedup, end to end.

use ikam_kernel::{Form, Id, Kind, MemStore, Object, Repo, Store, TreeEntry, func};

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

/// Ingest is greedy, so pair templates may overfit; repack induces one
/// template generalized over the whole cluster.
#[test]
fn similar_documents_share_one_template_after_repack() {
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

    let templates: Vec<Id> = ids
        .iter()
        .map(|id| {
            let d = repo
                .derivations(id)
                .unwrap()
                .into_iter()
                .find(|d| d.func == func::fill())
                .expect("fill");
            d.args[0].id()
        })
        .collect();
    assert!(
        templates.iter().all(|t| *t == templates[0]),
        "one shared template"
    );
    assert!(
        ids.iter().all(|id| !repo.store().has(id)),
        "no document keeps its bytes"
    );

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
    // A new document from the same generator fits the induced template.
    let put = repo.put_content(&invoice(99)).unwrap();
    let d = &repo.derivations(&put.id).unwrap()[0];
    assert_eq!(d.func, func::fill(), "new document reuses the template");
}

fn pdf_fixtures() -> Vec<Vec<u8>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/cases");
    let mut out = Vec::new();
    for case in std::fs::read_dir(root).unwrap() {
        let case = case.unwrap().path();
        if !case.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&case).unwrap() {
            let f = f.unwrap().path();
            if f.extension().is_some_and(|e| e == "pdf") {
                out.push(std::fs::read(f).unwrap());
            }
        }
    }
    out.sort();
    out
}

/// Templates and fillers are content too, so repack templates them again:
/// on the pdf fixtures some level-1 template or filler list is itself a
/// `fill` of a level-2 template (measured: -1.9% vs one level).
#[test]
fn templates_layer_on_real_documents() {
    let docs = pdf_fixtures();
    assert!(docs.len() >= 10, "pdf fixtures missing");
    let mut repo = Repo::new(MemStore::default());
    let ids = commit_all(&mut repo, &docs);
    let r = repo.repack().unwrap();
    assert!(r.applied, "{r:?}");
    for (id, d) in ids.iter().zip(&docs) {
        assert_eq!(&repo.read_content(id).unwrap(), d);
    }
    assert!(repo.fsck().unwrap().is_empty());

    let fill_of = |id: &Id| {
        repo.derivations(id)
            .unwrap()
            .into_iter()
            .find(|d| d.func == func::fill())
    };
    let level1: Vec<Id> = ids
        .iter()
        .filter_map(fill_of)
        .flat_map(|d| d.args.into_iter().map(|a| a.id()))
        .collect();
    assert!(!level1.is_empty(), "documents use templates");
    assert!(
        level1.iter().any(|part| fill_of(part).is_some()),
        "some template or fillers are themselves templated"
    );
}
