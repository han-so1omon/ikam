//! E259: are E254's fusion settings (RRF k 60, convex alpha 0.5) right
//! for real text, and does a setting tuned on one half of the queries,
//! or on the other task, carry over?
//! `cargo run --release --features search-tantivy --example fusion_tune -- DIR`
//!
//! The lists are E258's (BM25 and whole-README WordLlama vectors, top 100
//! each). Queries are split in half by a hash of their id (`dev`, `test`).
//! For each task, convex alpha (weight on BM25) is swept from 0 to 1 by
//! 0.1 and RRF's k over 1, 10, 30, 60, 100; the best on `dev` by nDCG@10
//! is reported on `test`, beside the defaults and beside the setting
//! tuned on the other task's `dev`.

mod real_common;

use ikam_kernel::vectors::{convex, rrf};
use real_common::{Lists, Query, keys, mean, quality};

#[derive(Clone, Copy)]
enum Fuse {
    Convex(f32),
    Rrf(f32),
}

fn ranked(l: &Lists, i: usize, f: Fuse) -> Vec<String> {
    match f {
        Fuse::Convex(a) => keys(&convex(&l[i].0, &l[i].1, a)),
        Fuse::Rrf(k) => keys(&rrf(&[keys(&l[i].0), keys(&l[i].1)], k)),
    }
}

/// Mean nDCG@10 of `f` over one task's split.
fn score(qs: &[Query], l: &Lists, task: &str, split: &str, f: Fuse) -> f64 {
    let v: Vec<_> = qs
        .iter()
        .enumerate()
        .filter(|(_, q)| q.task == task && q.split == split)
        .map(|(i, q)| quality(&ranked(l, i, f), &q.relevant))
        .collect();
    mean(&v).ndcg10
}

/// The setting best on `dev`.
fn tune(qs: &[Query], l: &Lists, task: &str, opts: &[Fuse]) -> Fuse {
    *opts
        .iter()
        .max_by(|a, b| score(qs, l, task, "dev", **a).total_cmp(&score(qs, l, task, "dev", **b)))
        .unwrap()
}

fn label(f: Fuse) -> String {
    match f {
        Fuse::Convex(a) => format!("\"alpha\":{a:.1}"),
        Fuse::Rrf(k) => format!("\"k\":{k}"),
    }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let dim = real_common::dim(&dir);
    let docs = real_common::docs(&dir);
    let qs = real_common::queries(&dir);
    let qv = real_common::vectors(&format!("{dir}/queries.f32"), dim);
    let l = real_common::lists(
        &docs,
        &real_common::vectors(&format!("{dir}/doc_whole.f32"), dim),
        &qs,
        &qv,
    );
    let convexes: Vec<Fuse> = (0..=10).map(|i| Fuse::Convex(i as f32 / 10.0)).collect();
    let rrfs: Vec<Fuse> = [1.0, 10.0, 30.0, 60.0, 100.0].map(Fuse::Rrf).to_vec();
    for task in ["known", "topic"] {
        for f in convexes.iter().chain(&rrfs) {
            let way = if matches!(f, Fuse::Convex(_)) {
                "convex"
            } else {
                "rrf"
            };
            println!(
                "{{\"task\":\"{task}\",\"way\":\"{way}\",{},\"dev_ndcg_at_10\":{:.4},\"test_ndcg_at_10\":{:.4}}}",
                label(*f),
                score(&qs, &l, task, "dev", *f),
                score(&qs, &l, task, "test", *f)
            );
        }
    }
    for (task, other) in [("known", "topic"), ("topic", "known")] {
        for (way, opts, default) in [
            ("convex", &convexes, Fuse::Convex(0.5)),
            ("rrf", &rrfs, Fuse::Rrf(60.0)),
        ] {
            let (mine, theirs) = (tune(&qs, &l, task, opts), tune(&qs, &l, other, opts));
            let t = |f: Fuse| score(&qs, &l, task, "test", f);
            println!(
                "{{\"task\":\"{task}\",\"split\":\"test\",\"way\":\"{way}\",\"bm25\":{:.4},\"vectors\":{:.4},\"default\":{:.4},\"tuned\":{:.4},\"tuned_on\":{{{}}},\"other_task\":{:.4},\"other_on\":{{{}}}}}",
                t(Fuse::Convex(1.0)),
                t(Fuse::Convex(0.0)),
                t(default),
                t(mine),
                label(mine),
                t(theirs),
                label(theirs)
            );
        }
    }
}
