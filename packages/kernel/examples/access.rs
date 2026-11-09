//! E124 to E127: how much a linked store must touch to serve one request.
//!
//! `cargo run --release --example access -- SRC REF DIR`
//!
//! SRC is an FsStore the CLI ingested from DIR under REF (the bench's fixture
//! cases). It is loaded into memory behind a counting store, so every number
//! below is store reads and bytes fetched: the I/O, and on an object store
//! the GETs, each request costs. Nothing is written to SRC.
//!
//! - E124 sections: a whole file, a 4 KiB range and one markdown section.
//! - E125 queries: path lookup (tree walk vs a path index), key lookup and
//!   range on a database view, outgoing links, and incoming links (a full scan
//!   vs a reverse-link view).
//! - E126 permissions: a capability's MAC check, scoped read checks (Merkle
//!   path vs a precomputed reachable set), which stored objects reading a
//!   tenant's files actually touches outside that tenant's tree, the
//!   deduplication tenants share, and a scoped edit.
//! - E127 goal-scoped working sets: an agent with a goal (a search term)
//!   reads every file, the matching files, or windows around each match.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use ikam_kernel::{
    Arg, Edit, Error, Event, FsStore, Id, Keys, Kind, MemStore, Node, Object, Repo, Store, Target,
};

/// A store that counts reads, bytes fetched and writes, and can record ids read.
struct Counting<S: Store> {
    inner: S,
    reads: AtomicU64,
    bytes: AtomicU64,
    writes: AtomicU64,
    wbytes: AtomicU64,
    scans: AtomicU64,
    seen: Mutex<Option<HashSet<Id>>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Io {
    reads: u64,
    bytes: u64,
    writes: u64,
    wbytes: u64,
    scans: u64,
}

impl<S: Store> Counting<S> {
    fn new(inner: S) -> Self {
        Counting {
            inner,
            reads: 0.into(),
            bytes: 0.into(),
            writes: 0.into(),
            wbytes: 0.into(),
            scans: 0.into(),
            seen: Mutex::new(None),
        }
    }
    fn take(&self) -> Io {
        Io {
            reads: self.reads.swap(0, Relaxed),
            bytes: self.bytes.swap(0, Relaxed),
            writes: self.writes.swap(0, Relaxed),
            wbytes: self.wbytes.swap(0, Relaxed),
            scans: self.scans.swap(0, Relaxed),
        }
    }
    fn record(&self, on: bool) -> Option<HashSet<Id>> {
        let mut s = self.seen.lock().unwrap();
        std::mem::replace(&mut *s, on.then(HashSet::new))
    }
}

impl<S: Store> Store for Counting<S> {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        let new = self.inner.write(id, encoded)?;
        if new {
            self.writes.fetch_add(1, Relaxed);
            self.wbytes.fetch_add(encoded.len() as u64, Relaxed);
        }
        Ok(new)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        let out = self.inner.read(id)?;
        self.reads.fetch_add(1, Relaxed);
        self.bytes.fetch_add(out.len() as u64, Relaxed);
        if let Some(s) = self.seen.lock().unwrap().as_mut() {
            s.insert(*id);
        }
        Ok(out)
    }
    fn has(&self, id: &Id) -> bool {
        self.inner.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.scans.fetch_add(1, Relaxed);
        self.inner.ids()
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        self.inner.ids_with_prefix(prefix)
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        self.inner.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.inner.get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        self.inner.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.inner.refs()
    }
}

type R = Repo<Counting<MemStore>>;

fn load(src: &Path) -> MemStore {
    let fs = FsStore::open(src).unwrap();
    let mut mem = MemStore::default();
    for id in fs.ids().unwrap() {
        mem.write(id, &fs.read(&id).unwrap()).unwrap();
    }
    for (name, id) in fs.refs().unwrap() {
        mem.set_ref(&name, None, id).unwrap();
    }
    mem
}

/// Every file under `tree`: (path, id, depth), and every tree with its path.
fn walk(repo: &R, tree: Id, prefix: &str, depth: usize, files: &mut Vec<(String, Id, usize)>, trees: &mut Vec<(String, Id)>) {
    trees.push((prefix.to_string(), tree));
    let Object::Tree(entries) = repo.get(&tree).unwrap() else { panic!("not a tree") };
    for e in entries {
        let path = if prefix.is_empty() { e.name.clone() } else { format!("{prefix}/{}", e.name) };
        match e.kind {
            Kind::Tree => walk(repo, e.id, &path, depth + 1, files, trees),
            Kind::File => files.push((path, e.id, depth + 1)),
            Kind::Graph => {}
        }
    }
}

fn ext(path: &str) -> String {
    Path::new(path).extension().map_or("none".into(), |e| e.to_string_lossy().into_owned())
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Byte range of the middle `## ` section of a markdown file.
fn md_section(bytes: &[u8]) -> Option<(u64, u64)> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut starts: Vec<usize> = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with("## ") {
            starts.push(at);
        }
        at += line.len();
    }
    if starts.is_empty() {
        return None;
    }
    let i = starts.len() / 2;
    let end = starts.get(i + 1).copied().unwrap_or(text.len());
    Some((starts[i] as u64, (end - starts[i]) as u64))
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (src, refname, dir) = (PathBuf::from(&a[0]), a[1].clone(), PathBuf::from(&a[2]));
    let t = Instant::now();
    let mut repo: R = Repo::new(Counting::new(load(&src)));
    eprintln!("loaded in {:.1} s", t.elapsed().as_secs_f64());
    let store_objects = repo.store().ids().unwrap().len();
    let store_bytes: u64 = repo.store().ids().unwrap().iter().map(|id| repo.store().inner.read(id).unwrap().len() as u64).sum();
    repo.store().take();

    let commit = repo.resolve(&refname).unwrap();
    let Object::Commit(c) = repo.get(&commit).unwrap() else { panic!() };
    let (mut files, mut trees) = (Vec::new(), Vec::new());
    walk(&repo, c.tree, "", 0, &mut files, &mut trees);
    repo.store().take();

    // Cold handle: the first content read on a fresh Repo.
    let fresh: R = Repo::new(Counting::new(load(&src)));
    let t = Instant::now();
    let _ = fresh.read_content(&files[0].1).unwrap();
    let cold = fresh.store().take();
    println!(
        "{{\"exp\":\"E124\",\"what\":\"cold_handle\",\"store_objects\":{store_objects},\"store_bytes\":{store_bytes},\"files\":{},\"trees\":{},\"reads\":{},\"bytes\":{},\"scans\":{},\"ms\":{:.1}}}",
        files.len(),
        trees.len(),
        cold.reads,
        cold.bytes,
        cold.scans,
        t.elapsed().as_secs_f64() * 1e3
    );
    drop(fresh);
    let _ = repo.read_content(&files[0].1).unwrap();
    repo.store().take();

    anatomy(&repo, &files);
    e124(&repo, &files, &dir);
    e125(&mut repo, &files, c.tree);
    e126(&mut repo, &files, &trees, c.tree, commit, &refname);
    e127(&mut repo, &files);
}

/// What one read fetches: for a few files of each family, every stored
/// object read, its size, its encoding tag, and which file trees hold it.
fn anatomy(repo: &R, files: &[(String, Id, usize)]) {
    let mut owner: HashMap<Id, Vec<&str>> = HashMap::new();
    for (p, id, _) in files {
        owner.entry(*id).or_default().push(p);
    }
    let mut done: HashMap<String, usize> = HashMap::new();
    for (path, id, _) in files {
        let n = done.entry(ext(path)).or_default();
        if *n >= 2 {
            continue;
        }
        *n += 1;
        repo.store().record(true);
        let bytes = repo.read_content(id).unwrap();
        let seen = repo.store().record(false).unwrap();
        repo.store().take();
        let mut parts = Vec::new();
        for s in &seen {
            let enc = repo.store().inner.read(s).unwrap();
            let tag = enc.first().map_or('?', |b| *b as char);
            let other: Vec<&str> = owner.get(s).map_or(vec![], |v| v.iter().filter(|o| **o != path).copied().take(1).collect());
            parts.push(format!(
                "{{\"id\":\"{}\",\"bytes\":{},\"tag\":\"{}\",\"self\":{},\"is_file_of\":\"{}\"}}",
                &s.to_string()[..12],
                enc.len(),
                tag.escape_default(),
                s == id,
                other.first().copied().unwrap_or("")
            ));
        }
        println!(
            "{{\"exp\":\"E124\",\"what\":\"anatomy\",\"path\":\"{path}\",\"bytes\":{},\"reads\":[{}]}}",
            bytes.len(),
            parts.join(",")
        );
    }
}

fn e124(repo: &R, files: &[(String, Id, usize)], dir: &Path) {
    // Per family: whole reads, 4 KiB ranges, markdown sections.
    let mut by: BTreeMap<String, (u64, Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, u64, u64)> = BTreeMap::new();
    let (mut sec_n, mut sec_bytes, mut sec_fetched, mut sec_reads, mut sec_whole_fetched) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for (path, id, _) in files {
        let bytes = repo.read_content(id).unwrap();
        let whole = repo.store().take();
        let e = by.entry(ext(path)).or_default();
        e.0 += 1;
        e.1.push(whole.reads as f64);
        e.2.push(whole.bytes as f64 / bytes.len().max(1) as f64);
        e.5 += bytes.len() as u64;
        e.6 += whole.bytes;
        if bytes.len() > 8192 {
            let start = (bytes.len() / 2) as u64;
            let (part, _) = repo.read_range(id, start, 4096).unwrap();
            assert_eq!(part, bytes[start as usize..start as usize + 4096]);
            let r = repo.store().take();
            e.3.push(r.reads as f64);
            e.4.push(r.bytes as f64 / 4096.0);
        }
        if ext(path) == "md"
            && let Some((s, l)) = md_section(&bytes)
        {
            // The section's offsets come from the source file, as a section
            // index (heading -> byte range) would give them.
            let on_disk = std::fs::read(dir.join(path)).unwrap();
            assert_eq!(on_disk, bytes, "{path}: stored bytes differ from the source");
            let (part, _) = repo.read_range(id, s, l).unwrap();
            assert_eq!(part, bytes[s as usize..(s + l) as usize]);
            let r = repo.store().take();
            sec_n += 1;
            sec_bytes += l;
            sec_fetched += r.bytes;
            sec_reads += r.reads;
            sec_whole_fetched += whole.bytes;
        }
    }
    for (family, (n, mut reads, mut amp, mut rreads, mut ramp, bytes, fetched)) in by {
        println!(
            "{{\"exp\":\"E124\",\"what\":\"family\",\"family\":\"{family}\",\"files\":{n},\"bytes\":{bytes},\"fetched\":{fetched},\"whole_reads_median\":{},\"whole_fetch_per_byte_median\":{:.3},\"range4k_n\":{},\"range4k_reads_median\":{},\"range4k_fetch_per_byte_median\":{:.2}}}",
            median(&mut reads),
            median(&mut amp),
            ramp.len(),
            median(&mut rreads),
            median(&mut ramp),
        );
    }
    // Office files: a byte range of the zip needs the whole zip re-deflated,
    // but each part (word/document.xml, a slide, a sheet) is stored as its
    // own content: reading one part by its id.
    let (mut parts_n, mut whole_fetch, mut part_fetch, mut part_bytes, mut whole_bytes) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for (path, id, _) in files {
        if !matches!(ext(path).as_str(), "docx" | "pptx" | "xlsx") {
            continue;
        }
        let bytes = repo.read_content(id).unwrap();
        let w = repo.store().take();
        let mut args = Vec::new();
        for (_, d) in repo.links(id).unwrap() {
            if let Ok(Object::Derivation(d)) = repo.get(&d) {
                args.extend(d.args.iter().map(|a| a.id()));
            }
        }
        repo.store().take();
        // The largest part, as an agent after the document's text would read it.
        let mut best: Option<(usize, u64)> = None;
        for a in &args {
            if let Ok(b) = repo.read_content(a) {
                let f = repo.store().take().bytes;
                if best.is_none_or(|(n, _)| b.len() > n) {
                    best = Some((b.len(), f));
                }
            }
        }
        repo.store().take();
        if let Some((n, f)) = best {
            parts_n += 1;
            whole_fetch += w.bytes;
            whole_bytes += bytes.len() as u64;
            part_fetch += f;
            part_bytes += n as u64;
        }
    }
    println!(
        "{{\"exp\":\"E124\",\"what\":\"office_part\",\"files\":{parts_n},\"whole_bytes\":{whole_bytes},\"whole_fetched\":{whole_fetch},\"largest_part_bytes\":{part_bytes},\"largest_part_fetched\":{part_fetch}}}"
    );
    println!(
        "{{\"exp\":\"E124\",\"what\":\"md_section\",\"sections\":{sec_n},\"section_bytes\":{sec_bytes},\"fetched\":{sec_fetched},\"reads\":{sec_reads},\"whole_file_fetched\":{sec_whole_fetched}}}"
    );
}

fn e125(repo: &mut R, files: &[(String, Id, usize)], root: Id) {
    // Path lookup by walking trees from the root.
    let mut walk_reads = Vec::new();
    for (path, id, _) in files {
        repo.store().take();
        let mut tree = root;
        let parts: Vec<&str> = path.split('/').collect();
        for (i, p) in parts.iter().enumerate() {
            let Object::Tree(entries) = repo.get(&tree).unwrap() else { panic!() };
            let e = entries.iter().find(|e| e.name == *p).unwrap();
            if i + 1 == parts.len() {
                assert_eq!(e.id, *id);
            } else {
                tree = e.id;
            }
        }
        walk_reads.push(repo.store().take().reads as f64);
    }
    // A path index: one graph, label = full path.
    let t = Instant::now();
    let nodes: Vec<Node> = files
        .iter()
        .map(|(p, id, _)| Node { label: p.clone(), target: Target::Content(Arg::Whole(*id)), edges: vec![] })
        .collect();
    let index = repo.put_graph_with(nodes, Keys::Any).unwrap();
    let build = repo.store().take();
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let mut index_reads = Vec::new();
    for (path, id, _) in files {
        let n = repo.graph_node(&index, path).unwrap().unwrap();
        assert_eq!(n.target, Target::Content(Arg::Whole(*id)));
        index_reads.push(repo.store().take().reads as f64);
    }
    let depth_max = files.iter().map(|f| f.2).max().unwrap();
    println!(
        "{{\"exp\":\"E125\",\"what\":\"path\",\"lookups\":{},\"depth_max\":{depth_max},\"tree_walk_reads_median\":{},\"tree_walk_reads_max\":{},\"path_index_reads_median\":{},\"path_index_reads_max\":{},\"path_index_objects\":{},\"path_index_bytes\":{},\"path_index_build_ms\":{build_ms:.1}}}",
        files.len(),
        median(&mut walk_reads.clone()),
        walk_reads.iter().cloned().fold(0.0, f64::max),
        median(&mut index_reads.clone()),
        index_reads.iter().cloned().fold(0.0, f64::max),
        build.writes,
        build.wbytes,
    );

    // Key lookup and range on a database view of 100,000 keys.
    let empty = repo.db_empty().unwrap();
    let mut d = empty;
    for b in 0..10u32 {
        let ev: Vec<Event> = (0..10_000u32)
            .map(|i| {
                let k = b * 10_000 + i;
                Event::upsert(&format!("user/{:06}/profile", k), format!("value {k}").as_bytes(), &format!("e{k}"))
            })
            .collect();
        d = repo.db_append(&d, &ev).unwrap();
    }
    repo.store().take();
    let mut get_reads = Vec::new();
    for k in (0..100_000u32).step_by(997) {
        let v = repo.db_get(&d, &format!("user/{k:06}/profile")).unwrap().unwrap();
        assert_eq!(v, format!("value {k}").into_bytes());
        get_reads.push(repo.store().take().reads as f64);
    }
    // The same lookups again through a handle-level cache: distinct objects
    // fetched per lookup once the upper chunks are warm.
    repo.store().record(true);
    let mut rng = 12345u64;
    let mut gets = 0;
    for _ in 0..1000 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let k = (rng >> 33) % 100_000;
        repo.db_get(&d, &format!("user/{k:06}/profile")).unwrap();
        gets += 1;
    }
    let distinct = repo.store().record(false).unwrap();
    let cached_bytes: u64 = distinct.iter().map(|i| repo.store().inner.read(i).unwrap().len() as u64).sum();
    let hot = repo.store().take();
    println!(
        "{{\"exp\":\"E125\",\"what\":\"db_cache\",\"gets\":{gets},\"reads_without_cache\":{},\"distinct_objects\":{},\"distinct_bytes\":{cached_bytes}}}",
        hot.reads,
        distinct.len()
    );
    let range = repo.graph_prefix(&d.kv, "user/0420").unwrap();
    let range_io = repo.store().take();
    println!(
        "{{\"exp\":\"E125\",\"what\":\"db\",\"keys\":100000,\"get_reads_median\":{},\"get_reads_max\":{},\"range_keys\":{},\"range_reads\":{},\"range_bytes\":{}}}",
        median(&mut get_reads.clone()),
        get_reads.iter().cloned().fold(0.0, f64::max),
        range.len(),
        range_io.reads,
        range_io.bytes
    );

    // Neighbourhood: outgoing links read one object; incoming links scan the store.
    let target = files.iter().find(|f| f.0.ends_with(".md")).unwrap().1;
    let t = Instant::now();
    let _ = repo.links(&target).unwrap();
    let out_io = repo.store().take();
    let out_us = t.elapsed().as_secs_f64() * 1e6;
    let t = Instant::now();
    let users = repo.used_by(&target).unwrap();
    let in_io = repo.store().take();
    let in_ms = t.elapsed().as_secs_f64() * 1e3;
    // A reverse-link view: label "target/source/label", built once.
    let t = Instant::now();
    let mut rev = Vec::new();
    for src in repo.store().ids().unwrap() {
        if let Ok(obj) = repo.get(&src) {
            for (label, to) in obj.links() {
                rev.push(Node {
                    label: format!("{to}/{src}/{label}"),
                    target: Target::Object(src),
                    edges: vec![],
                });
            }
        }
    }
    rev.sort_by(|a, b| a.label.cmp(&b.label));
    rev.dedup_by(|a, b| a.label == b.label);
    let edges = rev.len();
    let view = repo.put_graph_with(rev, Keys::Any).unwrap();
    let vbuild = repo.store().take();
    let vbuild_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let via = repo.graph_prefix(&view, &format!("{target}/")).unwrap();
    let via_io = repo.store().take();
    let via_us = t.elapsed().as_secs_f64() * 1e6;
    let mut a: Vec<Id> = users.iter().map(|(s, _)| *s).collect();
    let mut b: Vec<Id> = via.iter().map(|n| match n.target {
        Target::Object(s) => s,
        _ => panic!(),
    }).collect();
    a.sort();
    a.dedup();
    b.sort();
    b.dedup();
    assert_eq!(a, b, "reverse view must answer what the scan answers");
    println!(
        "{{\"exp\":\"E125\",\"what\":\"links\",\"out_reads\":{},\"out_us\":{out_us:.0},\"in_scan_reads\":{},\"in_scan_bytes\":{},\"in_scan_ms\":{in_ms:.1},\"users\":{},\"reverse_view_edges\":{edges},\"reverse_view_objects\":{},\"reverse_view_bytes\":{},\"reverse_view_build_ms\":{vbuild_ms:.0},\"in_view_reads\":{},\"in_view_bytes\":{},\"in_view_us\":{via_us:.0}}}",
        out_io.reads,
        in_io.reads,
        in_io.bytes,
        a.len(),
        vbuild.writes,
        vbuild.wbytes,
        via_io.reads,
        via_io.bytes,
    );
}

/// Every id a tree reaches through its entries (trees and files), with the tree itself.
fn closure(repo: &R, tree: Id, out: &mut HashSet<Id>) {
    if !out.insert(tree) {
        return;
    }
    let Object::Tree(entries) = repo.get(&tree).unwrap() else { return };
    for e in entries {
        match e.kind {
            Kind::Tree => closure(repo, e.id, out),
            _ => content_closure(repo, e.id, out),
        }
    }
}

/// A file's content id and everything its encoding names (chunks, bases).
fn content_closure(repo: &R, id: Id, out: &mut HashSet<Id>) {
    if !out.insert(id) {
        return;
    }
    for (_, to) in repo.links(&id).unwrap_or_default() {
        content_closure(repo, to, out);
    }
}

fn e126(repo: &mut R, files: &[(String, Id, usize)], trees: &[(String, Id)], root: Id, commit: Id, refname: &str) {
    // A capability: (root, rights, expiry) under a keyed BLAKE3 MAC.
    let key = *blake3::hash(b"bake-off key, not a secret").as_bytes();
    let tenants: Vec<&(String, Id)> = trees.iter().filter(|(p, _)| !p.is_empty() && !p.contains('/') && !p.starts_with('_')).collect();
    let mint = |root: &Id, rights: u8, expiry: u64| {
        let mut m = root.as_bytes().to_vec();
        m.push(rights);
        m.extend(expiry.to_le_bytes());
        (m.clone(), *blake3::keyed_hash(&key, &m).as_bytes())
    };
    let (msg, tag) = mint(&tenants[0].1, 1, 1_900_000_000);
    let t = Instant::now();
    let n = 1_000_000;
    let mut ok = 0;
    for _ in 0..n {
        ok += (blake3::keyed_hash(&key, std::hint::black_box(&msg)).as_bytes() == &tag) as u32;
    }
    let mac_ns = t.elapsed().as_secs_f64() * 1e9 / n as f64;
    assert_eq!(ok, n);

    // Scoped read checks for every file of one tenant: a Merkle path from the
    // capability's root (tree reads, cached per session) or a reachable set.
    let (tname, troot) = tenants[0];
    let mine: Vec<&(String, Id, usize)> = files.iter().filter(|f| f.0.starts_with(&format!("{tname}/"))).collect();
    repo.store().take();
    let t = Instant::now();
    let mut cache: HashMap<Id, Vec<ikam_kernel::TreeEntry>> = HashMap::new();
    for (path, id, _) in &mine {
        let mut tree = *troot;
        let rel: Vec<&str> = path[tname.len() + 1..].split('/').collect();
        for (i, p) in rel.iter().enumerate() {
            let entries = cache.entry(tree).or_insert_with(|| match repo.get(&tree).unwrap() {
                Object::Tree(e) => e,
                _ => panic!(),
            });
            let e = entries.iter().find(|e| e.name == *p).unwrap().clone();
            if i + 1 == rel.len() {
                assert_eq!(e.id, *id);
            } else {
                tree = e.id;
            }
        }
    }
    let path_io = repo.store().take();
    let path_us = t.elapsed().as_secs_f64() * 1e6 / mine.len() as f64;
    let t = Instant::now();
    let mut reach = HashSet::new();
    closure(repo, *troot, &mut reach);
    let set_io = repo.store().take();
    let set_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let mut hits = 0;
    for _ in 0..100 {
        for (_, id, _) in &mine {
            hits += reach.contains(id) as u32;
        }
    }
    let set_ns = t.elapsed().as_secs_f64() * 1e9 / (100 * mine.len()) as f64;
    assert_eq!(hits as usize, 100 * mine.len());
    println!(
        "{{\"exp\":\"E126\",\"what\":\"checks\",\"mac_verify_ns\":{mac_ns:.0},\"tenant\":\"{tname}\",\"tenant_files\":{},\"path_check_reads_total\":{},\"path_check_us_per_file\":{path_us:.1},\"reachable_set_ids\":{},\"reachable_set_build_reads\":{},\"reachable_set_build_ms\":{set_ms:.1},\"reachable_set_check_ns\":{set_ns:.0}}}",
        mine.len(),
        path_io.reads,
        reach.len(),
        set_io.reads,
    );

    // What reading a tenant's files touches in the store, outside its tree.
    let mut closures: Vec<HashSet<Id>> = Vec::new();
    for (_, r) in &tenants {
        let mut s = HashSet::new();
        closure(repo, *r, &mut s);
        closures.push(s);
    }
    let dict = repo.store().get_ref("meta/zstd-dict").unwrap();
    let mut readers: HashMap<Id, HashSet<usize>> = HashMap::new();
    for (ti, (tn, _)) in tenants.iter().enumerate() {
        repo.store().record(true);
        for (path, id, _) in files.iter().filter(|f| f.0.starts_with(&format!("{tn}/"))) {
            let _ = path;
            repo.read_content(id).unwrap();
        }
        let seen = repo.store().record(false).unwrap();
        repo.store().take();
        for id in &seen {
            readers.entry(*id).or_default().insert(ti);
        }
        let (mut inside, mut dictionary, mut other_tenant, mut internal) = (0, 0, 0, 0);
        for id in &seen {
            if closures[ti].contains(id) {
                inside += 1;
            } else if Some(*id) == dict {
                dictionary += 1;
            } else if closures.iter().enumerate().any(|(j, c)| j != ti && c.contains(id)) {
                other_tenant += 1;
            } else {
                internal += 1;
            }
        }
        let shared_files = closures[ti]
            .iter()
            .filter(|id| closures.iter().enumerate().any(|(j, c)| j != ti && c.contains(id)))
            .count();
        println!(
            "{{\"exp\":\"E126\",\"what\":\"touch\",\"tenant\":\"{tn}\",\"objects_read\":{},\"in_own_tree\":{inside},\"dictionary\":{dictionary},\"other_tenants_content\":{other_tenant},\"storage_internal\":{internal},\"ids_shared_with_other_tenants\":{shared_files}}}",
            seen.len()
        );
    }

    // Stored objects that serve more than one tenant's reads while sitting
    // in no tenant's tree (groups, dictionary): bytes a tenant's read
    // fetches that hold other tenants' content.
    let (mut shared, mut shared_bytes, mut max_tenants) = (0, 0u64, 0);
    for (id, ts) in &readers {
        if ts.len() >= 2 && !closures.iter().any(|c| c.contains(id)) {
            shared += 1;
            shared_bytes += repo.store().inner.read(id).unwrap().len() as u64;
            max_tenants = max_tenants.max(ts.len());
        }
    }
    println!(
        "{{\"exp\":\"E126\",\"what\":\"cross_tenant\",\"tenants\":{},\"objects_read\":{},\"internal_objects_serving_2plus_tenants\":{shared},\"their_bytes\":{shared_bytes},\"most_tenants_on_one_object\":{max_tenants}}}",
        tenants.len(),
        readers.len()
    );

    // A scoped edit: change one file in one tenant, splice it into the root,
    // and move the ref; count what that reads and writes.
    let (path, _, _) = mine.iter().find(|f| f.0.ends_with(".md")).unwrap();
    let rel = &path[tname.len() + 1..];
    repo.store().take();
    let new = repo.put_content(b"# Edited by an agent with a capability on one folder\n").unwrap().id;
    let sub = repo.edit_tree(Some(*troot), &[Edit::Put { path: rel.to_string(), id: new }]).unwrap();
    let top = repo.edit_tree(Some(root), &[Edit::Remove { path: tname.clone() }]).unwrap();
    let _ = top;
    // Put the new subtree back at its name: a one-entry change of the root.
    let Object::Tree(mut entries) = repo.get(&root).unwrap() else { panic!() };
    for e in entries.iter_mut() {
        if e.name == *tname {
            e.id = sub;
        }
    }
    let new_root = repo.put(&Object::tree(entries).unwrap()).unwrap();
    let new_commit = repo.commit(refname, new_root, "scoped edit").unwrap();
    let edit_io = repo.store().take();
    // The same edit by path from the root, as an unscoped writer would do it.
    let via_root = repo.edit_tree(Some(root), &[Edit::Put { path: path.clone(), id: new }]).unwrap();
    let root_io = repo.store().take();
    assert_eq!(via_root, new_root, "scoped splice must equal the edit from the root");
    let _ = (commit, new_commit);
    println!(
        "{{\"exp\":\"E126\",\"what\":\"scoped_edit\",\"path\":\"{path}\",\"reads\":{},\"writes\":{},\"write_bytes\":{},\"from_root_reads\":{},\"tenant_objects\":{},\"store_objects\":{}}}",
        edit_io.reads,
        edit_io.writes,
        edit_io.wbytes,
        root_io.reads,
        reach.len(),
        repo.store().ids().unwrap().len()
    );
}

fn e127(repo: &mut R, files: &[(String, Id, usize)]) {
    // Text-like files an agent would search.
    let text: Vec<&(String, Id, usize)> = files
        .iter()
        .filter(|f| matches!(ext(&f.0).as_str(), "md" | "json" | "jsonl" | "txt" | "py" | "log"))
        .collect();
    let ids: Vec<Id> = text.iter().map(|f| f.1).collect();
    let t = Instant::now();
    repo.build_search_index(&ids).unwrap();
    let build = repo.store().take();
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let index = repo.search_index().unwrap().unwrap();
    let load = repo.store().take();
    let mut all_bytes = 0u64;
    let mut contents: HashMap<Id, Vec<u8>> = HashMap::new();
    for id in &ids {
        let b = repo.read_content(id).unwrap();
        all_bytes += b.len() as u64;
        contents.insert(*id, b);
    }
    let all_io = repo.store().take();
    println!(
        "{{\"exp\":\"E127\",\"what\":\"index\",\"text_files\":{},\"text_bytes\":{all_bytes},\"read_all_fetched\":{},\"read_all_reads\":{},\"index_build_ms\":{build_ms:.0},\"index_bytes_written\":{},\"index_load_fetched\":{}}}",
        ids.len(),
        all_io.bytes,
        all_io.reads,
        build.wbytes,
        load.bytes
    );
    for goal in ["invoice", "churn", "triage", "safety", "revenue", "onboarding", "Q3"] {
        let t = Instant::now();
        let (hits, _) = repo.search(goal.as_bytes(), &ids, Some(&index)).unwrap();
        let search_io = repo.store().take();
        let search_us = t.elapsed().as_secs_f64() * 1e6;
        // Ground truth from the bytes.
        let truth: Vec<Id> = ids.iter().filter(|id| contents[id].windows(goal.len()).any(|w| w == goal.as_bytes())).copied().collect();
        let mut h = hits.clone();
        h.sort();
        let mut tr = truth.clone();
        tr.sort();
        assert_eq!(h, tr, "{goal}: search must equal a scan");
        let mut whole = 0u64;
        for id in &hits {
            repo.read_content(id).unwrap();
            whole += repo.store().take().bytes;
        }
        // Windows of 2 KiB around each match (at most 5 per file).
        let (mut win_fetched, mut win_bytes, mut windows) = (0u64, 0u64, 0u64);
        for id in &hits {
            let b = &contents[id];
            let mut last_end = 0usize;
            let mut n = 0;
            for (pos, w) in b.windows(goal.len()).enumerate() {
                if w == goal.as_bytes() && pos >= last_end && n < 5 {
                    let s = pos.saturating_sub(1024);
                    let e = (pos + 1024).min(b.len());
                    let (part, _) = repo.read_range(id, s as u64, (e - s) as u64).unwrap();
                    assert_eq!(part, b[s..e]);
                    win_fetched += repo.store().take().bytes;
                    win_bytes += (e - s) as u64;
                    windows += 1;
                    last_end = e;
                    n += 1;
                }
            }
        }
        println!(
            "{{\"exp\":\"E127\",\"what\":\"goal\",\"goal\":\"{goal}\",\"matching_files\":{},\"search_fetched\":{},\"search_us\":{search_us:.0},\"matched_whole_fetched\":{whole},\"windows\":{windows},\"window_bytes\":{win_bytes},\"window_fetched\":{win_fetched},\"capability_ids\":{}}}",
            hits.len(),
            search_io.bytes,
            hits.len()
        );
    }
}
