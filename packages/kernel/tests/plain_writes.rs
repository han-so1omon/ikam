//! `Repo::plain_hot_writes` (E261): off by default; with it on, a content
//! that today is stored as a slice of another is stored whole, reads back
//! the same, and still reads the same after a cold pass and a full repack.

use ikam_kernel::{MemStore, Repo, Store};

fn text(i: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for line in 0..400 {
        v.extend_from_slice(
            format!("line {line}: the same long sentence repeated here\n").as_bytes(),
        );
    }
    v.extend_from_slice(format!("edit {i}\n").as_bytes());
    v
}

fn stored(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn session(plain: bool) -> (Repo<MemStore>, Vec<ikam_kernel::Id>) {
    let mut repo = Repo::new(MemStore::default());
    repo.plain_hot_writes = plain;
    let ids = (0..20)
        .map(|i| repo.put_content(&text(i)).unwrap().id)
        .collect();
    (repo, ids)
}

#[test]
fn off_by_default() {
    assert!(!Repo::new(MemStore::default()).plain_hot_writes);
}

#[test]
fn plain_writes_skip_slices_and_read_back() {
    let (planned, _) = session(false);
    let (mut plain, ids) = session(true);
    assert!(
        stored(&plain) > stored(&planned),
        "plain writes store each edit on its own"
    );
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(plain.read_content(id).unwrap(), text(i));
    }
    let tree = plain
        .edit_tree(
            None,
            &ids.iter()
                .enumerate()
                .map(|(i, id)| ikam_kernel::Edit::Put {
                    path: format!("f{i:02}.txt"),
                    id: *id,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    plain.commit("main", tree, "files").unwrap();
    plain.repack_incremental().unwrap();
    plain.gc().unwrap();
    assert!(plain.fsck().unwrap().is_empty());
    plain.repack().unwrap();
    plain.gc().unwrap();
    assert!(plain.fsck().unwrap().is_empty());
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(plain.read_content(id).unwrap(), text(i));
    }
}
