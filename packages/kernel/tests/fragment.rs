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

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn refused_grant_writes_nothing() {
    use ikam_kernel::{FileGrant, Grant, Store};
    let mut repo = Repo::new(MemStore::default());
    let docs: Vec<Vec<u8>> = (0..4)
        .map(|i| {
            format!("head {i}\n\n")
                .into_bytes()
                .into_iter()
                .chain(SHARED.iter().copied())
                .chain(b"tail\n".iter().copied())
                .collect()
        })
        .collect();
    let mut edits = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        let id = repo.put_fragmented(d, None).unwrap();
        edits.push(Edit::Put {
            path: format!("d/{i}.md"),
            id,
        });
    }
    let tree = repo.edit_tree(None, &edits).unwrap();
    let mut ix = repo.fragment_index(&tree).unwrap();
    let range = |d: &[u8]| {
        let at = d.windows(SHARED.len()).position(|w| w == SHARED).unwrap() as u64;
        vec![at..at + SHARED.len() as u64]
    };
    let ids: Vec<Id> = (0..4).map(|i| ix.files[&format!("d/{i}.md")].0).collect();
    let file = |i: usize| {
        FileGrant::new(
            &format!("d/{i}.md"),
            ids[i],
            range(&docs[i]),
            range(&docs[i]),
        )
    };
    let old = Id::of_content(SHARED);
    let new = b"Revised.\n\n";
    let count = |r: &Repo<MemStore>| r.store().ids().unwrap().len();
    // Three of four files granted: refused, nothing stored.
    let partial = Grant {
        goal: "g".into(),
        files: (0..3).map(file).collect(),
    };
    let before = count(&repo);
    assert!(
        repo.propagate_granted(&mut ix, &old, new, Some(&partial))
            .unwrap()
            .is_err()
    );
    assert_eq!(count(&repo), before);
    assert_eq!(ix.by_fragment[&old].len(), 4);
    // All four: allowed, and every file now renders the new paragraph.
    let full = Grant {
        goal: "g".into(),
        files: (0..4).map(file).collect(),
    };
    let changes = repo
        .propagate_granted(&mut ix, &old, new, Some(&full))
        .unwrap()
        .unwrap();
    assert_eq!(changes.len(), 4);
    for (i, d) in docs.iter().enumerate() {
        let got = repo
            .read_content(&ix.files[&format!("d/{i}.md")].0)
            .unwrap();
        let at = range(d)[0].start as usize;
        assert_eq!(&got[at..at + new.len()], new);
    }
}

#[test]
fn fragment_grant_scopes_one_fragment() {
    use ikam_kernel::{Denied, FragmentGrant, Store};
    let mut repo = Repo::new(MemStore::default());
    let mut edits = Vec::new();
    for p in ["a/1.md", "a/2.md", "b/1.md"] {
        let d: Vec<u8> = [b"head\n\n".as_slice(), SHARED, b"tail\n"].concat();
        let d = [p.as_bytes(), b"\n\n", &d].concat();
        let id = repo.put_fragmented(&d, None).unwrap();
        edits.push(Edit::Put { path: p.into(), id });
    }
    let tree = repo.edit_tree(None, &edits).unwrap();
    let mut ix = repo.fragment_index(&tree).unwrap();
    let old = Id::of_content(SHARED);
    let key = [1u8; 32];
    let g = |fragment: Id, within: &str| FragmentGrant {
        goal: "g".into(),
        fragment,
        within: within.into(),
    };
    let count = |r: &Repo<MemStore>| r.store().ids().unwrap().len();
    let before = count(&repo);
    // b/1.md renders the fragment too, so a grant within a/ is refused.
    let r = repo
        .propagate_fragment_granted(&mut ix, &old, b"New.\n\n", &g(old, "a/"))
        .unwrap();
    assert_eq!(r.unwrap_err(), ("b/1.md".to_string(), Denied::NoFile));
    let other = Id::of_content(b"other");
    let r = repo
        .propagate_fragment_granted(&mut ix, &old, b"New.\n\n", &g(other, ""))
        .unwrap();
    assert_eq!(r.unwrap_err().1, Denied::Stale);
    assert_eq!(count(&repo), before, "refusals write nothing");
    // A sealed grant does not verify once changed.
    let full = g(old, "");
    let tag = full.seal(&key);
    assert!(full.verify(&key, &tag) && !g(old, "a/").verify(&key, &tag));
    let changes = repo
        .propagate_fragment_granted(&mut ix, &old, b"New.\n\n", &full)
        .unwrap()
        .unwrap();
    assert_eq!(changes.len(), 3);
    // The fragment is the version: the same grant no longer applies.
    let new = Id::of_content(b"New.\n\n");
    let r = repo
        .propagate_fragment_granted(&mut ix, &new, b"Again.\n\n", &full)
        .unwrap();
    assert_eq!(r.unwrap_err().1, Denied::Stale);
}

#[test]
fn range_read_of_a_render_tree_skips_what_it_does_not_read() {
    // 4,096 lines in a tree of renders, 8 children each (E164, E166).
    let mut b = Vec::new();
    for i in 0..4096u32 {
        b.extend_from_slice(
            format!("{i:05},{}\n", i.wrapping_mul(2654435761) % 1_000_003).as_bytes(),
        );
    }
    let mut lines = Vec::new();
    let mut s = 0;
    for (i, &c) in b.iter().enumerate() {
        if c == b'\n' {
            lines.push(s..i + 1);
            s = i + 1;
        }
    }
    let mut repo = Repo::new(MemStore::default());
    let id = repo.put_cut(&b, &lines, Some(8)).unwrap();
    assert_eq!(repo.read_content(&id).unwrap(), b);
    let mid = b.len() as u64 / 2;
    let (got, work) = repo.read_range(&id, mid, 100).unwrap();
    assert_eq!(&got[..], &b[mid as usize..mid as usize + 100]);
    eprintln!("range work {work} of {}", b.len());
    assert!(
        work < b.len() / 8,
        "a range read decoded {work} of {} bytes",
        b.len()
    );
}
