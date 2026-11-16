//! E333: the best ranking measured so far (E330: BM25 fused with a
//! README's best paragraph and its whole vector) with both vector lists
//! from graph indexes (E332) instead of exact scans, and its time a
//! query end to end: BM25, two graph searches, the rescore and fusion.
//!
//! `cargo run --release --example hybrid_graph -- DIR`
//!
//! DIR is what `prepare.py` writes (see `hybrid_kernel` and
//! `fragment_vectors`). Nothing from it is committed. `THREADS=n` builds
//! the graphs on n threads (default 4); `CODES=1` keeps int8 codes in
//! them (E334). The graph settings score candidates from the graphs'
//! own rows; the exact reference from the example's copy.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use ikam_kernel::fusion::convex;
use ikam_kernel::hnsw::HnswIndex;
use ikam_kernel::vectors::{dot_lanes, from_f32s, to_f32s};
use ikam_kernel::{Event, MemStore, Repo};

/// A ranked list of keys.
type Ranked = Vec<(String, f32)>;
/// Task, split, relevant keys, text, vector.
type Query = (String, String, BTreeSet<String>, String, Vec<f32>);

fn read_f32s(path: &str, dim: usize) -> Vec<Vec<f32>> {
    to_f32s(&std::fs::read(path).unwrap())
        .chunks_exact(dim)
        .map(<[f32]>::to_vec)
        .collect()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn ndcg10(ranked: &[(String, f32)], rel: &BTreeSet<String>) -> f64 {
    let dcg: f64 = ranked
        .iter()
        .take(10)
        .enumerate()
        .filter(|(_, r)| rel.contains(&r.0))
        .map(|(i, _)| 1.0 / ((i + 2) as f64).log2())
        .sum();
    let ideal: f64 = (0..rel.len().min(10))
        .map(|i| 1.0 / ((i + 2) as f64).log2())
        .sum();
    dcg / ideal
}

fn top(mut v: Ranked, k: usize) -> Ranked {
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(k);
    v
}

/// Best paragraph plus whole (E330): over the documents in `cands`, the
/// mean of the whole README's score and its best paragraph's.
fn both(
    cands: &BTreeSet<usize>,
    q: &[f32],
    frags_of: &[Vec<usize>],
    fvec: &[Vec<f32>],
    whole: &[Vec<f32>],
    keys: &[String],
) -> Ranked {
    let scored = cands
        .iter()
        .map(|&d| {
            let b = frags_of[d]
                .iter()
                .map(|&f| dot_lanes(q, &fvec[f]))
                .fold(f32::MIN, f32::max);
            (keys[d].clone(), 0.5 * b + 0.5 * dot_lanes(q, &whole[d]))
        })
        .collect();
    top(scored, 100)
}

/// `both` with each score as a graph keeps it (E334: from codes, with
/// `CODES=1`), so no second copy of the vectors is needed.
fn both_kept(
    cands: &BTreeSet<usize>,
    q: &[f32],
    frags_of: &[Vec<usize>],
    gf: &HnswIndex,
    gw: &HnswIndex,
    keys: &[String],
    names: (&[String], &[String]),
) -> Ranked {
    let scored = cands
        .iter()
        .map(|&d| {
            let b = frags_of[d]
                .iter()
                .filter_map(|&f| gf.score(q, &names.0[f]))
                .fold(f32::MIN, f32::max);
            let w = gw.score(q, &names.1[d]).unwrap_or(0.0);
            (keys[d].clone(), 0.5 * b + 0.5 * w)
        })
        .collect();
    top(scored, 100)
}

/// `both_kept` with each row's graph node found once beforehand (E337),
/// so a query scores by position instead of looking each key up.
fn both_at(
    cands: &BTreeSet<usize>,
    q: &[f32],
    frags_of: &[Vec<usize>],
    gf: &HnswIndex,
    gw: &HnswIndex,
    keys: &[String],
    nodes: (&[Option<u32>], &[Option<u32>]),
) -> Ranked {
    let scored = cands
        .iter()
        .map(|&d| {
            let b = frags_of[d]
                .iter()
                .filter_map(|&f| gf.score_at(q, nodes.0[f]?))
                .fold(f32::MIN, f32::max);
            let w = nodes.1[d].and_then(|n| gw.score_at(q, n)).unwrap_or(0.0);
            (keys[d].clone(), 0.5 * b + 0.5 * w)
        })
        .collect();
    top(scored, 100)
}

fn main() {
    let dir = std::env::args().nth(1).expect("DIR");
    let threads: usize = std::env::var("THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(4);
    let codes = std::env::var("CODES").as_deref() == Ok("1");
    let docs: Vec<(String, Vec<String>)> = std::fs::read_to_string(format!("{dir}/docs.tsv"))
        .unwrap()
        .lines()
        .map(|l| {
            let (k, t) = l.split_once('\t').unwrap();
            (k.to_string(), t.split('\u{1e}').map(String::from).collect())
        })
        .collect();
    let keys: Vec<String> = docs.iter().map(|d| d.0.clone()).collect();
    let pos: HashMap<&str, usize> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.as_str(), i))
        .collect();
    let raw = std::fs::read(format!("{dir}/doc_whole.f32")).unwrap();
    let dim = raw.len() / 4 / docs.len();
    let whole = read_f32s(&format!("{dir}/doc_whole.f32"), dim);
    let frag_text: Vec<String> = std::fs::read_to_string(format!("{dir}/frags.tsv"))
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    let fvec = read_f32s(&format!("{dir}/frags.f32"), dim);
    let index: HashMap<&str, usize> = frag_text
        .iter()
        .enumerate()
        .map(|(i, t)| (t.as_str(), i))
        .collect();
    let mut owner: Vec<Vec<usize>> = vec![Vec::new(); fvec.len()];
    for (d, (_, paras)) in docs.iter().enumerate() {
        for p in paras {
            if let Some(&f) = index.get(p.as_str())
                && owner[f].last() != Some(&d)
            {
                owner[f].push(d);
            }
        }
    }
    let mut frags_of: Vec<Vec<usize>> = vec![Vec::new(); docs.len()];
    for (f, ds) in owner.iter().enumerate() {
        for &d in ds {
            frags_of[d].push(f);
        }
    }
    let qvec = read_f32s(&format!("{dir}/queries.f32"), dim);
    let queries: Vec<Query> = std::fs::read_to_string(format!("{dir}/queries.tsv"))
        .unwrap()
        .lines()
        .zip(qvec)
        .map(|(l, v)| {
            let f: Vec<&str> = l.splitn(5, '\t').collect();
            (
                f[0].into(),
                f[2].into(),
                f[3].split(',').map(String::from).collect(),
                f[4].into(),
                v,
            )
        })
        .collect();
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let mut events: Vec<Event> = docs
        .iter()
        .map(|(k, p)| Event::upsert(&format!("t/{k}"), p.join("\n").as_bytes(), &format!("t{k}")))
        .collect();
    events.extend(
        whole
            .iter()
            .zip(&keys)
            .map(|(v, k)| Event::upsert(&format!("w/{k}"), &from_f32s(v), &format!("w{k}"))),
    );
    events.extend(
        fvec.iter()
            .enumerate()
            .map(|(i, v)| Event::upsert(&format!("f/{i:07}"), &from_f32s(v), &format!("f{i}"))),
    );
    let db = repo.db_append(&empty, &events).unwrap();
    drop(events);
    let bid = repo.bm25_build(&db, "t/").unwrap();
    let bm25 = repo.bm25_load(&bid).unwrap();
    let graph = |repo: &mut Repo<MemStore>, prefix: &str| {
        let t = Instant::now();
        let (_, g) = repo
            .hnsw_build(
                &db,
                prefix,
                HnswIndex::new(dim, 16, 100)
                    .with_lanes(true)
                    .with_threads(threads)
                    .with_codes(codes),
            )
            .unwrap();
        (g, ms(t))
    };
    let (mut gw, gw_ms) = graph(&mut repo, "w/");
    let (mut gf, gf_ms) = graph(&mut repo, "f/");
    println!(
        "{{\"docs\":{},\"paragraphs\":{},\"queries\":{},\"threads\":{threads},\"codes\":{codes},\"graph_docs_build_ms\":{gw_ms:.0},\"graph_paragraphs_build_ms\":{gf_ms:.0}}}",
        docs.len(),
        fvec.len(),
        queries.len()
    );
    // Settings: (whole ef, paragraphs returned, paragraph ef).
    let settings = [(80, 100, 100), (80, 200, 200), (160, 400, 400)];
    let nq = queries.len() as f64;
    let mut lists: Vec<HashMap<String, Ranked>> = vec![HashMap::new(); queries.len()];
    let (mut t_bm25, mut t_exact) = (0.0, 0.0);
    for (i, (_, _, _, text, q)) in queries.iter().enumerate() {
        let t = Instant::now();
        let kw: Ranked = bm25
            .search(text, 100)
            .into_iter()
            .map(|(k, s)| (k.trim_start_matches("t/").to_string(), s))
            .collect();
        t_bm25 += ms(t);
        // Reference: both lists exactly (E330's best+whole).
        let t = Instant::now();
        let w = top(
            whole
                .iter()
                .zip(&keys)
                .map(|(v, k)| (k.clone(), dot_lanes(q, v)))
                .collect(),
            100,
        );
        let mut all: Vec<(usize, f32)> = fvec
            .iter()
            .enumerate()
            .map(|(i, v)| (i, dot_lanes(q, v)))
            .collect();
        all.select_nth_unstable_by(2000, |a, b| b.1.total_cmp(&a.1));
        all.truncate(2000);
        let mut cands: BTreeSet<usize> = w.iter().map(|(k, _)| pos[k.as_str()]).collect();
        let mut best: HashMap<usize, f32> = HashMap::new();
        for &(f, s) in &all {
            for &d in &owner[f] {
                let e = best.entry(d).or_insert(f32::MIN);
                *e = e.max(s);
            }
        }
        let best = top(
            best.into_iter()
                .map(|(d, s)| (keys[d].clone(), s))
                .collect(),
            100,
        );
        cands.extend(best.iter().map(|(k, _)| pos[k.as_str()]));
        let exact = both(&cands, q, &frags_of, &fvec, &whole, &keys);
        t_exact += ms(t);
        lists[i].insert("exact".into(), exact);
        lists[i].insert("bm25".into(), kw);
    }
    println!(
        "{{\"bm25_ms\":{:.3},\"exact_vectors_ms\":{:.3}}}",
        t_bm25 / nq,
        t_exact / nq
    );
    // Row keys of each paragraph and README, made once.
    let fkeys: Vec<String> = (0..fvec.len()).map(|f| format!("f/{f:07}")).collect();
    let wkeys: Vec<String> = keys.iter().map(|k| format!("w/{k}")).collect();
    let fnodes: Vec<Option<u32>> = fkeys.iter().map(|k| gf.position(k)).collect();
    let wnodes: Vec<Option<u32>> = wkeys.iter().map(|k| gw.position(k)).collect();
    let mut names = vec!["exact".to_string()];
    for (&(wef, pk, pef), at) in settings.iter().flat_map(|s| [(s, 0), (s, 1), (s, 2)]) {
        // 0 by key (E334), 1 by position, 2 by position with each
        // document's best paragraph taken from the paragraphs the graph
        // returned (a document with none gets the lowest of them).
        let name = format!("graph-w{wef}-p{pk}{}", ["", "-at", "-hits"][at]);
        let (mut t_all, mut recall) = (0.0, 0.0);
        // E337: time of each step, and candidates scored.
        let mut phase = [0.0f64; 5];
        let mut scored = (0usize, 0usize);
        for (i, (_, _, _, text, q)) in queries.iter().enumerate() {
            let t = Instant::now();
            let kw: Ranked = bm25
                .search(text, 100)
                .into_iter()
                .map(|(k, s)| (k.trim_start_matches("t/").to_string(), s))
                .collect();
            phase[0] += ms(t);
            let s = Instant::now();
            let mut cands: BTreeSet<usize> = gw
                .search(q, 100, wef)
                .into_iter()
                .map(|(k, _)| pos[&k[2..]])
                .collect();
            phase[1] += ms(s);
            let s = Instant::now();
            let mut hits: HashMap<usize, f32> = HashMap::new();
            let mut floor = f32::MAX;
            for (k, sc) in gf.search(q, pk, pef) {
                let f: usize = k[2..].parse().unwrap();
                cands.extend(&owner[f]);
                floor = floor.min(sc);
                for &d in &owner[f] {
                    let e = hits.entry(d).or_insert(f32::MIN);
                    *e = e.max(sc);
                }
            }
            phase[2] += ms(s);
            let s = Instant::now();
            scored.0 += cands.len();
            if at < 2 {
                scored.1 += cands.iter().map(|&d| frags_of[d].len()).sum::<usize>();
            }
            let v = match at {
                0 => both_kept(&cands, q, &frags_of, &gf, &gw, &keys, (&fkeys, &wkeys)),
                1 => both_at(&cands, q, &frags_of, &gf, &gw, &keys, (&fnodes, &wnodes)),
                _ => top(
                    cands
                        .iter()
                        .map(|&d| {
                            let b = hits.get(&d).copied().unwrap_or(floor);
                            let w = wnodes[d].and_then(|n| gw.score_at(q, n)).unwrap_or(0.0);
                            (keys[d].clone(), 0.5 * b + 0.5 * w)
                        })
                        .collect(),
                    100,
                ),
            };
            phase[3] += ms(s);
            let s = Instant::now();
            let _ = convex(&kw, &v, 0.6);
            phase[4] += ms(s);
            t_all += ms(t);
            let e: BTreeSet<&String> = lists[i]["exact"].iter().take(10).map(|x| &x.0).collect();
            recall += v.iter().take(10).filter(|x| e.contains(&x.0)).count() as f64 / 10.0;
            lists[i].insert(name.clone(), v);
        }
        println!(
            "{{\"setting\":\"{name}\",\"paragraph_ef\":{pef},\"end_to_end_ms\":{:.3},\"recall10_vs_exact\":{:.3}}}",
            t_all / nq,
            recall / nq
        );
        println!(
            "{{\"setting\":\"{name}\",\"ms\":{{\"bm25\":{:.3},\"docs_graph\":{:.3},\"paragraphs_graph\":{:.3},\"score_candidates\":{:.3},\"fuse\":{:.3}}},\"candidates\":{:.0},\"paragraphs_scored\":{:.0}}}",
            phase[0] / nq,
            phase[1] / nq,
            phase[2] / nq,
            phase[3] / nq,
            phase[4] / nq,
            scored.0 as f64 / nq,
            scored.1 as f64 / nq
        );
        names.push(name);
    }
    for task in ["known", "topic"] {
        for split in ["dev", "test"] {
            let idx: Vec<usize> = (0..queries.len())
                .filter(|&i| queries[i].0 == task && queries[i].1 == split)
                .collect();
            let stat = |f: &dyn Fn(usize) -> Ranked| {
                let s: Vec<f64> = idx.iter().map(|&i| ndcg10(&f(i), &queries[i].2)).collect();
                let m = s.iter().sum::<f64>() / s.len() as f64;
                let sd = (s.iter().map(|x| (x - m).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
                format!("[{m:.4},{:.4}]", sd / (s.len() as f64).sqrt())
            };
            println!(
                "{{\"task\":\"{task}\",\"split\":\"{split}\",\"queries\":{},\"bm25\":{}}}",
                idx.len(),
                stat(&|i| lists[i]["bm25"].clone())
            );
            for s in &names {
                let mut row = format!(
                    "{{\"task\":\"{task}\",\"split\":\"{split}\",\"vectors\":\"{s}\",\"alone\":{}",
                    stat(&|i| lists[i][s].clone())
                );
                for a in [0.5f32, 0.6, 0.7] {
                    row += &format!(
                        ",\"convex-{a}\":{}",
                        stat(&|i| convex(&lists[i]["bm25"], &lists[i][s], a))
                    );
                }
                println!("{row}}}");
            }
        }
    }
}
