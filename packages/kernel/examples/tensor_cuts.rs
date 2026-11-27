//! E411: 20 checkpoints of a small model stored with and without
//! `Repo::tensor_cuts`.
//!
//! `cargo run --release --example tensor_cuts`
//!
//! Each checkpoint is a safetensors file of eight float32 tensors (about
//! 1M parameters, 4 MB). Three ways of training between checkpoints:
//! `all` nudges every weight; `half` keeps the first four tensors frozen
//! (as a model with frozen layers); `finetune` changes 5% of the rows of
//! the embedding table and the last layer only. For each, prints the bytes
//! stored after writing the 20 files and after a repack, the write time,
//! and whether every file reads back exact.

use ikam_kernel::{Edit, MemStore, Repo, Store};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    fn normal(&mut self) -> f32 {
        ((0..12)
            .map(|_| self.next() as f64 / (1u64 << 53) as f64)
            .sum::<f64>()
            - 6.0) as f32
    }
}

const NAMES: [&str; 8] = [
    "pos", "embed", "l1.w", "l1.b", "l2.w", "l2.b", "head.w", "head.b",
];
const SIZES: [usize; 8] = [65536, 262144, 262144, 1024, 262144, 1024, 131072, 256];

fn safetensors(w: &[Vec<f32>]) -> Vec<u8> {
    let mut header = String::from("{\"__metadata__\":{\"format\":\"pt\"}");
    let mut at = 0;
    for (name, t) in NAMES.iter().zip(w) {
        let len = 4 * t.len();
        header.push_str(&format!(
            ",\"{name}\":{{\"dtype\":\"F32\",\"shape\":[{}],\"data_offsets\":[{at},{}]}}",
            t.len(),
            at + len
        ));
        at += len;
    }
    header.push('}');
    while header.len() % 8 != 0 {
        header.push(' ');
    }
    let mut out = (header.len() as u64).to_le_bytes().to_vec();
    out.extend(header.as_bytes());
    for t in w {
        out.extend(t.iter().flat_map(|x| x.to_le_bytes()));
    }
    out
}

fn checkpoints(how: &str) -> Vec<Vec<u8>> {
    let mut r = Lcg(411);
    let mut w: Vec<Vec<f32>> = SIZES
        .iter()
        .map(|&n| (0..n).map(|_| r.normal() * 0.02).collect())
        .collect();
    let mut out = Vec::new();
    for _ in 0..20 {
        for (i, t) in w.iter_mut().enumerate() {
            match how {
                "all" => t.iter_mut().for_each(|x| *x += r.normal() * 1e-4),
                "half" if i >= 4 => t.iter_mut().for_each(|x| *x += r.normal() * 1e-4),
                "finetune" if i == 1 => {
                    // 1,024 rows of 256: about 5% of them change.
                    for row in t.chunks_mut(256) {
                        if r.next().is_multiple_of(20) {
                            row.iter_mut().for_each(|x| *x += r.normal() * 1e-3);
                        }
                    }
                }
                "finetune" if i >= 6 => t.iter_mut().for_each(|x| *x += r.normal() * 1e-4),
                _ => {}
            }
        }
        out.push(safetensors(&w));
    }
    out
}

fn stored(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.stored_len(id).unwrap())
        .sum()
}

fn main() {
    for how in ["all", "half", "finetune"] {
        let files = checkpoints(how);
        let one = files[0].len();
        for cuts in [false, true] {
            let mut repo = Repo::new(MemStore::default());
            repo.tensor_cuts = cuts;
            let t = std::time::Instant::now();
            let ids: Vec<_> = files
                .iter()
                .map(|f| repo.put_content(f).unwrap().id)
                .collect();
            // Repack keeps only what a commit reaches.
            let edits: Vec<_> = ids
                .iter()
                .enumerate()
                .map(|(i, id)| Edit::Put {
                    path: format!("c/{i:04}"),
                    id: *id,
                })
                .collect();
            let tree = repo.edit_tree(None, &edits).unwrap();
            repo.commit("main", tree, "load").unwrap();
            let write = t.elapsed().as_secs_f64();
            let written = stored(&repo);
            let t = std::time::Instant::now();
            repo.repack().unwrap();
            let repack = t.elapsed().as_secs_f64();
            let t = std::time::Instant::now();
            let exact = ids
                .iter()
                .zip(&files)
                .all(|(id, f)| repo.read_content(id).unwrap() == *f);
            let read = t.elapsed().as_secs_f64();
            println!(
                "{{\"training\":\"{how}\",\"tensor_cuts\":{cuts},\"files\":20,\"one_file\":{one},\"written\":{written},\"after_repack\":{},\"write_s\":{write:.2},\"repack_s\":{repack:.2},\"read_s\":{read:.2},\"exact\":{exact},\"objects\":{}}}",
                stored(&repo),
                repo.store().ids().unwrap().len(),
            );
        }
    }
}
