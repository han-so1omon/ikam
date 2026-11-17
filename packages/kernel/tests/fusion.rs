use ikam_kernel::fusion::{DEFAULT_ALPHA, STRONG_VECTORS_ALPHA, convex, hybrid, rrf};

fn list(rows: &[(&str, f32)]) -> Vec<(String, f32)> {
    rows.iter().map(|(k, s)| (k.to_string(), *s)).collect()
}

fn keys(l: &[(String, f32)]) -> Vec<&str> {
    l.iter().map(|x| x.0.as_str()).collect()
}

#[test]
fn hybrid_is_convex_at_the_default_weight() {
    let kw = list(&[("a", 12.0), ("b", 7.0), ("c", 2.0)]);
    let vec = list(&[("c", 0.9), ("d", 0.8), ("a", 0.1)]);
    assert_eq!(hybrid(&kw, &vec), convex(&kw, &vec, DEFAULT_ALPHA));
    assert_eq!(DEFAULT_ALPHA, 0.5);
    assert_eq!(STRONG_VECTORS_ALPHA, 0.4);
}

#[test]
fn convex_normalizes_each_list_and_weights_the_keyword_list() {
    let kw = list(&[("a", 10.0), ("b", 5.0), ("c", 0.0)]);
    let vec = list(&[("c", 1.0), ("b", 0.5), ("a", 0.0)]);
    let at = |alpha| convex(&kw, &vec, alpha);
    // a: alpha * 1 + 0; b: 0.5; c: (1 - alpha) * 1.
    assert_eq!(keys(&at(0.7)), ["a", "b", "c"]);
    assert_eq!(keys(&at(0.3)), ["c", "b", "a"]);
    let s = at(0.4);
    assert!((s.iter().find(|x| x.0 == "a").unwrap().1 - 0.4).abs() < 1e-6);
    assert!((s.iter().find(|x| x.0 == "b").unwrap().1 - 0.5).abs() < 1e-6);
}

#[test]
fn convex_scores_a_missing_row_zero_and_breaks_ties_by_key() {
    let kw = list(&[("b", 3.0), ("a", 1.0)]);
    let vec = list(&[("z", 0.4)]);
    // A one-row list normalizes to 0, so z ties with a.
    assert_eq!(keys(&convex(&kw, &vec, 0.5)), ["b", "a", "z"]);
    assert!(convex(&[], &[], 0.5).is_empty());
}

#[test]
fn rrf_sums_reciprocal_ranks() {
    let out = rrf(
        &[vec!["a".into(), "b".into()], vec!["b".into(), "c".into()]],
        60.0,
    );
    assert_eq!(keys(&out), ["b", "a", "c"]);
    assert!((out[0].1 - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-7);
}
