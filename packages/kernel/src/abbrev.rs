//! Abbreviated ids in stored trees (stored form only; ids and canonical
//! encodings are unchanged).
//!
//! A tree's entry ids are most of its size and do not compress. As in git's
//! abbreviated object names and Meister et al.'s page-based file recipes
//! (FAST 2013), an id the store can resolve is written as its first
//! `ABBREV` bytes: only if it is stored or derivable now and no other such
//! id shares the prefix. Later objects may come to share a prefix; decoding
//! then tries the combinations and accepts only the one whose canonical
//! bytes hash to the tree's id, so ambiguity can cost time, never exactness.
//!
//! Form of the tree's encoding after its tag:
//!   count:u32 { name_len:u32 name kind (0 id[32] | 1 id[ABBREV]) }
//!
//! Derivation records (E104) are mostly ids too: a container's record names
//! every member. Their short form after the tag:
//!   output[32] func (0 id[32] | 1 id[ABBREV] | 2 builtin:u8)
//!   count:varint { kind (0 whole | 1 range, | SHORT_ID) id[32 | ABBREV]
//!   [start:varint len:varint] }
//! The output stays whole: the ledger is built from the records, so the
//! outputs (what "derivable" means) must be readable first. Arguments and
//! WASM functions resolve against stored ids and those outputs.

use crate::group::{put_varint, varint};
use crate::{Arg, Derivation, Error, Id, Object, Repo, Store, func};

/// Stored or derivable ids starting with a prefix.
pub(crate) type Resolve<'a> = &'a dyn Fn(&[u8]) -> Result<Vec<Id>, Error>;

/// Argument-kind flag: the id is abbreviated.
const SHORT_ID: u8 = 2;

/// Bytes kept of an abbreviated id: a deliberate collision costs ~2^64 work.
const ABBREV: usize = 8;
/// Bound on candidate combinations tried when prefixes became ambiguous.
const MAX_TRIES: usize = 1 << 12;

/// One parsed entry: (name and kind bytes, id or id prefix).
type Entry<'a> = (&'a [u8], &'a [u8]);

fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (head, tail) = b.split_at_checked(n)?;
    *b = tail;
    Some(head)
}

fn u32_at(b: &mut &[u8]) -> Option<usize> {
    Some(u32::from_be_bytes(take(b, 4)?.try_into().ok()?) as usize)
}

/// Entries of a tree encoding after its tag; `short` for the abbreviated form.
fn entries(mut b: &[u8], short: bool) -> Option<Vec<Entry<'_>>> {
    let count = u32_at(&mut b)?;
    let mut out = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        let start = b;
        let name = u32_at(&mut b)?;
        take(&mut b, name + 1)?;
        let head = &start[..4 + name + 1];
        let len = match short {
            false => 32,
            true => [32, ABBREV][*take(&mut b, 1)?.first()? as usize % 2],
        };
        out.push((head, take(&mut b, len)?));
    }
    b.is_empty().then_some(out)
}

impl<S: Store> Repo<S> {
    /// The abbreviated form of a canonical tree encoding (after its tag),
    /// or `None` if no id can be abbreviated.
    pub(crate) fn abbreviate(&self, rest: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let Some(entries) = entries(rest, false) else {
            return Ok(None);
        };
        let (mut out, mut any) = (rest[..4].to_vec(), false);
        for (head, id) in entries {
            out.extend_from_slice(head);
            let unique =
                self.ids_with_prefix(&id[..ABBREV])? == [Id::from_bytes(id.try_into().unwrap())];
            out.push(unique as u8);
            out.extend_from_slice(if unique { &id[..ABBREV] } else { id });
            any |= unique;
        }
        Ok(any.then_some(out))
    }

    /// The canonical tree encoding (after its tag) of tree `id`, from its
    /// abbreviated form.
    pub(crate) fn expand(&self, id: &Id, short: &[u8]) -> Result<Vec<u8>, Error> {
        let entries = entries(short, true).ok_or(Error::Corrupt(*id))?;
        let mut candidates = Vec::with_capacity(entries.len());
        for (_, key) in &entries {
            candidates.push(match key.len() {
                32 => vec![Id::from_bytes((*key).try_into().unwrap())],
                _ => self.ids_with_prefix(key)?,
            });
        }
        let tries: usize = candidates.iter().map(Vec::len).product();
        // Odometer over candidate choices; almost always exactly one.
        let mut pick = vec![0; candidates.len()];
        for _ in 0..tries.min(MAX_TRIES) {
            let mut rest = short[..4].to_vec();
            for ((head, _), (c, &p)) in entries.iter().zip(candidates.iter().zip(&pick)) {
                rest.extend_from_slice(head);
                rest.extend_from_slice(c[p].as_bytes());
            }
            if Id::of(&[&b"T"[..], &rest].concat()) == *id {
                return Ok(rest);
            }
            for (p, c) in pick.iter_mut().zip(&candidates) {
                *p += 1;
                if *p < c.len() {
                    break;
                }
                *p = 0;
            }
        }
        Err(Error::Corrupt(*id))
    }
}

/// The short form of a canonical derivation encoding (after its tag).
pub(crate) fn abbreviate_derivation(
    rest: &[u8],
    resolve: Resolve,
) -> Result<Option<Vec<u8>>, Error> {
    let Ok(Object::Derivation(d)) = Object::decode(&[&b"D"[..], rest].concat()) else {
        return Ok(None);
    };
    let unique =
        |id: &Id| -> Result<bool, Error> { Ok(resolve(&id.as_bytes()[..ABBREV])? == [*id]) };
    let mut out = d.output.as_bytes().to_vec();
    if let Some(k) = func::builtin_index(&d.func) {
        out.extend_from_slice(&[2, k]);
    } else if unique(&d.func)? {
        out.push(1);
        out.extend_from_slice(&d.func.as_bytes()[..ABBREV]);
    } else {
        out.push(0);
        out.extend_from_slice(d.func.as_bytes());
    }
    put_varint(&mut out, d.args.len());
    for a in &d.args {
        let id = a.id();
        let short = unique(&id)?;
        let kind = u8::from(matches!(a, Arg::Range { .. })) | if short { SHORT_ID } else { 0 };
        out.push(kind);
        out.extend_from_slice(if short {
            &id.as_bytes()[..ABBREV]
        } else {
            id.as_bytes()
        });
        if let Arg::Range { start, len, .. } = a {
            put_varint(&mut out, *start as usize);
            put_varint(&mut out, *len as usize);
        }
    }
    Ok(Some(out))
}

/// One argument of a short derivation: its candidates and range, if any.
type ShortArg = (Vec<Id>, Option<(u64, u64)>);

/// The canonical derivation encoding (after its tag) of record `id`, from
/// its short form.
pub(crate) fn expand_derivation(id: &Id, short: &[u8], resolve: Resolve) -> Result<Vec<u8>, Error> {
    let bad = || Error::Corrupt(*id);
    let mut b = short;
    let output = Id::from_bytes(take(&mut b, 32).ok_or_else(bad)?.try_into().unwrap());
    let candidates = |key: &[u8]| -> Result<Vec<Id>, Error> {
        Ok(match key.len() {
            32 => vec![Id::from_bytes(key.try_into().unwrap())],
            _ => resolve(key)?,
        })
    };
    let funcs = match *take(&mut b, 1).ok_or_else(bad)?.first().ok_or_else(bad)? {
        0 => candidates(take(&mut b, 32).ok_or_else(bad)?)?,
        1 => candidates(take(&mut b, ABBREV).ok_or_else(bad)?)?,
        2 => vec![
            func::builtin_at(*take(&mut b, 1).ok_or_else(bad)?.first().ok_or_else(bad)?)
                .ok_or_else(bad)?,
        ],
        _ => return Err(bad()),
    };
    let count = varint(&mut b).ok_or_else(bad)?;
    let mut args: Vec<ShortArg> = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        let kind = *take(&mut b, 1).ok_or_else(bad)?.first().ok_or_else(bad)?;
        let len = if kind & SHORT_ID != 0 { ABBREV } else { 32 };
        let ids = candidates(take(&mut b, len).ok_or_else(bad)?)?;
        let range = match kind & !SHORT_ID {
            0 => None,
            1 => Some((
                varint(&mut b).ok_or_else(bad)? as u64,
                varint(&mut b).ok_or_else(bad)? as u64,
            )),
            _ => return Err(bad()),
        };
        args.push((ids, range));
    }
    if !b.is_empty() {
        return Err(bad());
    }
    let mut choices: Vec<&[Id]> = vec![&funcs];
    choices.extend(args.iter().map(|(ids, _)| ids.as_slice()));
    let tries: usize = choices.iter().map(|c| c.len()).product();
    let mut pick = vec![0; choices.len()];
    for _ in 0..tries.min(MAX_TRIES) {
        let d = Derivation {
            output,
            func: funcs[pick[0]],
            args: (args.iter().zip(&pick[1..]))
                .map(|((ids, range), &p)| match range {
                    None => Arg::Whole(ids[p]),
                    Some((start, len)) => Arg::Range {
                        id: ids[p],
                        start: *start,
                        len: *len,
                    },
                })
                .collect(),
        };
        let canonical = Object::Derivation(d).encode();
        if Id::of(&canonical) == *id {
            return Ok(canonical[1..].to_vec());
        }
        for (p, c) in pick.iter_mut().zip(&choices) {
            *p += 1;
            if *p < c.len() {
                break;
            }
            *p = 0;
        }
    }
    Err(bad())
}

#[cfg(test)]
mod tests {
    use crate::{Id, Kind, MemStore, Object, Repo, Store, TreeEntry};

    /// Ids the store holds are abbreviated, others kept whole; a decoy that
    /// later shares an abbreviated prefix costs a retry, not exactness.
    #[test]
    fn abbreviated_trees_decode_exactly() {
        let mut repo = Repo::new(MemStore::default());
        let mut entries: Vec<TreeEntry> = (0..40u32)
            .map(|i| TreeEntry {
                name: format!("docs__page-{i:03}.md"),
                kind: Kind::File,
                id: repo.put_content(format!("page {i}").as_bytes()).unwrap().id,
            })
            .collect();
        entries.push(TreeEntry {
            name: "missing".into(),
            kind: Kind::File,
            id: Id::of_content(b"never stored"),
        });
        let tree = Object::tree(entries.clone()).unwrap();
        let canonical = tree.encode();
        let short = repo.abbreviate(&canonical[1..]).unwrap().unwrap();
        assert_eq!(short.len(), canonical.len() - 1 + 41 - 40 * 24);
        assert_eq!(repo.expand(&tree.id(), &short).unwrap(), &canonical[1..]);

        let mut decoy = *entries[7].id.as_bytes();
        decoy[31] ^= 1;
        repo.store.write(Id::from_bytes(decoy), b"Bdecoy").unwrap();
        assert_eq!(repo.expand(&tree.id(), &short).unwrap(), &canonical[1..]);
    }

    /// A derivation record stores its stored or derivable arguments and its
    /// builtin by short forms, reads back exactly, and survives a decoy and
    /// a fresh ledger (built from the short records themselves).
    #[test]
    fn short_derivation_records_decode_exactly() {
        use crate::{Arg, Derivation, func};
        let mut repo = Repo::new(MemStore::default());
        let parts: Vec<Id> = (0..20u32)
            .map(|i| {
                repo.put_content(format!("part number {i} of the whole").as_bytes())
                    .unwrap()
                    .id
            })
            .collect();
        let mut args: Vec<Arg> = parts.iter().map(|&id| Arg::Whole(id)).collect();
        args[3] = Arg::Range {
            id: parts[3],
            start: 0,
            len: 4,
        };
        let d = Derivation {
            output: Id::of_content(b"some output"),
            func: func::concat(),
            args,
        };
        let canonical = Object::Derivation(d.clone()).encode();
        let short = super::abbreviate_derivation(&canonical[1..], &|p| repo.ids_with_prefix(p))
            .unwrap()
            .unwrap();
        assert!(
            short.len() < canonical.len() - 20 * 24,
            "{} vs {}",
            short.len(),
            canonical.len()
        );
        let id = Id::of(&canonical);
        let expanded = super::expand_derivation(&id, &short, &|p| repo.ids_with_prefix(p)).unwrap();
        assert_eq!(expanded, &canonical[1..]);

        let rec = repo.record(d.clone()).unwrap();
        let mut decoy = *parts[5].as_bytes();
        decoy[31] ^= 1;
        repo.store.write(Id::from_bytes(decoy), b"Bdecoy").unwrap();
        repo.reset_projections();
        assert_eq!(repo.read_object(&rec).unwrap(), canonical);
        assert_eq!(repo.derivations(&d.output).unwrap(), vec![d]);
    }

    /// A tree too small to compress still stores abbreviated ids.
    #[test]
    fn tiny_trees_store_abbreviated_ids() {
        let mut repo = Repo::new(MemStore::default());
        let file = repo
            .put_content(b"one file in its own directory")
            .unwrap()
            .id;
        let tree = Object::tree(vec![TreeEntry {
            name: "notes.md".into(),
            kind: Kind::File,
            id: file,
        }])
        .unwrap();
        let encoded = repo.encode_object(&tree.encode());
        assert!(
            encoded.len() < tree.encode().len() - 20,
            "{} bytes",
            encoded.len()
        );
        assert_eq!(
            repo.decode_object(&tree.id(), encoded).unwrap(),
            tree.encode()
        );
    }
}
