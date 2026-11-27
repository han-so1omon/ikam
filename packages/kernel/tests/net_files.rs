//! E422: nets as files. A net sits at a path in a commit's tree like any
//! other file, a tree may hold many, and on a net that tracks versions each
//! firing names the version it ran under, so a net edited while it runs
//! still replays.

use ikam_kernel::net::Net;
use ikam_kernel::{Db, Error, MemStore, Repo};

const NIGHT: &str = "\
# the night loop, written by hand
net night 1
place cpu 1
place queued colored
place running colored
place measured colored
transition propose agent: -> queued
transition start cmd: queued, cpu -> running
transition finish pure: running -> measured, cpu
";

const TRAIN: &str = "\
net train 1
place gpu 2
place batch colored
place trained colored
transition load cmd: -> batch
transition step cmd: batch, gpu -> trained, gpu
";

const SMALL: &str = "\
net small 1
place a 3
place b
transition move pure: a -> b
";

/// The night net with a review step added: version 2.
fn night_v2() -> Net {
    let mut n = Net::parse(NIGHT).unwrap();
    n.version = 2;
    let more = Net::parse(
        "net x 1\nplace measured colored\nplace reviewed colored\n\
         transition review human: measured -> reviewed\n",
    )
    .unwrap();
    n.places.push(more.places[1].clone());
    n.transitions.push(more.transitions[0].clone());
    n.check().unwrap();
    n
}

fn tracked(repo: &mut Repo<MemStore>, db: &Db, net: &Net) -> Db {
    let db = repo.net_init(db, net).unwrap();
    repo.net_track_versions(&db, net).unwrap()
}

#[test]
fn a_net_is_a_file_read_back_at_each_commit() {
    let mut repo = Repo::new(MemStore::default());
    let night = Net::parse(NIGHT).unwrap();
    let (tree, v1) = repo.put_net_file(None, "nets/night.net", &night).unwrap();
    assert_eq!(v1, night.version_id());
    let c1 = repo.commit("main", tree, "night v1").unwrap();
    let v2net = night_v2();
    let (tree2, v2) = repo
        .put_net_file(Some(tree), "nets/night.net", &v2net)
        .unwrap();
    assert_ne!(v1, v2);
    let c2 = repo.commit("main", tree2, "night v2").unwrap();
    assert_eq!(repo.net_file(&c1, "nets/night.net").unwrap(), night);
    assert_eq!(repo.net_file(&c2, "nets/night.net").unwrap(), v2net);
    assert_eq!(repo.net_version(&v1).unwrap(), night);
    assert!(repo.net_file(&c2, "nets/none.net").is_err());
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn a_hand_written_file_is_the_same_version_as_its_canonical_form() {
    let mut repo = Repo::new(MemStore::default());
    let id = repo.put_content(NIGHT.as_bytes()).unwrap().id;
    let edit = ikam_kernel::worktree::Edit::Put {
        path: "night.net".into(),
        id,
    };
    let tree = repo.edit_tree(None, &[edit]).unwrap();
    let net = repo.net_file(&tree, "night.net").unwrap();
    assert_ne!(id, net.version_id(), "the file keeps its comments");
    assert_eq!(net.version_id(), Net::parse(NIGHT).unwrap().version_id());
    // The file's own id is not a version id.
    assert!(repo.net_version(&id).is_err());
}

#[test]
fn a_tree_holds_many_nets() {
    let mut repo = Repo::new(MemStore::default());
    let mut tree = None;
    for (path, text) in [
        ("nets/night.net", NIGHT),
        ("train/train.net", TRAIN),
        ("small.net", SMALL),
    ] {
        tree = Some(
            repo.put_net_file(tree, path, &Net::parse(text).unwrap())
                .unwrap()
                .0,
        );
    }
    let notes = repo.put_content(b"not a net").unwrap().id;
    let edit = ikam_kernel::worktree::Edit::Put {
        path: "nets/README".into(),
        id: notes,
    };
    let tree = repo.edit_tree(tree, &[edit]).unwrap();
    let paths: Vec<String> = repo
        .net_files(&tree)
        .unwrap()
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    assert_eq!(paths, ["nets/night.net", "small.net", "train/train.net"]);
}

#[test]
fn three_nets_fire_interleaved_and_an_edit_mid_run_keeps_earlier_firings() {
    let mut repo = Repo::new(MemStore::default());
    let night = Net::parse(NIGHT).unwrap();
    let train = Net::parse(TRAIN).unwrap();
    let small = Net::parse(SMALL).unwrap();
    let mut db = repo.db_empty().unwrap();
    for n in [&night, &train, &small] {
        db = tracked(&mut repo, &db, n);
    }
    let v2 = night_v2();
    for i in 0..6 {
        let c = format!("c{i}");
        // The night net is edited after its third candidate.
        let n = if i < 3 { &night } else { &v2 };
        db = repo
            .net_fire(&db, n, "propose", Some(&c), &format!("p{i}"))
            .unwrap()
            .0;
        db = repo
            .net_fire(&db, &train, "load", Some(&c), &format!("l{i}"))
            .unwrap()
            .0;
        db = repo
            .net_fire(&db, n, "start", Some(&c), &format!("s{i}"))
            .unwrap()
            .0;
        db = repo
            .net_fire(&db, &train, "step", Some(&c), &format!("t{i}"))
            .unwrap()
            .0;
        if i < 3 {
            db = repo
                .net_fire(&db, &small, "move", None, &format!("m{i}"))
                .unwrap()
                .0;
        }
        db = repo
            .net_fire(&db, n, "finish", Some(&c), &format!("f{i}"))
            .unwrap()
            .0;
        if i >= 3 {
            db = repo
                .net_fire(&db, n, "review", Some(&c), &format!("r{i}"))
                .unwrap()
                .0;
        }
    }
    for name in ["night", "train", "small"] {
        assert_eq!(
            repo.net_replay_versions(&db, name).unwrap(),
            repo.net_marking(&db, name).unwrap(),
            "{name}"
        );
    }
    let firings = repo.net_firings(&db, "night").unwrap();
    let (old, new): (Vec<_>, Vec<_>) = firings
        .iter()
        .partition(|f| f.net == Some(night.version_id()));
    assert_eq!(old.len(), 9);
    assert_eq!(new.len(), 12);
    assert!(new.iter().all(|f| f.net == Some(v2.version_id())));
    // The first version alone cannot replay the run: review fired under
    // the second.
    assert!(repo.net_replay(&db, &night).is_err());
    assert_eq!(
        repo.net_marking(&db, "small").unwrap()["b"][ikam_kernel::net::ANON],
        3
    );
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn a_retry_under_another_version_is_refused() {
    let mut repo = Repo::new(MemStore::default());
    let night = Net::parse(NIGHT).unwrap();
    let db = repo.db_empty().unwrap();
    let db = tracked(&mut repo, &db, &night);
    let (db, f) = repo
        .net_fire(&db, &night, "propose", Some("c"), "p")
        .unwrap();
    assert_eq!(f.net, Some(night.version_id()));
    let (same, g) = repo
        .net_fire(&db, &night, "propose", Some("c"), "p")
        .unwrap();
    assert_eq!((same, g), (db, f));
    assert!(matches!(
        repo.net_fire(&db, &night_v2(), "propose", Some("c"), "p"),
        Err(Error::IdempotencyConflict(_))
    ));
}

#[test]
fn an_untracked_net_records_as_before() {
    let mut repo = Repo::new(MemStore::default());
    let night = Net::parse(NIGHT).unwrap();
    let db = repo.db_empty().unwrap();
    let db = repo.net_init(&db, &night).unwrap();
    let (db, f) = repo
        .net_fire(&db, &night, "propose", Some("c"), "p")
        .unwrap();
    assert_eq!(f.net, None);
    let rec = repo.db_get(&db, "net/night/fire/p").unwrap().unwrap();
    assert_eq!(String::from_utf8(rec).unwrap().lines().count(), 4);
    assert!(repo.net_replay_versions(&db, "night").is_err());
    assert_eq!(repo.net_first_version(&db, "night").unwrap(), None);
}

#[test]
fn tracking_starts_from_the_initialized_version_before_any_firing() {
    let mut repo = Repo::new(MemStore::default());
    let night = Net::parse(NIGHT).unwrap();
    let empty = repo.db_empty().unwrap();
    assert!(repo.net_track_versions(&empty, &night).is_err());
    let db = repo.net_init(&empty, &night).unwrap();
    let mut other = night.clone();
    other.places[0].initial = 2;
    assert!(repo.net_track_versions(&db, &other).is_err());
    let (fired, _) = repo
        .net_fire(&db, &night, "propose", Some("c"), "p")
        .unwrap();
    assert!(repo.net_track_versions(&fired, &night).is_err());
    let db = repo.net_track_versions(&db, &night).unwrap();
    assert_eq!(repo.net_track_versions(&db, &night).unwrap(), db);
    assert!(repo.net_track_versions(&db, &night_v2()).is_err());
}

#[test]
fn tracking_works_with_a_sum_hash_and_outputs() {
    let mut repo = Repo::new(MemStore::default());
    let night = Net::parse(NIGHT).unwrap();
    let empty = repo.db_empty().unwrap();
    let db = repo.net_init_sum(&empty, &night).unwrap();
    let mut db = repo.net_track_versions(&db, &night).unwrap();
    let v2 = night_v2();
    for i in 0..4 {
        let n = if i < 2 { &night } else { &v2 };
        let c = format!("c{i}");
        db = repo
            .net_fire(&db, n, "propose", Some(&c), &format!("p{i}"))
            .unwrap()
            .0;
        db = repo
            .net_fire(&db, n, "start", Some(&c), &format!("s{i}"))
            .unwrap()
            .0;
        db = repo
            .net_fire_output(&db, n, "finish", Some(&c), &format!("f{i}"), c.as_bytes())
            .unwrap()
            .0;
    }
    let m = repo.net_replay_versions(&db, "night").unwrap();
    assert_eq!(m, repo.net_marking(&db, "night").unwrap());
    let f = repo.net_firings(&db, "night").unwrap();
    assert_eq!(f[11].net, Some(v2.version_id()));
    assert!(f[11].output.is_some());
}
