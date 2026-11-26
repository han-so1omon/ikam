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

/// The pieces of a safetensors file, or `None` if `bytes` is not one.
pub fn cuts(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    if bytes.len() < MIN_FILE {
        return None;
    }
    let n = u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?) as usize;
    let header = bytes.get(8..8usize.checked_add(n)?)?;
    let text = std::str::from_utf8(header).ok()?;
    if !text.trim_start().starts_with('{') || !text.trim_end().ends_with('}') {
        return None;
    }
    let data = &bytes[8 + n..];
    let mut spans = Vec::new();
    let key = "\"data_offsets\"";
    let mut rest = text;
    while let Some(at) = rest.find(key) {
        rest = &rest[at + key.len()..];
        let open = rest.find('[')?;
        if !rest[..open]
            .trim()
            .trim_start_matches(':')
            .trim()
            .is_empty()
        {
            return None;
        }
        let close = rest.find(']')?;
        let mut nums = rest[open + 1..close]
            .split(',')
            .map(|s| s.trim().parse::<usize>());
        let (start, end) = (nums.next()?.ok()?, nums.next()?.ok()?);
        if nums.next().is_some() || start > end || end > data.len() {
            return None;
        }
        spans.push((start, end));
        if spans.len() > MAX_TENSORS {
            return None;
        }
        rest = &rest[close..];
    }
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
}
