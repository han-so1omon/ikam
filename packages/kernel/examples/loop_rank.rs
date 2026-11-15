//! E276: does the experiment loop's ranking rule order a queue better than
//! the order the experiments were run in?
//!
//! Backtest on the registry and log. At each cutoff, the queue is the next
//! WINDOW experiments that ended in a keep-or-discard decision (kept against
//! discarded, refuted, dropped, partial, not applicable or inconclusive;
//! benchmarks and plain measurements are left out, since they always end
//! "measured"). Each rule orders the queue using only what was recorded up
//! to the cutoff:
//!
//! - `actual`: the order they were run in.
//! - `score`: the plan's value-per-hour rule with what the records hold
//!   ahead of time: P(keep) as the mean over the experiment's mechanisms of
//!   each one's Beta(1, 1) posterior mean from the verdicts so far, plus 0.5 x
//!   info, where info is 1 for an id named in the log ahead of time (E275's
//!   planned gaps). Gain and hours are not recorded ahead of time, so they
//!   are 1.
//! - `p-keep`: the posterior alone. `planned`: planned ids first, then
//!   actual order. `weighty`: experiments touching a WEIGHTY mechanism first.
//! - `thompson`: one draw from each experiment's Beta posterior (fixed seed).
//! - `random`: the mean over SHUFFLES seeded shuffles.
//!
//! Metric: where the queue's non-kept experiments land, as the mean of
//! position / (n - 1) over them (0 = first, 1 = last). A rule that runs the
//! failures last scores near 1; a random order 0.5. And kept in the first
//! FIRST: how many of the first FIRST picks were kept.
//!
//!   cargo run --release --example loop_rank
//!
//! Deterministic. No kernel change, nothing written.

#[path = "loop_common/mod.rs"]
mod loop_common;

use std::collections::BTreeMap;

use loop_common::*;

const WINDOW: usize = 20;
const STEP: u32 = 10;
const FIRST: usize = 5;
const SHUFFLES: u64 = 200;

const DECISIONS: [&str; 8] = [
    "kept",
    "discarded",
    "refuted",
    "dropped",
    "partial",
    "not-applicable",
    "conditional",
    "inconclusive",
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Gamma(k, 1) for integer k >= 1 (sum of exponentials).
    fn gamma_int(&mut self, k: u32) -> f64 {
        (0..k).map(|_| -(1.0 - self.unit()).ln()).sum()
    }
    fn beta_int(&mut self, a: u32, b: u32) -> f64 {
        let x = self.gamma_int(a);
        let y = self.gamma_int(b);
        x / (x + y)
    }
}

/// Kept and decided counts per mechanism, from verdicts up to the cutoff.
fn history(rows: &[Row], cutoff: u32) -> BTreeMap<String, (u32, u32)> {
    let mut h: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.id <= cutoff && decided(r)) {
        for m in &r.mechanisms {
            let e = h.entry(m.clone()).or_default();
            e.0 += (r.verdict == "kept") as u32;
            e.1 += 1;
        }
    }
    h
}

fn decided(r: &Row) -> bool {
    r.kind != "benchmark" && DECISIONS.contains(&r.verdict.as_str())
}

/// Posterior counts (alpha, beta) for an experiment's Thompson draw:
/// Beta(1, 1) plus its mechanisms' counts, averaged and rounded down.
fn posterior(r: &Row, h: &BTreeMap<String, (u32, u32)>) -> (u32, u32) {
    let (mut k, mut n) = (0, 0);
    for m in &r.mechanisms {
        if let Some(&(km, nm)) = h.get(m) {
            k += km;
            n += nm;
        }
    }
    let m = r.mechanisms.len().max(1) as u32;
    // Average over mechanisms, rounded, so one experiment with many tags is
    // not weighed as many.
    (1 + k / m, 1 + (n - k) / m)
}

/// Mean normalized position of the non-kept experiments (0 first, 1 last),
/// and kept among the first FIRST. None when the queue has no failure.
fn measure(order: &[&Row]) -> (Option<f64>, usize) {
    let n = order.len();
    let fails: Vec<f64> = (order.iter().enumerate())
        .filter(|(_, r)| r.verdict != "kept")
        .map(|(i, _)| i as f64 / (n - 1).max(1) as f64)
        .collect();
    let mean = (!fails.is_empty()).then(|| fails.iter().sum::<f64>() / fails.len() as f64);
    let kept_first = order
        .iter()
        .take(FIRST)
        .filter(|r| r.verdict == "kept")
        .count();
    (mean, kept_first)
}

#[derive(Default)]
struct Acc {
    fail_pos: f64,
    fail_n: usize,
    kept_first: usize,
}

fn main() {
    let docs = docs();
    let rows = rows(&docs);
    let entries = entries(&docs);
    let mut rows_sorted: Vec<&Row> = rows.iter().filter(|r| decided(r)).collect();
    rows_sorted.sort_by_key(|r| r.id);

    let rules = [
        "actual", "score", "p-keep", "planned", "weighty", "thompson", "random",
    ];
    let mut acc: BTreeMap<&str, Acc> = BTreeMap::new();
    let mut queues = 0;
    let mut picks = 0;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let max = rows_sorted.last().map(|r| r.id).unwrap_or(0);
    let mut c = 50;
    while c < max {
        let queue: Vec<&Row> = (rows_sorted.iter().copied())
            .filter(|r| r.id > c)
            .take(WINDOW)
            .collect();
        if queue.len() < WINDOW / 2 {
            break;
        }
        let h = history(&rows, c);
        let planned: Vec<u32> = (gaps(&rows, &entries, &[], Some(c)).into_iter())
            .filter(|g| g.source == Source::Planned)
            .filter_map(|g| g.from)
            .collect();
        // Mean over the experiment's mechanisms of each one's Beta(1, 1)
        // posterior mean.
        let pk = |r: &Row| {
            let ms = r.mechanisms.len().max(1) as f64;
            (r.mechanisms.iter())
                .map(|m| {
                    let (k, n) = h.get(m).copied().unwrap_or((0, 0));
                    (k + 1) as f64 / (n + 2) as f64
                })
                .sum::<f64>()
                / ms
        };
        let info = |r: &Row| planned.contains(&r.id) as u32 as f64;
        let score = |r: &Row| pk(r) + 0.5 * info(r);
        let ordered = |key: &dyn Fn(&Row) -> f64| {
            let mut q = queue.clone();
            // Highest first; ties keep the actual order (stable sort).
            q.sort_by(|a, b| key(b).partial_cmp(&key(a)).unwrap());
            q
        };
        let mut row_fail = |rule: &'static str, order: &[&Row]| {
            let (m, k) = measure(order);
            let a = acc.entry(rule).or_default();
            if let Some(m) = m {
                a.fail_pos += m;
                a.fail_n += 1;
            }
            a.kept_first += k;
        };
        row_fail("actual", &queue);
        row_fail("score", &ordered(&score));
        row_fail("p-keep", &ordered(&pk));
        row_fail("planned", &ordered(&info));
        row_fail("weighty", &ordered(&|r: &Row| r.weighty() as u32 as f64));
        let draws: BTreeMap<u32, f64> = (queue.iter())
            .map(|r| {
                let (a, b) = posterior(r, &h);
                (r.id, rng.beta_int(a, b))
            })
            .collect();
        row_fail("thompson", &ordered(&|r: &Row| draws[&r.id]));
        // Random: mean over seeded shuffles.
        let (mut fp, mut fn_, mut kf) = (0.0, 0usize, 0usize);
        for _ in 0..SHUFFLES {
            let mut q = queue.clone();
            for i in (1..q.len()).rev() {
                let j = (rng.next() % (i as u64 + 1)) as usize;
                q.swap(i, j);
            }
            let (m, k) = measure(&q);
            if let Some(m) = m {
                fp += m;
                fn_ += 1;
            }
            kf += k;
        }
        let a = acc.entry("random").or_default();
        if fn_ > 0 {
            a.fail_pos += fp / fn_ as f64;
            a.fail_n += 1;
        }
        a.kept_first += (kf as f64 / SHUFFLES as f64).round() as usize;
        let fails = queue.iter().filter(|r| r.verdict != "kept").count();
        println!(
            "cutoff E{c:03}: queue E{:03} to E{:03}, {} decided, {fails} not kept, {} planned in it",
            queue[0].id,
            queue[queue.len() - 1].id,
            queue.len(),
            queue.iter().filter(|r| planned.contains(&r.id)).count()
        );
        queues += 1;
        picks += FIRST.min(queue.len());
        c += STEP;
    }
    let decided_n = rows_sorted.len();
    let kept_n = rows_sorted.iter().filter(|r| r.verdict == "kept").count();
    println!(
        "{decided_n} decided experiments, {kept_n} kept ({:.0}%); {queues} queues of up to {WINDOW}",
        100.0 * kept_n as f64 / decided_n.max(1) as f64
    );
    println!("rule      failure position (0 first, 1 last)   kept in the first {FIRST}");
    for rule in rules {
        let a = &acc[rule];
        println!(
            "{rule:<9} {:.3} over {} queues with a failure        {} of {picks}",
            a.fail_pos / a.fail_n.max(1) as f64,
            a.fail_n,
            a.kept_first
        );
    }
}
