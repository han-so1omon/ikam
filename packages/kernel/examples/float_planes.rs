//! E410: float content as byte planes (Xet's BG4), chosen by a predictor.
//!
//! `cargo run --release --example float_planes`
//!
//! Synthetic corpora, each stored as contents in a fresh MemStore, then
//! repacked with `Repo::float_planes` off and on: float32 weights (20
//! checkpoints of a 1M-parameter model, a tensor a content, each
//! checkpoint the last plus small updates), float32 unit vectors (384
//! dimensions), float64 columns (prices with two decimals, and
//! measurements), and content that is not float (this crate's source
//! text, JSON rows, sorted u32 ids). Prints, per corpus, the stored bytes
//! after each repack and whether every content reads back; then, for every
//! content of 4 KiB or more, zstd of the bytes against zstd of the planes
//! (both at repack's level, 19),
//! and how often the predictor at each threshold picks the smaller.

use ikam_kernel::{Edit, MemStore, Repo, Store, planes};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    fn unit(&mut self) -> f64 {
        self.next() as f64 / (1u64 << 53) as f64
    }
    /// About N(0, 1): a sum of 12 uniforms less 6.
    fn normal(&mut self) -> f64 {
        (0..12).map(|_| self.unit()).sum::<f64>() - 6.0
    }
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn corpora() -> Vec<(&'static str, Vec<Vec<u8>>)> {
    let mut r = Lcg(410);
    // Weights: tensors of 256x256, 1024x256, 256x1024 and smaller, about
    // 1M parameters; each checkpoint nudges every weight a little.
    let shapes = [65536usize, 262144, 262144, 262144, 131072, 65536, 1024, 256];
    let mut w: Vec<Vec<f32>> = shapes
        .iter()
        .map(|&n| (0..n).map(|_| (r.normal() * 0.02) as f32).collect())
        .collect();
    let mut weights = Vec::new();
    for _ in 0..20 {
        for t in &mut w {
            for x in t.iter_mut() {
                *x += (r.normal() * 1e-4) as f32;
            }
            weights.push(f32s(t));
        }
    }
    // Vectors: 2,000 unit vectors of 384 dimensions, 100 to a content.
    let mut vectors = Vec::new();
    for _ in 0..20 {
        let mut c = Vec::new();
        for _ in 0..100 {
            let v: Vec<f64> = (0..384).map(|_| r.normal()).collect();
            let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            c.extend(f32s(&v.iter().map(|x| (x / n) as f32).collect::<Vec<_>>()));
        }
        vectors.push(c);
    }
    // Float64 columns, 16,384 values to a content.
    let col = |r: &mut Lcg, f: &dyn Fn(&mut Lcg) -> f64| -> Vec<u8> {
        (0..16384).flat_map(|_| f(r).to_le_bytes()).collect()
    };
    let mut prices = Vec::new();
    let mut measures = Vec::new();
    for _ in 0..8 {
        prices.push(col(&mut r, &|r| (r.unit() * 50000.0).round() / 100.0));
        measures.push(col(&mut r, &|r| 20.0 + r.normal() * 3.0));
    }
    // Not floats.
    let mut text = Vec::new();
    for f in ["src/repo.rs", "src/dict.rs", "src/bm25.rs", "src/graph.rs"] {
        let b = std::fs::read(f).unwrap();
        text.extend(b.chunks(65536).map(|c| c.to_vec()));
    }
    let mut json = Vec::new();
    for c in 0..8 {
        let mut s = String::new();
        for i in 0..1500 {
            let n = c * 1500 + i;
            s.push_str(&format!(
                "{{\"id\":{n},\"name\":\"item{}\",\"qty\":{},\"price\":{:.2}}}\n",
                r.next() % 5000,
                r.next() % 100,
                r.unit() * 500.0
            ));
        }
        json.push(s.into_bytes());
    }
    let mut ids = Vec::new();
    let mut at = 0u32;
    for _ in 0..8 {
        let c: Vec<u8> = (0..16384)
            .flat_map(|_| {
                at += 1 + (r.next() % 50) as u32;
                at.to_le_bytes()
            })
            .collect();
        ids.push(c);
    }
    vec![
        ("weights f32", weights),
        ("vectors f32", vectors),
        ("prices f64", prices),
        ("measures f64", measures),
        ("source text", text),
        ("json rows", json),
        ("sorted u32 ids", ids),
    ]
}

/// As `planes::split`, with 8 planes.
fn split8(bytes: &[u8]) -> Vec<u8> {
    let n = bytes.len() / 8;
    let mut out = vec![0u8; bytes.len()];
    for (i, v) in bytes.chunks_exact(8).enumerate() {
        for (p, b) in v.iter().enumerate() {
            out[p * n + i] = *b;
        }
    }
    out[8 * n..].copy_from_slice(&bytes[8 * n..]);
    out
}

/// Brotli as repack runs it (quality 11, window fitted to the length).
fn brotli(bytes: &[u8]) -> usize {
    let params = brotli::enc::BrotliEncoderParams {
        quality: 11,
        lgwin: (usize::BITS - bytes.len().max(1).leading_zeros()).clamp(10, 24) as i32,
        ..Default::default()
    };
    let mut out = Vec::new();
    brotli::BrotliCompress(&mut &bytes[..], &mut out, &params).unwrap();
    out.len()
}

fn stored(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.stored_len(id).unwrap())
        .sum()
}

fn tags(repo: &Repo<MemStore>) -> String {
    let s = repo.store();
    let mut n = std::collections::BTreeMap::new();
    for id in s.ids().unwrap() {
        *n.entry(s.read(&id).unwrap()[0] as char).or_insert(0) += 1;
    }
    n.iter()
        .map(|(t, c)| format!("{t}:{c}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() {
    let thresholds = [0.0, 0.005, 0.01, 0.02, 0.03, 0.05, 0.1, 0.2, 0.5];
    let mut calls = vec![(0usize, 0usize); thresholds.len()];
    let mut all = 0usize;
    for (name, contents) in corpora() {
        let raw: usize = contents.iter().map(Vec::len).sum();
        let mut sizes = Vec::new();
        for on in [false, true] {
            let mut repo = Repo::new(MemStore::default());
            repo.float_planes = on;
            let ids: Vec<_> = contents
                .iter()
                .map(|c| repo.put_content(c).unwrap().id)
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
            let t = std::time::Instant::now();
            repo.repack().unwrap();
            let secs = t.elapsed().as_secs_f64();
            let t = std::time::Instant::now();
            let exact = ids
                .iter()
                .zip(&contents)
                .all(|(id, c)| repo.read_content(id).unwrap() == *c);
            let read = t.elapsed().as_secs_f64();
            sizes.push((stored(&repo), secs, exact, tags(&repo), read));
        }
        // Per content: which is smaller, and what the predictor says.
        let (mut planes_win, mut big) = (0, 0);
        let mut divs = Vec::new();
        for c in contents.iter().filter(|c| c.len() >= planes::MIN_LEN) {
            big += 1;
            let z = zstd::bulk::compress(c, 19).unwrap().len();
            let p = zstd::bulk::compress(&planes::split(c), 19).unwrap().len();
            let win = p < z;
            planes_win += win as usize;
            let d = planes::divergence(c);
            divs.push(d);
            for (i, &t) in thresholds.iter().enumerate() {
                calls[i].0 += ((d > t) == win) as usize;
            }
            all += 1;
        }
        // Each content's best of zstd and brotli (repack's settings), on the
        // bytes, on 4 planes and on 8 planes (one per byte of a float64).
        let mut best = [0usize; 6];
        for c in contents.iter().filter(|c| c.len() >= planes::MIN_LEN) {
            let p4 = planes::split(c);
            let p8 = split8(c);
            for (i, b) in [&c[..], &p4, &p8].iter().enumerate() {
                best[2 * i] += zstd::bulk::compress(b, 19).unwrap().len();
                best[2 * i + 1] += brotli(b);
            }
        }
        println!(
            "{{\"corpus\":\"{name}\",\"zstd\":{},\"brotli\":{},\"zstd_planes4\":{},\"brotli_planes4\":{},\"zstd_planes8\":{},\"brotli_planes8\":{}}}",
            best[0], best[1], best[2], best[3], best[4], best[5]
        );
        divs.sort_by(f64::total_cmp);
        println!(
            "{{\"corpus\":\"{name}\",\"contents\":{},\"raw\":{raw},\"off\":{},\"on\":{},\"off_s\":{:.2},\"on_s\":{:.2},\"read_off_s\":{:.3},\"read_on_s\":{:.3},\"exact\":{},\"tags_off\":\"{}\",\"tags_on\":\"{}\",\"planes_smaller\":\"{planes_win}/{big}\",\"divergence_min\":{:.4},\"divergence_max\":{:.4}}}",
            contents.len(),
            sizes[0].0,
            sizes[1].0,
            sizes[0].1,
            sizes[1].1,
            sizes[0].4,
            sizes[1].4,
            sizes[0].2 && sizes[1].2,
            sizes[0].3,
            sizes[1].3,
            divs.first().copied().unwrap_or(0.0),
            divs.last().copied().unwrap_or(0.0),
        );
    }
    for (i, t) in thresholds.iter().enumerate() {
        calls[i].1 = all;
        println!(
            "{{\"threshold\":{t},\"right\":\"{}/{}\"}}",
            calls[i].0, calls[i].1
        );
    }
}
