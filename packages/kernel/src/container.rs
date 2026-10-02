//! Zip containers (xlsx, docx, pptx, jar, ...) as function applications.
//!
//! `plan_zip` splits a zip into a manifest (every byte that is not a
//! compressed member stream, kept verbatim) and the *uncompressed* members, so
//! dedup and graph links see the real content. `pack` rebuilds the original
//! bytes by re-deflating each member at the level recorded in the manifest.
//! A member is expanded only if re-deflating reproduces its stream exactly;
//! otherwise its compressed bytes stay in the manifest. The caller still
//! verifies the whole reconstruction before accepting it.
//!
//! Manifest: count:u32 { 0 len:u32 bytes | 1 member:u32 level:u8 }

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

use crate::Error;

/// Levels to try, most common first (zlib's default is 6).
const LEVELS: [u8; 10] = [6, 4, 5, 1, 2, 3, 7, 8, 9, 0];
/// Never inflate a member beyond this many bytes (zip-bomb guard).
const MAX_MEMBER: usize = 1 << 30;

enum Segment<'a> {
    Literal(&'a [u8]),
    Deflate { member: u32, level: u8 },
}

pub fn deflate(data: &[u8], level: u8) -> Vec<u8> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(level.into()));
    enc.write_all(data).expect("writing to a Vec cannot fail");
    enc.finish().expect("writing to a Vec cannot fail")
}

fn inflate(stream: &[u8], expected: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(expected.min(MAX_MEMBER));
    DeflateDecoder::new(stream)
        .take(MAX_MEMBER as u64 + 1)
        .read_to_end(&mut out)
        .ok()?;
    (out.len() == expected).then_some(out)
}

fn u16_at(b: &[u8], i: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as usize)
}

fn u32_at(b: &[u8], i: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize)
}

/// Compressed member streams as `(start, end, uncompressed_size)`, from the
/// central directory. `None` for anything that is not a plain (non-zip64) zip.
fn member_streams(b: &[u8]) -> Option<Vec<(usize, usize, usize)>> {
    let eocd = (0..b.len().checked_sub(22)? + 1)
        .rev()
        .take(65_557)
        .find(|&i| b[i..].starts_with(b"PK\x05\x06"))?;
    let (count, cd_off) = (u16_at(b, eocd + 10)?, u32_at(b, eocd + 16)?);
    let (mut pos, mut out) = (cd_off, Vec::new());
    for _ in 0..count {
        if !b.get(pos..)?.starts_with(b"PK\x01\x02") {
            return None;
        }
        let (method, csize, usize_) = (
            u16_at(b, pos + 10)?,
            u32_at(b, pos + 20)?,
            u32_at(b, pos + 24)?,
        );
        let lho = u32_at(b, pos + 42)?;
        pos += 46 + u16_at(b, pos + 28)? + u16_at(b, pos + 30)? + u16_at(b, pos + 32)?;
        if method != 8 || !b.get(lho..)?.starts_with(b"PK\x03\x04") {
            continue;
        }
        let start = lho + 30 + u16_at(b, lho + 26)? + u16_at(b, lho + 28)?;
        out.push((
            start,
            start.checked_add(csize).filter(|&e| e <= b.len())?,
            usize_,
        ));
    }
    out.sort();
    out.windows(2).all(|w| w[0].1 <= w[1].0).then_some(out)
}

/// `(manifest, members)` if at least one member can be expanded exactly.
pub fn plan_zip(bytes: &[u8]) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
    let (mut segments, mut members, mut cursor) = (Vec::new(), Vec::new(), 0);
    for (start, end, size) in member_streams(bytes)? {
        let stream = &bytes[start..end];
        let Some(data) = inflate(stream, size) else {
            continue;
        };
        let Some(level) = LEVELS.into_iter().find(|&l| deflate(&data, l) == stream) else {
            continue;
        };
        segments.push(Segment::Literal(&bytes[cursor..start]));
        segments.push(Segment::Deflate {
            member: members.len() as u32,
            level,
        });
        members.push(data);
        cursor = end;
    }
    if members.is_empty() {
        return None;
    }
    segments.push(Segment::Literal(&bytes[cursor..]));
    let mut manifest = (segments.len() as u32).to_be_bytes().to_vec();
    for s in segments {
        match s {
            Segment::Literal(b) => {
                manifest.push(0);
                manifest.extend_from_slice(&(b.len() as u32).to_be_bytes());
                manifest.extend_from_slice(b);
            }
            Segment::Deflate { member, level } => {
                manifest.push(1);
                manifest.extend_from_slice(&member.to_be_bytes());
                manifest.push(level);
            }
        }
    }
    Some((manifest, members))
}

/// Builtin `deflate-pack`: args are `[manifest, member0, member1, ...]`.
pub fn pack(args: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
    let bad = || Error::Exec("deflate-pack: malformed manifest".into());
    let (manifest, members) = args.split_first().ok_or_else(bad)?;
    let mut m = Cursor(manifest);
    let mut out = Vec::new();
    for _ in 0..m.u32().ok_or_else(bad)? {
        match m.take(1).ok_or_else(bad)?[0] {
            0 => {
                let len = m.u32().ok_or_else(bad)?;
                out.extend_from_slice(m.take(len).ok_or_else(bad)?);
            }
            1 => {
                let member = members.get(m.u32().ok_or_else(bad)?).ok_or_else(bad)?;
                let level = m.take(1).ok_or_else(bad)?[0];
                if level > 9 {
                    return Err(bad());
                }
                out.extend_from_slice(&deflate(member, level));
            }
            _ => return Err(bad()),
        }
    }
    if !m.0.is_empty() {
        return Err(bad());
    }
    Ok(out)
}

struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }

    fn u32(&mut self) -> Option<usize> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Id;

    /// Reconstruction of stored containers depends on this exact output. If
    /// this test fails after a dependency change, existing stores may no
    /// longer reconstruct: keep the old zlib, or add `deflate-pack/2`.
    #[test]
    fn deflate_output_is_pinned() {
        // Pseudo-random words: varied enough that every level makes
        // different match-finding decisions.
        let words = [
            "sheet", "cell", "row ", "<c r=\"", "\"/>", "value", "42", "styles", " ", "\n",
        ];
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut input = Vec::new();
        while input.len() < 50_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            input.extend_from_slice(words[(state % 10) as usize].as_bytes());
        }
        let got: Vec<String> = (0..=9)
            .map(|l| Id::of(&deflate(&input, l)).to_hex()[..16].to_string())
            .collect();
        assert_eq!(got, GOLDEN);
    }

    const GOLDEN: [&str; 10] = [
        "a1bce5b0daf407a9",
        "e846ff82f7a97f27",
        "0a4b7b84dbb73db4",
        "5e20802f45c32457",
        "feb16d0feb794a60",
        "eb1531eecd585173",
        "ecfc44b02aecc096",
        "0cad520927f86b7c",
        "4afffe21e6e10699",
        "4afffe21e6e10699",
    ];
}
