//! The experiment registry as an IKAM graph (docs/experiments/README.md,
//! "Relevance"): every experiment, the objective it was measured under, the
//! mechanisms it touched, and what that makes of it now, committed as a
//! versioned graph to a filesystem store, with each log entry stored as
//! content. The stale experiments and what to rerun are then read back from
//! the stored graph.
//!
//!   cargo run --release --example registry [STORE_DIR]
//!
//! STORE_DIR defaults to target/experiments.ikam. Each run with a changed
//! registry or log commits a new version to the ref `experiments`.

use std::collections::BTreeMap;
use std::path::Path;

use ikam_kernel::{Arg, Edge, FsStore, Kind, Node, Object, Repo, Target, TreeEntry, node_key};

/// The objective the score measures now. Change it with the score.
const CURRENT: &str = "cost";

/// Mechanisms that matter under the current objective (heavy workload at
/// list prices: CPU time dominates, storage is nearly free; E064, E069).
const WEIGHTY: [&str; 8] = [
    "read-path",
    "search",
    "repack",
    "planner",
    "programs",
    "graph",
    "speed",
    "blocks",
];

struct Row {
    id: String,
    slug: String,
    kind: String,
    verdict: String,
    objective: String,
    mechanisms: Vec<String>,
    weight_sensitive: bool,
    note: String,
}

impl Row {
    /// Whether the result still means what it meant: measured under the
    /// current objective, holding under any (no storage-versus-read
    /// trade-off in its verdict), or stale.
    fn relevance(&self) -> &'static str {
        match () {
            _ if self.kind == "benchmark" => "reference",
            _ if self.objective == CURRENT => "current",
            _ if !self.weight_sensitive => "holds",
            _ => "stale",
        }
    }

    /// What a stale result needs: a kept mechanism is ablated (its current
    /// contribution measured, and the code deleted if there is none), a
    /// rejected one retested.
    fn action(&self) -> &'static str {
        match (self.relevance(), self.verdict.as_str()) {
            ("stale", "kept") => "ablate",
            ("stale", _) => "retest",
            _ => "none",
        }
    }

    fn priority(&self) -> &'static str {
        match self
            .mechanisms
            .iter()
            .any(|m| WEIGHTY.contains(&m.as_str()))
        {
            true => "high",
            false => "low",
        }
    }
}

fn main() {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/experiments");
    let rows: Vec<Row> = std::fs::read_to_string(docs.join("registry.tsv"))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Row {
                id: f[0].into(),
                slug: f[1].into(),
                kind: f[2].into(),
                verdict: f[3].into(),
                objective: f[5].into(),
                mechanisms: f[6].split(',').map(String::from).collect(),
                weight_sensitive: f[7] == "yes",
                note: f[8].into(),
            }
        })
        .collect();
    // Log entries by id (entries recorded only on their branches keep their note).
    let log = std::fs::read_to_string(docs.join("log.md")).unwrap();
    let mut entries: BTreeMap<String, String> = BTreeMap::new();
    for part in log.split("\n### ").skip(1) {
        entries.insert(part[..4].to_string(), format!("### {part}"));
    }
    let store = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("{}/target/experiments.ikam", env!("CARGO_MANIFEST_DIR")));
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
    for r in &rows {
        let text = entries
            .get(&r.id)
            .cloned()
            .unwrap_or(format!("{} {}: {}", r.id, r.slug, r.note));
        let content = repo.put_content(text.as_bytes()).unwrap().id;
        let mut edges = vec![
            ("measured-under", format!("objective/{}", r.objective)),
            ("verdict", format!("verdict/{}", r.verdict)),
            ("relevance", format!("relevance/{}", r.relevance())),
            ("action", format!("action/{}", r.action())),
            ("priority", format!("priority/{}", r.priority())),
        ];
        edges.extend(
            r.mechanisms
                .iter()
                .map(|m| ("touches", format!("mechanism/{m}"))),
        );
        for (_, to) in &edges {
            nodes
                .entry(to.clone())
                .or_insert_with(|| node(to.clone(), Target::None, vec![]));
        }
        let label = format!("experiment/{} {}", r.id, r.slug);
        nodes.insert(
            label.clone(),
            node(label, Target::Content(Arg::Whole(content)), edges),
        );
    }
    let graph = repo.put_graph(nodes.into_values().collect()).unwrap();
    let entry = TreeEntry {
        name: "registry".into(),
        kind: Kind::Graph,
        id: graph,
    };
    let tree = repo.put(&Object::tree(vec![entry]).unwrap()).unwrap();
    let head = repo.resolve("experiments").ok();
    let unchanged =
        head.is_some_and(|h| matches!(repo.get(&h), Ok(Object::Commit(c)) if c.tree == tree));
    if !unchanged {
        repo.commit("experiments", tree, "experiment registry")
            .unwrap();
    }

    // Read back from the store: what is stale, and what to rerun first.
    let stored = repo.graph_nodes(&graph).unwrap();
    let key = |s: &str| node_key(s);
    let has = |n: &Node, label: &str, to: &str| {
        n.edges.iter().any(|e| e.label == label && e.to == key(to))
    };
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for n in stored.iter().filter(|n| n.label.starts_with("experiment/")) {
        for r in ["current", "holds", "stale", "reference"] {
            if has(n, "relevance", &format!("relevance/{r}")) {
                *counts.entry(r).or_default() += 1;
            }
        }
    }
    println!(
        "store {store}: graph {graph}, {} versions",
        repo.log(repo.resolve("experiments").unwrap())
            .unwrap()
            .len()
    );
    println!("relevance under the {CURRENT} objective: {counts:?}");
    for (action, priority) in [
        ("ablate", "high"),
        ("retest", "high"),
        ("ablate", "low"),
        ("retest", "low"),
    ] {
        let ids: Vec<&str> = (stored.iter())
            .filter(|n| {
                has(n, "action", &format!("action/{action}"))
                    && has(n, "priority", &format!("priority/{priority}"))
            })
            .map(|n| n.label.trim_start_matches("experiment/"))
            .collect();
        println!(
            "{action} ({priority} priority, {}): {}",
            ids.len(),
            ids.join("; ")
        );
    }
    assert!(repo.fsck().unwrap().is_empty(), "registry store fails fsck");
}
