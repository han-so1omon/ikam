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

use crate::{Error, Id, Repo, Store};

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
