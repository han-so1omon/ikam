//! E448: a new version of a graph chunk (or tree) stored as a zstd frame
//! against the stored chunk it most resembles (`Repo::object_contexts`),
//! and the prolly tree's chunk size bounds (`Repo::graph_chunks`). Both
//! off by default.

use ikam_kernel::net::{Arc, Kind, Layout, Net, Place, Transition};
use ikam_kernel::{ChunkSizes, Edge, Id, MemStore, Node, Repo, SharedStore, Store, Target};

fn arc(place: &str, weight: u32) -> Arc {
    Arc {
        place: place.into(),
        weight,
    }
}

/// A synthetic net of `n` transitions over `n / 2` places.
fn synth(n: usize) -> Net {
    let np = (n / 2).max(4);
    Net {
        name: "s".into(),
        version: 1,
        places: (0..np)
            .map(|i| Place {
                name: format!("p{i}"),
                colored: false,
                initial: (i % 5 == 0) as u32 * 2,
            })
            .collect(),
        transitions: (0..n)
            .map(|i| Transition {
                name: format!("t{i}"),
                inputs: vec![arc(&format!("p{}", (i * 7 + 1) % np), 1 + (i % 3) as u32)],
                outputs: vec![arc(&format!("p{}", (i * 3 + 2) % np), 1)],
                kind: [Kind::Pure, Kind::Cmd, Kind::Agent, Kind::Human][i % 4],
                capability: None,
            })
            .collect(),
    }
}

/// `versions` versions: a weight changed, and every other version a
/// transition added.
fn chain(n: usize, versions: usize) -> Vec<Net> {
    let mut out = vec![synth(n)];
    for k in 2..=versions {
        let mut next = out.last().unwrap().clone();
        next.version = k as u32;
        let j = (k * 7919) % next.transitions.len();
        next.transitions[j].inputs[0].weight += 1;
        if k % 2 == 0 {
            let at = (k * 104_729) % next.transitions.len();
            next.transitions.insert(
                at,
                Transition {
                    name: format!("n{k}"),
                    inputs: vec![arc("p0", 1)],
                    outputs: vec![arc("p1", 2)],
                    kind: Kind::Pure,
                    capability: None,
                },
            );
        }
        next.check().unwrap();
        out.push(next);
    }
    out
}

fn stored(repo: &Repo<impl Store>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|i| s.read(i).unwrap().len())
        .sum()
}

/// Contents stored as a frame against another ("Q").
fn frames(repo: &Repo<impl Store>) -> usize {
    let s = repo.store();
    (s.ids().unwrap().iter())
        .filter(|i| s.read(i).unwrap().first() == Some(&b'Q'))
        .count()
}

/// Commit each version at `nets/s.net`; returns the commits and roots.
fn commit_all<S: Store>(repo: &mut Repo<S>, nets: &[Net]) -> (Vec<Id>, Vec<Id>, Vec<usize>) {
    let (mut tree, mut commits, mut roots, mut adds) = (None, Vec::new(), Vec::new(), Vec::new());
    for net in nets {
        let before = stored(repo);
        let (t, root) = repo
            .put_net_fragment_file(tree, "nets/s.net", net, Layout::Tree)
            .unwrap();
        adds.push(stored(repo) - before);
        tree = Some(t);
        roots.push(root);
        commits.push(repo.commit("main", t, "v").unwrap());
    }
    (commits, roots, adds)
}

#[test]
fn frames_store_versions_in_fewer_bytes_and_read_back() {
    let nets = chain(2000, 8);
    let mut plain = Repo::new(MemStore::default());
    let (_, plain_roots, plain_adds) = commit_all(&mut plain, &nets);
    assert_eq!(frames(&plain), 0, "off by default: ingest writes no frame");
    let mut repo = Repo::new(MemStore::default());
    repo.object_contexts = true;
    let (commits, roots, adds) = commit_all(&mut repo, &nets);
    // The same versions, by the same ids: only how chunks are stored differs.
    assert_eq!(roots, plain_roots);
    assert!(frames(&repo) >= 7, "{} frames", frames(&repo));
    let (with, without): (usize, usize) = (adds[1..].iter().sum(), plain_adds[1..].iter().sum());
    assert!(
        (with as f64) < 0.8 * without as f64,
        "later versions add {with} B with frames, {without} B without"
    );
    for (k, c) in commits.iter().enumerate() {
        assert_eq!(repo.net_fragment_file(c, "nets/s.net").unwrap(), nets[k]);
    }
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.fsck_deep().unwrap().is_empty());
    assert_eq!(repo.net_fragments(&roots[7]).unwrap(), nets[7]);
}

#[test]
fn a_frame_is_one_step_from_a_base_stored_whole() {
    let nets = chain(2000, 10);
    let mut repo = Repo::new(MemStore::default());
    repo.object_contexts = true;
    commit_all(&mut repo, &nets);
    let s = repo.store();
    for id in s.ids().unwrap() {
        let e = s.read(&id).unwrap();
        if e[0] == b'Q' {
            let base = Id::from_bytes(e[1..33].try_into().unwrap());
            let b = s.read(&base).unwrap();
            assert!(!matches!(b[0], b'Q' | b'M' | b'S'), "a chained base");
        }
    }
}

#[test]
fn deltas_skip_shared_chunks_and_apply_exactly_over_frames() {
    let nets = chain(3000, 6);
    let mut repo = Repo::new(MemStore::default());
    repo.object_contexts = true;
    let (_, roots, _) = commit_all(&mut repo, &nets);
    for k in 1..nets.len() {
        let d = repo.net_delta(&roots[k - 1], &roots[k]).unwrap();
        let full = repo
            .net_delta_with(&roots[k - 1], &roots[k], false)
            .unwrap();
        assert_eq!(d.transitions, full.transitions);
        assert!(
            d.chunks_read * 3 < full.chunks_read,
            "{} of {}",
            d.chunks_read,
            full.chunks_read
        );
        assert_eq!(d.apply(&nets[k - 1]).unwrap(), nets[k]);
    }
}

#[test]
fn gc_keeps_a_frames_base_when_only_the_frame_is_live() {
    let nets = chain(2000, 3);
    let shared = SharedStore::new(MemStore::default());
    let mut repo = Repo::new(shared.clone());
    repo.object_contexts = true;
    // Version 1 on one branch, version 3 on another: version 3's chunks
    // are framed against version 1's, which only the old branch names.
    let (t1, _) = repo
        .put_net_fragment_file(None, "nets/s.net", &nets[0], Layout::Tree)
        .unwrap();
    let old = repo.commit("old", t1, "v1").unwrap();
    let (t3, root3) = repo
        .put_net_fragment_file(None, "nets/s.net", &nets[2], Layout::Tree)
        .unwrap();
    repo.commit("new", t3, "v3").unwrap();
    assert!(frames(&repo) > 0);
    let mut handle = shared.clone();
    handle.delete_ref("old", old).unwrap();
    let deleted = repo.gc().unwrap();
    assert!(deleted > 0, "version 1's own objects go");
    let fresh = Repo::new(shared.clone());
    assert_eq!(fresh.net_fragments(&root3).unwrap(), nets[2]);
    assert!(fresh.fsck().unwrap().is_empty());
}

#[test]
fn repack_after_frames_reads_back_and_keeps_frames() {
    let nets = chain(1000, 6);
    let mut repo = Repo::new(MemStore::default());
    repo.object_contexts = true;
    let (commits, _, _) = commit_all(&mut repo, &nets);
    let before = stored(&repo);
    repo.repack().unwrap();
    // Framed chunks stay content through repack: in object form every
    // version of a chunk would be stored whole again.
    assert!(
        stored(&repo) <= before,
        "{} after, {before} before",
        stored(&repo)
    );
    for (k, c) in commits.iter().enumerate() {
        assert_eq!(repo.net_fragment_file(c, "nets/s.net").unwrap(), nets[k]);
    }
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn smaller_chunk_bounds_cut_more_chunks_of_the_same_graph() {
    let net = synth(3000);
    let mut a = Repo::new(MemStore::default());
    let mut b = Repo::new(MemStore::default());
    b.graph_chunks = ChunkSizes {
        leaf: (256, 2048),
        parent: (256, 2048),
    };
    assert_eq!(a.graph_chunks, ChunkSizes::default());
    let (ra, rb) = (
        a.put_net_fragments(&net, Layout::Tree).unwrap(),
        b.put_net_fragments(&net, Layout::Tree).unwrap(),
    );
    assert_ne!(ra, rb);
    assert_eq!(a.net_fragments(&ra).unwrap(), net);
    assert_eq!(b.net_fragments(&rb).unwrap(), net);
    assert!(b.store().ids().unwrap().len() > a.store().ids().unwrap().len() + 100);
}

#[test]
fn graph_edits_cut_with_the_same_bounds_as_put_graph() {
    let mut repo = Repo::new(MemStore::default());
    repo.graph_chunks = ChunkSizes {
        leaf: (256, 2048),
        parent: (300, 3000),
    };
    let node = |i: usize, v: usize| Node {
        label: format!("n{i:05}"),
        target: Target::Content(ikam_kernel::Arg::Whole(Id::of_content(
            format!("payload {i} {v}").as_bytes(),
        ))),
        edges: vec![Edge {
            to: ikam_kernel::node_key(&format!("n{:05}", (i * 31) % 4000)),
            label: "link".into(),
        }],
    };
    let mut nodes: Vec<Node> = (0..4000).map(|i| node(i, 0)).collect();
    let root = repo.put_graph(nodes.clone()).unwrap();
    let mut edits = std::collections::BTreeMap::new();
    for i in [3, 1500, 1501, 3999] {
        edits.insert(format!("n{i:05}"), Some(node(i, 1)));
        nodes[i] = node(i, 1);
    }
    edits.insert("n02000".into(), None);
    nodes.remove(2000);
    let edited = repo.graph_edit(&root, edits).unwrap();
    assert_eq!(edited, repo.put_graph(nodes).unwrap());
}

#[test]
fn firings_tracked_by_framed_fragments_replay_to_the_same_marking() {
    let night = |k: u32| {
        let mut t = format!(
            "net night {k}\nplace cpu 2\nplace queued colored\nplace running colored\nplace measured colored\n"
        );
        t += "transition propose agent: -> queued\ntransition start cmd: queued, cpu -> running\n";
        t += "transition finish pure: running -> measured, cpu\n";
        for j in 0..k * 10 {
            t += &format!("place q{j}\ntransition x{j} pure: q{j} -> cpu\n");
        }
        Net::parse(&t).unwrap()
    };
    let mut markings = Vec::new();
    for contexts in [false, true] {
        let mut repo = Repo::new(MemStore::default());
        repo.object_contexts = contexts;
        let mut cur = night(1);
        let db = repo.db_empty().unwrap();
        let db = repo.net_init(&db, &cur).unwrap();
        let mut db = repo.net_track_fragments(&db, &cur, Layout::Tree).unwrap();
        for i in 0..24 {
            if i > 0 && i % 4 == 0 {
                cur = night(cur.version + 1);
            }
            let tok = format!("c{i}");
            for (j, t) in ["propose", "start", "finish"].iter().enumerate() {
                db = repo
                    .net_fire(&db, &cur, t, Some(&tok), &format!("{i}.{j}"))
                    .unwrap()
                    .0;
            }
        }
        let m = repo.net_marking(&db, "night").unwrap();
        assert_eq!(repo.net_replay_versions(&db, "night").unwrap(), m);
        assert_eq!(repo.net_firings(&db, "night").unwrap().len(), 72);
        if contexts {
            assert!(frames(&repo) > 0, "the grown nets' chunks are framed");
        }
        assert!(repo.fsck().unwrap().is_empty());
        markings.push(m);
    }
    assert_eq!(markings[0], markings[1]);
}
