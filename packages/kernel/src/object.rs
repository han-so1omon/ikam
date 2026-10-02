//! L0 objects and their canonical encoding.
//!
//! Encoding (u32/u64 are big-endian):
//!   blob   = "B" bytes
//!   rep    = "R" count:u32 { src[32] start:u64 len:u64 }
//!   tree   = "T" count:u32 { name_len:u32 name kind("F"|"T") id[32] }   names strictly ascending
//!   commit = "C" tree[32] nparents:u32 { parent[32] } msg_len:u32 msg
//!   apply  = "A" func[32] nargs:u32 { arg[32] }
//!   run    = "X" func[32] nargs:u32 { arg[32] } output[32]
//!
//! Trees, commits and runs are named by `BLAKE3(encoding)`. A file's identity
//! is the id of its blob form. `Rep` (slices of other blobs) and `Apply` (a
//! pure function of other files) are alternative storage forms kept under
//! that same id, so how a file is stored never changes what it is called.
//! Fragments are therefore values (blobs), references (reps) or function
//! applications (applies); all three must reproduce the identical bytes.
//! Decoding is strict: `encode(decode(b)) == b` whenever it succeeds.

use crate::{Error, Id};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    File,
    Tree,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub name: String,
    pub kind: Kind,
    pub id: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slice {
    pub src: Id,
    pub start: u64,
    pub len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub tree: Id,
    pub parents: Vec<Id>,
    pub message: String,
}

/// `func` applied to the contents of `args`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Apply {
    pub func: Id,
    pub args: Vec<Id>,
}

/// A record that evaluating `apply` produced `output`: provenance and memo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub apply: Apply,
    pub output: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Object {
    Blob(Vec<u8>),
    Rep(Vec<Slice>),
    Tree(Vec<TreeEntry>),
    Commit(Commit),
    Apply(Apply),
    Run(Run),
}

impl Object {
    /// Build a tree, sorting entries; fails on duplicate or invalid names.
    pub fn tree(mut entries: Vec<TreeEntry>) -> Result<Object, Error> {
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let tree = Object::Tree(entries);
        Object::decode(&tree.encode())?;
        Ok(tree)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Object::Blob(bytes) => {
                out.push(b'B');
                out.extend_from_slice(bytes);
            }
            Object::Rep(slices) => {
                out.push(b'R');
                put_u32(&mut out, slices.len());
                for s in slices {
                    out.extend_from_slice(s.src.as_bytes());
                    out.extend_from_slice(&s.start.to_be_bytes());
                    out.extend_from_slice(&s.len.to_be_bytes());
                }
            }
            Object::Tree(entries) => {
                out.push(b'T');
                put_u32(&mut out, entries.len());
                for e in entries {
                    put_str(&mut out, &e.name);
                    out.push(if e.kind == Kind::File { b'F' } else { b'T' });
                    out.extend_from_slice(e.id.as_bytes());
                }
            }
            Object::Commit(c) => {
                out.push(b'C');
                out.extend_from_slice(c.tree.as_bytes());
                put_u32(&mut out, c.parents.len());
                c.parents
                    .iter()
                    .for_each(|p| out.extend_from_slice(p.as_bytes()));
                put_str(&mut out, &c.message);
            }
            Object::Apply(a) => put_apply(&mut out, b'A', a),
            Object::Run(run) => {
                put_apply(&mut out, b'X', &run.apply);
                out.extend_from_slice(run.output.as_bytes());
            }
        }
        out
    }

    /// Outgoing edges `(label, target)`: the object graph that gc walks and
    /// that connectivity queries read.
    pub fn links(&self) -> Vec<(String, Id)> {
        let apply_links = |a: &Apply| {
            let args = a
                .args
                .iter()
                .enumerate()
                .map(|(i, id)| (format!("arg{i}"), *id));
            std::iter::once(("func".to_string(), a.func))
                .chain(args)
                .collect::<Vec<_>>()
        };
        match self {
            Object::Blob(_) => vec![],
            Object::Rep(slices) => {
                let mut srcs: Vec<Id> = slices.iter().map(|s| s.src).collect();
                srcs.sort();
                srcs.dedup();
                srcs.into_iter()
                    .map(|id| ("slice".to_string(), id))
                    .collect()
            }
            Object::Tree(entries) => entries.iter().map(|e| (e.name.clone(), e.id)).collect(),
            Object::Commit(c) => std::iter::once(("tree".to_string(), c.tree))
                .chain(c.parents.iter().map(|p| ("parent".to_string(), *p)))
                .collect(),
            Object::Apply(a) => apply_links(a),
            Object::Run(run) => {
                let mut links = apply_links(&run.apply);
                links.push(("output".to_string(), run.output));
                links
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Object, Error> {
        let mut r = Reader(bytes);
        let obj = match r.take(1)?[0] {
            b'B' => Object::Blob(r.rest().to_vec()),
            b'R' => Object::Rep(r.many(|r| {
                let s = Slice {
                    src: r.id()?,
                    start: r.u64()?,
                    len: r.u64()?,
                };
                if s.len == 0 {
                    return Err(Error::Decode("empty slice"));
                }
                Ok(s)
            })?),
            b'T' => decode_tree(&mut r)?,
            b'C' => Object::Commit(Commit {
                tree: r.id()?,
                parents: r.many(Reader::id)?,
                message: r.string()?,
            }),
            b'A' => Object::Apply(Apply {
                func: r.id()?,
                args: r.many(Reader::id)?,
            }),
            b'X' => Object::Run(Run {
                apply: Apply {
                    func: r.id()?,
                    args: r.many(Reader::id)?,
                },
                output: r.id()?,
            }),
            _ => return Err(Error::Decode("unknown object tag")),
        };
        r.finish()?;
        Ok(obj)
    }

    pub fn id(&self) -> Id {
        Id::of(&self.encode())
    }
}

fn decode_tree(r: &mut Reader) -> Result<Object, Error> {
    let entries = r.many(|r| {
        let name = r.string()?;
        let kind = match r.take(1)?[0] {
            b'F' => Kind::File,
            b'T' => Kind::Tree,
            _ => return Err(Error::Decode("unknown entry kind")),
        };
        Ok(TreeEntry {
            name,
            kind,
            id: r.id()?,
        })
    })?;
    for e in &entries {
        if e.name.is_empty() || e.name == "." || e.name == ".." || e.name.contains(['/', '\0']) {
            return Err(Error::InvalidName(e.name.clone()));
        }
    }
    if entries.windows(2).any(|w| w[0].name >= w[1].name) {
        return Err(Error::Decode("tree names not strictly ascending"));
    }
    Ok(Object::Tree(entries))
}

fn put_apply(out: &mut Vec<u8>, tag: u8, a: &Apply) {
    out.push(tag);
    out.extend_from_slice(a.func.as_bytes());
    put_u32(out, a.args.len());
    a.args
        .iter()
        .for_each(|id| out.extend_from_slice(id.as_bytes()));
}

fn put_u32(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&u32::try_from(n).expect("length exceeds u32").to_be_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len());
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.0.len() < n {
            return Err(Error::Decode("truncated"));
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.0)
    }

    fn u32(&mut self) -> Result<usize, Error> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize)
    }

    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn id(&mut self) -> Result<Id, Error> {
        Ok(Id::from_bytes(self.take(32)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String, Error> {
        let len = self.u32()?;
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| Error::Decode("string not utf-8"))
    }

    fn many<T>(
        &mut self,
        mut item: impl FnMut(&mut Self) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        let count = self.u32()?;
        let mut out = Vec::with_capacity(count.min(self.0.len() / 32));
        for _ in 0..count {
            out.push(item(self)?);
        }
        Ok(out)
    }

    fn finish(self) -> Result<(), Error> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Error::Decode("trailing bytes"))
        }
    }
}
