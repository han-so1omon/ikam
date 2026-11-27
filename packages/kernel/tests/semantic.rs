//! Semantic definitions as versioned artifacts (E419): Ossie-shaped YAML
//! cut into one fragment per definition, exported byte for byte, edited
//! one definition at a time, certified only by a person, shared to a
//! second project and merged back only once a person authorizes it.
//! The documents here are small synthetic ones in Ossie's shape.

use ikam_kernel::merge::Merged;
use ikam_kernel::semantic::{
    Approval, MergeRefused, certify_net, kind_of, merge_net, segment, share_token, trigram_vector,
};
use ikam_kernel::share::{Refused, Right, Scope, Share, import_share};
use ikam_kernel::{Edit, Id, MemStore, Object, Repo};
use proptest::prelude::*;

const MODEL: &str = r#"# A small synthetic shop model, in Ossie's shape.
version: "0.3.0.dev0"

name: shop_model
description: >
  A synthetic shop model.

  Second paragraph of the description.
datasets:
  # Orders fact table
  - name: orders
    source: shop.public.orders
    primary_key: [order_id]
    unique_keys:
      - [order_id, line_no]  # composite key: order and line
    fields:
      - name: order_id
        expression:
          sql: order_id
        datatype: Integer
      # The customer who ordered
      - name: customer_id
        expression:
          sql: customer_id
        datatype: Integer
      - name: amount
        expression:
          sql: amount
        description: |
          Order amount.

          In cents.
        datatype: Decimal
    ai_context:
      synonyms:
        - "sales"
  - name: customers
    source: shop.public.customers
    primary_key: [ customer_id ]
    fields:
    - name: customer_id
      expression:
        sql: id
    - name: region
      expression:
        sql: region
relationships:
  - name: orders_to_customers
    from: orders
    to: customers
    from_columns: [customer_id]
    to_columns:
      - customer_id
metrics:
  - name: total_amount
    expression:
      sql: SUM(orders.amount)
    description: Total order amount
  # Per-customer value
  - name: amount_per_customer
    expression:
      sql: SUM(orders.amount) / COUNT(DISTINCT customers.customer_id)
    description: Average amount per customer
nonportable_extensions:
  - vendor_name: EXAMPLE
    data:
      x: 1
"#;

const ONTOLOGY: &str = "version: 0.3.0.dev0
name: Shop
ontology:
- concept: Customer
  type: EntityType
  identify_by: [ id ]
- concept: Region
  type: ValueType
  extends: [ String ]";

const MAPPING: &str = "version: 0.3.0.dev0
name: shop_mapping
concept_mappings:
- concept: Customer
  object_mappings:
  - referent_mappings:
    - relationship: id
      expression: customers.customer_id
";

const KEY: [u8; 32] = [9; 32];
const KEY_B: [u8; 32] = [5; 32];
const NOW: u64 = 1_000;

fn docs() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("shop.yaml", MODEL.as_bytes()),
        ("shop.ontology.yaml", ONTOLOGY.as_bytes()),
        ("shop.mapping.yaml", MAPPING.as_bytes()),
    ]
}

fn tree(r: &Repo<MemStore>, c: &Id) -> Id {
    match r.get(c).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    }
}

fn entry(r: &Repo<MemStore>, c: &Id, path: &str) -> Option<Id> {
    let mut id = tree(r, c);
    for part in path.split('/') {
        let Object::Tree(es) = r.get(&id).unwrap() else {
            return None;
        };
        id = es.into_iter().find(|e| e.name == part)?.id;
    }
    Some(id)
}

fn fragment(r: &Repo<MemStore>, c: &Id, set: &str, key: &str) -> Vec<u8> {
    let d = r
        .defs_list(c, set)
        .unwrap()
        .into_iter()
        .find(|d| d.key() == key)
        .unwrap();
    r.read_content(&d.id).unwrap()
}

fn approve(net: &str, transition: &str, token: &str) -> (Approval, [u8; 32]) {
    let a = Approval {
        person: "owner".into(),
        net: net.into(),
        transition: transition.into(),
        token: token.into(),
    };
    let tag = a.seal(&KEY);
    (a, tag)
}

#[test]
fn segment_cuts_one_fragment_per_definition() {
    let spans = segment(MODEL.as_bytes());
    let paths: Vec<&str> = spans.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "_model/version",
            "_model/name",
            "_model/description",
            "datasets/_",
            "datasets/orders/_",
            "datasets/orders/fields/order_id",
            "datasets/orders/fields/customer_id",
            "datasets/orders/fields/amount",
            "datasets/orders/_rest",
            "datasets/customers/_",
            "datasets/customers/fields/customer_id",
            "datasets/customers/fields/region",
            "relationships/_",
            "relationships/orders_to_customers",
            "metrics/_",
            "metrics/total_amount",
            "metrics/amount_per_customer",
            "_model/nonportable_extensions",
        ]
    );
    let b = MODEL.as_bytes();
    let text = |p: &str| {
        let s = spans.iter().find(|s| s.path == p).unwrap();
        std::str::from_utf8(&b[s.start..s.end]).unwrap()
    };
    // Comments go with the definition below them; a block scalar's blank
    // line stays inside its definition.
    assert!(text("_model/version").starts_with("# A small synthetic"));
    assert!(text("datasets/orders/_").starts_with("  # Orders fact table\n  - name: orders"));
    assert!(text("datasets/orders/fields/customer_id").starts_with("      # The customer"));
    assert!(text("metrics/amount_per_customer").starts_with("  # Per-customer value"));
    assert!(text("datasets/orders/fields/amount").contains("Order amount.\n\n          In cents."));
    assert!(text("datasets/orders/_rest").starts_with("    ai_context:"));
    assert_eq!(kind_of("datasets/orders/_rest"), "dataset");
    assert_eq!(kind_of("datasets/orders/fields/amount"), "field");
    assert_eq!(kind_of("metrics/total_amount"), "metric");
    let joined: Vec<u8> = spans
        .iter()
        .flat_map(|s| b[s.start..s.end].to_vec())
        .collect();
    assert_eq!(joined, b);
    // Zero-indented sequences, no trailing newline.
    let o: Vec<String> = segment(ONTOLOGY.as_bytes())
        .into_iter()
        .map(|s| s.path)
        .collect();
    assert_eq!(
        o,
        [
            "_model/version",
            "_model/name",
            "ontology/_",
            "ontology/Customer",
            "ontology/Region"
        ]
    );
    // Odd input: one fragment, still exact.
    assert_eq!(segment(b"").len(), 1);
    assert_eq!(segment(b"- a\n- b\n")[0].path, "_model/_doc");
}

fn check_cover(b: &[u8]) {
    let spans = segment(b);
    assert!(!spans.is_empty());
    assert_eq!(spans[0].start, 0);
    assert_eq!(spans.last().unwrap().end, b.len());
    for w in spans.windows(2) {
        assert_eq!(w[0].end, w[1].start);
        assert!(w[0].start < w[0].end || b.is_empty());
    }
    let mut paths: Vec<&String> = spans.iter().map(|s| &s.path).collect();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), spans.len(), "paths are unique");
}

proptest! {
    #[test]
    fn segments_cover_any_bytes(b in proptest::collection::vec(any::<u8>(), 0..400)) {
        check_cover(&b);
    }

    #[test]
    fn segments_cover_any_yaml_like_lines(
        ls in proptest::collection::vec(
            prop::sample::select(vec![
                "datasets:", "metrics:", "fields:", "  fields:", "- name: a", "  - name: a",
                "    - name: a", "      - name: b", "  name: x", "    sql: a.b", "# c", "  # c",
                "", "   ", "key: v", "  - [a, b]", "-", "  -", "name: _", "  - name: _rest",
                "\t- x", "ontology:", "- concept: C", "  x: |", "    text",
            ]),
            0..60,
        ),
        tail in any::<bool>(),
    ) {
        let mut s = ls.join("\n");
        if tail { s.push('\n'); }
        check_cover(s.as_bytes());
        let mut r = Repo::new(MemStore::default());
        let c = r.defs_import("main", "p", &[("d.yaml", s.as_bytes())], "p").unwrap();
        prop_assert_eq!(r.defs_export(&c, "p", "d.yaml").unwrap(), s.as_bytes());
    }
}

#[test]
fn import_exports_exactly_and_an_edit_changes_one_fragment() {
    let mut r = Repo::new(MemStore::default());
    let c1 = r.defs_import("main", "shop", &docs(), "import").unwrap();
    for (name, bytes) in docs() {
        assert_eq!(r.defs_export(&c1, "shop", name).unwrap(), bytes);
    }
    let links = r.defs_links(&c1, "shop").unwrap();
    let cols = |k: &str| links.get(k).unwrap().clone();
    assert_eq!(
        cols("shop.yaml/datasets/orders/fields/amount"),
        ["orders/amount"]
    );
    assert_eq!(
        cols("shop.yaml/datasets/customers/fields/customer_id"),
        ["customers/customer_id"]
    );
    assert_eq!(
        cols("shop.yaml/datasets/orders/_"),
        ["orders/line_no", "orders/order_id"]
    );
    assert_eq!(
        cols("shop.yaml/relationships/orders_to_customers"),
        ["customers/customer_id", "orders/customer_id"]
    );
    assert_eq!(
        cols("shop.yaml/metrics/amount_per_customer"),
        ["customers/customer_id", "orders/amount"]
    );
    assert_eq!(
        cols("shop.mapping.yaml/concept_mappings/Customer"),
        ["customers/customer_id"]
    );
    // A text-only edit: one fragment changes, the links graph does not.
    let key = "shop.yaml/metrics/total_amount";
    let old = fragment(&r, &c1, "shop", key);
    let new = String::from_utf8(old.clone())
        .unwrap()
        .replace("Total order amount", "Total order amount, all regions");
    let c2 = r
        .defs_edit(
            "main",
            "shop",
            "shop.yaml",
            "metrics/total_amount",
            new.as_bytes(),
            "edit",
        )
        .unwrap();
    assert_eq!(r.defs_diff(&c1, &c2, "shop").unwrap(), [key]);
    assert_eq!(
        entry(&r, &c1, "semantic/shop/_links"),
        entry(&r, &c2, "semantic/shop/_links")
    );
    assert_eq!(
        r.defs_export(&c1, "shop", "shop.yaml").unwrap(),
        MODEL.as_bytes()
    );
    assert_eq!(
        r.defs_export(&c2, "shop", "shop.yaml").unwrap(),
        MODEL
            .replace("Total order amount\n", "Total order amount, all regions\n")
            .as_bytes()
    );
    // An edit of what a metric names changes its links.
    let new2 = new.replace(
        "SUM(orders.amount)",
        "SUM(orders.amount) - SUM(orders.order_id)",
    );
    let c3 = r
        .defs_edit(
            "main",
            "shop",
            "shop.yaml",
            "metrics/total_amount",
            new2.as_bytes(),
            "e2",
        )
        .unwrap();
    assert_eq!(
        r.defs_links(&c3, "shop").unwrap()[key],
        ["orders/amount", "orders/order_id"]
    );
    assert_eq!(r.defs_links(&c1, "shop").unwrap()[key], ["orders/amount"]);
    assert!(
        r.defs_edit("main", "shop", "shop.yaml", "metrics/nope", b"x", "x")
            .is_err()
    );
    assert!(r.fsck().unwrap().is_empty());
}

#[test]
fn certification_fires_only_with_a_persons_approval() {
    let mut r = Repo::new(MemStore::default());
    let c1 = r.defs_import("main", "shop", &docs(), "import").unwrap();
    let key = "shop.yaml/metrics/total_amount";
    let id = r
        .defs_list(&c1, "shop")
        .unwrap()
        .into_iter()
        .find(|d| d.key() == key)
        .unwrap()
        .id;
    r.defs_submit("shop", &id).unwrap();
    // A scheduler never sees the human transitions, and cannot fire one.
    let db = r.defs_cert_db("shop").unwrap();
    let auto = r.net_enabled(&db, &certify_net(), true).unwrap();
    assert!(auto.iter().all(|(t, _)| t != "certify" && t != "reject"));
    assert!(
        r.net_enabled(&db, &certify_net(), false)
            .unwrap()
            .iter()
            .any(|(t, _)| t == "certify")
    );
    assert!(r.defs_certify_auto("shop", &id).is_err());
    // A forged seal, or an approval of another version, is refused.
    let (a, _) = approve("certify", "certify", &id.to_hex());
    assert!(r.defs_review("shop", &a, &[0; 32], &KEY).is_err());
    let (other, otag) = approve("certify", "certify", &Id::of_content(b"x").to_hex());
    assert!(
        r.defs_review("shop", &other, &otag, &KEY).is_err(),
        "not submitted"
    );
    let (a, tag) = approve("certify", "certify", &id.to_hex());
    let f = r.defs_review("shop", &a, &tag, &KEY).unwrap();
    assert_eq!(f.transition, "certify");
    let certified = |r: &mut Repo<MemStore>, c: &Id| {
        r.defs_certified(c, "shop")
            .unwrap()
            .into_iter()
            .find(|(d, _)| d.key() == key)
            .unwrap()
            .1
    };
    assert!(certified(&mut r, &c1));
    // An edit is a new version, not certified; the old commit's still is.
    let new = String::from_utf8(fragment(&r, &c1, "shop", key))
        .unwrap()
        .replace("Total", "All");
    let c2 = r
        .defs_edit(
            "main",
            "shop",
            "shop.yaml",
            "metrics/total_amount",
            new.as_bytes(),
            "e",
        )
        .unwrap();
    assert!(!certified(&mut r, &c2));
    assert!(certified(&mut r, &c1));
    // Who certified is recorded; replay equals the marking.
    let db = r.defs_cert_db("shop").unwrap();
    let out = r.net_output(&db, "certify", &f.key).unwrap().unwrap().1;
    assert_eq!(out, a.encode());
    assert_eq!(
        r.net_replay(&db, &certify_net()).unwrap(),
        r.net_marking(&db, "certify").unwrap()
    );
}

/// Project "a" with a definition set and other files; a share of the set
/// alone to project "b".
fn two_projects() -> (Repo<MemStore>, Repo<MemStore>, Share) {
    let mut a = Repo::new(MemStore::default());
    a.defs_import("main", "shop", &docs(), "import").unwrap();
    a.defs_import("main", "other", &[("o.yaml", b"name: other\n")], "other")
        .unwrap();
    let t = tree(&a, &a.resolve("main").unwrap());
    let id = a.put_content(b"id,amount\n1,10\n").unwrap().id;
    let t = a
        .edit_tree(
            Some(t),
            &[Edit::Put {
                path: "data/orders.csv".into(),
                id,
            }],
        )
        .unwrap();
    let c = a.commit("main", t, "data").unwrap();
    let share = Share {
        name: "shop-defs".into(),
        from: "a".into(),
        to: "b".into(),
        commit: c,
        scope: Scope::Path("semantic/shop".into()),
        right: Right::Merge,
        expires: NOW + 60,
    };
    a.share_issue(&share, &KEY).unwrap();
    (a, Repo::new(MemStore::default()), share)
}

fn back_share(b: &mut Repo<MemStore>) -> (Share, [u8; 32]) {
    let s = Share {
        name: "shop-back".into(),
        from: "b".into(),
        to: "a".into(),
        commit: b.resolve("main").unwrap(),
        scope: Scope::Path("semantic/shop".into()),
        right: Right::Merge,
        expires: NOW + 60,
    };
    let tag = b.share_issue(&s, &KEY_B).unwrap();
    (s, tag)
}

#[test]
fn a_shared_definition_set_merges_back_only_when_authorized() {
    let (mut a, mut b, s) = two_projects();
    let tag = s.seal(&KEY);
    a.share_check(&s, &tag, &KEY, "b", Right::Read, NOW)
        .unwrap();
    // Reads inside the scope only.
    assert!(
        a.share_read(&s, "semantic/shop/shop.yaml/metrics/total_amount", None)
            .is_ok()
    );
    for p in [
        "data/orders.csv",
        "semantic/other/o.yaml/_model/name",
        "semantic/shopx",
    ] {
        assert_eq!(a.share_read(&s, p, None), Err(Refused::Outside), "{p}");
    }
    assert!(matches!(
        import_share(&mut a, &s, &mut b, "main", "take").unwrap(),
        Merged::FastForward(_)
    ));
    b.defs_relink("main", "shop", "links").unwrap();
    let bh = b.resolve("main").unwrap();
    assert_eq!(
        b.defs_export(&bh, "shop", "shop.yaml").unwrap(),
        MODEL.as_bytes()
    );
    assert!(b.defs_docs(&bh, "other").unwrap().is_empty());
    assert!(entry(&b, &bh, "data").is_none());
    // Link the back share once, unchanged (E365's import needs a base).
    let (s0, t0) = back_share(&mut b);
    let tok0 = a.defs_offer(&s0).unwrap();
    assert_eq!(
        a.defs_merge_in("a", &mut b, &s0, &t0, &KEY_B, "main", "m", NOW),
        Err(MergeRefused::NotAuthorized)
    );
    let (ap, apt) = approve("merge-in", "authorize", &tok0);
    a.defs_authorize(&ap, &apt, &KEY).unwrap();
    let before = a.resolve("main").unwrap();
    assert!(matches!(
        a.defs_merge_in("a", &mut b, &s0, &t0, &KEY_B, "main", "link", NOW)
            .unwrap(),
        Merged::Commit(_)
    ));
    let linked = a.resolve("main").unwrap();
    assert!(a.defs_diff(&before, &linked, "shop").unwrap().is_empty());
    // The receiver edits one metric; it stays there until authorized.
    let key = "shop.yaml/metrics/amount_per_customer";
    let new = String::from_utf8(fragment(&b, &bh, "shop", key))
        .unwrap()
        .replace("Average amount per customer", "Mean amount per customer");
    b.defs_edit(
        "main",
        "shop",
        "shop.yaml",
        "metrics/amount_per_customer",
        new.as_bytes(),
        "e",
    )
    .unwrap();
    let (s1, t1) = back_share(&mut b);
    let tok1 = a.defs_offer(&s1).unwrap();
    assert_ne!(tok1, tok0);
    assert_eq!(
        a.defs_merge_in("a", &mut b, &s1, &t1, &KEY_B, "main", "m", NOW),
        Err(MergeRefused::NotAuthorized)
    );
    assert!(!a.has_content(&Id::of_content(new.as_bytes())).unwrap());
    assert_eq!(a.resolve("main").unwrap(), linked);
    // The old share's authorization, sent again, is only that firing again
    // (idempotent) and does not cover the new share; a scheduler cannot
    // authorize.
    let (stale, stag) = approve("merge-in", "authorize", &tok0);
    a.defs_authorize(&stale, &stag, &KEY).unwrap();
    assert_eq!(
        a.defs_merge_in("a", &mut b, &s1, &t1, &KEY_B, "main", "m", NOW),
        Err(MergeRefused::NotAuthorized)
    );
    let db = a.defs_merge_db().unwrap();
    let auto = a.net_enabled(&db, &merge_net(), true).unwrap();
    assert!(auto.iter().all(|(t, _)| t != "authorize"));
    // Authorized: exactly that fragment arrives.
    let (ap, apt) = approve("merge-in", "authorize", &tok1);
    a.defs_authorize(&ap, &apt, &KEY).unwrap();
    assert!(matches!(
        a.defs_merge_in("a", &mut b, &s1, &t1, &KEY_B, "main", "merge", NOW)
            .unwrap(),
        Merged::Commit(_)
    ));
    let after = a.resolve("main").unwrap();
    assert_eq!(a.defs_diff(&linked, &after, "shop").unwrap(), [key]);
    assert_eq!(
        a.defs_export(&after, "shop", "shop.yaml").unwrap(),
        b.defs_export(&b.resolve("main").unwrap(), "shop", "shop.yaml")
            .unwrap()
    );
    assert!(entry(&a, &after, "data/orders.csv").is_some());
    assert_eq!(
        a.defs_relink("main", "shop", "l").unwrap(),
        after,
        "links unchanged"
    );
    // The authorization is used up.
    assert_eq!(
        a.defs_merge_in("a", &mut b, &s1, &t1, &KEY_B, "main", "again", NOW),
        Err(MergeRefused::NotAuthorized)
    );
    // A revoked share is refused even when authorized.
    b.defs_edit(
        "main",
        "shop",
        "shop.yaml",
        "metrics/total_amount",
        b"  - name: t\n",
        "e",
    )
    .unwrap();
    let (s2, t2) = back_share(&mut b);
    let tok2 = a.defs_offer(&s2).unwrap();
    let (ap, apt) = approve("merge-in", "authorize", &tok2);
    a.defs_authorize(&ap, &apt, &KEY).unwrap();
    b.share_revoke("shop-back").unwrap();
    assert_eq!(
        a.defs_merge_in("a", &mut b, &s2, &t2, &KEY_B, "main", "m", NOW),
        Err(MergeRefused::Share(Refused::Revoked))
    );
    let db = a.defs_merge_db().unwrap();
    assert_eq!(
        a.net_replay(&db, &merge_net()).unwrap(),
        a.net_marking(&db, "merge-in").unwrap()
    );
    assert_eq!(share_token(&s1), tok1);
    assert!(a.fsck().unwrap().is_empty());
    assert!(b.fsck().unwrap().is_empty());
}

#[test]
fn hybrid_search_finds_definitions() {
    let mut r = Repo::new(MemStore::default());
    let c = r.defs_import("main", "shop", &docs(), "import").unwrap();
    let s = r
        .defs_search(&c, "shop", Box::new(|t: &str| trigram_vector(t, 256)))
        .unwrap();
    assert_eq!(s.len(), r.defs_list(&c, "shop").unwrap().len());
    let top = |l: Vec<(String, f32)>| l.into_iter().next().unwrap().0;
    assert_eq!(
        top(s.keyword("average amount per customer", 5)),
        "shop.yaml/metrics/amount_per_customer"
    );
    assert_eq!(
        top(s.hybrid("average amount per customer", 5, 20, 0.5)),
        "shop.yaml/metrics/amount_per_customer"
    );
    // Spelling-near words: the vector side still finds it.
    assert_eq!(
        top(s.vector("customer averages", 5))
            .split('/')
            .next()
            .unwrap(),
        "shop.yaml"
    );
}
