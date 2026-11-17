//! E341: compiled modes for the kernel's data-parallel loops, where
//! compiling could pay more than in a net's step (E311), and what compiled
//! code is worth once built (load time from a cached library).
//!
//! Two kernels on random unit vectors of 256 dimensions (the WordLlama
//! vectors' size), one thread:
//!
//! - `scan`: one query scored against `n` rows (an exact scan, and the
//!   inner loop of the IVF and graph rescoring), 10,000 rows (in cache)
//!   and 200,000 (205 MB, from memory).
//! - `assign`: one k-means round's assignment, each of `n` rows to the
//!   centroid it scores highest against (`vectors::kmeans`), 31,600 rows
//!   against 316 centroids (E331's longer training on 100,000 paragraphs).
//!   A matrix product, the shape where loop order and tiling matter.
//!
//! Modes:
//! - `kernel`: the kernel's code as shipped. scan: `vectors::dot_lanes`
//!   per row. assign: `kmeans`'s nearest-centroid rule as written
//!   (`max_by` over `vectors::dot`, which scores each centroid twice).
//! - `kernel-once` (assign): each centroid scored once with `dot_lanes`.
//! - `cranelift`: 16 lanes as four f32x4 sums, the dimension fixed.
//! - `clang`: the lanes loop in C, -O3, -ffp-contract=off (no fused
//!   multiply-add, so the same bits); `clang-fast`: -ffast-math.
//! - `mlir`: the lanes loop in MLIR's vector dialect (see `mlir.rs`).
//! - `clang-ikj`, `mlir-linalg` (assign): all scores as a product with the
//!   centroid innermost (row, dimension, centroid) over the transposed
//!   centroids, each score summed in order, as `vectors::dot`; then the
//!   largest per row.
//! - `accelerate` (macOS): Apple's BLAS (`cblas_sgemv`, `cblas_sgemm`),
//!   whatever cores and units it chooses.
//!
//! Each mode is checked against the kernel: the same bits (scan), the same
//! centroids (assign), or, for fast and BLAS modes, the largest difference
//! and the share of rows assigned alike. One JSON line per kernel and mode:
//! ns a row (fastest of 5 runs of at least 300 ms), GFLOP/s, compile ms
//! and the ms to load the compiled library again from disk.
//!
//!   MLIR_BIN=<folder of mlir-opt, mlir-translate, opt, llc> cargo run --release
//!
//! `-- smoke` runs small sizes once. Without MLIR_BIN the MLIR modes are
//! skipped.

#![allow(clippy::type_complexity)]

mod cl;
mod mlir;

use std::ffi::CString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use ikam_kernel::vectors;

pub const D: usize = 256;

pub type ScanFn = unsafe extern "C" fn(*const f32, *const f32, u64, *mut f32);
pub type AssignFn = unsafe extern "C" fn(*const f32, u64, *const f32, u64, *mut u32);
pub type ScoresFn = unsafe extern "C" fn(*const f32, u64, *const f32, u64, *mut f32);

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn units(&mut self, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n * D);
        for _ in 0..n {
            let v: Vec<f32> = (0..D)
                .map(|_| {
                    (0..4)
                        .map(|_| (self.next() % 10_001) as f32 / 1e4)
                        .sum::<f32>()
                        - 2.0
                })
                .collect();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            out.extend(v.into_iter().map(|x| x / norm));
        }
        out
    }
}

fn c_src() -> String {
    format!(
        r#"typedef unsigned long long u64;
#define D {D}
void scan(const float *q, const float *r, u64 n, float *out) {{
  for (u64 i = 0; i < n; i++) {{
    const float *x = r + i * D;
    float a[16] = {{0}};
    for (int j = 0; j < D; j += 16)
      for (int l = 0; l < 16; l++) a[l] += q[j + l] * x[j + l];
    float s = a[0];
    for (int l = 1; l < 16; l++) s += a[l];
    out[i] = s;
  }}
}}
void assign(const float *r, u64 n, const float *c, u64 k, unsigned *out) {{
  for (u64 i = 0; i < n; i++) {{
    const float *x = r + i * D;
    float best = -__builtin_inff();
    unsigned bi = 0;
    for (u64 j = 0; j < k; j++) {{
      const float *y = c + j * D;
      float a[16] = {{0}};
      for (int m = 0; m < D; m += 16)
        for (int l = 0; l < 16; l++) a[l] += y[m + l] * x[m + l];
      float s = a[0];
      for (int l = 1; l < 16; l++) s += a[l];
      if (s >= best) {{ best = s; bi = (unsigned)j; }}
    }}
    out[i] = bi;
  }}
}}
void scores_ikj(const float *r, u64 n, const float *ct, u64 k, float *o) {{
  for (u64 i = 0; i < n; i++) {{
    float *oi = o + i * k;
    for (u64 j = 0; j < k; j++) oi[j] = 0.0f;
    for (int l = 0; l < D; l++) {{
      float a = r[i * D + l];
      const float *cl = ct + (u64)l * k;
      for (u64 j = 0; j < k; j++) oi[j] += a * cl[j];
    }}
  }}
}}
"#
    )
}

const ACC_SRC: &str = r#"#define ACCELERATE_NEW_LAPACK
#include <Accelerate/Accelerate.h>
typedef unsigned long long u64;
void scan(const float *q, const float *r, u64 n, float *out) {
  cblas_sgemv(CblasRowMajor, CblasNoTrans, (int)n, 256, 1.0f, r, 256, q, 1, 0.0f, out, 1);
}
void scores(const float *r, u64 n, const float *c, u64 k, float *o) {
  cblas_sgemm(CblasRowMajor, CblasNoTrans, CblasTrans, (int)n, (int)k, 256, 1.0f, r, 256, c, 256, 0.0f, o, (int)k);
}
"#;

fn cpu_flag() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "-march=native"
    } else {
        "-mcpu=native"
    }
}

/// clang builds `src` into a library; the host CPU flag is dropped if this
/// clang refuses it. (library, ms, flags used).
fn clang(dir: &Path, name: &str, src: &str, extra: &[&str]) -> (PathBuf, f64, String) {
    let c = dir.join(format!("{name}.c"));
    std::fs::write(&c, src).unwrap();
    let so = dir.join(format!("{name}.so"));
    for cpu in [Some(cpu_flag()), None] {
        let mut args: Vec<&str> = vec!["-O3", "-shared", "-fPIC"];
        args.extend(cpu);
        args.extend(extra);
        let t = Instant::now();
        let out = Command::new("clang")
            .args(&args)
            .arg("-o")
            .arg(&so)
            .arg(&c)
            .output()
            .expect("clang");
        let ms = t.elapsed().as_secs_f64() * 1e3;
        if out.status.success() {
            return (so, ms, args[3..].join(" "));
        }
        if cpu.is_none() {
            panic!("clang {name}: {}", String::from_utf8_lossy(&out.stderr));
        }
    }
    unreachable!()
}

struct Lib(*mut libc::c_void);

fn open(so: &Path) -> Lib {
    let p = CString::new(so.to_str().unwrap()).unwrap();
    // SAFETY: a library this program just built.
    let h = unsafe { libc::dlopen(p.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    assert!(!h.is_null(), "dlopen {}", so.display());
    Lib(h)
}

fn sym<F: Copy>(lib: &Lib, name: &str) -> F {
    let n = CString::new(name).unwrap();
    // SAFETY: the symbol was compiled with the signature F names.
    unsafe {
        let f = libc::dlsym(lib.0, n.as_ptr());
        assert!(!f.is_null(), "dlsym {name}");
        std::mem::transmute_copy::<*mut libc::c_void, F>(&f)
    }
}

/// The ms to load a compiled library again, as from a cache on disk: a copy
/// under a new name, opened and one symbol found.
fn reload_ms(so: &Path, name: &str) -> f64 {
    // A new file each time: rewriting a library that is mapped would change
    // the code under it.
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let i = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let copy = so.with_file_name(format!(
        "cached{i}-{}",
        so.file_name().unwrap().to_str().unwrap()
    ));
    std::fs::copy(so, &copy).unwrap();
    let t = Instant::now();
    let lib = open(&copy);
    let _: *const u8 = sym(&lib, name);
    t.elapsed().as_secs_f64() * 1e3
}

/// Fastest of `runs` runs, each repeating `f` for at least `min_ms`; ns a call.
fn time(runs: usize, min_ms: u128, mut f: impl FnMut()) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..runs {
        let (t, mut reps) = (Instant::now(), 0u64);
        while t.elapsed().as_millis() < min_ms || reps == 0 {
            f();
            reps += 1;
        }
        best = best.min(t.elapsed().as_secs_f64() * 1e9 / reps as f64);
    }
    best
}

fn version(cmd: &Path) -> String {
    Command::new(cmd)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .find(|l| l.contains("version"))
                .map(|l| l.trim().replace('"', "'"))
        })
        .unwrap_or_else(|| "none".into())
}

struct Line<'a> {
    kernel: &'a str,
    rows: usize,
    k: usize,
    mode: &'a str,
    ns_call: f64,
    compile_ms: Option<f64>,
    load_ms: Option<f64>,
    check: String,
}

fn print(l: Line) {
    let per_row = l.ns_call / l.rows as f64;
    let flops = 2.0 * D as f64 * l.k.max(1) as f64 * l.rows as f64;
    let mut s = format!(
        "{{\"exp\":\"E341\",\"kernel\":\"{}\",\"rows\":{},\"k\":{},\"mode\":\"{}\",\"ns_per_row\":{:.2},\"gflops\":{:.2}",
        l.kernel,
        l.rows,
        l.k,
        l.mode,
        per_row,
        flops / l.ns_call
    );
    if let Some(c) = l.compile_ms {
        write!(s, ",\"compile_ms\":{c:.2}").unwrap();
    }
    if let Some(c) = l.load_ms {
        write!(s, ",\"load_ms\":{c:.3}").unwrap();
    }
    println!("{s},{}}}", l.check);
}

fn check_scan(reference: &[f32], got: &[f32]) -> String {
    let same = reference
        .iter()
        .zip(got)
        .all(|(a, b)| a.to_bits() == b.to_bits());
    let err = reference
        .iter()
        .zip(got)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    format!("\"same_bits\":{same},\"max_abs_diff\":{err:.3e}")
}

fn check_assign(reference: &[u32], got: &[u32], strict: bool) -> String {
    let agree = reference.iter().zip(got).filter(|(a, b)| a == b).count();
    let share = agree as f64 / reference.len() as f64;
    if strict {
        assert_eq!(agree, reference.len(), "assignment differs");
    }
    format!("\"same_centroids\":{share:.4}")
}

fn argmax_rows(s: &[f32], k: usize, out: &mut [u32]) {
    for (row, o) in s.chunks_exact(k).zip(out.iter_mut()) {
        let (mut best, mut bi) = (f32::NEG_INFINITY, 0u32);
        for (j, &v) in row.iter().enumerate() {
            if v >= best {
                best = v;
                bi = j as u32;
            }
        }
        *o = bi;
    }
}

fn main() {
    let smoke = std::env::args().any(|a| a == "smoke");
    let (runs, min_ms) = if smoke { (1, 1) } else { (5, 300) };
    let scan_sizes: &[usize] = if smoke { &[2_000] } else { &[10_000, 200_000] };
    let (an, ak) = if smoke { (500, 64) } else { (31_600, 316) };
    let bin = std::env::var("MLIR_BIN").ok();
    let dir = tempfile::tempdir().unwrap();

    // Build every compiled mode first, each timed.
    let (clang_so, clang_ms, flags) = clang(dir.path(), "plain", &c_src(), &["-ffp-contract=off"]);
    let (fast_so, fast_ms, _) = clang(dir.path(), "fast", &c_src(), &["-ffast-math"]);
    let acc = cfg!(target_os = "macos")
        .then(|| clang(dir.path(), "acc", ACC_SRC, &["-framework", "Accelerate"]));
    let mlir = bin
        .as_deref()
        .map(|b| mlir::build(b, dir.path(), Some("native")));
    let t = Instant::now();
    let (_m1, cl_scan) = cl::scan();
    let cl_scan_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let (_m2, cl_assign) = cl::assign();
    let cl_assign_ms = t.elapsed().as_secs_f64() * 1e3;

    let mut tools = format!(
        "{{\"exp\":\"E341\",\"arch\":\"{}\",\"os\":\"{}\",\"cpus\":{},\"clang\":\"{}\",\"clang_flags\":\"{}\",\"mlir_opt\":\"{}\",\"cranelift_ms\":{{\"scan\":{:.2},\"assign\":{:.2}}}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::thread::available_parallelism().map_or(0, |n| n.get()),
        version(Path::new("clang")),
        flags,
        bin.as_deref()
            .map_or("none".into(), |b| version(&mlir::tool(b, "mlir-opt"))),
        cl_scan_ms,
        cl_assign_ms
    );
    if let Some((_, st)) = &mlir {
        let parts: Vec<String> = st
            .iter()
            .map(|(n, ms)| format!("\"{n}\":{ms:.2}"))
            .collect();
        write!(tools, ",\"mlir_stages_ms\":{{{}}}", parts.join(",")).unwrap();
    } else {
        tools.push_str(",\"note\":\"MLIR_BIN not set: MLIR modes skipped\"");
    }
    println!("{tools}}}");

    let clang_lib = open(&clang_so);
    let fast_lib = open(&fast_so);
    let acc_lib = acc.as_ref().map(|(so, _, _)| open(so));
    let mlir_lib = mlir.as_ref().map(|(so, _)| open(so));
    let mlir_ms = mlir
        .as_ref()
        .map(|(_, st)| st.iter().map(|(_, ms)| ms).sum::<f64>());
    let load = |so: &Path, s: &str| Some(reload_ms(so, s));

    // scan
    let mut rng = Lcg(0x5EED_0341);
    let max = *scan_sizes.iter().max().unwrap();
    let rows = rng.units(max);
    let q = rng.units(1);
    for &n in scan_sizes {
        let r = &rows[..n * D];
        let mut reference = vec![0f32; n];
        let ns = time(runs, min_ms, || {
            for (i, o) in reference.iter_mut().enumerate() {
                *o = vectors::dot_lanes(&q, &r[i * D..][..D]);
            }
        });
        let line =
            |mode: &str, ns: f64, compile_ms: Option<f64>, load_ms: Option<f64>, check: String| {
                print(Line {
                    kernel: "scan",
                    rows: n,
                    k: 1,
                    mode,
                    ns_call: ns,
                    compile_ms,
                    load_ms,
                    check,
                })
            };
        line("kernel", ns, None, None, "\"same_bits\":true".into());
        let mut out = vec![0f32; n];
        let mut go =
            |mode: &str, f: &mut dyn FnMut(&mut [f32]), compile_ms, load_ms, strict: bool| {
                out.iter_mut().for_each(|o| *o = f32::NAN);
                let ns = time(runs, min_ms, || f(&mut out));
                let check = check_scan(&reference, &out);
                if strict {
                    assert!(check.contains("\"same_bits\":true"), "{mode}: {check}");
                }
                line(mode, ns, compile_ms, load_ms, check);
            };
        // SAFETY (each call below): buffers of n rows of D f32s and n outputs.
        go(
            "cranelift",
            &mut |o| unsafe { cl_scan(q.as_ptr(), r.as_ptr(), n as u64, o.as_mut_ptr()) },
            Some(cl_scan_ms),
            None,
            true,
        );
        let f: ScanFn = sym(&clang_lib, "scan");
        go(
            "clang",
            &mut |o| unsafe { f(q.as_ptr(), r.as_ptr(), n as u64, o.as_mut_ptr()) },
            Some(clang_ms),
            load(&clang_so, "scan"),
            true,
        );
        let f: ScanFn = sym(&fast_lib, "scan");
        go(
            "clang-fast",
            &mut |o| unsafe { f(q.as_ptr(), r.as_ptr(), n as u64, o.as_mut_ptr()) },
            Some(fast_ms),
            load(&fast_so, "scan"),
            false,
        );
        if let (Some(lib), Some((so, _))) = (&mlir_lib, &mlir) {
            let f: mlir::MScan = sym(lib, "_mlir_ciface_scan");
            go(
                "mlir",
                &mut |o| unsafe {
                    f(
                        &mut mlir::d1(q.as_ptr(), D),
                        &mut mlir::d1(r.as_ptr(), n * D),
                        &mut mlir::d1(o.as_mut_ptr().cast_const(), n),
                    )
                },
                mlir_ms,
                load(so, "_mlir_ciface_scan"),
                true,
            );
        }
        if let (Some(lib), Some((so, ms, _))) = (&acc_lib, &acc) {
            let f: ScanFn = sym(lib, "scan");
            go(
                "accelerate",
                &mut |o| unsafe { f(q.as_ptr(), r.as_ptr(), n as u64, o.as_mut_ptr()) },
                Some(*ms),
                load(so, "scan"),
                false,
            );
        }
    }

    // assign
    let rows = rng.units(an);
    let cents = rng.units(ak);
    let mut cents_t = vec![0f32; D * ak];
    for j in 0..ak {
        for l in 0..D {
            cents_t[l * ak + j] = cents[j * D + l];
        }
    }
    let line =
        |mode: &str, ns: f64, compile_ms: Option<f64>, load_ms: Option<f64>, check: String| {
            print(Line {
                kernel: "assign",
                rows: an,
                k: ak,
                mode,
                ns_call: ns,
                compile_ms,
                load_ms,
                check,
            })
        };
    // The kernel's rule as `kmeans` runs it (scores twice per comparison).
    let mut seq = vec![0u32; an];
    let kruns = if smoke { 1 } else { 2 };
    let ns = time(kruns, min_ms, || {
        for (i, o) in seq.iter_mut().enumerate() {
            let v = &rows[i * D..][..D];
            *o = (0..ak)
                .max_by(|&a, &b| {
                    vectors::dot(&cents[a * D..][..D], v)
                        .total_cmp(&vectors::dot(&cents[b * D..][..D], v))
                })
                .unwrap_or(0) as u32;
        }
    });
    line("kernel", ns, None, None, "\"same_centroids\":1.0000".into());
    let mut lanes = vec![0u32; an];
    let ns = time(runs, min_ms, || {
        for (i, o) in lanes.iter_mut().enumerate() {
            let v = &rows[i * D..][..D];
            let (mut best, mut bi) = (f32::NEG_INFINITY, 0u32);
            for j in 0..ak {
                let s = vectors::dot_lanes(&cents[j * D..][..D], v);
                if s.total_cmp(&best).is_ge() {
                    best = s;
                    bi = j as u32;
                }
            }
            *o = bi;
        }
    });
    line(
        "kernel-once",
        ns,
        None,
        None,
        check_assign(&seq, &lanes, false),
    );
    let mut out = vec![0u32; an];
    let mut scores = vec![0f32; an * ak];
    {
        let mut go = |mode: &str,
                      f: &mut dyn FnMut(&mut [u32]),
                      compile_ms,
                      load_ms,
                      reference: &[u32],
                      strict: bool| {
            out.iter_mut().for_each(|o| *o = u32::MAX);
            let ns = time(runs, min_ms, || f(&mut out));
            let mut check = check_assign(reference, &out, strict);
            let k = out.iter().zip(&seq).filter(|(a, b)| a == b).count() as f64 / an as f64;
            write!(check, ",\"same_as_kernel\":{k:.4}").unwrap();
            line(mode, ns, compile_ms, load_ms, check);
        };
        // SAFETY (each call below): an rows and ak centroids of D f32s, an outputs.
        go(
            "cranelift",
            &mut |o| unsafe {
                cl_assign(
                    rows.as_ptr(),
                    an as u64,
                    cents.as_ptr(),
                    ak as u64,
                    o.as_mut_ptr(),
                )
            },
            Some(cl_assign_ms),
            None,
            &lanes,
            true,
        );
        let f: AssignFn = sym(&clang_lib, "assign");
        go(
            "clang",
            &mut |o| unsafe {
                f(
                    rows.as_ptr(),
                    an as u64,
                    cents.as_ptr(),
                    ak as u64,
                    o.as_mut_ptr(),
                )
            },
            Some(clang_ms),
            load(&clang_so, "assign"),
            &lanes,
            true,
        );
        let f: AssignFn = sym(&fast_lib, "assign");
        go(
            "clang-fast",
            &mut |o| unsafe {
                f(
                    rows.as_ptr(),
                    an as u64,
                    cents.as_ptr(),
                    ak as u64,
                    o.as_mut_ptr(),
                )
            },
            Some(fast_ms),
            None,
            &lanes,
            false,
        );
        let f: ScoresFn = sym(&clang_lib, "scores_ikj");
        go(
            "clang-ikj",
            &mut |o| {
                unsafe {
                    f(
                        rows.as_ptr(),
                        an as u64,
                        cents_t.as_ptr(),
                        ak as u64,
                        scores.as_mut_ptr(),
                    )
                };
                argmax_rows(&scores, ak, o);
            },
            Some(clang_ms),
            load(&clang_so, "scores_ikj"),
            &seq,
            true,
        );
        let f: ScoresFn = sym(&fast_lib, "scores_ikj");
        go(
            "clang-fast-ikj",
            &mut |o| {
                unsafe {
                    f(
                        rows.as_ptr(),
                        an as u64,
                        cents_t.as_ptr(),
                        ak as u64,
                        scores.as_mut_ptr(),
                    )
                };
                argmax_rows(&scores, ak, o);
            },
            Some(fast_ms),
            None,
            &seq,
            false,
        );
        if let (Some(lib), Some((so, _))) = (&mlir_lib, &mlir) {
            let f: mlir::MAssign = sym(lib, "_mlir_ciface_assign");
            go(
                "mlir",
                &mut |o| unsafe {
                    f(
                        &mut mlir::d1(rows.as_ptr(), an * D),
                        &mut mlir::d1(cents.as_ptr(), ak * D),
                        &mut mlir::d1(o.as_mut_ptr().cast_const(), an),
                    )
                },
                mlir_ms,
                load(so, "_mlir_ciface_assign"),
                &lanes,
                true,
            );
            let g: mlir::MScores = sym(lib, "_mlir_ciface_scores");
            go(
                "mlir-linalg",
                &mut |o| {
                    scores.iter_mut().for_each(|s| *s = 0.0);
                    unsafe {
                        g(
                            &mut mlir::d2(rows.as_ptr(), an, D),
                            &mut mlir::d2(cents_t.as_ptr(), D, ak),
                            &mut mlir::d2(scores.as_mut_ptr().cast_const(), an, ak),
                        )
                    };
                    argmax_rows(&scores, ak, o);
                },
                mlir_ms,
                None,
                &seq,
                true,
            );
        }
        if let (Some(lib), Some((so, ms, _))) = (&acc_lib, &acc) {
            let f: ScoresFn = sym(lib, "scores");
            go(
                "accelerate",
                &mut |o| {
                    unsafe {
                        f(
                            rows.as_ptr(),
                            an as u64,
                            cents.as_ptr(),
                            ak as u64,
                            scores.as_mut_ptr(),
                        )
                    };
                    argmax_rows(&scores, ak, o);
                },
                Some(*ms),
                load(so, "scores"),
                &seq,
                false,
            );
        }
    }
}
