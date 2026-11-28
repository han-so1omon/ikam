//! E429: rebuild only what a change reaches (src/lineage.rs). A table
//! built in units after a change, reusing the units whose keys did not
//! change, is the table built at once; batches of a view numbered by paths,
//! sampled again only where they hold a changed node, are the batches
//! sampled again in full; fsck evaluates the reused records. E444: the
//! unit of rebuild chosen by a pattern over keys.

use std::collections::{BTreeMap, BTreeSet};

use ikam_kernel::lineage::{
    BatchIndex, Delta, Manifest, Unit, glob_prefix, units_reached, view_delta,
};
use ikam_kernel::sampler::{Order, Spec, Targets};
use ikam_kernel::train::{self, History, TrainRule};
use ikam_kernel::{Arg, Edit, Event, Id, MemStore, Repo, func};

fn rnd(mut s: u64) -> u64 {
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    s
}

/// A database of `rows` keys under `t/` over `commits` commits on `name`;
/// keys in `fixed` get every value from commit `from` on raised by 1000
/// (a correction of their history). Returns the commits.
fn history(
    repo: &mut Repo<MemStore>,
    name: &str,
    rows: u64,
    commits: u64,
    fixed: &BTreeSet<u64>,
    from: u64,
) -> Vec<Id> {
    let mut db = repo.db_empty().unwrap();
    let mut values: BTreeMap<u64, u64> = BTreeMap::new();
    let mut out = Vec::new();
    for c in 0..commits {
        let mut ev = Vec::new();
        for i in 0..rows {
            let r = rnd((c << 32) ^ i ^ 0x5bd1_e995);
            let next = match values.get(&i) {
                None => Some(r % 1000),
                Some(v) if r.is_multiple_of(5) => Some(v + 1 + r % 7),
                Some(&v) if r % 10 == 1 => Some(v),
                Some(_) => None,
            };
            let lift = if fixed.contains(&i) && c >= from {
                1000
            } else {
                0
            };
            match next {
                Some(v) => {
                    values.insert(i, v);
                    ev.push(Event::upsert(
                        &format!("t/{i:04}"),
                        (v + lift).to_string().as_bytes(),
                        &format!("{c}-{i}"),
                    ));
                }
                None if lift > 0 && c == from => ev.push(Event::upsert(
                    &format!("t/{i:04}"),
                    (values[&i] + lift).to_string().as_bytes(),
                    &format!("{c}-{i}-fix"),
                )),
                None => {}
            }
        }
        db = repo.db_append(&db, &ev).unwrap();
        out.push(repo.db_commit(name, &db, "c").unwrap());
    }
    out
}

fn rule() -> TrainRule {
    TrainRule {
        prefix: "t/".into(),
        horizon: 2,
        every: 2,
    }
}

/// Every row of a units table, as (key, observation, label, value,
/// changes, age).
type Key = (String, u32, bool, i64, u32, u32);

fn unit_rows(repo: &Repo<MemStore>, manifest: &Id) -> BTreeSet<Key> {
    let m = Manifest::decode(&repo.read_content(manifest).unwrap()).unwrap();
    let mut out = BTreeSet::new();
    for u in m.units {
        let shard = train::Shard::decode(&repo.read_content(&u.shard).unwrap()).unwrap();
        for r in train::decode_table(&repo.read_content(&u.table).unwrap()).unwrap() {
            let key = shard.keys[r.row as usize].clone();
            assert!(out.insert((key, r.obs, r.label, r.value, r.changes, r.age)));
        }
    }
    out
}

fn whole_rows(repo: &mut Repo<MemStore>, tip: &Id) -> BTreeSet<Key> {
    let table = repo.train_table("train/whole", tip, &rule()).unwrap();
    let (_, h) = repo.train_inputs(&table).unwrap();
    let h = History::decode(&repo.read_content(&h).unwrap()).unwrap();
    train::decode_table(&repo.read_content(&table).unwrap())
        .unwrap()
        .into_iter()
        .map(|r| {
            let key = h.keys[r.row as usize].clone();
            (key, r.obs, r.label, r.value, r.changes, r.age)
        })
        .collect()
}

#[test]
fn a_units_table_rebuilt_after_a_change_is_the_table_built_at_once() {
    let mut repo = Repo::new(MemStore::default());
    let fixed: BTreeSet<u64> = [3, 41, 42, 97].into();
    let a = history(&mut repo, "db/a", 120, 11, &BTreeSet::new(), 0);
    let b = history(&mut repo, "db/b", 120, 11, &fixed, 4);
    let (ta, tb) = (*a.last().unwrap(), *b.last().unwrap());
    let ha = History::decode(&repo.train_history(&ta, &rule()).unwrap()).unwrap();
    let hb = History::decode(&repo.train_history(&tb, &rule()).unwrap()).unwrap();
    let changed = train::changed_keys(&rule(), &ha, &hb);
    let expect: Vec<String> = fixed.iter().map(|i| format!("t/{i:04}")).collect();
    assert_eq!(changed, expect);
    for unit in [Unit::Row, Unit::Prefix(5), Unit::Table] {
        assert_eq!(Unit::parse(&unit.text()).unwrap(), unit);
        let first = repo
            .train_table_units("units/a", &ta, &rule(), &unit, false)
            .unwrap();
        let inc = repo
            .train_table_units("units/b", &tb, &rule(), &unit, false)
            .unwrap();
        let full = repo
            .train_table_units("units/b", &tb, &rule(), &unit, true)
            .unwrap();
        assert_eq!(inc.manifest, full.manifest, "{unit:?}");
        assert_eq!(full.evaluated.len(), full.units);
        let reached = units_reached(&unit, changed.iter().map(String::as_str));
        assert_eq!(inc.evaluated, reached, "{unit:?}");
        assert!(first.units >= inc.evaluated.len());
        assert_eq!(unit_rows(&repo, &inc.manifest), whole_rows(&mut repo, &tb));
        assert_eq!(
            unit_rows(&repo, &first.manifest),
            whole_rows(&mut repo, &ta)
        );
    }
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn a_change_past_the_last_label_reaches_no_unit() {
    let mut repo = Repo::new(MemStore::default());
    let a = history(&mut repo, "db/a", 60, 12, &BTreeSet::new(), 0);
    // A correction at the last commit (11): the last observation (8) reads
    // up to commit 10, so no row observes or labels it.
    let b = history(&mut repo, "db/b", 60, 12, &[5, 6].into(), 11);
    let (ta, tb) = (*a.last().unwrap(), *b.last().unwrap());
    assert_eq!(train::window(&rule(), 12), 11);
    repo.train_table_units("u/a", &ta, &rule(), &Unit::Row, false)
        .unwrap();
    let inc = repo
        .train_table_units("u/b", &tb, &rule(), &Unit::Row, false)
        .unwrap();
    assert!(inc.evaluated.is_empty());
    let ha = History::decode(&repo.train_history(&ta, &rule()).unwrap()).unwrap();
    let hb = History::decode(&repo.train_history(&tb, &rule()).unwrap()).unwrap();
    assert!(train::changed_keys(&rule(), &ha, &hb).is_empty());
}

/// Rows as fragments of two exports: files of 8 consecutive keys, and
/// files of keys by hash. Returns the tree.
fn export(repo: &mut Repo<MemStore>, values: &BTreeMap<u64, u64>, buckets: u64) -> Id {
    let mut files: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (i, v) in values {
        let line = format!("t/{i:04},{v}\n");
        files
            .entry(format!("range/{:03}.csv", i / 8))
            .or_default()
            .push(line.clone());
        files
            .entry(format!("bucket/{:03}.csv", rnd(i + 1) % buckets))
            .or_default()
            .push(line);
    }
    let mut edits = Vec::new();
    for (path, lines) in files {
        let bytes = lines.concat().into_bytes();
        let mut cuts = Vec::new();
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

fn specs(n: u32, batch: u32, hops: u32) -> Vec<Spec> {
    (0..n)
        .map(|index| Spec {
            seed: 7,
            fanout: 3,
            hops,
            batch,
            index,
            targets: Targets::All,
        })
        .collect()
}

#[test]
fn batches_sampled_again_where_they_hold_a_changed_node_are_a_full_resample() {
    let mut repo = Repo::new(MemStore::default());
    let values: BTreeMap<u64, u64> = (0..200).map(|i| (i, rnd(i + 9) % 100)).collect();
    let tree_a = export(&mut repo, &values, 23);
    let ca = repo.commit("exp", tree_a, "a").unwrap();
    let (view_a_id, _) = repo
        .record_fragment_view_ordered(&ca, Order::Paths)
        .unwrap();
    let view_a = repo.fragment_view_ordered(&ca, Order::Paths).unwrap();
    assert_eq!(view_a_id, Id::of_content(&view_a.encode()));
    assert!(repo.check_fragment_view(&view_a_id).unwrap());
    for (batch, hops) in [(1, 1), (4, 2)] {
        let n = (view_a.nodes() as u32).div_ceil(batch);
        let sp = specs(n, batch, hops);
        let old: Vec<Id> = sp
            .iter()
            .map(|s| repo.apply_sample(view_a_id, s).unwrap())
            .collect();
        let bytes: Vec<Vec<u8>> = old.iter().map(|b| repo.read_content(b).unwrap()).collect();
        let mut ix = BatchIndex::new(bytes.iter().map(Vec::as_slice)).unwrap();

        // Change three rows through propagate.
        let mut ixf = repo.fragment_index(&tree_a).unwrap();
        let mut edits = Vec::new();
        let mut vals = values.clone();
        for i in [5u64, 77, 150] {
            let old_line = format!("t/{i:04},{}\n", vals[&i]);
            vals.insert(i, vals[&i] + 1000);
            let new_line = format!("t/{i:04},{}\n", vals[&i]);
            edits.extend(
                repo.propagate(
                    &mut ixf,
                    &Id::of_content(old_line.as_bytes()),
                    new_line.as_bytes(),
                )
                .unwrap(),
            );
        }
        let tree_b = repo.edit_tree(Some(tree_a), &edits).unwrap();
        assert_eq!(tree_b, export(&mut repo, &vals, 23));
        let cb = repo.commit("exp", tree_b, "b").unwrap();
        let (view_b, view_b_id, from_old) = repo.record_fragment_view_after(&view_a, &cb).unwrap();
        assert!(from_old);
        let full_view = repo.fragment_view_ordered(&cb, Order::Paths).unwrap();
        assert_eq!(view_b, full_view);
        assert!(repo.check_fragment_view(&view_b_id).unwrap());

        let r = repo
            .resample(&mut ix, &view_a, (&view_b, view_b_id), &sp, &old)
            .unwrap();
        assert!(r.all.is_none());
        assert!(!r.rebuilt.is_empty());
        if batch == 1 {
            assert!(
                r.rebuilt.len() < sp.len() / 2,
                "batch {batch}: {} rebuilt",
                r.rebuilt.len()
            );
        }
        assert_eq!(r.by_id, r.rebuilt.len());
        for (s, got) in sp.iter().zip(&r.batches) {
            let text = repo.put_content(s.to_text().as_bytes()).unwrap().id;
            let (bytes, _) = repo
                .evaluate(&func::sample(), &[Arg::Whole(view_b_id), Arg::Whole(text)])
                .unwrap();
            assert_eq!(Id::of_content(&bytes), *got);
        }
        // The index now describes the new batches.
        let fresh: Vec<Vec<u8>> = r
            .batches
            .iter()
            .map(|b| repo.read_content(b).unwrap())
            .collect();
        let again = BatchIndex::new(fresh.iter().map(Vec::as_slice)).unwrap();
        let all: Vec<u32> = (0..view_b.nodes() as u32).collect();
        for k in all.chunks(7) {
            assert_eq!(ix.reached(k), again.reached(k));
        }
    }
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn a_change_of_shape_builds_the_view_again_and_ids_order_resamples_all() {
    let mut repo = Repo::new(MemStore::default());
    let mut values: BTreeMap<u64, u64> = (0..40).map(|i| (i, i * 3)).collect();
    let ta = export(&mut repo, &values, 5);
    let ca = repo.commit("exp", ta, "a").unwrap();
    let va = repo.fragment_view_ordered(&ca, Order::Paths).unwrap();
    // A new row adds a fragment: the shape changes.
    values.insert(40, 1);
    let tb = export(&mut repo, &values, 5);
    let cb = repo.commit("exp", tb, "b").unwrap();
    assert!(repo.fragment_view_after(&va, &cb).unwrap().is_none());
    let (vb, _, from_old) = repo.record_fragment_view_after(&va, &cb).unwrap();
    assert!(!from_old);
    assert!(matches!(view_delta(&va, &vb), Delta::All(_)));
    // Views numbered by id: every batch may differ.
    let ia = repo.fragment_view(&ca).unwrap();
    assert_eq!(ia.order, Order::Ids);
    assert!(matches!(view_delta(&ia, &ia), Delta::All(_)));
    // A row value changed in place keeps the shape: two nodes per layout
    // (the row and its files) and the row change.
    let mut v2: BTreeMap<u64, u64> = (0..40).map(|i| (i, i * 3)).collect();
    v2.insert(9, 900);
    let tc = export(&mut repo, &v2, 5);
    let cc = repo.commit("exp", tc, "c").unwrap();
    let vc = repo.fragment_view_after(&va, &cc).unwrap().unwrap();
    assert_eq!(vc, repo.fragment_view_ordered(&cc, Order::Paths).unwrap());
    let Delta::Nodes { changed, .. } = view_delta(&va, &vc) else {
        panic!("shape kept");
    };
    assert_eq!(changed.len(), 3);
}

#[test]
fn a_pattern_names_the_longest_prefix_of_a_key_it_matches_whole() {
    let g = |p, k| glob_prefix(p, k);
    assert_eq!(g("*/*", "t/0012345"), Some("t/0012345"));
    assert_eq!(g("*/", "t/0012345"), Some("t/"));
    assert_eq!(g("*/?????", "t/0012345"), Some("t/00123"));
    assert_eq!(g("*/????????", "t/0012345"), None);
    assert_eq!(g("t/00*", "t/0012345"), Some("t/0012345"));
    assert_eq!(g("t/01*", "t/0012345"), None);
    assert_eq!(g("*", "a/b/c"), Some("a"));
    assert_eq!(g("*/*/", "a/b/c"), Some("a/b/"));
    assert_eq!(g("*/*/", "a/b"), None);
    assert_eq!(g("?/*", "ab/c"), None);
    assert_eq!(g("", "t/1"), Some(""));
    for unit in [
        Unit::Pattern("*/???".into()),
        Unit::Pattern("t/0* x".into()),
    ] {
        assert_eq!(Unit::parse(&unit.text()).unwrap(), unit);
    }
    // A key no prefix of which matches is its own unit.
    let p = Unit::Pattern("t/01*".into());
    assert_eq!(p.of("t/0003"), "t/0003");
    assert_eq!(p.of("t/0107"), "t/0107");
    let q = Unit::Pattern("*/??".into());
    assert_eq!(q.of("t/0107"), "t/01");
    assert_eq!(q.of("t"), "t");
}

#[test]
fn every_pattern_rebuilds_the_table_built_at_once_and_finer_ones_rebuild_less() {
    let mut repo = Repo::new(MemStore::default());
    let fixed: BTreeSet<u64> = [3, 41, 42, 97].into();
    let a = history(&mut repo, "db/a", 120, 11, &BTreeSet::new(), 0);
    let b = history(&mut repo, "db/b", 120, 11, &fixed, 4);
    let (ta, tb) = (*a.last().unwrap(), *b.last().unwrap());
    let ha = History::decode(&repo.train_history(&ta, &rule()).unwrap()).unwrap();
    let hb = History::decode(&repo.train_history(&tb, &rule()).unwrap()).unwrap();
    let changed = train::changed_keys(&rule(), &ha, &hb);
    let whole = whole_rows(&mut repo, &tb);
    let build = |repo: &mut Repo<MemStore>, unit: &Unit| {
        repo.train_table_units("units/a", &ta, &rule(), unit, false)
            .unwrap();
        let inc = repo
            .train_table_units("units/b", &tb, &rule(), unit, false)
            .unwrap();
        let mut fresh = Repo::new(MemStore::default());
        let c = history(&mut fresh, "db/b", 120, 11, &fixed, 4);
        let full = fresh
            .train_table_units("units/b", c.last().unwrap(), &rule(), unit, true)
            .unwrap();
        assert_eq!(inc.manifest, full.manifest, "{unit:?}");
        assert_eq!(full.evaluated.len(), full.units, "{unit:?}");
        assert_eq!(unit_rows(repo, &inc.manifest), whole, "{unit:?}");
        inc
    };
    // Coarse to fine: one unit; 100 keys a unit; 10 keys a unit; one row.
    let mut rows = Vec::new();
    for p in ["*/", "*/??", "*/???", "*/*"] {
        let unit = Unit::Pattern(p.into());
        let inc = build(&mut repo, &unit);
        let reached = units_reached(&unit, changed.iter().map(String::as_str));
        assert_eq!(inc.evaluated, reached, "{p}");
        rows.push((p, inc.evaluated.len(), inc.rows_evaluated));
    }
    let units: Vec<usize> = rows.iter().map(|r| r.1).collect();
    assert_eq!(units, [1, 1, 3, 4]);
    assert!(rows.windows(2).all(|w| w[0].2 > w[1].2), "{rows:?}");
    // A pattern naming the same groups as a fixed unit cuts the same
    // shards and builds the same tables.
    let ids = |repo: &mut Repo<MemStore>, unit: &Unit| {
        let m = repo
            .train_table_units("units/c", &tb, &rule(), unit, false)
            .unwrap()
            .manifest;
        let m = Manifest::decode(&repo.read_content(&m).unwrap()).unwrap();
        m.units
            .into_iter()
            .map(|u| (u.shard, u.table))
            .collect::<Vec<_>>()
    };
    for (p, unit) in [
        ("*/*", Unit::Row),
        ("*/??", Unit::Prefix(4)),
        ("*/", Unit::Table),
    ] {
        let want = ids(&mut repo, &unit);
        assert_eq!(ids(&mut repo, &Unit::Pattern(p.into())), want, "{p}");
    }
    // A pattern matching only some keys: `t/01*` keeps t/0100..t/0119 as
    // rows and every other key too (none of them matches), so it is per
    // row, and its units are the shards `*/*` built: none is evaluated.
    let inc = build(&mut repo, &Unit::Pattern("t/01*".into()));
    assert_eq!(inc.units, 120);
    assert!(inc.evaluated.is_empty());
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.fsck_deep().unwrap().is_empty());
}
