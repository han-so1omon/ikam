//! L1 codecs and ingest.
//!
//! A codec only proposes a `Shape`: a labelled tree whose leaves are byte
//! ranges of the input. Rendering is fixed by the kernel (concatenate leaves in
//! order), so a codec cannot invent a private reconstruction rule. Ingest
//! enforces the law `render(ingest(x)) == x`: any codec that proposes a bad
//! shape, or returns none, falls back to storing `x` as a single blob.
//! Codecs may be heuristic or AI-driven; correctness never depends on them.

use std::ops::Range;

use crate::{Entry, Error, Id, Kind, Object, Store};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub label: String,
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Bytes(Range<usize>),
    Group(Vec<Shape>),
}

impl Shape {
    pub fn leaf(label: impl Into<String>, range: Range<usize>) -> Shape {
        Shape {
            label: label.into(),
            body: Body::Bytes(range),
        }
    }

    pub fn group(label: impl Into<String>, children: Vec<Shape>) -> Shape {
        Shape {
            label: label.into(),
            body: Body::Group(children),
        }
    }
}

pub trait Codec {
    fn name(&self) -> &str;
    /// Propose a decomposition, or `None` if this input is not understood.
    fn split(&self, input: &[u8]) -> Option<Shape>;
}

#[derive(Debug)]
pub struct Ingested {
    pub root: Id,
    pub kind: Kind,
    /// True when the codec's shape was rejected and the input stored whole.
    pub fell_back: bool,
    pub objects_written: usize,
    pub bytes_written: usize,
}

pub fn ingest(store: &mut dyn Store, codec: &dyn Codec, input: &[u8]) -> Result<Ingested, Error> {
    let mut out = Ingested {
        root: Id::of(b""),
        kind: Kind::Blob,
        fell_back: true,
        objects_written: 0,
        bytes_written: 0,
    };
    if let Some(shape) = codec.split(input).filter(|s| tiles(s, input.len())) {
        let (kind, root) = materialize(store, &shape, input, &mut out)?;
        // The tiling check makes this hold by construction; verify anyway so
        // the guarantee rests on a byte comparison, not on reasoning.
        if render(store, &root)? == input {
            return Ok(Ingested {
                root,
                kind,
                fell_back: false,
                ..out
            });
        }
    }
    let (root, fresh) = store.put(&Object::Blob(input.to_vec()))?;
    count(&mut out, fresh, input.len() + 1);
    Ok(Ingested {
        root,
        kind: Kind::Blob,
        fell_back: true,
        ..out
    })
}

/// Reconstruct the bytes an object stands for: a blob is itself, a node is
/// the concatenation of its children. Every read is hash-verified.
pub fn render(store: &dyn Store, id: &Id) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    render_into(store, id, None, &mut out)?;
    Ok(out)
}

fn render_into(
    store: &dyn Store,
    id: &Id,
    expect: Option<Kind>,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let obj = store.get(id)?;
    if expect.is_some_and(|k| k != obj.kind()) {
        return Err(Error::Corrupt(*id));
    }
    match obj {
        Object::Blob(bytes) => out.extend_from_slice(&bytes),
        Object::Node(entries) => {
            for e in entries {
                render_into(store, &e.id, Some(e.kind), out)?;
            }
        }
    }
    Ok(())
}

/// Leaves, read in order, must cover `0..len` exactly with no empty leaf
/// (except a single empty leaf for empty input).
fn tiles(shape: &Shape, len: usize) -> bool {
    let mut leaves = Vec::new();
    collect_leaves(shape, &mut leaves);
    if len == 0 {
        return leaves.len() == 1 && leaves[0].is_empty();
    }
    let mut pos = 0;
    for r in leaves {
        if r.start != pos || r.end <= r.start {
            return false;
        }
        pos = r.end;
    }
    pos == len
}

fn collect_leaves(shape: &Shape, out: &mut Vec<Range<usize>>) {
    match &shape.body {
        Body::Bytes(r) => out.push(r.clone()),
        Body::Group(children) => children.iter().for_each(|c| collect_leaves(c, out)),
    }
}

fn materialize(
    store: &mut dyn Store,
    shape: &Shape,
    input: &[u8],
    stats: &mut Ingested,
) -> Result<(Kind, Id), Error> {
    let obj = match &shape.body {
        Body::Bytes(r) => Object::Blob(input[r.clone()].to_vec()),
        Body::Group(children) => {
            let mut entries = Vec::with_capacity(children.len());
            for child in children {
                let (kind, id) = materialize(store, child, input, stats)?;
                entries.push(Entry {
                    label: child.label.clone(),
                    kind,
                    id,
                });
            }
            Object::Node(entries)
        }
    };
    let size = obj.encode().len();
    let (id, fresh) = store.put(&obj)?;
    count(stats, fresh, size);
    Ok((obj.kind(), id))
}

fn count(stats: &mut Ingested, fresh: bool, size: usize) {
    if fresh {
        stats.objects_written += 1;
        stats.bytes_written += size;
    }
}

/// The whole input as one blob.
pub struct Raw;

impl Codec for Raw {
    fn name(&self) -> &str {
        "raw"
    }

    fn split(&self, input: &[u8]) -> Option<Shape> {
        Some(Shape::leaf("", 0..input.len()))
    }
}

/// Content-defined chunking (FastCDC): dedups shifted or partially edited
/// binary content without understanding it.
pub struct Cdc {
    pub min: usize,
    pub avg: usize,
    pub max: usize,
}

impl Default for Cdc {
    fn default() -> Cdc {
        Cdc {
            min: 2048,
            avg: 8192,
            max: 65536,
        }
    }
}

impl Codec for Cdc {
    fn name(&self) -> &str {
        "cdc"
    }

    fn split(&self, input: &[u8]) -> Option<Shape> {
        if input.is_empty() {
            return Raw.split(input);
        }
        let chunks = fastcdc::v2020::FastCDC::new(input, self.min, self.avg, self.max)
            .map(|c| Shape::leaf("", c.offset..c.offset + c.length))
            .collect();
        Some(Shape::group("cdc", chunks))
    }
}

/// Markdown: ATX-heading sections, each split into a heading leaf and
/// paragraph leaves. Blank lines stay attached to what they follow; fenced
/// code is never split.
pub struct Markdown;

impl Codec for Markdown {
    fn name(&self) -> &str {
        "md"
    }

    fn split(&self, input: &[u8]) -> Option<Shape> {
        if input.is_empty() {
            return Raw.split(input);
        }
        let mut sections: Vec<Shape> = Vec::new();
        let mut paras: Vec<Shape> = Vec::new();
        let (mut title, mut para_start, mut in_fence) = (String::new(), 0, false);
        let (mut prev_blank, mut prev_heading) = (false, false);
        for line in lines(input) {
            let text = &input[line.clone()];
            let trimmed = text.trim_ascii_start();
            let blank = text.trim_ascii().is_empty();
            let heading = !in_fence && trimmed.starts_with(b"#");
            let para_break = !in_fence && !blank && (prev_blank || prev_heading);
            if (heading || para_break) && line.start > para_start {
                paras.push(Shape::leaf("", para_start..line.start));
                para_start = line.start;
            }
            if heading && !paras.is_empty() {
                sections.push(Shape::group(
                    std::mem::take(&mut title),
                    std::mem::take(&mut paras),
                ));
            }
            if heading {
                title = String::from_utf8_lossy(trimmed.trim_ascii()).into_owned();
            }
            if trimmed.starts_with(b"```") || trimmed.starts_with(b"~~~") {
                in_fence = !in_fence;
            }
            prev_blank = blank;
            prev_heading = heading;
        }
        paras.push(Shape::leaf("", para_start..input.len()));
        sections.push(Shape::group(title, paras));
        Some(Shape::group("md", sections))
    }
}

/// Byte ranges of lines, each including its trailing `\n` if present.
fn lines(input: &[u8]) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start >= input.len() {
            return None;
        }
        let end = input[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(input.len(), |i| start + i + 1);
        let r = start..end;
        start = end;
        Some(r)
    })
}

pub fn by_name(name: &str) -> Option<Box<dyn Codec>> {
    match name {
        "raw" => Some(Box::new(Raw)),
        "cdc" => Some(Box::new(Cdc::default())),
        "md" => Some(Box::new(Markdown)),
        _ => None,
    }
}
