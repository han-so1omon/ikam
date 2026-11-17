//! E255: embeddings keyed by content hash give every row its fragments'
//! vectors, embedding each distinct fragment once per model.

use std::collections::BTreeSet;

use ikam_kernel::{Event, MemStore, Repo};

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

fn model(seed: u32) -> impl FnMut(&[u8]) -> Vec<f32> {
    move |f: &[u8]| {
        (0..8)
            .map(|i| {
                f.iter()
                    .map(|&b| u32::from(b))
                    .sum::<u32>()
                    .wrapping_mul(seed + i) as f32
            })
            .collect()
    }
}

fn text(rng: &mut Lcg) -> String {
    (0..1 + rng.next() % 4)
        .map(|_| format!("line {}", rng.next() % 30))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn rows_get_their_fragments_vectors_and_each_fragment_is_embedded_once() {
    let mut rng = Lcg(8);
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let start: Vec<Event> = (0..200)
        .map(|i| {
            Event::upsert(
                &format!("r/{i:04}"),
                text(&mut rng).as_bytes(),
                &format!("s{i}"),
            )
        })
        .collect();
    let mut db = repo.db_append(&empty, &start).unwrap();
    let e0 = repo.embeds_empty().unwrap();
    let (mut e, stats) = repo
        .embeds_update(&e0, &empty, &db, "r/", "m1", &mut model(3))
        .unwrap();
    let distinct: BTreeSet<String> = repo
        .db_scan(&db)
        .unwrap()
        .iter()
        .flat_map(|(_, v)| {
            String::from_utf8(v.clone())
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(stats.embedded, distinct.len());
    for round in 0..10 {
        let batch: Vec<Event> = (0..1 + rng.next() % 20)
            .map(|j| {
                let k = format!("r/{:04}", rng.next() % 220);
                match rng.next() % 4 {
                    0 => Event::remove(&k, false, &format!("u{round}-{j}")),
                    _ => Event::upsert(&k, text(&mut rng).as_bytes(), &format!("u{round}-{j}")),
                }
            })
            .collect();
        let next = repo.db_append(&db, &batch).unwrap();
        e = repo
            .embeds_update(&e, &db, &next, "r/", "m1", &mut model(3))
            .unwrap()
            .0;
        db = next;
    }
    let check = |repo: &Repo<MemStore>, e, m: &mut dyn FnMut(&[u8]) -> Vec<f32>| {
        for (k, v) in repo.db_scan(&db).unwrap() {
            let want: Vec<Vec<f32>> = v
                .split(|&b| b == b'\n')
                .filter(|f| !f.is_empty())
                .map(&mut *m)
                .collect();
            assert_eq!(repo.embeds_of(e, &k).unwrap(), want, "{k}");
        }
    };
    check(&repo, &e, &mut model(3));
    // A new model: the row view is rebuilt, each distinct fragment once.
    let fresh = ikam_kernel::embeds::Embeds { rows: e0.rows, ..e };
    let (e2, stats) = repo
        .embeds_update(&fresh, &empty, &db, "r/", "m2", &mut model(5))
        .unwrap();
    let distinct: BTreeSet<String> = repo
        .db_scan(&db)
        .unwrap()
        .iter()
        .flat_map(|(_, v)| {
            String::from_utf8(v.clone())
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(stats.embedded, distinct.len());
    check(&repo, &e2, &mut model(5));
}
