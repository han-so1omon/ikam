//! Semantic claims: measured weights, graph edges, lifetime, promotion.

use std::fs;
use std::path::Path;

use ikam_kernel::{Arg, Commit, Form, FsStore, Id, Kind, MemStore, Object, Repo, Store, TreeEntry};

fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 56) as u8
        })
        .collect()
}

fn prose(topic: &str, n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| {
            format!(
                "Section {i}: the {topic} plan covers scope, budget and schedule for {topic}.\n"
            )
        })
        .collect::<String>()
        .into_bytes()
}

/// Store bytes as-is, bypassing the planners (content imported unrelated).
fn import(dir: &Path, bytes: &[u8]) -> Id {
    let id = Id::of_content(bytes);
    let hex = id.to_hex();
    let path = dir.join("objects").join(&hex[..2]).join(&hex[2..]);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, [b"B".as_slice(), bytes].concat()).unwrap();
    id
}

#[test]
fn claim_weights_are_measured_information() {
    let mut repo = Repo::new(MemStore::default());
    let a = repo.put_content(&prose("bridge", 40)).unwrap().id;
    let b = repo.put_content(&prose("bridge", 41)).unwrap().id;
    let r = repo.put_content(&noise(3000, 3)).unwrap().id;
    let related = repo.gain_bits(&Arg::Whole(b), &Arg::Whole(a)).unwrap();
    let unrelated = repo.gain_bits(&Arg::Whole(b), &Arg::Whole(r)).unwrap();
    assert!(related > 1000, "related gain {related}");
    assert!(unrelated.abs() < 200, "unrelated gain {unrelated}");

    let c = repo
        .claim(Arg::Whole(b), "revision of", Arg::Whole(a), None)
        .unwrap();
    let Object::Claim(claim) = repo.get(&c).unwrap() else {
        panic!()
    };
    assert_eq!(claim.gain_bits, related, "weight is measured, not asserted");
    assert_eq!(repo.claims(&a).unwrap(), [(c, claim.clone())]);
    assert_eq!(repo.claims(&b).unwrap().len(), 1);
    let links = repo.links(&c).unwrap();
    assert!(links.contains(&("subject".to_string(), b)));
    assert!(links.contains(&("object (revision of)".to_string(), a)));
}

#[test]
fn claims_can_relate_byte_ranges() {
    let mut repo = Repo::new(MemStore::default());
    let doc = repo.put_content(&prose("tunnel", 30)).unwrap().id;
    let entity = repo.put_content(b"tunnel").unwrap().id;
    let mention = Arg::Range {
        id: doc,
        start: 15,
        len: 6,
    };
    assert_eq!(repo.select(&mention).unwrap(), b"tunnel");
    let c = repo
        .claim(mention, "mentions", Arg::Whole(entity), None)
        .unwrap();
    assert!(
        repo.links(&c)
            .unwrap()
            .contains(&("subject[15..21]".to_string(), doc))
    );
    let out_of_range = Arg::Range {
        id: doc,
        start: 1 << 40,
        len: 6,
    };
    assert!(
        repo.claim(out_of_range, "mentions", Arg::Whole(entity), None)
            .is_err()
    );
}

#[test]
fn relate_ranks_informative_neighbours_first() {
    let mut repo = Repo::new(MemStore::default());
    let a = repo.put_content(&prose("harbor", 60)).unwrap().id;
    let weak = repo
        .put_content(&[prose("harbor", 3), noise(4000, 9)].concat())
        .unwrap()
        .id;
    let q = repo.put_content(&prose("harbor", 61)).unwrap().id;
    let ranked = repo.relate(&q, 3).unwrap();
    assert_eq!(ranked[0].0, a, "{ranked:?}");
    assert!(ranked.iter().all(|(id, _)| *id != q));
    assert!(ranked.windows(2).all(|w| w[0].1 >= w[1].1), "best first");
    if let Some((_, gain)) = ranked.iter().find(|(id, _)| *id == weak) {
        assert!(*gain < ranked[0].1);
    }
}

fn tree_of(repo: &mut Repo<FsStore>, ids: &[Id]) -> Id {
    let entries = ids
        .iter()
        .enumerate()
        .map(|(i, id)| TreeEntry {
            name: format!("f{i}"),
            kind: Kind::File,
            id: *id,
        })
        .collect();
    repo.put(&Object::tree(entries).unwrap()).unwrap()
}

#[test]
fn claims_annotate_without_keeping_content_alive() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open(dir.path()).unwrap());
    let a = repo.put_content(&prose("canal", 20)).unwrap().id;
    let b = repo.put_content(&noise(2000, 4)).unwrap().id;
    let c = repo
        .claim(Arg::Whole(a), "related to", Arg::Whole(b), None)
        .unwrap();
    let both = tree_of(&mut repo, &[a, b]);
    let first = repo.commit("main", both, "").unwrap();
    repo.gc().unwrap();
    assert!(repo.get(&c).is_ok(), "both endpoints live: claim kept");

    // Point main at a history that no longer contains b.
    let only_a = tree_of(&mut repo, &[a]);
    let root = repo
        .put(&Object::Commit(Commit {
            tree: only_a,
            parents: vec![],
            message: String::new(),
        }))
        .unwrap();
    FsStore::open(dir.path())
        .unwrap()
        .set_ref("main", Some(first), root)
        .unwrap();
    repo.gc().unwrap();
    assert!(repo.get(&c).is_err(), "claim dropped with its endpoint");
    assert!(
        repo.read_content(&b).is_err(),
        "a claim does not keep content alive"
    );
    assert_eq!(repo.read_content(&a).unwrap(), prose("canal", 20));
}

#[test]
fn promotion_turns_a_claim_into_verified_storage() {
    let dir = tempfile::tempdir().unwrap();
    let base = prose("viaduct", 80);
    let revised = [
        &base[..2000],
        b"one inserted sentence about the viaduct.\n",
        &base[2000..],
    ]
    .concat();
    let (x, y) = (import(dir.path(), &revised), import(dir.path(), &base));
    let mut repo = Repo::new(FsStore::open(dir.path()).unwrap());
    let c = repo
        .claim(Arg::Whole(x), "revision of", Arg::Whole(y), None)
        .unwrap();
    let before = repo.store().read(&x).unwrap().len();
    let put = repo.promote(&c).unwrap().expect("smaller derivation");
    assert_eq!((put.id, put.form), (x, Form::Derived));
    assert!(!repo.store().has(&x), "x's stored bytes dropped");
    assert_eq!(repo.read_content(&x).unwrap(), revised);
    assert!(repo.bytes_written < before);
    assert!(repo.fsck().unwrap().is_empty());

    // The reverse claim would make y need x while x needs y: refused.
    let back = repo
        .claim(Arg::Whole(y), "base of", Arg::Whole(x), None)
        .unwrap();
    assert!(repo.promote(&back).unwrap().is_none());
    assert_eq!(repo.read_content(&y).unwrap(), base);
    assert_eq!(repo.read_content(&x).unwrap(), revised);
}
