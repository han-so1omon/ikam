//! PDF text layer (E102): a page's content stream as the text it shows.
//!
//! A PDF content stream draws text as hex strings of glyph codes in a
//! subset font (`<0102...>Tj`), so the words a note, a web page and a
//! document share with the PDF are invisible to dedup and to compression
//! contexts. Each font's `/ToUnicode` CMap (a stream `plan_zlib` already
//! expands) maps codes to text. `plan` splits a content stream into
//!
//! - a skeleton: the stream with every hex string it can decode replaced
//!   by a marker naming the font and the string's length in characters;
//! - the text: the decoded strings in order, UTF-8, one line per text
//!   operator (`Tj`, `TJ`, `'`, `"`), so its words line up with the same
//!   words elsewhere;
//!
//! and `build` rebuilds the stream from them and the CMaps (a
//! `deflate-pack` text-layer segment; see `container.rs`).
//! A string is replaced only if encoding its text back through the CMap
//! (longest match, lowest code first) gives its codes exactly; anything
//! else stays in the skeleton verbatim. Nothing here needs the PDF to be
//! well formed: reconstruction only inverts the skeleton, and ingest
//! verifies it.
//!
//! Skeleton: stream bytes, with escape byte `ESC`:
//! `ESC 0` is a literal `ESC`; `ESC t n` (t >= 1, varint n) is a hex string
//! in font `(t - 1) >> 1` (the CMap at argument 2 + font) holding the next
//! n characters of the text, followed by one newline in the text if
//! `(t - 1) & 1`.

use std::collections::{BTreeMap, HashMap};

use crate::Error;
use crate::group::{put_varint, varint};

const ESC: u8 = 0x01;
/// Fonts one skeleton can name (the tag byte holds `1 + 2 * font + 1`).
const MAX_FONTS: usize = 127;

/// A font's `/ToUnicode` map.
pub(crate) struct CMap {
    width: usize,
    to_text: HashMap<Vec<u8>, String>,
    from_text: HashMap<String, Vec<u8>>,
    /// Longest text, in characters, one code maps to.
    longest: usize,
}

impl CMap {
    /// Parse the `bfchar` and `bfrange` entries of a CMap (one code width,
    /// the first code space range's). `None` if it maps nothing.
    pub(crate) fn parse(b: &[u8]) -> Option<CMap> {
        let space = section(b, b"begincodespacerange", b"endcodespacerange")?;
        let width = hex_tokens(space).next()?.len();
        if !(1..=4).contains(&width) {
            return None;
        }
        let mut map = BTreeMap::new();
        let mut rest = b;
        while let Some(at) = find(rest, b"beginbfchar") {
            rest = &rest[at..];
            let block = section(rest, b"beginbfchar", b"endbfchar")?;
            let toks: Vec<Vec<u8>> = hex_tokens(block).collect();
            for pair in toks.chunks_exact(2) {
                if let Some(t) = utf16(&pair[1]) {
                    map.insert(pair[0].clone(), t);
                }
            }
            rest = &rest[11..];
        }
        let mut rest = b;
        while let Some(at) = find(rest, b"beginbfrange") {
            rest = &rest[at..];
            let block = section(rest, b"beginbfrange", b"endbfrange")?;
            // Only the `<lo> <hi> <dst>` form; entries with a destination
            // array are skipped (their codes stay unmapped).
            for line in block.split(|&c| c == b'\n') {
                if line.contains(&b'[') {
                    continue;
                }
                let toks: Vec<Vec<u8>> = hex_tokens(line).collect();
                if toks.len() != 3 || toks[0].len() != toks[1].len() || toks[2].is_empty() {
                    continue;
                }
                let (lo, hi) = (be(&toks[0]), be(&toks[1]));
                if hi < lo || hi - lo > 0xffff {
                    continue;
                }
                let dst = toks[2].clone();
                for k in 0..=(hi - lo) {
                    let mut d = dst.clone();
                    let last = d.len() - 1;
                    let Some(v) = u64::from(d[last]).checked_add(k) else {
                        continue;
                    };
                    if v > 0xff {
                        continue;
                    }
                    d[last] = v as u8;
                    let code = (lo + k).to_be_bytes()[8 - toks[0].len()..].to_vec();
                    if let Some(t) = utf16(&d) {
                        map.insert(code, t);
                    }
                }
            }
            rest = &rest[12..];
        }
        map.retain(|code: &Vec<u8>, t: &mut String| code.len() == width && !t.is_empty());
        if map.is_empty() {
            return None;
        }
        let mut from_text = HashMap::new();
        for (code, t) in &map {
            // Lowest code first (the map is ordered by code).
            from_text.entry(t.clone()).or_insert_with(|| code.clone());
        }
        let longest = from_text.keys().map(|t| t.chars().count()).max()?;
        Some(CMap {
            width,
            to_text: map.into_iter().collect(),
            from_text,
            longest,
        })
    }

    fn decode(&self, codes: &[u8]) -> Option<String> {
        if codes.is_empty() || !codes.len().is_multiple_of(self.width) {
            return None;
        }
        let mut s = String::new();
        for c in codes.chunks(self.width) {
            s.push_str(self.to_text.get(c)?);
        }
        Some(s)
    }

    /// The codes for `text`: longest match first.
    fn encode(&self, text: &[char]) -> Option<Vec<u8>> {
        let (mut out, mut i, mut key) = (Vec::new(), 0, String::new());
        'next: while i < text.len() {
            for l in (1..=self.longest.min(text.len() - i)).rev() {
                key.clear();
                key.extend(&text[i..i + l]);
                if let Some(code) = self.from_text.get(&key) {
                    out.extend_from_slice(code);
                    i += l;
                    continue 'next;
                }
            }
            return None;
        }
        Some(out)
    }
}

/// `(skeleton, text, fonts)` for a content stream drawing text in the named
/// fonts, `fonts[i]` being the index into `cmaps` of the skeleton's font i;
/// `None` if no string can be replaced.
pub(crate) fn plan(
    stream: &[u8],
    cmaps: &[(&[u8], &CMap)],
) -> Option<(Vec<u8>, Vec<u8>, Vec<usize>)> {
    let (mut skel, mut text, mut used) = (Vec::new(), String::new(), Vec::<usize>::new());
    let (mut last_name, mut font): (&[u8], Option<usize>) = (b"", None);
    let mut i = 0;
    while i < stream.len() {
        let c = stream[i];
        if c == ESC {
            skel.extend_from_slice(&[ESC, 0]);
            i += 1;
            continue;
        }
        if c == b'/' {
            let end = i + 1 + stream[i + 1..].iter().take_while(|&&b| regular(b)).count();
            last_name = &stream[i + 1..end];
            skel.extend_from_slice(&stream[i..end]);
            i = end;
            continue;
        }
        if c == b'T'
            && stream.get(i + 1) == Some(&b'f')
            && i > 0
            && !regular(stream[i - 1])
            && stream.get(i + 2).is_none_or(|&b| !regular(b))
        {
            font = cmaps.iter().position(|(name, _)| *name == last_name);
        }
        if c == b'<'
            && let Some(f) = font
            && let Some((len, chars)) = hex_string(&stream[i..], cmaps[f].1)
        {
            let fi = match used.iter().position(|&u| u == f) {
                Some(p) => p,
                None if used.len() < MAX_FONTS => {
                    used.push(f);
                    used.len() - 1
                }
                None => {
                    skel.push(c);
                    i += 1;
                    continue;
                }
            };
            let sep = ends_operator(&stream[i + len..]);
            skel.extend_from_slice(&[ESC, 1 + (fi as u8) * 2 + u8::from(sep)]);
            put_varint(&mut skel, chars.chars().count());
            text.push_str(&chars);
            if sep {
                text.push('\n');
            }
            i += len;
            continue;
        }
        skel.push(c);
        i += 1;
    }
    (!used.is_empty()).then(|| (skel, text.into_bytes(), used))
}

/// The content stream from its skeleton, text and fonts' CMaps.
pub(crate) fn build(skel: &[u8], text: &[u8], cmaps: &[&CMap]) -> Result<Vec<u8>, Error> {
    let bad = || Error::Exec("pdf-text: malformed skeleton".into());
    let text: Vec<char> = std::str::from_utf8(text)
        .map_err(|_| bad())?
        .chars()
        .collect();
    let (mut out, mut s, mut t) = (Vec::with_capacity(skel.len() * 2), skel, 0usize);
    while let Some((&c, rest)) = s.split_first() {
        s = rest;
        if c != ESC {
            out.push(c);
            continue;
        }
        let (&tag, rest) = s.split_first().ok_or_else(bad)?;
        s = rest;
        if tag == 0 {
            out.push(ESC);
            continue;
        }
        let (f, sep) = (usize::from(tag - 1) >> 1, (tag - 1) & 1 == 1);
        let n = varint(&mut s).ok_or_else(bad)?;
        let chars = text
            .get(t..t.checked_add(n).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        t += n;
        let codes = cmaps
            .get(f)
            .ok_or_else(bad)?
            .encode(chars)
            .ok_or_else(bad)?;
        out.push(b'<');
        for b in codes {
            out.extend_from_slice(format!("{b:02X}").as_bytes());
        }
        out.push(b'>');
        if sep {
            if text.get(t) != Some(&'\n') {
                return Err(bad());
            }
            t += 1;
        }
    }
    if t != text.len() {
        return Err(bad());
    }
    Ok(out)
}

/// A replaceable hex string at the start of `b` (`<` uppercase hex `>`):
/// its length in bytes and its text, if the CMap round-trips its codes.
fn hex_string(b: &[u8], cmap: &CMap) -> Option<(usize, String)> {
    let n = b[1..]
        .iter()
        .take_while(|c| matches!(c, b'0'..=b'9' | b'A'..=b'F'))
        .count();
    if n == 0 || n % 2 != 0 || b.get(1 + n) != Some(&b'>') {
        return None;
    }
    let codes = unhex(&b[1..1 + n])?;
    let s = cmap.decode(&codes)?;
    let chars: Vec<char> = s.chars().collect();
    (cmap.encode(&chars)? == codes).then_some((n + 2, s))
}

/// Whether a text operator follows a string: `Tj`, `'`, `"`, or the `]` of
/// a `TJ` array.
fn ends_operator(b: &[u8]) -> bool {
    let b = skip_ws(b);
    if b.starts_with(b"Tj") || b.starts_with(b"'") || b.starts_with(b"\"") {
        return true;
    }
    b.starts_with(b"]") && skip_ws(&b[1..]).starts_with(b"TJ")
}

fn skip_ws(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|c| c.is_ascii_whitespace()).count();
    &b[n..]
}

/// A PDF regular character (not whitespace or a delimiter).
fn regular(b: u8) -> bool {
    !b.is_ascii_whitespace() && !b"()<>[]{}/%".contains(&b) && b != 0
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn section<'a>(b: &'a [u8], begin: &[u8], end: &[u8]) -> Option<&'a [u8]> {
    let s = find(b, begin)? + begin.len();
    let e = find(&b[s..], end)?;
    Some(&b[s..s + e])
}

/// The hex strings in `b`, as bytes.
fn hex_tokens(b: &[u8]) -> impl Iterator<Item = Vec<u8>> + '_ {
    b.split(|&c| c == b'<').skip(1).filter_map(|t| {
        let end = t.iter().position(|&c| c == b'>')?;
        unhex(&t[..end])
    })
}

fn unhex(h: &[u8]) -> Option<Vec<u8>> {
    let h: Vec<u8> = h
        .iter()
        .copied()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    if !h.len().is_multiple_of(2) {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    h.chunks(2)
        .map(|p| Some(digit(p[0])? << 4 | digit(p[1])?))
        .collect()
}

fn be(b: &[u8]) -> u64 {
    b.iter().fold(0, |n, &x| n << 8 | u64::from(x))
}

fn utf16(b: &[u8]) -> Option<String> {
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = b
        .chunks(2)
        .map(|p| u16::from_be_bytes([p[0], p[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// For each zlib member of a PDF (`starts`: the offset of each member's
/// stream), the fonts of the page it is the content stream of, as
/// `(resource name, index of the member holding its ToUnicode CMap)`;
/// empty for other members. Reads only what it needs: objects by number,
/// pages' `/Contents`, `/Resources` and `/Font`, fonts' `/ToUnicode`.
pub(crate) fn content_fonts(pdf: &[u8], starts: &[usize]) -> Vec<Vec<(Vec<u8>, usize)>> {
    let objects = objects(pdf);
    let mut member_of: HashMap<u32, usize> = HashMap::new();
    for (k, &s) in starts.iter().enumerate() {
        let at = objects.partition_point(|o| o.start <= s);
        if let Some(o) = at.checked_sub(1).map(|a| &objects[a])
            && s < o.end
        {
            member_of.entry(o.num).or_insert(k);
        }
    }
    let by_num: HashMap<u32, &Obj> = objects.iter().map(|o| (o.num, o)).collect();
    let dict_of = |num: u32| by_num.get(&num).map(|o| o.dict(pdf));
    // A dictionary value: inline, or a reference to an object's dictionary.
    let resolve = |v: &'_ [u8]| -> Option<Vec<u8>> {
        let v = skip_ws(v);
        if v.starts_with(b"<<") {
            return balanced(v).map(<[u8]>::to_vec);
        }
        dict_of(reference(v)?).map(<[u8]>::to_vec)
    };
    let mut out = vec![Vec::new(); starts.len()];
    for o in &objects {
        let dict = o.dict(pdf);
        if !is_page(dict) {
            continue;
        }
        let Some(fonts) = value(dict, b"/Resources")
            .and_then(resolve)
            .and_then(|r| value(&r, b"/Font").and_then(resolve))
        else {
            continue;
        };
        let mut named = Vec::new();
        let mut rest = fonts.as_slice();
        while let Some(at) = rest.iter().position(|&c| c == b'/') {
            rest = &rest[at + 1..];
            let n = rest.iter().take_while(|&&b| regular(b)).count();
            let name = rest[..n].to_vec();
            rest = &rest[n..];
            let Some(cmap) = reference(rest)
                .and_then(dict_of)
                .and_then(|f| value(f, b"/ToUnicode"))
                .and_then(reference)
                .and_then(|n| member_of.get(&n).copied())
            else {
                continue;
            };
            named.push((name, cmap));
        }
        let Some(contents) = value(dict, b"/Contents") else {
            continue;
        };
        let contents = skip_ws(contents);
        let refs: Vec<u32> = if contents.starts_with(b"[") {
            let end = contents
                .iter()
                .position(|&c| c == b']')
                .unwrap_or(contents.len());
            let mut refs = Vec::new();
            let mut r = &contents[1..end];
            while let Some(n) = reference(r) {
                refs.push(n);
                let at = r.iter().position(|&c| c == b'R').unwrap_or(r.len() - 1);
                r = &r[at + 1..];
            }
            refs
        } else {
            reference(contents).into_iter().collect()
        };
        for n in refs {
            if let Some(&m) = member_of.get(&n)
                && out[m].is_empty()
            {
                out[m] = named.clone();
            }
        }
    }
    out
}

/// An indirect object: its number and byte span (header to `endobj`).
struct Obj {
    num: u32,
    start: usize,
    body: usize,
    end: usize,
}

impl Obj {
    /// The object's dictionary, up to `stream` if it has one.
    fn dict<'a>(&self, pdf: &'a [u8]) -> &'a [u8] {
        let body = &pdf[self.body..self.end];
        balanced(skip_ws(body)).unwrap_or(&body[..0])
    }
}

/// Indirect objects (`N G obj ... endobj`) in file order.
fn objects(pdf: &[u8]) -> Vec<Obj> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = find(&pdf[i..], b" obj") {
        let kw = i + at;
        i = kw + 4;
        if pdf.get(kw + 4).is_some_and(|&b| regular(b)) {
            continue;
        }
        // Backwards over "num gen".
        let digits = |end: usize| {
            pdf[..end]
                .iter()
                .rev()
                .take_while(|b| b.is_ascii_digit())
                .count()
        };
        let g = digits(kw);
        if g == 0 || kw - g == 0 || pdf[kw - g - 1] != b' ' {
            continue;
        }
        let n_end = kw - g - 1;
        let n = digits(n_end);
        if n == 0 {
            continue;
        }
        let Some(num) = std::str::from_utf8(&pdf[n_end - n..n_end])
            .ok()
            .and_then(|s| s.parse().ok())
        else {
            continue;
        };
        let end = find(&pdf[i..], b"endobj").map_or(pdf.len(), |e| i + e);
        out.push(Obj {
            num,
            start: n_end - n,
            body: i,
            end,
        });
        i = end;
    }
    out
}

fn is_page(dict: &[u8]) -> bool {
    let mut rest = dict;
    while let Some(v) = value(rest, b"/Type") {
        let v = skip_ws(v);
        if v.starts_with(b"/Page") && v.get(5).is_none_or(|&b| !regular(b)) {
            return true;
        }
        rest = v;
    }
    false
}

/// The bytes after `key` (a whole name) in a dictionary.
fn value<'a>(dict: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut i = 0;
    while let Some(at) = find(&dict[i..], key) {
        let end = i + at + key.len();
        if dict.get(end).is_none_or(|&b| !regular(b)) {
            return Some(&dict[end..]);
        }
        i = end;
    }
    None
}

/// The object number of an indirect reference `N G R` at the start of `v`.
fn reference(v: &[u8]) -> Option<u32> {
    let v = skip_ws(v);
    let n = v.iter().take_while(|b| b.is_ascii_digit()).count();
    let rest = skip_ws(&v[n..]);
    let g = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    let rest = skip_ws(&rest[g..]);
    if n == 0 || g == 0 || !rest.starts_with(b"R") || rest.get(1).is_some_and(|&b| regular(b)) {
        return None;
    }
    std::str::from_utf8(&v[..n]).ok()?.parse().ok()
}

/// The balanced `<< ... >>` at the start of `v`, delimiters included.
fn balanced(v: &[u8]) -> Option<&[u8]> {
    if !v.starts_with(b"<<") {
        return None;
    }
    let (mut depth, mut i) = (0usize, 0);
    while i + 1 < v.len() {
        if &v[i..i + 2] == b"<<" {
            depth += 1;
            i += 2;
        } else if &v[i..i + 2] == b">>" {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Some(&v[..i]);
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMAP: &[u8] = b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
1 begincodespacerange\n<00> <FF>\nendcodespacerange\n4 beginbfchar\n<01> <0048>\n<02> <0069>\n\
<03> <0020>\n<04> <FB01>\nendbfchar\n1 beginbfrange\n<05> <07> <0061>\nendbfrange\nendcmap\n";

    #[test]
    fn parses_chars_and_ranges() {
        let m = CMap::parse(CMAP).unwrap();
        assert_eq!(m.decode(&[1, 2, 3, 5, 6, 7, 4]).unwrap(), "Hi abc\u{fb01}");
        let t: Vec<char> = "Hi abc".chars().collect();
        assert_eq!(m.encode(&t).unwrap(), vec![1, 2, 3, 5, 6, 7]);
    }

    #[test]
    fn plan_and_build_round_trip() {
        let m = CMap::parse(CMAP).unwrap();
        let stream = b"/P<</MCID 0>>BDC BT /F1 12 Tf<010203>Tj ET\nBT /F1 9 Tf[<0506>1<07>-3<0102>]TJ\x01 <zz><0909>Tj <> ET";
        let (skel, text, used) = plan(stream, &[(b"F1", &m)]).unwrap();
        assert_eq!(used, vec![0]);
        assert_eq!(text, "Hi \nabcHi\n".as_bytes());
        let out = build(&skel, &text, &[&m]).unwrap();
        assert_eq!(out, stream.to_vec());
    }

    #[test]
    fn unknown_fonts_and_lowercase_hex_stay_verbatim() {
        let m = CMap::parse(CMAP).unwrap();
        let stream = b"BT /F2 12 Tf<0102>Tj /F1 1 Tf<0a0b>Tj ET";
        assert!(plan(stream, &[(b"F1", &m)]).is_none());
    }

    #[test]
    fn build_rejects_short_text() {
        let m = CMap::parse(CMAP).unwrap();
        let (skel, _, _) = plan(b"/F1 1 Tf<0102>Tj", &[(b"F1", &m)]).unwrap();
        assert!(build(&skel, b"H", &[&m]).is_err());
    }

    #[test]
    fn finds_page_fonts() {
        let pdf = b"%PDF-1.4\n1 0 obj\n<</Type/Page/Resources 3 0 R/Contents 2 0 R>>\nendobj\n\
2 0 obj\n<</Length 5>>\nstream\nXXXXX\nendstream\nendobj\n3 0 obj\n<</Font<</F1 4 0 R>>>>\nendobj\n\
4 0 obj\n<</Type/Font/ToUnicode 5 0 R>>\nendobj\n5 0 obj\n<</Length 5>>\nstream\nYYYYY\nendstream\nendobj\n";
        let s2 = find(pdf, b"XXXXX").unwrap();
        let s5 = find(pdf, b"YYYYY").unwrap();
        let fonts = content_fonts(pdf, &[s2, s5]);
        assert_eq!(fonts, vec![vec![(b"F1".to_vec(), 1)], vec![]]);
    }
}
