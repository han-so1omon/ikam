//! E420: join keys proposed by name are kept only when the data confirms
//! them, with their evidence.

use std::collections::{BTreeMap, HashSet};

use ikam_kernel::colstats::{TableStats, field_value, value_text};
use ikam_kernel::joinkeys::{
    Candidate, Evidence, Rejected, TableInfo, ValueCache, column_prefix, markers, name_score,
    propose,
};
use ikam_kernel::{Event, Id, MemStore, Repo, Target};

#[test]
fn names_say_which_columns_are_one_thing() {
    assert_eq!(column_prefix(["ss_item_sk", "ss_sold_date_sk"]), "ss_");
    assert_eq!(column_prefix(["ca_address_sk", "c_customer_sk"]), "");
    assert_eq!(column_prefix(["id", "name"]), "");
    // Key markers as `markers` learns them from TPC-DS's tables.
    let m: HashSet<String> = ["sk", "id", "dim"].map(String::from).into();
    let name_score = |c: &str, p: &str, t: &str| name_score(c, p, t, &m);
    // Roles before the parent's words are allowed; a prefix of three
    // letters matches a longer word.
    assert_eq!(name_score("sold_date_sk", "date_sk", "date_dim"), 1.0);
    assert_eq!(
        name_score("current_addr_sk", "address_sk", "customer_address"),
        1.0
    );
    assert_eq!(name_score("item_sk", "item_sk", "item"), 1.0);
    // A key suffix must be on both sides.
    assert_eq!(name_score("sold_time_sk", "time", "time_dim"), 0.0);
    assert_eq!(name_score("store_sk", "store_id", "store"), 0.0);
    // The shared suffix alone is not a match.
    assert_eq!(name_score("reason_sk", "item_sk", "item"), 0.0);
    assert_eq!(name_score("cdemo_sk", "customer_sk", "customer"), 0.0);
    // A column called only by its suffix stands for its table.
    assert_eq!(name_score("order_id", "id", "orders"), 1.0);
    assert_eq!(name_score("date_sk", "sk", "date_dim"), 1.0);
    // A child may end in a word that marks no key in these tables.
    assert_eq!(name_score("customer_fk", "id", "customer"), 1.0);
    assert_eq!(name_score("customer_fk", "id", "store"), 0.0);
    // Object keys are read with their escapes decoded, string values as
    // `observe` counts them.
    assert_eq!(field_value(b"{\"\x5cu0061\":1}", "a"), Some(&b"1"[..]));
    assert_eq!(field_value(b"{\"b\x5c\"c\":2}", "b\"c"), Some(&b"2"[..]));
    assert_eq!(field_value(b"{\"\x5cu0061\":1}", "\x5cu0061"), None);
    assert_eq!(&value_text(b"\"\x5cu0041\"")[..], b"\"A\"");
    assert_eq!(&value_text(b"\"A\"")[..], b"\"A\"");
    assert_eq!(&value_text(b"12")[..], b"12");
    assert_eq!(
        field_value(br#"{"a":1,"b":"x"}"#, "b"),
        Some(&br#""x""#[..])
    );
    assert_eq!(field_value(b"[1, null]", "1"), Some(&b"null"[..]));
}

fn load(repo: &mut Repo<MemStore>, rows: &[(&str, String)]) -> ikam_kernel::Db {
    let db = repo.db_empty().unwrap();
    let events: Vec<Event> = rows
        .iter()
        .enumerate()
        .map(|(i, (k, v))| Event::upsert(k, v.as_bytes(), &format!("e{i}")))
        .collect();
    repo.db_append(&db, &events).unwrap()
}

#[test]
fn only_pairs_the_data_confirms_are_kept() {
    let mut repo = Repo::new(MemStore::default());
    let mut rows = Vec::new();
    for i in 1..=50 {
        rows.push((
            "t/owner",
            format!(
                r#"{{"o_owner_sk":{i},"o_region_sk":{},"o_name":"n{i}"}}"#,
                1 + i % 5
            ),
        ));
    }
    for i in 0..400 {
        let owner = if i % 13 == 0 {
            "null".to_string()
        } else {
            (1 + i % 50).to_string()
        };
        rows.push((
            "t/pet",
            format!(
                r#"{{"p_pet_sk":{i},"p_owner_sk":{owner},"p_region_sk":{},"p_prev_owner_sk":{}}}"#,
                1 + i % 7,
                40 + i % 30
            ),
        ));
    }
    let rows: Vec<(String, String)> = rows
        .into_iter()
        .enumerate()
        .map(|(n, (t, v))| (format!("{t}/{n:05}"), v))
        .collect();
    let rows: Vec<(&str, String)> = rows.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let db = load(&mut repo, &rows);

    let mut stats = BTreeMap::new();
    for t in ["t/owner", "t/pet"] {
        let mut s = TableStats::with_zones(16);
        repo.bm25_view_build_with_stats(&db, &format!("{t}/"), &mut s)
            .unwrap();
        stats.insert(t.to_string(), s);
    }
    let infos: Vec<TableInfo> = ["owner", "pet"]
        .iter()
        .map(|n| TableInfo {
            table: format!("t/{n}"),
            name: n.to_string(),
            stats: &stats[&format!("t/{n}")],
        })
        .collect();
    let cands = propose(&infos, 1.0, None);
    let labels: Vec<String> = cands.iter().map(|c| c.label()).collect();
    for want in [
        "t/pet.p_owner_sk>t/owner.o_owner_sk",
        "t/pet.p_prev_owner_sk>t/owner.o_owner_sk",
        "t/pet.p_region_sk>t/owner.o_region_sk",
        "t/owner.o_owner_sk>t/pet.p_owner_sk",
    ] {
        assert!(labels.contains(&want.to_string()), "{want} in {labels:?}");
    }
    let mut cache = ValueCache::new();
    let mut kept = Vec::new();
    for c in &cands {
        let r = repo.join_key_confirm(&db, c, &stats, &mut cache).unwrap();
        match c.label().as_str() {
            "t/pet.p_owner_sk>t/owner.o_owner_sk" => {
                let e = r.clone().unwrap();
                assert_eq!(e.contained, 400 - 31);
                assert_eq!(e.child_distinct, 50);
                assert_eq!(e.parent_distinct, 50);
                assert_eq!(e.join_rows, 400 - 31);
                assert_eq!(e.child_nulls, 31);
            }
            // 40..69 is not all inside 1..50: the range rules it out.
            "t/pet.p_prev_owner_sk>t/owner.o_owner_sk" => assert_eq!(r, Err(Rejected::Range)),
            // The owner's region repeats: not a key.
            "t/pet.p_region_sk>t/owner.o_region_sk" => assert_eq!(r, Err(Rejected::NotUnique)),
            "t/owner.o_owner_sk>t/pet.p_owner_sk" => assert_eq!(r, Err(Rejected::NotUnique)),
            _ => {}
        }
        if let Ok(e) = r {
            kept.push((c.clone(), e));
        }
    }
    assert_eq!(kept.len(), 1);
    let root = repo.put_join_keys(&kept).unwrap();
    let nodes = repo.graph_nodes(&root).unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].label, "t/pet.p_owner_sk>t/owner.o_owner_sk");
    assert_eq!(nodes[1].label, "t/pet.p_owner_sk>t/owner.o_owner_sk/join");
    assert_eq!(nodes[1].target, Target::Object(kept[0].1.join_root));
    assert_eq!(nodes[0].edges[0].label, "join_root");
    // Kept by a ref, the evidence keeps its join from gc.
    repo.commit("keys", root, "").unwrap();
    repo.gc().unwrap();
    assert!(repo.graph_nodes(&kept[0].1.join_root).is_ok());
    let Target::Content(a) = &nodes[0].target else {
        panic!("evidence is content")
    };
    let ev = String::from_utf8(repo.read_content(&a.id()).unwrap()).unwrap();
    assert!(ev.contains("\"contained\":369") && ev.contains("\"join_rows\":369"));
}

#[test]
fn exact_checks_catch_what_stats_let_through() {
    // A child value inside the parent's range and zones but not a parent
    // value; and a parent that repeats a value the sketch cannot see.
    let mut repo = Repo::new(MemStore::default());
    let mut rows = Vec::new();
    for i in 1..=40 {
        let k = if i == 20 { 22 } else { i * 2 }; // even keys, 22 twice
        rows.push((format!("t/a/{i:03}"), format!("[{k}, {}]", i * 2)));
    }
    for i in 0..30 {
        let v = if i == 7 { 41 } else { 2 + 2 * (i % 40) };
        rows.push((
            format!("t/b/{i:03}"),
            format!("[{v}, {}]", 2 + 2 * (i % 40)),
        ));
    }
    let rows: Vec<(&str, String)> = rows.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let db = load(&mut repo, &rows);
    let mut stats = BTreeMap::new();
    for t in ["t/a", "t/b"] {
        let mut s = TableStats::with_zones(8);
        repo.bm25_view_build_with_stats(&db, &format!("{t}/"), &mut s)
            .unwrap();
        stats.insert(t.to_string(), s);
    }
    let cand = |child: &str, parent: &str| ikam_kernel::joinkeys::Candidate {
        child_table: "t/b".into(),
        child: child.into(),
        parent_table: "t/a".into(),
        parent: parent.into(),
        name_score: 1.0,
    };
    let mut cache = ValueCache::new();
    // Column 0 of a repeats 22: the sketch counts 39 of 40 distinct, the
    // exact pass finds the repeat.
    let r = repo
        .join_key_confirm(&db, &cand("1", "0"), &stats, &mut cache)
        .unwrap();
    assert_eq!(r, Err(Rejected::NotUnique));
    // Column 1 of a is a key of even numbers 2..80; b's column 0 has 41.
    let r = repo
        .join_key_confirm(&db, &cand("0", "1"), &stats, &mut cache)
        .unwrap();
    assert_eq!(r, Err(Rejected::NotContained(1)));
    let r = repo
        .join_key_confirm(&db, &cand("1", "1"), &stats, &mut cache)
        .unwrap();
    assert_eq!(r.unwrap().join_rows, 30);
}

#[test]
fn key_markers_are_learned_from_the_tables() {
    // `id` is the key of both tables; `customer_fk` and `customer_code`
    // name the customer in words no fixed list holds. Some orders spell a
    // code with escapes, which must read as the code it spells.
    let mut repo = Repo::new(MemStore::default());
    let mut rows = Vec::new();
    for i in 1..=30 {
        rows.push((
            format!("t/customer/{i:03}"),
            format!(r#"{{"id":{i},"code":"C{i}","city":"x{}"}}"#, i % 4),
        ));
    }
    for i in 0..90 {
        let c = 1 + i % 30;
        let code = if i % 9 == 0 {
            format!("\\u0043{c}")
        } else {
            format!("C{c}")
        };
        rows.push((
            format!("t/orders/{i:03}"),
            format!(
                r#"{{"id":{},"customer_fk":{c},"customer_code":"{code}","amount":{}}}"#,
                100 + i,
                i % 7
            ),
        ));
    }
    let rows: Vec<(&str, String)> = rows.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let db = load(&mut repo, &rows);
    let mut stats = BTreeMap::new();
    for t in ["t/customer", "t/orders"] {
        let mut s = TableStats::with_zones(16);
        repo.bm25_view_build_with_stats(&db, &format!("{t}/"), &mut s)
            .unwrap();
        stats.insert(t.to_string(), s);
    }
    let infos: Vec<TableInfo> = ["customer", "orders"]
        .iter()
        .map(|n| TableInfo {
            table: format!("t/{n}"),
            name: n.to_string(),
            stats: &stats[&format!("t/{n}")],
        })
        .collect();
    assert_eq!(markers(&infos), HashSet::from(["id".to_string()]));
    let cands = propose(&infos, 1.0, None);
    let mut cache = ValueCache::new();
    for want in [
        "t/orders.customer_fk>t/customer.id",
        "t/orders.customer_code>t/customer.code",
    ] {
        let c = cands.iter().find(|c| c.label() == want);
        let c = c.unwrap_or_else(|| panic!("{want} in {cands:?}"));
        let e = repo.join_key_confirm(&db, c, &stats, &mut cache).unwrap();
        let e = e.unwrap_or_else(|r| panic!("{want}: {r:?}"));
        assert_eq!((e.contained, e.join_rows, e.child_distinct), (90, 90, 30));
    }
    // One table's id names nothing in the other.
    assert!(!cands.iter().any(|c| c.child == "id"));
}

#[test]
fn evidence_json_escapes_names() {
    let c = Candidate {
        child_table: "t/a\"b".into(),
        child: "x\\y".into(),
        parent_table: "t/p".into(),
        parent: "k\nl".into(),
        name_score: 1.0,
    };
    let e = Evidence {
        name_score: 1.0,
        child_rows: 1,
        child_nulls: 0,
        child_distinct_est: 1.0,
        parent_rows: 1,
        parent_distinct_est: 1.0,
        ranges: None,
        contained: 1,
        child_distinct: 1,
        parent_distinct: 1,
        join_rows: 1,
        join_root: Id::of_content(b""),
    };
    let v: serde_json::Value = serde_json::from_str(&e.to_json(&c)).unwrap();
    assert_eq!(v["child"], "t/a\"b.x\\y");
    assert_eq!(v["parent"], "t/p.k\nl");
}
