//! Byte planes for float content (E410), as Hugging Face Xet's BG4: the
//! bytes of a run of 4-byte values regrouped so byte 0 of every value
//! comes first, then byte 1, and so on. In float32 tensors and vectors the
//! high bytes (sign, exponent, top of the mantissa) repeat far more than
//! the low ones, and grouped they compress better.
//!
//! A stored encoding "V" is a zstd frame of the planes: `V` then the frame.
//! Content whose length is not a multiple of 4 keeps its last 1 to 3 bytes
//! in place after the planes. Reading always accepts "V"; repack writes it
//! only with `Repo::float_planes` set, for content the predictor picks.

/// `bytes` as 4 planes, then the 0 to 3 bytes left over.
pub fn split(bytes: &[u8]) -> Vec<u8> {
    let n = bytes.len() / 4;
    let mut out = vec![0u8; bytes.len()];
    for (i, v) in bytes.chunks_exact(4).enumerate() {
        out[i] = v[0];
        out[n + i] = v[1];
        out[2 * n + i] = v[2];
        out[3 * n + i] = v[3];
    }
    out[4 * n..].copy_from_slice(&bytes[4 * n..]);
    out
}

/// The inverse of `split`.
pub fn join(planes: &[u8]) -> Vec<u8> {
    let n = planes.len() / 4;
    let mut out = vec![0u8; planes.len()];
    for i in 0..n {
        out[4 * i] = planes[i];
        out[4 * i + 1] = planes[n + i];
        out[4 * i + 2] = planes[2 * n + i];
        out[4 * i + 3] = planes[3 * n + i];
    }
    out[4 * n..].copy_from_slice(&planes[4 * n..]);
    out
}

/// Content shorter than this is never split: too small for the planes'
/// statistics to differ, or to repay a second zstd run.
pub const MIN_LEN: usize = 4096;

/// The predictor's threshold on `divergence` (E410: on synthetic float32
/// weights and vectors, float64 columns, sorted integers, text and JSON,
/// 0.005 and 0.05 each picked the smaller of planes and bytes for 188 of
/// 196 contents; the lower one is kept because a wrong yes costs one more
/// compression, and repack keeps the smaller, while a wrong no costs bytes:
/// float64 measurements, at 0.006 to 0.01, gain from planes).
pub const THRESHOLD: f64 = 0.005;

/// How differently the 4 byte positions of `bytes` are distributed, as
/// Xet's predictor measures it: the histogram of each position's popcount
/// (0 to 8 bits set), then the largest Kullback-Leibler divergence between
/// any two positions' histograms (in nats, smoothed by half a count).
/// Float32 data has a high byte far from uniform and low bytes close to
/// it; text and integers look alike at every position. It reads at most
/// 64 KiB, sampled evenly, so it costs little next to compressing.
pub fn divergence(bytes: &[u8]) -> f64 {
    let n = bytes.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let step = (n / 16384).max(1);
    let mut h = [[0.5f64; 9]; 4];
    for v in bytes.chunks_exact(4).step_by(step) {
        for (p, b) in v.iter().enumerate() {
            h[p][b.count_ones() as usize] += 1.0;
        }
    }
    let dist: Vec<[f64; 9]> = h
        .iter()
        .map(|c| {
            let t: f64 = c.iter().sum();
            c.map(|x| x / t)
        })
        .collect();
    let mut best = 0.0f64;
    for a in &dist {
        for b in &dist {
            let kl: f64 = a.iter().zip(b).map(|(p, q)| p * (p / q).ln()).sum();
            best = best.max(kl);
        }
    }
    best
}

/// Whether `bytes` should be tried as planes.
pub fn predict(bytes: &[u8]) -> bool {
    bytes.len() >= MIN_LEN && divergence(bytes) > THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planes_round_trip_at_every_length() {
        for len in 0..40 {
            let b: Vec<u8> = (0..len as u8).map(|i| i.wrapping_mul(37)).collect();
            let s = split(&b);
            assert_eq!(join(&s), b, "{len}");
            if len >= 8 {
                assert_eq!(s[0], b[0]);
                assert_eq!(s[1], b[4]);
                assert_eq!(s[len / 4], b[1]);
            }
        }
    }

    #[test]
    fn floats_diverge_and_text_does_not() {
        let mut x = 7u64;
        let floats: Vec<u8> = (0..20000)
            .flat_map(|_| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                let u = (x >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
                (u * 0.04).to_le_bytes()
            })
            .collect();
        let text = include_bytes!("planes.rs").repeat(4);
        assert!(predict(&floats), "{}", divergence(&floats));
        assert!(!predict(&text), "{}", divergence(&text));
        assert!(!predict(&floats[..1000]));
    }
}
