//! E453: `Repo::net_chunk_policy` picks a net's chunk bounds from its size
//! and the read weight when the net is first written, and every later
//! version is cut with the same bounds: by whole and by delta writes, in
//! the Tree and Inline layouts, across handles through the tree that holds
//! the net, and never for the flat layouts. Off by default.

use ikam_kernel::net::{Arc, Kind, Layout, Net, NetChunkPolicy, Place, Transition};
use ikam_kernel::{ChunkSizes, Id, MemStore, Repo, SharedStore};

const BIG: ChunkSizes = ChunkSizes {
    leaf: (16384, 131072),
    parent: (16384, 131072),
};
const SMALL: ChunkSizes = ChunkSizes {
    leaf: (512, 4096),
    parent: (1024, 8192),
};

fn arc(place: &str, weight: u32) -> Arc {
    Arc {
        place: place.into(),
        weight,
    }
}

/// A synthetic net of `n` transitions, with the shapes a line can take:
/// colored places, initial tokens, weights, capabilities, a source and a
/// sink transition, names with `-`, `_` and `.`.
fn synth(n: usize) -> Net {
    let np = (n / 2).max(6);
    let kinds = [Kind::Pure, Kind::Cmd, Kind::Agent, Kind::Human];
    let name = |i: usize| match i % 3 {
        0 => format!("p{i}"),
        1 => format!("p-{i}."),
        _ => format!("p_{i}-"),
    };
    Net {
        name: "s-1.x".into(),
        version: 1,
        places: (0..np)
            .map(|i| Place {
                name: name(i),
                colored: i % 4 == 3,
                initial: if i % 4 == 3 { 0 } else { (i % 5) as u32 * 10 },
            })
            .collect(),
        transitions: (0..n)
            .map(|i| {
                let (a, b, c) = ((i * 7 + 1) % np, (i * 3 + 2) % np, (i * 5 + 4) % np);
                let kind = kinds[i % 4];
                let (inputs, outputs) = match i % 7 {
                    0 => (vec![], vec![arc(&name(b), 1)]),
                    1 => (vec![arc(&name(a), 3)], vec![]),
                    _ => (
                        vec![arc(&name(a), 1 + (i % 3) as u32), arc(&name(c), 1)],
                        vec![arc(&name(b), 1), arc(&name(c), 12)],
                    ),
                };
                Transition {
                    name: format!("t{i}"),
                    inputs,
                    outputs,
                    kind,
                    capability: (kind == Kind::Cmd).then(|| "run.x-1".to_string()),
                }
            })
            .collect(),
    }
}

/// Version `k`: rename, reweight, insert, remove a place, burst, move a
/// transition, change the net's name or nothing but the version.
fn edit(net: &mut Net, k: usize) {
    net.version = k as u32;
    let pick = |len: usize| (k * 7919) % len.max(1);
    match k % 7 {
        0 => {
            let j = pick(net.transitions.len());
            net.transitions[j].name += &format!("r{k}");
        }
        1 => {
            let j = pick(net.transitions.len());
            let t = &mut net.transitions[j];
            match t.outputs.first_mut() {
                Some(a) => a.weight += 1,
                None => t.inputs[0].weight += 1,
            }
        }
        2 => {
            let place = format!("q{k}");
            net.places.insert(
                pick(net.places.len() + 1),
                Place {
                    name: place.clone(),
                    colored: false,
                    initial: 0,
                },
            );
            let at = pick(net.transitions.len() + 1);
            net.transitions.insert(
                at,
                Transition {
                    name: format!("n{k}"),
                    inputs: vec![],
                    outputs: vec![arc(&place, 2)],
                    kind: Kind::Pure,
                    capability: None,
                },
            );
        }
        3 => {
            let gone = net.places.remove(1 + pick(net.places.len() - 1)).name;
            let keep = net.places[0].name.clone();
            for t in &mut net.transitions {
                for side in [&mut t.inputs, &mut t.outputs] {
                    for a in side.iter_mut().filter(|a| a.place == gone) {
                        a.place = keep.clone();
                    }
                }
            }
        }
        4 => {
            let m = (net.transitions.len() / 2).clamp(1, 20);
            let j = pick(net.transitions.len() - m + 1);
            for t in &mut net.transitions[j..j + m] {
                t.kind = match t.kind {
                    Kind::Human => Kind::Pure,
                    k => k,
                };
            }
            net.transitions[j].capability = None;
            net.transitions[j].kind = Kind::Agent;
        }
        5 => {
            // Move a transition (and the first place) to the end.
            let t = net.transitions.remove(pick(net.transitions.len()));
            net.transitions.push(t);
            let p = net.places.remove(0);
            net.places.push(p);
        }
        _ => {}
    }
    net.check().unwrap();
}

fn chain(n: usize, versions: usize) -> Vec<Net> {
    let mut nets = vec![synth(n)];
    for k in 2..=versions {
        let mut next = nets.last().unwrap().clone();
        edit(&mut next, k);
        nets.push(next);
    }
    nets
}

/// A policy that gives `BIG` to a net of up to `most` bytes and `SMALL`
/// at the heavy weight, and the handle's bounds otherwise.
fn policy(most: usize) -> NetChunkPolicy {
    NetChunkPolicy {
        steps: vec![(most, BIG)],
        heavy_from: 10.0,
        heavy: vec![(usize::MAX, SMALL)],
    }
}

/// `net` with `extra` more transitions at the end.
fn grown(net: &Net, extra: usize, k: usize) -> Net {
    let mut out = net.clone();
    for i in 0..extra {
        out.transitions.push(Transition {
            name: format!("g{k}x{i}"),
            inputs: vec![arc(&net.places[i % net.places.len()].name, 1)],
            outputs: vec![arc(&net.places[(i * 3) % net.places.len()].name, 2)],
            kind: Kind::Pure,
            capability: None,
        });
    }
    out.check().unwrap();
    out
}

/// A chain whose first version fits the policy's step in `layout` and
/// whose later versions outgrow it, so a pick by the current size would
/// change; and the step's bytes.
fn growing_chain(layout: Layout) -> (Vec<Net>, usize) {
    let mut nets = chain(1500, 6);
    let most = NetChunkPolicy::net_bytes(&nets[0], layout) + 64;
    for (k, net) in nets.iter_mut().enumerate().skip(1) {
        *net = grown(net, 600 * k, k);
        assert!(NetChunkPolicy::net_bytes(net, layout) > most);
    }
    // The bounds give v1 other roots, so which bounds a version was cut
    // with shows in its root.
    assert_ne!(
        root_with(&nets[0], layout, BIG),
        root_with(&nets[0], layout, ChunkSizes::default())
    );
    (nets, most)
}

/// The root a fresh handle gives `net` cut with `bounds`.
fn root_with(net: &Net, layout: Layout, bounds: ChunkSizes) -> Id {
    let mut repo = Repo::new(MemStore::default());
    repo.graph_chunks = bounds;
    repo.put_net_fragments(net, layout).unwrap()
}

#[test]
fn off_by_default() {
    let repo = Repo::new(MemStore::default());
    assert!(repo.net_chunk_policy.is_none());
    let nets = chain(400, 3);
    for layout in [Layout::Tree, Layout::Inline] {
        let mut repo = Repo::new(MemStore::default());
        for net in &nets {
            let root = repo.put_net_fragments(net, layout).unwrap();
            assert_eq!(root, root_with(net, layout, ChunkSizes::default()));
        }
        assert_eq!(repo.net_chunk_bounds(&nets[0].name), None);
    }
}

#[test]
fn the_policy_picks_by_bytes_and_read_weight() {
    let p = NetChunkPolicy {
        steps: vec![(1000, BIG), (5000, SMALL)],
        heavy_from: 20.0,
        heavy: vec![(usize::MAX, ChunkSizes::uniform(256, 2048))],
    };
    let d = ChunkSizes::default();
    assert_eq!(p.pick(0, 0.001, d), BIG);
    assert_eq!(p.pick(1000, 0.001, d), BIG);
    assert_eq!(p.pick(1001, 0.001, d), SMALL);
    assert_eq!(p.pick(5000, 19.9, d), SMALL);
    assert_eq!(p.pick(5001, 0.001, d), d);
    assert_eq!(p.pick(5001, 20.0, d), ChunkSizes::uniform(256, 2048));
    assert_eq!(p.pick(10, 48.8, d), ChunkSizes::uniform(256, 2048));
    assert_eq!(
        p.candidates(d),
        vec![BIG, SMALL, ChunkSizes::uniform(256, 2048), d]
    );
    assert_eq!(
        NetChunkPolicy::fixed(SMALL).pick(usize::MAX, 99.0, d),
        SMALL
    );
    // E453's preset: leaves of a sixth of the net, 4 to 8 KiB, up to 1 MiB.
    let by_size = NetChunkPolicy::by_size();
    let leaf = |min: usize| ChunkSizes {
        leaf: (min << 10, min << 13),
        parent: (16 << 10, 128 << 10),
    };
    assert_eq!(by_size.pick(10 << 10, 0.001, d), leaf(4));
    assert_eq!(by_size.pick(45 << 10, 0.001, d), leaf(7));
    assert_eq!(by_size.pick(45 << 10, 48.8, d), leaf(7));
    assert_eq!(by_size.pick(100 << 10, 0.001, d), leaf(8));
    assert_eq!(by_size.pick(1 << 20, 0.001, d), leaf(8));
    assert_eq!(by_size.pick((1 << 20) + 1, 0.001, d), d);
    assert_eq!(
        by_size.candidates(d),
        vec![leaf(4), leaf(5), leaf(6), leaf(7), leaf(8), d]
    );
    // The size is the nodes' encoded bytes, larger for a larger net.
    let (a, b) = (synth(100), synth(1000));
    for layout in [Layout::Tree, Layout::Inline] {
        assert!(NetChunkPolicy::net_bytes(&a, layout) > 0);
        assert!(NetChunkPolicy::net_bytes(&b, layout) > 5 * NetChunkPolicy::net_bytes(&a, layout));
    }
    assert!(
        NetChunkPolicy::net_bytes(&b, Layout::Tree) > NetChunkPolicy::net_bytes(&b, Layout::Inline)
    );
}

#[test]
fn a_net_keeps_the_bounds_it_was_first_written_with() {
    for layout in [Layout::Tree, Layout::Inline] {
        let (nets, most) = growing_chain(layout);
        for delta in [false, true] {
            let mut repo = Repo::new(MemStore::default());
            repo.net_delta_write = delta;
            repo.net_chunk_policy = Some(policy(most));
            let mut roots = Vec::new();
            for (i, net) in nets.iter().enumerate() {
                let root = repo.put_net_fragments(net, layout).unwrap();
                // Cut as a fresh whole write cuts it with v1's bounds, not
                // the bounds the policy gives the grown net.
                assert_eq!(
                    root,
                    root_with(net, layout, BIG),
                    "{layout:?} delta {delta} v{i}"
                );
                if i > 0 {
                    assert_ne!(root, root_with(net, layout, ChunkSizes::default()));
                }
                assert_eq!(repo.net_chunk_bounds(&net.name), Some(BIG));
                // The handle's own bounds are untouched.
                assert_eq!(repo.graph_chunks, ChunkSizes::default());
                roots.push(root);
            }
            // Changing the handle's bounds or the policy later changes
            // nothing for this net; a delta write edits with its bounds.
            repo.graph_chunks = SMALL;
            repo.net_chunk_policy = Some(NetChunkPolicy::fixed(SMALL));
            let mut next = nets.last().unwrap().clone();
            edit(&mut next, 9);
            assert_eq!(
                repo.put_net_fragments(&next, layout).unwrap(),
                root_with(&next, layout, BIG)
            );
            // Every version reads back, deltas apply, the store is sound.
            for (k, net) in nets.iter().enumerate() {
                assert_eq!(&repo.net_fragments(&roots[k]).unwrap(), net);
                if k > 0 {
                    let d = repo.net_delta(&roots[k - 1], &roots[k]).unwrap();
                    assert_eq!(&d.apply(&nets[k - 1]).unwrap(), net);
                }
            }
            assert!(repo.fsck().unwrap().is_empty());
        }
    }
}

#[test]
fn a_large_net_and_the_heavy_weight_get_their_own_bounds() {
    for layout in [Layout::Tree, Layout::Inline] {
        let (nets, most) = growing_chain(layout);
        let big = &nets[3];
        // Above every step: the handle's bounds, kept as well.
        let mut repo = Repo::new(MemStore::default());
        repo.net_delta_write = true;
        repo.net_chunk_policy = Some(policy(most));
        let root = repo.put_net_fragments(big, layout).unwrap();
        assert_eq!(root, root_with(big, layout, ChunkSizes::default()));
        assert_eq!(
            repo.net_chunk_bounds(&big.name),
            Some(ChunkSizes::default())
        );
        // At the heavy weight: the heavy steps, for later versions too.
        let mut heavy = Repo::new(MemStore::default());
        heavy.read_weight = 48.8;
        heavy.net_delta_write = true;
        heavy.net_chunk_policy = Some(policy(most));
        for net in &nets {
            let mut fresh = Repo::new(MemStore::default());
            fresh.read_weight = 48.8;
            fresh.graph_chunks = SMALL;
            let want = fresh.put_net_fragments(net, layout).unwrap();
            assert_eq!(heavy.put_net_fragments(net, layout).unwrap(), want);
        }
        assert_eq!(heavy.net_chunk_bounds(&big.name), Some(SMALL));
    }
}

#[test]
fn a_new_handle_finds_the_bounds_of_the_version_its_tree_holds() {
    for layout in [Layout::Tree, Layout::Inline] {
        let (nets, most) = growing_chain(layout);
        let store = SharedStore::new(MemStore::default());
        let path = "nets/s.net";
        let mut first = Repo::new(store.clone());
        first.net_delta_write = true;
        first.net_chunk_policy = Some(policy(most));
        let (mut tree, root) = first
            .put_net_fragment_file(None, path, &nets[0], layout)
            .unwrap();
        assert_eq!(root, root_with(&nets[0], layout, BIG));
        // A second handle on the same store, without the first's memory:
        // the grown net would pick the default bounds, but the tree holds a
        // version cut with BIG, so BIG is kept.
        for (k, net) in nets.iter().enumerate().skip(1) {
            let mut repo = Repo::new(store.clone());
            repo.net_delta_write = k % 2 == 0;
            repo.net_chunk_policy = Some(policy(most));
            let (t, root) = repo
                .put_net_fragment_file(Some(tree), path, net, layout)
                .unwrap();
            assert_eq!(root, root_with(net, layout, BIG), "{layout:?} v{k}");
            assert_eq!(repo.net_chunk_bounds(&net.name), Some(BIG));
            assert_eq!(&repo.net_fragment_file(&t, path).unwrap(), net);
            tree = t;
        }
        // A version this handle delta-wrote before the policy was set keeps
        // its bounds too.
        let mut before = Repo::new(MemStore::default());
        before.net_delta_write = true;
        before.put_net_fragments(&nets[0], layout).unwrap();
        before.net_chunk_policy = Some(NetChunkPolicy::fixed(BIG));
        assert_eq!(
            before.put_net_fragments(&nets[1], layout).unwrap(),
            root_with(&nets[1], layout, ChunkSizes::default())
        );
        // A history written without the policy keeps the default bounds
        // when a handle with the policy writes its next version.
        let shared = SharedStore::new(MemStore::default());
        let mut plain = Repo::new(shared.clone());
        let (t, _) = plain
            .put_net_fragment_file(None, path, &nets[0], layout)
            .unwrap();
        let mut plain = Repo::new(shared);
        plain.net_chunk_policy = Some(NetChunkPolicy::fixed(BIG));
        let (_, root) = plain
            .put_net_fragment_file(Some(t), path, &nets[1], layout)
            .unwrap();
        assert_eq!(root, root_with(&nets[1], layout, ChunkSizes::default()));
        // With no version at the path the policy picks.
        let mut fresh = Repo::new(MemStore::default());
        fresh.net_chunk_policy = Some(policy(most));
        let (_, root) = fresh
            .put_net_fragment_file(None, path, &nets[0], layout)
            .unwrap();
        assert_eq!(root, root_with(&nets[0], layout, BIG));
    }
}

#[test]
fn the_flat_layouts_ignore_the_policy() {
    let nets = chain(400, 3);
    for layout in [Layout::Flat, Layout::InlineFlat] {
        let mut plain = Repo::new(MemStore::default());
        let mut repo = Repo::new(MemStore::default());
        repo.net_chunk_policy = Some(NetChunkPolicy::fixed(SMALL));
        for net in &nets {
            assert_eq!(
                repo.put_net_fragments(net, layout).unwrap(),
                plain.put_net_fragments(net, layout).unwrap()
            );
        }
        assert_eq!(repo.net_chunk_bounds(&nets[0].name), None);
    }
}

#[test]
fn an_invalid_net_keeps_no_bounds() {
    let mut net = synth(50);
    net.transitions[0].inputs.push(arc("no-such-place", 1));
    let mut repo = Repo::new(MemStore::default());
    repo.net_chunk_policy = Some(NetChunkPolicy::fixed(SMALL));
    assert!(repo.put_net_fragments(&net, Layout::Inline).is_err());
    assert_eq!(repo.net_chunk_bounds(&net.name), None);
    assert_eq!(repo.graph_chunks, ChunkSizes::default());
}
