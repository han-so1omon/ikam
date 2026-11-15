//! E275: the experiment loop's gap list, from the project's own records.
//!
//! Reads `docs/experiments/registry.tsv`, `docs/experiments/log.md` and the
//! research notes, lists what they say is worth an experiment and has not
//! had one (stale verdicts, "Not measured" lines, open hypotheses, ids
//! mentioned but never recorded), commits the list to an IKAM store as a
//! versioned graph, and reads it back.
//!
//! Then a hindsight check: at each cutoff, the list built from the log as it
//! stood then is compared with the experiments that came next. A listed gap
//! counts as taken up when a later entry is the planned id, or cites the
//! gap's experiment and shares at least three content words with the gap's
//! line. A swap control reruns the word test with every gap's line swapped
//! for another's (as E051's swap test): if it scores the same, the words
//! carry nothing.
//!
//!   cargo run --release --example loop_gaps [-- --json] [-- STORE_DIR]
//!
//! STORE_DIR defaults to target/loop.ikam; the list is committed to the ref
//! `loop` only when it changed. No kernel code changes.

#[path = "loop_common/mod.rs"]
mod loop_common;

use std::collections::BTreeMap;

use ikam_kernel::{Arg, Edge, FsStore, Kind, Node, Object, Repo, Target, TreeEntry, node_key};
use loop_common::*;

/// How many later experiments a cutoff's list is compared with.
const WINDOW: u32 = 20;
/// Cutoffs every STEP ids.
const STEP: u32 = 10;
/// Content words a later entry must share with a "Not measured" line.
const SHARED_WORDS: usize = 3;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let store = (args.iter().find(|a| !a.starts_with("--")).cloned())
        .unwrap_or_else(|| format!("{}/target/loop.ikam", env!("CARGO_MANIFEST_DIR")));

    let docs = docs();
    let rows = rows(&docs);
    let entries = entries(&docs);
    let notes = open_hypotheses(&docs);
    let gaps = gaps(&rows, &entries, &notes, None);

    // Commit the list as a graph: one node per gap (its line as content),
    // edges to its source kind, the experiment it comes from and that
    // experiment's mechanisms.
    let mut repo = Repo::new(FsStore::open(&store).unwrap());
    let node = |label: String, target: Target, edges: Vec<(&str, String)>| Node {
        label,
        target,
        edges: (edges.into_iter())
            .map(|(l, to)| Edge {
                to: node_key(&to),
                label: l.to_string(),
            })
            .collect(),
    };
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    for g in &gaps {
        let content = repo.put_content(g.text.as_bytes()).unwrap().id;
        let mut edges = vec![("source", format!("source/{}", g.source.name()))];
        if let Some(f) = g.from {
            edges.push(("from", format!("experiment/E{f:03}")));
        }
        edges.extend(
            g.mechanisms
                .iter()
                .map(|m| ("touches", format!("mechanism/{m}"))),
        );
        for (_, to) in &edges {
            nodes
                .entry(to.clone())
                .or_insert_with(|| node(to.clone(), Target::None, vec![]));
        }
        let label = format!("gap/{}", g.label);
        nodes.insert(
            label.clone(),
            node(label, Target::Content(Arg::Whole(content)), edges),
        );
    }
    let graph = repo.put_graph(nodes.into_values().collect()).unwrap();
    let tree = (repo.put(
        &Object::tree(vec![TreeEntry {
            name: "gaps".into(),
            kind: Kind::Graph,
            id: graph,
        }])
        .unwrap(),
    ))
    .unwrap();
    let head = repo.resolve("loop").ok();
    let unchanged =
        head.is_some_and(|h| matches!(repo.get(&h), Ok(Object::Commit(c)) if c.tree == tree));
    if !unchanged {
        repo.commit("loop", tree, "experiment loop: gap list")
            .unwrap();
    }
    let stored = repo.graph_nodes(&graph).unwrap();
    let stored_gaps = stored
        .iter()
        .filter(|n| n.label.starts_with("gap/"))
        .count();
    assert_eq!(stored_gaps, gaps.len(), "stored gap count differs");
    assert!(repo.fsck().unwrap().is_empty(), "loop store fails fsck");

    if json {
        for g in &gaps {
            println!(
                "{{\"label\":{},\"source\":\"{}\",\"from\":{},\"mechanisms\":[{}],\"text\":{}}}",
                json_str(&g.label),
                g.source.name(),
                g.from
                    .map(|f| format!("\"E{f:03}\""))
                    .unwrap_or("null".into()),
                g.mechanisms
                    .iter()
                    .map(|m| json_str(m))
                    .collect::<Vec<_>>()
                    .join(","),
                json_str(&g.text)
            );
        }
    }

    let mut by_source: BTreeMap<&str, usize> = BTreeMap::new();
    for g in &gaps {
        *by_source.entry(g.source.name()).or_default() += 1;
    }
    let weighty = (gaps.iter())
        .filter(|g| g.mechanisms.iter().any(|m| WEIGHTY.contains(&m.as_str())))
        .count();
    println!(
        "store {store}: graph {graph}, {} versions of ref loop",
        repo.log(repo.resolve("loop").unwrap()).unwrap().len()
    );
    println!(
        "gaps now: {} ({by_source:?}); {weighty} touch a weighty mechanism; from {} registry rows, {} log entries, {} open hypotheses",
        gaps.len(),
        rows.len(),
        entries.len(),
        notes.len()
    );
    for src in [
        Source::Planned,
        Source::Stale,
        Source::NotMeasured,
        Source::OpenHypothesis,
    ] {
        let list: Vec<&str> = (gaps.iter())
            .filter(|g| g.source == src)
            .map(|g| g.label.as_str())
            .take(12)
            .collect();
        println!("  {}: {}", src.name(), list.join("; "));
    }

    // Hindsight: would the list have pointed at what was run next?
    let max = entries.keys().copied().max().unwrap_or(0);
    let ran: Vec<u32> = entries.keys().copied().collect();
    let mut tot = Totals::default();
    let mut c = 50;
    while c + WINDOW <= max {
        let list = loop_common::gaps(&rows, &entries, &[], Some(c));
        let next: Vec<u32> = ran
            .iter()
            .copied()
            .filter(|&e| e > c && e <= c + WINDOW)
            .collect();
        let t = hindsight(&list, &next, &entries, false);
        let s = hindsight(&list, &next, &entries, true);
        println!(
            "cutoff E{c:03}: {} gaps ({} planned); next {} experiments: {} take one up (swap control {}); {} of the gaps taken up",
            list.len(),
            list.iter().filter(|g| g.source == Source::Planned).count(),
            next.len(),
            t.experiments_hit,
            s.experiments_hit,
            t.gaps_hit
        );
        tot.add(&t, &s, list.len(), next.len());
        c += STEP;
    }
    println!(
        "hindsight over {} cutoffs: {:.0}% of next experiments took up a listed gap (swap control {:.0}%); {:.0}% of listed gaps were taken up within {WINDOW}; planned ids run within {WINDOW}: {} of {}",
        tot.cutoffs,
        100.0 * tot.hit as f64 / tot.next.max(1) as f64,
        100.0 * tot.swap_hit as f64 / tot.next.max(1) as f64,
        100.0 * tot.gaps_hit as f64 / tot.gaps.max(1) as f64,
        tot.planned_hit,
        tot.planned
    );
}

#[derive(Default)]
struct Hit {
    experiments_hit: usize,
    gaps_hit: usize,
    planned: usize,
    planned_hit: usize,
}

#[derive(Default)]
struct Totals {
    cutoffs: usize,
    next: usize,
    hit: usize,
    swap_hit: usize,
    gaps: usize,
    gaps_hit: usize,
    planned: usize,
    planned_hit: usize,
}

impl Totals {
    fn add(&mut self, t: &Hit, s: &Hit, gaps: usize, next: usize) {
        self.cutoffs += 1;
        self.next += next;
        self.hit += t.experiments_hit;
        self.swap_hit += s.experiments_hit;
        self.gaps += gaps;
        self.gaps_hit += t.gaps_hit;
        self.planned += t.planned;
        self.planned_hit += t.planned_hit;
    }
}

/// Which of `next` took up a gap from `list`. With `swap`, each
/// not-measured gap's words come from the next one in the list instead
/// (the control).
fn hindsight(list: &[Gap], next: &[u32], entries: &BTreeMap<u32, String>, swap: bool) -> Hit {
    let nm: Vec<&Gap> = list
        .iter()
        .filter(|g| g.source == Source::NotMeasured)
        .collect();
    let word_of = |i: usize| words(&nm[if swap { (i + 1) % nm.len() } else { i }].text);
    let mut gap_hit = vec![false; nm.len()];
    let mut h = Hit::default();
    let planned: Vec<u32> = (list.iter())
        .filter(|g| g.source == Source::Planned)
        .filter_map(|g| g.from)
        .collect();
    h.planned = planned.len();
    h.planned_hit = planned.iter().filter(|p| next.contains(p)).count();
    for &e in next {
        let text = &entries[&e];
        let cited = mentions(text);
        let ew = words(text);
        let mut took = planned.contains(&e);
        for (i, g) in nm.iter().enumerate() {
            let f = g.from.unwrap();
            if f != e && cited.contains(&f) && word_of(i).intersection(&ew).count() >= SHARED_WORDS
            {
                gap_hit[i] = true;
                took = true;
            }
        }
        h.experiments_hit += took as usize;
    }
    h.gaps_hit = gap_hit.iter().filter(|x| **x).count() + h.planned_hit;
    h
}
