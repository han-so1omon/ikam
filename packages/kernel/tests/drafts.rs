//! Laws of drafts as branches (E266): a draft is invisible on its target
//! until promoted; promoting fast-forwards or merges and deletes the draft;
//! a conflict changes nothing; a discarded draft leaves nothing after gc.

use std::collections::BTreeSet;

use ikam_kernel::merge::{Conflict, Merged};
use ikam_kernel::{Edit, FsStore, Id, MemStore, Object, Repo, Store};

fn tree_of<S: Store>(repo: &Repo<S>, commit: Id) -> Id {
    match repo.get(&commit).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    }
}

fn put<S: Store>(repo: &mut Repo<S>, name: &str, edits: &[(&str, &str)]) -> Id {
    let root = repo.resolve(name).ok().map(|c| tree_of(repo, c));
    let edits: Vec<Edit> = edits
        .iter()
        .map(|(p, v)| Edit::Put {
            path: p.to_string(),
            id: repo.put_content(v.as_bytes()).unwrap().id,
        })
        .collect();
    let tree = repo.edit_tree(root, &edits).unwrap();
    repo.commit(name, tree, "edit").unwrap()
}

fn read<S: Store>(repo: &Repo<S>, rev: &str, path: &str) -> Option<Vec<u8>> {
    let mut id = tree_of(repo, repo.resolve(rev).unwrap());
    for part in path.split('/') {
        let Object::Tree(entries) = repo.get(&id).unwrap() else {
            return None;
        };
        id = entries.into_iter().find(|e| e.name == part)?.id;
    }
    repo.read_content(&id).ok()
}

fn base<S: Store>(store: S) -> Repo<S> {
    let mut repo = Repo::new(store);
    put(&mut repo, "main", &[("a.txt", "a0"), ("d/b.txt", "b0")]);
    repo
}

#[test]
fn a_draft_is_invisible_until_promoted() {
    let mut repo = base(MemStore::default());
    let main = repo.resolve("main").unwrap();
    let d = repo.draft("main", "notes").unwrap();
    assert_eq!(d, "drafts/notes");
    put(&mut repo, &d, &[("a.txt", "a1"), ("c.txt", "c1")]);
    assert_eq!(repo.resolve("main").unwrap(), main, "main did not move");
    assert_eq!(read(&repo, "main", "a.txt").unwrap(), b"a0");
    assert!(read(&repo, "main", "c.txt").is_none());
    let head = repo.resolve(&d).unwrap();
    assert_eq!(repo.drafts().unwrap(), vec![(d.clone(), head)]);
    // The target has not moved: a fast-forward, and the draft is gone.
    assert_eq!(
        repo.promote_draft("main", &d, "promote").unwrap(),
        Merged::FastForward(head)
    );
    assert_eq!(read(&repo, "main", "c.txt").unwrap(), b"c1");
    assert!(repo.drafts().unwrap().is_empty());
    assert!(repo.discard_draft(&d).is_err(), "already gone");
}

#[test]
fn a_second_draft_merges_and_a_conflict_changes_nothing() {
    let mut repo = base(MemStore::default());
    let one = repo.draft("main", "one").unwrap();
    let two = repo.draft("main", "two").unwrap();
    let three = repo.draft("main", "three").unwrap();
    put(&mut repo, &one, &[("a.txt", "from one")]);
    put(&mut repo, &two, &[("d/b.txt", "from two")]);
    put(&mut repo, &three, &[("a.txt", "from three")]);
    assert!(matches!(
        repo.promote_draft("main", &one, "one").unwrap(),
        Merged::FastForward(_)
    ));
    assert!(matches!(
        repo.promote_draft("main", &two, "two").unwrap(),
        Merged::Commit(_)
    ));
    assert_eq!(read(&repo, "main", "a.txt").unwrap(), b"from one");
    assert_eq!(read(&repo, "main", "d/b.txt").unwrap(), b"from two");
    let main = repo.resolve("main").unwrap();
    let head = repo.resolve(&three).unwrap();
    let Merged::Conflicts(c) = repo.promote_draft("main", &three, "three").unwrap() else {
        panic!("expected a conflict");
    };
    assert_eq!(c, vec![Conflict::Path("a.txt".into())]);
    assert_eq!(repo.resolve("main").unwrap(), main, "main did not move");
    assert_eq!(repo.resolve(&three).unwrap(), head, "draft kept");
    assert_eq!(repo.drafts().unwrap().len(), 1);
}

#[test]
fn a_discarded_draft_leaves_nothing_after_gc() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = base(FsStore::open(dir.path()).unwrap());
    let before: BTreeSet<Id> = repo.store().ids().unwrap().into_iter().collect();
    for i in 0..5 {
        let d = repo.draft("main", &format!("try-{i}")).unwrap();
        for j in 0..3 {
            put(
                &mut repo,
                &d,
                &[(&format!("d/n{j}.md"), &format!("draft {i} edit {j}"))],
            );
        }
        repo.discard_draft(&d).unwrap();
    }
    assert!(repo.store().ids().unwrap().len() > before.len());
    repo.gc().unwrap();
    let after: BTreeSet<Id> = repo.store().ids().unwrap().into_iter().collect();
    // Induction may re-store a live file as a derivation from a template
    // the drafts' writes taught it, so the ids can differ; no draft's
    // commit or tree may survive, and main reads the same.
    for id in after.difference(&before) {
        assert!(!matches!(
            repo.get(id),
            Ok(Object::Commit(_) | Object::Tree(_))
        ));
    }
    assert_eq!(read(&repo, "main", "a.txt").unwrap(), b"a0");
    assert_eq!(read(&repo, "main", "d/b.txt").unwrap(), b"b0");
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.drafts().unwrap().is_empty());
}

#[test]
fn promoting_again_after_an_interrupted_promote_changes_nothing() {
    let mut repo = base(MemStore::default());
    let d = repo.draft("main", "half").unwrap();
    let head = put(&mut repo, &d, &[("a.txt", "a1")]);
    // As if a promote moved main and stopped before deleting the draft.
    repo.merge("main", &d, "promote").unwrap();
    assert_eq!(repo.resolve(&d).unwrap(), head);
    assert_eq!(
        repo.promote_draft("main", &d, "again").unwrap(),
        Merged::UpToDate
    );
    assert!(repo.drafts().unwrap().is_empty());
}

#[test]
fn deleting_a_ref_is_compare_and_swap() {
    let dir = tempfile::tempdir().unwrap();
    let mut fs = FsStore::open(dir.path()).unwrap();
    let mut mem = MemStore::default();
    let stores: [&mut dyn Store; 2] = [&mut fs, &mut mem];
    let (a, b) = (Id::of_content(b"a"), Id::of_content(b"b"));
    for s in stores {
        s.set_ref("drafts/x", None, a).unwrap();
        assert!(s.delete_ref("drafts/x", b).is_err(), "stale expectation");
        assert_eq!(s.get_ref("drafts/x").unwrap(), Some(a));
        s.delete_ref("drafts/x", a).unwrap();
        assert_eq!(s.get_ref("drafts/x").unwrap(), None);
        assert!(s.refs().unwrap().is_empty());
        assert!(s.delete_ref("drafts/x", a).is_err(), "already gone");
        // The name can be used again.
        s.set_ref("drafts/x", None, b).unwrap();
    }
}

#[test]
fn only_drafts_can_be_promoted_or_discarded() {
    let mut repo = base(MemStore::default());
    assert!(repo.discard_draft("main").is_err());
    assert!(repo.promote_draft("main", "main", "m").is_err());
    assert!(repo.draft("main", "x").is_ok());
    assert!(repo.draft("main", "x").is_err(), "already open");
}
