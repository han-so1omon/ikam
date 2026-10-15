//! Does measured conditional gain find related content? For each markdown
//! fixture, the other fixture with the highest gain should more often come
//! from the same case (same fictional company) than chance predicts.
//!
//! Since E049 it also asks the same of the storage plan: after repack, the
//! fixture sharing the most *concepts* (templates, slice sources, groups
//! and anything else its derivations reach), each weighted by its rarity.
//!
//!   cargo run --release --example relatedness

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ikam_kernel::{Arg, Id, Kind, MemStore, Object, Repo, Store, TreeEntry};

fn markdown(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// What the storage plan built `id` from: every content its derivations
/// reach, and the group of every stored one ("S" + 8-byte group prefix).
fn concepts(repo: &Repo<MemStore>, id: Id) -> BTreeSet<Vec<u8>> {
    let (mut out, mut stack) = (BTreeSet::new(), vec![id]);
    while let Some(id) = stack.pop() {
        if let Ok(stored) = repo.store().read(&id)
            && let Some(group) = stored.get(..9).filter(|_| stored[0] == b'S')
        {
            out.insert(group.to_vec());
        }
        for d in repo.derivations(&id).unwrap() {
            for a in d.args {
                if out.insert(a.id().as_bytes().to_vec()) {
                    stack.push(a.id());
                }
            }
        }
    }
    out
}

fn varint(b: &mut &[u8]) -> usize {
    let (mut n, mut shift) = (0, 0);
    while let Some((&byte, rest)) = b.split_first() {
        *b = rest;
        n |= ((byte & 0x7f) as usize) << shift;
        if byte < 0x80 {
            break;
        }
        shift += 7;
    }
    n
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/cases");
    let files: Vec<PathBuf> = markdown(&root)
        .into_iter()
        .filter(|p| !p.starts_with(root.join("_image_tools")))
        .collect();
    let case = |p: &PathBuf| {
        p.strip_prefix(&root)
            .unwrap()
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_owned()
    };
    let mut repo = Repo::new(MemStore::default());
    let ids: Vec<_> = files
        .iter()
        .map(|p| repo.put_content(&std::fs::read(p).unwrap()).unwrap().id)
        .collect();
    let (mut same_case, mut same_name, mut chance_case, mut chance_name) = (0, 0, 0.0, 0.0);
    for (i, a) in ids.iter().enumerate() {
        let best = (0..ids.len())
            .filter(|&j| j != i)
            .max_by_key(|&j| {
                (
                    repo.gain_bits(&Arg::Whole(*a), &Arg::Whole(ids[j]))
                        .unwrap(),
                    std::cmp::Reverse(j),
                )
            })
            .unwrap();
        same_case += (case(&files[best]) == case(&files[i])) as usize;
        same_name += (files[best].file_name() == files[i].file_name()) as usize;
        let others = (ids.len() - 1) as f64;
        chance_case +=
            (files.iter().filter(|p| case(p) == case(&files[i])).count() - 1) as f64 / others;
        chance_name += (files
            .iter()
            .filter(|p| p.file_name() == files[i].file_name())
            .count()
            - 1) as f64
            / others;
    }
    let n = ids.len() as f64;
    // The storage plan's view: concepts shared with each other fixture,
    // weighted by rarity (log N / files using it).
    // Repack plans committed content, so commit the fixtures first.
    let entries = ids
        .iter()
        .enumerate()
        .map(|(i, &id)| TreeEntry {
            name: format!("{i:04}.md"),
            kind: Kind::File,
            id,
        })
        .collect();
    let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
    let repacked = repo.repack().unwrap();
    let sets: Vec<_> = ids.iter().map(|&id| concepts(&repo, id)).collect();
    let mut df: BTreeMap<&Vec<u8>, usize> = BTreeMap::new();
    sets.iter()
        .flatten()
        .for_each(|c| *df.entry(c).or_default() += 1);
    let idf = |c: &Vec<u8>| (n / df[c] as f64).ln();
    let (mut covered, mut plan_case, mut plan_name) = (0, 0, 0);
    for i in 0..ids.len() {
        let score = |j: usize| sets[i].intersection(&sets[j]).map(idf).sum::<f64>();
        let best = (0..ids.len())
            .filter(|&j| j != i)
            .max_by(|&a, &b| score(a).total_cmp(&score(b)).then(b.cmp(&a)))
            .unwrap();
        if score(best) > 0.0 {
            covered += 1;
            plan_case += (case(&files[best]) == case(&files[i])) as usize;
            plan_name += (files[best].file_name() == files[i].file_name()) as usize;
        }
    }
    // Finer: the member stored just before it in its group (the one its
    // compression can reference), or just after it if it comes first.
    let mut members: BTreeMap<Vec<u8>, Vec<(usize, usize)>> = BTreeMap::new();
    for (i, id) in ids.iter().enumerate() {
        let stored = repo.store().read(id).unwrap();
        if stored[0] == b'S' {
            let mut rest = &stored[9..];
            members
                .entry(stored[..9].to_vec())
                .or_default()
                .push((varint(&mut rest), i));
        }
    }
    let (mut adjacent, mut adj_case, mut adj_name) = (0, 0, 0);
    for list in members.values_mut().filter(|l| l.len() > 1) {
        list.sort();
        for k in 0..list.len() {
            let (i, j) = (list[k].1, list[if k == 0 { 1 } else { k - 1 }].1);
            adjacent += 1;
            adj_case += (case(&files[j]) == case(&files[i])) as usize;
            adj_name += (files[j].file_name() == files[i].file_name()) as usize;
        }
    }
    println!("{} markdown files", ids.len());
    println!(
        "top-gain neighbour from same case:     {:.1}%  (chance {:.1}%)",
        100.0 * same_case as f64 / n,
        100.0 * chance_case / n
    );
    println!(
        "top-gain neighbour with same filename: {:.1}%  (chance {:.1}%)",
        100.0 * same_name as f64 / n,
        100.0 * chance_name / n
    );
    println!(
        "storage plan ({} -> {} B, applied {}): {covered} files share a concept",
        repacked.before, repacked.after, repacked.applied
    );
    println!(
        "  plan neighbour from same case:     {:.1}% of those ({:.1}% of all)",
        100.0 * plan_case as f64 / covered.max(1) as f64,
        100.0 * plan_case as f64 / n
    );
    println!(
        "  plan neighbour with same filename: {:.1}% of those ({:.1}% of all)",
        100.0 * plan_name as f64 / covered.max(1) as f64,
        100.0 * plan_name as f64 / n
    );
    println!(
        "  adjacent group member ({adjacent} files): same case {:.1}%, same filename {:.1}%",
        100.0 * adj_case as f64 / adjacent.max(1) as f64,
        100.0 * adj_name as f64 / adjacent.max(1) as f64
    );
}
