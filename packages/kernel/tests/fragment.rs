//! Laws of fragment renders (`fragment.rs`, E155).

use std::collections::HashSet;

use ikam_kernel::{Edit, Id, MemStore, Repo, blocks};
use proptest::prelude::*;

fn para() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop::sample::select(b"ab \n".to_vec()), 0..40)
}

fn doc(shared: &[u8]) -> impl Strategy<Value = Vec<u8>> {
    let shared = shared.to_vec();
    (prop::collection::vec(para(), 0..6), any::<bool>()).prop_map(move |(ps, with)| {
        let mut out = Vec::new();
        for (i, p) in ps.iter().enumerate() {
            if with && i == ps.len() / 2 {
                out.extend_from_slice(&shared);
            }
            out.extend_from_slice(p);
            out.extend_from_slice(b"\n\n");
        }
        if with && ps.is_empty() {
            out.extend_from_slice(&shared);
        }
        out
    })
}

const SHARED: &[u8] = b"A shared notice paragraph.\n\n";

proptest! {
    #[test]
    fn blocks_cover_their_input(b in prop::collection::vec(prop::sample::select(b"ab\n\r".to_vec()), 0..200)) {
        let rs = blocks(&b);
        let joined: Vec<u8> = rs.iter().flat_map(|r| b[r.clone()].iter().copied()).collect();
        prop_assert_eq!(joined, b.clone());
        prop_assert!(rs.iter().all(|r| !r.is_empty()));
    }

    #[test]
    fn propagate_replaces_one_fragment_everywhere(
        docs in prop::collection::vec(doc(SHARED), 1..6),
        new in para(),
        runs in any::<bool>(),
    ) {
        let mut repo = Repo::new(MemStore::default());
        repo.short_records = runs;
        // With runs, no block stands alone: the shared one is found inside
        // run fragments (E157).
        let none = HashSet::new();
        let shared = runs.then_some(&none);
        let mut edits = Vec::new();
        for (i, d) in docs.iter().enumerate() {
            let id = repo.put_fragmented(d, shared).unwrap();
            prop_assert_eq!(&repo.read_content(&id).unwrap(), d);
            edits.push(Edit::Put { path: format!("d/{i}.md"), id });
        }
        let tree = repo.edit_tree(None, &edits).unwrap();
        let mut ix = repo.fragment_index(&tree).unwrap();
        let old = Id::of_content(SHARED);
        let mut new_bytes = new.clone();
        new_bytes.extend_from_slice(b"\n\n");
        let changes = repo.propagate(&mut ix, &old, &new_bytes).unwrap();
        let tree2 = repo.edit_tree(Some(tree), &changes).unwrap();
        let ix2 = repo.fragment_index(&tree2).unwrap();
        for (i, d) in docs.iter().enumerate() {
            let path = format!("d/{i}.md");
            let id = ix2.files[&path].0;
            prop_assert_eq!(ix.files[&path].0, id, "the index follows its edits");
            let got = repo.read_content(&id).unwrap();
            // The expected bytes: the old file's blocks, the shared one replaced.
            let expect: Vec<u8> = blocks(d)
                .into_iter()
                .flat_map(|r| {
                    if &d[r.clone()] == SHARED { new_bytes.clone() } else { d[r].to_vec() }
                })
                .collect();
            // A content of one fragment is stored as itself, not a render.
            let frags = if runs { 1 } else { blocks(d).len() };
            let rendered = blocks(d).iter().any(|r| &d[r.clone()] == SHARED) && frags > 1;
            if rendered {
                prop_assert_eq!(got, expect);
            } else {
                prop_assert_eq!(&got, d);
            }
        }
        prop_assert!(repo.fsck().unwrap().is_empty());
    }
}

#[test]
fn render_survives_repack() {
    let mut repo = Repo::new(MemStore::default());
    let d = b"one\n\ntwo\n\nthree\n".to_vec();
    let id = repo.put_fragmented(&d, None).unwrap();
    let tree = repo
        .edit_tree(
            None,
            &[Edit::Put {
                path: "a.md".into(),
                id,
            }],
        )
        .unwrap();
    repo.commit("main", tree, "m").unwrap();
    repo.repack().unwrap();
    assert_eq!(repo.read_content(&id).unwrap(), d);
    assert_eq!(repo.fragments_of(&id).unwrap().map(|f| f.len()), Some(3));
}
