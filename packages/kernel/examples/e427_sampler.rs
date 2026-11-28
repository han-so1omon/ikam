//! E427: a neighborhood sampler over the fragment graph, and an outside
//! trainer whose run is a recorded step.
//!
//! `run` builds the store and does every step; `reproduce` opens the store
//! in a new process and works from ids alone.
//!
//! 1. The bench's three north corpora as one commit, each file stored as a
//!    render of its fragments (blocks shared by two or more files stand
//!    alone, E157). The fragment view of that commit: nodes, edges, hubs.
//! 2. Sampling: 2-hop, fanout-10 batches from the view, batches a second,
//!    peak heap, adjacency reads against parent-to-node occurrences, and
//!    every sampled edge checked against the ledger's renders. Streamed to
//!    a consumer thread over a bounded channel.
//! 3. Recording: a net `trainer` (a file in the commit, versions tracked)
//!    fires `view` (cmd: the view as a recorded step), then `train` (cmd:
//!    the outside trainer, its program a content naming its sources,
//!    Cargo.lock and binary hash; its inputs the config, the seed and every
//!    batch, each batch a pure `sample` derivation). Outputs committed.
//!    Repack at the archive weight, fsck, gc.
//! 4. `reproduce`: from the weights id, the lineage walk; every input read
//!    back (batches rebuilt where repack dropped them); the trainer rerun
//!    with the thread counts and core pinnings available, each compared
//!    bit for bit; a second net version fires `reproduce` (pure: the
//!    check's verdict); replay by versions and step verification.
//!
//!   cargo run --release --example e427_sampler -- run TRAINER_BIN TRAINER_SRC STORE_DIR [EPOCHS]
//!   cargo run --release --example e427_sampler -- reproduce TRAINER_BIN STORE_DIR

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use ikam_kernel::net::{Net, Step};
use ikam_kernel::sampler::{Batch, FILE, FRAGMENT, RENDER, Sampler, Spec, Targets, View};
use ikam_kernel::step::{Design, Found};
use ikam_kernel::{Arg, Check, Db, Edit, FsStore, Id, Object, Repo, Store, blocks, func};

struct Counting;
static HEAP: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = HEAP.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(now, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        HEAP.fetch_sub(layout.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, layout, size) };
        if !q.is_null() {
            let now = HEAP.fetch_add(size, Relaxed) + size - layout.size();
            HEAP.fetch_sub(layout.size(), Relaxed);
            PEAK.fetch_max(now, Relaxed);
        }
        q
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Peak heap above the heap in use now, from here on.
fn peak_reset() -> usize {
    let now = HEAP.load(Relaxed);
    PEAK.store(now, Relaxed);
    now
}

fn mb(b: usize) -> f64 {
    b as f64 / 1e6
}

const NET_V1: &str = "\
net trainer 1
place commits colored
place viewed colored
place trained colored
transition pick human: -> commits
transition view cmd: commits -> viewed
transition train cmd: viewed -> trained
";

fn net_v2() -> Net {
    let text = NET_V1.replace("net trainer 1", "net trainer 2")
        + "place reproduced colored\ntransition reproduce pure: trained -> reproduced\n";
    Net::parse(&text).unwrap()
}

const SEED: u64 = 7;
const FANOUT: u32 = 10;
const HOPS: u32 = 2;
const BATCH: u32 = 256;
/// Epochs the throughput rows sample (the training run uses its own).
const BENCH_EPOCHS: u32 = 50;
/// Files of the synthetic graph, and batches sampled from it.
const SYNTHETIC_FILES: usize = 200_000;
const SYNTHETIC_BATCHES: u32 = 2_000;
/// The bench's archive read weight (E099), so repack may drop batches.
const ARCHIVE_READ_WEIGHT: f64 = 0.00097;

fn walk(root: &Path, keep: &dyn Fn(&Path) -> bool) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if keep(&path) {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((name, std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// Which files of a directory a corpus keeps.
type Keep = Box<dyn Fn(&Path) -> bool>;

/// The bench's north corpora (examples/bench.rs), each under its own
/// directory, file names flattened as the bench's one-store does.
fn north() -> Vec<(String, Vec<u8>)> {
    let kernel = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases = kernel.join("../../tests/fixtures/cases");
    let keep = [
        "md", "pdf", "docx", "xlsx", "pptx", "json", "jsonl", "log", "py", "html",
    ];
    let mut out = Vec::new();
    let sets: [(&str, PathBuf, Keep); 3] = [
        (
            "cases",
            cases,
            Box::new(move |p: &Path| {
                !p.to_string_lossy().contains("_image_tools")
                    && p.extension().is_some_and(|e| keep.iter().any(|k| e == *k))
            }),
        ),
        (
            "multiformat",
            kernel.join("fixtures/multiformat"),
            Box::new(|p: &Path| p.extension().is_some_and(|e| e != "txt")),
        ),
        (
            "tables",
            kernel.join("fixtures/tables"),
            Box::new(|p: &Path| p.extension().is_some_and(|e| e != "txt" && e != "py")),
        ),
    ];
    for (dir, root, keep) in sets {
        for (name, bytes) in walk(&root, &*keep) {
            out.push((format!("corpora/{dir}/{}", name.replace('/', "|")), bytes));
        }
    }
    out
}

/// The trainer's program as one content: its sources and Cargo.lock, each
/// a fragment of a render, and the built binary's BLAKE3.
fn program<S: Store>(repo: &mut Repo<S>, src: &Path, binary: &[u8]) -> (Id, String) {
    let mut files: Vec<(String, Vec<u8>)> = walk(src, &|p: &Path| {
        !p.components().any(|c| c.as_os_str() == "target")
    });
    files.retain(|(n, _)| n == "Cargo.toml" || n == "Cargo.lock" || n.starts_with("src/"));
    let hash = blake3::hash(binary).to_hex().to_string();
    let mut bytes = b"ikam-program/1 sampler-trainer\n".to_vec();
    let mut cuts = Vec::new();
    let mut at = 0;
    for (name, b) in &files {
        bytes.extend_from_slice(format!("file {name} {}\n", b.len()).as_bytes());
        cuts.push(at..bytes.len());
        at = bytes.len();
        bytes.extend_from_slice(b);
        cuts.push(at..bytes.len());
        at = bytes.len();
    }
    bytes.extend_from_slice(format!("\nbinary blake3 {hash} {}\n", binary.len()).as_bytes());
    cuts.push(at..bytes.len());
    (repo.put_cut(&bytes, &cuts, None).unwrap(), hash)
}

fn frame(w: &mut impl Write, b: &[u8]) -> std::io::Result<()> {
    w.write_all(&(b.len() as u64).to_le_bytes())?;
    w.write_all(b)
}

struct Trained {
    weights: Vec<u8>,
    loss: Vec<u8>,
    ms: f64,
    /// The trainer's CPU time, all its threads.
    cpu_ms: f64,
}

/// CPU time of every child waited for so far, in ms.
fn children_cpu_ms() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut u) };
    let t = |v: libc::timeval| v.tv_sec as f64 * 1e3 + v.tv_usec as f64 / 1e3;
    t(u.ru_utime) + t(u.ru_stime)
}

/// Run the trainer on `inputs` (config, seed, batches), streamed as frames
/// on its stdin. `threads` overrides the run's thread count, `cpus` pins it.
fn train(
    bin: &Path,
    inputs: &[Vec<u8>],
    out: &Path,
    threads: Option<u32>,
    cpus: Option<&str>,
) -> Trained {
    let _ = std::fs::remove_dir_all(out);
    let mut cmd = match cpus {
        Some(c) => {
            let mut t = std::process::Command::new("taskset");
            t.args(["-c", c]).arg(bin);
            t
        }
        None => std::process::Command::new(bin),
    };
    cmd.arg(out).stdin(std::process::Stdio::piped());
    cmd.env_remove("RAYON_NUM_THREADS");
    if let Some(t) = threads {
        cmd.env("RAYON_NUM_THREADS", t.to_string());
    }
    let (at, cpu) = (Instant::now(), children_cpu_ms());
    let mut child = cmd.spawn().expect("trainer");
    let mut stdin = std::io::BufWriter::new(child.stdin.take().unwrap());
    for b in inputs {
        frame(&mut stdin, b).unwrap();
    }
    drop(stdin);
    assert!(child.wait().unwrap().success(), "trainer failed");
    Trained {
        weights: std::fs::read(out.join("weights.f64")).unwrap(),
        loss: std::fs::read(out.join("loss.log")).unwrap(),
        ms: at.elapsed().as_secs_f64() * 1e3,
        cpu_ms: children_cpu_ms() - cpu,
    }
}

fn open(dir: &Path) -> Repo<FsStore> {
    Repo::new(FsStore::open(dir).unwrap())
}

fn stored_bytes<S: Store>(repo: &Repo<S>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

/// Every (render, argument) pair the ledger holds for the view's nodes,
/// read from the ledger's derivations, not from the view.
fn ledger_edges<S: Store>(repo: &Repo<S>, view: &View) -> HashSet<(Id, Id)> {
    let mut out = HashSet::new();
    for id in &view.ids {
        for d in repo.derivations(id).unwrap() {
            if d.func == func::render() {
                for a in &d.args {
                    out.insert((*id, a.id()));
                    out.insert((a.id(), *id));
                }
            }
        }
    }
    out
}

/// Sample `specs` from `view_bytes` directly and through a bounded
/// channel to a consumer thread that decodes each batch and checks every
/// edge against the view. One row a mode.
fn throughput(graph: &str, view_bytes: &[u8], specs: &[Spec]) {
    let base = peak_reset();
    let sampler = Arc::new(Sampler::new(view_bytes).unwrap());
    let at = Instant::now();
    let (mut st_reads, mut st_occ, mut st_edges, mut bytes_out, mut roots) = (0, 0, 0, 0, 0);
    for s in specs {
        let (b, st) = sampler.sample(s).unwrap();
        st_reads += st.reads;
        st_occ += st.occurrences;
        st_edges += st.edges;
        bytes_out += b.len();
        roots += Batch::decode(&b).unwrap().layers[0].len();
    }
    let secs = at.elapsed().as_secs_f64();
    let peak = PEAK.load(Relaxed) - base;
    let n = specs.len() as f64;
    println!(
        "| {graph} | direct | {} | {secs:.2} | {:.0} | {:.0} | {:.0} | {:.1} | {:.1} | {st_reads} / {st_occ} ({:.2}x) |",
        specs.len(),
        n / secs,
        roots as f64 / secs,
        st_edges as f64 / secs,
        mb(peak),
        bytes_out as f64 / n / 1e3,
        st_occ as f64 / st_reads as f64
    );
    for cap in [1usize, 8] {
        let base = peak_reset();
        let at = Instant::now();
        let rx = sampler.clone().stream(specs.to_vec(), cap);
        let (mut got, mut bad, mut edges) = (0usize, 0usize, 0usize);
        for r in rx {
            let (_, b, _) = r.unwrap();
            let batch = Batch::decode(&b).unwrap();
            for (u, v) in batch.edge_ordinals() {
                edges += 1;
                bad += usize::from(sampler.view().neighbors(u).binary_search(&v).is_err());
            }
            got += 1;
        }
        let secs = at.elapsed().as_secs_f64();
        assert_eq!(bad, 0, "{graph}: sampled edges not in the view");
        println!(
            "| {graph} | streamed, channel {cap} | {got} | {secs:.2} | {:.0} | {:.0} | {:.0} | {:.1} | | |",
            got as f64 / secs,
            roots as f64 / secs,
            edges as f64 / secs,
            mb(PEAK.load(Relaxed) - base)
        );
    }
}

/// A synthetic fragment graph for scale (no store): `files` files of eight
/// fragments each, six their own and two drawn from a shared pool of
/// `files / 5` with a skew that makes hubs (the pool's first entries).
fn synthetic_view(files: usize) -> View {
    let pool = files / 5;
    let n = files + files * 6 + pool;
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut z = 0x5eed_u64;
    let mut next = || {
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut x = z;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^ (x >> 31)
    };
    for f in 0..files {
        for k in 0..6 {
            let g = files + f * 6 + k;
            lists[f].push(g as u32);
            lists[g].push(f as u32);
        }
        for _ in 0..2 {
            let u = (next() >> 11) as f64 / (1u64 << 53) as f64;
            let g = files * 7 + ((pool as f64) * u * u * u) as usize;
            lists[f].push(g as u32);
            lists[g].push(f as u32);
        }
    }
    let (mut off, mut adj) = (vec![0u32], Vec::new());
    for mut l in lists {
        l.sort_unstable();
        l.dedup();
        adj.extend(l);
        off.push(adj.len() as u32);
    }
    View {
        commit: Id::from_bytes([0; 32]),
        tree: Id::from_bytes([0; 32]),
        labels: vec!["a".into(), "b".into(), "c".into()],
        ids: (0..n as u64)
            .map(|i| {
                let mut b = [0u8; 32];
                b[..8].copy_from_slice(&i.to_be_bytes());
                Id::from_bytes(b)
            })
            .collect(),
        kind: (0..n)
            .map(|v| if v < files { FILE } else { FRAGMENT })
            .collect(),
        label: (0..n)
            .map(|v| if v < files { (v % 3) as u16 } else { u16::MAX })
            .collect(),
        len: (0..n as u64).map(|v| 64 + v % 4096).collect(),
        off,
        adj,
    }
}

fn run(args: &[String]) {
    let (bin, src, dir) = (
        Path::new(&args[0]),
        Path::new(&args[1]),
        Path::new(&args[2]),
    );
    let epochs: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let mut repo = open(&dir.join("store"));

    println!("## 1. the fragment graph of the north corpora\n");
    let files = north();
    let at = Instant::now();
    let mut count: HashMap<Id, HashSet<Id>> = HashMap::new();
    for (_, b) in &files {
        let fid = Id::of_content(b);
        for r in blocks(b) {
            count.entry(Id::of_content(&b[r])).or_default().insert(fid);
        }
    }
    let shared: HashSet<Id> = count
        .into_iter()
        .filter(|(_, f)| f.len() >= 2)
        .map(|(k, _)| k)
        .collect();
    let mut edits = Vec::new();
    for (path, b) in &files {
        let id = repo.put_fragmented(b, Some(&shared)).unwrap();
        edits.push(Edit::Put {
            path: path.clone(),
            id,
        });
    }
    let tree = repo.edit_tree(None, &edits).unwrap();
    let net1 = Net::parse(NET_V1).unwrap();
    let (tree, _) = repo
        .put_net_file(Some(tree), "nets/trainer.net", &net1)
        .unwrap();
    let data = repo.commit("main", tree, "corpora").unwrap();
    let ingest_ms = at.elapsed().as_secs_f64() * 1e3;
    let at = Instant::now();
    let view = repo.fragment_view(&data).unwrap();
    let view_bytes = view.encode();
    let view_ms = at.elapsed().as_secs_f64() * 1e3;
    let kinds = |k: u8| view.kind.iter().filter(|&&x| x == k).count();
    let degs: Vec<usize> = (0..view.nodes() as u32)
        .map(|v| view.neighbors(v).len())
        .collect();
    let max_deg = degs.iter().copied().max().unwrap_or(0);
    let over = degs.iter().filter(|&&d| d > FANOUT as usize).count();
    let pairs = ledger_edges(&repo, &view);
    let total_bytes: usize = files.iter().map(|f| f.1.len()).sum();
    println!(
        "| files | bytes | nodes | files / fragments / renders | edges | max degree | nodes over fanout | view bytes | labels |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    println!(
        "| {} | {:.1} MB | {} | {} / {} / {} | {} | {} | {} | {:.2} MB | {} |",
        files.len(),
        mb(total_bytes),
        view.nodes(),
        kinds(FILE),
        kinds(FRAGMENT),
        kinds(RENDER),
        view.edges(),
        max_deg,
        over,
        mb(view_bytes.len()),
        view.labels.len()
    );
    // No stored truncation: the view holds every render edge the ledger has.
    let distinct_pairs = pairs.len() / 2;
    println!(
        "\nIngest (fragmenting) {ingest_ms:.0} ms; view built in {view_ms:.0} ms. Render edges in the ledger (distinct, undirected): {distinct_pairs}; in the view: {} ({}).\n",
        view.edges(),
        if distinct_pairs == view.edges() {
            "all of them"
        } else {
            "MISMATCH"
        }
    );

    println!("## 2. sampling: {HOPS} hops, fanout {FANOUT}, {BATCH} roots a batch\n");
    let sampler = Arc::new(Sampler::new(&view_bytes).unwrap());
    let spec0 = Spec {
        seed: SEED,
        fanout: FANOUT,
        hops: HOPS,
        batch: BATCH,
        index: 0,
        targets: Targets::All,
    };
    let per = sampler.batches_per_epoch(&spec0);
    let specs: Vec<Spec> = (0..per * epochs)
        .map(|index| Spec {
            index,
            ..spec0.clone()
        })
        .collect();
    println!(
        "| graph | mode | batches | seconds | batches/s | roots/s | edges/s | peak heap MB | batch KB (mean) | reads / occurrences |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    let bench: Vec<Spec> = (0..per * BENCH_EPOCHS)
        .map(|index| Spec {
            index,
            ..spec0.clone()
        })
        .collect();
    throughput("north", &view_bytes, &bench);
    drop(bench);
    let synthetic = synthetic_view(SYNTHETIC_FILES).encode();
    let s = Sampler::new(&synthetic).unwrap();
    let n = s.batches_per_epoch(&spec0).min(SYNTHETIC_BATCHES);
    drop(s);
    let bench: Vec<Spec> = (0..n)
        .map(|index| Spec {
            index,
            ..spec0.clone()
        })
        .collect();
    throughput(
        &format!("synthetic, {:.2} MB view", mb(synthetic.len())),
        &synthetic,
        &bench,
    );
    drop(synthetic);
    // Every sampled edge, against the ledger's own render records.
    let (mut checked, mut missing) = (0usize, 0usize);
    let mut seen_nodes = HashSet::new();
    for s in &specs {
        let b = Batch::decode(&sampler.sample(s).unwrap().0).unwrap();
        for (u, v) in b.edge_ordinals() {
            checked += 1;
            let (a, c) = (view.ids[u as usize], view.ids[v as usize]);
            missing += usize::from(!pairs.contains(&(a, c)));
            seen_nodes.insert(u);
            seen_nodes.insert(v);
        }
    }
    let present = seen_nodes
        .iter()
        .filter(|&&v| repo.has_content(&view.ids[v as usize]).unwrap())
        .count();
    println!(
        "\nEdges checked against the ledger's renders: {checked}, not found: {missing}. Distinct nodes sampled: {} of {}, present in the store: {present}.\n",
        seen_nodes.len(),
        view.nodes()
    );

    println!("## 3. recording: the view and the training run through the net\n");
    let mut db = repo.db_empty().unwrap();
    db = repo.net_init(&db, &net1).unwrap();
    db = repo.net_track_versions(&db, &net1).unwrap();
    db = repo
        .net_fire(&db, &net1, "pick", Some("r1"), "pick-r1")
        .unwrap()
        .0;
    let pin = repo.view_pin(&data).unwrap();
    let at = Instant::now();
    let (d, fview) = repo
        .net_fire_step(
            &db,
            &net1,
            "view",
            Some("r1"),
            "view-r1",
            Step::Recorded {
                func: func::fragment_view(),
                args: vec![Arg::Whole(pin)],
                output: Some(&view_bytes),
                check: None,
            },
        )
        .unwrap();
    db = d;
    let view_id = fview.output.unwrap();
    let fire_view_ms = at.elapsed().as_secs_f64() * 1e3;
    // Every training batch as a pure derivation sample(view, spec).
    let at = Instant::now();
    let mut batch_ids = Vec::new();
    for s in &specs {
        batch_ids.push(repo.apply_sample(view_id, s).unwrap());
    }
    let apply_ms = at.elapsed().as_secs_f64() * 1e3;
    let classes = view.labels.len() + 2;
    let config =
        format!("ikam-train/1 classes={classes} hidden=256 fanout={FANOUT} lr=0.01 threads=1\n");
    let config_id = repo.put_content(config.as_bytes()).unwrap().id;
    let seed_id = repo
        .put_content(format!("seed {SEED}\n").as_bytes())
        .unwrap()
        .id;
    let binary = std::fs::read(bin).unwrap();
    let (prog, hash) = program(&mut repo, src, &binary);
    let mut inputs = vec![config.into_bytes(), format!("seed {SEED}\n").into_bytes()];
    for id in &batch_ids {
        inputs.push(repo.read_content(id).unwrap());
    }
    let t = train(bin, &inputs, &dir.join("run-r1"), None, None);
    let mut run_args = vec![Arg::Whole(config_id), Arg::Whole(seed_id)];
    run_args.extend(batch_ids.iter().map(|b| Arg::Whole(*b)));
    let spec_check = repo.put_content(b"rel=1e-6").unwrap().id;
    let at = Instant::now();
    let (d, ftrain) = repo
        .net_fire_step(
            &db,
            &net1,
            "train",
            Some("r1"),
            "train-r1",
            Step::Recorded {
                func: prog,
                args: run_args.clone(),
                output: Some(&t.weights),
                check: Some(Check {
                    func: func::within_f64(),
                    args: vec![Arg::Whole(spec_check)],
                }),
            },
        )
        .unwrap();
    db = d;
    let (loss_id, loss_rec) = repo
        .record_step(prog, run_args, Some(&t.loss), None, Design::Run)
        .unwrap();
    let fire_train_ms = at.elapsed().as_secs_f64() * 1e3;
    let weights_id = ftrain.output.unwrap();
    let edits = vec![
        Edit::Put {
            path: "runs/r1/weights.f64".into(),
            id: weights_id,
        },
        Edit::Put {
            path: "runs/r1/loss.log".into(),
            id: loss_id,
        },
    ];
    let tree = repo.edit_tree(Some(tree), &edits).unwrap();
    repo.commit("main", tree, "run r1").unwrap();
    let head = repo.db_commit("main", &db, "net db").unwrap();
    let last = String::from_utf8_lossy(&t.loss)
        .lines()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .join(" | ");
    println!("| step | what | ms |");
    println!("| --- | --- | --- |");
    println!(
        "| view (cmd, recorded) | {:.2} MB view | {fire_view_ms:.0} |",
        mb(view_bytes.len())
    );
    println!(
        "| sample (pure, apply) | {} batches, {:.1} MB | {apply_ms:.0} |",
        batch_ids.len(),
        mb(inputs.iter().skip(2).map(Vec::len).sum())
    );
    println!(
        "| train (cmd, recorded) | {} weights, {} loss lines | {:.0} (trainer) + {fire_train_ms:.0} (record) |",
        t.weights.len() / 8,
        t.loss.iter().filter(|&&b| b == b'\n').count(),
        t.ms
    );
    println!("\nLast loss lines: {last}");
    println!(
        "Program {prog} (binary blake3 {}…); weights {weights_id}; loss log {loss_id} (record {loss_rec}); head {head}.\n",
        &hash[..16]
    );

    // Repack at the archive weight, then fsck and gc.
    let before = stored_bytes(&repo);
    let kept_before = batch_ids.iter().filter(|b| repo.store().has(b)).count();
    repo.read_weight = ARCHIVE_READ_WEIGHT;
    let at = Instant::now();
    repo.repack().unwrap();
    let repack_ms = at.elapsed().as_secs_f64() * 1e3;
    let fsck = repo.fsck().unwrap();
    let gc = repo.gc().unwrap();
    let after = stored_bytes(&repo);
    let kept_after = batch_ids.iter().filter(|b| repo.store().has(b)).count();
    let outputs_kept = [weights_id, loss_id, view_id]
        .iter()
        .all(|o| repo.store().has(o));
    println!(
        "| stage | stored bytes | batches stored as bytes | weights, loss log, view kept as bytes | fsck failures |"
    );
    println!("| --- | --- | --- | --- | --- |");
    println!(
        "| before repack | {:.1} MB | {kept_before} of {} | yes | |",
        mb(before),
        batch_ids.len()
    );
    println!(
        "| after repack ({repack_ms:.0} ms) and gc ({gc} deleted) | {:.1} MB | {kept_after} of {} | {} | {} |",
        mb(after),
        batch_ids.len(),
        if outputs_kept { "yes" } else { "NO" },
        fsck.len()
    );
    std::fs::write(
        dir.join("ids.txt"),
        format!("weights {weights_id}\ndata {data}\n"),
    )
    .unwrap();
}

fn reproduce(args: &[String]) {
    let (bin, dir) = (Path::new(&args[0]), Path::new(&args[1]));
    let ids = std::fs::read_to_string(dir.join("ids.txt")).unwrap();
    let get = |k: &str| -> Id {
        ids.lines()
            .find_map(|l| l.strip_prefix(&format!("{k} ")))
            .unwrap()
            .parse()
            .unwrap()
    };
    let (weights, data) = (get("weights"), get("data"));
    let mut repo = open(&dir.join("store"));
    let head = repo.store().get_ref("main").unwrap().unwrap();
    let mut db: Db = repo.db_at(&head).unwrap();

    println!("## 4. reproduce, from the weights id ({weights}), in a new process\n");
    let at = Instant::now();
    let lin = repo.lineage(&weights).unwrap();
    let walk_ms = at.elapsed().as_secs_f64() * 1e3;
    let mut of_record = None;
    for (id, f) in lin.recorded() {
        if *id == weights
            && let Found::Recorded {
                record,
                func,
                args,
                check,
            } = f
        {
            of_record = Some((*record, *func, args.clone(), check.clone()));
        }
    }
    let (record, prog, run_args, check) = of_record.expect("weights have a recorded step");
    let found = |id: &Id| {
        lin.nodes
            .iter()
            .find(|(n, _)| n == id)
            .map(|(_, f)| f.clone())
    };
    let mut batches = 0;
    let mut views = BTreeMap::new();
    for a in &run_args[2..] {
        for f in found(a).unwrap_or_default() {
            if let Found::Derived { func: f, args, .. } = f
                && f == func::sample()
            {
                batches += 1;
                views.insert(args[0], ());
            }
        }
    }
    let view_id = *views.keys().next().unwrap();
    let view_steps = found(&view_id).unwrap_or_default();
    let pin = view_steps.iter().find_map(|f| match f {
        Found::Recorded { func: f, args, .. } if *f == func::fragment_view() => Some(args[0]),
        _ => None,
    });
    let prog_text = repo.read_content(&prog).unwrap();
    let prog_text = String::from_utf8_lossy(&prog_text);
    let named_hash = prog_text
        .lines()
        .find_map(|l| l.strip_prefix("binary blake3 "))
        .and_then(|l| l.split(' ').next())
        .unwrap()
        .to_string();
    let sources: Vec<&str> = prog_text
        .lines()
        .filter_map(|l| l.strip_prefix("file "))
        .map(|l| l.rsplit_once(' ').unwrap().0)
        .collect();
    // The program's sources, written out from the store, so the binary can
    // be rebuilt from what the run names.
    let bytes = repo.read_content(&prog).unwrap();
    let out = dir.join("program-src");
    let mut rest = &bytes[bytes.iter().position(|&b| b == b'\n').unwrap() + 1..];
    while let Some(line) = rest.strip_prefix(b"file ".as_slice()) {
        let nl = line.iter().position(|&b| b == b'\n').unwrap();
        let head = std::str::from_utf8(&line[..nl]).unwrap();
        let (name, len) = head.rsplit_once(' ').unwrap();
        let len: usize = len.parse().unwrap();
        let path = out.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &line[nl + 1..nl + 1 + len]).unwrap();
        rest = &line[nl + 1 + len..];
    }
    let prog_pieces = found(&prog)
        .unwrap_or_default()
        .iter()
        .find_map(|f| match f {
            Found::Derived { args, .. } => Some(args.len()),
            _ => None,
        })
        .unwrap_or(0);
    println!("| walk names | found |");
    println!("| --- | --- |");
    println!("| the run (recorded step) | {record} |");
    println!(
        "| its program | {prog}: {} sources ({}) as {prog_pieces} fragments, binary blake3 {}… |",
        sources.len(),
        sources.join(", "),
        &named_hash[..16]
    );
    println!(
        "| config and seed | {} and {} |",
        short(&run_args[0]),
        short(&run_args[1])
    );
    println!(
        "| batches (pure sample derivations) | {batches} of {} |",
        run_args.len() - 2
    );
    println!(
        "| the view (recorded step, fragment-view) | {view_id} via pin {} |",
        pin.map(|p| p.to_string()).unwrap_or("NONE".into())
    );
    println!(
        "| the commit sampled from | {} (expected {data}: {}) |",
        lin.commits
            .iter()
            .map(Id::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        if lin.commits == [data] { "yes" } else { "NO" }
    );
    println!(
        "| the check | {} |",
        check
            .map(|(f, a)| format!(
                "within-f64 {}",
                if f == func::within_f64() { a.len() } else { 99 }
            ))
            .unwrap_or("none".into())
    );
    println!(
        "| contents walked / missing | {} / {} ({walk_ms:.0} ms) |",
        lin.nodes.len(),
        lin.missing.len()
    );
    let view_ok = repo.check_fragment_view(&view_id).unwrap();
    println!("\nView rebuilt from its commit equals the view of record: {view_ok}.\n");

    // Inputs, read back from their ids alone (dropped batches rebuilt).
    let at = Instant::now();
    let rebuilt = run_args.iter().filter(|a| !repo.store().has(a)).count();
    let inputs: Vec<Vec<u8>> = run_args
        .iter()
        .map(|a| repo.read_content(a).unwrap())
        .collect();
    let read_ms = at.elapsed().as_secs_f64() * 1e3;
    let ok_ids = inputs
        .iter()
        .zip(&run_args)
        .all(|(b, a)| Id::of_content(b) == *a);
    let binary = std::fs::read(bin).unwrap();
    let bin_ok = blake3::hash(&binary).to_hex().to_string() == named_hash;
    println!(
        "Inputs read from ids: {} ({rebuilt} rebuilt by their derivations), every id matches: {ok_ids}, in {read_ms:.0} ms. Binary matches the program's hash: {bin_ok}.\n",
        inputs.len()
    );
    assert!(ok_ids && bin_ok);

    println!(
        "| rerun | threads | cpus | trainer ms | trainer CPU ms | weights | loss log | verdict |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    let loss_of_record = repo.read_content(&loss_id_from(&repo, &head)).unwrap();
    let mut nows = Vec::new();
    let cpus_n = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut variants: Vec<(Option<u32>, Option<String>)> =
        vec![(None, None), (None, None), (Some(4), None), (Some(2), None)];
    if Path::new("/usr/bin/taskset").exists() || Path::new("/bin/taskset").exists() {
        variants.push((None, Some(format!("{}", cpus_n - 1))));
        variants.push((Some(4), Some(format!("0-{}", cpus_n - 1))));
        variants.push((Some(4), Some("0".into())));
    }
    for (k, (threads, cpus)) in variants.iter().enumerate() {
        let t = train(
            bin,
            &inputs,
            &dir.join(format!("rerun-{k}")),
            *threads,
            cpus.as_deref(),
        );
        let r = repo.rerun(&record, Some(&t.weights)).unwrap();
        let verdict = r
            .verdict
            .map(|v| {
                String::from_utf8_lossy(&repo.read_content(&v).unwrap())
                    .trim()
                    .to_string()
            })
            .unwrap_or_else(|| "same".into());
        println!(
            "| {k} | {} | {} | {:.0} | {:.0} | {} | {} | {verdict} |",
            threads.map_or("1 (of record)".to_string(), |t| t.to_string()),
            cpus.clone().unwrap_or("any".into()),
            t.ms,
            t.cpu_ms,
            if r.same { "bit-identical" } else { "DIFFERENT" },
            if t.loss == loss_of_record {
                "bit-identical"
            } else {
                "DIFFERENT"
            },
        );
        nows.push(r.now);
    }

    // Net version 2 fires the reproduce check as a pure step.
    let net2 = net_v2();
    let tree = ikam_kernel::snapshot::tree_of(&repo, &head).unwrap();
    let (tree, _) = repo
        .put_net_file(Some(tree), "nets/trainer.net", &net2)
        .unwrap();
    repo.commit("main", tree, "net v2").unwrap();
    let spec = repo.put_content(b"ulp=0").unwrap().id;
    let (d, f) = repo
        .net_fire_step(
            &db,
            &net2,
            "reproduce",
            Some("r1"),
            "reproduce-r1",
            Step::Pure {
                func: func::within_f64(),
                args: vec![Arg::Whole(weights), Arg::Whole(nows[0]), Arg::Whole(spec)],
            },
        )
        .unwrap();
    db = d;
    let verdict = String::from_utf8_lossy(&repo.read_content(&f.output.unwrap()).unwrap())
        .trim()
        .to_string();
    repo.db_commit("main", &db, "reproduce").unwrap();
    let at = Instant::now();
    let replay = repo.net_replay_versions(&db, "trainer").unwrap()
        == repo.net_marking(&db, "trainer").unwrap();
    let verify = repo.net_verify_steps(&db, "trainer").unwrap();
    let verify_ms = at.elapsed().as_secs_f64() * 1e3;
    let firings = repo.net_firings(&db, "trainer").unwrap();
    let fsck = repo.fsck().unwrap();
    println!(
        "\nNet v2 fired `reproduce` (pure within-f64, ulp=0): {verdict}. Firings: {} ({}); net_replay_versions equals the marking: {replay}; net_verify_steps: {} of {} pass ({verify_ms:.0} ms); fsck failures: {}.",
        firings.len(),
        firings
            .iter()
            .map(|f| format!(
                "{} under v{}",
                f.transition,
                if f.net == Some(Net::parse(NET_V1).unwrap().version_id()) {
                    1
                } else {
                    2
                }
            ))
            .collect::<Vec<_>>()
            .join(", "),
        verify.iter().filter(|v| v.1).count(),
        verify.len(),
        fsck.len()
    );
}

/// The loss log the run committed at `runs/r1/loss.log`.
fn loss_id_from<S: Store>(repo: &Repo<S>, head: &Id) -> Id {
    let mut t = ikam_kernel::snapshot::tree_of(repo, head).unwrap();
    for name in ["runs", "r1", "loss.log"] {
        let Object::Tree(es) = repo.get(&t).unwrap() else {
            panic!()
        };
        t = es.into_iter().find(|e| e.name == name).unwrap().id;
    }
    t
}

fn short(id: &Id) -> String {
    id.to_string()[..16].to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => run(&args[1..]),
        Some("reproduce") => reproduce(&args[1..]),
        _ => eprintln!(
            "usage: e427_sampler run TRAINER_BIN TRAINER_SRC DIR [EPOCHS] | reproduce TRAINER_BIN DIR"
        ),
    }
}
