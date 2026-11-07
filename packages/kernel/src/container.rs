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
//! Manifest: count:u32 { 0 len:u32 bytes | 1 member:u32 level:u8
//! | 2 level:u8 skeleton:u32 start:u32 len:u32 text:u32 start:u32 len:u32
//!   fonts:u8 cmap:u32* }
//! where a `2` segment (E102) is a PDF content stream rebuilt from spans of
//! a skeleton member and a text member and its fonts' CMap members
//! (`pdftext.rs`), then deflated at `level`;
//! and level 0-9 is zlib's level at memLevel 8 (zlib's default), and
//! 10-19 the same levels at memLevel 9 (E083: LibreOffice writes PDF fonts
//! so).
//!
//! `plan_zlib` (E080) finds zlib streams anywhere in a file (PDF
//! FlateDecode streams, PNG data, embedded objects): a valid zlib header,
//! a deflate stream that ends, and the Adler-32 that follows matching the
//! inflated bytes. It finds gzip members the same way (E106: .tar.gz, .gz):
//! a valid gzip header, and the CRC-32 and size that follow matching.
//! Headers and checksums stay literal, so `pack` rebuilds the file
//! unchanged. No file type is named: any bytes holding reproducible
//! zlib streams are planned the same way.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, OnceLock};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

use crate::{Error, Id, pdftext};

/// Levels to try, most common first (zlib's default is 6), at memLevel 8
/// and then 9 (`MEM9` added; see the manifest).
const LEVELS: [u8; 20] = [
    6, 4, 5, 1, 2, 3, 7, 8, 9, 0, 16, 14, 15, 11, 12, 13, 17, 18, 19, 10,
];
const MEM9: u8 = 10;
/// Never inflate a member beyond this many bytes (zip-bomb guard).
const MAX_MEMBER: usize = 1 << 30;

enum Segment<'a> {
    Literal(&'a [u8]),
    Deflate { member: u32, level: u8 },
}

/// Raw deflate outputs of large members, kept for one repack (E109). A
/// repack rebuilds each container many times (reading the store, verifying
/// each re-plan, fsck, pricing plans), and each rebuild re-deflates every
/// member: on four pandas source archives that was most of the repack. Keyed
/// by the member's content id and level, so a hit is the same pure function
/// of the same bytes; a member is deflated once even when re-plans running
/// side by side ask for it at once.
#[derive(Default)]
pub struct DeflateMemo(Mutex<HashMap<(Id, u8), Deflated>>);

/// One member's deflate at one level, made once.
type Deflated = Arc<OnceLock<Vec<u8>>>;

/// Members below this are deflated each time: hashing and holding them would
/// cost about what deflating them does.
const MEMO_MIN: usize = 1 << 20;

impl DeflateMemo {
    fn deflate(&self, data: &[u8], level: u8) -> Vec<u8> {
        if data.len() < MEMO_MIN {
            return deflate(data, level);
        }
        let key = (Id::of_content(data), level);
        let cell = Arc::clone(
            self.0
                .lock()
                .expect("memo poisoned")
                .entry(key)
                .or_default(),
        );
        cell.get_or_init(|| deflate(data, level)).clone()
    }
}

/// Raw deflate of `data` at a manifest level (0-19, see the manifest).
pub fn deflate(data: &[u8], level: u8) -> Vec<u8> {
    if level >= MEM9 {
        return deflate_mem9(data, level - MEM9);
    }
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(level.into()));
    enc.write_all(data).expect("writing to a Vec cannot fail");
    enc.finish().expect("writing to a Vec cannot fail")
}

/// Raw deflate at zlib `level` with memLevel 9, through the pinned zlib
/// directly: flate2 fixes memLevel at 8.
fn deflate_mem9(data: &[u8], level: u8) -> Vec<u8> {
    use libz_sys as z;
    use std::ffi::{c_uint, c_void};
    unsafe extern "C" {
        fn calloc(n: usize, size: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }
    unsafe extern "C" fn zalloc(_: *mut c_void, n: c_uint, size: c_uint) -> *mut c_void {
        unsafe { calloc(n as usize, size as usize) }
    }
    unsafe extern "C" fn zfree(_: *mut c_void, p: *mut c_void) {
        unsafe { free(p) }
    }
    let mut s = z::z_stream {
        next_in: data.as_ptr().cast_mut(),
        avail_in: c_uint::try_from(data.len()).expect("members are below 4 GiB"),
        total_in: 0,
        next_out: std::ptr::null_mut(),
        avail_out: 0,
        total_out: 0,
        msg: std::ptr::null_mut(),
        state: std::ptr::null_mut(),
        zalloc,
        zfree,
        opaque: std::ptr::null_mut(),
        data_type: 0,
        adler: 0,
        reserved: 0,
    };
    // SAFETY: `s` is a zeroed stream with valid allocators; zlib reads only
    // `data` and writes only within `out`, sized by `deflateBound` so one
    // Z_FINISH call completes; the stream is ended before `s` is dropped.
    unsafe {
        let size = std::mem::size_of::<z::z_stream>() as i32;
        let init = z::deflateInit2_(&mut s, level.into(), 8, -15, 9, 0, z::zlibVersion(), size);
        assert_eq!(init, 0, "deflateInit2 failed");
        let mut out = vec![0u8; z::deflateBound(&mut s, data.len() as _) as usize];
        (s.next_out, s.avail_out) = (out.as_mut_ptr(), out.len() as c_uint);
        let done = z::deflate(&mut s, 4); // Z_FINISH
        let n = s.total_out as usize;
        z::deflateEnd(&mut s);
        assert_eq!(done, 1, "deflate did not finish"); // Z_STREAM_END
        out.truncate(n);
        out
    }
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
    let streams = member_streams(bytes)?
        .into_iter()
        .filter_map(|(start, end, size)| Some((start, end, inflate(&bytes[start..end], size)?)));
    manifest(bytes, streams, None)
}

/// Smallest inflated stream `plan_zlib` expands: shorter ones gain less
/// than their manifest entry and are more often chance matches.
const MIN_ZLIB: usize = 64;

/// `(manifest, members)` for the zlib streams in `bytes` that re-deflate
/// exactly, if any. In a PDF, page content streams whose text decodes
/// through their fonts' CMaps become text-layer segments (E102).
pub fn plan_zlib(bytes: &[u8]) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
    let (manifest, members, starts) = plan_zlib_at(bytes)?;
    if bytes.starts_with(b"%PDF")
        && let Some(plan) = text_layers(bytes, &manifest, &members, &starts)
    {
        return Some(plan);
    }
    Some((manifest, members))
}

/// The plan with the PDF's decodable content streams replaced by one
/// skeleton member and one text member (`pdftext.rs`), each stream a
/// segment naming its spans of both and its fonts' CMap members: the text
/// of every page is one content, like the same document in another
/// format, and costs no derivation record of its own. `None` if no stream
/// decodes or the plan does not rebuild `bytes`.
fn text_layers(
    bytes: &[u8],
    manifest: &[u8],
    members: &[Vec<u8>],
    starts: &[usize],
) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
    let fonts = pdftext::content_fonts(bytes, starts);
    let mut parsed: HashMap<usize, Option<pdftext::CMap>> = HashMap::new();
    for f in fonts.iter().flatten() {
        parsed
            .entry(f.1)
            .or_insert_with(|| pdftext::CMap::parse(&members[f.1]));
    }
    let (mut skeleton, mut text) = (Vec::new(), Vec::new());
    // Per layered member: (skeleton span, text span, CMap members).
    let mut layers: HashMap<usize, Layer> = HashMap::new();
    for (k, f) in fonts.iter().enumerate() {
        let named: Vec<(&[u8], usize, &pdftext::CMap)> = f
            .iter()
            .filter_map(|(name, m)| Some((name.as_slice(), *m, parsed[m].as_ref()?)))
            .collect();
        let cmaps: Vec<(&[u8], &pdftext::CMap)> = named.iter().map(|(n, _, c)| (*n, *c)).collect();
        if named.is_empty() || layers.len() >= MAX_LAYERS {
            continue;
        }
        let Some((s, t, used)) = pdftext::plan(&members[k], &cmaps) else {
            continue;
        };
        layers.insert(
            k,
            (
                [skeleton.len(), s.len()],
                [text.len(), t.len()],
                used.iter().map(|&u| named[u].1).collect(),
            ),
        );
        skeleton.extend_from_slice(&s);
        text.extend_from_slice(&t);
    }
    if layers.is_empty() {
        return None;
    }
    // Members: the others in order, then the skeleton and the text.
    let mut renumber = vec![None; members.len()];
    let mut kept = Vec::new();
    for (k, m) in members.iter().enumerate() {
        if !layers.contains_key(&k) {
            renumber[k] = Some(kept.len() as u32);
            kept.push(m.clone());
        }
    }
    let (skel_member, text_member) = (kept.len() as u32, kept.len() as u32 + 1);
    kept.push(skeleton);
    kept.push(text);
    let mut m = Cursor(manifest);
    let count = m.u32()?;
    let mut out = (count as u32).to_be_bytes().to_vec();
    for _ in 0..count {
        match m.take(1)?[0] {
            0 => {
                let len = m.u32()?;
                out.push(0);
                out.extend_from_slice(&(len as u32).to_be_bytes());
                out.extend_from_slice(m.take(len)?);
            }
            _ => {
                let member = m.u32()?;
                let level = m.take(1)?[0];
                match layers.get(&member) {
                    None => {
                        out.push(1);
                        out.extend_from_slice(&renumber[member]?.to_be_bytes());
                        out.push(level);
                    }
                    Some((s, t, cmaps)) => {
                        out.extend_from_slice(&[2, level]);
                        for n in [
                            skel_member,
                            s[0] as u32,
                            s[1] as u32,
                            text_member,
                            t[0] as u32,
                            t[1] as u32,
                        ] {
                            out.extend_from_slice(&n.to_be_bytes());
                        }
                        out.push(cmaps.len() as u8);
                        for &c in cmaps {
                            out.extend_from_slice(&renumber[c]?.to_be_bytes());
                        }
                    }
                }
            }
        }
    }
    let mut args = vec![out];
    args.extend(kept);
    if pack(&args, None).ok()? != bytes {
        return None;
    }
    let members = args.split_off(1);
    Some((args.pop()?, members))
}

/// A layered stream: its skeleton span, text span and CMap members.
type Layer = ([usize; 2], [usize; 2], Vec<usize>);

/// A zlib plan with each member's stream offset.
type ZlibPlan = (Vec<u8>, Vec<Vec<u8>>, Vec<usize>);

/// Layered streams per PDF; also bounds a manifest's font counts (u8).
const MAX_LAYERS: usize = 1 << 20;

/// `plan_zlib`, with the offset in `bytes` of each member's zlib stream
/// (its header), so a caller can relate members to the file's structure
/// (E102: PDF objects).
pub fn plan_zlib_at(bytes: &[u8]) -> Option<ZlibPlan> {
    let mut found = Vec::new();
    let mut i = 0;
    while i + 2 < bytes.len() {
        let b = &bytes[i..];
        match zlib_stream(b)
            .map(|(n, d)| (2, n, d, 4))
            .or_else(|| gzip_stream(b))
        {
            Some((head, n, data, tail)) => {
                found.push((i + head, i + head + n, data));
                i += head + n + tail;
            }
            None => i += 1,
        }
    }
    let mut starts = Vec::new();
    let (manifest, members) = manifest(bytes, found.into_iter(), Some(&mut starts))?;
    Some((manifest, members, starts))
}

/// A zlib stream at the start of `b`: the deflate stream's length and its
/// inflated bytes, if the header is valid, the stream ends, and the
/// Adler-32 after it matches.
fn zlib_stream(b: &[u8]) -> Option<(usize, Vec<u8>)> {
    let (cmf, flg) = (*b.first()?, *b.get(1)?);
    if cmf & 0x0f != 8
        || cmf >> 4 > 7
        || flg & 0x20 != 0
        || (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0
    {
        return None;
    }
    let (n, out) = inflate_raw(&b[2..])?;
    let adler = u32::from_be_bytes(b.get(2 + n..2 + n + 4)?.try_into().ok()?);
    (out.len() >= MIN_ZLIB && adler32(&out) == adler).then_some((n, out))
}

/// A gzip member at the start of `b` (E106: .tar.gz, .gz, HTTP bodies):
/// header length, deflate stream length, inflated bytes and trailer length,
/// if the header is valid, the stream ends, and the CRC-32 and size after
/// it match. Header and trailer stay literal, as a zlib stream's do.
fn gzip_stream(b: &[u8]) -> Option<(usize, usize, Vec<u8>, usize)> {
    if !b.starts_with(&[0x1f, 0x8b, 8]) || b.get(3)? & 0xe0 != 0 {
        return None;
    }
    let flags = b[3];
    let mut head = 10;
    if flags & 4 != 0 {
        let x = b.get(head..head + 2)?;
        head += 2 + usize::from(u16::from_le_bytes([x[0], x[1]]));
    }
    for bit in [8, 16] {
        if flags & bit != 0 {
            head += memchr::memchr(0, b.get(head..)?)? + 1;
        }
    }
    if flags & 2 != 0 {
        head += 2;
    }
    let (n, out) = inflate_raw(b.get(head..)?)?;
    let trailer = b.get(head + n..head + n + 8)?;
    let crc = u32::from_le_bytes(trailer[..4].try_into().ok()?);
    let size = u32::from_le_bytes(trailer[4..].try_into().ok()?);
    (out.len() >= MIN_ZLIB && crc32fast::hash(&out) == crc && out.len() as u32 == size)
        .then_some((head, n, out, 8))
}

/// A raw deflate stream at the start of `b`: its length and inflated bytes,
/// if it ends within `MAX_MEMBER` bytes of output.
fn inflate_raw(b: &[u8]) -> Option<(usize, Vec<u8>)> {
    let mut inflater = flate2::Decompress::new(false);
    let mut out = Vec::with_capacity(4096);
    loop {
        if out.len() == out.capacity() {
            out.reserve(out.len().max(4096));
        }
        let consumed = inflater.total_in() as usize;
        let status = inflater
            .decompress_vec(&b[consumed..], &mut out, flate2::FlushDecompress::None)
            .ok()?;
        if status == flate2::Status::StreamEnd {
            break;
        }
        if out.len() > MAX_MEMBER
            || (inflater.total_in() as usize == consumed && out.len() < out.capacity())
        {
            return None;
        }
    }
    Some((inflater.total_in() as usize, out))
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        (a, b) = (a % 65521, b % 65521);
    }
    b << 16 | a
}

/// The manifest for `bytes` given its deflate streams `(start, end,
/// inflated)` in order: each stream that re-deflates exactly at some level
/// becomes a member, the rest stays literal.
fn manifest(
    bytes: &[u8],
    streams: impl Iterator<Item = (usize, usize, Vec<u8>)>,
    mut starts: Option<&mut Vec<usize>>,
) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
    let (mut segments, mut members, mut cursor) = (Vec::new(), Vec::new(), 0);
    for (start, end, data) in streams {
        let stream = &bytes[start..end];
        let Some(level) = LEVELS.into_iter().find(|&l| deflate(&data, l) == stream) else {
            continue;
        };
        segments.push(Segment::Literal(&bytes[cursor..start]));
        segments.push(Segment::Deflate {
            member: members.len() as u32,
            level,
        });
        members.push(data);
        if let Some(starts) = starts.as_deref_mut() {
            starts.push(start - 2);
        }
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
/// With a memo, large members' deflates are reused (see `DeflateMemo`).
pub fn pack(args: &[Vec<u8>], memo: Option<&DeflateMemo>) -> Result<Vec<u8>, Error> {
    let deflate = |data: &[u8], level| match memo {
        Some(m) => m.deflate(data, level),
        None => deflate(data, level),
    };
    let bad = || Error::Exec("deflate-pack: malformed manifest".into());
    let (manifest, members) = args.split_first().ok_or_else(bad)?;
    let mut m = Cursor(manifest);
    let mut out = Vec::new();
    let mut cmaps: HashMap<usize, pdftext::CMap> = HashMap::new();
    for _ in 0..m.u32().ok_or_else(bad)? {
        match m.take(1).ok_or_else(bad)?[0] {
            0 => {
                let len = m.u32().ok_or_else(bad)?;
                out.extend_from_slice(m.take(len).ok_or_else(bad)?);
            }
            1 => {
                let member = members.get(m.u32().ok_or_else(bad)?).ok_or_else(bad)?;
                let level = m.take(1).ok_or_else(bad)?[0];
                if level > 19 {
                    return Err(bad());
                }
                out.extend_from_slice(&deflate(member, level));
            }
            2 => {
                let level = m.take(1).ok_or_else(bad)?[0];
                let mut span = || -> Result<&[u8], Error> {
                    let member = members.get(m.u32().ok_or_else(bad)?).ok_or_else(bad)?;
                    let (start, len) = (m.u32().ok_or_else(bad)?, m.u32().ok_or_else(bad)?);
                    member
                        .get(start..start.checked_add(len).ok_or_else(bad)?)
                        .ok_or_else(bad)
                };
                let (skeleton, text) = (span()?, span()?);
                let n = m.take(1).ok_or_else(bad)?[0];
                let mut fonts = Vec::new();
                for _ in 0..n {
                    let k = m.u32().ok_or_else(bad)?;
                    if let std::collections::hash_map::Entry::Vacant(e) = cmaps.entry(k) {
                        let cmap = pdftext::CMap::parse(members.get(k).ok_or_else(bad)?);
                        e.insert(cmap.ok_or_else(bad)?);
                    }
                    fonts.push(k);
                }
                let fonts: Vec<&pdftext::CMap> = fonts.iter().map(|k| &cmaps[k]).collect();
                if level > 19 {
                    return Err(bad());
                }
                out.extend_from_slice(&deflate(&pdftext::build(skeleton, text, &fonts)?, level));
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
        let got: Vec<String> = (0..=19)
            .map(|l| Id::of(&deflate(&input, l)).to_hex()[..16].to_string())
            .collect();
        assert_eq!(got[..10], GOLDEN);
        assert_eq!(got[10..], GOLDEN_MEM9);
        let back = |l| inflate(&deflate(&input, l), input.len()).unwrap();
        assert!((0..=19).all(|l| back(l) == input));
    }

    /// zlib streams anywhere in a file (here as a PDF holds them) are
    /// expanded and rebuilt byte-exactly; a stream that does not re-deflate
    /// at any level stays literal (E080).
    #[test]
    fn zlib_streams_expand_and_rebuild() {
        let zlib = |data: &[u8], level: u32| {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), Compression::new(level));
            e.write_all(data).unwrap();
            e.finish().unwrap()
        };
        let text: Vec<u8> = (0..300)
            .flat_map(|i| format!("BT /F1 12 Tf ({i}) Tj ET\n").into_bytes())
            .collect();
        let font: Vec<u8> = (0..4000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8 % 7)
            .collect();
        let mut pdf = b"%PDF-1.7\n1 0 obj <</Filter /FlateDecode>> stream\n".to_vec();
        pdf.extend(zlib(&text, 6));
        pdf.extend(b"\nendstream endobj 2 0 obj stream\n");
        pdf.extend(zlib(&font, 9));
        pdf.extend(b"\nendstream endobj trailer");
        let (manifest, members) = plan_zlib(&pdf).unwrap();
        assert_eq!(members, vec![text, font]);
        let args: Vec<Vec<u8>> = [vec![manifest], members].concat();
        assert_eq!(pack(&args, None).unwrap(), pdf);
        assert!(plan_zlib(b"x\x9c not a stream, just text with a header").is_none());
    }

    /// A memoised deflate is the deflate, asked once or again, at every
    /// level; only members at or above the threshold are kept (E109).
    #[test]
    fn memoised_deflates_are_the_deflates() {
        let memo = DeflateMemo::default();
        let big: Vec<u8> = (0..MEMO_MIN / 8)
            .flat_map(|i| format!("{:07}\n", i * 7919 % 1_000_003).into_bytes())
            .collect();
        for data in [&b"a member below the memo's threshold"[..], &big] {
            for level in [6, 9, MEM9 + 9] {
                assert_eq!(memo.deflate(data, level), deflate(data, level));
                assert_eq!(memo.deflate(data, level), deflate(data, level));
            }
        }
        assert_eq!(memo.0.lock().unwrap().len(), 3); // only the large member
    }

    /// gzip members (a .tar.gz, with a file name in the header; and two
    /// members back to back) are expanded and rebuilt byte-exactly; a
    /// member whose CRC does not match stays literal (E106).
    #[test]
    fn gzip_members_expand_and_rebuild() {
        let gz = |data: &[u8], name: &[u8]| {
            let mut e = flate2::GzBuilder::new()
                .filename(name)
                .mtime(1_700_000_000)
                .write(Vec::new(), Compression::new(9));
            e.write_all(data).unwrap();
            e.finish().unwrap()
        };
        let tar: Vec<u8> = (0..400)
            .flat_map(|i| format!("src/click/core_{i}.py\0def f{i}(): return {i}\n").into_bytes())
            .collect();
        let more = b"a second member, long enough to be worth expanding: ".repeat(4);
        let file = [gz(&tar, b"click-8.1.7.tar"), gz(&more, b"b")].concat();
        let (manifest, members) = plan_zlib(&file).unwrap();
        assert_eq!(members, vec![tar.clone(), more.to_vec()]);
        let args: Vec<Vec<u8>> = [vec![manifest], members].concat();
        assert_eq!(pack(&args, None).unwrap(), file);
        let mut bad = gz(&tar, b"x");
        let n = bad.len();
        bad[n - 8] ^= 1; // CRC-32
        assert!(plan_zlib(&bad).is_none());
    }

    const GOLDEN_MEM9: [&str; 10] = [
        "2f67743f12a102c2",
        "e846ff82f7a97f27",
        "8aa0adf771c4e0c9",
        "5e20802f45c32457",
        "7b2d3bc0d1e880c1",
        "ff6d980e075f8029",
        "fb2aba5f7c9064ff",
        "6494c74d184c6dd7",
        "4afffe21e6e10699",
        "4afffe21e6e10699",
    ];

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
