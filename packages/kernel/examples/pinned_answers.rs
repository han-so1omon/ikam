//! E421: answers pinned to a commit, flagged stale when their data changes.
//!
//! `cargo run --release --example pinned_answers -- [ROWS [ANSWERS [CHANGED [SEED]]]]`
//!
//! Loads E382's table (`bm25_build`'s rows: a JSON array of a row number,
//! a date, one of 5,000 words, a float and a run of x's), commits it,
//! records ANSWERS answers at that commit (gets, sums over 10, 100 or
//! 1,000 rows, counts of a word or a date over the whole table, and
//! recorded answers that cannot be rerun), changes a share CHANGED of the rows (0.01) and
//! commits again. Then it flags the answers the change touches
//! (`answers_flag`) and reruns every pure answer at the new commit (a
//! full recheck), and prints both, the misses (changed, not flagged) and
//! the extra flags (flagged, unchanged), the time and rows read each way,
//! and the bytes an answer takes.

use std::collections::BTreeSet;
use std::time::Instant;

use ikam_kernel::answers::{Answer, AnswerKind, Query, ReadSet};
use ikam_kernel::{Event, MemStore, Repo, Store};

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

fn key(i: usize) -> String {
    format!("t/big/p{i:019}")
}

fn store_bytes(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.stored_len(id).unwrap())
        .sum()
}

/// The kind of an answer, for the tables: get, sum, count-word,
/// count-date, recorded.
fn kind_of(a: &Answer) -> &'static str {
    match (a.kind, Query::parse(&a.query)) {
        (AnswerKind::Recorded, _) => "recorded",
        (_, Some(Query::Get { .. })) => "get",
        (_, Some(Query::Sum { .. })) => "sum",
        (_, Some(Query::CountEq { field, .. })) if field == "2" => "count-word",
        _ => "count-date",
    }
}

fn main() {
    let arg = |n: usize| std::env::args().nth(n);
    let rows: usize = arg(1).map_or(200_000, |a| a.parse().unwrap());
    let n_answers: usize = arg(2).map_or(400, |a| a.parse().unwrap());
    let share: f64 = arg(3).map_or(0.01, |a| a.parse().unwrap());
    let seed: u64 = arg(4).map_or(1, |a| a.parse().unwrap());

    // E382's rows, as `bm25_build` writes them.
    let mut rng = Lcg(42);
    let mut repo = Repo::new(MemStore::default());
    let mut db = repo.db_empty().unwrap();
    let t = Instant::now();
    for start in (0..rows).step_by(10_000) {
        let events: Vec<Event> = (start..(start + 10_000).min(rows))
            .map(|i| {
                let v = format!(
                    "[{i}, \"2026-01-{:02}\", \"word{}\", {}.{:016}, \"{}\"]",
                    1 + i % 28,
                    rng.next() % 5000,
                    rng.next() % 1000,
                    rng.next(),
                    "x".repeat(40 + (rng.next() % 120) as usize)
                );
                Event::upsert(&key(i), v.as_bytes(), &format!("r{i}"))
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
    }
    let c1 = repo.db_commit("db/t", &db, "load").unwrap();
    let load_s = t.elapsed().as_secs_f64();

    // Answers: 40% gets, 25% sums over 1,000 rows of the float, 15%
    // counts of a word, 10% counts of a date, 10% recorded.
    let mut rng = Lcg(seed);
    let mut answers: Vec<(String, Answer)> = Vec::new();
    let before_bytes = store_bytes(&repo);
    let t = Instant::now();
    for j in 0..n_answers {
        let i = (rng.next() as usize) % rows;
        let slot = j % 20;
        let a = if slot < 18 {
            let q = match slot {
                0..8 => Query::Get { key: key(i) },
                8..13 => Query::Sum {
                    start: key(i),
                    end: key(i + [10, 100, 1_000][j % 3]),
                    field: "3".into(),
                },
                13..16 => Query::CountEq {
                    prefix: "t/big/".into(),
                    field: "2".into(),
                    value: format!("\"word{}\"", rng.next() % 5000).into_bytes(),
                },
                _ => Query::CountEq {
                    prefix: "t/big/".into(),
                    field: "1".into(),
                    value: format!("\"2026-01-{:02}\"", 1 + rng.next() % 28).into_bytes(),
                },
            };
            repo.answer_pure(&format!("question {j}"), &q, &c1, None)
                .unwrap()
        } else {
            // A recorded answer: what a model said about 500 rows' text;
            // its output is kept as given and never rerun.
            Answer {
                question: format!("question {j}: describe rows {i} to {}", i + 500),
                query: "a model's summary of the rows' text column".into(),
                kind: AnswerKind::Recorded,
                definitions: None,
                commit: c1,
                read: ReadSet {
                    ranges: vec![(key(i), key(i + 500))],
                    fields: Some(vec!["4".into()]),
                    filter: None,
                },
                result: format!("summary {}", rng.next()).into_bytes(),
            }
        };
        answers.push((format!("a{j:05}"), a));
    }
    let answer_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let set = repo.answers_put(&answers).unwrap();
    let put_s = t.elapsed().as_secs_f64();
    let set_bytes = store_bytes(&repo) - before_bytes;
    let record_bytes: usize = answers.iter().map(|(_, a)| a.encode().len()).sum();

    // Change a share of the rows: the float, the word, the run of x's, the
    // date, the same bytes again, deletes, and new rows between keys.
    let n_change = ((rows as f64) * share).round() as usize;
    let mut picked = BTreeSet::new();
    while picked.len() < n_change {
        picked.insert((rng.next() as usize) % rows);
    }
    let picked: Vec<usize> = picked.into_iter().collect();
    let olds = repo
        .db_get_many(&db, &picked.iter().map(|&i| key(i)).collect())
        .unwrap();
    let mut changes = Vec::new();
    let mut by_kind = [0usize; 7];
    for (n, (k, old)) in olds.into_iter().enumerate() {
        let s = String::from_utf8(old.clone()).unwrap();
        let mut f: Vec<String> = s
            .trim_matches(|c| c == '[' || c == ']')
            .split(", ")
            .map(str::to_string)
            .collect();
        let id = format!("change{n}");
        let pick = (rng.next() % 40) as usize;
        let kind = match pick {
            0..14 => 0,  // float, 35%
            14..22 => 1, // word, 20%
            22..28 => 2, // x's, 15%
            28..30 => 3, // date, 5%
            30..34 => 4, // same bytes, 10%
            34..37 => 5, // delete, 7.5%
            _ => 6,      // a new row between keys, 7.5%
        };
        by_kind[kind] += 1;
        match kind {
            0 => f[3] = format!("{}.{:016}", rng.next() % 1000, rng.next()),
            1 => f[2] = format!("\"word{}\"", rng.next() % 5000),
            2 => f[4] = format!("\"{}\"", "x".repeat(40 + (rng.next() % 120) as usize)),
            3 => f[1] = format!("\"2026-01-{:02}\"", 1 + rng.next() % 28),
            _ => {}
        }
        let v = format!("[{}]", f.join(", "));
        changes.push(match kind {
            4 => Event::upsert(&k, &old, &id),
            5 => Event::remove(&k, false, &id),
            6 => Event::upsert(&format!("{k}n"), v.as_bytes(), &id),
            _ => Event::upsert(&k, v.as_bytes(), &id),
        });
    }
    let db2 = repo.db_append(&db, &changes).unwrap();
    let c2 = repo.db_commit("db/t", &db2, "change").unwrap();

    // Flags from the read sets.
    let t = Instant::now();
    let flagged = repo.answers_flag(&set, &c1, &c2).unwrap();
    let flag_s = t.elapsed().as_secs_f64();
    let flags: BTreeSet<String> = flagged.stale.iter().cloned().collect();

    // A full recheck: every pure answer rerun at the new commit.
    let rows_read = |repo: &Repo<MemStore>, a: &Answer, db: &ikam_kernel::Db| -> usize {
        match Query::parse(&a.query) {
            Some(Query::Get { .. }) => 1,
            Some(Query::Sum { start, end, .. }) => repo.db_count(db, &start, Some(&end)).unwrap(),
            Some(Query::CountEq { prefix, .. }) => repo
                .db_count(db, &prefix, ikam_kernel::prefix_end(&prefix).as_deref())
                .unwrap(),
            None => 0,
        }
    };
    let t = Instant::now();
    let mut changed = BTreeSet::new();
    for (name, a) in &answers {
        if repo.answer_recheck(a, &c2).unwrap() == Some(false) {
            changed.insert(name.clone());
        }
    }
    let full_s = t.elapsed().as_secs_f64();
    let full_rows: usize = answers.iter().map(|(_, a)| rows_read(&repo, a, &db2)).sum();

    // Rerun only the flagged pure answers.
    let t = Instant::now();
    let mut confirmed = BTreeSet::new();
    for (name, a) in answers.iter().filter(|(n, _)| flags.contains(n)) {
        if repo.answer_recheck(a, &c2).unwrap() == Some(false) {
            confirmed.insert(name.clone());
        }
    }
    let confirm_s = t.elapsed().as_secs_f64();
    let confirm_rows: usize = answers
        .iter()
        .filter(|(n, _)| flags.contains(n))
        .map(|(_, a)| rows_read(&repo, a, &db2))
        .sum();

    // Every pure answer reruns exactly at its own commit.
    let t = Instant::now();
    let exact = answers
        .iter()
        .filter(|(_, a)| repo.answer_recheck(a, &c1).unwrap() == Some(true))
        .count();
    let exact_s = t.elapsed().as_secs_f64();

    let kinds = ["get", "sum", "count-word", "count-date", "recorded"];
    let mut per = String::new();
    for k in kinds {
        let names: Vec<&String> = answers
            .iter()
            .filter(|(_, a)| kind_of(a) == k)
            .map(|(n, _)| n)
            .collect();
        let fl = names.iter().filter(|n| flags.contains(**n)).count();
        let ch = names.iter().filter(|n| changed.contains(**n)).count();
        let miss = names
            .iter()
            .filter(|n| changed.contains(**n) && !flags.contains(**n))
            .count();
        let extra = names
            .iter()
            .filter(|n| flags.contains(**n) && !changed.contains(**n))
            .count();
        let sep = if per.is_empty() { "" } else { "," };
        if k == "recorded" {
            // Not rerun: only its flags are known.
            per.push_str(&format!(
                "{sep}\"{k}\":{{\"answers\":{},\"flagged\":{fl}}}",
                names.len()
            ));
            continue;
        }
        per.push_str(&format!(
            "{sep}\"{k}\":{{\"answers\":{},\"flagged\":{fl},\"changed\":{ch},\"missed\":{miss},\"extra\":{extra}}}",
            names.len()
        ));
    }
    let pure = answers
        .iter()
        .filter(|(_, a)| a.kind == AnswerKind::Pure)
        .count();
    let pure_flags: BTreeSet<String> = flags
        .iter()
        .filter(|n| {
            answers
                .iter()
                .any(|(m, a)| m == *n && a.kind == AnswerKind::Pure)
        })
        .cloned()
        .collect();
    let missed = changed.difference(&pure_flags).count();
    let extra = pure_flags.difference(&changed).count();
    println!(
        "{{\"rows\":{rows},\"answers\":{n_answers},\"pure\":{pure},\"seed\":{seed},\"rows_changed_events\":{},\"change_kinds\":{by_kind:?},\"rows_changed\":{},\"load_s\":{load_s:.2},\"answer_ms_each\":{:.3},\"put_ms\":{:.2},\"set_bytes\":{set_bytes},\"bytes_per_answer\":{:.1},\"record_bytes_per_answer\":{:.1},\"flag_ms\":{:.2},\"flagged\":{},\"full_recheck_ms\":{:.1},\"full_rows_read\":{full_rows},\"changed\":{},\"missed\":{missed},\"extra\":{extra},\"confirm_ms\":{:.1},\"confirm_rows_read\":{confirm_rows},\"confirmed_equal_full\":{},\"rerun_exact_at_own_commit\":{exact},\"rerun_own_ms\":{:.1},\"by_kind\":{{{per}}}}}",
        changes.len(),
        flagged.rows_changed,
        answer_s * 1000.0 / n_answers as f64,
        put_s * 1000.0,
        set_bytes as f64 / n_answers as f64,
        record_bytes as f64 / n_answers as f64,
        flag_s * 1000.0,
        flags.len(),
        full_s * 1000.0,
        changed.len(),
        confirm_s * 1000.0,
        confirmed == changed,
        exact_s * 1000.0,
    );
}
