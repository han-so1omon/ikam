//! E249: a diff that skips shared subtrees gives the changes a full diff gives.

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

#[test]
fn pruned_diff_equals_full_diff() {
    let mut rng = Lcg(3);
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let start: Vec<Event> = (0..5000)
        .map(|i| {
            Event::upsert(
                &format!("t/{:06}", rng.next() % 9000),
                format!("v{i}").as_bytes(),
                &format!("s{i}"),
            )
        })
        .collect();
    let mut states = vec![empty, repo.db_append(&empty, &start).unwrap()];
    for round in 0..40 {
        let size = [1, 3, 10, 100, 1000][round % 5];
        let batch: Vec<Event> = (0..size)
            .map(|j| {
                let k = format!("t/{:06}", rng.next() % 9000);
                let idem = format!("r{round}-{j}");
                match rng.next() % 5 {
                    0 => Event::remove(&k, false, &idem),
                    1 if size == 1 => Event::remove("t", true, &idem),
                    _ => Event::upsert(&k, format!("w{round}-{j}").as_bytes(), &idem),
                }
            })
            .collect();
        let next = repo.db_append(states.last().unwrap(), &batch).unwrap();
        states.push(next);
    }
    for (i, a) in states.iter().enumerate() {
        for b in [
            states[(i * 7) % states.len()],
            states[(i + 1).min(states.len() - 1)],
            *a,
        ] {
            repo.prune_diff = false;
            let full = repo.graph_diff(&a.kv, &b.kv).unwrap();
            repo.prune_diff = true;
            assert_eq!(repo.graph_diff(&a.kv, &b.kv).unwrap(), full, "state {i}");
        }
    }
}
