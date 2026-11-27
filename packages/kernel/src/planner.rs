//! A learned planner (E425): an option, off by default.
//!
//! Some planning steps build every candidate plan and keep the cheapest.
//! Building a candidate can cost far more than comparing it, and most
//! candidates lose. A `Planner` holds small decision trees, trained offline
//! on the planner's own records (`PlanLog`), that predict from a few cheap
//! features whether a candidate can win; a candidate predicted to lose is
//! not built. The trees only ever remove candidates from a choice among
//! plans that are each exact, so a wrong guess costs bytes (a better plan
//! not built), never reconstruction.
//!
//! Sites (each names its candidates by index):
//! - `group`: the grouping proposals of `group_small` (byte order, seed
//!   resemblance, measured with the dictionary, measured in byte order,
//!   the caller's proposed order). Features: `group_features`.
//! - `codec`: the encodings `encode_cheapest` compares (0 raw, 1 zstd,
//!   then 2 + each of `codec::CODECS`: 3 is brotli). Features:
//!   `codec_features`.
//!
//! A candidate without a tree is always built, so an empty model plans as
//! no model does.
//!
//! The model is plain text, one node a line:
//! `site candidate node feature threshold left right value`. A node with
//! feature -1 is a leaf, and its value is the predicted chance that the
//! candidate wins; an inner node goes left when the feature is at most the
//! threshold. Node 0 is the root. Lines starting with `#` are comments.

use std::collections::HashMap;
use std::sync::Mutex;

/// A candidate whose predicted chance of winning is below this is not built.
pub const BUILD_AT: f64 = 0.5;

#[derive(Clone, Debug)]
struct Node {
    feature: i32,
    threshold: f64,
    left: usize,
    right: usize,
    value: f64,
}

/// Decision trees per site and candidate.
#[derive(Debug, Default)]
pub struct Planner {
    trees: HashMap<(String, usize), Vec<Node>>,
    /// The model's text, so a repack under another model is not skipped as
    /// unchanged.
    text: String,
}

impl Planner {
    /// Parse a model (see the module documentation).
    pub fn parse(text: &str) -> Result<Planner, String> {
        let mut trees: HashMap<(String, usize), Vec<Node>> = HashMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split_whitespace().collect();
            let bad = || format!("planner model line {}: {line}", n + 1);
            if f.len() != 8 {
                return Err(bad());
            }
            let num = |s: &str| s.parse::<f64>().map_err(|_| bad());
            let idx = |s: &str| s.parse::<usize>().map_err(|_| bad());
            let (cand, id) = (idx(f[1])?, idx(f[2])?);
            let node = Node {
                feature: f[3].parse().map_err(|_| bad())?,
                threshold: num(f[4])?,
                left: idx(f[5])?,
                right: idx(f[6])?,
                value: num(f[7])?,
            };
            let tree = trees.entry((f[0].to_string(), cand)).or_default();
            if tree.len() <= id {
                tree.resize(
                    id + 1,
                    Node {
                        feature: -2,
                        threshold: 0.0,
                        left: 0,
                        right: 0,
                        value: 1.0,
                    },
                );
            }
            tree[id] = node;
        }
        for ((site, cand), tree) in &trees {
            let ok = tree.iter().all(|n| match n.feature {
                -1 => true,
                f if f >= 0 => n.left < tree.len() && n.right < tree.len(),
                _ => false,
            });
            if !ok {
                return Err(format!("planner model: tree {site} {cand} is incomplete"));
            }
        }
        Ok(Planner {
            trees,
            text: text.to_string(),
        })
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The predicted chance that `site`'s candidate `cand` wins, or `None`
    /// without a tree for it.
    pub fn chance(&self, site: &str, cand: usize, features: &[f64]) -> Option<f64> {
        let tree = self.trees.get(&(site.to_string(), cand))?;
        let mut at = 0;
        // A tree is acyclic as trained; a bounded walk keeps a bad file from
        // looping.
        for _ in 0..=tree.len() {
            let n = &tree[at];
            if n.feature < 0 {
                return Some(n.value);
            }
            let x = features.get(n.feature as usize).copied().unwrap_or(0.0);
            at = if x <= n.threshold { n.left } else { n.right };
        }
        None
    }

    /// Whether to build `site`'s candidate `cand`: always without a tree.
    pub fn build(&self, site: &str, cand: usize, features: &[f64]) -> bool {
        self.chance(site, cand, features)
            .is_none_or(|p| p >= BUILD_AT)
    }
}

/// Records of plan features and outcomes, one JSON object a line, for
/// training a `Planner` offline. Off unless a caller sets `Repo::plan_log`.
#[derive(Debug, Default)]
pub struct PlanLog {
    lines: Mutex<Vec<String>>,
}

impl PlanLog {
    /// Take the records written so far.
    pub fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.lines.lock().expect("plan log"))
    }

    /// Record one choice: the site, its features, each candidate's outcome
    /// (the bytes it would save, or its cost; `None` when not built) and
    /// the milliseconds spent building it.
    pub(crate) fn record(
        &self,
        site: &str,
        features: &[f64],
        outcome: &[Option<f64>],
        ms: &[f64],
        chosen: Option<usize>,
    ) {
        let list = |v: &mut dyn Iterator<Item = String>| v.collect::<Vec<_>>().join(",");
        let line = format!(
            "{{\"site\":\"{site}\",\"f\":[{}],\"outcome\":[{}],\"ms\":[{}],\"chosen\":{}}}",
            list(&mut features.iter().map(|x| format!("{x:.6}"))),
            list(
                &mut outcome
                    .iter()
                    .map(|o| o.map_or("null".into(), |x| format!("{x:.1}")))
            ),
            list(&mut ms.iter().map(|x| format!("{x:.3}"))),
            chosen.map_or("null".into(), |c| c.to_string()),
        );
        self.lines.lock().expect("plan log").push(line);
    }
}

fn log2(x: f64) -> f64 {
    (x + 1.0).log2()
}

/// Features of one `group_small` pass, from its items (bytes and stored
/// size): 0 items, 1 total bytes, 2 mean item size (log2 each), 3 share of
/// items under 1 KiB, 4 a dictionary (0 or 1), 5 log10 of the read weight,
/// 6 stored over plain bytes, 7 share of items in the proposed order, 8 the
/// group size (log2).
pub(crate) fn group_features(
    sizes: &[(usize, usize)],
    dict: bool,
    read_weight: f64,
    proposed: usize,
    group_size: usize,
) -> Vec<f64> {
    let n = sizes.len().max(1) as f64;
    let total: usize = sizes.iter().map(|s| s.0).sum();
    let stored: usize = sizes.iter().map(|s| s.1).sum();
    vec![
        log2(sizes.len() as f64),
        log2(total as f64),
        sizes.iter().map(|s| log2(s.0 as f64)).sum::<f64>() / n,
        sizes.iter().filter(|s| s.0 < 1024).count() as f64 / n,
        dict as u8 as f64,
        read_weight.max(1e-12).log10(),
        stored as f64 / total.max(1) as f64,
        proposed as f64 / n,
        log2(group_size as f64),
    ]
}

/// Features of one content whose encoding `encode_cheapest` picks: 0 size
/// (log2), 1 log10 of the read weight, 2 bytes read over size (log2), 3
/// order-0 entropy of the first 16 KiB in bits a byte, 4 share of text
/// bytes there, 5 a dictionary (0 or 1), 6 whether slow codecs may run.
pub(crate) fn codec_features(
    bytes: &[u8],
    reads: usize,
    read_weight: f64,
    dict: bool,
    slow: bool,
) -> Vec<f64> {
    let sample = &bytes[..bytes.len().min(16 << 10)];
    let mut counts = [0usize; 256];
    sample.iter().for_each(|&b| counts[b as usize] += 1);
    let n = sample.len().max(1) as f64;
    let entropy: f64 = (counts.iter().filter(|&&c| c > 0))
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum();
    let text = (sample.iter())
        .filter(|&&b| b == b'\n' || b == b'\t' || b == b'\r' || (0x20..0x7f).contains(&b))
        .count() as f64
        / n;
    vec![
        log2(bytes.len() as f64),
        read_weight.max(1e-12).log10(),
        log2(reads as f64) - log2(bytes.len() as f64),
        entropy,
        text,
        dict as u8 as f64,
        slow as u8 as f64,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use std::sync::Arc;

    #[test]
    fn a_model_builds_what_it_predicts_and_everything_it_has_no_tree_for() {
        let text = "# two trees\n\
            group 2 0 5 0.5 1 2 0\n\
            group 2 1 -1 0 0 0 0.9\n\
            group 2 2 -1 0 0 0 0.1\n\
            codec 1 0 -1 0 0 0 0.2\n";
        let p = Planner::parse(text).unwrap();
        let light = [0.0, 0.0, 0.0, 0.0, 0.0, -3.0];
        let heavy = [0.0, 0.0, 0.0, 0.0, 0.0, 1.7];
        assert!(p.build("group", 2, &light));
        assert!(!p.build("group", 2, &heavy));
        assert_eq!(p.chance("group", 2, &heavy), Some(0.1));
        assert!(!p.build("codec", 1, &light));
        // No tree: always built, so an empty model plans as none does.
        assert!(p.build("group", 1, &heavy));
        assert!(p.build("codec", 0, &heavy));
        assert!(Planner::parse("").unwrap().build("group", 2, &heavy));
        // A tree naming a node it lacks is refused, as is a short line.
        assert!(Planner::parse("group 0 0 1 0.5 1 2 0\n").is_err());
        assert!(Planner::parse("group 0 0 1\n").is_err());
    }

    /// A store of small similar notes, repacked at the default read weight
    /// (where groups pay) with `planner` and `log` set.
    fn notes_repacked(
        planner: Option<Planner>,
        log: Option<Arc<PlanLog>>,
    ) -> (crate::Repo<crate::MemStore>, Vec<Vec<u8>>) {
        use crate::{Kind, MemStore, Object, Repo, TreeEntry};
        let mut repo = Repo::new(MemStore::default());
        repo.planner = planner.map(Arc::new);
        repo.plan_log = log;
        let docs: Vec<Vec<u8>> = (0..60)
            .map(|i| {
                format!(
                    "# Note {i}\n\nShared boilerplate, item {}.\n{}",
                    i * 7,
                    "x".repeat(i)
                )
                .into_bytes()
            })
            .collect();
        let entries: Vec<TreeEntry> = (docs.iter().enumerate())
            .map(|(i, d)| TreeEntry {
                name: format!("n{i:02}.md"),
                kind: Kind::File,
                id: repo.put_content(d).unwrap().id,
            })
            .collect();
        let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
        repo.commit("main", tree, "").unwrap();
        repo.repack().unwrap();
        (repo, docs)
    }

    fn stored(repo: &crate::Repo<crate::MemStore>) -> Vec<(crate::Id, Vec<u8>)> {
        let mut ids = repo.store.ids().unwrap();
        ids.sort();
        ids.into_iter()
            .map(|id| (id, repo.store.read(&id).unwrap()))
            .collect()
    }

    /// An empty model and the log leave every stored object as no planner
    /// does; a model that builds no grouping proposal and no compressed
    /// encoding stores more, and every file still reads back exactly.
    #[test]
    fn a_planner_only_ever_costs_bytes() {
        let (plain, docs) = notes_repacked(None, None);
        let log = Arc::new(PlanLog::default());
        let (empty, _) = notes_repacked(Some(Planner::parse("").unwrap()), Some(log.clone()));
        assert_eq!(stored(&plain), stored(&empty));
        let lines = log.drain();
        assert!(lines.iter().any(|l| l.contains("\"site\":\"group\"")));
        assert!(lines.iter().any(|l| l.contains("\"site\":\"codec\"")));
        let none: String = (0..5)
            .map(|c| format!("group {c} 0 -1 0 0 0 0\n"))
            .chain((1..5).map(|c| format!("codec {c} 0 -1 0 0 0 0\n")))
            .collect();
        let (skipped, _) = notes_repacked(Some(Planner::parse(&none).unwrap()), None);
        let size = |r: &crate::Repo<crate::MemStore>| r.content_bytes().unwrap();
        assert!(
            size(&skipped) > size(&plain),
            "groups and zstd were paying here"
        );
        for repo in [&plain, &skipped] {
            for d in &docs {
                assert_eq!(&repo.read_content(&crate::Id::of_content(d)).unwrap(), d);
            }
            assert!(repo.fsck().unwrap().is_empty());
        }
        // No stored member entry: no group was built.
        assert!(
            stored(&skipped)
                .iter()
                .all(|(_, e)| e.first() != Some(&b'S'))
        );
        assert!(stored(&plain).iter().any(|(_, e)| e.first() == Some(&b'S')));
    }

    #[test]
    fn the_log_writes_one_json_line_a_choice() {
        let log = PlanLog::default();
        log.record(
            "group",
            &[1.0, 2.5],
            &[Some(3.0), None],
            &[0.25, 0.0],
            Some(0),
        );
        let lines = log.drain();
        assert_eq!(
            lines,
            vec![
                "{\"site\":\"group\",\"f\":[1.000000,2.500000],\"outcome\":[3.0,null],\"ms\":[0.250,0.000],\"chosen\":0}"
            ]
        );
        assert!(log.drain().is_empty());
    }
}
