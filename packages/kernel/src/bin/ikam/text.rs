//! What `ikam live` searches and answers from: the text a reader sees in
//! each file, kept in memory and updated as files change (E110).
//!
//! A file's text is its bytes when they are text; for an office document or
//! PDF, the text of the parts the kernel stores separately (XML members,
//! content streams, decoded as a reader sees them, so words split across
//! PDF kerning pieces are found); for a tar or tar.gz archive, each text
//! member, by its path inside the archive. Search is exact and
//! case-insensitive over that text; questions rank passages with BM25.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::Arc;

use ikam_kernel::{Id, Repo, Store};

use crate::corpus::{is_pdf_text, looks_like_xml, pdf_text, xml_text};

/// Archive members above this are not indexed.
const MAX_MEMBER: usize = 8 << 20;
/// Lines per passage when answering questions.
const PASSAGE_LINES: usize = 12;

/// One searchable piece of a file: the file itself, or a member or part.
pub struct Part {
    /// `""` for the file's own text, else the member path or `part N`.
    pub name: String,
    pub text: String,
    /// `text` with ASCII letters lowercased (same byte offsets).
    pub lower: String,
    /// Byte offset of each line start.
    pub lines: Vec<u32>,
}

impl Part {
    fn new(name: String, text: String) -> Part {
        let lower = text.to_ascii_lowercase();
        let mut lines = vec![0];
        lines.extend(memchr::memchr_iter(b'\n', text.as_bytes()).map(|i| i as u32 + 1));
        Part {
            name,
            text,
            lower,
            lines,
        }
    }

    /// 1-based line number of byte `at`.
    fn line_of(&self, at: usize) -> usize {
        self.lines.partition_point(|&s| s as usize <= at)
    }

    fn line(&self, n: usize) -> &str {
        let start = self.lines[n - 1] as usize;
        let end = self
            .lines
            .get(n)
            .map_or(self.text.len(), |&e| e as usize - 1);
        self.text[start..end.max(start)].trim_end_matches('\r')
    }
}

pub struct Doc {
    pub parts: Vec<Part>,
}

impl Doc {
    pub fn bytes(&self) -> usize {
        self.parts.iter().map(|p| p.text.len()).sum()
    }
}

/// The text of a file with content `id` at `path` whose bytes are `bytes`.
pub fn extract<S: Store>(repo: &Repo<S>, id: &Id, path: &str, bytes: &[u8]) -> Doc {
    let lower = path.to_ascii_lowercase();
    let mut parts = Vec::new();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        let mut tar = Vec::new();
        if flate2::read::MultiGzDecoder::new(bytes)
            .read_to_end(&mut tar)
            .is_ok()
        {
            parts = tar_members(&tar);
        }
    } else if lower.ends_with(".tar") {
        parts = tar_members(bytes);
    }
    if parts.is_empty() {
        if let Some(text) = as_text(bytes) {
            parts.push(Part::new(String::new(), text));
        } else {
            parts = stored_parts(repo, id);
        }
    }
    Doc { parts }
}

/// Bytes as text if they look like text (UTF-8, no NUL in the first 8 KiB).
fn as_text(bytes: &[u8]) -> Option<String> {
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Some(s.to_string()),
        // A few bad bytes (Latin-1 in a CSV) still read as text.
        Err(e) if e.valid_up_to() * 10 >= bytes.len() * 9 => {
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
        Err(_) => None,
    }
}

/// The text of the parts the kernel derives a binary file from.
fn stored_parts<S: Store>(repo: &Repo<S>, id: &Id) -> Vec<Part> {
    let mut out = Vec::new();
    let Ok(ds) = repo.derivations(id) else {
        return out;
    };
    let mut seen = HashSet::new();
    let mut joined = String::new();
    for d in ds.iter().take(1) {
        for a in &d.args {
            if !seen.insert(a.id()) {
                continue;
            }
            let Ok(bytes) = repo.read_content(&a.id()) else {
                continue;
            };
            let text = if looks_like_xml(&bytes) {
                xml_text(&bytes)
            } else if is_pdf_text(&bytes) {
                pdf_text(&bytes)
            } else if let Some(t) = as_text(&bytes) {
                t
            } else {
                continue;
            };
            let text = text.trim();
            if !text.is_empty() {
                joined.push_str(text);
                joined.push_str("\n\n");
            }
        }
    }
    if !joined.is_empty() {
        out.push(Part::new(String::new(), joined));
    }
    out
}

/// Text members of a tar archive (ustar, GNU long names, pax paths).
fn tar_members(tar: &[u8]) -> Vec<Part> {
    let mut out = Vec::new();
    let (mut at, mut long_name): (usize, Option<String>) = (0, None);
    while at + 512 <= tar.len() {
        let h = &tar[at..at + 512];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let field = |r: std::ops::Range<usize>| {
            let f = &h[r];
            let end = f.iter().position(|&b| b == 0).unwrap_or(f.len());
            String::from_utf8_lossy(&f[..end]).into_owned()
        };
        let Ok(size) = usize::from_str_radix(field(124..136).trim(), 8) else {
            break;
        };
        let kind = h[156];
        let data_start = at + 512;
        let data = &tar[data_start..(data_start + size).min(tar.len())];
        at = data_start + size.div_ceil(512) * 512;
        match kind {
            b'L' => {
                long_name = Some(String::from_utf8_lossy(data).trim_end_matches('\0').into());
                continue;
            }
            b'x' => {
                let s = String::from_utf8_lossy(data);
                long_name = s.lines().find_map(|l| {
                    let (_, kv) = l.split_once(' ')?;
                    kv.strip_prefix("path=").map(String::from)
                });
                continue;
            }
            b'0' | 0 => {}
            _ => {
                long_name = None;
                continue;
            }
        }
        let name = long_name.take().unwrap_or_else(|| {
            let (prefix, name) = (field(345..500), field(0..100));
            if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            }
        });
        if size > MAX_MEMBER {
            continue;
        }
        if let Some(text) = as_text(data)
            && !text.trim().is_empty()
        {
            out.push(Part::new(name, text));
        }
    }
    out
}

/// A search hit: a file (and member), with matching lines.
pub struct Hit {
    pub path: String,
    pub part: String,
    pub lines: Vec<(usize, String)>,
}

const MAX_LINES: usize = 5;

/// Files whose text contains `query` (case-insensitive), with up to five
/// lines each, at most `limit` hits.
pub fn search(docs: &HashMap<String, Arc<Doc>>, query: &str, limit: usize) -> (Vec<Hit>, usize) {
    let needle = query.to_ascii_lowercase();
    let finder = memchr::memmem::Finder::new(needle.as_bytes());
    let mut paths: Vec<&String> = docs.keys().collect();
    paths.sort();
    let (mut hits, mut total) = (Vec::new(), 0);
    if needle.is_empty() {
        return (hits, 0);
    }
    for path in paths {
        for part in &docs[path].parts {
            let mut it = finder.find_iter(part.lower.as_bytes()).peekable();
            if it.peek().is_none() {
                continue;
            }
            total += 1;
            if hits.len() >= limit {
                continue;
            }
            let mut lines = Vec::new();
            let mut last = 0;
            for at in it {
                let n = part.line_of(at);
                if n != last {
                    lines.push((n, snippet(part.line(n), &needle)));
                    last = n;
                }
                if lines.len() >= MAX_LINES {
                    break;
                }
            }
            hits.push(Hit {
                path: path.clone(),
                part: part.name.clone(),
                lines,
            });
        }
    }
    (hits, total)
}

/// About 160 characters of `line` around the first match.
fn snippet(line: &str, needle: &str) -> String {
    let at = line.to_ascii_lowercase().find(needle).unwrap_or(0);
    let start = line[..at]
        .char_indices()
        .rev()
        .nth(70)
        .map_or(0, |(i, _)| i);
    line[start..].chars().take(160).collect()
}

/// A passage that answers a question: where it is and its lines.
pub struct Passage {
    pub path: String,
    pub part: String,
    pub first_line: usize,
    pub text: String,
    pub score: f64,
}

const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "has",
    "have", "how", "i", "in", "is", "it", "its", "me", "my", "of", "on", "or", "that", "the",
    "there", "this", "to", "was", "what", "when", "where", "which", "who", "why", "with", "you",
    "about", "any", "all", "we", "our", "us", "they", "them", "their", "list", "show", "tell",
    "find", "file", "files", "many", "much", "did",
];

pub fn terms(q: &str) -> Vec<String> {
    let mut out: Vec<String> = q
        .to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 2 && !STOP.contains(t))
        .map(String::from)
        .collect();
    out.dedup();
    out
}

/// The `k` passages that best match the question's terms (BM25 over
/// passages of `PASSAGE_LINES` lines; a term matches a word that starts
/// with it). A passage from a file whose path has a term scores a little
/// higher.
pub fn retrieve(docs: &HashMap<String, Arc<Doc>>, question: &str, k: usize) -> Vec<Passage> {
    let terms = terms(question);
    if terms.is_empty() {
        return Vec::new();
    }
    let finders: Vec<_> = terms
        .iter()
        .map(|t| memchr::memmem::Finder::new(t.as_bytes()))
        .collect();
    // (path, part index, passage index) -> term frequencies.
    let mut tf: HashMap<(&String, usize, usize), Vec<u32>> = HashMap::new();
    let mut df = vec![0usize; terms.len()];
    let mut n_passages = 0usize;
    for (path, doc) in docs {
        for (pi, part) in doc.parts.iter().enumerate() {
            n_passages += part.lines.len().div_ceil(PASSAGE_LINES);
            let hay = part.lower.as_bytes();
            for (ti, f) in finders.iter().enumerate() {
                let mut last_passage = usize::MAX;
                for at in f.find_iter(hay) {
                    if at > 0 && (hay[at - 1].is_ascii_alphanumeric() || hay[at - 1] == b'_') {
                        continue;
                    }
                    let p = (part.line_of(at) - 1) / PASSAGE_LINES;
                    let e = tf
                        .entry((path, pi, p))
                        .or_insert_with(|| vec![0; terms.len()]);
                    e[ti] += 1;
                    if p != last_passage {
                        df[ti] += 1;
                        last_passage = p;
                    }
                }
            }
        }
    }
    let n = n_passages.max(1) as f64;
    let idf: Vec<f64> = df
        .iter()
        .map(|&d| ((n - d as f64 + 0.5) / (d as f64 + 0.5) + 1.0).ln())
        .collect();
    let k1 = 1.2;
    let mut scored: Vec<((&String, usize, usize), f64)> = tf
        .into_iter()
        .map(|(key, f)| {
            let path_lower = key.0.to_ascii_lowercase();
            let mut s: f64 = f
                .iter()
                .zip(&idf)
                .map(|(&c, w)| w * (c as f64 * (k1 + 1.0)) / (c as f64 + k1))
                .sum();
            let in_path = terms
                .iter()
                .filter(|t| path_lower.contains(t.as_str()))
                .count();
            s *= 1.0 + 0.2 * in_path as f64;
            (key, s)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored
        .into_iter()
        .take(k)
        .map(|((path, pi, p), score)| {
            let part = &docs[path].parts[pi];
            let first = p * PASSAGE_LINES + 1;
            let last = (first + PASSAGE_LINES - 1).min(part.lines.len());
            let text: Vec<&str> = (first..=last).map(|l| part.line(l)).collect();
            let mut text = text.join("\n");
            if text.len() > 2000 {
                let mut end = 2000;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
            Passage {
                path: path.clone(),
                part: part.name.clone(),
                first_line: first,
                text,
                score,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docs(files: &[(&str, &str)]) -> HashMap<String, Arc<Doc>> {
        files
            .iter()
            .map(|(p, t)| {
                (
                    p.to_string(),
                    Arc::new(Doc {
                        parts: vec![Part::new(String::new(), t.to_string())],
                    }),
                )
            })
            .collect()
    }

    #[test]
    fn search_is_case_insensitive_with_lines() {
        let d = docs(&[
            ("a.txt", "one\nThe Quick fox\nthree quick"),
            ("b.txt", "none"),
        ]);
        let (hits, total) = search(&d, "quick", 10);
        assert_eq!(total, 1);
        assert_eq!(hits[0].path, "a.txt");
        assert_eq!(
            hits[0].lines,
            vec![(2, "The Quick fox".into()), (3, "three quick".into())]
        );
    }

    #[test]
    fn retrieval_ranks_the_passage_with_the_terms() {
        let filler = "lorem ipsum\n".repeat(30);
        let d = docs(&[
            (
                "notes.md",
                &format!("{filler}the invoice total was 42 dollars\n{filler}"),
            ),
            ("other.md", &format!("{filler}a total of nothing\n")),
        ]);
        let p = retrieve(&d, "What was the invoice total?", 2);
        assert_eq!(p[0].path, "notes.md");
        assert!(p[0].text.contains("invoice total"));
    }

    #[test]
    fn tar_members_are_read_by_path() {
        let mut tar = Vec::new();
        let mut header = [0u8; 512];
        header[..9].copy_from_slice(b"pkg/a.txt");
        header[124..135].copy_from_slice(b"00000000005");
        header[156] = b'0';
        tar.extend_from_slice(&header);
        let mut data = [0u8; 512];
        data[..5].copy_from_slice(b"hello");
        tar.extend_from_slice(&data);
        tar.extend_from_slice(&[0u8; 1024]);
        let parts = tar_members(&tar);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            (parts[0].name.as_str(), parts[0].text.as_str()),
            ("pkg/a.txt", "hello")
        );
    }
}
