//! Templates: lossless anti-unification of similar contents.
//!
//! Two similar contents become one shared *template* (the segments they
//! have in common, in order) plus per-content *fillers* (whatever sits in
//! each hole). `fill(template, fillers)` rebuilds each one exactly.
//! Templates and fillers are ordinary content, so they dedup, appear in the
//! graph, and can themselves be templated again.
//!
//! Boundaries come from a byte-level alignment, not from any format rule.
//! An equal run becomes a segment only if it is longer than a hole costs (a
//! length header in the template and in every filler list); shorter runs
//! stay in the fillers.
//!
//! The alignment is anchored: exact matches at content-defined seeds
//! (extended byte-by-byte) anchor it in linear time even when most bytes
//! differ, and Myers runs only inside small gaps between anchors, where
//! short equal runs (a shared digit, a closing tag) hide. Contents dominated
//! by unique payload can therefore still share their boilerplate, without
//! paying for a full diff.
//!
//! Encoding of both template and fillers: count:u32 { len:u32 bytes }.
//! A template with m segments takes m + 1 fillers:
//! output = f0 s1 f1 s2 ... sm fm.

use std::collections::HashMap;

use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::Error;
use crate::matcher::{SEED, anchors, common_prefix, common_suffix, seed_key};

/// Two u32 length headers: what a hole costs in the template plus in one
/// filler list. Equal runs no longer than this are cheaper left in fillers.
const HOLE_COST: usize = 8;
/// Alignment work budget: gap bytes diffed per alignment. A count, not a
/// clock, so planning (and therefore every stored size) is deterministic
/// regardless of machine load. Correctness never depends on it: every
/// result is verified.
const ALIGN_WORK: usize = 256 * 1024;
/// Gaps between anchors larger than this (either side) are unique payload,
/// not worth a diff: they stay holes.
const GAP_DIFF: usize = 1024;
/// Positions remembered per seed when indexing one side.
const SEED_POSITIONS: usize = 8;

/// Equal runs `(a_pos, b_pos, len)` between `a` and `b`, increasing in both.
pub(crate) fn align(a: &[u8], b: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut budget = ALIGN_WORK;
    let mut seeds: HashMap<u64, Vec<usize>> = HashMap::new();
    for p in anchors(a) {
        let slot = seeds.entry(seed_key(&a[p..p + SEED])).or_default();
        if slot.len() < SEED_POSITIONS {
            slot.push(p);
        }
    }
    let (mut runs, mut ca, mut cb) = (Vec::new(), 0, 0);
    for q in anchors(b) {
        if q < cb {
            continue;
        }
        let Some(ps) = seeds.get(&seed_key(&b[q..q + SEED])) else {
            continue;
        };
        let best = ps
            .iter()
            .filter(|&&p| p >= ca && a[p..p + SEED] == b[q..q + SEED])
            .map(|&p| {
                let back = common_suffix(&b[cb..q], &a[ca..p]);
                (p - back, q - back, back + common_prefix(&b[q..], &a[p..]))
            })
            .max_by_key(|&(_, _, len)| len);
        if let Some((pa, pb, len)) = best {
            diff_gap(a, b, (ca, pa), (cb, pb), &mut budget, &mut runs);
            runs.push((pa, pb, len));
            (ca, cb) = (pa + len, pb + len);
        }
    }
    diff_gap(a, b, (ca, a.len()), (cb, b.len()), &mut budget, &mut runs);
    runs
}

/// Myers inside one small gap between anchors, appending its equal runs.
fn diff_gap(
    a: &[u8],
    b: &[u8],
    (a0, a1): (usize, usize),
    (b0, b1): (usize, usize),
    budget: &mut usize,
    runs: &mut Vec<(usize, usize, usize)>,
) {
    let work = (a1 - a0) + (b1 - b0);
    if a0 >= a1 || b0 >= b1 || a1 - a0 > GAP_DIFF || b1 - b0 > GAP_DIFF || work > *budget {
        return;
    }
    *budget -= work;
    for op in capture_diff_slices(Algorithm::Myers, &a[a0..a1], &b[b0..b1]) {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            runs.push((a0 + old_index, b0 + new_index, len));
        }
    }
}

/// Bytes `a` and `b` share in runs long enough to become segments.
pub(crate) fn shared(a: &[u8], b: &[u8]) -> usize {
    align(a, b)
        .iter()
        .map(|r| r.2)
        .filter(|&len| len > HOLE_COST)
        .sum()
}

pub(crate) fn encode_parts<P: AsRef<[u8]>>(parts: &[P]) -> Vec<u8> {
    let mut out = (parts.len() as u32).to_be_bytes().to_vec();
    for p in parts {
        out.extend_from_slice(&(p.as_ref().len() as u32).to_be_bytes());
        out.extend_from_slice(p.as_ref());
    }
    out
}

pub(crate) fn decode_parts(mut b: &[u8]) -> Option<Vec<&[u8]>> {
    let mut take = |n: usize| -> Option<&[u8]> {
        let (head, tail) = b.split_at_checked(n)?;
        b = tail;
        Some(head)
    };
    let count = u32::from_be_bytes(take(4)?.try_into().ok()?) as usize;
    let mut parts = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        let len = u32::from_be_bytes(take(4)?.try_into().ok()?) as usize;
        parts.push(take(len)?);
    }
    b.is_empty().then_some(parts)
}

/// Builtin `fill`: args are `[template, fillers]`.
pub fn fill(args: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
    let bad = || Error::Exec("fill: malformed template or fillers".into());
    let [template, fillers] = args else {
        return Err(bad());
    };
    let (segments, fillers) = (
        decode_parts(template).ok_or_else(bad)?,
        decode_parts(fillers).ok_or_else(bad)?,
    );
    if fillers.len() != segments.len() + 1 {
        return Err(bad());
    }
    let mut out = fillers[0].to_vec();
    for (s, f) in segments.iter().zip(&fillers[1..]) {
        out.extend_from_slice(s);
        out.extend_from_slice(f);
    }
    Ok(out)
}

/// Anti-unify `a` and `b`: `(template, fillers_a, fillers_b)`, or `None`
/// if they share no segment worth a hole.
pub(crate) fn anti_unify(a: &[u8], b: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let (mut segments, mut fa, mut fb) = (Vec::new(), Vec::new(), Vec::new());
    let (mut ca, mut cb) = (0, 0);
    for (old_index, new_index, len) in align(a, b) {
        if len > HOLE_COST {
            fa.push(&a[ca..old_index]);
            fb.push(&b[cb..new_index]);
            segments.push(&a[old_index..old_index + len]);
            (ca, cb) = (old_index + len, new_index + len);
        }
    }
    if segments.is_empty() {
        return None;
    }
    fa.push(&a[ca..]);
    fb.push(&b[cb..]);
    Some((
        encode_parts(&segments),
        encode_parts(&fa),
        encode_parts(&fb),
    ))
}

/// Fillers that make `template` reproduce `x`, finding each segment in
/// order (first occurrence). `None` if some segment does not occur.
pub(crate) fn fit(template: &[u8], x: &[u8]) -> Option<Vec<u8>> {
    let segments = decode_parts(template)?;
    let (mut fillers, mut pos) = (Vec::with_capacity(segments.len() + 1), 0);
    for s in segments {
        let at = memchr::memmem::find(&x[pos..], s)?;
        fillers.push(&x[pos..pos + at]);
        pos += at + s.len();
    }
    fillers.push(&x[pos..]);
    Some(encode_parts(&fillers))
}

/// Generalize `template` so that `x` fits too: keep only the parts of its
/// segments that `x` shares, in order. Every content that fitted the old
/// template still fits the new one, since each new segment is a substring
/// of an old one and order is kept.
pub(crate) fn generalize(template: &[u8], x: &[u8]) -> Option<Vec<u8>> {
    let segments = decode_parts(template)?;
    let (mut joined, mut bounds) = (Vec::new(), Vec::new());
    for s in &segments {
        bounds.push(joined.len());
        joined.extend_from_slice(s);
    }
    bounds.push(joined.len());
    let mut kept = Vec::new();
    for (old_index, _, len) in align(&joined, x) {
        let mut start = old_index;
        // Split runs at old segment boundaries, so holes are never removed.
        for &b in bounds
            .iter()
            .filter(|&&b| b > old_index && b < old_index + len)
            .chain([&(old_index + len)])
        {
            if b - start > HOLE_COST {
                kept.push(&joined[start..b]);
            }
            start = b;
        }
    }
    (!kept.is_empty()).then(|| encode_parts(&kept))
}

/// One template for a group of similar contents, generalized over all.
pub(crate) fn induce(members: &[&[u8]]) -> Option<Vec<u8>> {
    let (first, rest) = members.split_first()?;
    let (second, rest) = rest.split_first()?;
    let mut t = anti_unify(first, second)?.0;
    for m in rest {
        t = generalize(&t, m)?;
    }
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn similar_pair() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
        (
            prop::collection::vec(any::<u8>(), 0..600),
            prop::collection::vec(
                (0..600usize, prop::collection::vec(any::<u8>(), 0..20)),
                0..8,
            ),
        )
            .prop_map(|(a, edits)| {
                let mut b = a.clone();
                for (at, ins) in edits {
                    let at = at.min(b.len());
                    b.splice(at..at, ins);
                }
                (a, b)
            })
    }

    proptest! {
        #[test]
        fn anti_unification_is_lossless((a, b) in similar_pair()) {
            if let Some((t, fa, fb)) = anti_unify(&a, &b) {
                prop_assert_eq!(fill(&[t.clone(), fa]).unwrap(), a);
                prop_assert_eq!(fill(&[t, fb]).unwrap(), b);
            }
        }

        #[test]
        fn fit_is_lossless((a, b) in similar_pair(), c in prop::collection::vec(any::<u8>(), 0..600)) {
            if let Some((t, _, _)) = anti_unify(&a, &b) {
                for x in [&a, &b, &c] {
                    if let Some(f) = fit(&t, x) {
                        prop_assert_eq!(&fill(&[t.clone(), f]).unwrap(), x);
                    }
                }
            }
        }

        #[test]
        fn fill_never_panics(t in prop::collection::vec(any::<u8>(), 0..64), f in prop::collection::vec(any::<u8>(), 0..64)) {
            let _ = fill(&[t, f]);
        }
    }

    #[test]
    fn induced_templates_fit_every_member() {
        let doc = |v: &str| {
            format!(
                "<row>header text shared by all</row><v>{v}</v><row>footer text shared by all</row>"
            )
            .into_bytes()
        };
        let docs: Vec<Vec<u8>> = ["17", "1", "29", "7"].iter().map(|v| doc(v)).collect();
        let refs: Vec<&[u8]> = docs.iter().map(Vec::as_slice).collect();
        let pair = anti_unify(&docs[0], &docs[1]).unwrap().0;
        assert!(
            fit(&pair, &docs[2]).is_none(),
            "a pair template overfits the shared leading '1'"
        );
        let t = induce(&refs).unwrap();
        for d in &docs {
            assert_eq!(&fill(&[t.clone(), fit(&t, d).unwrap()]).unwrap(), d);
        }
    }

    #[test]
    fn short_common_runs_stay_in_fillers() {
        let (a, b) = (b"<v>1</v><v>2</v>".to_vec(), b"<v>3</v><v>4</v>".to_vec());
        assert!(
            anti_unify(&a, &b).is_none(),
            "no equal run is longer than a hole costs"
        );
    }
}
