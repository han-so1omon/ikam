//! Codecs besides zstd, offered only for what repack finally keeps
//! (`Repo::recompress`), where the smallest encoding of each piece wins:
//!   "X" .lzma (LZMA, xz's strongest preset)  | "R" brotli (quality 11)
//!   | "P" mem:u8 PPMd variant H (order 8, 2^mem bytes of model)
//! Neither uses the shared dictionary, so they pay mostly on large frames
//! (groups, whole files). Both decoders stream, so a prefix read stops
//! early. Encoder output is pinned by the crate versions in Cargo.toml;
//! decoding follows the formats' specifications.

use std::io::Read;

use xz2::stream::{LzmaOptions, Stream};

/// Decode time per byte relative to zstd at `STORE_LEVEL`, measured by
/// `tests::codec_decode_speeds` (E034: xz 12.9–13.5x, brotli 3.6–3.9x,
/// PPMd 69–77x). Read costs count decoded bytes with these weights.
const XZ_READ: usize = 13;
const BROTLI_READ: usize = 4;
const PPMD_READ: usize = 70;

/// An encoder for one of these codecs.
pub(crate) type Encoder = fn(&[u8]) -> Option<Vec<u8>>;

/// The codecs `recompress` may try besides zstd: (tag, encoder, active).
/// An inactive codec is **archived**: `recompress` no longer tries it,
/// but its encoder and decoder stay, so frames stored with it still read
/// and setting `active` again brings it back. Archived in E038: .lzma and
/// PPMd, never chosen under the read-aware objective (E035–E037) at
/// `DEFAULT_READ_WEIGHT`, while trying them cost repack time. They may
/// pay for cold data (a lower read weight).
pub(crate) const CODECS: [(u8, Encoder, bool); 3] =
    [(b'X', xz, false), (b'R', brotli, true), (b'P', ppmd, false)];

/// Decode time per byte of a plain encoding, relative to zstd.
pub(crate) fn read_weight(tag: u8) -> f64 {
    match tag {
        b'X' => XZ_READ as f64,
        b'R' => BROTLI_READ as f64,
        b'P' => PPMD_READ as f64,
        // Stored bytes are copied and verified, not decoded: 1/8 of zstd
        // (E069; BLAKE3 0.25 ns/B against zstd's 2.2), as `decode_cost`
        // counts them.
        b'B' => 1.0 / crate::dict::DECODE_UNITS as f64,
        _ => 1.0,
    }
}

/// LZMA preset: 9 with the "extreme" flag (xz -9e).
const XZ_PRESET: u32 = 9 | 1 << 31;

/// `bytes` as an .lzma stream whose dictionary is just large enough for
/// them, so decoding a small frame allocates little.
pub(crate) fn xz(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut opts = LzmaOptions::new_preset(XZ_PRESET).ok()?;
    opts.dict_size(bytes.len().next_power_of_two().clamp(4096, 1 << 26) as u32);
    let stream = Stream::new_lzma_encoder(&opts).ok()?;
    let mut out = Vec::new();
    xz2::read::XzEncoder::new_stream(bytes, stream)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

/// At most the first `limit` bytes of an `xz` stream.
pub(crate) fn unxz_prefix(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    let stream = Stream::new_lzma_decoder(1 << 27).ok()?;
    read_prefix(
        xz2::read::XzDecoder::new_stream(data, stream),
        limit,
        XZ_READ,
    )
}

/// `bytes` as a brotli stream at quality 11, with a window just large
/// enough for them.
pub(crate) fn brotli(bytes: &[u8]) -> Option<Vec<u8>> {
    let params = brotli::enc::BrotliEncoderParams {
        quality: 11,
        lgwin: (usize::BITS - bytes.len().max(1).leading_zeros()).clamp(10, 24) as i32,
        ..Default::default()
    };
    let mut out = Vec::new();
    brotli::BrotliCompress(&mut &bytes[..], &mut out, &params).ok()?;
    Some(out)
}

/// At most the first `limit` bytes of a `brotli` stream.
pub(crate) fn unbrotli_prefix(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    read_prefix(brotli::Decompressor::new(data, 4096), limit, BROTLI_READ)
}

/// PPMd model order: measured best of 6 and 8 on the stored leaves (E032).
const PPMD_ORDER: u32 = 8;

/// `bytes` as PPMd variant H, prefixed by the model size's log2: the
/// model is sized to the input (16x, 1–16 MiB), since a decoder allocates
/// it whole even to read a prefix.
pub(crate) fn ppmd(bytes: &[u8]) -> Option<Vec<u8>> {
    let mem = (bytes
        .len()
        .saturating_mul(16)
        .next_power_of_two()
        .trailing_zeros())
    .clamp(20, 24);
    let mut enc = ppmd_rust::Ppmd7Encoder::new(vec![mem as u8], PPMD_ORDER, 1 << mem).ok()?;
    std::io::Write::write_all(&mut enc, bytes).ok()?;
    enc.finish(true).ok()
}

/// At most the first `limit` bytes of a `ppmd` stream.
pub(crate) fn unppmd_prefix(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    let (&mem, rest) = data.split_first()?;
    let mem = (20..=24).contains(&mem).then_some(1u32 << mem)?;
    read_prefix(
        ppmd_rust::Ppmd7Decoder::new(rest, PPMD_ORDER, mem).ok()?,
        limit,
        PPMD_READ,
    )
}

fn read_prefix(r: impl Read, limit: usize, weight: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    r.take(limit as u64).read_to_end(&mut out).ok()?;
    crate::dict::count_decode(out.len(), weight * crate::dict::DECODE_UNITS);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode speed of each codec relative to zstd at `STORE_LEVEL`, on
    /// the repository's own docs and sources in 64 KiB pieces (like
    /// groups). Where the `*_READ` weights come from; run with
    /// `cargo test --release codec_decode_speeds -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn codec_decode_speeds() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut text = Vec::new();
        for dir in ["docs", "packages/kernel/src"] {
            let mut stack = vec![root.join(dir)];
            while let Some(d) = stack.pop() {
                let mut entries: Vec<_> = std::fs::read_dir(d)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .collect();
                entries.sort();
                for p in entries {
                    match p.is_dir() {
                        true => stack.push(p),
                        false => text.extend(std::fs::read(&p).unwrap()),
                    }
                }
            }
        }
        let pieces: Vec<&[u8]> = text.chunks(64 * 1024).collect();
        let zstd = |b: &[u8]| crate::dict::zstd_frame(b, &[], crate::dict::STORE_LEVEL).unwrap();
        type Codec<'a> = (
            &'a str,
            &'a dyn Fn(&[u8]) -> Vec<u8>,
            &'a dyn Fn(&[u8]) -> Vec<u8>,
        );
        let codecs: [Codec; 4] = [
            ("zstd", &zstd, &|c| {
                crate::dict::unzstd_frame(c, &[]).unwrap()
            }),
            ("xz", &|b| xz(b).unwrap(), &|c| {
                unxz_prefix(c, usize::MAX).unwrap()
            }),
            ("brotli", &|b| brotli(b).unwrap(), &|c| {
                unbrotli_prefix(c, usize::MAX).unwrap()
            }),
            ("ppmd", &|b| ppmd(b).unwrap(), &|c| {
                unppmd_prefix(c, usize::MAX).unwrap()
            }),
        ];
        let mut base = 0.0;
        for (name, enc, dec) in codecs {
            let coded: Vec<Vec<u8>> = pieces.iter().map(|p| enc(p)).collect();
            let t = std::time::Instant::now();
            for _ in 0..3 {
                coded.iter().for_each(|c| assert!(!dec(c).is_empty()));
            }
            let mbs = 3.0 * text.len() as f64 / t.elapsed().as_secs_f64() / 1e6;
            base = if base == 0.0 { mbs } else { base };
            println!(
                "{name:8} {mbs:8.1} MB/s  x{:.1} zstd's decode time",
                base / mbs
            );
        }
    }

    #[test]
    fn codecs_round_trip_and_read_prefixes() {
        let text: Vec<u8> = (0..5000)
            .flat_map(|i| format!("line {i} of text\n").into_bytes())
            .collect();
        for b in [&b""[..], b"x", &text] {
            assert_eq!(unxz_prefix(&xz(b).unwrap(), usize::MAX).unwrap(), b);
            assert_eq!(unbrotli_prefix(&brotli(b).unwrap(), usize::MAX).unwrap(), b);
            assert_eq!(unppmd_prefix(&ppmd(b).unwrap(), usize::MAX).unwrap(), b);
        }
        assert_eq!(
            unppmd_prefix(&ppmd(&text).unwrap(), 100).unwrap(),
            text[..100]
        );
        assert_eq!(unxz_prefix(&xz(&text).unwrap(), 100).unwrap(), text[..100]);
        assert_eq!(
            unbrotli_prefix(&brotli(&text).unwrap(), 100).unwrap(),
            text[..100]
        );
    }
}
