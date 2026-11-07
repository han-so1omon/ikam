//! Laws of path edits on trees (`worktree.rs`, E110), against a model.

use std::collections::BTreeMap;

use ikam_kernel::{Edit, Id, Kind, MemStore, Object, Repo, Store};
use proptest::prelude::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    File(Vec<u8>),
    Dir(BTreeMap<String, M>),
}

fn parts(path: &str) -> Vec<String> {
    path.split('/').map(String::from).collect()
}

/// The directory holding the last part, creating directories (and
/// replacing files in the way) when `create`.
fn model_dir<'a>(
    mut dir: &'a mut BTreeMap<String, M>,
    p: &[String],
    create: bool,
) -> Option<&'a mut BTreeMap<String, M>> {
    for name in &p[..p.len() - 1] {
        match dir.get(name) {
            Some(M::Dir(_)) => {}
            _ if create => {
                dir.insert(name.clone(), M::Dir(BTreeMap::new()));
            }
            _ => return None,
        }
        let Some(M::Dir(next)) = dir.get_mut(name) else {
            unreachable!()
        };
        dir = next;
    }
    Some(dir)
}

/// Apply one edit to the model; `false` if the kernel must refuse it.
fn model_apply(root: &mut BTreeMap<String, M>, e: &Op) -> bool {
    match e {
        Op::Put(path, bytes) => {
            let p = parts(path);
            let dir = model_dir(root, &p, true).unwrap();
            dir.insert(p.last().unwrap().clone(), M::File(bytes.clone()));
            true
        }
        Op::Mkdir(path) => {
            let p = parts(path);
            let mut work = root.clone();
            let dir = model_dir(&mut work, &p, true).unwrap();
            match dir.get(p.last().unwrap()) {
                Some(M::Dir(_)) => {}
                Some(M::File(_)) => return false,
                None => {
                    dir.insert(p.last().unwrap().clone(), M::Dir(BTreeMap::new()));
                }
            }
            *root = work;
            true
        }
        Op::Remove(path) => {
            let p = parts(path);
            if let Some(dir) = model_dir(root, &p, false) {
                dir.remove(p.last().unwrap());
            }
            true
        }
        Op::Rename(from, to) | Op::Copy(from, to) => {
            let (s, d) = (parts(from), parts(to));
            if d.starts_with(&s) && d.len() > s.len() {
                return false;
            }
            let mut work = root.clone();
            let Some(dir) = model_dir(&mut work, &s, false) else {
                return false;
            };
            let node = if matches!(e, Op::Rename(..)) {
                dir.remove(s.last().unwrap())
            } else {
                dir.get(s.last().unwrap()).cloned()
            };
            let Some(node) = node else { return false };
            let dir = model_dir(&mut work, &d, true).unwrap();
            dir.insert(d.last().unwrap().clone(), node);
            *root = work;
            true
        }
    }
}

#[derive(Clone, Debug)]
enum Op {
    Put(String, Vec<u8>),
    Mkdir(String),
    Remove(String),
    Rename(String, String),
    Copy(String, String),
}

fn any_path() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(vec!["a", "b", "c"]), 1..4).prop_map(|p| p.join("/"))
}

fn any_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (any_path(), prop::collection::vec(0u8..4, 0..6)).prop_map(|(p, b)| Op::Put(p, b)),
        1 => any_path().prop_map(Op::Mkdir),
        2 => any_path().prop_map(Op::Remove),
        2 => (any_path(), any_path()).prop_map(|(a, b)| Op::Rename(a, b)),
        1 => (any_path(), any_path()).prop_map(|(a, b)| Op::Copy(a, b)),
    ]
}

fn to_edit(repo: &mut Repo<MemStore>, op: &Op) -> Edit {
    match op {
        Op::Put(path, bytes) => Edit::Put {
            path: path.clone(),
            id: repo.put_content(bytes).unwrap().id,
        },
        Op::Mkdir(path) => Edit::Mkdir { path: path.clone() },
        Op::Remove(path) => Edit::Remove { path: path.clone() },
        Op::Rename(from, to) => Edit::Rename {
            from: from.clone(),
            to: to.clone(),
        },
        Op::Copy(from, to) => Edit::Copy {
            from: from.clone(),
            to: to.clone(),
        },
    }
}

/// The tree as the model holds it, read back through the repository.
fn read_back<S: Store>(repo: &Repo<S>, id: &Id) -> BTreeMap<String, M> {
    let Object::Tree(entries) = repo.get(id).unwrap() else {
        panic!("not a tree")
    };
    entries
        .into_iter()
        .map(|e| {
            let node = match e.kind {
                Kind::File => M::File(repo.read_content(&e.id).unwrap()),
                Kind::Tree => M::Dir(read_back(repo, &e.id)),
                Kind::Graph => unreachable!(),
            };
            (e.name, node)
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Every edit, one at a time, leaves exactly the model's files; refused
    /// edits change nothing; applying the accepted edits in one call gives
    /// the same tree.
    #[test]
    fn edits_match_the_model(ops in prop::collection::vec(any_op(), 1..24)) {
        let mut repo = Repo::new(MemStore::default());
        let mut model = BTreeMap::new();
        let mut root = repo.edit_tree(None, &[]).unwrap();
        let (start, mut accepted) = (root, Vec::new());
        for op in &ops {
            let edit = to_edit(&mut repo, op);
            let ok = model_apply(&mut model, op);
            match repo.edit_tree(Some(root), std::slice::from_ref(&edit)) {
                Ok(next) => {
                    prop_assert!(ok, "kernel accepted {:?}", op);
                    root = next;
                    accepted.push(edit);
                }
                Err(_) => prop_assert!(!ok, "kernel refused {:?}", op),
            }
            prop_assert_eq!(&read_back(&repo, &root), &model);
        }
        prop_assert_eq!(repo.edit_tree(Some(start), &accepted).unwrap(), root);
    }
}

#[test]
fn untouched_subtrees_keep_their_ids() {
    let mut repo = Repo::new(MemStore::default());
    let a = repo.put_content(b"alpha").unwrap().id;
    let b = repo.put_content(b"beta").unwrap().id;
    let put = |path: &str, id| Edit::Put {
        path: path.into(),
        id,
    };
    let root = repo
        .edit_tree(None, &[put("x/1", a), put("y/z/2", b)])
        .unwrap();
    let sub = |repo: &Repo<MemStore>, root: &Id, name: &str| {
        let Object::Tree(e) = repo.get(root).unwrap() else {
            panic!()
        };
        e.into_iter().find(|e| e.name == name).unwrap().id
    };
    let y = sub(&repo, &root, "y");
    let edited = repo.edit_tree(Some(root), &[put("x/1", b)]).unwrap();
    assert_ne!(edited, root);
    assert_eq!(sub(&repo, &edited, "y"), y);
    // Back as it was: the same tree.
    assert_eq!(
        repo.edit_tree(Some(edited), &[put("x/1", a)]).unwrap(),
        root
    );
}
