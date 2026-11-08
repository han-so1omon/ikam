//! Markup text layers (E112): an XML or HTML content as its tags and its
//! words, apart.
//!
//! The same text in a markdown file, a docx's `word/document.xml` and a PDF
//! shares words but not bytes: in the XML every run of words sits between
//! tags (`<w:t>...</w:t>`), so a compressor matching the docx against the
//! markdown finds short pieces at best. `split` takes the text runs out of
//! the markup: the *skeleton* is the markup with a `0x00` byte where each
//! run was, and the *text* is the runs, one per line. The text then reads
//! like the same document in another format (E102 did this for PDF pages),
//! and the skeleton like every other file written by the same program.
//!
//! A run moves to the text only if it holds no newline (so lines delimit
//! runs); a run with a newline stays in the skeleton verbatim. Input with a
//! `0x00` byte is not split. `build` inverts `split` exactly.

use crate::Error;

/// Byte that stands for one text run in a skeleton.
const RUN: u8 = 0;

/// `(skeleton, text)` for markup `bytes`, or `None` if it holds a `0x00`
/// byte or no text run moves.
pub fn split(bytes: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    if bytes.contains(&RUN) {
        return None;
    }
    let (mut skeleton, mut text) = (Vec::with_capacity(bytes.len()), Vec::new());
    let mut rest = bytes;
    while !rest.is_empty() {
        let run_end = rest.iter().position(|&b| b == b'<').unwrap_or(rest.len());
        let run = &rest[..run_end];
        if !run.is_empty() && !run.contains(&b'\n') {
            skeleton.push(RUN);
            text.extend_from_slice(run);
            text.push(b'\n');
        } else {
            skeleton.extend_from_slice(run);
        }
        rest = &rest[run_end..];
        let tag_end = rest
            .iter()
            .position(|&b| b == b'>')
            .map_or(rest.len(), |e| e + 1);
        skeleton.extend_from_slice(&rest[..tag_end]);
        rest = &rest[tag_end..];
    }
    (!text.is_empty()).then_some((skeleton, text))
}

/// The markup `split` took apart.
pub fn build(skeleton: &[u8], text: &[u8]) -> Result<Vec<u8>, Error> {
    let bad = || Error::Exec("markup: text runs do not match skeleton".into());
    let mut out = Vec::with_capacity(skeleton.len() + text.len());
    let mut lines = text.split(|&b| b == b'\n');
    for &b in skeleton {
        if b == RUN {
            out.extend_from_slice(lines.next().ok_or_else(bad)?);
        } else {
            out.push(b);
        }
    }
    // `text` ends with a newline, so one empty piece is left.
    match (lines.next(), lines.next()) {
        (Some([]), None) => Ok(out),
        (None, None) if text.is_empty() => Ok(out),
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn splits_runs_out_of_tags() {
        let xml = b"<?xml version=\"1.0\"?>\r\n<w:p><w:t>Hello world</w:t><w:t xml:space=\"preserve\"> again</w:t></w:p>";
        let (s, t) = split(xml).unwrap();
        assert_eq!(t, b"Hello world\n again\n");
        assert!(s.starts_with(b"<?xml version=\"1.0\"?>\r\n<w:p><w:t>\0</w:t>"));
        assert_eq!(build(&s, &t).unwrap(), xml);
    }

    #[test]
    fn refuses_nul_and_markup_without_text() {
        assert!(split(b"<a>x\0</a>").is_none());
        assert!(split(b"<a/><b/>").is_none());
        assert!(build(b"<a>\0</a>", b"x\ny\n").is_err());
        assert!(build(b"<a>\0\0</a>", b"x\n").is_err());
    }

    proptest! {
        #[test]
        fn build_inverts_split(bytes in proptest::collection::vec(
            prop_oneof![Just(b'<'), Just(b'>'), Just(b'\n'), Just(b'a'), Just(b' ')], 0..200)) {
            if let Some((s, t)) = split(&bytes) {
                prop_assert_eq!(build(&s, &t).unwrap(), bytes);
            }
        }
    }
}
