//! Corpus-level reads shared by the CLI and the local web view: paths in a
//! revision, search with matching lines, and what the store holds.

use std::collections::{BTreeMap, HashMap};

use ikam_kernel::{Error, Id, Kind, Object, Repo, Store};

/// One file of a revision: its path and content id.
pub struct File {
    pub path: String,
    pub id: Id,
}

/// Resolve `REV`, `REV:PATH`, a full hex id or a unique hex prefix of a
/// stored object to an id.
pub fn lookup<S: Store>(repo: &Repo<S>, spec: &str) -> Result<Id, String> {
    if let Some((rev, path)) = spec.split_once(':') {
        let root = tree(repo, &lookup(repo, rev)?)?;
        return at_path(repo, root, path);
    }
    if let Ok(id) = repo.resolve(spec) {
        return Ok(id);
    }
    let hex = spec.to_ascii_lowercase();
    if hex.len() >= 4 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        let ids = repo.store().ids().map_err(err)?;
        let hits: Vec<&Id> = ids
            .iter()
            .filter(|i| i.to_hex().starts_with(&hex))
            .collect();
        if let [one] = hits[..] {
            return Ok(*one);
        }
        if hits.len() > 1 {
            return Err(format!("{spec}: ambiguous ({} objects)", hits.len()));
        }
    }
    Err(format!("{spec}: no such revision, path or object"))
}

/// Tree id of a commit, or the id itself if it names a tree.
pub fn tree<S: Store>(repo: &Repo<S>, id: &Id) -> Result<Id, String> {
    ikam_kernel::snapshot::tree_of(repo, id).map_err(err)
}

fn at_path<S: Store>(repo: &Repo<S>, mut id: Id, path: &str) -> Result<Id, String> {
    for part in path.split('/').filter(|p| !p.is_empty()) {
        let Object::Tree(entries) = repo.get(&id).map_err(err)? else {
            return Err(format!("{path}: {part} is not a directory"));
        };
        id = entries
            .iter()
            .find(|e| e.name == part)
            .map(|e| e.id)
            .ok_or_else(|| format!("{path}: no entry {part}"))?;
    }
    Ok(id)
}

/// Every file under tree `root`, depth first, with its path.
pub fn files<S: Store>(repo: &Repo<S>, root: &Id) -> Result<Vec<File>, Error> {
    let mut out = Vec::new();
    walk(repo, root, "", &mut out)?;
    Ok(out)
}

fn walk<S: Store>(repo: &Repo<S>, id: &Id, prefix: &str, out: &mut Vec<File>) -> Result<(), Error> {
    let Object::Tree(entries) = repo.get(id)? else {
        return Err(Error::WrongKind(*id));
    };
    for e in entries {
        let path = format!("{prefix}{}", e.name);
        match e.kind {
            Kind::Tree => walk(repo, &e.id, &format!("{path}/"), out)?,
            Kind::File => out.push(File { path, id: e.id }),
            Kind::Graph => {}
        }
    }
    Ok(())
}

/// A search hit: a file and up to `MAX_LINES` lines that contain the
/// pattern, from the file itself or from a part of it the kernel stores
/// separately (a zip member such as a docx's XML, a PDF's text layer).
pub struct Hit {
    pub path: String,
    pub id: Id,
    pub lines: Vec<(usize, String)>,
}

const MAX_LINES: usize = 5;

/// Files of `root` whose bytes, or whose stored parts, contain `pattern`,
/// found through the store's search index when it has one.
pub fn grep<S: Store>(repo: &Repo<S>, root: &Id, pattern: &str) -> Result<Vec<Hit>, Error> {
    let all = files(repo, root)?;
    // Each searched content and the files it is (part of).
    let mut owners: HashMap<Id, Vec<usize>> = HashMap::new();
    let mut whole = std::collections::HashSet::new();
    for (k, f) in all.iter().enumerate() {
        owners.entry(f.id).or_default().push(k);
        whole.insert(f.id);
        for d in repo.derivations(&f.id)?.iter().take(1) {
            for a in &d.args {
                owners.entry(a.id()).or_default().push(k);
            }
        }
    }
    let ids: Vec<Id> = owners.keys().copied().collect();
    let index = repo.search_index()?;
    let (found, _) = repo.search(pattern.as_bytes(), &ids, index.as_ref())?;
    let mut lines: BTreeMap<usize, Vec<(usize, String)>> = BTreeMap::new();
    for id in found {
        let bytes = repo.read_content(&id)?;
        // A part of a file (a docx's XML member) is searched as the text a
        // reader sees, not its markup; a file itself is searched as it is.
        let text = match whole.contains(&id) {
            false if looks_like_xml(&bytes) => matching_lines(xml_text(&bytes).as_bytes(), pattern),
            false if is_pdf_text(&bytes) => matching_lines(pdf_text(&bytes).as_bytes(), pattern),
            _ => matching_lines(&bytes, pattern),
        };
        if text.is_empty() && !whole.contains(&id) {
            continue;
        }
        for &k in &owners[&id] {
            let l = lines.entry(k).or_default();
            l.extend(text.iter().cloned());
            l.truncate(MAX_LINES);
        }
    }
    Ok(lines
        .into_iter()
        .map(|(k, lines)| Hit {
            path: all[k].path.clone(),
            id: all[k].id,
            lines,
        })
        .collect())
}

/// A part holding XML: a member, or several members the kernel stores as
/// one content (their declarations anywhere in it).
fn looks_like_xml(bytes: &[u8]) -> bool {
    memchr::memmem::find(bytes, b"<?xml").is_some()
}

/// The character data of an XML document: tags dropped, a line break after
/// each closed paragraph (`</p>`, `</w:p>`, `</a:p>`), entities decoded.
fn xml_text(bytes: &[u8]) -> String {
    let src = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    let mut rest = src.as_ref();
    // A part may begin inside a tag: drop that tag's tail.
    if let Some(gt) = rest.find('>')
        && rest.find('<').is_none_or(|lt| gt < lt)
    {
        rest = &rest[gt + 1..];
    }
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        // Only `<name`, `</`, `<?` and `<!` open a tag; other `<` bytes (in
        // binary around the members) are kept as they are.
        let opens = after
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || "/?!_".contains(c));
        if !opens {
            out.push('<');
            rest = after;
            continue;
        }
        // A tag the part ends inside of is dropped with the rest.
        let Some(close) = after.find('>') else {
            return decode_entities(&out);
        };
        let tag = &after[..close];
        if tag == "/p" || tag.starts_with('/') && tag.ends_with(":p") {
            out.push('\n');
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    decode_entities(&out)
}

/// Entities decoded, lines trimmed; blank lines and lines holding binary
/// (bytes around the members, or a tag the part begins inside of after
/// them) dropped.
fn decode_entities(s: &str) -> String {
    let text: String = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(['\0', '\u{fffd}']))
        .flat_map(|l| [l, "\n"])
        .collect();
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The text a reader sees in a binary file's stored parts (a docx's XML
/// members, a PDF's content streams), for a preview; `None` if it has none.
pub fn readable_text<S: Store>(
    repo: &Repo<S>,
    id: &Id,
    max: usize,
) -> Result<Option<String>, Error> {
    let mut out = String::new();
    let mut seen = std::collections::HashSet::new();
    for d in repo.derivations(id)?.iter().take(1) {
        for a in &d.args {
            if out.len() >= max || !seen.insert(a.id()) {
                continue;
            }
            let bytes = repo.read_content(&a.id())?;
            let text = if looks_like_xml(&bytes) {
                xml_text(&bytes)
            } else if is_pdf_text(&bytes) {
                pdf_text(&bytes)
            } else {
                continue;
            };
            let text = text.trim();
            if !text.is_empty() {
                out.push_str(text);
                out.push_str("\n\n");
            }
        }
    }
    let mut end = out.len().min(max);
    while !out.is_char_boundary(end) {
        end -= 1;
    }
    out.truncate(end);
    Ok((!out.trim().is_empty()).then_some(out))
}

/// A PDF content stream (a part the kernel stores inflated): text objects
/// with show-text operators.
fn is_pdf_text(bytes: &[u8]) -> bool {
    let has = |op: &[u8]| memchr::memmem::find(bytes, op).is_some();
    has(b"BT") && (has(b"Tj") || has(b"TJ"))
}

/// The literal strings a PDF content stream shows, a line per text line
/// (broken at `Td`, `TD`, `T*`, `Tm`, `ET`; a large `TJ` kerning gap reads
/// as a space). Strings in a font's own encoding (hex, CID) are skipped.
fn pdf_text(bytes: &[u8]) -> String {
    let (mut out, mut line) = (String::new(), Vec::new());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                let mut depth = 1;
                i += 1;
                while i < bytes.len() && depth > 0 {
                    match bytes[i] {
                        b'\\' if i + 1 < bytes.len() => {
                            i += 1;
                            line.push(match bytes[i] {
                                b'n' | b'r' => b' ',
                                b't' => b'\t',
                                c => c,
                            });
                        }
                        b'(' => {
                            depth += 1;
                            line.push(b'(');
                        }
                        b')' => {
                            depth -= 1;
                            if depth > 0 {
                                line.push(b')');
                            }
                        }
                        c => line.push(c),
                    }
                    i += 1;
                }
                continue;
            }
            b'-' | b'0'..=b'9' | b'.' => {
                let start = i;
                while i < bytes.len() && matches!(bytes[i], b'-' | b'0'..=b'9' | b'.') {
                    i += 1;
                }
                let gap = std::str::from_utf8(&bytes[start..i])
                    .ok()
                    .and_then(|n| n.parse::<f32>().ok());
                if gap.is_some_and(|g| g <= -200.0) && !line.is_empty() {
                    line.push(b' ');
                }
                continue;
            }
            b'T' | b'E' if i + 1 < bytes.len() => {
                let op = &bytes[i..i + 2];
                if [&b"Td"[..], b"TD", b"T*", b"Tm", b"ET"].contains(&op) && !line.is_empty() {
                    out.push_str(&String::from_utf8_lossy(&line));
                    out.push('\n');
                    line.clear();
                }
            }
            _ => {}
        }
        i += 1;
    }
    if !line.is_empty() {
        out.push_str(&String::from_utf8_lossy(&line));
        out.push('\n');
    }
    out
}

fn matching_lines(bytes: &[u8], pattern: &str) -> Vec<(usize, String)> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.contains(pattern))
        .take(MAX_LINES)
        .map(|(n, l)| (n + 1, snippet(l, pattern)))
        .collect()
}

/// About 160 characters of `line` around the first `pattern`.
fn snippet(line: &str, pattern: &str) -> String {
    let at = line.find(pattern).unwrap_or(0);
    let start = line[..at]
        .char_indices()
        .rev()
        .nth(70)
        .map_or(0, |(i, _)| i);
    line[start..].chars().take(160).collect()
}

/// What the store holds: objects and stored bytes by kind, and the input
/// bytes of the files `rev` names.
pub struct Stats {
    pub kinds: BTreeMap<String, (usize, usize)>,
    pub stored: usize,
    pub files: usize,
    pub input: usize,
}

pub fn stats<S: Store>(repo: &Repo<S>, rev: Option<&Id>) -> Result<Stats, Error> {
    let mut kinds: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut stored = 0;
    for id in repo.store().ids()? {
        let encoded = repo.store().read(&id)?;
        let k = kinds.entry(kind_name(encoded.first())).or_default();
        (k.0, k.1) = (k.0 + 1, k.1 + encoded.len());
        stored += encoded.len();
    }
    let (mut files_n, mut input) = (0, 0);
    if let Some(rev) = rev {
        let root = ikam_kernel::snapshot::tree_of(repo, rev)?;
        let mut seen = std::collections::HashSet::new();
        for f in files(repo, &root)? {
            files_n += 1;
            if seen.insert(f.id) {
                input += repo.read_content(&f.id)?.len();
            }
        }
    }
    Ok(Stats {
        kinds,
        stored,
        files: files_n,
        input,
    })
}

/// Objects by kind, contents by how they are encoded (see `dict.rs`).
fn kind_name(tag: Option<&u8>) -> String {
    let name = match tag.map(u8::to_ascii_uppercase) {
        Some(b'T') => "tree",
        Some(b'C') => "commit",
        Some(b'D') => "derivation",
        Some(b'L') => "claim",
        Some(b'G') => "graph",
        Some(b'B' | b'W') => "content: raw",
        Some(b'Z') => "content: zstd",
        Some(b'Y') => "content: zstd+dict",
        Some(b'Q' | b'M') => "content: zstd+related",
        Some(b'X') => "content: xz",
        Some(b'R') => "content: brotli",
        Some(b'P') => "content: ppmd",
        Some(b'S') => "content: in a group",
        Some(b'K') => "group (blocks)",
        Some(t) => return format!("other ({})", t as char),
        None => "empty",
    };
    name.into()
}

pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_parts_search_as_text() {
        let doc = br#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>Q3 rev</w:t></w:r><w:r><w:t>enue &amp; costs</w:t></w:r></w:p><w:p><w:t>next</w:t></w:p></w:body></w:document>"#;
        assert!(looks_like_xml(doc));
        assert_eq!(xml_text(doc), "Q3 revenue & costs\nnext\n");
        assert!(matching_lines(xml_text(doc).as_bytes(), "document").is_empty());
        assert_eq!(xml_text(b"ttr=\"x\">Hi<b>there</b>"), "Hithere\n");
        assert_eq!(
            xml_text(b"<?xml?><p>a < b</p><w:settings xmlns:r=\"x"),
            "a < b\n"
        );
        assert_eq!(matching_lines(xml_text(doc).as_bytes(), "revenue")[0].0, 1);
        assert_eq!(xml_text(b"<p>a</p>\0\0xmlns:r=\"x\""), "a\n");
    }

    #[test]
    fn pdf_streams_search_as_text() {
        let s = b"BT\n/F1 12 Tf\n(1.\\ Strip\\ the\\ cable) Tj\n0 -14 Td\n[(Hel) -20 (lo \\(world\\))] TJ\n0 -14 Td\nET\n";
        assert!(is_pdf_text(s));
        assert_eq!(pdf_text(s), "1. Strip the cable\nHello (world)\n");
        let one = b"BT /F32 20.6 Tf 89.2 690.6 Td [(1)-550(Y)92(our)-367(Chapter)]TJ 0 -36.9 Td [(Lorem)-331(ipsum)]TJ ET";
        assert_eq!(pdf_text(one), "1 Your Chapter\nLorem ipsum\n");
    }
}
