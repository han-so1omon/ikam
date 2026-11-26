//! Checkpoint-aware cuts (E411, after Hugging Face's Xet and the
//! safetensors format): a safetensors file split at its tensors'
//! boundaries, so a checkpoint whose tensors are unchanged since the last
//! one stores only the tensors that changed.
//!
//! A safetensors file is an 8-byte little-endian header length `n`, `n`
//! bytes of JSON naming each tensor's `data_offsets` `[start, end]` within
//! the data that follows, then the data. `cuts` returns the header (with
//! its length prefix) and the data cut at every tensor boundary, gaps kept
//! as pieces of their own, so the pieces concatenated are the file.

/// Files smaller than this are not cut: their tensors are too small to
/// repay a record each.
pub const MIN_FILE: usize = 64 * 1024;

/// The most tensors a file may name and still be cut (a bound on the
/// header scan and on the record's arguments).
pub const MAX_TENSORS: usize = 4096;

/// Each tensor's `data_offsets` in a safetensors header, read as JSON: a
/// top-level object whose `__metadata__` entry (any value) is skipped and
/// whose other entries are objects, each with `data_offsets` `[start,
/// end]` within `len` bytes of data. Text inside strings (free-form
/// metadata) is never taken for a field. `None` for anything else.
fn header_spans(h: &[u8], len: usize) -> Option<Vec<(usize, usize)>> {
    let mut i = 0;
    let mut spans = Vec::new();
    ws(h, &mut i);
    eat(h, &mut i, b'{')?;
    ws(h, &mut i);
    if h.get(i) == Some(&b'}') {
        i += 1;
    } else {
        loop {
            let key = string(h, &mut i)?;
            ws(h, &mut i);
            eat(h, &mut i, b':')?;
            ws(h, &mut i);
            if key == b"__metadata__" {
                skip(h, &mut i, 0)?;
            } else {
                spans.push(tensor(h, &mut i, len)?);
                if spans.len() > MAX_TENSORS {
                    return None;
                }
            }
            ws(h, &mut i);
            match h.get(i)? {
                b',' => {
                    i += 1;
                    ws(h, &mut i);
                }
                b'}' => {
                    i += 1;
                    break;
                }
                _ => return None,
            }
        }
    }
    ws(h, &mut i);
    (i == h.len()).then_some(spans)
}

fn ws(h: &[u8], i: &mut usize) {
    while h.get(*i).is_some_and(|c| c.is_ascii_whitespace()) {
        *i += 1;
    }
}

fn eat(h: &[u8], i: &mut usize, c: u8) -> Option<()> {
    (h.get(*i) == Some(&c)).then(|| *i += 1)
}

/// A string's raw bytes (escapes left as written), after its quotes.
fn string<'a>(h: &'a [u8], i: &mut usize) -> Option<&'a [u8]> {
    eat(h, i, b'"')?;
    let from = *i;
    loop {
        match h.get(*i)? {
            b'"' => break,
            b'\\' => *i += 2,
            _ => *i += 1,
        }
    }
    let s = h.get(from..*i)?;
    *i += 1;
    Some(s)
}

/// Step over one JSON value of any kind (nested to a bounded depth).
fn skip(h: &[u8], i: &mut usize, depth: usize) -> Option<()> {
    if depth > 64 {
        return None;
    }
    match h.get(*i)? {
        b'"' => string(h, i).map(|_| ()),
        open @ (b'{' | b'[') => {
            let close = if *open == b'{' { b'}' } else { b']' };
            *i += 1;
            ws(h, i);
            if h.get(*i) == Some(&close) {
                *i += 1;
                return Some(());
            }
            loop {
                if *open == b'{' {
                    string(h, i)?;
                    ws(h, i);
                    eat(h, i, b':')?;
                    ws(h, i);
                }
                skip(h, i, depth + 1)?;
                ws(h, i);
                match h.get(*i)? {
                    b',' => {
                        *i += 1;
                        ws(h, i);
                    }
                    c if *c == close => {
                        *i += 1;
                        return Some(());
                    }
                    _ => return None,
                }
            }
        }
        _ => {
            let from = *i;
            while h
                .get(*i)
                .is_some_and(|c| !matches!(c, b',' | b'}' | b']') && !c.is_ascii_whitespace())
            {
                *i += 1;
            }
            (*i > from).then_some(())
        }
    }
}

/// A tensor's entry: an object with `data_offsets` `[start, end]`.
fn tensor(h: &[u8], i: &mut usize, len: usize) -> Option<(usize, usize)> {
    eat(h, i, b'{')?;
    let mut span = None;
    loop {
        ws(h, i);
        let key = string(h, i)?;
        ws(h, i);
        eat(h, i, b':')?;
        ws(h, i);
        if key == b"data_offsets" {
            eat(h, i, b'[')?;
            let mut nums = [0usize; 2];
            for (k, n) in nums.iter_mut().enumerate() {
                ws(h, i);
                let from = *i;
                while h.get(*i).is_some_and(u8::is_ascii_digit) {
                    *i += 1;
                }
                *n = std::str::from_utf8(&h[from..*i]).ok()?.parse().ok()?;
                ws(h, i);
                eat(h, i, if k == 0 { b',' } else { b']' })?;
            }
            if span.is_some() || nums[0] > nums[1] || nums[1] > len {
                return None;
            }
            span = Some((nums[0], nums[1]));
        } else {
            skip(h, i, 1)?;
        }
        ws(h, i);
        match h.get(*i)? {
            b',' => *i += 1,
            b'}' => {
                *i += 1;
                return span;
            }
            _ => return None,
        }
    }
}

/// The pieces of a safetensors file, or `None` if `bytes` is not one.
pub fn cuts(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    if bytes.len() < MIN_FILE {
        return None;
    }
    let n = u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?) as usize;
    let header = bytes.get(8..8usize.checked_add(n)?)?;
    std::str::from_utf8(header).ok()?;
    let data = &bytes[8 + n..];
    let mut spans = header_spans(header, data.len())?;
    if spans.is_empty() {
        return None;
    }
    spans.sort_unstable();
    let mut pieces = vec![&bytes[..8 + n]];
    let mut at = 0;
    for (start, end) in spans {
        if start < at {
            return None; // tensors that overlap: not a file to cut
        }
        if start > at {
            pieces.push(&data[at..start]);
        }
        if end > start {
            pieces.push(&data[start..end]);
        }
        at = end;
    }
    if at < data.len() {
        pieces.push(&data[at..]);
    }
    Some(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(tensors: &[(&str, usize)], gap: usize) -> Vec<u8> {
        let mut header = String::from("{");
        let mut at = 0;
        for (i, (name, len)) in tensors.iter().enumerate() {
            if i > 0 {
                header.push(',');
            }
            header.push_str(&format!(
                "\"{name}\":{{\"dtype\":\"F32\",\"shape\":[{}],\"data_offsets\":[{at},{}]}}",
                len / 4,
                at + len
            ));
            at += len + gap;
        }
        header.push('}');
        let mut out = (header.len() as u64).to_le_bytes().to_vec();
        out.extend(header.as_bytes());
        out.extend((0..at).map(|i| (i * 31 % 251) as u8));
        out
    }

    #[test]
    fn pieces_concatenate_to_the_file() {
        for gap in [0, 8] {
            let f = file(&[("a", 40000), ("b", 4), ("c", 0), ("d", 30000)], gap);
            let p = cuts(&f).unwrap();
            assert_eq!(p.concat(), f, "gap {gap}");
            // The header, then a, b and d (c is empty); with gaps, also the
            // gaps after a, b and c and the one after d at the end.
            assert_eq!(p.len(), if gap == 0 { 4 } else { 8 });
        }
    }

    #[test]
    fn other_files_are_not_cut() {
        let f = file(&[("a", 70000)], 0);
        assert!(cuts(&f[..1000]).is_none());
        let mut bad = f.clone();
        bad[0] = 0xff; // header length past the end
        assert!(cuts(&bad).is_none());
        let mut over = f.clone();
        let at = String::from_utf8_lossy(&over).find("70000]").unwrap();
        over[at..at + 6].copy_from_slice(b"99999]"); // past the data
        assert!(cuts(&over).is_none());
        assert!(cuts(&vec![b'x'; 100_000]).is_none());
        let text = include_bytes!("tensors.rs").repeat(40);
        assert!(cuts(&text).is_none());
    }

    /// Free-form metadata strings may hold any text, `data_offsets`
    /// included: only real fields count (Codex review on PR 99).
    #[test]
    fn metadata_text_is_not_taken_for_a_tensor() {
        let f = file(&[("a", 40000), ("b", 30000)], 0);
        let n = u64::from_le_bytes(f[..8].try_into().unwrap()) as usize;
        let header = std::str::from_utf8(&f[8..8 + n]).unwrap();
        let meta = r#"{"__metadata__":{"note":"\"data_offsets\":[0,4] {[","n":[1,{"x":null}]},"#;
        let header = header.replacen('{', meta, 1);
        let mut g = (header.len() as u64).to_le_bytes().to_vec();
        g.extend(header.as_bytes());
        g.extend(&f[8 + n..]);
        let p = cuts(&g).unwrap();
        assert_eq!(p.concat(), g);
        assert_eq!(p.len(), 3);
    }
}
