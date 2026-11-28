//! E426: point-in-time training tables pinned to commits (src/train.rs).
//! Every row is checked against the database read at its own commit
//! (`db_at`), and its label against the commits after it.

use std::collections::BTreeMap;

use ikam_kernel::train::{self, History, TrainRule};
use ikam_kernel::{Event, Id, MemStore, Object, Repo, SharedStore, Store};

fn rnd(mut s: u64) -> u64 {
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    s
}

/// A database of `rows` keys under `t/` (and a few under `u/`, outside
/// the prefix) over `commits` commits on `db/t`: changes, writes of the
/// same value, a removal, a subtree removal, a range removal and keys
/// coming back. Returns the commits, oldest first.
fn history<S: Store>(repo: &mut Repo<S>, rows: u64, commits: u64) -> Vec<Id> {
    let mut db = repo.db_empty().unwrap();
    let mut values: BTreeMap<u64, u64> = BTreeMap::new();
    let mut out = Vec::new();
    for c in 0..commits {
        let mut ev = Vec::new();
        let idem = |i: u64, what: &str| format!("{c}-{i}-{what}");
        for i in 0..rows {
            let key = format!("t/{i:04}");
            let r = rnd((c << 32) ^ i ^ 0x5bd1_e995);
            let rate = if i.is_multiple_of(4) { 3 } else { 12 }; // 1 in 3, 1 in 12
            let next = match values.get(&i) {
                None if c == 0 || r.is_multiple_of(9) => Some(r % 1000),
                None => None,
                Some(v) if r.is_multiple_of(rate) => Some(v + 1 + r % 7),
                Some(&v) if r % 10 == 1 => Some(v), // the same value again
                Some(_) => None,
            };
            if let Some(v) = next {
                values.insert(i, v);
                ev.push(Event::upsert(
                    &key,
                    v.to_string().as_bytes(),
                    &idem(i, "up"),
                ));
            }
        }
        ev.push(Event::upsert(
            "u/x",
            c.to_string().as_bytes(),
            &idem(0, "u"),
        ));
        match c {
            3 => {
                ev.push(Event::remove("t/0007", false, &idem(7, "rm")));
                values.remove(&7);
            }
            5 => {
                // keys below `t/0009`, then `t/0009` itself stays
                ev.push(Event::upsert("t/0009/a", b"5", &idem(9, "a")));
                ev.push(Event::upsert("t/0009/b", b"word", &idem(9, "b")));
            }
            6 => ev.push(Event::remove("t/0009", true, &idem(9, "sub"))),
            7 => {
                ev.push(Event::remove_range("t/0020", "t/0030", &idem(20, "range")));
                values.retain(|i, _| !(20..30).contains(i));
            }
            _ => {}
        }
        if c == 6 {
            values.remove(&9);
        }
        db = repo.db_append(&db, &ev).unwrap();
        out.push(repo.db_commit("db/t", &db, "c").unwrap());
    }
    out
}

/// The value of every key under `t/` in a commit, read from its database.
fn snapshot(repo: &mut Repo<MemStore>, commit: &Id) -> BTreeMap<String, Vec<u8>> {
    let db = repo.db_at(commit).unwrap();
    repo.db_prefix(&db, "t/", usize::MAX, false)
        .unwrap()
        .into_iter()
        .collect()
}

fn int_of(v: &[u8]) -> i64 {
    std::str::from_utf8(v)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(v.len() as i64)
}

fn rule(horizon: u32, every: u32) -> TrainRule {
    TrainRule {
        prefix: "t/".into(),
        horizon,
        every,
    }
}

#[test]
fn every_row_matches_the_database_at_its_commit_and_after() {
    let mut repo = Repo::new(MemStore::default());
    let chain = history(&mut repo, 120, 14);
    let snaps: Vec<_> = chain.iter().map(|c| snapshot(&mut repo, c)).collect();
    let tip = *chain.last().unwrap();
    for (horizon, every) in [(1, 1), (3, 2), (5, 4)] {
        let table = repo
            .train_table("train/t", &tip, &rule(horizon, every))
            .unwrap();
        let rows = repo.train_rows(&table).unwrap();
        let mut expected = 0;
        let mut ones = 0;
        for t in (0..chain.len()).step_by(every as usize) {
            if t + horizon as usize >= chain.len() {
                break;
            }
            let at: Vec<_> = rows.iter().filter(|r| r.commit == chain[t]).collect();
            assert_eq!(at.len(), snaps[t].len(), "rows at commit {t}");
            expected += at.len();
            for r in at {
                let v = &snaps[t][&r.key];
                assert_eq!(r.value, int_of(v), "{} at {t}", r.key);
                let changes = (0..=t)
                    .filter(|&k| {
                        snaps[k].get(&r.key)
                            != if k == 0 {
                                None
                            } else {
                                snaps[k - 1].get(&r.key)
                            }
                    })
                    .collect::<Vec<_>>();
                assert_eq!(r.changes as usize, changes.len(), "{} at {t}", r.key);
                assert_eq!(
                    r.age as usize,
                    t - changes.last().unwrap(),
                    "{} at {t}",
                    r.key
                );
                let label = (t + 1..=t + horizon as usize).any(|k| snaps[k].get(&r.key) != Some(v));
                assert_eq!(r.label, label, "{} at {t}", r.key);
                ones += usize::from(label);
            }
        }
        assert_eq!(rows.len(), expected);
        assert!(ones > 0 && ones < expected, "both labels occur");
        // `u/x` is outside the prefix; `t/0009/a` and `/b` were under it.
        assert!(rows.iter().all(|r| r.key.starts_with("t/")));
        if every == 1 {
            // set to `word` (4 bytes) at commit 5, removed with its parent at 6
            assert!(
                rows.iter()
                    .any(|r| r.key == "t/0009/b" && r.value == 4 && r.label)
            );
        }
        assert_eq!(
            repo.train_verify(&repo.resolve("train/t").unwrap())
                .unwrap()
                .past_obs,
            0
        );
    }
}

#[test]
fn a_rebuild_gives_the_same_ids_and_the_graph_leads_back() {
    let mut repo = Repo::new(MemStore::default());
    let chain = history(&mut repo, 80, 10);
    let tip = *chain.last().unwrap();
    let r = rule(2, 3);
    let table = repo.train_table("train/t", &tip, &r).unwrap();
    let pin = repo.resolve("train/t").unwrap();
    assert_eq!(repo.train_table("train/t", &tip, &r).unwrap(), table);
    assert_eq!(repo.resolve("train/t").unwrap(), pin);

    // From scratch, in another store: the same commits, history and table.
    let mut other = Repo::new(MemStore::default());
    assert_eq!(history(&mut other, 80, 10), chain);
    assert_eq!(other.train_table("train/t", &tip, &r).unwrap(), table);
    assert_eq!(other.resolve("train/t").unwrap(), pin);
    let (rule_id, history_id) = repo.train_inputs(&table).unwrap();
    let (out, reads) = train::build(
        &repo.read_content(&rule_id).unwrap(),
        &repo.read_content(&history_id).unwrap(),
    )
    .unwrap();
    assert_eq!(Id::of_content(&out), table);
    assert!(reads.feature > 0 && reads.past_obs == 0);

    // table -> derivation -> rule and history -> every commit it read.
    let links = repo.links(&table).unwrap();
    assert!(links.iter().any(|(l, _)| l == "derivation"));
    assert_eq!(
        TrainRule::decode(&repo.read_content(&rule_id).unwrap()).unwrap(),
        r
    );
    let h = History::decode(&repo.read_content(&history_id).unwrap()).unwrap();
    assert_eq!(h.commits, chain);
    // The pin: its tree names the table, its parent is the tip.
    let Object::Commit(c) = repo.get(&pin).unwrap() else {
        panic!("pin is a commit")
    };
    assert_eq!(c.parents, vec![tip]);
    assert!(
        repo.used_by(&table)
            .unwrap()
            .iter()
            .any(|(id, _)| *id == c.tree)
    );

    // Another rule is another table; another tip another history.
    let other_table = repo.train_table("train/u", &tip, &rule(1, 3)).unwrap();
    assert_ne!(other_table, table);
    let earlier = repo.train_table("train/v", &chain[7], &r).unwrap();
    assert_ne!(repo.train_inputs(&earlier).unwrap().1, history_id);
}

#[test]
fn repack_drops_the_table_and_fsck_evaluates_it_again() {
    let mut shared = SharedStore::new(MemStore::default());
    let mut repo = Repo::new(shared.clone());
    let chain = history(&mut repo, 150, 12);
    let tip = *chain.last().unwrap();
    let table = repo.train_table("train/t", &tip, &rule(2, 2)).unwrap();
    let bytes = repo.read_content(&table).unwrap();
    let (_, history_id) = repo.train_inputs(&table).unwrap();
    assert!(repo.store().has(&table));
    assert!(repo.fsck().unwrap().is_empty());
    let packed = repo.repack().unwrap();
    assert!(packed.applied, "{packed:?}");
    assert!(!repo.store().has(&table), "the table's bytes are dropped");
    assert!(
        repo.derivations(&history_id).unwrap().is_empty(),
        "the run's output is stored"
    );
    assert!(repo.fsck().unwrap().is_empty());
    assert_eq!(repo.read_content(&table).unwrap(), bytes);
    let pin = repo.resolve("train/t").unwrap();
    repo.train_verify(&pin).unwrap();

    // The database's own ref gone, the pin still keeps what it was read from.
    shared.delete_ref("db/t", tip).unwrap();
    repo.gc().unwrap();
    assert!(repo.fsck().unwrap().is_empty());
    assert_eq!(repo.read_content(&table).unwrap(), bytes);
    repo.train_verify(&pin).unwrap();
}

#[test]
fn a_pin_whose_history_was_not_read_from_its_parent_is_refused() {
    let mut repo = Repo::new(MemStore::default());
    let chain = history(&mut repo, 40, 6);
    let r = rule(1, 1);
    repo.train_table("train/t", &chain[5], &r).unwrap();
    let real = repo.resolve("train/t").unwrap();
    let Object::Commit(mut c) = repo.get(&real).unwrap() else {
        panic!()
    };
    c.parents = vec![chain[4]];
    let forged = repo.put(&Object::Commit(c)).unwrap();
    assert!(repo.train_verify(&forged).is_err());
    assert!(repo.train_verify(&real).is_ok());
}
