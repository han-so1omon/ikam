//! Codecs besides zstd, offered only for what repack finally keeps
//! (`Repo::recompress`), where the smallest encoding of each piece wins:
//!   "X" .lzma (LZMA, xz's strongest preset)  | "R" brotli (quality 11)
//! Neither uses the shared dictionary, so they pay mostly on large frames
//! (groups, whole files). Both decoders stream, so a prefix read stops
//! early. Encoder output is pinned by the crate versions in Cargo.toml;
//! decoding follows the formats' specifications.

use std::io::Read;

use xz2::stream::{LzmaOptions, Stream};

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
    read_prefix(xz2::read::XzDecoder::new_stream(data, stream), limit)
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
    read_prefix(brotli::Decompressor::new(data, 4096), limit)
}

fn read_prefix(r: impl Read, limit: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    r.take(limit as u64).read_to_end(&mut out).ok()?;
    crate::dict::count_decompressed(out.len());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codecs_round_trip_and_read_prefixes() {
        let text: Vec<u8> = (0..5000)
            .flat_map(|i| format!("line {i} of text\n").into_bytes())
            .collect();
        for b in [&b""[..], b"x", &text] {
            assert_eq!(unxz_prefix(&xz(b).unwrap(), usize::MAX).unwrap(), b);
            assert_eq!(unbrotli_prefix(&brotli(b).unwrap(), usize::MAX).unwrap(), b);
        }
        assert_eq!(unxz_prefix(&xz(&text).unwrap(), 100).unwrap(), text[..100]);
        assert_eq!(
            unbrotli_prefix(&brotli(&text).unwrap(), 100).unwrap(),
            text[..100]
        );
    }
}
