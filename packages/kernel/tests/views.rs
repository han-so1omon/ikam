//! E242: a view graph's updates give the roots a full build gives.

use ikam_kernel::views::{Mode, View, ViewGraph, ViewOp};
use ikam_kernel::{Db, Event, MemStore, Repo};

fn field(v: &[u8], j: usize) -> &str {
    std::str::from_utf8(v).unwrap().split(',').nth(j).unwrap()
}

fn by_group(_: &str, v: &[u8]) -> Option<(String, i64)> {
    Some((field(v, 0).to_string(), field(v, 1).parse().ok()?))
}

fn flagged(k: &str, v: &[u8]) -> Option<(String, Vec<u8>)> {
    (field(v, 2) == "warn").then(|| {
        (
            format!("flagged/{k}"),
            format!("{},{}", field(v, 0), field(v, 1)).into_bytes(),
        )
    })
}

fn small(_: &str, v: &[u8]) -> Result<(), String> {
    let n: i64 = field(v, 1).parse().map_err(|_| "not a number")?;
    if n < 0 {
        Err("negative".into())
    } else {
        Ok(())
    }
}

fn graph(mode: Mode) -> ViewGraph {
    ViewGraph::new(vec![
        // Listed out of order on purpose.
        View {
            name: "flagged_by_group".into(),
            input: "flagged".into(),
            mode,
            op: ViewOp::GroupSum(by_group),
            check: None,
        },
        View {
            name: "by_group".into(),
            input: "kv".into(),
            mode,
            op: ViewOp::GroupSum(by_group),
            check: None,
        },
        View {
            name: "flagged".into(),
            input: "kv".into(),
            mode,
            op: ViewOp::Map(flagged),
            check: Some(small),
        },
        View {
            name: "all_by_group".into(),
            input: "kv".into(),
            mode: Mode::OnRead,
            op: ViewOp::GroupSum(by_group),
            check: None,
        },
    ])
    .unwrap()
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

fn value(rng: &mut Lcg) -> Vec<u8> {
    let s = ["ok", "warn", "idle"][rng.next() as usize % 3];
    format!("g{},{},{s}", rng.next() % 7, rng.next() % 100).into_bytes()
}

#[test]
fn incremental_and_stored_updates_equal_a_full_build() {
    let mut rng = Lcg(9);
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let events: Vec<Event> = (0..3000)
        .map(|i| Event::upsert(&format!("r/{i:05}"), &value(&mut rng), &format!("w{i}")))
        .collect();
    let mut db: Db = repo.db_append(&empty, &events).unwrap();
    let (inc, sto) = (graph(Mode::Incremental), graph(Mode::Stored));
    let (mut s_inc, mut s_sto) = (
        repo.views_build(&inc, &db).unwrap(),
        repo.views_build(&sto, &db).unwrap(),
    );
    assert_eq!(s_inc, s_sto);
    // Values live in the content each update writes, so roots can differ
    // between ways; the rows may not.
    let rows =
        |repo: &Repo<MemStore>, g: &ViewGraph, s: &ikam_kernel::views::ViewState| -> Vec<_> {
            ["by_group", "flagged", "flagged_by_group"]
                .iter()
                .map(|v| repo.view_rows(g, s, v).unwrap())
                .collect()
        };
    for round in 0..20 {
        let mut batch = Vec::new();
        for j in 0..(1 + rng.next() % 40) {
            let k = format!("r/{:05}", rng.next() % 3300);
            let idem = format!("b{round}-{j}");
            batch.push(match rng.next() % 4 {
                0 => Event::remove(&k, false, &idem),
                1 if round % 5 == 0 => Event::remove("r", true, &idem),
                _ => Event::upsert(&k, &value(&mut rng), &idem),
            });
        }
        db = repo.db_append(&db, &batch).unwrap();
        s_inc = repo.views_update(&inc, &s_inc, &db).unwrap();
        s_sto = repo.views_update(&sto, &s_sto, &db).unwrap();
        let full = repo.views_build(&inc, &db).unwrap();
        assert_eq!(
            rows(&repo, &inc, &s_inc),
            rows(&repo, &inc, &full),
            "round {round}"
        );
        assert_eq!(
            rows(&repo, &sto, &s_sto),
            rows(&repo, &inc, &full),
            "round {round}"
        );
        assert_eq!(
            s_sto.roots, full.roots,
            "a rebuild stores what a build does"
        );
        // The on-read view equals the stored one it mirrors.
        assert_eq!(
            repo.view_rows(&inc, &s_inc, "all_by_group").unwrap(),
            repo.view_rows(&inc, &s_inc, "by_group").unwrap()
        );
    }
}

#[test]
fn bad_graphs_and_failed_checks_are_refused() {
    let v = |name: &str, input: &str, mode| View {
        name: name.into(),
        input: input.into(),
        mode,
        op: ViewOp::Map(flagged),
        check: None,
    };
    assert!(ViewGraph::new(vec![v("a", "b", Mode::Stored), v("b", "a", Mode::Stored)]).is_err());
    assert!(ViewGraph::new(vec![v("a", "missing", Mode::Stored)]).is_err());
    assert!(ViewGraph::new(vec![v("a", "kv", Mode::Stored), v("a", "kv", Mode::Stored)]).is_err());
    assert!(
        ViewGraph::new(vec![
            v("a", "kv", Mode::OnRead),
            v("b", "a", Mode::Incremental)
        ])
        .is_err()
    );
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let db = repo
        .db_append(&empty, &[Event::upsert("r/1", b"g1,5,warn", "a")])
        .unwrap();
    let g = graph(Mode::Incremental);
    let state = repo.views_build(&g, &db).unwrap();
    let bad = repo
        .db_append(&db, &[Event::upsert("r/2", b"g1,-3,warn", "b")])
        .unwrap();
    assert!(repo.views_update(&g, &state, &bad).is_err());
    assert!(repo.views_build(&g, &bad).is_err());
}
