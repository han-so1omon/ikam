//! E428: a platform-keyed step cache for recorded and float steps.
//!
//! A training-like step (seeded least squares, full-batch gradient descent,
//! `experiments/step-cache`) runs as an outside command through
//! `step_cached`: twice on one platform tag (the second must be a hit),
//! then under two more tags, each a separate build of the same source:
//! `target-cpu=x86-64-v3` alone (Rust never fuses on its own, so the same
//! bits are expected) and `target-cpu=x86-64-v3 --cfg fused` (fused
//! multiply-add and four-lane sums, as a vectorising FMA build would). Each
//! platform's output sits beside the others under one key; a `within-f64`
//! check step against the first is applied as a derivation, so the
//! divergence is in the store. A float step (an E441 WASM function the
//! kernel runs) is cached the same way. Then gc, repack and fsck.
//!
//!   cargo run --release --example e428_step_cache -- WORK PLAIN V3 FUSED

#[path = "../tests/common/float_wat.rs"]
mod float_wat;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use ikam_kernel::stepcache::{STEP_CACHE_REFS, StepKey, StepKind};
use ikam_kernel::{Arg, Error, FsStore, Repo, Store, func};

const N: usize = 4096;
const D: usize = 64;
const CONFIG: &str = "iters=400 lr=0.05";

/// Seeded rows: features uniform in [-1, 1), a target from fixed weights
/// plus small noise, all from one 64-bit LCG (the same bits anywhere).
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

fn f64s(b: &[u8]) -> Vec<f64> {
    b.chunks_exact(8)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn ulps(a: f64, b: f64) -> u64 {
    let key = |x: f64| {
        let b = x.to_bits() as i64;
        if b < 0 { i64::MIN - b } else { b }
    };
    key(a).abs_diff(key(b))
}

/// Run the training command on files holding the step's inputs.
fn command(bin: &Path, inputs: &Path) -> Result<Vec<u8>, Error> {
    let out = Command::new(bin)
        .arg(inputs.join("data"))
        .arg(inputs.join("config"))
        .output()?;
    if !out.status.success() {
        return Err(Error::Exec(format!("train-gd: {}", out.status)));
    }
    Ok(out.stdout)
}

fn main() -> Result<(), Error> {
    let a: Vec<String> = std::env::args().collect();
    let [_, work, plain, v3, fused] = &a[..] else {
        panic!("usage: e428_step_cache WORK PLAIN V3 FUSED");
    };
    let work = PathBuf::from(work);
    let store = work.join("store");
    let _ = std::fs::remove_dir_all(&store);
    let inputs = work.join("inputs");
    std::fs::create_dir_all(&inputs)?;
    let mut repo = Repo::new(FsStore::open(&store)?);

    // The step: its source is the function's id (each build is a
    // platform), the seeded data and the settings are its inputs.
    let source = include_bytes!("../experiments/step-cache/src/main.rs");
    let func = repo.put_content(source)?.id;
    let data = data();
    let data_id = repo.put_content(&data)?.id;
    let config_id = repo.put_content(CONFIG.as_bytes())?.id;
    std::fs::write(inputs.join("data"), &data)?;
    std::fs::write(inputs.join("config"), CONFIG)?;
    let args = vec![Arg::Whole(data_id), Arg::Whole(config_id)];
    let key = |platform: &str| StepKey::new(func, args.clone(), StepKind::Recorded, platform);

    println!("| # | platform tag | build | hit | ms | output |");
    println!("| --- | --- | --- | --- | --- | --- |");
    let plan = [
        ("x86-64-plain", plain, "target-cpu=x86-64"),
        ("x86-64-plain", plain, "target-cpu=x86-64"),
        ("x86-64-plain", plain, "target-cpu=x86-64"),
        ("x86-64-v3", v3, "target-cpu=x86-64-v3"),
        ("x86-64-v3-fma", fused, "target-cpu=x86-64-v3, cfg fused"),
        ("x86-64-v3-fma", fused, "target-cpu=x86-64-v3, cfg fused"),
    ];
    let (mut miss_ms, mut hit_ms) = (Vec::new(), Vec::new());
    for (i, (tag, bin, build)) in plan.iter().enumerate() {
        let bin = PathBuf::from(bin);
        let mut work = || command(&bin, &inputs);
        let t = Instant::now();
        let c = repo.step_cached(&key(tag), None, Some(&mut work))?;
        let ms = t.elapsed().as_secs_f64() * 1e3;
        if c.hit { &mut hit_ms } else { &mut miss_ms }.push(ms);
        let out = c.output.to_string();
        println!(
            "| {} | `{tag}` | {build} | {} | {ms:.3} | `{}` |",
            i + 1,
            if c.hit { "hit" } else { "miss (ran)" },
            &out[..12]
        );
    }

    // Side by side: every platform's output of the one step, each judged
    // against the first by a check step stored as a derivation.
    let entries = repo.step_cache_entries(&key("x86-64-plain"))?;
    let first = entries
        .iter()
        .find(|e| e.platform == "x86-64-plain")
        .unwrap();
    let base = f64s(&repo.read_content(&first.run.output)?);
    let spec = repo.put_content(b"ulp=4")?.id;
    println!();
    println!(
        "| platform | output | same bits as plain | max ULP (weights) | final mse | within-f64 ulp=4 |"
    );
    println!("| --- | --- | --- | --- | --- | --- |");
    for e in &entries {
        let v = f64s(&repo.read_content(&e.run.output)?);
        let max = (base.iter().zip(&v))
            .map(|(x, y)| ulps(*x, *y))
            .take(D)
            .max()
            .unwrap();
        let verdict = repo.apply(
            func::within_f64(),
            vec![
                Arg::Whole(first.run.output),
                Arg::Whole(e.run.output),
                Arg::Whole(spec),
            ],
        )?;
        let text = String::from_utf8(repo.read_content(&verdict)?).unwrap();
        let out = e.run.output.to_string();
        println!(
            "| `{}` | `{}` | {} | {max} | {:.6e} | {} |",
            e.platform,
            &out[..12],
            if e.run.output == first.run.output {
                "yes"
            } else {
                "no"
            },
            v[D],
            text.trim()
        );
    }

    // A float step: an E441 WASM function the kernel runs, pinned.
    let module = repo
        .put_content(&wat::parse_str(float_wat::wat("matmul")).unwrap())?
        .id;
    let (x, y) = float_wat::inputs("plain", 64);
    let x = repo.put_content(&float_wat::bytes(&x))?.id;
    let y = repo.put_content(&float_wat::bytes(&y))?.id;
    let fkey = StepKey::new(
        module,
        vec![Arg::Whole(x), Arg::Whole(y)],
        StepKind::Float,
        "x86-64-plain",
    );
    println!();
    println!("| float step (matmul 8x8, kernel-run) | hit | µs |");
    println!("| --- | --- | --- |");
    for i in 1..=2 {
        let t = Instant::now();
        let c = repo.step_cached(&fkey, None, None)?;
        println!(
            "| run {i} | {} | {:.1} |",
            c.hit,
            t.elapsed().as_secs_f64() * 1e6
        );
    }

    // gc, repack, fsck: the cache's refs keep every output.
    let before = repo.store().ids()?.len();
    let removed = repo.gc()?;
    let t = Instant::now();
    repo.repack()?;
    let repack_ms = t.elapsed().as_secs_f64() * 1e3;
    let failures = repo.fsck()?;
    let deep = repo.fsck_deep()?;
    let after = repo.store().ids()?.len();
    let refs = (repo.store().refs()?.iter())
        .filter(|(n, _)| n.starts_with(STEP_CACHE_REFS))
        .count();
    let readable = entries
        .iter()
        .all(|e| repo.read_content(&e.run.output).is_ok());
    let mut never = || Err(Error::Exec("a hit must not run".into()));
    let still = repo.step_cached(&key("x86-64-v3-fma"), None, Some(&mut never))?;
    println!();
    println!(
        "objects {before} -> {after} (gc removed {removed}), repack {repack_ms:.0} ms, cache refs {refs}, \
         outputs readable {readable}, fsck {} failures, fsck_deep {} failures, hit after repack {}",
        failures.len(),
        deep.len(),
        still.hit
    );
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    println!(
        "{{\"miss_ms_mean\":{:.3},\"hit_ms_mean\":{:.3},\"saved_ms_per_hit\":{:.3},\"misses\":{},\"hits\":{},\"platforms\":{},\"fsck_clean\":{}}}",
        mean(&miss_ms),
        mean(&hit_ms),
        mean(&miss_ms) - mean(&hit_ms),
        miss_ms.len(),
        hit_ms.len(),
        entries.len(),
        failures.is_empty() && deep.is_empty()
    );
    Ok(())
}
