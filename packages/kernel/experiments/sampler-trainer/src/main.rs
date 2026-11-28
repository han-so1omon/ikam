//! E427: a small outside trainer for batches sampled by the kernel's
//! neighborhood sampler (`ikam_kernel::sampler`).
//!
//! Input, on stdin, as frames (a u64 little-endian length, then the bytes):
//! the config (text), the seed (text), then every batch in order. Output, in
//! the directory given as the only argument: `weights.f64` (every parameter
//! as little-endian f64, in the order w1, b1, w2, b2) and `loss.log` (one
//! line a step, the loss as decimal and as its f32 bits).
//!
//! The model is a two-layer MLP predicting a node's class (a file's type,
//! or a fragment) from its sampled neighborhood, with the node's own class
//! hidden. CPU, one thread unless `RAYON_NUM_THREADS` says otherwise,
//! weights initialized from the seed with an integer generator, so a run
//! is a function of its inputs on one machine.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use candle_core::{DType, Device, Tensor, Var};
use candle_nn::{AdamW, Optimizer, ParamsAdamW};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const BATCH_MAGIC: &[u8] = b"ikam-sample-batch/1\n";

/// One sampled node: what the batch carries for it.
#[derive(Clone, Copy)]
struct NodeRec {
    kind: u8,
    label: u16,
    lenlog: u8,
    degree: u32,
}

struct Batch {
    layers: Vec<Vec<NodeRec>>,
    /// Per hop: (position in layer h, position in layer h + 1).
    edges: Vec<Vec<(u32, u32)>>,
}

struct Cursor<'a>(&'a [u8]);

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Res<&[u8]> {
        if self.0.len() < n {
            return Err("batch: truncated".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u32(&mut self) -> Res<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn u16(&mut self) -> Res<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn u8(&mut self) -> Res<u8> {
        Ok(self.take(1)?[0])
    }
}

fn parse_batch(b: &[u8]) -> Res<Batch> {
    let mut c = Cursor(b);
    if c.take(BATCH_MAGIC.len())? != BATCH_MAGIC {
        return Err("batch: magic".into());
    }
    c.take(32)?; // view id
    c.take(8)?; // seed
    let (_fanout, hops) = (c.u32()?, c.u32()?);
    c.take(12)?; // batch, index, epoch
    let mut layers = Vec::new();
    for _ in 0..=hops {
        let n = c.u32()? as usize;
        let mut layer = Vec::with_capacity(n);
        for _ in 0..n {
            c.u32()?; // ordinal
            layer.push(NodeRec {
                kind: c.u8()?,
                label: c.u16()?,
                lenlog: c.u8()?,
                degree: c.u32()?,
            });
        }
        layers.push(layer);
    }
    let mut edges = Vec::new();
    for _ in 0..hops {
        let n = c.u32()? as usize;
        let mut e = Vec::with_capacity(n);
        for _ in 0..n {
            e.push((c.u32()?, c.u32()?));
        }
        edges.push(e);
    }
    Ok(Batch { layers, edges })
}

fn class(n: &NodeRec, labels: usize) -> usize {
    match n.kind {
        0 => usize::from(n.label).min(labels - 1),
        k => labels + usize::from(k.min(2)) - 1,
    }
}

fn log2(x: u32) -> f32 {
    (32 - x.leading_zeros()) as f32
}

/// Features of every root: its own size and degree, then, for hop 1 and
/// hop 2, a class histogram, mean size, mean degree and count, each scaled.
fn features(b: &Batch, classes: usize, fanout: usize) -> (Vec<f32>, Vec<u32>, usize) {
    let labels = classes - 2;
    let dim = 2 + 2 * (classes + 3);
    let roots = &b.layers[0];
    let mut x = vec![0f32; roots.len() * dim];
    let mut y = Vec::with_capacity(roots.len());
    let mut out1: Vec<Vec<u32>> = vec![Vec::new(); roots.len()];
    for &(s, d) in &b.edges[0] {
        out1[s as usize].push(d);
    }
    let mut out2: Vec<Vec<u32>> = vec![Vec::new(); b.layers.get(1).map_or(0, Vec::len)];
    if let Some(e) = b.edges.get(1) {
        for &(s, d) in e {
            out2[s as usize].push(d);
        }
    }
    for (i, r) in roots.iter().enumerate() {
        y.push(class(r, labels) as u32);
        let row = &mut x[i * dim..(i + 1) * dim];
        row[0] = f32::from(r.lenlog) / 32.0;
        row[1] = log2(r.degree) / 16.0;
        let hop1: Vec<u32> = out1[i].clone();
        let mut hop2: Vec<u32> = hop1
            .iter()
            .flat_map(|&j| out2[j as usize].clone())
            .collect();
        hop2.sort_unstable();
        hop2.dedup();
        for (h, (set, layer)) in [(&hop1, 1usize), (&hop2, 2)].into_iter().enumerate() {
            let base = 2 + h * (classes + 3);
            let Some(nodes) = b.layers.get(layer) else {
                continue;
            };
            for &j in set {
                let n = &nodes[j as usize];
                row[base + class(n, labels)] += 1.0;
                row[base + classes] += f32::from(n.lenlog) / 32.0;
                row[base + classes + 1] += log2(n.degree) / 16.0;
            }
            let k = set.len() as f32;
            if k > 0.0 {
                for v in &mut row[base..base + classes + 2] {
                    *v /= k;
                }
            }
            row[base + classes + 2] = k / (fanout * fanout.pow(h as u32)) as f32;
        }
    }
    (x, y, dim)
}

/// splitmix64: weights are drawn from the seed with integers only.
fn splitmix(s: &mut u64) -> u64 {
    *s = s.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn init(rng: &mut u64, rows: usize, cols: usize) -> Res<Var> {
    let scale = 1.0 / (cols as f32).sqrt();
    let v: Vec<f32> = (0..rows * cols)
        .map(|_| ((splitmix(rng) >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * scale)
        .collect();
    let shape: Vec<usize> = if rows == 1 {
        vec![cols]
    } else {
        vec![rows, cols]
    };
    Ok(Var::from_tensor(&Tensor::from_vec(
        v,
        shape,
        &Device::Cpu,
    )?)?)
}

fn read_frame(r: &mut impl Read) -> Res<Option<Vec<u8>>> {
    let mut len = [0u8; 8];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let mut b = vec![0u8; u64::from_le_bytes(len) as usize];
    r.read_exact(&mut b)?;
    Ok(Some(b))
}

fn main() -> Res<()> {
    let out = std::env::args()
        .nth(1)
        .ok_or("usage: sampler-trainer OUT_DIR < frames")?;
    let mut stdin = std::io::stdin().lock();
    let config = String::from_utf8(read_frame(&mut stdin)?.ok_or("no config")?)?;
    let seed_text = String::from_utf8(read_frame(&mut stdin)?.ok_or("no seed")?)?;
    let cfg: BTreeMap<&str, &str> = config
        .split_whitespace()
        .filter_map(|p| p.split_once('='))
        .collect();
    let get = |k: &str| -> Res<&str> { Ok(cfg.get(k).copied().ok_or(format!("config: {k}"))?) };
    let (classes, hidden, fanout): (usize, usize, usize) = (
        get("classes")?.parse()?,
        get("hidden")?.parse()?,
        get("fanout")?.parse()?,
    );
    let lr: f64 = get("lr")?.parse()?;
    // The thread count of record, unless the environment overrides it (the
    // reproduce check does, to see whether it matters).
    if std::env::var_os("RAYON_NUM_THREADS").is_none() {
        // SAFETY: set before any thread is started.
        unsafe { std::env::set_var("RAYON_NUM_THREADS", get("threads")?) };
    }
    let seed: u64 = seed_text
        .trim()
        .strip_prefix("seed ")
        .ok_or("seed: format")?
        .parse()?;

    let dim = 2 + 2 * (classes + 3);
    let mut rng = seed;
    let (w1, b1) = (init(&mut rng, hidden, dim)?, init(&mut rng, 1, hidden)?);
    let (w2, b2) = (
        init(&mut rng, classes, hidden)?,
        init(&mut rng, 1, classes)?,
    );
    let vars = vec![w1.clone(), b1.clone(), w2.clone(), b2.clone()];
    let mut opt = AdamW::new(
        vars.clone(),
        ParamsAdamW {
            lr,
            ..ParamsAdamW::default()
        },
    )?;
    let mut log = String::new();
    let (mut steps, mut correct, mut seen) = (0usize, 0usize, 0usize);
    while let Some(frame) = read_frame(&mut stdin)? {
        let batch = parse_batch(&frame)?;
        let (x, y, d) = features(&batch, classes, fanout);
        let n = y.len();
        let x = Tensor::from_vec(x, (n, d), &Device::Cpu)?;
        let y = Tensor::from_vec(y, n, &Device::Cpu)?;
        let h = x.matmul(&w1.t()?)?.broadcast_add(&b1)?.relu()?;
        let logits = h.matmul(&w2.t()?)?.broadcast_add(&b2)?;
        let loss = candle_nn::loss::cross_entropy(&logits, &y)?;
        opt.backward_step(&loss)?;
        let l = loss.to_dtype(DType::F32)?.to_scalar::<f32>()?;
        let pred = logits.argmax(1)?.to_vec1::<u32>()?;
        let ys = y.to_vec1::<u32>()?;
        correct += pred.iter().zip(&ys).filter(|(a, b)| a == b).count();
        seen += n;
        log += &format!("step {steps} n {n} loss {l:.6} bits {:08x}\n", l.to_bits());
        steps += 1;
    }
    log += &format!(
        "steps {steps} train_accuracy {:.4}\n",
        correct as f64 / seen.max(1) as f64
    );
    let mut weights = Vec::new();
    for v in &vars {
        for x in v.as_tensor().flatten_all()?.to_vec1::<f32>()? {
            weights.extend_from_slice(&f64::from(x).to_le_bytes());
        }
    }
    let out = std::path::Path::new(&out);
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("weights.f64"), &weights)?;
    std::fs::File::create(out.join("loss.log"))?.write_all(log.as_bytes())?;
    eprintln!(
        "trained {steps} steps on {seen} roots, {} weights",
        weights.len() / 8
    );
    Ok(())
}
