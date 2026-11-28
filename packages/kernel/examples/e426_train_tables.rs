//! E426: point-in-time training tables pinned to commits, at 200k rows.
//!
//! A synthetic database of ROWS integer rows under `t/` over COMMITS
//! commits: commit 0 writes every row; after it a hot fifth of the rows
//! (`i % 5 == 0`) changes with probability 0.2 a commit and the rest with
//! 0.01, a further 0.01 of rows is written again with the same value (an
//! event that is not a change), and rows with `i % 1000 == 7` are removed
//! at commit 20 and written again at 30. For each horizon N the example
//! builds the table "will this row change within N commits?" observed
//! every EVERY commits, then checks it: the same ids on a rebuild (the run
//! and the builtin both evaluated again, not memo hits), SAMPLE rows read
//! back against `db_at` of their commit and the commits after, fsck, and
//! fsck_deep, and with `repack` a repack (bytes dropped, then fsck
//! evaluating the tables again; slow on a large store).
//!
//! `cargo run --release --example e426_train_tables -- [ROWS] [COMMITS] [EVERY] [SAMPLE] [repack]`
//!
//! Prints one JSON line per step.
use std::collections::BTreeMap;
use std::time::Instant;

use ikam_kernel::train::{self, History, TrainRule};
use ikam_kernel::{Event, Id, MemStore, Repo, Store, func};

fn rnd(mut s: u64) -> u64 {
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    s
}

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

/// The events of commit `c` given the current values (updated in place).
fn commit_events(c: u64, rows: u64, values: &mut [Option<u64>]) -> Vec<Event> {
    let mut out = Vec::new();
    for i in 0..rows {
        let key = format!("t/{i:07}");
        let r = rnd((c << 40) ^ (i + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let p = (r >> 11) % 10_000; // in 1/10,000
        let rate = if i.is_multiple_of(5) { 2_000 } else { 100 };
        let slot = &mut values[i as usize];
        let new = match *slot {
            None if c == 0 || (c == 30 && i % 1000 == 7) => Some(r % 1_000_000),
            None => None,
            Some(_) if c == 20 && i % 1000 == 7 => {
                *slot = None;
                out.push(Event::remove(&key, false, &format!("{c}/{i}/rm")));
                continue;
            }
            Some(v) if p < rate => Some(v + 1 + r % 997),
            Some(v) if p < rate + 100 => Some(v), // written again, unchanged
            Some(_) => None,
        };
        if let Some(v) = new {
            *slot = Some(v);
            out.push(Event::upsert(
                &key,
                v.to_string().as_bytes(),
                &format!("{c}/{i}"),
            ));
        }
    }
    out
}

/// The probability a row is labelled 1 at a horizon, ignoring removals.
fn expected_rate(horizon: i32) -> f64 {
    0.2 * (1.0 - 0.8f64.powi(horizon)) + 0.8 * (1.0 - 0.99f64.powi(horizon))
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, d: u64| a.get(i).map_or(d, |s| s.parse().unwrap());
    let (rows, commits, every, sample) = (arg(0, 200_000), arg(1, 50), arg(2, 10), arg(3, 2_000));
    let do_repack = a.iter().any(|s| s == "repack");

    let mut repo = Repo::new(MemStore::default());
    let mut db = repo.db_empty().unwrap();
    let mut values = vec![None; rows as usize];
    let mut chain = Vec::new();
    let (t0, mut events) = (Instant::now(), 0usize);
    for c in 0..commits {
        let ev = commit_events(c, rows, &mut values);
        events += ev.len();
        for batch in ev.chunks(20_000) {
            db = repo.db_append_deferred(&db, batch).unwrap();
        }
        db = repo.db_catch_up(&db).unwrap();
        chain.push(repo.db_commit("db/t", &db, "c").unwrap());
    }
    let tip = *chain.last().unwrap();
    let store_bytes = |r: &Repo<MemStore>| -> usize {
        let s = r.store();
        s.ids()
            .unwrap()
            .iter()
            .map(|id| s.stored_len(id).unwrap())
            .sum()
    };
    println!(
        "{{\"step\":\"load\",\"rows\":{rows},\"commits\":{commits},\"events\":{events},\"s\":{:.1},\"store_bytes\":{}}}",
        secs(t0),
        store_bytes(&repo)
    );

    let mut tables = Vec::new();
    for horizon in [1u32, 5] {
        let rule = TrainRule {
            prefix: "t/".into(),
            horizon,
            every: every as u32,
        };
        let pin = format!("train/n{horizon}");
        let t = Instant::now();
        let history = repo.train_history(&tip, &rule).unwrap();
        let run_s = secs(t);
        let grown = store_bytes(&repo) as i64;
        let t = Instant::now();
        let table = repo.train_table(&pin, &tip, &rule).unwrap();
        let total_s = secs(t);
        let grown = store_bytes(&repo) as i64 - grown;
        let (rule_id, history_id) = repo.train_inputs(&table).unwrap();
        assert_eq!(Id::of_content(&history), history_id);
        let bytes = repo.read_content(&table).unwrap();
        let t = Instant::now();
        let (rebuilt, reads) = train::build(&rule.encode(), &history).unwrap();
        let build_s = secs(t);
        let rows_t = train::decode_table(&bytes).unwrap();
        let ones = rows_t.iter().filter(|r| r.label).count();

        // Rebuild: the run again, the builtin evaluated again (no memo),
        // and train_table again (its memo).
        let again = repo.train_history(&tip, &rule).unwrap();
        let args = [
            ikam_kernel::Arg::Whole(rule_id),
            ikam_kernel::Arg::Whole(history_id),
        ];
        let (evaluated, _) = repo.evaluate(&func::train_table(), &args).unwrap();
        let same = Id::of_content(&again) == history_id
            && Id::of_content(&rebuilt) == table
            && Id::of_content(&evaluated) == table
            && repo.train_table(&pin, &tip, &rule).unwrap() == table;
        // Stored as itself, or -1 when ingest planned it from other content.
        let stored = |id: &Id| repo.store().stored_len(id).map_or(-1, |n| n as i64);
        println!(
            "{{\"step\":\"table\",\"horizon\":{horizon},\"every\":{every},\"table_rows\":{},\"label_1\":{ones},\
             \"label_1_share\":{:.4},\"expected_share\":{:.4},\"history_run_s\":{run_s:.2},\"build_s\":{build_s:.2},\
             \"train_table_s\":{total_s:.2},\"store_grew\":{grown},\"table_bytes\":{},\"table_stored\":{},\
             \"history_bytes\":{},\"history_stored\":{},\"feature_reads\":{},\"past_obs_reads\":{},\"label_reads\":{},\"rebuild_same_ids\":{same},\
             \"table\":\"{table}\"}}",
            rows_t.len(),
            ones as f64 / rows_t.len() as f64,
            expected_rate(horizon as i32),
            bytes.len(),
            stored(&table),
            history.len(),
            stored(&history_id),
            reads.feature,
            reads.past_obs,
            reads.label,
        );
        tables.push((horizon, table, bytes, rows_t, history_id));
    }

    // Point-in-time check against the database itself: SAMPLE rows of
    // each table, their features read at their commit, their label from
    // the commits after it.
    for (horizon, _, _, rows_t, history_id) in &tables {
        let h = History::decode(&repo.read_content(history_id).unwrap()).unwrap();
        assert_eq!(h.commits, chain);
        let t = Instant::now();
        let mut by_obs: BTreeMap<u32, Vec<&train::Row>> = BTreeMap::new();
        let step = (rows_t.len() / sample as usize).max(1);
        for r in rows_t.iter().step_by(step) {
            by_obs.entry(r.obs).or_default().push(r);
        }
        let mut dbs = BTreeMap::new();
        let (mut checked, mut wrong) = (0, 0);
        for (obs, rs) in by_obs {
            for k in obs..=obs + horizon {
                dbs.entry(k)
                    .or_insert_with(|| repo.db_at(&chain[k as usize]).unwrap());
            }
            for r in rs {
                let key = &h.keys[r.row as usize];
                let at = repo.db_get(&dbs[&obs], key).unwrap().unwrap();
                let v: i64 = std::str::from_utf8(&at).unwrap().parse().unwrap();
                let label = (obs + 1..=obs + horizon)
                    .any(|k| repo.db_get(&dbs[&k], key).unwrap().as_ref() != Some(&at));
                checked += 1;
                wrong += usize::from(v != r.value || label != r.label);
            }
        }
        println!(
            "{{\"step\":\"db_at_check\",\"horizon\":{horizon},\"rows_checked\":{checked},\"mismatches\":{wrong},\"s\":{:.1}}}",
            secs(t)
        );
    }

    let t = Instant::now();
    let failures = repo.fsck().unwrap().len();
    println!(
        "{{\"step\":\"fsck\",\"failures\":{failures},\"s\":{:.1}}}",
        secs(t)
    );
    // fsck_deep evaluates every derivation, the tables' included.
    let t = Instant::now();
    let failures = repo.fsck_deep().unwrap().len();
    println!(
        "{{\"step\":\"fsck_deep\",\"failures\":{failures},\"s\":{:.1}}}",
        secs(t)
    );
    if !do_repack {
        return;
    }
    let before = store_bytes(&repo);
    let t = Instant::now();
    let packed = repo.repack().unwrap();
    let repack_s = secs(t);
    let dropped: Vec<bool> = tables
        .iter()
        .map(|(_, id, ..)| !repo.store().has(id))
        .collect();
    let t = Instant::now();
    let failures = repo.fsck().unwrap().len();
    let fsck_s = secs(t);
    let t = Instant::now();
    let same = tables
        .iter()
        .all(|(_, id, bytes, ..)| repo.read_content(id).unwrap() == *bytes);
    let read_s = secs(t);
    let pins_ok = tables.iter().all(|(h, ..)| {
        let pin = repo.resolve(&format!("train/n{h}")).unwrap();
        repo.train_verify(&pin).is_ok()
    });
    println!(
        "{{\"step\":\"repack\",\"applied\":{},\"store_before\":{before},\"store_after\":{},\"s\":{repack_s:.1},\
         \"tables_dropped\":{dropped:?},\"fsck_failures\":{failures},\"fsck_s\":{fsck_s:.1},\"read_back_equal\":{same},\
         \"read_s\":{read_s:.2},\"pins_verify\":{pins_ok}}}",
        packed.applied,
        store_bytes(&repo)
    );
}
