//! What one version writes, object by object: chunks by tree level, how
//! many of each chunk's bytes the chunk it replaces already held, and the
//! bytes by kind of object (fragment, chunk wrapper, slice record,
//! literal, chunk stored whole, tree, ...).

use std::collections::{BTreeMap, HashMap, HashSet};

use ikam_kernel::{Graph, Id, MemStore, Object, Repo, Store};

/// One version's new objects, by kind: (count, stored bytes).
#[derive(Default, Clone)]
pub struct Anatomy {
    pub kinds: BTreeMap<&'static str, (usize, usize)>,
    /// Per tree level: (chunks written, canonical bytes, bytes the
    /// replaced chunk at that level already held as a common prefix and
    /// suffix).
    pub levels: BTreeMap<u8, (usize, usize, usize)>,
    /// Graph chunk contents stored as slices: (count, ranges, of which
    /// reused, literal bytes).
    pub sliced: (usize, usize, usize, usize),
}

impl Anatomy {
    pub fn add(&mut self, o: &Anatomy) {
        for (k, v) in &o.kinds {
            let e = self.kinds.entry(k).or_default();
            e.0 += v.0;
            e.1 += v.1;
        }
        for (k, v) in &o.levels {
            let e = self.levels.entry(*k).or_default();
            e.0 += v.0;
            e.1 += v.1;
            e.2 += v.2;
        }
        self.sliced.0 += o.sliced.0;
        self.sliced.1 += o.sliced.1;
        self.sliced.2 += o.sliced.2;
        self.sliced.3 += o.sliced.3;
    }
}

/// Chunk ids under `root` by level, with each one's canonical encoding.
pub fn chunks(repo: &Repo<MemStore>, root: &Id) -> Vec<(u8, Id, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![*root];
    while let Some(id) = stack.pop() {
        let obj = repo.get(&id).unwrap();
        let enc = obj.encode();
        match obj {
            Object::Graph(Graph::Children(h, cs)) => {
                stack.extend(cs.into_iter().map(|c| c.1));
                out.push((h, id, enc));
            }
            Object::Graph(Graph::Nodes(_)) => out.push((0, id, enc)),
            _ => panic!("not a graph"),
        }
    }
    out
}

fn common(a: &[u8], b: &[u8]) -> usize {
    let p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let s = a[p..]
        .iter()
        .rev()
        .zip(b[p..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    p + s
}

/// The objects `after` holds that `before` did not, classified. `root`
/// and `prev` are the graph roots of this version and the last.
pub fn anatomy(
    repo: &Repo<MemStore>,
    before: &HashSet<Id>,
    root: &Id,
    prev: Option<&Id>,
    fragments: &HashSet<Id>,
) -> Anatomy {
    let s = repo.store();
    let new: Vec<Id> = s
        .ids()
        .unwrap()
        .into_iter()
        .filter(|i| !before.contains(i))
        .collect();
    let mut a = Anatomy::default();
    // Chunk contents (the content a "g" wrapper names) and the contents
    // derivations read as literals.
    let mut chunk_content: HashMap<Id, Id> = HashMap::new();
    for id in &new {
        let enc = s.read(id).unwrap();
        if let [b'g', 2, rest @ ..] = &enc[..] {
            chunk_content.insert(Id::from_bytes(rest.try_into().unwrap()), *id);
        }
    }
    let mut literal: HashSet<Id> = HashSet::new();
    for c in chunk_content.keys() {
        for d in repo.derivations(c).unwrap() {
            a.sliced.0 += 1;
            for arg in &d.args {
                a.sliced.1 += 1;
                let (src, len) = match arg {
                    ikam_kernel::Arg::Whole(i) => (*i, 0),
                    ikam_kernel::Arg::Range { id, len, .. } => (*id, *len as usize),
                    ikam_kernel::Arg::Graph(i) => (*i, 0),
                };
                if new.contains(&src) && !chunk_content.contains_key(&src) {
                    literal.insert(src);
                    a.sliced.3 += len;
                } else {
                    a.sliced.2 += 1;
                }
            }
        }
    }
    for id in &new {
        let enc = s.read(id).unwrap();
        let kind = match enc[0] {
            b'g' => "chunk wrapper (g)",
            b't' => "tree wrapper (t)",
            b'C' | b'c' => "commit",
            b'D' | b'd' => "derivation record",
            b'T' => "tree",
            _ if fragments.contains(id) => "fragment",
            _ if literal.contains(id) => "slice literal",
            _ if chunk_content.contains_key(id) => "chunk stored whole",
            _ => "other content",
        };
        if std::env::var("E448_DEBUG").is_ok() {
            let extra = match kind {
                "derivation record" => match repo.get(id) {
                    Ok(Object::Derivation(d)) => format!(
                        "out {} args {} chunk {}",
                        &d.output.to_string()[..8],
                        d.args.len(),
                        chunk_content.contains_key(&d.output)
                    ),
                    _ => String::new(),
                },
                "slice literal" | "fragment" | "other content" | "chunk stored whole" => {
                    format!(
                        "len {}",
                        repo.read_content(id).map(|b| b.len()).unwrap_or(0)
                    )
                }
                _ => String::new(),
            };
            eprintln!("  {kind} {} {} {extra}", enc.len(), enc[0] as char);
        }
        let e = a.kinds.entry(kind).or_default();
        e.0 += 1;
        e.1 += enc.len();
    }
    // Levels, and bytes the replaced chunk already held.
    let old = prev.map(|p| chunks(repo, p)).unwrap_or_default();
    let old_ids: HashSet<Id> = old.iter().map(|c| c.1).collect();
    for (h, id, enc) in chunks(repo, root) {
        if old_ids.contains(&id) {
            continue;
        }
        let best = old
            .iter()
            .filter(|c| c.0 == h)
            .map(|c| common(&c.2, &enc))
            .max()
            .unwrap_or(0);
        if std::env::var("E448_LEVELS").is_ok() {
            let stored = repo.store().read(&id).unwrap();
            let content = match &stored[..] {
                [b'g', 2, rest @ ..] => {
                    let c = Id::from_bytes(rest.try_into().unwrap());
                    repo.store().read(&c).map(|e| (e[0] as char, e.len())).ok()
                }
                _ => None,
            };
            eprintln!(
                "  level {h} canon {} held {best} stored {content:?}",
                enc.len()
            );
        }
        let e = a.levels.entry(h).or_default();
        e.0 += 1;
        e.1 += enc.len();
        e.2 += best;
    }
    a
}
