//! E327: execution modes for an IVF query's inner loop, scoring `n` int8
//! codes of `d` dimensions against one query (E323, E325).
//!
//! f32 query against int8 codes (what `vectors::code_dot` computes):
//! - `rust`: the kernel's loop as written, one running f32 sum. Float
//!   addition is not associative, so the compiler keeps the order and does
//!   not vectorize the sum.
//! - `rust-lanes`: 16 independent sums, added at the end (the reordering
//!   written out, so it vectorizes).
//! - `clang`: the kernel's loop in C, clang -O3 -march=native; `clang-fast`
//!   adds -ffast-math, which allows the reordering.
//! - `mlir`: the loop in MLIR's vector dialect (16 lanes, `vector.reduction`
//!   at the end), lowered by mlir-opt, then `mlir+llc` (llc -O3 alone) or
//!   `mlir+opt` (opt -O3 first).
//!
//! int8 query (each component times 127, rounded) against the codes, exact
//! i32 sums, so every mode must agree bit for bit:
//! - `rust-i8`, `avx2-i8` (std::arch, widen to i16 and `madd`), `clang-i8`,
//!   `mlir-i8+llc`, `mlir-i8+opt`.
//!
//! Prints one JSON line per mode and size: ns a code (fastest of 5 runs of
//! at least 300 ms each) and, for the int8 query, how many of the f32
//! query's top 100 it keeps.
//!
//! `cargo run --release -- [d]`, with `MLIR_BIN` naming the folder of
//! mlir-opt, mlir-translate, opt and llc (else those modes are skipped).

#![allow(clippy::type_complexity)]

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn unit(&mut self, d: usize) -> Vec<f32> {
        let v: Vec<f32> = (0..d)
            .map(|_| {
                (0..4)
                    .map(|_| (self.next() % 10_001) as f32 / 1e4)
                    .sum::<f32>()
                    - 2.0
            })
            .collect();
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }
}

fn quantize(v: &[f32]) -> Vec<i8> {
    v.iter()
        .map(|x| (x * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

fn score_rust(q: &[f32], codes: &[i8], d: usize, out: &mut [f32]) {
    for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
        *o = q.iter().zip(c).map(|(x, &c)| x * f32::from(c)).sum();
    }
}

fn score_lanes(q: &[f32], codes: &[i8], d: usize, out: &mut [f32]) {
    for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
        let mut acc = [0f32; 16];
        for (qs, cs) in q.chunks_exact(16).zip(c.chunks_exact(16)) {
            for l in 0..16 {
                acc[l] += qs[l] * f32::from(cs[l]);
            }
        }
        *o = acc.iter().sum();
    }
}

fn score_i8(q: &[i8], codes: &[i8], d: usize, out: &mut [i32]) {
    for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
        *o = q
            .iter()
            .zip(c)
            .map(|(&x, &c)| i32::from(x) * i32::from(c))
            .sum();
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn score_avx2(q: &[i8], codes: &[i8], d: usize, out: &mut [i32]) {
    use std::arch::x86_64::*;
    // SAFETY: loads read 16 bytes at j < d, with d a multiple of 16.
    unsafe {
        for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
            let mut acc = _mm256_setzero_si256();
            for j in (0..d).step_by(16) {
                let qv = _mm256_cvtepi8_epi16(_mm_loadu_si128(q.as_ptr().add(j).cast()));
                let cv = _mm256_cvtepi8_epi16(_mm_loadu_si128(c.as_ptr().add(j).cast()));
                acc = _mm256_add_epi32(acc, _mm256_madd_epi16(qv, cv));
            }
            let mut lanes = [0i32; 8];
            _mm256_storeu_si256(lanes.as_mut_ptr().cast(), acc);
            *o = lanes.iter().sum();
        }
    }
}

const C_SRC: &str = r#"
#include <stdint.h>
void score_f32(const float *q, const int8_t *c, float *out, long n, long d) {
  for (long i = 0; i < n; i++) {
    float s = 0.0f;
    for (long j = 0; j < d; j++) s += q[j] * (float)c[i * d + j];
    out[i] = s;
  }
}
void score_i8(const int8_t *q, const int8_t *c, int32_t *out, long n, long d) {
  for (long i = 0; i < n; i++) {
    int32_t s = 0;
    for (long j = 0; j < d; j++) s += (int32_t)q[j] * (int32_t)c[i * d + j];
    out[i] = s;
  }
}
"#;

fn mlir_src() -> String {
    let body = |name: &str,
                qt: &str,
                acc: &str,
                widen: &str,
                qref: &str,
                mul: &str,
                add: &str,
                red: &str| {
        format!(
            r#"  func.func @{name}(%q: memref<?x{qt}>, %c: memref<?xi8>, %out: memref<?x{red}>, %n: index, %d: index) attributes {{llvm.emit_c_interface}} {{
    %c0 = arith.constant 0 : index
    %c1 = arith.constant 1 : index
    %c16 = arith.constant 16 : index
    %z = arith.constant dense<0{zero}> : vector<16x{acc}>
    scf.for %i = %c0 to %n step %c1 {{
      %base = arith.muli %i, %d : index
      %sum = scf.for %j = %c0 to %d step %c16 iter_args(%a = %z) -> (vector<16x{acc}>) {{
        %qv = vector.load %q[%j] : memref<?x{qt}>, vector<16x{qt}>
        %k = arith.addi %base, %j : index
        %cv = vector.load %c[%k] : memref<?xi8>, vector<16xi8>
{widen}
        %p = {mul} {qref}, %cw : vector<16x{acc}>
        %s = {add} %a, %p : vector<16x{acc}>
        scf.yield %s : vector<16x{acc}>
      }}
      %r = vector.reduction <add>, %sum : vector<16x{acc}> into {red}
      memref.store %r, %out[%i] : memref<?x{red}>
    }}
    return
  }}
"#,
            zero = if acc == "f32" { ".0" } else { "" }
        )
    };
    let f32w = "        %cw = arith.sitofp %cv : vector<16xi8> to vector<16xf32>";
    let i8w = "        %qw = arith.extsi %qv : vector<16xi8> to vector<16xi32>\n        %cw = arith.extsi %cv : vector<16xi8> to vector<16xi32>";
    format!(
        "module {{\n{}{}}}\n",
        body(
            "score_f32",
            "f32",
            "f32",
            f32w,
            "%qv",
            "arith.mulf",
            "arith.addf",
            "f32"
        ),
        body(
            "score_i8",
            "i8",
            "i32",
            i8w,
            "%qw",
            "arith.muli",
            "arith.addi",
            "i32"
        )
    )
}

const MLIR_PASSES: &[&str] = &[
    "--canonicalize",
    "--cse",
    "--convert-scf-to-cf",
    "--convert-vector-to-llvm",
    "--expand-strided-metadata",
    "--finalize-memref-to-llvm",
    "--convert-arith-to-llvm",
    "--convert-index-to-llvm",
    "--convert-func-to-llvm",
    "--convert-cf-to-llvm",
    "--reconcile-unrealized-casts",
];

/// A one-dimensional memref as MLIR's C interface passes it.
#[repr(C)]
struct Desc<T> {
    allocated: *const T,
    aligned: *const T,
    offset: i64,
    size: i64,
    stride: i64,
}

fn desc<T>(s: &[T]) -> Desc<T> {
    Desc {
        allocated: s.as_ptr(),
        aligned: s.as_ptr(),
        offset: 0,
        size: s.len() as i64,
        stride: 1,
    }
}

type CF32 = unsafe extern "C" fn(*const f32, *const i8, *mut f32, i64, i64);
type CI8 = unsafe extern "C" fn(*const i8, *const i8, *mut i32, i64, i64);
type MF32 = unsafe extern "C" fn(*const Desc<f32>, *const Desc<i8>, *const Desc<f32>, i64, i64);
type MI8 = unsafe extern "C" fn(*const Desc<i8>, *const Desc<i8>, *const Desc<i32>, i64, i64);

fn sym<T: Copy>(lib: &Path, name: &str) -> T {
    let path = CString::new(lib.to_str().unwrap()).unwrap();
    let name = CString::new(name).unwrap();
    unsafe {
        let h = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        assert!(!h.is_null(), "dlopen {}", lib.display());
        let f = libc::dlsym(h, name.as_ptr());
        assert!(!f.is_null(), "dlsym {}", name.to_str().unwrap());
        std::mem::transmute_copy(&f)
    }
}

fn run(cmd: &mut Command) -> f64 {
    let t = Instant::now();
    let out = cmd.output().expect("tool");
    assert!(
        out.status.success(),
        "{cmd:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    t.elapsed().as_secs_f64() * 1e3
}

/// Shared libraries: clang, clang-fast, and the MLIR pair (if MLIR_BIN).
fn build(
    dir: &Path,
) -> (
    PathBuf,
    PathBuf,
    Option<(PathBuf, PathBuf, Vec<(String, f64)>)>,
) {
    let c = dir.join("k.c");
    std::fs::write(&c, C_SRC).unwrap();
    let lib = |name: &str, fast: bool| {
        let so = dir.join(name);
        let mut cmd = Command::new("clang");
        cmd.args(["-O3", "-march=native", "-shared", "-fPIC", "-o"])
            .arg(&so)
            .arg(&c);
        if fast {
            cmd.arg("-ffast-math");
        }
        let ms = run(&mut cmd);
        eprintln!("{name}: clang {ms:.0} ms");
        so
    };
    let (plain, fast) = (lib("clang.so", false), lib("clang_fast.so", true));
    let Ok(bin) = std::env::var("MLIR_BIN") else {
        return (plain, fast, None);
    };
    let tool = |t: &str| PathBuf::from(&bin).join(t);
    let src = dir.join("k.mlir");
    std::fs::write(&src, mlir_src()).unwrap();
    let (low, ll, opt_ll) = (dir.join("low.mlir"), dir.join("k.ll"), dir.join("k.opt.ll"));
    let mut stages = Vec::new();
    stages.push((
        "mlir-opt".to_string(),
        run(Command::new(tool("mlir-opt"))
            .args(MLIR_PASSES)
            .arg(&src)
            .arg("-o")
            .arg(&low)),
    ));
    stages.push((
        "mlir-translate".to_string(),
        run(Command::new(tool("mlir-translate"))
            .arg("--mlir-to-llvmir")
            .arg(&low)
            .arg("-o")
            .arg(&ll)),
    ));
    stages.push((
        "opt -O3".to_string(),
        run(Command::new(tool("opt"))
            .args(["-O3", "-S"])
            .arg(&ll)
            .arg("-o")
            .arg(&opt_ll)),
    ));
    let mut so = |input: &Path, name: &str| {
        let obj = dir.join(format!("{name}.o"));
        let ms = run(Command::new(tool("llc"))
            .args([
                "-O3",
                "-mcpu=native",
                "-filetype=obj",
                "-relocation-model=pic",
            ])
            .arg(input)
            .arg("-o")
            .arg(&obj));
        stages.push((format!("llc {name}"), ms));
        let so = dir.join(format!("{name}.so"));
        run(Command::new("cc")
            .arg("-shared")
            .arg("-o")
            .arg(&so)
            .arg(&obj));
        so
    };
    let a = so(&ll, "mlir_llc");
    let b = so(&opt_ll, "mlir_opt");
    (plain, fast, Some((a, b, stages)))
}

/// Fastest of 5 runs, each repeating `f` for at least 300 ms; ns a call.
fn time(mut f: impl FnMut()) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..5 {
        let (t, mut reps) = (Instant::now(), 0u64);
        while t.elapsed().as_millis() < 300 || reps == 0 {
            f();
            reps += 1;
        }
        best = best.min(t.elapsed().as_secs_f64() * 1e9 / reps as f64);
    }
    best
}

fn top(scores: &[f32], k: usize) -> Vec<usize> {
    let mut i: Vec<usize> = (0..scores.len()).collect();
    i.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));
    i.truncate(k);
    i
}

fn main() {
    let d: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(256);
    assert_eq!(d % 16, 0);
    let dir = tempfile::tempdir().unwrap();
    let (clang, clang_fast, mlir) = build(dir.path());
    if let Some((_, _, stages)) = &mlir {
        for (s, ms) in stages {
            println!("{{\"compile\":\"{s}\",\"ms\":{ms:.1}}}");
        }
    }
    let mut rng = Lcg(327);
    for n in [4096usize, 100_000] {
        let codes: Vec<i8> = (0..n).flat_map(|_| quantize(&rng.unit(d))).collect();
        let q = rng.unit(d);
        let qi = quantize(&q);
        let mut want = vec![0f32; n];
        score_rust(&q, &codes, d, &mut want);
        let mut f32_modes: Vec<(String, Box<dyn Fn(&mut [f32])>)> = vec![
            (
                "rust".into(),
                Box::new(|o: &mut [f32]| score_rust(&q, &codes, d, o)),
            ),
            (
                "rust-lanes".into(),
                Box::new(|o: &mut [f32]| score_lanes(&q, &codes, d, o)),
            ),
        ];
        let mut i8_modes: Vec<(String, Box<dyn Fn(&mut [i32])>)> = vec![(
            "rust-i8".into(),
            Box::new(|o: &mut [i32]| score_i8(&qi, &codes, d, o)),
        )];
        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx2") {
            i8_modes.push((
                "avx2-i8".into(),
                Box::new(|o: &mut [i32]| unsafe { score_avx2(&qi, &codes, d, o) }),
            ));
        }
        let (qp, cp, qip) = (q.as_ptr(), codes.as_ptr(), qi.as_ptr());
        for (name, lib) in [("clang", &clang), ("clang-fast", &clang_fast)] {
            let f: CF32 = sym(lib, "score_f32");
            f32_modes.push((
                name.into(),
                Box::new(move |o: &mut [f32]| unsafe {
                    f(qp, cp, o.as_mut_ptr(), n as i64, d as i64)
                }),
            ));
        }
        let g: CI8 = sym(&clang, "score_i8");
        i8_modes.push((
            "clang-i8".into(),
            Box::new(move |o: &mut [i32]| unsafe {
                g(qip, cp, o.as_mut_ptr(), n as i64, d as i64)
            }),
        ));
        if let Some((llc, opt, _)) = &mlir {
            for (tag, lib) in [("llc", llc), ("opt", opt)] {
                let f: MF32 = sym(lib, "_mlir_ciface_score_f32");
                let (q, codes) = (&q, &codes);
                f32_modes.push((
                    format!("mlir+{tag}"),
                    Box::new(move |o: &mut [f32]| unsafe {
                        f(&desc(q), &desc(codes), &desc(o), n as i64, d as i64)
                    }),
                ));
                let g: MI8 = sym(lib, "_mlir_ciface_score_i8");
                let qi = &qi;
                i8_modes.push((
                    format!("mlir-i8+{tag}"),
                    Box::new(move |o: &mut [i32]| unsafe {
                        g(&desc(qi), &desc(codes), &desc(o), n as i64, d as i64)
                    }),
                ));
            }
        }
        let want_top = top(&want, 100);
        for (name, f) in &f32_modes {
            let mut out = vec![0f32; n];
            f(&mut out);
            let diff = out
                .iter()
                .zip(&want)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            let same_top = top(&out, 100) == want_top;
            let ns = time(|| f(&mut out)) / n as f64;
            println!(
                "{{\"n\":{n},\"d\":{d},\"query\":\"f32\",\"mode\":\"{name}\",\"ns_per_code\":{ns:.2},\"gb_s\":{:.2},\"max_abs_diff\":{diff:.2e},\"same_top100\":{same_top}}}",
                d as f64 / ns
            );
        }
        let mut ref_i8 = vec![0i32; n];
        score_i8(&qi, &codes, d, &mut ref_i8);
        let as_f: Vec<f32> = ref_i8.iter().map(|&x| x as f32).collect();
        let kept = top(&as_f, 100)
            .iter()
            .filter(|i| want_top.contains(i))
            .count();
        for (name, f) in &i8_modes {
            let mut out = vec![0i32; n];
            f(&mut out);
            assert_eq!(out, ref_i8, "{name} disagrees with rust-i8");
            let ns = time(|| f(&mut out)) / n as f64;
            println!(
                "{{\"n\":{n},\"d\":{d},\"query\":\"i8\",\"mode\":\"{name}\",\"ns_per_code\":{ns:.2},\"gb_s\":{:.2},\"exact\":true,\"f32_top100_kept\":{kept}}}",
                d as f64 / ns
            );
        }
    }
}
