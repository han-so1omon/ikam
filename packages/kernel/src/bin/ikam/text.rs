//! What `ikam live` searches and answers from: the text a reader sees in
//! each file, kept in memory and updated as files change (E110).
//!
//! A file's text is its bytes when they are text; for an office document or
//! PDF, the text of the parts the kernel stores separately (XML members,
//! content streams, decoded as a reader sees them, so words split across
//! PDF kerning pieces are found); for a tar or tar.gz archive, each text
//! member, by its path inside the archive. Search is exact and
//! case-insensitive over that text; questions rank passages with BM25,
//! by a scan of every file's text or, with `--passage-index` (E378, off by
//! default), from an index of words kept as files change, which gives the
//! same passages and scores.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::Arc;

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
pub fn extract(path: &str, bytes: &[u8]) -> Doc {
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
            parts = binary_parts(bytes);
        }
    }
    Doc { parts }
}

/// Text a person could read: almost no control characters or invalid
/// bytes. Image data inflated from a PDF can hold `BT` and `Tj` by chance,
/// and its "strings" are noise.
fn readable(text: &str) -> bool {
    let (mut n, mut bad) = (0usize, 0usize);
    for c in text.chars() {
        n += 1;
        if c == '\u{fffd}' || (c.is_control() && !c.is_whitespace()) {
            bad += 1;
        }
    }
    bad * 50 <= n
}

/// Bytes as text if they look like text (no NUL, at least 90% UTF-8).
fn as_text(bytes: &[u8]) -> Option<String> {
    if memchr::memchr(0, bytes).is_some() {
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

/// The text in a binary file's parts, as ingest expands them from its own
/// bytes (a zip's members, a PDF's streams and decoded text layers). Not
/// from the stored derivation: after a repack that is slices of other
/// files, not members.
fn binary_parts(bytes: &[u8]) -> Vec<Part> {
    let mut joined = String::new();
    for m in ikam_kernel::expand(bytes) {
        let text = if looks_like_xml(&m) {
            xml_text(&m)
        } else if is_pdf_text(&m) {
            pdf_text(&m)
        } else if !m.contains(&0)
            && let Ok(t) = std::str::from_utf8(&m)
        {
            t.to_string()
        } else {
            continue;
        };
        let text = text.trim();
        if !text.is_empty() && readable(text) {
            joined.push_str(text);
            joined.push_str("\n\n");
        }
    }
    if joined.is_empty() {
        Vec::new()
    } else {
        vec![Part::new(String::new(), joined)]
    }
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
    rank(docs, &terms, tf, &df, n_passages, k)
}

/// Score passages by BM25 from their term counts `tf`, each term's passage
/// count `df` and the number of passages, and return the best `k`
/// (shared by the scan and the index, so both rank alike).
fn rank(
    docs: &HashMap<String, Arc<Doc>>,
    terms: &[String],
    tf: HashMap<(&String, usize, usize), Vec<u32>>,
    df: &[usize],
    n_passages: usize,
    k: usize,
) -> Vec<Passage> {
    let n = n_passages.max(1) as f64;
    let idf: Vec<f64> = df
        .iter()
        .map(|&d| ((n - d as f64 + 0.5) / (d as f64 + 0.5) + 1.0).ln())
        .collect();
    let k1 = 1.2;
    // A path's boost, worked out once per path rather than per passage.
    let mut boost: HashMap<&String, f64> = HashMap::new();
    let mut scored: Vec<((&String, usize, usize), f64)> = tf
        .into_iter()
        .map(|(key, f)| {
            let mut s: f64 = f
                .iter()
                .zip(&idf)
                .map(|(&c, w)| w * (c as f64 * (k1 + 1.0)) / (c as f64 + k1))
                .sum();
            s *= *boost.entry(key.0).or_insert_with(|| {
                let path_lower = key.0.to_ascii_lowercase();
                let in_path = terms
                    .iter()
                    .filter(|t| path_lower.contains(t.as_str()))
                    .count();
                1.0 + 0.2 * in_path as f64
            });
            (key, s)
        })
        .collect();
    let order = |a: &((&String, usize, usize), f64), b: &((&String, usize, usize), f64)| {
        b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0))
    };
    // Only the best `k` are put in order (the same `k` a full sort gives).
    if k == 0 {
        return Vec::new();
    }
    if scored.len() > k {
        scored.select_nth_unstable_by(k - 1, order);
        scored.truncate(k);
    }
    scored.sort_by(order);
    scored
        .into_iter()
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

/// E378: the words of every passage, kept as files change, so a question
/// reads the postings of its terms instead of scanning all text. It answers
/// as `retrieve` does: a word here is a run of ASCII letters, digits and
/// `_` in the lowercased text (a term matches a word it begins, as the
/// scan's word-start match), postings count a passage's words per word, and
/// scoring is the same `rank`. A question with a term that is not ASCII is
/// answered by the scan.
#[derive(Default)]
pub struct PassageIndex {
    /// Word -> its number in `postings`.
    words: BTreeMap<String, u32>,
    /// By word number: (file slot, part, passage, times in it) for each
    /// passage holding the word.
    postings: Vec<Vec<[u32; 4]>>,
    /// Path -> its slot, the numbers of the words it holds and its passage
    /// count.
    files: HashMap<String, (u32, Vec<u32>, usize)>,
    /// Slot -> path (freed slots are `None` until reused).
    slots: Vec<Option<String>>,
    free: Vec<u32>,
    passages: usize,
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'
}

fn doc_passages(doc: &Doc) -> usize {
    doc.parts
        .iter()
        .map(|p| p.lines.len().div_ceil(PASSAGE_LINES))
        .sum()
}

impl PassageIndex {
    /// An index of `docs`.
    pub fn build(docs: &HashMap<String, Arc<Doc>>) -> PassageIndex {
        let mut idx = PassageIndex::default();
        for (path, doc) in docs {
            idx.insert(path, doc);
        }
        for p in &mut idx.postings {
            p.shrink_to_fit();
        }
        idx
    }

    /// Index `doc` as `path`, in place of what `path` held.
    pub fn insert(&mut self, path: &str, doc: &Doc) {
        self.remove(path);
        let slot = self.free.pop().unwrap_or_else(|| {
            self.slots.push(None);
            self.slots.len() as u32 - 1
        });
        self.slots[slot as usize] = Some(path.to_string());
        let mut counts: HashMap<(&str, u32, u32), u32> = HashMap::new();
        for (pi, part) in doc.parts.iter().enumerate() {
            let b = part.lower.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if !is_word_byte(b[i]) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < b.len() && is_word_byte(b[i]) {
                    i += 1;
                }
                if i - start >= 2 {
                    let p = ((part.line_of(start) - 1) / PASSAGE_LINES) as u32;
                    *counts
                        .entry((&part.lower[start..i], pi as u32, p))
                        .or_default() += 1;
                }
            }
        }
        let mut held = Vec::with_capacity(counts.len());
        for ((word, part, passage), n) in counts {
            let w = match self.words.get(word) {
                Some(&w) => w,
                None => {
                    let w = self.postings.len() as u32;
                    self.postings.push(Vec::new());
                    self.words.insert(word.to_string(), w);
                    w
                }
            };
            self.postings[w as usize].push([slot, part, passage, n]);
            held.push(w);
        }
        held.sort_unstable();
        held.dedup();
        held.shrink_to_fit();
        let passages = doc_passages(doc);
        self.passages += passages;
        self.files.insert(path.to_string(), (slot, held, passages));
    }

    /// Drop `path`'s postings.
    pub fn remove(&mut self, path: &str) {
        let Some((slot, held, passages)) = self.files.remove(path) else {
            return;
        };
        self.passages -= passages;
        // A word no file holds any more keeps its (empty) number.
        for w in held {
            let p = &mut self.postings[w as usize];
            p.retain(|q| q[0] != slot);
            if p.capacity() > 2 * p.len() + 4 {
                p.shrink_to_fit();
            }
        }
        self.slots[slot as usize] = None;
        self.free.push(slot);
    }

    /// Files indexed, distinct words and postings (for logs and benches).
    pub fn size(&self) -> (usize, usize, usize) {
        let postings = self.postings.iter().map(Vec::len).sum();
        let words = self.postings.iter().filter(|p| !p.is_empty()).count();
        (self.files.len(), words, postings)
    }
}

/// `retrieve` answered from `idx`, an index of `docs`; `None` when a term
/// is not ASCII (the caller scans).
pub fn retrieve_indexed(
    idx: &PassageIndex,
    docs: &HashMap<String, Arc<Doc>>,
    question: &str,
    k: usize,
) -> Option<Vec<Passage>> {
    let terms = terms(question);
    if terms.is_empty() {
        return Some(Vec::new());
    }
    if !terms.iter().all(|t| t.bytes().all(is_word_byte)) {
        return None;
    }
    let mut tf: HashMap<(&String, usize, usize), Vec<u32>> = HashMap::new();
    let mut df = vec![0usize; terms.len()];
    for (ti, t) in terms.iter().enumerate() {
        let mut hits: HashMap<[u32; 3], u32> = HashMap::new();
        let from = (
            std::ops::Bound::Included(t.as_str()),
            std::ops::Bound::Unbounded,
        );
        for (_, &w) in idx
            .words
            .range::<str, _>(from)
            .take_while(|(w, _)| w.starts_with(t.as_str()))
        {
            for q in &idx.postings[w as usize] {
                *hits.entry([q[0], q[1], q[2]]).or_default() += q[3];
            }
        }
        df[ti] = hits.len();
        for ([slot, part, passage], n) in hits {
            let path = idx.slots[slot as usize].as_ref()?;
            let (path, _) = docs.get_key_value(path)?;
            tf.entry((path, part as usize, passage as usize))
                .or_insert_with(|| vec![0; terms.len()])[ti] = n;
        }
    }
    Some(rank(docs, &terms, tf, &df, idx.passages, k))
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

    fn same(a: &[Passage], b: &[Passage]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|(x, y)| {
                x.path == y.path
                    && x.part == y.part
                    && x.first_line == y.first_line
                    && x.text == y.text
                    && x.score == y.score
            })
    }

    #[test]
    fn the_passage_index_answers_as_the_scan() {
        // Random files over a small vocabulary, then edits and removals;
        // every question must get the same passages and scores both ways.
        let vocab = [
            "alpha",
            "alphabet",
            "Beta",
            "beta_max",
            "gamma",
            "gam",
            "delta9",
            "x",
            "Ünïcode",
            "café",
            "zeta-ray",
            "eta.eta",
            "th",
            "the",
            "quota",
            "QUOTE",
        ];
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut rnd = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let file = |rnd: &mut dyn FnMut(usize) -> usize| {
            let mut t = String::new();
            for _ in 0..rnd(200) {
                t.push_str(vocab[rnd(vocab.len())]);
                t.push_str([" ", "\n", ", ", "\n\n", "_", "("][rnd(6)]);
            }
            Arc::new(Doc {
                parts: vec![
                    Part::new(String::new(), t.clone()),
                    Part::new("part 2".into(), t.chars().rev().collect()),
                ],
            })
        };
        let questions = [
            "alpha",
            "alp beta",
            "gamma delta9",
            "quot",
            "café",
            "unicode",
            "zeta ray",
            "eta",
            "th",
            "beta_m",
            "x",
            "nothing here",
            "alpha alpha gamma",
            "the",
        ];
        let mut d: HashMap<String, Arc<Doc>> = HashMap::new();
        for i in 0..30 {
            d.insert(format!("dir{}/f{i}.txt", i % 3), file(&mut rnd));
        }
        let mut idx = PassageIndex::build(&d);
        for round in 0..20 {
            for q in questions {
                for k in [1, 5, 50] {
                    let scan = retrieve(&d, q, k);
                    match retrieve_indexed(&idx, &d, q, k) {
                        Some(got) => assert!(same(&scan, &got), "round {round}, {q:?}, k {k}"),
                        None => assert!(q.bytes().any(|b| !b.is_ascii())),
                    }
                }
            }
            for _ in 0..5 {
                let path = format!("dir{}/f{}.txt", rnd(3), rnd(40));
                if rnd(3) == 0 {
                    d.remove(&path);
                    idx.remove(&path);
                } else {
                    let doc = file(&mut rnd);
                    idx.insert(&path, &doc);
                    d.insert(path, doc);
                }
            }
        }
        assert_eq!(idx.size().0, d.len());
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
