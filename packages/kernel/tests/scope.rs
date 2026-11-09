//! E131: `seed_scope` bounds where new content looks for repeats.

use ikam_kernel::{Arg, Id, MemStore, Repo, Store};

fn noise(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 56) as u8
        })
        .collect()
}

fn edited(base: &[u8]) -> Vec<u8> {
    let mut v = base.to_vec();
    v.splice(2000..2000, b"an edit in the middle".iter().copied());
    v
}

/// Ids the derivations of `id` read.
fn sources(repo: &Repo<MemStore>, id: &Id) -> Vec<Id> {
    repo.derivations(id)
        .unwrap()
        .iter()
        .flat_map(|d| d.args.iter().map(Arg::id).collect::<Vec<_>>())
        .collect()
}

fn store_two(scope: impl Fn(Id, Id) -> Option<Vec<Id>>) -> (Repo<MemStore>, Id, Id) {
    let mut first = Repo::new(MemStore::default());
    let a = first.put_content(&noise(1, 8000)).unwrap().id;
    let b = first.put_content(&noise(2, 8000)).unwrap().id;
    // A new handle on the same objects, as a new process would open it.
    let mut store = MemStore::default();
    for id in first.store().ids().unwrap() {
        store.write(id, &first.store().read(&id).unwrap()).unwrap();
    }
    let mut repo = Repo::new(store);
    repo.seed_scope = scope(a, b);
    (repo, a, b)
}

#[test]
fn whole_store_scope_finds_repeats_anywhere() {
    let (mut repo, a, b) = store_two(|_, _| None);
    let ea = repo.put_content(&edited(&noise(1, 8000))).unwrap().id;
    let eb = repo.put_content(&edited(&noise(2, 8000))).unwrap().id;
    assert!(sources(&repo, &ea).contains(&a));
    assert!(sources(&repo, &eb).contains(&b));
}

#[test]
fn scoped_index_uses_only_content_in_scope() {
    let (mut repo, a, b) = store_two(|a, _| Some(vec![a]));
    let ea = repo.put_content(&edited(&noise(1, 8000))).unwrap().id;
    let eb = repo.put_content(&edited(&noise(2, 8000))).unwrap().id;
    assert!(sources(&repo, &ea).contains(&a));
    assert!(!sources(&repo, &eb).contains(&b));
    assert_eq!(repo.read_content(&ea).unwrap(), edited(&noise(1, 8000)));
    assert_eq!(repo.read_content(&eb).unwrap(), edited(&noise(2, 8000)));
    // Content written through the handle joins the index.
    let mut again = edited(&noise(2, 8000));
    again.extend_from_slice(b" and one more line");
    let e2 = repo.put_content(&again).unwrap().id;
    assert!(sources(&repo, &e2).contains(&eb));
    assert!(repo.fsck().unwrap().is_empty());
}

#[test]
fn an_empty_scope_stores_plain_bytes() {
    let (mut repo, a, _) = store_two(|_, _| Some(vec![]));
    let ea = repo.put_content(&edited(&noise(1, 8000))).unwrap().id;
    assert!(!sources(&repo, &ea).contains(&a));
    assert_eq!(repo.read_content(&ea).unwrap(), edited(&noise(1, 8000)));
}

#[test]
fn set_seed_scope_rebuilds_the_index() {
    let (mut repo, a, b) = store_two(|a, _| Some(vec![a]));
    let ea = repo.put_content(&edited(&noise(1, 8000))).unwrap().id;
    assert!(sources(&repo, &ea).contains(&a));
    repo.set_seed_scope(Some(vec![b]));
    let eb = repo.put_content(&edited(&noise(2, 8000))).unwrap().id;
    assert!(sources(&repo, &eb).contains(&b));
    assert!(repo.fsck().unwrap().is_empty());
}

#[test]
fn a_scoped_tree_lets_its_next_version_repeat_it() {
    use ikam_kernel::Edit;
    let mut first = Repo::new(MemStore::default());
    let mut edits = Vec::new();
    for i in 0..400 {
        let id = first.put_content(&noise(i, 64)).unwrap().id;
        edits.push(Edit::Put {
            path: format!("dir/file-{i:04}.bin"),
            id,
        });
    }
    let root = first.edit_tree(None, &edits).unwrap();
    let mut store = MemStore::default();
    for id in first.store().ids().unwrap() {
        store.write(id, &first.store().read(&id).unwrap()).unwrap();
    }
    let written = |scope: Option<Vec<Id>>| {
        let mut s2 = MemStore::default();
        for id in store.ids().unwrap() {
            s2.write(id, &store.read(&id).unwrap()).unwrap();
        }
        let mut repo = Repo::new(s2);
        repo.seed_scope = scope;
        let id = repo.put_content(b"one new file").unwrap().id;
        let before = repo.bytes_written;
        repo.edit_tree(
            Some(root),
            &[Edit::Put {
                path: "dir/file-0200.bin".into(),
                id,
            }],
        )
        .unwrap();
        assert!(repo.fsck().unwrap().is_empty());
        repo.bytes_written - before
    };
    let Ok(ikam_kernel::Object::Tree(entries)) = first.get(&root) else {
        panic!()
    };
    let dir = entries[0].id;
    let with_tree = written(Some(vec![dir]));
    let without = written(Some(vec![]));
    assert!(
        with_tree < without,
        "{with_tree} bytes with the tree in scope, {without} without"
    );
}
