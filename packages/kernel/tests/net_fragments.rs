//! E446: nets as connected fragments. Each line of a net's canonical text
//! is a content of its own and a graph names them; versions chain through
//! commits and reuse what earlier versions wrote, and a delta between two
//! versions is computed by walking both, skipping what they share.

use ikam_kernel::net::{Arc, Change, Kind, Layout, Net, Place, Transition};
use ikam_kernel::{Db, Edit, MemStore, Repo, Store};

fn arc(place: &str, weight: u32) -> Arc {
    Arc {
        place: place.into(),
        weight,
    }
}

/// A synthetic net of `n` transitions over `n / 2` places (at least 4).
fn synth(n: usize) -> Net {
    let np = (n / 2).max(4);
    let kinds = [Kind::Pure, Kind::Cmd, Kind::Agent, Kind::Human];
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
            .map(|i| {
                let (a, b) = ((i * 7 + 1) % np, (i * 3 + 2) % np);
                Transition {
                    name: format!("t{i}"),
                    inputs: vec![arc(&format!("p{a}"), 1 + (i % 3) as u32)],
                    outputs: vec![arc(&format!("p{b}"), 1)],
                    kind: kinds[i % 4],
                    capability: None,
                }
            })
            .collect(),
    }
}

/// Version `k` of a chain: rename, reweight, insert, remove a place, burst.
fn edit(net: &mut Net, k: usize) {
    net.version = k as u32;
    let pick = |len: usize| (k * 7919) % len.max(1);
    match k % 5 {
        0 => {
            let j = pick(net.transitions.len());
            net.transitions[j].name += &format!("r{k}");
        }
        1 => {
            let j = pick(net.transitions.len());
            net.transitions[j].outputs[0].weight += 1;
        }
        2 => {
            let at = pick(net.transitions.len() + 1);
            let place = format!("q{k}");
            net.places.insert(
                pick(net.places.len() + 1),
                Place {
                    name: place.clone(),
                    colored: false,
                    initial: 0,
                },
            );
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
        _ => {
            let m = (net.transitions.len() / 2).clamp(1, 20);
            let j = pick(net.transitions.len() - m + 1);
            for t in &mut net.transitions[j..j + m] {
                t.outputs[0].weight = t.outputs[0].weight % 4 + 1;
            }
        }
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

#[test]
fn fragments_read_back_in_both_layouts() {
    let mut repo = Repo::new(MemStore::default());
    for n in [0, 1, 10, 2000] {
        let net = synth(n.max(1));
        let flat = repo.put_net_fragments(&net, Layout::Flat).unwrap();
        let tree = repo.put_net_fragments(&net, Layout::Tree).unwrap();
        assert_eq!(repo.net_fragments(&flat).unwrap(), net);
        assert_eq!(repo.net_fragments(&tree).unwrap(), net);
        // `net_version` reads a fragment root as it reads E422's text id.
        assert_eq!(repo.net_version(&tree).unwrap(), net);
        let text = repo.put_net_file(None, "n.net", &net).unwrap().1;
        assert_eq!(repo.net_version(&text).unwrap(), net);
        if n <= 10 {
            assert_eq!(flat, tree, "one chunk: the same root in both layouts");
        } else {
            assert_ne!(flat, tree);
        }
    }
    // The same net always has the same root, whatever was stored before.
    let net = synth(300);
    let a = repo.put_net_fragments(&net, Layout::Tree).unwrap();
    let mut fresh = Repo::new(MemStore::default());
    assert_eq!(fresh.put_net_fragments(&net, Layout::Tree).unwrap(), a);
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn versions_chain_through_commits_and_reuse_fragments() {
    let mut repo = Repo::new(MemStore::default());
    let nets = chain(2000, 22);
    let (mut tree, mut commits) = (None, Vec::new());
    let mut sizes = Vec::new();
    for net in &nets {
        let before = repo.store().ids().unwrap().len();
        let (t, _) = repo
            .put_net_fragment_file(tree, "nets/s.net", net, Layout::Tree)
            .unwrap();
        tree = Some(t);
        commits.push(
            repo.commit("main", t, &format!("v{}", net.version))
                .unwrap(),
        );
        sizes.push(repo.store().ids().unwrap().len() - before);
    }
    // Version 1 writes every fragment; a later version writes only the
    // fragments it changed and the chunks above them (a burst of 20
    // reweighted transitions is the largest edit here).
    let first = sizes[0];
    assert!(first > 3000, "{first}");
    for (k, s) in sizes.iter().enumerate().skip(1) {
        assert!(*s < 60, "version {} wrote {s} objects", k + 1);
    }
    // Every commit reads back its version, after a gc too.
    repo.gc().unwrap();
    for (c, net) in commits.iter().zip(&nets) {
        assert_eq!(&repo.net_fragment_file(c, "nets/s.net").unwrap(), net);
    }
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn a_delta_applied_gives_the_later_version_exactly() {
    for (n, layout) in [
        (10, Layout::Flat),
        (10, Layout::Tree),
        (1500, Layout::Flat),
        (1500, Layout::Tree),
    ] {
        let mut repo = Repo::new(MemStore::default());
        let nets = chain(n, 21);
        let roots: Vec<_> = nets
            .iter()
            .map(|net| repo.put_net_fragments(net, layout).unwrap())
            .collect();
        let pairs = (1..nets.len())
            .map(|k| (k - 1, k))
            .chain([(0, nets.len() - 1), (nets.len() - 1, 0)]);
        for (a, b) in pairs {
            let d = repo.net_delta(&roots[a], &roots[b]).unwrap();
            assert_eq!(
                d.apply(&nets[a]).unwrap(),
                nets[b],
                "{n} {layout:?} {a}->{b}"
            );
            let full = repo.net_delta_with(&roots[a], &roots[b], false).unwrap();
            assert_eq!(
                (&d.head, &d.places, &d.transitions, &d.arcs, &d.links),
                (
                    &full.head,
                    &full.places,
                    &full.transitions,
                    &full.arcs,
                    &full.links
                ),
            );
        }
        assert!(repo.net_delta(&roots[3], &roots[3]).unwrap().is_empty());
    }
}

#[test]
fn a_delta_skips_the_chunks_both_versions_share() {
    let mut repo = Repo::new(MemStore::default());
    let a = synth(5000);
    let mut b = a.clone();
    b.transitions[2500].inputs[0].weight += 1;
    let ra = repo.put_net_fragments(&a, Layout::Tree).unwrap();
    let rb = repo.put_net_fragments(&b, Layout::Tree).unwrap();
    let d = repo.net_delta(&ra, &rb).unwrap();
    let full = repo.net_delta_with(&ra, &rb, false).unwrap();
    // One transition changed, one arc's weight, nothing else.
    assert_eq!(d.transitions.len(), 1);
    assert!(matches!(&d.transitions[0], Change::Changed { after, .. } if after.name == "t2500"));
    assert_eq!(d.arcs.len(), 1);
    assert_eq!((d.arcs[0].before, d.arcs[0].after), (Some(2), Some(3)));
    assert!(d.places.is_empty() && d.head.is_none());
    assert_eq!(d.fragments_read, 2);
    // The walk reads the path to the changed leaf on each side and that
    // leaf again; listing every leaf reads the whole of both graphs.
    assert!(
        d.chunks_read * 10 < full.chunks_read,
        "pruned {} unpruned {}",
        d.chunks_read,
        full.chunks_read
    );
    assert!(d.chunks_read <= 10, "{}", d.chunks_read);
}

#[test]
fn a_delta_names_what_changed() {
    let mut repo = Repo::new(MemStore::default());
    let a = synth(40);
    let ra = repo.put_net_fragments(&a, Layout::Tree).unwrap();
    // A rename is the old transition removed and the new one added; its
    // neighbour before it is relinked, not changed.
    let mut b = a.clone();
    b.version = 2;
    b.transitions[7].name = "renamed".into();
    let rb = repo.put_net_fragments(&b, Layout::Tree).unwrap();
    let d = repo.net_delta(&ra, &rb).unwrap();
    assert_eq!(d.head, Some((("s".into(), 1), ("s".into(), 2))));
    let mut names: Vec<String> = d
        .transitions
        .iter()
        .map(|c| match c {
            Change::Added(t) => format!("+{}", t.name),
            Change::Removed(t) => format!("-{}", t.name),
            Change::Changed { after, .. } => format!("~{}", after.name),
        })
        .collect();
    names.sort();
    assert_eq!(names, ["+renamed", "-t7"]);
    assert_eq!(d.relinked, ["t6"]);
    assert_eq!(d.apply(&a).unwrap(), b);
    // Removing a place changes the transitions whose arcs named it.
    let mut c = a.clone();
    let gone = c.places.remove(3).name;
    c.transitions
        .retain(|t| !(t.inputs.iter().chain(&t.outputs)).any(|x| x.place == gone));
    let rc = repo.put_net_fragments(&c, Layout::Tree).unwrap();
    let d = repo.net_delta(&ra, &rc).unwrap();
    assert_eq!(d.places, [Change::Removed(a.places[3].clone())]);
    let removed = a.transitions.len() - c.transitions.len();
    assert_eq!(d.transitions.len(), removed);
    assert_eq!(d.apply(&a).unwrap(), c);
}

#[test]
fn a_root_that_is_not_the_nets_own_nodes_is_refused() {
    let mut repo = Repo::new(MemStore::default());
    let net = synth(10);
    let good = repo.put_net_fragments(&net, Layout::Tree).unwrap();
    let mut nodes = repo.graph_nodes(&good).unwrap();
    // Drop one transition's `next` edge: the order no longer reaches every
    // transition.
    let t = nodes.iter_mut().find(|n| n.label == "t/t4").unwrap();
    t.edges.clear();
    let bad = repo.put_graph(nodes).unwrap();
    assert!(repo.net_fragments(&bad).is_err());
    assert!(repo.net_version(&bad).is_err());
}

const NIGHT: &str = "\
net night 1
place cpu 2
place queued colored
place running colored
place measured colored
transition propose agent: -> queued
transition start cmd: queued, cpu -> running
transition finish pure: running -> measured, cpu
";

fn night(k: u32) -> Net {
    let mut text = NIGHT.replace("net night 1", &format!("net night {k}"));
    if k > 1 {
        text += "place checked colored\n";
        text += &format!("transition check{k} pure: measured -> checked\n");
    }
    Net::parse(&text).unwrap()
}

/// The night net run for `n` candidates, edited every few, tracked by text
/// (None) or by fragments.
fn run(n: usize, layout: Option<Layout>) -> (Repo<MemStore>, Db) {
    let mut repo = Repo::new(MemStore::default());
    let mut cur = night(1);
    let db = repo.db_empty().unwrap();
    let db = repo.net_init(&db, &cur).unwrap();
    let mut db = match layout {
        None => repo.net_track_versions(&db, &cur).unwrap(),
        Some(l) => repo.net_track_fragments(&db, &cur, l).unwrap(),
    };
    for i in 0..n {
        if i > 0 && i % 4 == 0 {
            cur = night(cur.version + 1);
        }
        let tok = format!("c{i}");
        let check = format!("check{}", cur.version);
        let mut steps = vec!["propose", "start", "finish"];
        if cur.version > 1 {
            steps.push(&check);
        }
        for (j, t) in steps.iter().enumerate() {
            db = repo
                .net_fire(&db, &cur, t, Some(&tok), &format!("{i}.{j}"))
                .unwrap()
                .0;
        }
    }
    (repo, db)
}

#[test]
fn firings_name_fragment_versions_and_replay_like_text_ones() {
    let (text_repo, text_db) = run(20, None);
    let want = text_repo.net_marking(&text_db, "night").unwrap();
    assert_eq!(
        text_repo.net_replay_versions(&text_db, "night").unwrap(),
        want
    );
    for layout in [Layout::Flat, Layout::Tree] {
        let (mut repo, db) = run(20, Some(layout));
        let m = repo.net_marking(&db, "night").unwrap();
        assert_eq!(m, want, "the same marking as the run tracked by text");
        assert_eq!(repo.net_replay_versions(&db, "night").unwrap(), m);
        let firings = repo.net_firings(&db, "night").unwrap();
        let mut versions: Vec<u32> = Vec::new();
        for f in &firings {
            let root = f.net.unwrap();
            let net = repo.net_version(&root).unwrap();
            assert_eq!(repo.put_net_fragments(&net, layout).unwrap(), root);
            if versions.last() != Some(&net.version) {
                versions.push(net.version);
            }
        }
        assert_eq!(versions, (1..=5).collect::<Vec<_>>());
        // A firing under a known version finds its fragments: none is
        // written again.
        let before = repo.store().ids().unwrap().len();
        repo.put_net_fragments(&night(5), layout).unwrap();
        assert_eq!(repo.store().ids().unwrap().len(), before);
    }
    // Tracking by text and by fragments are told apart by the tracking
    // key; an E422 record still reads and replays.
    let (repo, db) = run(3, None);
    assert!(repo.net_replay_versions(&db, "night").is_ok());
}

#[test]
fn a_graph_entry_sits_in_a_tree_beside_files() {
    let mut repo = Repo::new(MemStore::default());
    let net = synth(20);
    let (t, root) = repo
        .put_net_fragment_file(None, "a/b.net", &net, Layout::Tree)
        .unwrap();
    let (t, _) = repo.put_net_file(Some(t), "a/c.net", &net).unwrap();
    assert_eq!(repo.net_fragment_file(&t, "a/b.net").unwrap(), net);
    assert_eq!(repo.net_file(&t, "a/c.net").unwrap(), net);
    // A text-file listing does not take a graph entry for a file.
    let files = repo.net_files(&t).unwrap();
    assert_eq!(files.len(), 1);
    let t2 = repo
        .edit_tree(
            Some(t),
            &[Edit::PutGraph {
                path: "a/b.net".into(),
                id: root,
            }],
        )
        .unwrap();
    assert_eq!(t2, t, "putting the same graph again changes nothing");
}
