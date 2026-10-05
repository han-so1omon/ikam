//! Groups: small contents compressed together (a "compression region",
//! Shilane et al., FAST 2012).
//!
//! Many small contents each compressed alone pay a frame per content and
//! lose the context their neighbours would give. A group is ordinary content,
//! the concatenation of similar small contents, stored in a plain encoding.
//! Each member's stored entry becomes `S group[8] start len`: its bytes are
//! that range of the group. A member's id is still checked on every read,
//! and a group prefix shared by later objects costs a retry, never exactness.
//! Groups are formed by repack's re-plans from contents that resemble each
//! other (shared seeds), and kept only if smaller than the members stored
//! separately; gc keeps a group while a member names it.

use std::collections::{HashMap, HashSet};

use crate::dict::framer;
use crate::{Error, Id, Repo, Store, matcher};

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

/// A group ready to write: its id and encoding, its members' entries, and
/// the bytes it saves.
struct Planned {
    group: Id,
    encoded: Vec<u8>,
    entries: Vec<(Id, Vec<u8>)>,
    saved: usize,
}

/// Groups of consecutive items in byte order, each up to `GROUP_SIZE`.
fn byte_order_groups(items: &[(Vec<u8>, Id, usize)]) -> Vec<Vec<usize>> {
    let (mut groups, mut start) = (Vec::new(), 0);
    while start < items.len() {
        let (mut end, mut size) = (start, 0);
        while end < items.len() && (end == start || size + items[end].0.len() <= GROUP_SIZE) {
            size += items[end].0.len();
            end += 1;
        }
        groups.push((start..end).collect());
        start = end;
    }
    groups
}

/// Groups of item indices, each up to `GROUP_SIZE` bytes. A group starts
/// from the first unplaced item (in byte order) and grows by the unplaced
/// item sharing the most content-defined seeds with the group so far (the
/// matcher's anchors: a MinHash-like resemblance), falling back to byte
/// order. Deterministic: ties go to byte order.
///
/// With `measure` (the dictionary to compress with), the seed scores only
/// shortlist `SHORTLIST` candidates, and the next member is the one whose
/// bytes the group compresses best: the largest saving `C(x) - (C(G x) -
/// C(G))`, measured with the kernel's own zstd and dictionary (a
/// compression-native resemblance, after Cilibrasi and Vitanyi's NCD).
fn similar_groups(items: &[(Vec<u8>, Id, usize)], measure: Option<&[u8]>) -> Vec<Vec<usize>> {
    let mut zstd = measure.and_then(framer);
    let mut cost = |b: &[u8]| {
        zstd.as_mut()
            .map_or(0, |z| z.compress(b).map_or(b.len(), |c| c.len()))
    };
    let alone: Vec<usize> = items.iter().map(|(b, _, _)| cost(b)).collect();
    let seeds: Vec<HashSet<u64>> = items.iter().map(|(b, _, _)| seed_keys(b)).collect();
    let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, s) in seeds.iter().enumerate() {
        s.iter().for_each(|k| index.entry(*k).or_default().push(i));
    }
    let (mut placed, mut groups) = (vec![false; items.len()], Vec::new());
    while let Some(first) = placed.iter().position(|p| !p) {
        let (mut group, mut size, mut seen) = (Vec::new(), 0, HashSet::new());
        let (mut score, mut joined) = (vec![0usize; items.len()], Vec::new());
        let mut next = Some(first);
        while let Some(i) = next {
            placed[i] = true;
            group.push(i);
            size += items[i].0.len();
            if measure.is_some() {
                joined.extend_from_slice(&items[i].0);
            }
            for k in &seeds[i] {
                if seen.insert(*k) {
                    index[k].iter().for_each(|&j| score[j] += 1);
                }
            }
            let fits = |j: usize| !placed[j] && size + items[j].0.len() <= GROUP_SIZE;
            let mut ranked: Vec<usize> = (0..items.len()).filter(|&j| fits(j)).collect();
            ranked.sort_by_key(|&j| (std::cmp::Reverse(score[j]), j));
            next = match measure {
                None => ranked.first().copied(),
                Some(_) => {
                    let base = cost(&joined);
                    ranked.truncate(SHORTLIST);
                    // Largest saving; ties to the seed ranking.
                    let mut saving = |j: usize| {
                        let with = cost(&[&joined[..], &items[j].0].concat());
                        alone[j] as isize - (with as isize - base as isize)
                    };
                    let scored: Vec<(isize, usize)> =
                        ranked.iter().map(|&j| (saving(j), j)).collect();
                    scored.iter().rev().max_by_key(|s| s.0).map(|s| s.1)
                }
            };
        }
        groups.push(group);
    }
    groups
}

/// Candidates measured per step when growing a group by compressed cost.
const SHORTLIST: usize = 8;

/// The matcher's seed keys of `bytes`.
fn seed_keys(bytes: &[u8]) -> HashSet<u64> {
    matcher::anchors(bytes)
        .map(|p| matcher::seed_key(&bytes[p..p + matcher::SEED]))
        .collect()
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
        // Proposers suggest groupings; the one that saves most is written.
        let mut best: (usize, Vec<Planned>) = (0, Vec::new());
        let dict = self
            .dictionary()
            .map(|(_, d)| d.clone())
            .unwrap_or_default();
        let proposals = [
            byte_order_groups(&items),
            similar_groups(&items, None),
            similar_groups(&items, Some(&dict)),
        ];
        for proposal in proposals {
            let plans: Vec<Planned> = proposal
                .iter()
                .filter_map(|g| self.plan_group(&g.iter().map(|&i| &items[i]).collect::<Vec<_>>()))
                .collect();
            let saved = plans.iter().map(|p| p.saved).sum();
            if saved > best.0 {
                best = (saved, plans);
            }
        }
        for p in best.1 {
            self.store.write(p.group, &p.encoded)?;
            for (id, entry) in p.entries {
                self.store.replace(id, &entry)?;
            }
        }
        self.reset_projections();
        Ok(())
    }

    /// The group of `members` and their entries, if it saves bytes.
    fn plan_group(&self, members: &[&(Vec<u8>, Id, usize)]) -> Option<Planned> {
        if members.len() < 2 {
            return None;
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
        let alone: usize = members.iter().map(|m| m.2).sum();
        (grouped < alone && !self.store.has(&group)).then_some(Planned {
            group,
            encoded,
            entries,
            saved: alone - grouped,
        })
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
