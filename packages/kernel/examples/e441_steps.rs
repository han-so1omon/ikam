//! E441: step kinds at the user's choice (pure, pinned-float, recorded),
//! check steps that bound a rerun, and nets fired through steps.
//!
//! 1. Pinned-float steps: E409's five float functions on its six inputs,
//!    stored as WASM and applied as `wasm-float` derivations: bits against
//!    a native run with the same arithmetic, after a repack that drops and
//!    rebuilds them, NaNs, and cost.
//! 2. Reruns elsewhere: the same functions run natively with host
//!    arithmetic (fused multiply-add, eight-lane sums, the host's `exp`),
//!    judged by `within-f64` checks at several tolerances.
//! 3. Recorded steps (`Run` records) against plain content: bytes per
//!    record, record, lookup, rerun, fsck, gc and repack times.
//! 4. A net fired through steps against `net_fire_output`.
//!
//!   cargo run --release --example e441_steps [-- N]

#[path = "../tests/common/float_wat.rs"]
mod float_wat;

use std::time::Instant;

use float_wat::{Arith, FUNCS, INPUTS, bytes, f64s, inputs, native, wat};
use ikam_kernel::net::{Net, Step};
use ikam_kernel::{Arg, Check, Id, Kind, MemStore, Object, Repo, Store, TreeEntry, func};

fn ulps(a: f64, b: f64) -> u64 {
    let key = |x: f64| {
        let b = x.to_bits() as i64;
        if b < 0 { i64::MIN - b } else { b }
    };
    key(a).abs_diff(key(b))
}

fn max_ulp(a: &[f64], b: &[f64]) -> String {
    let mut m = 0u64;
    for (x, y) in a.iter().zip(b) {
        if x.to_bits() == y.to_bits() {
            continue;
        }
        if x.is_nan() && y.is_nan() {
            return "nan bits".into();
        }
        m = m.max(ulps(*x, *y));
    }
    m.to_string()
}

fn commit(repo: &mut Repo<MemStore>, ids: &[Id]) {
    let entries = (ids.iter().enumerate())
        .map(|(i, id)| TreeEntry {
            name: format!("f{i:06}"),
            kind: Kind::File,
            id: *id,
        })
        .collect();
    let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
}

fn size(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

struct Case {
    f: &'static str,
    input: &'static str,
    module: Id,
    args: Vec<Arg>,
    a: Vec<f64>,
    b: Vec<f64>,
}

fn cases(repo: &mut Repo<MemStore>) -> Vec<Case> {
    let mut out = Vec::new();
    for f in FUNCS {
        let module = repo
            .put_content(&wat::parse_str(wat(f)).unwrap())
            .unwrap()
            .id;
        for input in INPUTS {
            let n = if f == "matmul" { 64 } else { 256 };
            let (a, b) = inputs(input, n);
            let ia = repo.put_content(&bytes(&a)).unwrap().id;
            let ib = repo.put_content(&bytes(&b)).unwrap().id;
            let args = match f {
                "dot" | "matmul" => vec![Arg::Whole(ia), Arg::Whole(ib)],
                _ => vec![Arg::Whole(ia)],
            };
            out.push(Case {
                f,
                input,
                module,
                args,
                a,
                b,
            });
        }
    }
    out
}

fn part1_and_2() {
    println!("## 1. pinned-float steps (wasm-float derivations) against native pinned arithmetic");
    println!(
        "{:<9} {:<10} {:>8} {:>8} {:>8} {:>10} {:>9} {:>9}",
        "func", "input", "bits", "rebuilt", "nans", "host ulp", "wasm us", "native us"
    );
    let mut repo = Repo::new(MemStore::default());
    let cases = cases(&mut repo);
    let mut outs = Vec::new();
    let mut host_runs = Vec::new();
    for c in &cases {
        let t = Instant::now();
        let out = repo.apply_pinned(c.module, c.args.clone()).unwrap();
        let first_us = t.elapsed().as_secs_f64() * 1e6;
        let got = f64s(&repo.read_content(&out).unwrap());
        // Time a fresh evaluation (apply would hit the memo).
        let mut all = vec![Arg::Whole(c.module)];
        all.extend(c.args.clone());
        let t = Instant::now();
        let again = repo.evaluate(&func::pinned_float(), &all).unwrap().0;
        let wasm_us = t.elapsed().as_secs_f64() * 1e6;
        assert_eq!(again, bytes(&got));
        let t = Instant::now();
        let want = native(c.f, &c.a, &c.b, Arith::Pinned);
        let native_us = t.elapsed().as_secs_f64() * 1e6;
        let same = got
            .iter()
            .zip(&want)
            .all(|(x, y)| x.to_bits() == y.to_bits());
        // Native NaNs are not canonical; count a row as the same when only NaN bits differ.
        let nan_aside = got
            .iter()
            .zip(&want)
            .all(|(x, y)| x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()));
        let nans: Vec<f64> = got.iter().copied().filter(|x| x.is_nan()).collect();
        let canonical = nans.iter().all(|x| x.to_bits() == 0x7ff8_0000_0000_0000);
        let host = native(c.f, &c.a, &c.b, Arith::Host);
        let _ = first_us;
        outs.push((
            out,
            bytes(&got),
            c,
            same,
            nan_aside,
            nans.len(),
            canonical,
            wasm_us,
            native_us,
        ));
        host_runs.push((out, bytes(&host), max_ulp(&got, &host)));
    }
    commit(&mut repo, &outs.iter().map(|o| o.0).collect::<Vec<_>>());
    let before = size(&repo);
    let t = Instant::now();
    repo.repack().unwrap();
    let repack_ms = ms(t);
    let dropped = outs.iter().filter(|o| !repo.store().has(&o.0)).count();
    for (k, (out, want, c, same, nan_aside, nans, canonical, wasm_us, native_us)) in
        outs.iter().enumerate()
    {
        let rebuilt = repo.read_content(out).unwrap() == *want;
        let nan = match (nans, canonical) {
            (0, _) => "-".to_string(),
            (n, true) => format!("{n} canon"),
            (n, false) => format!("{n} OTHER"),
        };
        println!(
            "{:<9} {:<10} {:>8} {:>8} {:>8} {:>10} {:>9.1} {:>9.1}",
            c.f,
            c.input,
            if *same {
                "same"
            } else if *nan_aside {
                "NaN bits"
            } else {
                "DIFF"
            },
            rebuilt,
            nan,
            host_runs[k].2,
            wasm_us,
            native_us
        );
    }
    println!(
        "repack: {before} -> {} stored bytes in {repack_ms:.0} ms; {dropped} of {} outputs now rebuilt from their step; fsck_deep failures {}",
        size(&repo),
        outs.len(),
        repo.fsck_deep().unwrap().len()
    );

    println!("\n## 2. reruns with host arithmetic, judged by within-f64 check steps");
    let specs = [
        "ulp=1",
        "ulp=4",
        "ulp=64",
        "ulp=100000",
        "rel=1e-12",
        "abs=1e-300 rel=1e-9",
    ];
    print!("{:<9} {:<10}", "func", "input");
    for s in specs {
        print!(" {s:>19}");
    }
    println!();
    let spec_ids: Vec<Id> = specs
        .iter()
        .map(|s| repo.put_content(s.as_bytes()).unwrap().id)
        .collect();
    let mut verdicts = vec![0usize; specs.len()];
    for (k, (first, host, _)) in host_runs.iter().enumerate() {
        let now = repo.put_content(host).unwrap().id;
        print!("{:<9} {:<10}", outs[k].2.f, outs[k].2.input);
        for (j, s) in spec_ids.iter().enumerate() {
            let v = repo
                .apply(
                    func::within_f64(),
                    vec![Arg::Whole(*first), Arg::Whole(now), Arg::Whole(*s)],
                )
                .unwrap();
            let text = String::from_utf8(repo.read_content(&v).unwrap()).unwrap();
            let word = text.split_whitespace().next().unwrap().to_string();
            if word != "outside" {
                verdicts[j] += 1;
            }
            print!(" {word:>19}");
        }
        println!();
    }
    print!("{:<20}", "within or same");
    for (j, _) in specs.iter().enumerate() {
        print!(" {:>16}/{:<2}", verdicts[j], host_runs.len());
    }
    println!();
}

fn part3(n: usize) {
    println!("\n## 3. recorded steps: Run records against plain content, {n} steps");
    println!(
        "{:<8} {:>11} {:>10} {:>11} {:>10} {:>9} {:>9} {:>10} {:>11}",
        "design",
        "B/record",
        "record ms",
        "lookup ms",
        "rerun us",
        "fsck ms",
        "gc ms",
        "repack ms",
        "after rep."
    );
    for recorded in [false, true] {
        let mut repo = Repo::new(MemStore::default());
        let cmd = repo.put_content(b"cmd: measure").unwrap().id;
        let spec = repo.put_content(b"ulp=4").unwrap().id;
        let check = Check {
            func: func::within_f64(),
            args: vec![Arg::Whole(spec)],
        };
        let inputs: Vec<Id> = (0..n)
            .map(|i| {
                repo.put_content(format!("input {i}").as_bytes())
                    .unwrap()
                    .id
            })
            .collect();
        let base = size(&repo);
        let t = Instant::now();
        let mut recs = Vec::new();
        for (i, a) in inputs.iter().enumerate() {
            let out = bytes(&[i as f64, (i as f64).sqrt(), 1.0 / (1.0 + i as f64)]);
            recs.push(if recorded {
                repo.record_step(cmd, vec![Arg::Whole(*a)], Some(&out), Some(check.clone()))
                    .unwrap()
            } else {
                // Baseline: the same outputs stored as plain content, no record.
                (repo.put_content(&out).unwrap().id, Id::of(b"none"))
            });
        }
        let record_ms = ms(t);
        // Bytes beyond the inputs and outputs: the records.
        let outputs: usize = recs
            .iter()
            .map(|(o, _)| repo.store().read(o).unwrap().len())
            .sum();
        let per = (size(&repo) - base - outputs) as f64 / n as f64;
        let t = Instant::now();
        let want = usize::from(recorded);
        for (o, _) in recs.iter().take(20) {
            assert_eq!(repo.step_records(o).unwrap().len(), want);
        }
        let lookup_ms = ms(t) / 20.0;
        let t = Instant::now();
        for (i, (_, r)) in recs.iter().enumerate().take(200 * want) {
            let now = bytes(&[i as f64, (i as f64).sqrt(), 1.0 / (1.0 + i as f64) + 1e-17]);
            repo.rerun(r, Some(&now)).unwrap();
        }
        let rerun_us = t.elapsed().as_secs_f64() * 1e6 / 200.0;
        // The baseline commits the inputs too: a record keeps its inputs
        // live, so without them the baseline would repack half the content.
        let mut live: Vec<Id> = recs.iter().map(|r| r.0).collect();
        if !recorded {
            live.extend(&inputs);
        }
        commit(&mut repo, &live);
        let t = Instant::now();
        assert!(repo.fsck().unwrap().is_empty());
        let fsck_ms = ms(t);
        let t = Instant::now();
        repo.gc().unwrap();
        let gc_ms = ms(t);
        let t = Instant::now();
        repo.repack().unwrap();
        let repack_ms = ms(t);
        let ok = recs
            .iter()
            .all(|(o, r)| repo.store().has(o) && (want == 0 || repo.rerun(r, None).is_err()))
            && repo.fsck().unwrap().is_empty()
            && repo.step_records(&recs[0].0).unwrap().len() == want;
        println!(
            "{:<8} {per:>11.1} {record_ms:>10.0} {lookup_ms:>11.2} {rerun_us:>10.1} {fsck_ms:>9.0} {gc_ms:>9.0} {repack_ms:>10.0} {:>11}",
            if recorded { "Run" } else { "none" },
            if ok { "kept, fsck" } else { "FAILED" }
        );
    }
}

fn rec(func: Id, out: &[u8]) -> Step<'_> {
    Step::Recorded {
        func,
        args: vec![],
        output: Some(out),
        check: None,
    }
}

const LOOP: &str = "\
net loop 1
place cpu 1
place queued colored
place running colored
place measured colored
place waiting colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
transition ask pure: measured -> waiting
transition merge human: waiting ->
";

fn part4(n: usize) {
    println!("\n## 4. the loop net, {n} candidates: fired through steps against net_fire_output");
    let net = Net::parse(LOOP).unwrap();
    for through_steps in [false, true] {
        let mut repo = Repo::new(MemStore::default());
        let empty = repo.db_empty().unwrap();
        let mut db = repo.net_init(&empty, &net).unwrap();
        let agent = repo.put_content(b"agent: propose").unwrap().id;
        let bench = repo.put_content(b"cmd: bench").unwrap().id;
        let mv = repo
            .put_content(&wat::parse_str(wat("mean_var")).unwrap())
            .unwrap()
            .id;
        let base = size(&repo);
        let t = Instant::now();
        for i in 0..n {
            let tok = format!("e{i}");
            let idea = format!("idea {i}");
            let times = bytes(&[1.0 + i as f64, 1.5, 0.75 * i as f64, 2.0]);
            let run = repo.put_content(&times).unwrap().id;
            let summary = repo
                .evaluate(&func::pinned_float(), &[Arg::Whole(mv), Arg::Whole(run)])
                .unwrap()
                .0;
            if through_steps {
                (db, _) = repo
                    .net_fire_step(
                        &db,
                        &net,
                        "propose",
                        Some(&tok),
                        &format!("p{i}"),
                        rec(agent, idea.as_bytes()),
                    )
                    .unwrap();
                (db, _) = repo
                    .net_fire_step(
                        &db,
                        &net,
                        "start",
                        Some(&tok),
                        &format!("s{i}"),
                        rec(bench, &times),
                    )
                    .unwrap();
                let step = Step::Pure {
                    func: func::pinned_float(),
                    args: vec![Arg::Whole(mv), Arg::Whole(run)],
                };
                (db, _) = repo
                    .net_fire_step(&db, &net, "finish", Some(&tok), &format!("f{i}"), step)
                    .unwrap();
            } else {
                (db, _) = repo
                    .net_fire_output(
                        &db,
                        &net,
                        "propose",
                        Some(&tok),
                        &format!("p{i}"),
                        idea.as_bytes(),
                    )
                    .unwrap();
                (db, _) = repo
                    .net_fire_output(&db, &net, "start", Some(&tok), &format!("s{i}"), &times)
                    .unwrap();
                (db, _) = repo
                    .net_fire_output(&db, &net, "finish", Some(&tok), &format!("f{i}"), &summary)
                    .unwrap();
            }
        }
        let fire_us = t.elapsed().as_secs_f64() * 1e6 / (3 * n) as f64;
        let per = (size(&repo) - base) as f64 / (3 * n) as f64;
        let t = Instant::now();
        let replay_ok =
            repo.net_replay(&db, &net).unwrap() == repo.net_marking(&db, "loop").unwrap();
        let replay_ms = ms(t);
        let t = Instant::now();
        let checks = repo.net_verify_steps(&db, "loop").unwrap();
        let verify_ms = ms(t);
        let passed = checks.iter().filter(|c| c.1).count();
        let t = Instant::now();
        let fsck = repo.fsck().unwrap().len();
        let fsck_ms = ms(t);
        println!(
            "{:<16} fire {fire_us:>6.0} us, {per:>6.0} B/firing, replay {} in {replay_ms:.0} ms, steps checked {passed}/{} in {verify_ms:.0} ms, fsck {} in {fsck_ms:.0} ms",
            if through_steps {
                "through steps"
            } else {
                "net_fire_output"
            },
            if replay_ok { "equal" } else { "DIFFERENT" },
            checks.len(),
            if fsck == 0 { "clean" } else { "FAILED" },
        );
    }
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000);
    part1_and_2();
    part3(n);
    part4(n / 10);
}
