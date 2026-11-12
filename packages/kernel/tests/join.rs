//! E243: joins kept up to date give the rows a recompute gives.

use ikam_kernel::join::{Arrangement, Join};
use ikam_kernel::{Event, MemStore, Repo};

fn field(v: &[u8], j: usize) -> String {
    String::from_utf8_lossy(v)
        .split(',')
        .nth(j)
        .unwrap_or("")
        .to_string()
}

fn by_group(_: &str, v: &[u8]) -> Option<String> {
    Some(field(v, 0)).filter(|g| !g.is_empty())
}

fn by_own_key(k: &str, _: &[u8]) -> Option<String> {
    Some(k.rsplit('/').next().unwrap().to_string())
}

fn pair(l: &[u8], r: &[u8]) -> Vec<u8> {
    [l, b"|", r].concat()
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn arrs() -> Vec<Arrangement> {
    vec![
        Arrangement {
            name: "a_by_group".into(),
            table: "a".into(),
            key: by_group,
        },
        Arrangement {
            name: "c_by_group".into(),
            table: "c".into(),
            key: by_group,
        },
        Arrangement {
            name: "b_by_key".into(),
            table: "b".into(),
            key: by_own_key,
        },
        // A many-to-many join: a with c on the group.
    ]
}

fn joins() -> Vec<Join> {
    vec![
        Join {
            name: "a_b".into(),
            left: "a_by_group".into(),
            right: "b_by_key".into(),
            combine: pair,
        },
        Join {
            name: "c_b".into(),
            left: "c_by_group".into(),
            right: "b_by_key".into(),
            combine: pair,
        },
        Join {
            name: "a_c".into(),
            left: "a_by_group".into(),
            right: "c_by_group".into(),
            combine: pair,
        },
    ]
}

fn change(rng: &mut Lcg, idem: String) -> Event {
    let t = ["a", "c", "b"][rng.next() as usize % 3];
    let k = format!("{t}/{}", rng.next() % if t == "b" { 12 } else { 300 });
    match rng.next() % 5 {
        0 => Event::remove(&k, false, &idem),
        1 if rng.next().is_multiple_of(8) => Event::remove(t, true, &idem),
        _ if t == "b" => Event::upsert(
            &k,
            format!("name{},{}", rng.next() % 9, rng.next() % 50).as_bytes(),
            &idem,
        ),
        _ => Event::upsert(
            &k,
            format!("{},{}", rng.next() % 14, rng.next() % 99).as_bytes(),
            &idem,
        ),
    }
}

#[test]
fn updates_equal_a_recompute() {
    let mut rng = Lcg(5);
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let start: Vec<Event> = (0..900)
        .map(|i| change(&mut rng, format!("s{i}")))
        .collect();
    let mut db = repo.db_append(&empty, &start).unwrap();
    let (arrs, joins) = (arrs(), joins());
    let mut state = repo.joins_build(&arrs, &joins, &db).unwrap();
    for round in 0..40 {
        let batch: Vec<Event> = (0..1 + rng.next() % 30)
            .map(|i| change(&mut rng, format!("r{round}-{i}")))
            .collect();
        db = repo.db_append(&db, &batch).unwrap();
        state = repo.joins_update(&arrs, &joins, &state, &db).unwrap();
        let fresh = repo.joins_build(&arrs, &joins, &db).unwrap();
        assert_eq!(
            state.arranged, fresh.arranged,
            "round {round}: arrangements"
        );
        for j in &joins {
            let want = repo.join_recompute(&arrs, j, &db).unwrap().1;
            assert_eq!(
                repo.join_rows(&state, &j.name).unwrap(),
                want,
                "round {round}: {}",
                j.name
            );
        }
    }
    assert!(!repo.join_rows(&state, "a_c").unwrap().is_empty());
}

#[test]
fn options_give_the_same_rows() {
    use ikam_kernel::join::JoinOpts;
    let mut rng = Lcg(9);
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let start: Vec<Event> = (0..600)
        .map(|i| change(&mut rng, format!("s{i}")))
        .collect();
    let db0 = repo.db_append(&empty, &start).unwrap();
    let (arrs, joins) = (arrs(), joins());
    let fast = JoinOpts {
        read_once: true,
        plain_writes: true,
        memo: true,
        ..Default::default()
    };
    let ways = [
        JoinOpts {
            plain_writes: true,
            ..Default::default()
        },
        JoinOpts {
            memo: true,
            ..Default::default()
        },
        JoinOpts {
            recompute_above: Some(0.0),
            ..fast
        },
        JoinOpts {
            recompute_above: Some(0.5),
            ..fast
        },
        JoinOpts {
            recompute_above: Some(2.0),
            ..fast
        },
    ];
    let mut states: Vec<_> = ways
        .iter()
        .map(|_| repo.joins_build(&arrs, &joins, &db0).unwrap())
        .collect();
    let mut db = db0;
    for round in 0..25 {
        let batch: Vec<Event> = (0..1 + rng.next() % 60)
            .map(|i| change(&mut rng, format!("o{round}-{i}")))
            .collect();
        db = repo.db_append(&db, &batch).unwrap();
        for (state, opts) in states.iter_mut().zip(&ways) {
            *state = repo
                .joins_update_with(&arrs, &joins, state, &db, opts)
                .unwrap();
            for j in &joins {
                let want = repo.join_recompute(&arrs, j, &db).unwrap().1;
                assert_eq!(
                    repo.join_rows_with(state, &j.name, opts).unwrap(),
                    want,
                    "round {round}: {}",
                    j.name
                );
                assert_eq!(repo.join_rows(state, &j.name).unwrap(), want);
                assert_eq!(
                    repo.join_recompute_with(&arrs, j, &db, opts).unwrap().1,
                    want
                );
            }
        }
    }
}
