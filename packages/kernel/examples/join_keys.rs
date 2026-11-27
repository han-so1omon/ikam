//! E420: join keys proposed by name and confirmed against the data.
//!
//! `cargo run --release --example join_keys -- ROWS.tsv DECLARED.tsv [MIN_SCORE [OUT.json]]`
//!
//! ROWS.tsv and DECLARED.tsv as `experiments/join-keys/tpcds_gen.py`
//! writes them (table, row key, JSON object; and the declared foreign
//! keys). Loads every row into one database (`tpcds/TABLE/KEY`), builds
//! each table's BM25 view with its column statistics and zone maps (E403,
//! E405), proposes pairs by name (`joinkeys::propose`, names only: no
//! embedder weights here) and confirms each against the data. Prints
//! precision and recall against the declared single-column keys, the
//! pairs kept and missed with the reason, the time per table, and two
//! baselines: the names alone (no data check) and the data alone (every
//! pair of columns of two tables, no name check).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use ikam_kernel::colstats::TableStats;
use ikam_kernel::joinkeys::{Candidate, Rejected, TableInfo, ValueCache, markers, propose};
use ikam_kernel::{Event, MemStore, Repo};

fn pair(c: &Candidate) -> (String, String, String, String) {
    let t = |s: &str| s.strip_prefix("tpcds/").unwrap_or(s).to_string();
    (
        t(&c.child_table),
        c.child.clone(),
        t(&c.parent_table),
        c.parent.clone(),
    )
}

type Pair = (String, String, String, String);

fn score(kept: &BTreeSet<Pair>, declared: &BTreeSet<Pair>) -> (usize, usize, f64, f64) {
    let tp = kept.intersection(declared).count();
    let p = if kept.is_empty() {
        1.0
    } else {
        tp as f64 / kept.len() as f64
    };
    (tp, kept.len(), p, tp as f64 / declared.len() as f64)
}

fn main() {
    let arg = |n: usize| std::env::args().nth(n);
    let rows_path = arg(1).expect("ROWS.tsv");
    let declared_path = arg(2).expect("DECLARED.tsv");
    let min_score: f64 = arg(3).map_or(1.0, |a| a.parse().unwrap());

    let text = std::fs::read_to_string(&rows_path).unwrap();
    let mut repo = Repo::new(MemStore::default());
    let mut db = repo.db_empty().unwrap();
    let mut tables: Vec<String> = Vec::new();
    let t = Instant::now();
    let mut batch = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut it = line.splitn(3, '\t');
        let (table, key, json) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        if tables.last().map(String::as_str) != Some(table) {
            tables.push(table.to_string());
        }
        batch.push(Event::upsert(
            &format!("tpcds/{table}/{key}"),
            json.as_bytes(),
            &format!("r{n}"),
        ));
        if batch.len() == 10_000 {
            db = repo.db_append(&db, &batch).unwrap();
            batch.clear();
        }
    }
    if !batch.is_empty() {
        db = repo.db_append(&db, &batch).unwrap();
    }
    let load_s = t.elapsed().as_secs_f64();
    drop(text);

    let mut declared = BTreeSet::new();
    let mut composite = Vec::new();
    for l in std::fs::read_to_string(&declared_path).unwrap().lines() {
        let f: Vec<&str> = l.split('\t').collect();
        let p = (f[0].into(), f[1].into(), f[2].into(), f[3].into());
        if f[1].contains(',') {
            composite.push(p);
        } else {
            declared.insert(p);
        }
    }

    // Stats and zone maps from each table's BM25 view build.
    let mut stats: BTreeMap<String, TableStats> = BTreeMap::new();
    let mut build_s: BTreeMap<String, f64> = BTreeMap::new();
    for t in &tables {
        let prefix = format!("tpcds/{t}/");
        let mut s = TableStats::with_zones(4096);
        let t0 = Instant::now();
        repo.bm25_view_build_with_stats(&db, &prefix, &mut s)
            .unwrap();
        build_s.insert(t.clone(), t0.elapsed().as_secs_f64());
        stats.insert(format!("tpcds/{t}"), s);
    }
    let infos: Vec<TableInfo> = tables
        .iter()
        .map(|t| TableInfo {
            table: format!("tpcds/{t}"),
            name: t.clone(),
            stats: &stats[&format!("tpcds/{t}")],
        })
        .collect();

    let mut marks: Vec<String> = markers(&infos).into_iter().collect();
    marks.sort();
    let t0 = Instant::now();
    let cands = propose(&infos, min_score, None);
    let propose_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let mut cache = ValueCache::new();
    let mut confirm_s: BTreeMap<String, f64> = BTreeMap::new();
    let mut kept = Vec::new();
    let mut rejected: Vec<(Candidate, Rejected)> = Vec::new();
    for c in &cands {
        let t0 = Instant::now();
        let r = repo.join_key_confirm(&db, c, &stats, &mut cache).unwrap();
        *confirm_s.entry(pair(c).0).or_default() += t0.elapsed().as_secs_f64();
        match r {
            Ok(e) => kept.push((c.clone(), e)),
            Err(why) => rejected.push((c.clone(), why)),
        }
    }
    let keys_root = repo.put_join_keys(&kept).unwrap();

    let kept_set: BTreeSet<Pair> = kept.iter().map(|(c, _)| pair(c)).collect();
    let named_set: BTreeSet<Pair> = cands.iter().map(pair).collect();
    let (tp, n_kept, precision, recall) = score(&kept_set, &declared);
    let (ntp, n_named, nprec, nrec) = score(&named_set, &declared);

    // The data alone: every pair of columns in two tables, no name check.
    let mut all = Vec::new();
    for c in &infos {
        for p in &infos {
            if c.table == p.table {
                continue;
            }
            for cc in c.stats.columns.keys() {
                for pc in p.stats.columns.keys() {
                    all.push(Candidate {
                        child_table: c.table.clone(),
                        child: cc.clone(),
                        parent_table: p.table.clone(),
                        parent: pc.clone(),
                        name_score: 0.0,
                    });
                }
            }
        }
    }
    let t0 = Instant::now();
    let mut data_kept = BTreeSet::new();
    let mut by_stats = 0usize;
    for c in &all {
        match repo.join_key_confirm(&db, c, &stats, &mut cache).unwrap() {
            Ok(_) => {
                data_kept.insert(pair(c));
            }
            Err(Rejected::NotContained(_) | Rejected::EmptyJoin) => {}
            Err(_) => by_stats += 1,
        }
    }
    let data_s = t0.elapsed().as_secs_f64();
    let (dtp, n_data, dprec, drec) = score(&data_kept, &declared);

    let show = |p: &Pair| format!("{}.{} > {}.{}", p.0, p.1, p.2, p.3);
    let mut table_times = String::new();
    for t in &tables {
        table_times.push_str(&format!(
            "{}\"{t}\":{{\"rows\":{},\"stats_build_s\":{:.3},\"confirm_s\":{:.3}}}",
            if table_times.is_empty() { "" } else { "," },
            stats[&format!("tpcds/{t}")].rows,
            build_s[t],
            confirm_s.get(t).copied().unwrap_or(0.0)
        ));
    }
    let list = |v: Vec<String>| {
        v.iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(",")
    };
    let false_pos: Vec<String> = kept_set.difference(&declared).map(show).collect();
    let missed: Vec<String> = declared
        .difference(&kept_set)
        .map(|p| {
            let why = rejected
                .iter()
                .find(|(c, _)| &pair(c) == p)
                .map_or("not proposed".to_string(), |(_, r)| format!("{r:?}"));
            format!("{} ({why})", show(p))
        })
        .collect();
    let rejected_named: Vec<String> = rejected
        .iter()
        .filter(|(c, _)| declared.contains(&pair(c)))
        .map(|(c, r)| format!("{} {r:?}", show(&pair(c))))
        .collect();
    let mut why_counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, r) in &rejected {
        let k = format!("{r:?}");
        let k = k.split('(').next().unwrap().to_string();
        *why_counts.entry(k).or_default() += 1;
    }
    let data_fp: Vec<String> = data_kept.difference(&declared).map(show).collect();
    println!(
        "{{\"rows\":{},\"tables\":{},\"load_s\":{load_s:.2},\"min_score\":{min_score},\"markers\":{marks:?},\"declared\":{},\"composite_declared\":{},\"proposed\":{n_named},\"kept\":{n_kept},\"true_kept\":{tp},\"precision\":{precision:.3},\"recall\":{recall:.3},\"propose_ms\":{propose_ms:.2},\"rejected\":{why_counts:?},\"names_only\":{{\"proposed\":{n_named},\"true\":{ntp},\"precision\":{nprec:.3},\"recall\":{nrec:.3}}},\"data_only\":{{\"pairs\":{},\"ruled_out_by_stats\":{by_stats},\"kept\":{n_data},\"true\":{dtp},\"precision\":{dprec:.3},\"recall\":{drec:.3},\"s\":{data_s:.2},\"false\":[{}]}},\"false_positives\":[{}],\"missed\":[{}],\"declared_rejected\":[{}],\"keys_root\":\"{keys_root}\",\"per_table\":{{{table_times}}}}}",
        stats.values().map(|s| s.rows).sum::<u64>(),
        tables.len(),
        declared.len(),
        composite.len(),
        all.len(),
        list(data_fp),
        list(false_pos),
        list(missed),
        list(rejected_named),
    );
}
