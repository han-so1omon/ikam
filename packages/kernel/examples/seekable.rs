//! E150: what window reads would cost if each stored file were cut into
//! independently compressed frames, each verifiable against the file's id
//! (a BLAKE3 outboard, as Bao keeps), and what the store would grow by.
//! A measurement only: ikam stores no such frames.
//!
//! `cargo run --release --example seekable -- SRC_STORE REF`
//!
//! For each goal term and frame size, prints one JSON line over the text
//! files that hold the term: their bytes, zstd of each whole file, the
//! windows (two lines around each mention), and the frames a window read
//! needs with their proof bytes. One more line per frame size gives the
//! growth over every text file. Compression is zstd level 3 without a
//! dictionary throughout, so the figures compare with each other, not with
//! the store's own encodings.

use std::path::Path;

use ikam_kernel::{FsStore, Id, Kind, MemStore, Object, Repo, Store, windows_around};

const TEXT: &[&str] = &[
    "md", "txt", "csv", "json", "jsonl", "html", "xml", "py", "log", "eml",
];

fn files(repo: &Repo<MemStore>, tree: Id, prefix: &str, out: &mut Vec<(String, Id)>) {
    let Object::Tree(entries) = repo.get(&tree).unwrap() else {
        panic!("not a tree")
    };
    for e in entries {
        let path = if prefix.is_empty() {
            e.name.clone()
        } else {
            format!("{prefix}/{}", e.name)
        };
        match e.kind {
            Kind::Tree => files(repo, e.id, &path, out),
            _ => out.push((path, e.id)),
        }
    }
}

fn z(b: &[u8]) -> usize {
    zstd::bulk::compress(b, 3).unwrap().len()
}

/// Proof bytes to verify a run of frames against the root of a binary hash
/// tree over `n` frames: two sibling hashes per level, on each side.
fn proof(n: usize) -> usize {
    if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as usize * 64
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let fs = FsStore::open(Path::new(&a[0])).unwrap();
    let mut mem = MemStore::default();
    for id in fs.ids().unwrap() {
        mem.write(id, &fs.read(&id).unwrap()).unwrap();
    }
    for (name, id) in fs.refs().unwrap() {
        mem.set_ref(&name, None, id).unwrap();
    }
    let repo = Repo::new(mem);
    let commit = repo.resolve(&a[1]).unwrap();
    let Object::Commit(c) = repo.get(&commit).unwrap() else {
        panic!()
    };
    let mut all = Vec::new();
    files(&repo, c.tree, "", &mut all);
    let text: Vec<Vec<u8>> = all
        .iter()
        .filter(|f| {
            f.0.rsplit_once('.')
                .is_some_and(|(_, e)| TEXT.contains(&e.to_lowercase().as_str()))
        })
        .map(|f| repo.read_content(&f.1).unwrap())
        .collect();
    let sizes = [1024usize, 4096, 8192, 16384, 65536];
    for frame in sizes {
        let (mut whole, mut framed, mut outboard) = (0, 0, 0);
        for b in &text {
            whole += z(b);
            let n = b.len().div_ceil(frame).max(1);
            framed += b.chunks(frame).map(z).sum::<usize>();
            // A frame table of compressed lengths, and the tree's inner
            // hashes, two per inner node.
            outboard += 4 * n + 64 * (n - 1);
        }
        println!(
            "{{\"exp\":\"E150\",\"frame\":{frame},\"part\":\"store\",\"text_files\":{},\"raw\":{},\"whole_z\":{whole},\"framed_z\":{framed},\"outboard\":{outboard}}}",
            text.len(),
            text.iter().map(Vec::len).sum::<usize>()
        );
    }
    for term in ["Q3", "AI"] {
        let finder = memchr::memmem::Finder::new(term.as_bytes());
        let hits: Vec<&Vec<u8>> = text.iter().filter(|b| finder.find(b).is_some()).collect();
        let raw: usize = hits.iter().map(|b| b.len()).sum();
        let whole: usize = hits.iter().map(|b| z(b)).sum();
        let windows: usize = hits
            .iter()
            .map(|b| {
                windows_around(b, term.as_bytes(), 2)
                    .iter()
                    .map(|r| (r.end - r.start) as usize)
                    .sum::<usize>()
            })
            .sum();
        for frame in sizes {
            let (mut need, mut proofs, mut frames_read) = (0, 0, 0);
            for b in &hits {
                let n = b.len().div_ceil(frame).max(1);
                let mut wanted = vec![false; n];
                for r in windows_around(b, term.as_bytes(), 2) {
                    for f in (r.start as usize / frame)..=((r.end as usize - 1) / frame) {
                        wanted[f] = true;
                    }
                }
                let mut runs = 0;
                for (i, w) in wanted.iter().enumerate() {
                    if *w {
                        let end = ((i + 1) * frame).min(b.len());
                        need += z(&b[i * frame..end]) + 4;
                        frames_read += 1;
                        if i == 0 || !wanted[i - 1] {
                            runs += 1;
                        }
                    }
                }
                proofs += runs * proof(n);
            }
            println!(
                "{{\"exp\":\"E150\",\"term\":\"{term}\",\"frame\":{frame},\"files\":{},\"raw\":{raw},\"whole_z\":{whole},\"windows\":{windows},\"frames_read\":{frames_read},\"frames_z\":{need},\"proofs\":{proofs},\"window_read\":{}}}",
                hits.len(),
                need + proofs
            );
        }
    }
}
