//! L0 objects and their canonical encoding.
//!
//! Encoding (u32/u64 are big-endian):
//!   blob       = "B" bytes                     (storage variant "Z" zstd(bytes), same id)
//!   tree       = "T" count:u32 { name_len:u32 name kind("F"|"T") id[32] }   names strictly ascending
//!   commit     = "C" tree[32] nparents:u32 { parent[32] } msg_len:u32 msg
//!   derivation = "D" output[32] func[32] nargs:u32 { arg }
//!   arg        = 0 id[32] | 1 id[32] start:u64 len:u64              (whole content | byte range)
//!
//! A file's identity is the id of its blob form, `Id::of_content(bytes)`.
//! Trees, commits and derivations are named by `BLAKE3(encoding)`.
//!
//! A derivation is a ledger entry: `output = func(args)`, verified by
//! reproducing the output's exact bytes before it is recorded. One output may
//! have many derivations. Whether an output's bytes are also stored is a
//! separate, cost-driven choice, so fragments can be values (stored bytes),
//! references (byte ranges of other content) or function applications, and
//! all of them name the same bytes.
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
pub struct Commit {
    pub tree: Id,
    pub parents: Vec<Id>,
    pub message: String,
}

/// A selector: all of a content's bytes, or a byte range of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arg {
    Whole(Id),
    Range { id: Id, start: u64, len: u64 },
}

impl Arg {
    pub fn id(&self) -> Id {
        match self {
            Arg::Whole(id) | Arg::Range { id, .. } => *id,
        }
    }
}

/// `output = func(args)`, recorded only after it has been verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Derivation {
    pub output: Id,
    pub func: Id,
    pub args: Vec<Arg>,
}

impl Derivation {
    /// Identity of the computation alone (func and args, not output): the
    /// memo key for pure functions.
    pub fn key(func: &Id, args: &[Arg]) -> Id {
        let mut out = vec![b'K'];
        out.extend_from_slice(func.as_bytes());
        put_args(&mut out, args);
        Id::of(&out)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Object {
    Blob(Vec<u8>),
    Tree(Vec<TreeEntry>),
    Commit(Commit),
    Derivation(Derivation),
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
            Object::Derivation(d) => {
                out.push(b'D');
                out.extend_from_slice(d.output.as_bytes());
                out.extend_from_slice(d.func.as_bytes());
                put_args(&mut out, &d.args);
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Object, Error> {
        let mut r = Reader(bytes);
        let obj = match r.take(1)?[0] {
            b'B' => Object::Blob(r.rest().to_vec()),
            b'T' => decode_tree(&mut r)?,
            b'C' => Object::Commit(Commit {
                tree: r.id()?,
                parents: r.many(Reader::id)?,
                message: r.string()?,
            }),
            b'D' => Object::Derivation(Derivation {
                output: r.id()?,
                func: r.id()?,
                args: r.many(Reader::arg)?,
            }),
            _ => return Err(Error::Decode("unknown object tag")),
        };
        r.finish()?;
        Ok(obj)
    }

    pub fn id(&self) -> Id {
        Id::of(&self.encode())
    }

    /// Outgoing edges `(label, target)`. Range selectors keep their offsets
    /// in the label, so the graph says which part of a source is used.
    pub fn links(&self) -> Vec<(String, Id)> {
        match self {
            Object::Blob(_) => vec![],
            Object::Tree(entries) => entries.iter().map(|e| (e.name.clone(), e.id)).collect(),
            Object::Commit(c) => std::iter::once(("tree".to_string(), c.tree))
                .chain(c.parents.iter().map(|p| ("parent".to_string(), *p)))
                .collect(),
            Object::Derivation(d) => {
                let mut links = vec![
                    ("output".to_string(), d.output),
                    ("func".to_string(), d.func),
                ];
                for (i, a) in d.args.iter().enumerate() {
                    let label = match a {
                        Arg::Whole(_) => format!("arg{i}"),
                        Arg::Range { start, len, .. } => {
                            format!("arg{i}[{start}..{}]", start + len)
                        }
                    };
                    links.push((label, a.id()));
                }
                links
            }
        }
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

fn put_args(out: &mut Vec<u8>, args: &[Arg]) {
    put_u32(out, args.len());
    for a in args {
        match a {
            Arg::Whole(id) => {
                out.push(0);
                out.extend_from_slice(id.as_bytes());
            }
            Arg::Range { id, start, len } => {
                out.push(1);
                out.extend_from_slice(id.as_bytes());
                out.extend_from_slice(&start.to_be_bytes());
                out.extend_from_slice(&len.to_be_bytes());
            }
        }
    }
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
        let (head, tail) = self
            .0
            .split_at_checked(n)
            .ok_or(Error::Decode("truncated"))?;
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

    fn arg(&mut self) -> Result<Arg, Error> {
        match self.take(1)?[0] {
            0 => Ok(Arg::Whole(self.id()?)),
            1 => {
                let (id, start, len) = (self.id()?, self.u64()?, self.u64()?);
                if len == 0 || start.checked_add(len).is_none() {
                    return Err(Error::Decode("empty or overflowing range"));
                }
                Ok(Arg::Range { id, start, len })
            }
            _ => Err(Error::Decode("unknown arg tag")),
        }
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
