//! The neighborhood sampler (E427): a fragment view holds every render
//! edge under a commit, batches are pure `sample(view, spec)` derivations
//! whose edges all exist in the graph, and a run recorded over them walks
//! back to the commit it was sampled from.

use std::collections::HashSet;
use std::sync::Arc;

use ikam_kernel::sampler::{Batch, FILE, FRAGMENT, Sampler, Spec, Targets, View};
use ikam_kernel::step::Found;
use ikam_kernel::{Arg, Edit, Error, Id, MemStore, Repo, Store, func};

/// Files sharing paragraphs: a hub paragraph in every file, a few shared
/// by pairs, the rest their own.
fn files(n: usize) -> Vec<(String, Vec<u8>)> {
    (0..n)
        .map(|i| {
            let mut t = String::from("A shared hub paragraph.\n\n");
            t += &format!("Shared by pair {}.\n\n", i / 2);
            t += &format!("Own text of file {i}.\n\n");
            if i % 3 == 0 {
                t += "A paragraph every third file has.\n\n";
            }
            let ext = ["md", "txt", "csv"][i % 3];
            (format!("dir/f{i}.{ext}"), t.into_bytes())
        })
        .collect()
}

fn store(n: usize) -> (Repo<MemStore>, Id) {
    let mut repo = Repo::new(MemStore::default());
    let mut edits = Vec::new();
    for (path, b) in files(n) {
        let id = repo.put_fragmented(&b, None).unwrap();
        edits.push(Edit::Put { path, id });
    }
    let tree = repo.edit_tree(None, &edits).unwrap();
    let commit = repo.commit("main", tree, "").unwrap();
    (repo, commit)
}

fn spec(index: u32) -> Spec {
    Spec {
        seed: 3,
        fanout: 4,
        hops: 2,
        batch: 8,
        index,
        targets: Targets::All,
    }
}

#[test]
fn a_view_holds_every_render_edge_untruncated() {
    let (repo, commit) = store(30);
    let view = repo.fragment_view(&commit).unwrap();
    assert_eq!(View::decode(&view.encode()).unwrap(), view);
    let files: HashSet<Id> = files(30).iter().map(|(_, b)| Id::of_content(b)).collect();
    let mut edges = HashSet::new();
    for f in &files {
        for g in repo.fragments_of(f).unwrap().unwrap() {
            edges.insert((*f, g));
        }
    }
    assert_eq!(view.edges(), edges.len());
    for (v, id) in view.ids.iter().enumerate() {
        let want = if files.contains(id) { FILE } else { FRAGMENT };
        assert_eq!(view.kind[v], want);
        assert_eq!(view.len[v], repo.read_content(id).unwrap().len() as u64);
    }
    // The hub is in every file: its whole adjacency is kept, far past any
    // fanout.
    let hub = Id::of_content(b"A shared hub paragraph.\n\n");
    let h = view.ordinal(&hub).unwrap();
    assert_eq!(view.neighbors(h).len(), 30);
    assert_eq!(view.labels, ["csv", "md", "txt"]);
}

#[test]
fn batches_are_pure_seeded_and_every_edge_exists() {
    let (mut repo, commit) = store(40);
    let (view_id, _) = repo.record_fragment_view(&commit).unwrap();
    let bytes = repo.read_content(&view_id).unwrap();
    let sampler = Sampler::new(&bytes).unwrap();
    let per = sampler.batches_per_epoch(&spec(0));
    let mut roots = Vec::new();
    for index in 0..per {
        let s = spec(index);
        let id = repo.apply_sample(view_id, &s).unwrap();
        // The same as the sampler's, and the same again.
        let (b, stats) = sampler.sample(&s).unwrap();
        assert_eq!(Id::of_content(&b), id);
        assert_eq!(repo.apply_sample(view_id, &s).unwrap(), id);
        assert!(stats.reads <= stats.occurrences);
        let batch = Batch::decode(&b).unwrap();
        assert_eq!(batch.view, view_id);
        for (u, v) in batch.edge_ordinals() {
            assert!(sampler.view().neighbors(u).binary_search(&v).is_ok());
            let (a, c) = (
                sampler.view().ids[u as usize],
                sampler.view().ids[v as usize],
            );
            let renders = |x: &Id, y: &Id| {
                repo.fragments_of(x)
                    .unwrap()
                    .is_some_and(|fs| fs.contains(y))
            };
            assert!(renders(&a, &c) || renders(&c, &a));
        }
        // At most `fanout` neighbors a frontier node.
        for (h, es) in batch.edges.iter().enumerate() {
            for p in 0..batch.layers[h].len() as u32 {
                assert!(es.iter().filter(|e| e.0 == p).count() <= 4);
            }
        }
        roots.extend(batch.layers[0].clone());
    }
    // One epoch covers every node once.
    roots.sort_unstable();
    assert_eq!(
        roots,
        (0..sampler.view().nodes() as u32).collect::<Vec<_>>()
    );
    // Another seed samples another batch.
    let other = Spec { seed: 4, ..spec(0) };
    assert_ne!(
        sampler.sample(&other).unwrap().0,
        sampler.sample(&spec(0)).unwrap().0
    );
}

#[test]
fn batches_rebuild_after_repack_and_views_are_kept() {
    let (mut repo, commit) = store(40);
    let (view_id, _) = repo.record_fragment_view(&commit).unwrap();
    let batches: Vec<Id> = (0..4)
        .map(|i| repo.apply_sample(view_id, &spec(i)).unwrap())
        .collect();
    let mut args = vec![];
    args.extend(batches.iter().map(|b| Arg::Whole(*b)));
    // A recorded step over the batches keeps them live, and its output is
    // what a commit names.
    let (out, _) = repo
        .record_step(func::concat(), args, Some(b"trained"), None)
        .unwrap();
    let tree = repo
        .edit_tree(
            None,
            &[Edit::Put {
                path: "out".into(),
                id: out,
            }],
        )
        .unwrap();
    repo.commit("main", tree, "").unwrap();
    let want: Vec<Vec<u8>> = batches
        .iter()
        .map(|b| repo.read_content(b).unwrap())
        .collect();
    repo.read_weight = 0.00097;
    repo.repack().unwrap();
    repo.gc().unwrap();
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.store().has(&out) && repo.store().has(&view_id));
    for (b, w) in batches.iter().zip(&want) {
        assert_eq!(&repo.read_content(b).unwrap(), w);
    }
    assert!(repo.check_fragment_view(&view_id).unwrap());
}

#[test]
fn a_view_step_is_refused_when_evaluated() {
    let (mut repo, commit) = store(6);
    let pin = repo.view_pin(&commit).unwrap();
    assert!(matches!(
        repo.apply(func::fragment_view(), vec![Arg::Whole(pin)]),
        Err(Error::Exec(_))
    ));
}

#[test]
fn specs_are_canonical() {
    let s = Spec {
        targets: Targets::Files,
        ..spec(7)
    };
    assert_eq!(Spec::parse(&s.to_text()).unwrap(), s);
    for bad in [
        "ikam-sample/1 seed=1 fanout=4 hops=2 batch=8 index=0 targets=all",
        "ikam-sample/1 seed=01 fanout=4 hops=2 batch=8 index=0 targets=all\n",
        "ikam-sample/1 seed=1 fanout=0 hops=2 batch=8 index=0 targets=all\n",
        "other seed=1\n",
    ] {
        assert!(Spec::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_stream_hands_over_every_batch_in_order() {
    let (repo, commit) = store(20);
    let bytes = repo.fragment_view(&commit).unwrap().encode();
    let sampler = Arc::new(Sampler::new(&bytes).unwrap());
    let specs: Vec<Spec> = (0..12).map(spec).collect();
    let got: Vec<(Spec, Vec<u8>)> = sampler
        .clone()
        .stream(specs.clone(), 1)
        .into_iter()
        .map(|r| {
            let (s, b, _) = r.unwrap();
            (s, b)
        })
        .collect();
    assert_eq!(got.len(), specs.len());
    for ((s, b), want) in got.iter().zip(&specs) {
        assert_eq!(s, want);
        assert_eq!(b, &sampler.sample(want).unwrap().0);
    }
}

#[test]
fn a_run_walks_back_to_the_commit_it_was_sampled_from() {
    let (mut repo, commit) = store(20);
    let (view_id, view_rec) = repo.record_fragment_view(&commit).unwrap();
    let batches: Vec<Id> = (0..3)
        .map(|i| repo.apply_sample(view_id, &spec(i)).unwrap())
        .collect();
    let program = repo
        .put_content(b"a program outside the kernel")
        .unwrap()
        .id;
    let config = repo.put_content(b"config").unwrap().id;
    let mut args = vec![Arg::Whole(config)];
    args.extend(batches.iter().map(|b| Arg::Whole(*b)));
    let (weights, rec) = repo
        .record_step(program, args, Some(b"weights"), None)
        .unwrap();
    let lin = repo.lineage(&weights).unwrap();
    assert!(lin.missing.is_empty());
    assert_eq!(lin.commits, [commit]);
    let steps: Vec<&Found> = lin.nodes.iter().flat_map(|(_, f)| f).collect();
    assert!(steps.iter().any(
        |f| matches!(f, Found::Recorded { record, func, .. } if *record == rec && *func == program)
    ));
    assert!(
        steps
            .iter()
            .any(|f| matches!(f, Found::Recorded { record, .. } if *record == view_rec))
    );
    let sampled = steps
        .iter()
        .filter(|f| matches!(f, Found::Derived { func: f, .. } if *f == func::sample()))
        .count();
    assert_eq!(sampled, batches.len());
    for id in [program, config, view_id] {
        assert!(lin.nodes.iter().any(|(n, _)| *n == id));
    }
}
