//! E148: lookups over a view with edits kept pending see the nodes that
//! folding the edits in gives, and folding several goals' edits at once
//! gives the root that applying them one by one gives.

use std::collections::BTreeMap;

use ikam_kernel::{Edge, Keys, MemStore, Node, Repo, Target, node_key};

fn rng(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

fn node(label: String, s: &mut u64) -> Node {
    // Edges out of order and repeated: lookups must give them as stored.
    let to = node_key(&format!("t{}", rng(s) % 50));
    let other = node_key(&format!("t{}", rng(s) % 50));
    Node {
        label,
        target: Target::None,
        edges: vec![
            Edge {
                to: other,
                label: "b".into(),
            },
            Edge {
                to,
                label: "a".into(),
            },
            Edge {
                to,
                label: "a".into(),
            },
        ],
    }
}

/// Labels in a few prefixes, so prefix lookups find several nodes.
fn label(s: &mut u64) -> String {
    format!("p{}/{:x}", rng(s) % 40, rng(s) % 100_000)
}

fn edits(nodes: &[String], n: usize, s: &mut u64) -> BTreeMap<String, Option<Node>> {
    let mut out = BTreeMap::new();
    for _ in 0..n {
        match rng(s) % 4 {
            0 if !nodes.is_empty() => {
                let l = nodes[(rng(s) % nodes.len() as u64) as usize].clone();
                out.insert(l, None);
            }
            1 if !nodes.is_empty() => {
                let l = nodes[(rng(s) % nodes.len() as u64) as usize].clone();
                out.insert(l.clone(), Some(node(l, s)));
            }
            2 => {
                out.insert(label(s), None);
            }
            _ => {
                let l = label(s);
                out.insert(l.clone(), Some(node(l, s)));
            }
        }
    }
    out
}

#[test]
fn pending_lookups_see_the_folded_graph() {
    let mut s = 0x9e3779b97f4a7c15u64;
    let mut repo = Repo::new(MemStore::default());
    let base: Vec<Node> = (0..4000).map(|_| node(label(&mut s), &mut s)).collect();
    let mut base: Vec<Node> = base
        .into_iter()
        .map(|mut n| {
            n.edges.sort();
            n.edges.dedup();
            (n.label.clone(), n)
        })
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect();
    base.sort_by(|a, b| a.label.cmp(&b.label));
    let root = repo.put_graph_with(base.clone(), Keys::Any).unwrap();
    let labels: Vec<String> = base.iter().map(|n| n.label.clone()).collect();
    let mut eager = root;
    let mut pending: BTreeMap<String, Option<Node>> = BTreeMap::new();
    for _ in 0..6 {
        let goal = edits(&labels, 150, &mut s);
        eager = repo
            .graph_edit_with(&eager, goal.clone(), Keys::Any)
            .unwrap();
        // Later goals override earlier pending edits of the same label.
        pending.extend(goal);
        for p in 0..40 {
            let prefix = format!("p{p}/");
            assert_eq!(
                repo.graph_prefix_pending(&root, &pending, &prefix).unwrap(),
                repo.graph_prefix(&eager, &prefix).unwrap(),
                "prefix {prefix}"
            );
        }
        for l in pending.keys().take(50).chain(labels.iter().take(50)) {
            assert_eq!(
                repo.graph_node_pending(&root, &pending, l).unwrap(),
                repo.graph_node(&eager, l).unwrap()
            );
        }
    }
    let folded = repo.graph_edit_with(&root, pending, Keys::Any).unwrap();
    assert_eq!(folded, eager);
    assert!(repo.fsck().unwrap().is_empty());
}
