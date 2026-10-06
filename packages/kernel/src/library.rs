//! The function library: the reconstruction functions in use, as a graph
//! (`docs/plans/2026-10-06-system-architecture.md`).
//!
//! Rebuilt from the ledger after every repack and kept under `meta/functions`.
//! One node per function that is not a builtin (its stored module), one per
//! application (its derivation record) and one per content an application
//! reads or produces. Edges: application -> "function", "argument",
//! "output (saves N B, work W)", measured on the current store: N is the
//! size the output's bytes would take stored on their own if the function
//! replaced them, or 0 if they are still stored; W is the decode work.
//! Labels are ids, so the same ledger always gives the same graph, and
//! successive versions share unchanged chunks.

use crate::{Arg, Edge, Error, Id, Node, Object, Repo, Store, Target, func, node_key};

pub(crate) const LIBRARY_REF: &str = "meta/functions";

impl<S: Store> Repo<S> {
    /// Rebuild the library graph from the ledger and point `meta/functions`
    /// at it. Returns its root, or `None` if no function is in use.
    pub fn function_library(&mut self) -> Result<Option<Id>, Error> {
        let mut applications = Vec::new();
        for ds in self.ledger()?.by_output.values() {
            for d in ds.iter().filter(|d| !func::is_builtin(&d.func)) {
                applications.push(d.clone());
            }
        }
        if applications.is_empty() {
            return Ok(None);
        }
        let mut nodes: std::collections::BTreeMap<String, Node> = Default::default();
        let content =
            |nodes: &mut std::collections::BTreeMap<String, Node>, label: String, id: Id| {
                nodes.entry(label.clone()).or_insert(Node {
                    label,
                    target: Target::Content(Arg::Whole(id)),
                    edges: vec![],
                });
            };
        for d in &applications {
            let record = Object::Derivation(d.clone()).id();
            let (f, out) = (
                format!("function {}", d.func),
                format!("content {}", d.output),
            );
            content(&mut nodes, f.clone(), d.func);
            content(&mut nodes, out.clone(), d.output);
            let (bytes, work) = self.evaluate(&d.func, &d.args)?;
            let saves = if self.store.has(&d.output) {
                0
            } else {
                self.encode_plain(&bytes).len()
            };
            let mut edges = vec![
                Edge {
                    to: node_key(&f),
                    label: "function".into(),
                },
                Edge {
                    to: node_key(&out),
                    label: format!("output (saves {saves} B, work {work})"),
                },
            ];
            for a in &d.args {
                let arg = format!("content {}", a.id());
                content(&mut nodes, arg.clone(), a.id());
                edges.push(Edge {
                    to: node_key(&arg),
                    label: "argument".into(),
                });
            }
            let label = format!("application {record}");
            nodes.insert(
                label.clone(),
                Node {
                    label,
                    target: Target::Object(record),
                    edges,
                },
            );
        }
        let root = self.put_graph(nodes.into_values().collect())?;
        let current = self.store.get_ref(LIBRARY_REF)?;
        if current != Some(root) {
            self.store.set_ref(LIBRARY_REF, current, root)?;
        }
        Ok(Some(root))
    }
}
