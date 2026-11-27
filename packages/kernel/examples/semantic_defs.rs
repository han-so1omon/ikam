//! E419's run: Apache Ossie's examples as versioned semantic definitions.
//!
//! `semantic_defs OSSIE_DIR WORK_DIR` reads a clone of apache/ossie (never
//! committed to ikam), round-trips every YAML file in it, imports the
//! TPC-DS, flights and SAP examples into a project on disk, edits a
//! metric, reads both commits back, searches the definitions, certifies
//! metrics through the `certify` net, and shares the TPC-DS set with a
//! second project that edits one metric, merged back only once
//! authorized. Prints what it measured.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use ikam_kernel::semantic::{Approval, MergeRefused, kind_of, segment, trigram_vector};
use ikam_kernel::share::{Refused, Right, Scope, Share, import_share};
use ikam_kernel::{Edit, FsStore, Id, MemStore, Object, Repo, Store};

const KEY_A: [u8; 32] = [11; 32];
const KEY_B: [u8; 32] = [22; 32];
const NOW: u64 = 1_000;

fn yaml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut es: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    es.sort_by_key(|e| e.path());
    for e in es {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            yaml_files(&p, out);
        } else if name.ends_with(".yaml") || name.ends_with(".yml") {
            out.push(p);
        }
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn du(p: &Path) -> u64 {
    let mut n = 0;
    if let Ok(es) = std::fs::read_dir(p) {
        for e in es.flatten() {
            let m = e.metadata().unwrap();
            n += if m.is_dir() { du(&e.path()) } else { m.len() };
        }
    }
    n
}

fn tree<S: Store>(r: &Repo<S>, c: &Id) -> Id {
    match r.get(c).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    }
}

fn has_path<S: Store>(r: &Repo<S>, c: &Id, path: &str) -> bool {
    let mut id = tree(r, c);
    for part in path.split('/') {
        let Ok(Object::Tree(es)) = r.get(&id) else {
            return false;
        };
        match es.into_iter().find(|e| e.name == part) {
            Some(e) => id = e.id,
            None => return false,
        }
    }
    true
}

fn exp<S: Store>(a: &Repo<S>, c: &Id) -> Vec<u8> {
    a.defs_export(c, "tpcds", "tpcds_semantic_model.yaml")
        .unwrap()
}

fn approve(net: &str, t: &str, token: &str) -> (Approval, [u8; 32]) {
    let a = Approval {
        person: "owner".into(),
        net: net.into(),
        transition: t.into(),
        token: token.into(),
    };
    let tag = a.seal(&KEY_A);
    (a, tag)
}

fn frag<S: Store>(r: &Repo<S>, c: &Id, set: &str, key: &str) -> (Id, Vec<u8>) {
    let d = r
        .defs_list(c, set)
        .unwrap()
        .into_iter()
        .find(|d| d.key() == key)
        .unwrap_or_else(|| panic!("no {key}"));
    (d.id, r.read_content(&d.id).unwrap())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (ossie, work) = (PathBuf::from(&args[1]), PathBuf::from(&args[2]));
    let ex = ossie.join("examples");
    let read = |n: &str| std::fs::read(ex.join(n)).unwrap();

    // 1. Every YAML file in the clone, round-tripped.
    let mut files = Vec::new();
    yaml_files(&ossie, &mut files);
    let mut r = Repo::new(MemStore::default());
    let (mut exact, mut bytes, mut frags) = (0, 0, 0);
    let t = Instant::now();
    for (i, f) in files.iter().enumerate() {
        let b = std::fs::read(f).unwrap();
        let c = r
            .defs_import("main", &format!("f{i}"), &[("d.yaml", &b)], "rt")
            .unwrap();
        exact += usize::from(r.defs_export(&c, &format!("f{i}"), "d.yaml").unwrap() == b);
        bytes += b.len();
        frags += segment(&b).len();
    }
    println!(
        "round trip, every yaml file in the clone: {exact}/{} exact, {bytes} bytes, {frags} fragments, {:.1} ms",
        files.len(),
        ms(t)
    );

    // 2. Project a on disk: three definition sets and some data files.
    let sets: &[(&str, &[&str])] = &[
        ("tpcds", &["tpcds_semantic_model.yaml"]),
        (
            "flights",
            &[
                "flights.semantic_model.yaml",
                "flights.ontology.yaml",
                "flights.mapping.yaml",
            ],
        ),
        (
            "sap",
            &[
                "sap_business_partner.semantic_model.yaml",
                "sap_business_partner.ontology.yaml",
                "sap_business_partner.mapping.yaml",
            ],
        ),
    ];
    let (pa, pb) = (work.join("a"), work.join("b"));
    let mut a = Repo::new(FsStore::open(&pa).unwrap());
    let mut src_bytes = 0;
    println!(
        "\nset | docs | bytes | fragments by kind | import ms | export exact | links: fragments naming columns, column refs, dangling"
    );
    for (set, docs) in sets {
        let owned: Vec<(String, Vec<u8>)> = docs.iter().map(|d| (d.to_string(), read(d))).collect();
        let refs: Vec<(&str, &[u8])> = owned
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let n: usize = owned.iter().map(|d| d.1.len()).sum();
        src_bytes += n;
        let t = Instant::now();
        let c = a
            .defs_import("main", set, &refs, &format!("import {set}"))
            .unwrap();
        let tm = ms(t);
        let mut ok = 0;
        for (name, b) in &owned {
            ok += usize::from(a.defs_export(&c, set, name).unwrap() == *b);
        }
        let mut mem = Repo::new(MemStore::default());
        let t = Instant::now();
        mem.defs_import("main", set, &refs, "mem").unwrap();
        let tmem = ms(t);
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for d in a.defs_list(&c, set).unwrap() {
            *kinds.entry(kind_of(&d.path)).or_default() += 1;
        }
        let links = a.defs_links(&c, set).unwrap();
        let fields: BTreeSet<&String> = links
            .iter()
            .filter(|(k, _)| k.contains("/fields/"))
            .flat_map(|(_, v)| v)
            .collect();
        let named = links.values().filter(|v| !v.is_empty()).count();
        let refs_n: usize = links.values().map(|v| v.len()).sum();
        let dangling: BTreeSet<&String> = links
            .iter()
            .filter(|(k, _)| !k.contains("/fields/"))
            .flat_map(|(_, v)| v)
            .filter(|c| !fields.contains(c))
            .collect();
        println!(
            "{set} | {} | {n} | {kinds:?} | {tm:.1} (MemStore {tmem:.1}) | {ok}/{} | {named}, {refs_n}, {} {dangling:?}",
            owned.len(),
            owned.len(),
            dangling.len()
        );
    }
    // Data the definitions describe (synthetic), outside every set.
    let mut csv = String::from("ss_item_sk,ss_ticket_number,ss_quantity,ss_net_profit\n");
    for i in 0..200u64 {
        csv.push_str(&format!(
            "{},{},{},{}\n",
            i % 37,
            i,
            1 + i % 9,
            (i * 7919) % 1000
        ));
    }
    let t0 = tree(&a, &a.resolve("main").unwrap());
    let id = a.put_content(csv.as_bytes()).unwrap().id;
    let t1 = a
        .edit_tree(
            Some(t0),
            &[Edit::Put {
                path: "data/store_sales.csv".into(),
                id,
            }],
        )
        .unwrap();
    let c1 = a.commit("main", t1, "data").unwrap();
    println!(
        "store a after import: {} bytes on disk for {src_bytes} source bytes",
        du(&pa)
    );

    // 3. Edit one metric (text), then what it names; read every commit.
    let mkey = "tpcds_semantic_model.yaml/metrics/total_profit";
    let (_, old) = frag(&a, &c1, "tpcds", mkey);
    let old_s = String::from_utf8(old.clone()).unwrap();
    let new = old_s.replace(
        "Total net profit from store sales",
        "Total net profit from store sales, after returns are netted out",
    );
    let c2 = a
        .defs_edit(
            "main",
            "tpcds",
            "tpcds_semantic_model.yaml",
            "metrics/total_profit",
            new.as_bytes(),
            "describe",
        )
        .unwrap();
    let new3 = new.replace(
        "SUM(store_sales.ss_net_profit)",
        "SUM(store_sales.ss_net_profit) / NULLIF(SUM(store_sales.ss_quantity), 0)",
    );
    let c3 = a
        .defs_edit(
            "main",
            "tpcds",
            "tpcds_semantic_model.yaml",
            "metrics/total_profit",
            new3.as_bytes(),
            "per unit",
        )
        .unwrap();
    let orig = read("tpcds_semantic_model.yaml");
    let want2 = String::from_utf8(orig.clone())
        .unwrap()
        .replacen(&old_s, &new, 1);
    let want3 = String::from_utf8(orig.clone())
        .unwrap()
        .replacen(&old_s, &new3, 1);
    println!("\nedit of {mkey}:");
    println!(
        "  diff c1..c2 (text): {:?}",
        a.defs_diff(&c1, &c2, "tpcds").unwrap()
    );
    println!(
        "  diff c2..c3 (expression): {:?}",
        a.defs_diff(&c2, &c3, "tpcds").unwrap()
    );
    let (l1, l2, l3) = (
        a.defs_links(&c1, "tpcds").unwrap(),
        a.defs_links(&c2, "tpcds").unwrap(),
        a.defs_links(&c3, "tpcds").unwrap(),
    );
    let changed = |x: &BTreeMap<String, Vec<String>>, y: &BTreeMap<String, Vec<String>>| {
        x.iter()
            .filter(|(k, v)| y.get(*k) != Some(v))
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>()
    };
    println!(
        "  links changed c1..c2: {:?}; c2..c3: {:?} -> {:?}",
        changed(&l1, &l2),
        changed(&l2, &l3),
        l3[mkey]
    );
    println!(
        "  read back: c1 == original {}, c2 == edited {}, c3 == edited {}",
        exp(&a, &c1) == orig,
        exp(&a, &c2) == want2.as_bytes(),
        exp(&a, &c3) == want3.as_bytes()
    );

    // 4. Certification: an agent submits every TPC-DS metric; only a person
    // certifies.
    let metrics: Vec<(String, Id)> = a
        .defs_list(&c1, "tpcds")
        .unwrap()
        .into_iter()
        .filter(|d| kind_of(&d.path) == "metric")
        .map(|d| (d.key(), d.id))
        .collect();
    let t = Instant::now();
    for (_, id) in &metrics {
        a.defs_submit("tpcds", id).unwrap();
    }
    let t_submit = ms(t) / metrics.len() as f64;
    let refused_auto = metrics
        .iter()
        .filter(|(_, id)| a.defs_certify_auto("tpcds", id).is_err())
        .count();
    let (fa, _) = approve("certify", "certify", &metrics[0].1.to_hex());
    let refused_forged = usize::from(a.defs_review("tpcds", &fa, &[0; 32], &KEY_A).is_err());
    let t = Instant::now();
    for (_, id) in &metrics {
        let (ap, tag) = approve("certify", "certify", &id.to_hex());
        a.defs_review("tpcds", &ap, &tag, &KEY_A).unwrap();
    }
    let t_cert = ms(t) / metrics.len() as f64;
    let count = |a: &mut Repo<FsStore>, c: &Id| {
        a.defs_certified(c, "tpcds")
            .unwrap()
            .into_iter()
            .filter(|(d, ok)| *ok && kind_of(&d.path) == "metric")
            .count()
    };
    let db = a.defs_cert_db("tpcds").unwrap();
    let replay = a
        .net_replay(&db, &ikam_kernel::semantic::certify_net())
        .unwrap()
        == a.net_marking(&db, "certify").unwrap();
    println!(
        "\ncertification: {} metrics submitted ({t_submit:.2} ms each), auto certify refused {refused_auto}/{}, forged seal refused {refused_forged}/1, certified by approval ({t_cert:.2} ms each); certified metrics at c1 {}/{}, at head (after the edit) {}/{}; replay == marking {replay}",
        metrics.len(),
        metrics.len(),
        count(&mut a, &c1),
        metrics.len(),
        count(&mut a, &c3),
        metrics.len()
    );

    // 5. Search over the definitions: keyword (BM25), vector (hashed
    // trigrams, a stand-in), fused.
    let judged: &[(&str, &str, &[&str])] = &[
        (
            "tpcds",
            "net profit",
            &["tpcds_semantic_model.yaml/metrics/total_profit"],
        ),
        (
            "tpcds",
            "lifetime value of a customer",
            &["tpcds_semantic_model.yaml/metrics/customer_lifetime_value"],
        ),
        (
            "tpcds",
            "sales per employee",
            &["tpcds_semantic_model.yaml/metrics/store_productivity"],
        ),
        (
            "tpcds",
            "running total of revenue",
            &["tpcds_semantic_model.yaml/metrics/cumulative_sales"],
        ),
        (
            "tpcds",
            "month over month change in sales",
            &["tpcds_semantic_model.yaml/metrics/monthly_sales_change"],
        ),
        (
            "tpcds",
            "top brands in each store",
            &["tpcds_semantic_model.yaml/metrics/brand_rank_in_store"],
        ),
        (
            "tpcds",
            "customer email",
            &["tpcds_semantic_model.yaml/datasets/customer/fields/c_email_address"],
        ),
        (
            "tpcds",
            "what was sold relationship",
            &["tpcds_semantic_model.yaml/relationships/store_sales_to_item"],
        ),
        (
            "flights",
            "machine capable of flight",
            &["flights.semantic_model.yaml/datasets/AIRCRAFT/_"],
        ),
        (
            "flights",
            "airport identified by IATA code",
            &["flights.semantic_model.yaml/datasets/AIRPORT/_"],
        ),
        (
            "flights",
            "departure delay",
            &["flights.semantic_model.yaml/datasets/FLIGHT/fields/dep_delay"],
        ),
        (
            "sap",
            "sales organization",
            &["sap_business_partner.ontology.yaml/ontology/SalesOrganization"],
        ),
        (
            "sap",
            "distribution channel",
            &["sap_business_partner.ontology.yaml/ontology/DistributionChannel"],
        ),
        (
            "sap",
            "full name of the business partner",
            &["sap_business_partner.ontology.yaml/ontology/PartnerName"],
        ),
    ];
    let head = a.resolve("main").unwrap();
    let mut search = BTreeMap::new();
    let t = Instant::now();
    for (set, _) in sets {
        let s = a
            .defs_search(&head, set, Box::new(|t: &str| trigram_vector(t, 256)))
            .unwrap();
        search.insert(*set, s);
    }
    let t_build = ms(t);
    let mut score: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
    let mut times = Vec::new();
    for (set, q, want) in judged {
        let s = &search[set];
        let t = Instant::now();
        let lists = [
            ("keyword", s.keyword(q, 10)),
            ("vector", s.vector(q, 10)),
            ("hybrid", s.hybrid(q, 10, 50, 0.5)),
        ];
        times.push(ms(t));
        for (name, l) in lists {
            let rank = l.iter().position(|(k, _)| want.contains(&k.as_str()));
            if rank != Some(0) {
                let got = l.first().map(|x| x.0.as_str()).unwrap_or("-");
                println!("  miss {name}: {q:?} rank {rank:?}, top {got}");
            }
            let e = score.entry(name).or_default();
            e.0 += usize::from(rank == Some(0));
            e.1 += rank.map_or(0.0, |r| 1.0 / (r as f64 + 1.0));
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "\nsearch: {} judged queries over {} definitions (index build {t_build:.1} ms; three lists per query p50 {:.2} ms)",
        judged.len(),
        search.values().map(|s| s.len()).sum::<usize>(),
        times[times.len() / 2]
    );
    for (name, (hit, rr)) in &score {
        println!(
            "  {name}: hit@1 {hit}/{}, MRR@10 {:.3}",
            judged.len(),
            rr / judged.len() as f64
        );
    }

    // 6. Sharing: b gets the TPC-DS set alone, edits one metric, and a
    // merges it back only once a person authorizes.
    let share = Share {
        name: "tpcds-defs".into(),
        from: "a".into(),
        to: "b".into(),
        commit: head,
        scope: Scope::Path("semantic/tpcds".into()),
        right: Right::Merge,
        expires: NOW + 600,
    };
    let tag = a.share_issue(&share, &KEY_A).unwrap();
    a.share_check(&share, &tag, &KEY_A, "b", Right::Read, NOW)
        .unwrap();
    let inside = a.share_files(&share).unwrap();
    let in_ok = inside
        .iter()
        .filter(|(p, _)| a.share_read(&share, p, None).is_ok())
        .count();
    let outside = [
        "data/store_sales.csv",
        "semantic/flights/flights.semantic_model.yaml/_order",
        "semantic/sap/sap_business_partner.ontology.yaml/_order",
        "semantic/tpcds-x",
        "semantic/tpcds/../flights/flights.semantic_model.yaml/_order",
    ];
    let out_refused = outside
        .iter()
        .filter(|p| a.share_read(&share, p, None) == Err(Refused::Outside))
        .count();
    let copy = a.share_import_bytes(&share).unwrap();
    let mut b = Repo::new(FsStore::open(&pb).unwrap());
    let t = Instant::now();
    let m = import_share(&mut a, &share, &mut b, "main", "take tpcds defs").unwrap();
    let t_imp = ms(t);
    b.defs_relink("main", "tpcds", "links").unwrap();
    let bh = b.resolve("main").unwrap();
    println!(
        "\nshare a -> b (path semantic/tpcds): {} files in scope, {in_ok} read; {out_refused}/{} reads outside refused; import {m:?} copied {copy} bytes in {t_imp:.1} ms; b's export == a's at the share's commit {}; b holds sets {:?}",
        inside.len(),
        outside.len(),
        b.defs_export(&bh, "tpcds", "tpcds_semantic_model.yaml")
            .unwrap()
            == exp(&a, &head),
        ["tpcds", "flights", "sap"]
            .iter()
            .filter(|s| !b.defs_docs(&bh, s).unwrap().is_empty())
            .collect::<Vec<_>>()
    );
    let back = |b: &mut Repo<FsStore>| {
        let s = Share {
            name: "tpcds-back".into(),
            from: "b".into(),
            to: "a".into(),
            commit: b.resolve("main").unwrap(),
            scope: Scope::Path("semantic/tpcds".into()),
            right: Right::Merge,
            expires: NOW + 600,
        };
        let tag = b.share_issue(&s, &KEY_B).unwrap();
        (s, tag)
    };
    // Link: the unchanged set back once (E365's import needs a base).
    let (s0, t0) = back(&mut b);
    let tok0 = a.defs_offer(&s0).unwrap();
    let r0 = a.defs_merge_in("a", &mut b, &s0, &t0, &KEY_B, "main", "m", NOW);
    let (ap, apt) = approve("merge-in", "authorize", &tok0);
    a.defs_authorize(&ap, &apt, &KEY_A).unwrap();
    let before = a.resolve("main").unwrap();
    let m0 = a
        .defs_merge_in("a", &mut b, &s0, &t0, &KEY_B, "main", "link", NOW)
        .unwrap();
    let linked = a.resolve("main").unwrap();
    println!(
        "  link step: unauthorized {r0:?}; authorized {m0:?}, fragments changed in a {}",
        a.defs_diff(&before, &linked, "tpcds").unwrap().len()
    );
    // b edits one metric.
    let bkey = "tpcds_semantic_model.yaml/metrics/customer_lifetime_value";
    let (_, bold) = frag(&b, &bh, "tpcds", bkey);
    let bnew = String::from_utf8(bold).unwrap().replace(
        "Average lifetime sales value per customer",
        "Average lifetime store sales value per distinct customer",
    );
    b.defs_edit(
        "main",
        "tpcds",
        "tpcds_semantic_model.yaml",
        "metrics/customer_lifetime_value",
        bnew.as_bytes(),
        "b edits clv",
    )
    .unwrap();
    let (s1, t1) = back(&mut b);
    let tok1 = a.defs_offer(&s1).unwrap();
    let r1 = a.defs_merge_in("a", &mut b, &s1, &t1, &KEY_B, "main", "m", NOW);
    let leaked = a.has_content(&Id::of_content(bnew.as_bytes())).unwrap();
    let unmoved = a.resolve("main").unwrap() == linked;
    let (st, stt) = approve("merge-in", "authorize", &tok0);
    a.defs_authorize(&st, &stt, &KEY_A).unwrap();
    let r_stale = a.defs_merge_in("a", &mut b, &s1, &t1, &KEY_B, "main", "m", NOW);
    let (ap, apt) = approve("merge-in", "authorize", &tok1);
    a.defs_authorize(&ap, &apt, &KEY_A).unwrap();
    let t = Instant::now();
    let objs = a.store().ids().unwrap().len();
    let m1 = a
        .defs_merge_in("a", &mut b, &s1, &t1, &KEY_B, "main", "merge b's clv", NOW)
        .unwrap();
    let t_m1 = ms(t);
    let after = a.resolve("main").unwrap();
    let objs = a.store().ids().unwrap().len() - objs;
    let diff = a.defs_diff(&linked, &after, "tpcds").unwrap();
    let same = a
        .defs_export(&after, "tpcds", "tpcds_semantic_model.yaml")
        .unwrap()
        == b.defs_export(
            &b.resolve("main").unwrap(),
            "tpcds",
            "tpcds_semantic_model.yaml",
        )
        .unwrap();
    let data_kept = has_path(&a, &after, "data/store_sales.csv");
    let again = a.defs_merge_in("a", &mut b, &s1, &t1, &KEY_B, "main", "again", NOW);
    let other_sets = ["flights", "sap"]
        .iter()
        .all(|s| a.defs_diff(&linked, &after, s).unwrap().is_empty());
    println!(
        "  b's edit: before authorization {r1:?}, b's bytes in a's store {leaked}, a's head unmoved {unmoved}; old authorization resent {r_stale:?}"
    );
    println!(
        "  authorized: {m1:?} in {t_m1:.1} ms ({objs} objects written to a); fragments changed in a {diff:?}; a's export == b's {same}; other sets unchanged {other_sets}, data file kept {data_kept}; merging again {again:?}"
    );
    let db = a.defs_merge_db().unwrap();
    let replay = a
        .net_replay(&db, &ikam_kernel::semantic::merge_net())
        .unwrap()
        == a.net_marking(&db, "merge-in").unwrap();
    println!(
        "  merge-in replay == marking {replay}; fsck a {} b {}; store bytes a {} b {}",
        a.fsck().unwrap().len(),
        b.fsck().unwrap().len(),
        du(&pa),
        du(&pb)
    );
    if let Err(MergeRefused::Share(e)) = &again {
        println!("  (merging again refused by the share: {e:?})");
    }
}
