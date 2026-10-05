//! Groups: small contents compressed together (a "compression region",
//! Shilane et al., FAST 2012).
//!
//! Many small contents each compressed alone pay a frame per content and
//! lose the context their neighbours would give. A group is ordinary content,
//! the concatenation of similar small contents, stored in a plain encoding.
//! Each member's stored entry becomes `S group[8] start len`: its bytes are
//! that range of the group. A member's id is still checked on every read,
//! and a group prefix shared by later objects costs a retry, never exactness.
//! Groups are formed by repack's re-plans and kept only if smaller than the
//! members stored separately; gc keeps a group while a member names it.

use crate::{Error, Id, Repo, Store};

/// Bytes of a group's id stored in each member entry.
pub(crate) const GROUP_REF_LEN: usize = 8;
/// Largest group, in uncompressed bytes. Reading a member decompresses its
/// group, so this bounds the read cost of a member.
const GROUP_SIZE: usize = 64 * 1024;

fn put_varint(out: &mut Vec<u8>, mut n: usize) {
    while n >= 0x80 {
        out.push(n as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn varint(b: &mut &[u8]) -> Option<usize> {
    let mut n = 0usize;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = b.split_first()?;
        *b = rest;
        n |= ((byte & 0x7f) as usize).checked_shl(shift)?;
        if byte < 0x80 {
            return Some(n);
        }
    }
    None
}

/// A member's stored entry.
fn member_entry(group: &Id, start: usize, len: usize) -> Vec<u8> {
    let mut out = [&b"S"[..], &group.as_bytes()[..GROUP_REF_LEN]].concat();
    put_varint(&mut out, start);
    put_varint(&mut out, len);
    out
}

impl<S: Store> Repo<S> {
    /// The bytes of member `id` from its entry `S group start len`: the first
    /// stored group the prefix names whose range hashes to `id`.
    pub(crate) fn read_member(&self, id: &Id, encoded: &[u8]) -> Result<Vec<u8>, Error> {
        let bad = || Error::Corrupt(*id);
        let prefix = encoded.get(1..1 + GROUP_REF_LEN).ok_or_else(bad)?;
        let mut rest = &encoded[1 + GROUP_REF_LEN..];
        let (start, len) = (
            varint(&mut rest).ok_or_else(bad)?,
            varint(&mut rest).ok_or_else(bad)?,
        );
        if !rest.is_empty() {
            return Err(bad());
        }
        for group in self.store.ids_with_prefix(prefix)? {
            let stored = self.store.read(&group)?;
            // A group is never itself a member, so decoding never chains.
            if stored.first() == Some(&b'S') {
                continue;
            }
            if let Ok(bytes) = self.decode_plain(&group, &stored)
                && let Some(range) = bytes.get(start..start.saturating_add(len))
                && Id::of_content(range) == *id
            {
                return Ok(range.to_vec());
            }
        }
        Err(bad())
    }

    /// Group the small contents stored on their own: sorted by bytes, so
    /// similar ones are neighbours, then packed up to `GROUP_SIZE`. A group is
    /// written only if it and its member entries are smaller than the
    /// members' current encodings.
    pub(crate) fn group_small(&mut self) -> Result<(), Error> {
        let dict = self.dictionary().map(|(id, _)| *id);
        let mut items: Vec<(Vec<u8>, Id, usize)> = Vec::new();
        for id in self.store.ids()? {
            let stored = self.store.read(&id)?;
            if matches!(stored.first(), Some(b'B' | b'Z' | b'Y')) && Some(id) != dict {
                let bytes = self.decode_plain(&id, &stored)?;
                if !bytes.is_empty() && bytes.len() < GROUP_SIZE {
                    items.push((bytes, id, stored.len()));
                }
            }
        }
        items.sort();
        let mut start = 0;
        while start < items.len() {
            let (mut end, mut size) = (start, 0);
            while end < items.len() && (end == start || size + items[end].0.len() <= GROUP_SIZE) {
                size += items[end].0.len();
                end += 1;
            }
            self.write_group(&items[start..end])?;
            start = end;
        }
        self.reset_projections();
        Ok(())
    }

    fn write_group(&mut self, members: &[(Vec<u8>, Id, usize)]) -> Result<(), Error> {
        if members.len() < 2 {
            return Ok(());
        }
        let joined: Vec<u8> = members.iter().flat_map(|m| m.0.iter().copied()).collect();
        let group = Id::of_content(&joined);
        let encoded = self.encode_plain(&joined);
        let (mut entries, mut at) = (Vec::new(), 0);
        for (bytes, id, _) in members {
            entries.push((*id, member_entry(&group, at, bytes.len())));
            at += bytes.len();
        }
        let grouped = encoded.len() + entries.iter().map(|e| e.1.len()).sum::<usize>();
        if grouped >= members.iter().map(|m| m.2).sum() || self.store.has(&group) {
            return Ok(());
        }
        self.store.write(group, &encoded)?;
        for (id, entry) in entries {
            self.store.replace(id, &entry)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, MemStore, Object, TreeEntry};

    #[test]
    fn varints_round_trip() {
        for n in [0, 1, 127, 128, 300, 1 << 20, usize::MAX >> 1] {
            let mut b = Vec::new();
            put_varint(&mut b, n);
            assert_eq!(varint(&mut b.as_slice()), Some(n));
        }
    }

    /// Small similar contents are grouped, read back exactly through their
    /// entries (also past a decoy under the group's prefix), and the group
    /// survives gc while its members are live.
    #[test]
    fn grouped_members_read_back_and_survive_gc() {
        let mut repo = Repo::new(MemStore::default());
        let docs: Vec<Vec<u8>> = (0..40)
            .map(|i| {
                format!(
                    "# Note {i}\n\nShared boilerplate for every note, item {}.\n",
                    i * 7
                )
                .into_bytes()
            })
            .collect();
        let entries: Vec<TreeEntry> = docs
            .iter()
            .enumerate()
            .map(|(i, d)| TreeEntry {
                name: format!("n{i:02}.md"),
                kind: Kind::File,
                id: repo.put_content(d).unwrap().id,
            })
            .collect();
        let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
        repo.commit("main", tree, "").unwrap();
        let before: usize = docs
            .iter()
            .map(|d| repo.store().read(&Id::of_content(d)).unwrap().len())
            .sum();
        repo.group_small().unwrap();
        let entry = repo.store().read(&Id::of_content(&docs[0])).unwrap();
        assert_eq!(entry[0], b'S', "small similar contents are grouped");
        let group = repo
            .store()
            .ids_with_prefix(&entry[1..1 + GROUP_REF_LEN])
            .unwrap()[0];
        let mut decoy = *group.as_bytes();
        decoy[31] ^= 1;
        repo.store
            .write(Id::from_bytes(decoy), b"Bnot the group")
            .unwrap();
        repo.gc().unwrap();
        for d in &docs {
            assert_eq!(&repo.read_content(&Id::of_content(d)).unwrap(), d);
        }
        assert!(repo.store().has(&group), "gc keeps the group");
        // fsck flags exactly the forged decoy (its bytes do not match its id).
        let bad: Vec<Id> = repo.fsck().unwrap().into_iter().map(|(id, _)| id).collect();
        assert_eq!(bad, [Id::from_bytes(decoy)]);
        let after: usize = docs
            .iter()
            .map(|d| repo.store().read(&Id::of_content(d)).unwrap().len())
            .sum::<usize>()
            + repo.store().read(&group).unwrap().len();
        assert!(after < before, "{after} >= {before}");
    }
}
