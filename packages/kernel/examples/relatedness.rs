//! Does measured conditional gain find related content? For each markdown
//! fixture, the other fixture with the highest gain should more often come
//! from the same case (same fictional company) than chance predicts.
//!
//!   cargo run --release --example relatedness

use std::path::{Path, PathBuf};

use ikam_kernel::{Arg, MemStore, Repo};

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
}
