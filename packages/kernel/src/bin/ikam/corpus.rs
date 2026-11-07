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
    for (k, f) in all.iter().enumerate() {
        owners.entry(f.id).or_default().push(k);
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
        let text = matching_lines(&repo.read_content(&id)?, pattern);
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
