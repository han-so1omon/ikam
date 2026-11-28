//! E443: step cache key rules (Bazel's `Action` fields, DVC's run cache).
//!
//! E428's runs of the training step (`experiments/step-cache`, three builds
//! of one source standing in for platforms), then one case per rule, each
//! with the hit or miss it must give:
//! - timeout in the key: 60 s, 60 s again, 10 s, 20 ms (the run is killed
//!   and nothing is filed), 20 ms again, 60 s;
//! - a salt that disowns every earlier result: on two platforms, then the
//!   old key again (its results are still there);
//! - do not cache: a step that is different on every run (`jitter=clock`),
//!   marked and then unmarked;
//! - exact-match platform properties, with what ran the step stored.
//!
//! Then gc, repack and fsck, counting refs and recorded steps before and
//! after: nothing may be deleted.
//!
//!   cargo run --release --example e443_cache_keys -- WORK PLAIN V3 FUSED

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ikam_kernel::stepcache::{STEP_CACHE_REFS, StepKey, StepKind};
use ikam_kernel::{Arg, Error, FsStore, Id, Kind, Object, Repo, Store, TreeEntry};

const N: usize = 4096;
const D: usize = 64;
const CONFIG: &str = "iters=400 lr=0.05";
const JITTER: &str = "iters=400 lr=0.05 jitter=clock";

/// E428's seeded rows.
fn data() -> Vec<u8> {
    let mut s: u64 = 0x2545_f491_4f6c_dd1d;
    let mut unit = || {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let truth: Vec<f64> = (0..D).map(|j| ((j % 7) as f64 - 3.0) / 4.0).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&(N as u64).to_le_bytes());
    out.extend_from_slice(&(D as u64).to_le_bytes());
    for _ in 0..N {
        let x: Vec<f64> = (0..D).map(|_| 2.0 * unit() - 1.0).collect();
        let mut y = 0.01 * (unit() - 0.5);
        for (a, b) in x.iter().zip(&truth) {
            y += a * b;
        }
        for v in x.iter().chain([&y]) {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// Run the training command, killed at `timeout`.
fn command(bin: &Path, data: &Path, config: &Path, timeout: Duration) -> Result<Vec<u8>, Error> {
    let mut child = Command::new(bin)
        .arg(data)
        .arg(config)
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > timeout {
            child.kill()?;
            child.wait()?;
            return Err(Error::Exec(format!("timed out after {timeout:?}")));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(Error::Exec(format!("train-gd: {}", out.status)));
    }
    Ok(out.stdout)
}

/// A build's name for `ran-by`: its file's hash (no machine names).
fn build_name(bin: &Path) -> String {
    let bytes = std::fs::read(bin).unwrap();
    format!("train-gd {}", &blake3::hash(&bytes).to_hex()[..12])
}

struct Case {
    group: &'static str,
    what: String,
    expect: &'static str,
}

fn main() -> Result<(), Error> {
    let a: Vec<String> = std::env::args().collect();
    let [_, work, plain, v3, fused] = &a[..] else {
        panic!("usage: e443_cache_keys WORK PLAIN V3 FUSED");
    };
    let (plain, v3, fused) = (
        PathBuf::from(plain),
        PathBuf::from(v3),
        PathBuf::from(fused),
    );
    let work = PathBuf::from(work);
    let store = work.join("store");
    let _ = std::fs::remove_dir_all(&store);
    let inputs = work.join("inputs");
    std::fs::create_dir_all(&inputs)?;
    let mut repo = Repo::new(FsStore::open(&store)?);

    let source = include_bytes!("../experiments/step-cache/src/main.rs");
    let func = repo.put_content(source)?.id;
    let data = data();
    let data_id = repo.put_content(&data)?.id;
    let config_id = repo.put_content(CONFIG.as_bytes())?.id;
    let jitter_id = repo.put_content(JITTER.as_bytes())?.id;
    let (data_path, config_path, jitter_path) = (
        inputs.join("data"),
        inputs.join("config"),
        inputs.join("jitter"),
    );
    std::fs::write(&data_path, &data)?;
    std::fs::write(&config_path, CONFIG)?;
    std::fs::write(&jitter_path, JITTER)?;
    let step = |config: Id, platform: &str| {
        StepKey::new(
            func,
            vec![Arg::Whole(data_id), Arg::Whole(config)],
            StepKind::Recorded,
            platform,
        )
    };
    let v3_props = [("arch", "x86_64"), ("cpu", "x86-64-v3"), ("fma", "on")];

    // (case, key, build, which config, ran-by override)
    type Plan = (Case, StepKey, PathBuf, bool);
    let c = |group, what: &str, expect| Case {
        group,
        what: what.into(),
        expect,
    };
    let base = |t| step(config_id, t);
    let plan: Vec<Plan> = vec![
        (
            c("E428", "`x86-64-plain`", "miss"),
            base("x86-64-plain"),
            plain.clone(),
            false,
        ),
        (
            c("E428", "`x86-64-plain` again", "hit"),
            base("x86-64-plain"),
            plain.clone(),
            false,
        ),
        (
            c("E428", "`x86-64-v3`", "miss"),
            base("x86-64-v3"),
            v3.clone(),
            false,
        ),
        (
            c("E428", "`x86-64-v3-fma`", "miss"),
            base("x86-64-v3-fma"),
            fused.clone(),
            false,
        ),
        (
            c("E428", "`x86-64-v3-fma` again", "hit"),
            base("x86-64-v3-fma"),
            fused.clone(),
            false,
        ),
        (
            c("timeout", "60 s", "miss"),
            base("x86-64-plain").with_timeout_ms(60_000),
            plain.clone(),
            false,
        ),
        (
            c("timeout", "60 s again", "hit"),
            base("x86-64-plain").with_timeout_ms(60_000),
            plain.clone(),
            false,
        ),
        (
            c("timeout", "10 s", "miss"),
            base("x86-64-plain").with_timeout_ms(10_000),
            plain.clone(),
            false,
        ),
        (
            c("timeout", "20 ms", "timed out"),
            base("x86-64-plain").with_timeout_ms(20),
            plain.clone(),
            false,
        ),
        (
            c("timeout", "20 ms again", "timed out"),
            base("x86-64-plain").with_timeout_ms(20),
            plain.clone(),
            false,
        ),
        (
            c("timeout", "60 s once more", "hit"),
            base("x86-64-plain").with_timeout_ms(60_000),
            plain.clone(),
            false,
        ),
        (
            c("salt", "salt `fixed-1`, `x86-64-plain`", "miss"),
            base("x86-64-plain").with_salt("fixed-1"),
            plain.clone(),
            false,
        ),
        (
            c("salt", "salt `fixed-1`, `x86-64-v3-fma`", "miss"),
            base("x86-64-v3-fma").with_salt("fixed-1"),
            fused.clone(),
            false,
        ),
        (
            c("salt", "salt `fixed-1`, `x86-64-plain` again", "hit"),
            base("x86-64-plain").with_salt("fixed-1"),
            plain.clone(),
            false,
        ),
        (
            c("salt", "no salt, `x86-64-plain` (the old key)", "hit"),
            base("x86-64-plain"),
            plain.clone(),
            false,
        ),
        (
            c("do not cache", "`jitter=clock`, marked", "miss"),
            step(jitter_id, "x86-64-plain").not_cached(),
            plain.clone(),
            true,
        ),
        (
            c("do not cache", "`jitter=clock`, marked, again", "miss"),
            step(jitter_id, "x86-64-plain").not_cached(),
            plain.clone(),
            true,
        ),
        (
            c("do not cache", "`jitter=clock`, unmarked", "miss"),
            step(jitter_id, "x86-64-plain"),
            plain.clone(),
            true,
        ),
        (
            c("do not cache", "`jitter=clock`, unmarked, again", "hit"),
            step(jitter_id, "x86-64-plain"),
            plain.clone(),
            true,
        ),
        (
            c("properties", "arch, cpu=x86-64-v3, fma=on", "miss"),
            step(config_id, "").with_properties(&v3_props),
            fused.clone(),
            false,
        ),
        (
            c(
                "properties",
                "the same, another order, another runner",
                "hit",
            ),
            step(config_id, "").with_properties(&[
                ("fma", "on"),
                ("cpu", "x86-64-v3"),
                ("arch", "x86_64"),
            ]),
            v3.clone(),
            false,
        ),
        (
            c("properties", "arch, cpu=x86-64-v3 (fma left out)", "miss"),
            step(config_id, "").with_properties(&v3_props[..2]),
            v3.clone(),
            false,
        ),
        (
            c("properties", "arch, cpu=x86-64-v3, fma=off", "miss"),
            step(config_id, "").with_properties(&[
                ("arch", "x86_64"),
                ("cpu", "x86-64-v3"),
                ("fma", "off"),
            ]),
            v3.clone(),
            false,
        ),
        (
            c(
                "properties",
                "arch, cpu=x86-64-v3, fma=on, os=linux",
                "miss",
            ),
            step(config_id, "").with_properties(&[
                ("arch", "x86_64"),
                ("cpu", "x86-64-v3"),
                ("fma", "on"),
                ("os", "linux"),
            ]),
            fused.clone(),
            false,
        ),
    ];

    println!("| # | rule | call | expected | got | ms | output | as expected |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    let (mut ok, mut outputs) = (0, Vec::<Id>::new());
    let mut jitter_outs = Vec::new();
    for (i, (case, key, bin, jitter)) in plan.iter().enumerate() {
        let timeout = Duration::from_millis(key.timeout_ms.unwrap_or(600_000));
        let config = if *jitter { &jitter_path } else { &config_path };
        let mut run = || command(bin, &data_path, config, timeout);
        let ran_by = build_name(bin);
        let t = Instant::now();
        let r = repo.step_cached_by(key, None, Some(&mut run), Some(&ran_by));
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let (got, out) = match &r {
            Ok(c) if c.hit => ("hit", c.output.to_string()[..12].to_string()),
            Ok(c) => ("miss", c.output.to_string()[..12].to_string()),
            Err(Error::Exec(m)) if m.starts_with("timed out") => ("timed out", "-".into()),
            Err(e) => panic!("{e}"),
        };
        if let Ok(c) = &r {
            outputs.push(c.output);
            if *jitter {
                jitter_outs.push(c.output);
            }
        }
        let good = got == case.expect;
        ok += good as usize;
        println!(
            "| {} | {} | {} | {} | {got} | {ms:.3} | `{out}` | {} |",
            i + 1,
            case.group,
            case.what,
            case.expect,
            if good { "yes" } else { "**no**" }
        );
    }
    println!();
    println!(
        "do not cache: the marked runs gave {} distinct outputs of 2; the unmarked hit returned the third run's output: {}",
        {
            let mut d = jitter_outs[..2].to_vec();
            d.dedup();
            d.len()
        },
        jitter_outs[3] == jitter_outs[2]
    );

    // What is stored with each result, side by side under the step's key.
    println!();
    println!("| filed as | props | output |");
    println!("| --- | --- | --- |");
    for key in [
        base("x86-64-plain"),
        base("x86-64-plain").with_timeout_ms(60_000),
        base("x86-64-plain").with_salt("fixed-1"),
    ] {
        for e in repo.step_cache_entries(&key)? {
            let props: Vec<String> = (e.run.props.iter())
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            let name = if e.platform.len() > 40 {
                format!("{}…", &e.platform[..40])
            } else {
                e.platform.clone()
            };
            println!(
                "| `{name}` | {} | `{}` |",
                props.join(", "),
                &e.run.output.to_string()[..12]
            );
        }
    }

    // The caller keeps the do-not-cache outputs, as any output it uses;
    // nothing else is committed.
    let entries = (jitter_outs.iter().enumerate())
        .map(|(i, id)| TreeEntry {
            name: format!("jitter{i}"),
            kind: Kind::File,
            id: *id,
        })
        .collect();
    let tree = repo.put(&Object::tree(entries)?)?;
    repo.commit("runs", tree, "the outputs used")?;

    let count = |repo: &Repo<FsStore>| -> Result<(usize, usize, usize), Error> {
        let refs = (repo.store().refs()?.iter())
            .filter(|(n, _)| n.starts_with(STEP_CACHE_REFS))
            .count();
        let mut runs = 0;
        for id in repo.store().ids()? {
            if matches!(repo.get(&id), Ok(Object::Run(_))) {
                runs += 1;
            }
        }
        Ok((refs, runs, repo.store().ids()?.len()))
    };
    let before = count(&repo)?;
    let removed = repo.gc()?;
    repo.repack()?;
    let after = count(&repo)?;
    let failures = repo.fsck()?;
    let readable = outputs.iter().all(|o| repo.read_content(o).is_ok());
    println!();
    println!(
        "cache refs {} -> {}, recorded steps {} -> {}, objects {} -> {} (gc removed {removed}), every output readable {readable}, fsck {} failures",
        before.0,
        after.0,
        before.1,
        after.1,
        before.2,
        after.2,
        failures.len()
    );
    println!(
        "{{\"cases\":{},\"as_expected\":{ok},\"refs_kept\":{},\"runs_kept\":{},\"fsck_clean\":{}}}",
        plan.len(),
        before.0 == after.0,
        before.1 == after.1,
        failures.is_empty()
    );
    Ok(())
}
