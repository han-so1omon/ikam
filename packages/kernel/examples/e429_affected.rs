//! E429: rebuild only what a change reaches, from a database's rows to a
//! training table (E426) and to sampled batches (E427).
//!
//! A synthetic database of ROWS integer rows over COMMITS commits (E426's
//! generator) is loaded three times into one store: as generated (`a`),
//! and with 1% of its rows corrected (every value of a corrected row from a
//! commit of its own on raised by 1,000,000), once for rows spread at
//! random (`random`) and once for a block of consecutive rows (`block`).
//! For each corrected database:
//!
//! - Table: the table "will this row change within HORIZON commits?",
//!   observed every EVERY commits, in units of UNIT (`row`, `prefix N`,
//!   `table`), built after `a`'s (units found recorded are reused), then
//!   built again with every unit evaluated; same manifest id, units and
//!   rows rebuilt, times.
//! - Batches: the database's tip exported as files whose fragments are its
//!   rows (`range/`: 32 consecutive keys a file; a second layout, `hash/`:
//!   32 keys by hash a file, or `window/`: 32 consecutive keys offset by 16),
//!   the view numbered by paths, batches of 2-hop (or 1-hop) fanout-10
//!   neighborhoods for one epoch over every node. The changed rows are
//!   propagated into the export (`propagate`), the view updated from the
//!   old one, and the batches sampled again where they hold a changed node;
//!   then the export, the view and every batch built again in full; same
//!   ids, batches rebuilt, times.
//!
//! `cargo run --release --example e429_affected -- [ROWS] [COMMITS] [EVERY] [HORIZON] [UNIT...] [deep]`
//!
//! UNIT is one or more of `row`, `prefix:N`, `table`, `glob:PATTERN`
//! (E444; default `row`). `nobatch` leaves out the batches, `plain` writes
//! contents as they are, `deep` adds `fsck_deep`.
//! Prints one JSON line per step.
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use ikam_kernel::lineage::{BatchIndex, Delta, Unit, units_reached, view_delta};
use ikam_kernel::sampler::{Order, Spec, Targets};
use ikam_kernel::train::{self, History, TrainRule};
use ikam_kernel::{Edit, Event, Id, MemStore, Op, Repo};

fn rnd(mut s: u64) -> u64 {
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    s
}

fn mix(i: u64, salt: u64) -> u64 {
    rnd((i + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ salt)
}

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

/// E426's generator: the events of commit `c` given the current values.
fn commit_events(c: u64, rows: u64, values: &mut [Option<u64>]) -> Vec<Event> {
    let mut out = Vec::new();
    for i in 0..rows {
        let key = format!("t/{i:07}");
        let r = rnd((c << 40) ^ (i + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let p = (r >> 11) % 10_000;
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
            Some(v) if p < rate + 100 => Some(v),
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

const LIFT: u64 = 1_000_000;

/// `base`'s events of commit `c` with the rows in `fixed` corrected from
/// their own commit on: values raised by LIFT, and one write at that commit
/// when the row had none.
fn corrected(
    c: u64,
    base: &[Event],
    values: &[Option<u64>],
    fixed: &BTreeMap<u64, u64>,
) -> Vec<Event> {
    let row = |k: &str| k[2..].parse::<u64>().unwrap();
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(base.len());
    for e in base {
        let i = row(&e.key);
        match (&e.op, fixed.get(&i)) {
            (Op::Upsert(v), Some(&from)) if c >= from => {
                seen.insert(i);
                let v: u64 = std::str::from_utf8(v).unwrap().parse().unwrap();
                out.push(Event::upsert(
                    &e.key,
                    (v + LIFT).to_string().as_bytes(),
                    &e.idempotency_key,
                ));
            }
            _ => out.push(e.clone()),
        }
    }
    for (&i, &from) in fixed {
        if from == c
            && !seen.contains(&i)
            && let Some(v) = values[i as usize]
        {
            out.push(Event::upsert(
                &format!("t/{i:07}"),
                (v + LIFT).to_string().as_bytes(),
                &format!("{c}/{i}/fix"),
            ));
        }
    }
    out
}

fn load(repo: &mut Repo<MemStore>, name: &str, per_commit: &[Vec<Event>]) -> Vec<Id> {
    let mut db = repo.db_empty().unwrap();
    let mut chain = Vec::new();
    for ev in per_commit {
        for batch in ev.chunks(20_000) {
            db = repo.db_append_deferred(&db, batch).unwrap();
        }
        db = repo.db_catch_up(&db).unwrap();
        chain.push(repo.db_commit(name, &db, "c").unwrap());
    }
    chain
}

fn tip_values(repo: &mut Repo<MemStore>, tip: &Id) -> BTreeMap<String, String> {
    let db = repo.db_at(tip).unwrap();
    repo.db_prefix(&db, "t/", usize::MAX, false)
        .unwrap()
        .into_iter()
        .map(|(k, v)| (k, String::from_utf8(v).unwrap()))
        .collect()
}

/// The second layout's file for row `i`.
fn second(layout: &str, i: u64, files: u64) -> String {
    match layout {
        "hash" => format!("hash/{:05}.csv", mix(i, 77) % files),
        _ => format!("window/{:05}.csv", (i + 16) / 32),
    }
}

/// The rows as fragments of two exports of 32 rows a file.
fn export(repo: &mut Repo<MemStore>, values: &BTreeMap<String, String>, layout: &str) -> Id {
    let rows = values.len() as u64;
    let mut files: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, v) in values {
        let i: u64 = k[2..].parse().unwrap();
        let line = format!("{k},{v}\n");
        let range = format!("range/{:05}.csv", i / 32);
        files.entry(range).or_default().push(line.clone());
        files
            .entry(second(layout, i, rows.div_ceil(32)))
            .or_default()
            .push(line);
    }
    let mut edits = Vec::with_capacity(files.len());
    for (path, lines) in files {
        let bytes = lines.concat().into_bytes();
        let mut cuts = Vec::with_capacity(lines.len());
        let mut at = 0;
        for l in &lines {
            cuts.push(at..at + l.len());
            at += l.len();
        }
        let id = repo.put_cut(&bytes, &cuts, None).unwrap();
        edits.push(Edit::Put { path, id });
    }
    repo.edit_tree(None, &edits).unwrap()
}

fn specs(nodes: usize, hops: u32, batch: u32) -> Vec<Spec> {
    let n = (nodes as u32).div_ceil(batch);
    (0..n)
        .map(|index| Spec {
            seed: 429,
            fanout: 10,
            hops,
            batch,
            index,
            targets: Targets::All,
        })
        .collect()
}

fn unit_of(s: &str) -> Unit {
    match s.split_once(':') {
        Some(("prefix", n)) => Unit::Prefix(n.parse().unwrap()),
        Some(("glob", p)) => Unit::Pattern(p.to_string()),
        _ => Unit::parse(s).unwrap(),
    }
}

const CONFIGS: [(u32, u32); 4] = [(2, 256), (2, 64), (2, 16), (1, 16)];
const LAYOUTS: [&str; 2] = ["hash", "window"];

/// What a full build gives: per unit the manifest and its time; per layout
/// the tree, the view and, per config, the batches.
#[derive(Default)]
struct Built {
    manifests: BTreeMap<String, (Id, usize, u64, f64)>,
    trees: BTreeMap<&'static str, (Id, Id, f64, f64)>,
    batches: BTreeMap<(&'static str, u32, u32), (Vec<Id>, f64)>,
}

/// Everything built from scratch in `repo` for the database at `tip`.
fn build_all(
    repo: &mut Repo<MemStore>,
    tag: &str,
    tip: &Id,
    rule: &TrainRule,
    units: &[Unit],
    batches: bool,
) -> Built {
    let mut out = Built::default();
    for (u, unit) in units.iter().enumerate() {
        let b = repo
            .train_table_units(&format!("units/{tag}/{u}"), tip, rule, unit, false)
            .unwrap();
        out.manifests.insert(
            unit.text(),
            (
                b.manifest,
                b.evaluated.len(),
                b.rows_evaluated,
                b.cut_s + b.build_s,
            ),
        );
    }
    if !batches {
        return out;
    }
    let values = tip_values(repo, tip);
    for layout in LAYOUTS {
        let t = Instant::now();
        let tree = export(repo, &values, layout);
        let export_s = secs(t);
        let c = repo
            .commit(&format!("exp/{layout}/{tag}"), tree, tag)
            .unwrap();
        let t = Instant::now();
        let (view, _) = repo.record_fragment_view_ordered(&c, Order::Paths).unwrap();
        out.trees.insert(layout, (tree, view, export_s, secs(t)));
        let nodes = ikam_kernel::sampler::View::decode(&repo.read_content(&view).unwrap())
            .unwrap()
            .nodes();
        for (hops, batch) in CONFIGS {
            let t = Instant::now();
            let ids = specs(nodes, hops, batch)
                .iter()
                .map(|s| repo.apply_sample(view, s).unwrap())
                .collect();
            out.batches.insert((layout, hops, batch), (ids, secs(t)));
        }
    }
    out
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize, d: u64| a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let (rows, commits, every, horizon) = (num(0, 50_000), num(1, 40), num(2, 5), num(3, 3));
    let deep = a.iter().any(|s| s == "deep");
    let batches = !a.iter().any(|s| s == "nobatch");
    // `plain`: every store writes new contents as they are (E261's
    // plain_hot_writes), without the slice search.
    let plain = a.iter().any(|s| s == "plain");
    let mut units: Vec<Unit> = a
        .iter()
        .skip(4)
        .filter(|s| !["deep", "nobatch", "plain"].contains(&s.as_str()))
        .map(|s| unit_of(s))
        .collect();
    if units.is_empty() {
        units.push(Unit::Row);
    }
    let rule = TrainRule {
        prefix: "t/".into(),
        horizon: horizon as u32,
        every: every as u32,
    };

    // The corrections: 1% of rows each, from a commit of their own.
    let share = rows / 100;
    let start = rows / 3;
    let block: BTreeMap<u64, u64> = (start..start + share)
        .map(|i| (i, 1 + mix(i, 2) % (commits - 1)))
        .collect();
    let mut random = BTreeMap::new();
    let mut k = 0u64;
    while (random.len() as u64) < share {
        let i = mix(k, 1) % rows;
        if !block.contains_key(&i) {
            random.insert(i, 1 + mix(i, 3) % (commits - 1));
        }
        k += 1;
    }
    let mut values = vec![None; rows as usize];
    let (mut base, mut fix_r, mut fix_b) = (Vec::new(), Vec::new(), Vec::new());
    for c in 0..commits {
        let ev = commit_events(c, rows, &mut values);
        fix_r.push(corrected(c, &ev, &values, &random));
        fix_b.push(corrected(c, &ev, &values, &block));
        base.push(ev);
    }
    let evs: BTreeMap<&str, &Vec<Vec<Event>>> =
        [("a", &base), ("random", &fix_r), ("block", &fix_b)].into();
    let mut repo = Repo::new(MemStore::default());
    repo.plain_hot_writes = plain;
    let mut tips = BTreeMap::new();
    for (name, evs) in &evs {
        let t = Instant::now();
        let chain = load(&mut repo, &format!("db/{name}"), evs);
        let events: usize = evs.iter().map(Vec::len).sum();
        println!(
            "{{\"step\":\"load\",\"db\":\"{name}\",\"rows\":{rows},\"commits\":{commits},\"events\":{events},\"corrected_rows\":{share},\"s\":{:.1}}}",
            secs(t)
        );
        tips.insert(*name, *chain.last().unwrap());
    }

    // E426's table, built at once, for scale.
    let t = Instant::now();
    let whole = repo.train_table("train/a", &tips["a"], &rule).unwrap();
    let whole_rows = ikam_kernel::lineage::table_rows(&repo.read_content(&whole).unwrap());
    println!(
        "{{\"step\":\"whole_table\",\"rows\":{whole_rows},\"s\":{:.2}}}",
        secs(t)
    );

    // The first build, of `a`: every unit, export, view and batch.
    let first = build_all(&mut repo, "a", &tips["a"], &rule, &units, batches);
    for (name, (_, evaluated, rows_e, s)) in &first.manifests {
        println!(
            "{{\"step\":\"units_first\",\"unit\":\"{name}\",\"units\":{evaluated},\"rows\":{rows_e},\"s\":{s:.2}}}"
        );
    }
    let history = |repo: &mut Repo<MemStore>, tip: &Id| {
        History::decode(&repo.train_history(tip, &rule).unwrap()).unwrap()
    };
    let ha = history(&mut repo, &tips["a"]);
    let tipv: BTreeMap<&str, BTreeMap<String, String>> = if batches {
        ["a", "random", "block"]
            .into_iter()
            .map(|m| (m, tip_values(&mut repo, &tips[m])))
            .collect()
    } else {
        BTreeMap::new()
    };
    let mut graph = BTreeMap::new();
    if batches {
        for layout in LAYOUTS {
            let (tree, view_id, export_s, view_s) = first.trees[layout];
            let view =
                ikam_kernel::sampler::View::decode(&repo.read_content(&view_id).unwrap()).unwrap();
            let t = Instant::now();
            let ix = repo.fragment_index(&tree).unwrap();
            let index_s = secs(t);
            println!(
                "{{\"step\":\"graph\",\"layout\":\"{layout}\",\"nodes\":{},\"edges\":{},\"files\":{},\"view_bytes\":{},\
                 \"export_s\":{export_s:.2},\"fragment_index_s\":{index_s:.2},\"view_s\":{view_s:.2}}}",
                view.nodes(),
                view.edges(),
                view.kind.iter().filter(|&&k| k == 0).count(),
                view.encode().len(),
            );
            let mut per = Vec::new();
            for (hops, batch) in CONFIGS {
                let (ids, s) = &first.batches[&(layout, hops, batch)];
                let bytes: Vec<Vec<u8>> =
                    ids.iter().map(|b| repo.read_content(b).unwrap()).collect();
                let bix = BatchIndex::new(bytes.iter().map(Vec::as_slice)).unwrap();
                let held: usize = bytes
                    .iter()
                    .map(|b| {
                        ikam_kernel::sampler::Batch::decode(b)
                            .unwrap()
                            .nodes()
                            .len()
                    })
                    .sum();
                println!(
                    "{{\"step\":\"batches_first\",\"layout\":\"{layout}\",\"hops\":{hops},\"batch\":{batch},\"batches\":{},\
                     \"nodes_held_mean\":{:.1},\"s\":{s:.2}}}",
                    ids.len(),
                    held as f64 / ids.len() as f64
                );
                per.push((
                    hops,
                    batch,
                    specs(view.nodes(), hops, batch),
                    ids.clone(),
                    bix,
                ));
            }
            graph.insert(layout, (tree, view, ix, per));
        }
    }

    for m in ["random", "block"] {
        // Incremental, in the store that holds `a`'s build.
        let hm = history(&mut repo, &tips[m]);
        let changed = train::changed_keys(&rule, &ha, &hm);
        let mut inc_units = BTreeMap::new();
        for (u, unit) in units.iter().enumerate() {
            let reached = units_reached(unit, changed.iter().map(String::as_str));
            let b = repo
                .train_table_units(&format!("units/{m}/{u}"), &tips[m], &rule, unit, false)
                .unwrap();
            inc_units.insert(
                unit.text(),
                (b.clone(), reached == b.evaluated, reached.len()),
            );
        }
        let mut inc_graph = BTreeMap::new();
        if batches {
            for layout in LAYOUTS {
                let (tree_a, view_a, ix_a, per) = &graph[layout];
                let changed_rows: Vec<(&String, &String, &String)> = tipv["a"]
                    .iter()
                    .filter_map(|(k, v)| {
                        let w = &tipv[m][k];
                        (w != v).then_some((k, v, w))
                    })
                    .collect();
                let t = Instant::now();
                let mut ix = ix_a.clone();
                let mut edits = Vec::new();
                for (k, v, w) in &changed_rows {
                    let old = Id::of_content(format!("{k},{v}\n").as_bytes());
                    edits.extend(
                        repo.propagate(&mut ix, &old, format!("{k},{w}\n").as_bytes())
                            .unwrap(),
                    );
                }
                let tree_m = repo.edit_tree(Some(*tree_a), &edits).unwrap();
                let propagate_s = secs(t);
                let cm = repo
                    .commit(&format!("exp/{layout}/{m}"), tree_m, m)
                    .unwrap();
                let t = Instant::now();
                let (view_m, view_m_id, from_old) =
                    repo.record_fragment_view_after(view_a, &cm).unwrap();
                let view_s = secs(t);
                let (nodes_changed, features) = match view_delta(view_a, &view_m) {
                    Delta::Nodes { changed, features } => {
                        (changed.len() as i64, features.len() as i64)
                    }
                    Delta::All(_) => (-1, -1),
                };
                let files: BTreeSet<&str> = edits
                    .iter()
                    .filter_map(|e| match e {
                        Edit::Put { path, .. } => Some(path.as_str()),
                        _ => None,
                    })
                    .collect();
                let mut res = Vec::new();
                for (hops, batch, sp, ids, bix) in per {
                    let mut bix = bix.clone();
                    let t = Instant::now();
                    let r = repo
                        .resample(&mut bix, view_a, (&view_m, view_m_id), sp, ids)
                        .unwrap();
                    res.push((*hops, *batch, r, secs(t)));
                }
                inc_graph.insert(
                    layout,
                    (
                        tree_m,
                        view_m_id,
                        changed_rows.len(),
                        files.len(),
                        nodes_changed,
                        features,
                        propagate_s,
                        view_s,
                        from_old,
                        res,
                    ),
                );
            }
        }

        // Full, from scratch in a store of its own.
        let mut fresh = Repo::new(MemStore::default());
        fresh.plain_hot_writes = plain;
        let t = Instant::now();
        let tip = *load(&mut fresh, &format!("db/{m}"), evs[m]).last().unwrap();
        let load_s = secs(t);
        assert_eq!(tip, tips[m]);
        let full = build_all(&mut fresh, m, &tip, &rule, &units, batches);
        println!("{{\"step\":\"fresh_load\",\"change\":\"{m}\",\"s\":{load_s:.1}}}");
        for unit in &units {
            let name = unit.text();
            let (b, reached_is_rebuilt, reached) = &inc_units[&name];
            let (fm, funits, frows, fs) = full.manifests[&name];
            println!(
                "{{\"step\":\"units_change\",\"unit\":\"{name}\",\"change\":\"{m}\",\"changed_keys\":{},\"units\":{funits},\
                 \"units_reached\":{reached},\"units_rebuilt\":{},\"rows\":{frows},\"rows_rebuilt\":{},\"inc_s\":{:.3},\
                 \"full_s\":{fs:.3},\"run_s\":{:.2},\"same_manifest\":{},\"reached_is_rebuilt\":{reached_is_rebuilt}}}",
                changed.len(),
                b.evaluated.len(),
                b.rows_evaluated,
                b.cut_s + b.build_s,
                b.run_s,
                b.manifest == fm,
            );
            assert_eq!(b.manifest, fm);
        }
        for (layout, g) in &inc_graph {
            let (tree_m, view_m, rows_c, files_c, nodes_c, feats, prop_s, view_s, from_old, res) =
                g;
            let (ftree, fview, fexport_s, fview_s) = full.trees[layout];
            println!(
                "{{\"step\":\"graph_change\",\"layout\":\"{layout}\",\"change\":\"{m}\",\"rows_changed\":{rows_c},\
                 \"files_changed\":{files_c},\"nodes_changed\":{nodes_c},\"nodes_changed_features\":{feats},\
                 \"propagate_s\":{prop_s:.3},\"export_full_s\":{fexport_s:.3},\"same_tree\":{},\"view_from_old\":{from_old},\
                 \"view_inc_s\":{view_s:.3},\"view_full_s\":{fview_s:.3},\"same_view\":{}}}",
                *tree_m == ftree,
                *view_m == fview
            );
            assert!(*tree_m == ftree && *view_m == fview);
            for (hops, batch, r, inc_s) in res {
                let (fids, fs) = &full.batches[&(*layout, *hops, *batch)];
                let same = *fids == r.batches;
                println!(
                    "{{\"step\":\"batches_change\",\"layout\":\"{layout}\",\"change\":\"{m}\",\"hops\":{hops},\"batch\":{batch},\
                     \"batches\":{},\"rebuilt\":{},\"rebuilt_share\":{:.4},\"by_features\":{},\"all\":{},\"inc_s\":{inc_s:.2},\
                     \"full_s\":{fs:.2},\"same_ids\":{same}}}",
                    fids.len(),
                    r.rebuilt.len(),
                    r.rebuilt.len() as f64 / fids.len() as f64,
                    r.by_features,
                    r.all.is_some()
                );
                assert!(same);
            }
        }
    }
    let t = Instant::now();
    let bad = repo.fsck().unwrap();
    println!(
        "{{\"step\":\"fsck\",\"failures\":{},\"s\":{:.1}}}",
        bad.len(),
        secs(t)
    );
    if deep {
        let t = Instant::now();
        let bad = repo.fsck_deep().unwrap();
        println!(
            "{{\"step\":\"fsck_deep\",\"failures\":{},\"s\":{:.1}}}",
            bad.len(),
            secs(t)
        );
    }
}
