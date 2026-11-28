//! E448: bytes each new version of a fragment net adds.
//!
//!   cargo run --release --example e448_net_chunk_deltas -- MODE ARG ROWS
//!
//! MODE anatomy: what one version writes (ARG sizes, ROWS options).
//! MODE nets: E446's measurement (ARG sizes, ROWS `text|tree[+option...]`;
//!   E448_REPACK=1000,10000 repacks those sizes).
//! MODE graphs: general graphs (ARG `links,links-small,props`, ROWS as nets).
//!
//! Options: `contexts` (`Repo::object_contexts`), `short` (`short_records`),
//! `leaf=MIN:MAX` and `parent=MIN:MAX` (`Repo::graph_chunks`).

mod anatomy;
mod chain;
mod graphs;
mod measure;

use std::collections::HashSet;

use ikam_kernel::net::Layout;
use ikam_kernel::{Id, MemStore, Repo, Store};

const PATH: &str = "nets/s.net";

fn size(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

/// Apply the named options (`+`-separated) to a fresh repo.
fn configure(repo: &mut Repo<MemStore>, opt: &str) {
    for o in opt.split('+') {
        match o {
            "base" => {}
            "contexts" => repo.object_contexts = true,
            "short" => repo.short_records = true,
            o if o.starts_with("leaf=") || o.starts_with("parent=") => {
                let (k, v) = o.split_once('=').unwrap();
                let (a, b) = v.split_once(':').unwrap();
                let bounds = (a.parse().unwrap(), b.parse().unwrap());
                match k {
                    "leaf" => repo.graph_chunks.leaf = bounds,
                    _ => repo.graph_chunks.parent = bounds,
                }
            }
            _ => panic!("unknown option {o}"),
        }
    }
}

fn run_anatomy(n: usize, weight: f64, opt: &str) {
    let nets = chain::chain(n);
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = weight;
    configure(&mut repo, opt);
    let (mut tree, mut prev): (Option<Id>, Option<Id>) = (None, None);
    let mut total = anatomy::Anatomy::default();
    let mut adds = Vec::new();
    for (k, net) in nets.iter().enumerate() {
        let before: HashSet<Id> = repo.store().ids().unwrap().into_iter().collect();
        let sizes: std::collections::HashMap<Id, usize> = before
            .iter()
            .map(|i| (*i, repo.store().read(i).unwrap().len()))
            .collect();
        let b0 = size(&repo);
        let (t, root) = repo
            .put_net_fragment_file(tree, PATH, net, Layout::Tree)
            .unwrap();
        tree = Some(t);
        let grown = size(&repo) - b0;
        let after: HashSet<Id> = repo.store().ids().unwrap().into_iter().collect();
        let gone: usize = before.difference(&after).map(|i| sizes[i]).sum();
        if std::env::var("E448_DEBUG").is_ok() {
            eprintln!("v{} grown {grown} gone {gone}", k + 1);
        }
        let frags: HashSet<Id> = net
            .to_text()
            .split_inclusive('\n')
            .map(|l| Id::of_content(l.as_bytes()))
            .collect();
        let a = anatomy::anatomy(&repo, &before, &root, prev.as_ref(), &frags);
        if k > 0 {
            adds.push(grown);
            total.add(&a);
        }
        repo.commit("main", t, "v").unwrap();
        prev = Some(root);
    }
    let v = (nets.len() - 1) as f64;
    println!(
        "\n### n={n} weight={weight} {opt}: a version adds {:.0} B (mean of {})",
        adds.iter().sum::<usize>() as f64 / v,
        adds.len()
    );
    println!("| kind | objects a version | bytes a version |");
    println!("| --- | --- | --- |");
    for (k, (c, b)) in &total.kinds {
        println!("| {k} | {:.1} | {:.0} |", *c as f64 / v, *b as f64 / v);
    }
    println!("| level | chunks a version | canonical bytes | already held (prefix+suffix) |");
    println!("| --- | --- | --- | --- |");
    for (h, (c, b, s)) in &total.levels {
        println!(
            "| {h} | {:.1} | {:.0} | {:.0} ({:.1}%) |",
            *c as f64 / v,
            *b as f64 / v,
            *s as f64 / v,
            100.0 * *s as f64 / (*b).max(1) as f64
        );
    }
    let (c, r, reused, lit) = total.sliced;
    println!(
        "- chunk contents stored as slices: {:.1} a version, {:.1} ranges each, {:.1} reused, {:.0} literal bytes a version",
        c as f64 / v,
        r as f64 / c.max(1) as f64,
        reused as f64 / c.max(1) as f64,
        lit as f64 / v
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "anatomy".into());
    let second = args.next();
    let sizes: Vec<usize> = match mode.as_str() {
        "graphs" => Vec::new(),
        _ => second
            .as_ref()
            .map(|s| s.split(',').map(|x| x.parse().unwrap()).collect())
            .unwrap_or_else(|| vec![1000, 10000]),
    };
    let args_opts: Vec<String> = args
        .next()
        .map(|s| s.split(',').map(String::from).collect())
        .unwrap_or_else(|| vec!["base".into(), "contexts".into()]);
    if mode == "graphs" {
        // SIZES here names the graphs: links, links-small, props.
        let which = second.unwrap_or_else(|| "links,props".into());
        for w in which.split(',') {
            graphs::run(w, &args_opts);
        }
        return;
    }
    if mode == "reads" {
        for &n in &sizes {
            measure::reads(n, &args_opts);
        }
        return;
    }
    if mode == "repack" {
        for &n in &sizes {
            for opt in &args_opts {
                repack_tags(n, opt);
            }
        }
        return;
    }
    if mode == "nets" {
        let repack: Vec<usize> = std::env::var("E448_REPACK")
            .ok()
            .map(|s| s.split(',').map(|x| x.parse().unwrap()).collect())
            .unwrap_or_default();
        nets(&sizes, &args_opts, &repack);
        return;
    }
    if mode == "anatomy" {
        for &n in &sizes {
            for w in [ikam_kernel::DEFAULT_READ_WEIGHT, 48.8] {
                for opt in args_opts.iter() {
                    run_anatomy(n, w, opt);
                }
            }
        }
    }
}

/// Rows: `text` or `tree` first, then options (`text`, `tree+contexts`, ...).
fn nets(sizes: &[usize], rows: &[String], repack: &[usize]) {
    let mut out = Vec::new();
    for &n in sizes {
        let nets = chain::chain(n);
        for r in rows {
            let (form, opt) = match r.split_once('+') {
                Some((f, o)) => (f, o.to_string()),
                None => (r.as_str(), "base".to_string()),
            };
            let form = match form {
                "text" => measure::Form::Text,
                "tree" => measure::Form::Tree,
                _ => panic!("form {form}"),
            };
            let at = std::time::Instant::now();
            let mut row = measure::measure(form, &opt, &nets, repack.contains(&n));
            row.label = r.clone();
            eprintln!("  {n} {r} done in {:.1} s", at.elapsed().as_secs_f64());
            out.push(row);
        }
    }
    println!("## stored bytes, {} versions\n", chain::VERSIONS);
    println!(
        "| transitions | row | bytes v1 | a version adds, mean / max | objects reused | bytes reused | heavy: v1, a version adds | after repack (s) | chunks in v25 |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for r in &out {
        let mean = r.added.iter().sum::<usize>() as f64 / r.added.len() as f64;
        println!(
            "| {} | {} | {} | {:.0} / {} | {:.1}% | {:.1}% | {}, {:.0} | {} | {} |",
            r.n,
            r.label,
            r.first,
            mean,
            r.added.iter().max().unwrap(),
            100.0 * r.reuse_objects.0 as f64 / r.reuse_objects.1.max(1) as f64,
            100.0 * r.reuse_bytes.0 as f64 / r.reuse_bytes.1.max(1) as f64,
            r.heavy.0,
            r.heavy.1,
            match r.repack {
                Some((b, s)) => format!("{b} ({s:.1})"),
                None => "not run".into(),
            },
            r.chunks
        );
    }
    println!("\n## time (µs, median of 3 fastest-of runs)\n");
    println!(
        "| transitions | row | write a version | read v1 | read v25 | read v25, heavy store | delta | delta chunks (unpruned) | applied exactly | fsck problems |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for r in &out {
        println!(
            "| {} | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.1} ({:.1}) | {}/{} | {} |",
            r.n,
            r.label,
            r.write_us,
            r.read_first_us,
            r.read_last_us,
            r.heavy_read_last_us,
            r.delta_us,
            r.delta_chunks,
            r.unpruned_chunks,
            r.applied.0,
            r.applied.1,
            r.fsck
        );
    }
}

/// Stored bytes by first byte of the encoding, before and after a repack.
fn repack_tags(n: usize, opt: &str) {
    let nets = chain::chain(n);
    let mut repo = Repo::new(MemStore::default());
    configure(&mut repo, opt);
    let mut tree = None;
    for net in &nets {
        let (t, _) = repo
            .put_net_fragment_file(tree, PATH, net, Layout::Tree)
            .unwrap();
        tree = Some(t);
        repo.commit("main", t, "v").unwrap();
    }
    let tags = |repo: &Repo<MemStore>| {
        let mut m: std::collections::BTreeMap<char, (usize, usize)> = Default::default();
        for id in repo.store().ids().unwrap() {
            let e = repo.store().read(&id).unwrap();
            let x = m.entry(e[0] as char).or_default();
            x.0 += 1;
            x.1 += e.len();
        }
        m
    };
    let before = tags(&repo);
    let r = repo.repack().unwrap();
    let after = tags(&repo);
    println!(
        "\n### repack n={n} {opt}: {r:?}, {} -> {}",
        size_of(&before),
        size_of(&after)
    );
    for k in before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        println!("| {k} | {:?} | {:?} |", before.get(k), after.get(k));
    }
}

fn size_of(m: &std::collections::BTreeMap<char, (usize, usize)>) -> usize {
    m.values().map(|v| v.1).sum()
}
