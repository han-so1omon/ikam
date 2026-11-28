//! `train-gd DATA CONFIG`: fit `y ~ X w` by full-batch gradient descent and
//! write the weights, then the final mean squared error, as little-endian
//! f64 values to stdout.
//!
//! DATA: `n` and `d` as little-endian u64, then `n` rows of `d` features and
//! a target, all little-endian f64. CONFIG: text such as
//! `iters=400 lr=0.05`; `jitter=clock` adds a tiny start offset from the
//! clock, a step that is different every time it runs (E443).
//!
//! Built plain, each dot product is one multiply and one add per term,
//! summed in order. Built with `--cfg fused`, each term is a fused
//! multiply-add into one of four lanes, the lanes summed at the end, as a
//! vectorising build on a machine with FMA would.

use std::io::Write;

fn f64s(b: &[u8]) -> Vec<f64> {
    b.chunks_exact(8)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

#[cfg(not(fused))]
fn dot(a: &[f64], b: &[f64]) -> f64 {
    let mut s = 0.0;
    for (x, y) in a.iter().zip(b) {
        s += x * y;
    }
    s
}

#[cfg(fused)]
fn dot(a: &[f64], b: &[f64]) -> f64 {
    let mut lanes = [0.0f64; 4];
    let (ca, cb) = (a.chunks_exact(4), b.chunks_exact(4));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for k in 0..4 {
            lanes[k] = x[k].mul_add(y[k], lanes[k]);
        }
    }
    for (k, (x, y)) in ra.iter().zip(rb).enumerate() {
        lanes[k] = x.mul_add(*y, lanes[k]);
    }
    (lanes[0] + lanes[1]) + (lanes[2] + lanes[3])
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, data, config] = &args[..] else {
        eprintln!("usage: train-gd DATA CONFIG");
        std::process::exit(2);
    };
    let data = std::fs::read(data).expect("data");
    let config = std::fs::read_to_string(config).expect("config");
    let (mut iters, mut lr, mut jitter) = (400usize, 0.05f64, false);
    for part in config.split_whitespace() {
        match part.split_once('=') {
            Some(("iters", v)) => iters = v.parse().expect("iters"),
            Some(("lr", v)) => lr = v.parse().expect("lr"),
            Some(("jitter", "clock")) => jitter = true,
            _ => panic!("config: {part}"),
        }
    }
    let n = u64::from_le_bytes(data[0..8].try_into().unwrap()) as usize;
    let d = u64::from_le_bytes(data[8..16].try_into().unwrap()) as usize;
    let rows = f64s(&data[16..]);
    assert_eq!(rows.len(), n * (d + 1));
    // Columns as rows too, so the gradient is a dot product per weight.
    let x: Vec<&[f64]> = rows.chunks_exact(d + 1).map(|r| &r[..d]).collect();
    let y: Vec<f64> = rows.chunks_exact(d + 1).map(|r| r[d]).collect();
    let cols: Vec<Vec<f64>> = (0..d).map(|j| x.iter().map(|r| r[j]).collect()).collect();
    let mut w = vec![0.0f64; d];
    if jitter {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        w[0] = f64::from(t) * 1e-18;
    }
    let mut r = vec![0.0f64; n];
    let mut mse = 0.0;
    for _ in 0..iters {
        for i in 0..n {
            r[i] = dot(x[i], &w) - y[i];
        }
        mse = dot(&r, &r) / n as f64;
        for j in 0..d {
            let g = 2.0 * dot(&cols[j], &r) / n as f64;
            w[j] -= lr * g;
        }
    }
    let mut out = Vec::with_capacity(8 * (d + 1));
    for v in w.iter().chain([&mse]) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::io::stdout().write_all(&out).unwrap();
}
