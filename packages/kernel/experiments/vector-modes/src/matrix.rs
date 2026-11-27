//! E423: one function run in every execution mode this machine has, with
//! each mode's outputs compared bit for bit and its compile and run cost.
//!
//! The function is E327's inner loop: score `n` int8 codes of `d`
//! dimensions against a query. Two forms:
//! - `i8`: an int8 query, exact i32 sums, so every mode must give the same
//!   bits. Every mode here runs it, the kernel's interpreter included.
//! - `f32`: an f32 query (what `vectors::code_dot` computes). Float sums
//!   depend on their order and on fused multiply-adds, so modes are only
//!   expected to agree with the modes that keep the same order. The
//!   kernel's interpreter runs with floats off on main, so it has no row.
//!
//! Extension point: a mode is a [`Mode`] (a name, a family, the
//! instruction set it needs, its compile cost, and a closure with the
//! function's signature), and a new mode is one more entry in
//! [`i8_modes`] or [`f32_modes`]: a builder that returns the mode, or a
//! [`Skip`] with the reason it cannot run here (tool or CPU feature
//! missing). Everything after that (the bit comparison against the
//! reference, timing at each size, the JSON lines) is shared.
//!
//! `cargo run --release -- matrix`, with `MLIR_BIN` for the MLIR rows
//! (E327's lowering; skipped without `mlir-opt`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, UserFuncName, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use ikam_kernel::{Arg, MemStore, Repo};

use crate::{CF32, CI8, Lcg, MF32, MI8, desc, quantize, sym};

type Run<Q, O> = Box<dyn Fn(&[Q], &[i8], usize, &mut [O])>;

/// One way to run the function.
pub struct Mode<Q, O> {
    pub name: String,
    /// interpreter, rust, lanes, intrinsics, kernel, clang, cranelift, mlir
    pub family: &'static str,
    /// The instruction set the mode is built for ("host" when it detects
    /// the processor's own at compile time).
    pub isa: &'static str,
    /// Milliseconds to compile at run time; `None` when built ahead of time
    /// by cargo.
    pub compile_ms: Option<f64>,
    pub run: Run<Q, O>,
}

/// A mode that cannot run here, and why.
pub struct Skip {
    pub name: String,
    pub reason: String,
}

type Built<Q, O> = Result<Mode<Q, O>, Skip>;

fn skip<Q, O>(name: &str, reason: impl Into<String>) -> Built<Q, O> {
    Err(Skip {
        name: name.into(),
        reason: reason.into(),
    })
}

fn aot<Q, O>(name: &str, family: &'static str, isa: &'static str, run: Run<Q, O>) -> Built<Q, O> {
    Ok(Mode {
        name: name.into(),
        family,
        isa,
        compile_ms: None,
        run,
    })
}

// ---------------------------------------------------------------- Rust ---

#[inline(always)]
fn i8_scalar(q: &[i8], codes: &[i8], d: usize, out: &mut [i32]) {
    for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
        *o = q
            .iter()
            .zip(c)
            .map(|(&x, &c)| i32::from(x) * i32::from(c))
            .sum();
    }
}

#[inline(always)]
fn i8_lanes(q: &[i8], codes: &[i8], d: usize, out: &mut [i32]) {
    for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
        let mut acc = [0i32; 16];
        let (qs, cs) = (q.chunks_exact(16), c.chunks_exact(16));
        let tail: i32 = qs
            .remainder()
            .iter()
            .zip(cs.remainder())
            .map(|(&x, &c)| i32::from(x) * i32::from(c))
            .sum();
        for (q, c) in qs.zip(cs) {
            for l in 0..16 {
                acc[l] += i32::from(q[l]) * i32::from(c[l]);
            }
        }
        *o = acc.iter().sum::<i32>() + tail;
    }
}

/// `vectors::code_dot` as written: one running f32 sum.
#[inline(always)]
fn f32_scalar(q: &[f32], codes: &[i8], d: usize, out: &mut [f32]) {
    for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
        *o = q.iter().zip(c).map(|(x, &c)| x * f32::from(c)).sum();
    }
}

/// `vectors::code_dot_lanes_body`: 16 sums added at the end.
#[inline(always)]
fn f32_lanes(q: &[f32], codes: &[i8], d: usize, out: &mut [f32]) {
    for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
        let mut acc = [0f32; 16];
        let (qs, cs) = (q.chunks_exact(16), c.chunks_exact(16));
        let tail: f32 = qs
            .remainder()
            .iter()
            .zip(cs.remainder())
            .map(|(x, &c)| x * f32::from(c))
            .sum();
        for (q, c) in qs.zip(cs) {
            for l in 0..16 {
                acc[l] += q[l] * f32::from(c[l]);
            }
        }
        *o = acc.iter().sum::<f32>() + tail;
    }
}

/// The same Rust body compiled for one instruction set (E339's mechanism:
/// `#[target_feature]` on a wrapper, chosen at run time after a check).
macro_rules! at {
    ($name:ident, $feat:literal, $body:ident, $q:ty, $o:ty) => {
        #[target_feature(enable = $feat)]
        pub(super) unsafe fn $name(q: &[$q], c: &[i8], d: usize, out: &mut [$o]) {
            $body(q, c, d, out)
        }
    };
}

macro_rules! isa_set {
    ($body:ident, $q:ty, $o:ty, $m:ident) => {
        mod $m {
            use super::*;
            at!(v2, "sse3,ssse3,sse4.1,sse4.2,popcnt", $body, $q, $o);
            at!(v3, "avx,avx2,bmi1,bmi2,fma,lzcnt,movbe", $body, $q, $o);
            at!(
                v4,
                "avx,avx2,bmi1,bmi2,fma,lzcnt,movbe,avx512f,avx512bw,avx512cd,avx512dq,avx512vl",
                $body,
                $q,
                $o
            );
            at!(
                v4vnni,
                "avx,avx2,bmi1,bmi2,fma,lzcnt,movbe,avx512f,avx512bw,avx512cd,avx512dq,avx512vl,avx512vnni",
                $body,
                $q,
                $o
            );
        }
    };
}
isa_set!(i8_scalar, i8, i32, i8_scalar_at);
isa_set!(i8_lanes, i8, i32, i8_lanes_at);
isa_set!(f32_scalar, f32, f32, f32_scalar_at);
isa_set!(f32_lanes, f32, f32, f32_lanes_at);

/// The ISA levels of the Rust rows: (label, what the CPU must have).
const LEVELS: [(&str, &[&str]); 5] = [
    ("sse2", &[]),
    ("x86-64-v2", &["sse4.2", "popcnt"]),
    ("x86-64-v3", &["avx2", "fma", "bmi2"]),
    (
        "x86-64-v4",
        &["avx2", "fma", "avx512f", "avx512bw", "avx512dq", "avx512vl"],
    ),
    (
        "v4+vnni",
        &[
            "avx2",
            "fma",
            "avx512f",
            "avx512bw",
            "avx512vl",
            "avx512vnni",
        ],
    ),
];

fn has(feature: &str) -> bool {
    match feature {
        "sse4.2" => is_x86_feature_detected!("sse4.2"),
        "popcnt" => is_x86_feature_detected!("popcnt"),
        "avx2" => is_x86_feature_detected!("avx2"),
        "fma" => is_x86_feature_detected!("fma"),
        "bmi2" => is_x86_feature_detected!("bmi2"),
        "avx512f" => is_x86_feature_detected!("avx512f"),
        "avx512bw" => is_x86_feature_detected!("avx512bw"),
        "avx512dq" => is_x86_feature_detected!("avx512dq"),
        "avx512vl" => is_x86_feature_detected!("avx512vl"),
        "avx512vnni" => is_x86_feature_detected!("avx512vnni"),
        "avxvnni" => is_x86_feature_detected!("avxvnni"),
        _ => false,
    }
}

fn missing(needs: &[&str]) -> Option<String> {
    let lack: Vec<&str> = needs.iter().copied().filter(|f| !has(f)).collect();
    (!lack.is_empty()).then(|| format!("CPU lacks {}", lack.join(", ")))
}

/// Rust rows for one body at every level. `fns[0]` is the plain build.
fn rust_rows<Q: 'static, O: 'static>(
    stem: &str,
    family: &'static str,
    fns: [fn(&[Q], &[i8], usize, &mut [O]); 5],
) -> Vec<Built<Q, O>> {
    LEVELS
        .iter()
        .zip(fns)
        .map(|(&(level, needs), f)| {
            let name = format!("{stem}@{level}");
            match missing(needs) {
                Some(why) => skip(&name, why),
                None => aot(&name, family, level, Box::new(f)),
            }
        })
        .collect()
}

macro_rules! leveled {
    ($plain:ident, $m:ident) => {
        [
            $plain,
            // Safety: each wrapper runs only where `missing` found the
            // features it is compiled for.
            |q, c, d, o| unsafe { $m::v2(q, c, d, o) },
            |q, c, d, o| unsafe { $m::v3(q, c, d, o) },
            |q, c, d, o| unsafe { $m::v4(q, c, d, o) },
            |q, c, d, o| unsafe { $m::v4vnni(q, c, d, o) },
        ]
    };
}

// ---------------------------------------------------------- intrinsics ---

/// E327's AVX2 row: widen to i16, `vpmaddwd`.
#[target_feature(enable = "avx2")]
unsafe fn i8_avx2_madd(q: &[i8], codes: &[i8], d: usize, out: &mut [i32]) {
    use std::arch::x86_64::*;
    // SAFETY: loads read 16 bytes at j < d, with d a multiple of 64.
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

/// The same at 512 bits.
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn i8_avx512_madd(q: &[i8], codes: &[i8], d: usize, out: &mut [i32]) {
    use std::arch::x86_64::*;
    // SAFETY: loads read 32 bytes at j < d, with d a multiple of 64.
    unsafe {
        for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
            let mut acc = _mm512_setzero_si512();
            for j in (0..d).step_by(32) {
                let qv = _mm512_cvtepi8_epi16(_mm256_loadu_si256(q.as_ptr().add(j).cast()));
                let cv = _mm512_cvtepi8_epi16(_mm256_loadu_si256(c.as_ptr().add(j).cast()));
                acc = _mm512_add_epi32(acc, _mm512_madd_epi16(qv, cv));
            }
            *o = _mm512_reduce_add_epi32(acc);
        }
    }
}

/// AVX-512 VNNI: `vpdpbusd` multiplies unsigned by signed bytes, so the
/// query is offset by 128 and 128 times the code's sum is taken back off
/// (both exact in i32: at most 255 * 127 * d).
#[target_feature(enable = "avx512f,avx512bw,avx512vnni")]
unsafe fn i8_avx512_vnni(q: &[i8], codes: &[i8], d: usize, out: &mut [i32]) {
    use std::arch::x86_64::*;
    let qu: Vec<u8> = q.iter().map(|&x| (i16::from(x) + 128) as u8).collect();
    // SAFETY: loads read 64 bytes at j < d, with d a multiple of 64.
    unsafe {
        let ones = _mm512_set1_epi8(1);
        for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
            let (mut acc, mut sum) = (_mm512_setzero_si512(), _mm512_setzero_si512());
            for j in (0..d).step_by(64) {
                let qv = _mm512_loadu_si512(qu.as_ptr().add(j).cast());
                let cv = _mm512_loadu_si512(c.as_ptr().add(j).cast());
                acc = _mm512_dpbusd_epi32(acc, qv, cv);
                sum = _mm512_dpbusd_epi32(sum, ones, cv);
            }
            *o = _mm512_reduce_add_epi32(acc) - 128 * _mm512_reduce_add_epi32(sum);
        }
    }
}

/// AVX-VNNI (the 256-bit VEX form), the same offset.
#[target_feature(enable = "avx2,avxvnni")]
unsafe fn i8_avx_vnni(q: &[i8], codes: &[i8], d: usize, out: &mut [i32]) {
    use std::arch::x86_64::*;
    let qu: Vec<u8> = q.iter().map(|&x| (i16::from(x) + 128) as u8).collect();
    // SAFETY: loads read 32 bytes at j < d, with d a multiple of 64.
    unsafe {
        let ones = _mm256_set1_epi8(1);
        for (o, c) in out.iter_mut().zip(codes.chunks_exact(d)) {
            let (mut acc, mut sum) = (_mm256_setzero_si256(), _mm256_setzero_si256());
            for j in (0..d).step_by(32) {
                let qv = _mm256_loadu_si256(qu.as_ptr().add(j).cast());
                let cv = _mm256_loadu_si256(c.as_ptr().add(j).cast());
                acc = _mm256_dpbusd_avx_epi32(acc, qv, cv);
                sum = _mm256_dpbusd_avx_epi32(sum, ones, cv);
            }
            let mut a = [0i32; 8];
            let mut s = [0i32; 8];
            _mm256_storeu_si256(a.as_mut_ptr().cast(), acc);
            _mm256_storeu_si256(s.as_mut_ptr().cast(), sum);
            *o = a.iter().sum::<i32>() - 128 * s.iter().sum::<i32>();
        }
    }
}

// --------------------------------------------------------- interpreter ---

/// The `i8` function as a stored WASM module, in the kernel's ABI
/// (`wasm.rs`): arguments the query and the codes, output `n` i32s
/// little-endian. Integer only, so it runs with the kernel's floats off.
pub const WAT: &str = r#"(module
  (memory (export "memory") 1)
  (global $top (mut i32) (i32.const 1024))
  (func $alloc (export "alloc") (param $len i32) (result i32)
    (local $p i32) (local $end i32) (local $have i32)
    (local.set $p (global.get $top))
    (local.set $end (i32.add (local.get $p) (local.get $len)))
    (local.set $have (i32.shl (memory.size) (i32.const 16)))
    (if (i32.gt_u (local.get $end) (local.get $have))
      (then (drop (memory.grow (i32.shr_u
        (i32.add (i32.sub (local.get $end) (local.get $have)) (i32.const 65535))
        (i32.const 16))))))
    (global.set $top (local.get $end))
    (local.get $p))
  (func $be (param $p i32) (result i32)
    (i32.or
      (i32.or (i32.shl (i32.load8_u (local.get $p)) (i32.const 24))
              (i32.shl (i32.load8_u offset=1 (local.get $p)) (i32.const 16)))
      (i32.or (i32.shl (i32.load8_u offset=2 (local.get $p)) (i32.const 8))
              (i32.load8_u offset=3 (local.get $p)))))
  (func (export "run") (param $ptr i32) (param $len i32) (result i64)
    (local $d i32) (local $q i32) (local $c i32) (local $n i32) (local $out i32)
    (local $i i32) (local $j i32) (local $s i32) (local $row i32)
    (local.set $d (call $be (i32.add (local.get $ptr) (i32.const 4))))
    (local.set $q (i32.add (local.get $ptr) (i32.const 8)))
    (local.set $c (i32.add (i32.add (local.get $q) (local.get $d)) (i32.const 4)))
    (if (i32.ne (local.get $d) (i32.const 0))
      (then (local.set $n (i32.div_u
        (call $be (i32.add (local.get $q) (local.get $d))) (local.get $d)))))
    (local.set $out (call $alloc (i32.shl (local.get $n) (i32.const 2))))
    (block $rows_done (loop $rows
      (br_if $rows_done (i32.ge_u (local.get $i) (local.get $n)))
      (local.set $row (i32.add (local.get $c) (i32.mul (local.get $i) (local.get $d))))
      (local.set $s (i32.const 0))
      (local.set $j (i32.const 0))
      (block $cols_done (loop $cols
        (br_if $cols_done (i32.ge_u (local.get $j) (local.get $d)))
        (local.set $s (i32.add (local.get $s) (i32.mul
          (i32.load8_s (i32.add (local.get $q) (local.get $j)))
          (i32.load8_s (i32.add (local.get $row) (local.get $j))))))
        (local.set $j (i32.add (local.get $j) (i32.const 1)))
        (br $cols)))
      (i32.store (i32.add (local.get $out) (i32.shl (local.get $i) (i32.const 2))) (local.get $s))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $rows)))
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $out)) (i64.const 32))
      (i64.extend_i32_u (i32.shl (local.get $n) (i32.const 2))))))
"#;

fn bytes(v: &[i8]) -> Vec<u8> {
    v.iter().map(|&x| x as u8).collect()
}

/// The kernel's own path: the module stored as content, applied with
/// `Repo::evaluate` (wasmi with fuel, floats off). Each call stores the
/// arguments as content first (deduplicated after the first call), as a
/// caller of `evaluate` would.
fn interp() -> Built<i8, i32> {
    let wasm = wat::parse_str(WAT).expect("wat");
    let repo = std::cell::RefCell::new(Repo::new(MemStore::default()));
    let module = repo.borrow_mut().put_content(&wasm).unwrap().id;
    let call = move |q: &[i8], c: &[i8], out: &mut [i32]| {
        let mut r = repo.borrow_mut();
        let qa = Arg::Whole(r.put_content(&bytes(q)).unwrap().id);
        let ca = Arg::Whole(r.put_content(&bytes(c)).unwrap().id);
        let (got, _work) = r.evaluate(&module, &[qa, ca]).expect("evaluate");
        for (o, b) in out.iter_mut().zip(got.chunks_exact(4)) {
            *o = i32::from_le_bytes(b.try_into().unwrap());
        }
    };
    // Compile cost: the first call (wasmi validates and translates, then
    // caches by hash) less a later call, on one code.
    let (q, c) = (vec![1i8; 64], vec![2i8; 64]);
    let mut one = [0i32; 1];
    let t = Instant::now();
    call(&q, &c, &mut one);
    let first = t.elapsed().as_secs_f64();
    let mut later = f64::MAX;
    for _ in 0..20 {
        let t = Instant::now();
        call(&q, &c, &mut one);
        later = later.min(t.elapsed().as_secs_f64());
    }
    assert_eq!(one[0], 128);
    Ok(Mode {
        name: "interp (kernel wasmi)".into(),
        family: "interpreter",
        isa: "portable",
        compile_ms: Some((first - later).max(0.0) * 1e3),
        run: Box::new(move |q, c, _d, out| call(q, c, out)),
    })
}

// ----------------------------------------------------------- Cranelift ---

#[derive(Clone, Copy, PartialEq)]
enum Clif {
    I8Scalar,
    I8Simd,
    F32Scalar,
}

/// The function built with cranelift-frontend and compiled by cranelift-jit
/// at `opt_level=speed`, for the host's instruction set (`native`) or
/// x86-64's baseline (SSE2 only). Signature as the C one:
/// `(q, codes, out, n, d)`; the SIMD form needs `d` a multiple of 16.
fn clif(kind: Clif, native: bool) -> (JITModule, *const u8, f64) {
    let t = Instant::now();
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    flags.set("use_colocated_libcalls", "false").unwrap();
    flags.set("is_pic", "false").unwrap();
    let isa = cranelift_native::builder_with_options(native)
        .unwrap()
        .finish(settings::Flags::new(flags))
        .unwrap();
    let mut module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));
    let mut ctx = module.make_context();
    let i64t = types::I64;
    for _ in 0..5 {
        ctx.func.signature.params.push(AbiParam::new(i64t));
    }
    ctx.func.name = UserFuncName::user(0, 0);
    let tcfg = module.target_config();
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let fl = MemFlagsData::trusted();
        let entry = b.create_block();
        let rows = b.create_block();
        let row = b.create_block();
        let cols = b.create_block();
        let col = b.create_block();
        let store = b.create_block();
        let exit = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let ps = b.block_params(entry).to_vec();
        let (q, c, out, n, d) = (ps[0], ps[1], ps[2], ps[3], ps[4]);
        let acc_ty = match kind {
            Clif::I8Scalar => types::I32,
            Clif::I8Simd => types::I32X4,
            Clif::F32Scalar => types::F32,
        };
        let (vi, vj, vrow, vacc) = (
            b.declare_var(i64t),
            b.declare_var(i64t),
            b.declare_var(i64t),
            b.declare_var(acc_ty),
        );
        let zero = b.ins().iconst(i64t, 0);
        b.def_var(vi, zero);
        b.ins().jump(rows, &[]);

        b.switch_to_block(rows);
        let i = b.use_var(vi);
        let done = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, n);
        b.ins().brif(done, exit, &[], row, &[]);

        b.switch_to_block(row);
        let i = b.use_var(vi);
        let off = b.ins().imul(i, d);
        let base = b.ins().iadd(c, off);
        b.def_var(vrow, base);
        let z = match kind {
            Clif::I8Scalar => b.ins().iconst(types::I32, 0),
            Clif::I8Simd => {
                let z = b.ins().iconst(types::I32, 0);
                b.ins().splat(types::I32X4, z)
            }
            Clif::F32Scalar => b.ins().f32const(0.0),
        };
        b.def_var(vacc, z);
        b.def_var(vj, zero);
        b.ins().jump(cols, &[]);

        b.switch_to_block(cols);
        let j = b.use_var(vj);
        let done = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, j, d);
        b.ins().brif(done, store, &[], col, &[]);

        b.switch_to_block(col);
        let j = b.use_var(vj);
        let base = b.use_var(vrow);
        let a = b.use_var(vacc);
        let ca = b.ins().iadd(base, j);
        let (next, step) = match kind {
            Clif::I8Scalar => {
                let qa = b.ins().iadd(q, j);
                let x = b.ins().sload8(types::I32, fl, qa, 0);
                let y = b.ins().sload8(types::I32, fl, ca, 0);
                let p = b.ins().imul(x, y);
                (b.ins().iadd(a, p), 1)
            }
            Clif::I8Simd => {
                let qa = b.ins().iadd(q, j);
                let x = b.ins().load(types::I8X16, fl, qa, 0);
                let y = b.ins().load(types::I8X16, fl, ca, 0);
                let (xl, xh) = (b.ins().swiden_low(x), b.ins().swiden_high(x));
                let (yl, yh) = (b.ins().swiden_low(y), b.ins().swiden_high(y));
                // i16 products are exact (at most 127 * 127), then widened.
                let pl = b.ins().imul(xl, yl);
                let ph = b.ins().imul(xh, yh);
                let mut s = a;
                for p in [pl, ph] {
                    let lo = b.ins().swiden_low(p);
                    let hi = b.ins().swiden_high(p);
                    s = b.ins().iadd(s, lo);
                    s = b.ins().iadd(s, hi);
                }
                (s, 16)
            }
            Clif::F32Scalar => {
                let j4 = b.ins().ishl_imm_u(j, 2);
                let qa = b.ins().iadd(q, j4);
                let x = b.ins().load(types::F32, fl, qa, 0);
                let y = b.ins().sload8(types::I32, fl, ca, 0);
                let y = b.ins().fcvt_from_sint(types::F32, y);
                let p = b.ins().fmul(x, y);
                (b.ins().fadd(a, p), 1)
            }
        };
        b.def_var(vacc, next);
        let j1 = b.ins().iadd_imm_s(j, step);
        b.def_var(vj, j1);
        b.ins().jump(cols, &[]);

        b.switch_to_block(store);
        let i = b.use_var(vi);
        let a = b.use_var(vacc);
        let r = if kind == Clif::I8Simd {
            let l0 = b.ins().extractlane(a, 0);
            let l1 = b.ins().extractlane(a, 1);
            let l2 = b.ins().extractlane(a, 2);
            let l3 = b.ins().extractlane(a, 3);
            let s0 = b.ins().iadd(l0, l1);
            let s1 = b.ins().iadd(l2, l3);
            b.ins().iadd(s0, s1)
        } else {
            a
        };
        let i4 = b.ins().ishl_imm_u(i, 2);
        let oa = b.ins().iadd(out, i4);
        b.ins().store(fl, r, oa, 0);
        let i1 = b.ins().iadd_imm_s(i, 1);
        b.def_var(vi, i1);
        b.ins().jump(rows, &[]);

        b.switch_to_block(exit);
        b.ins().return_(&[]);
        b.seal_all_blocks();
        b.finalize(tcfg);
    }
    let id = module
        .declare_function("score", Linkage::Export, &ctx.func.signature)
        .unwrap();
    module.define_function(id, &mut ctx).unwrap();
    module.clear_context(&mut ctx);
    module.finalize_definitions().unwrap();
    let ptr = module.get_finalized_function(id);
    (module, ptr, t.elapsed().as_secs_f64() * 1e3)
}

fn clif_i8(name: &str, kind: Clif, native: bool) -> Built<i8, i32> {
    let _ = clif(kind, native); // warm-up: the first JIT in a process pays for setup
    let (module, ptr, ms) = clif(kind, native);
    // SAFETY: compiled just above with the CI8 signature.
    let f = unsafe { std::mem::transmute::<*const u8, CI8>(ptr) };
    Ok(Mode {
        name: name.into(),
        family: "cranelift",
        isa: if native { "host" } else { "sse2" },
        compile_ms: Some(ms),
        run: Box::new(move |q, c, d, out| {
            let _keep = &module;
            unsafe {
                f(
                    q.as_ptr(),
                    c.as_ptr(),
                    out.as_mut_ptr(),
                    out.len() as i64,
                    d as i64,
                )
            }
        }),
    })
}

fn clif_f32(name: &str, native: bool) -> Built<f32, f32> {
    let _ = clif(Clif::F32Scalar, native);
    let (module, ptr, ms) = clif(Clif::F32Scalar, native);
    // SAFETY: compiled just above with the CF32 signature.
    let f = unsafe { std::mem::transmute::<*const u8, CF32>(ptr) };
    Ok(Mode {
        name: name.into(),
        family: "cranelift",
        isa: if native { "host" } else { "sse2" },
        compile_ms: Some(ms),
        run: Box::new(move |q, c, d, out| {
            let _keep = &module;
            unsafe {
                f(
                    q.as_ptr(),
                    c.as_ptr(),
                    out.as_mut_ptr(),
                    out.len() as i64,
                    d as i64,
                )
            }
        }),
    })
}

// --------------------------------------------------------------- clang ---

/// clang builds of E327's C file: (row, flags, what the CPU must have).
const CLANG: [(&str, &str, &[&str]); 8] = [
    ("clang@x86-64", "-march=x86-64", &[]),
    ("clang@x86-64-v2", "-march=x86-64-v2", &["sse4.2"]),
    ("clang@x86-64-v3", "-march=x86-64-v3", &["avx2", "fma"]),
    (
        "clang@x86-64-v4",
        "-march=x86-64-v4",
        &["avx512f", "avx512bw", "avx512vl"],
    ),
    ("clang@native", "-march=native", &[]),
    (
        "clang@v4 -ffp-contract=off",
        "-march=x86-64-v4 -ffp-contract=off",
        &["avx512f", "avx512bw", "avx512vl"],
    ),
    (
        "clang@v4 -ffast-math",
        "-march=x86-64-v4 -ffast-math",
        &["avx512f", "avx512bw", "avx512vl"],
    ),
    ("clang@native -ffast-math", "-march=native -ffast-math", &[]),
];

/// The `isa` label of a clang row.
fn clang_isa(flags: &str) -> &'static str {
    match flags.split(' ').next().unwrap() {
        "-march=x86-64" => "sse2",
        "-march=x86-64-v2" => "x86-64-v2",
        "-march=x86-64-v3" => "x86-64-v3",
        "-march=x86-64-v4" => "x86-64-v4",
        _ => "host",
    }
}

fn run_ms(cmd: &mut Command) -> Result<f64, String> {
    let t = Instant::now();
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(t.elapsed().as_secs_f64() * 1e3)
}

/// Each clang build once: the library and its compile time.
fn clang_libs(dir: &Path) -> Vec<(usize, Result<(PathBuf, f64), String>)> {
    let c = dir.join("m.c");
    std::fs::write(&c, crate::C_SRC).unwrap();
    // Untimed warm-up, so the first row does not pay for loading clang.
    let _ = run_ms(
        Command::new("clang")
            .args(["-O3", "-shared", "-fPIC", "-o"])
            .arg(dir.join("warm.so"))
            .arg(&c),
    );
    CLANG
        .iter()
        .enumerate()
        .map(|(i, &(_, flags, needs))| {
            if let Some(why) = missing(needs) {
                return (i, Err(why));
            }
            let so = dir.join(format!("clang{i}.so"));
            let ms = run_ms(
                Command::new("clang")
                    .args(["-O3", "-shared", "-fPIC"])
                    .args(flags.split(' '))
                    .arg("-o")
                    .arg(&so)
                    .arg(&c),
            );
            (i, ms.map(|ms| (so, ms)))
        })
        .collect()
}

// ---------------------------------------------------------------- MLIR ---

/// E327's MLIR libraries (`mlir+llc`, `mlir+opt`) with each one's compile
/// time (its stages summed), or why there are none.
fn mlir_libs(dir: &Path) -> Result<[(PathBuf, f64); 2], String> {
    let bin = match std::env::var("MLIR_BIN") {
        Ok(b) => PathBuf::from(b),
        Err(_) => {
            return Err("mlir-opt and mlir-translate not found (MLIR_BIN unset)".into());
        }
    };
    for tool in ["mlir-opt", "mlir-translate", "opt", "llc"] {
        if !bin.join(tool).exists() {
            return Err(format!("{tool} not found in MLIR_BIN"));
        }
    }
    let sub = dir.join("mlir");
    std::fs::create_dir_all(&sub).unwrap();
    let (_, _, m) = crate::build(&sub);
    let (llc, opt, stages) = m.ok_or("MLIR_BIN unset")?;
    let ms = |s: &str| {
        stages
            .iter()
            .filter(|(n, _)| n == s)
            .map(|(_, t)| t)
            .sum::<f64>()
    };
    let common = ms("mlir-opt") + ms("mlir-translate");
    Ok([
        (llc, common + ms("llc mlir_llc")),
        (opt, common + ms("opt -O3") + ms("llc mlir_opt")),
    ])
}

// ------------------------------------------------------------- modes ---

/// Every mode of the `i8` function, the reference (`rust@sse2`) first.
pub fn i8_modes(dir: &Path) -> Vec<Built<i8, i32>> {
    let mut m = rust_rows("rust", "rust", leveled!(i8_scalar, i8_scalar_at));
    m.extend(rust_rows("lanes", "lanes", leveled!(i8_lanes, i8_lanes_at)));
    m.push(match missing(&["avx2"]) {
        Some(w) => skip("avx2 madd", w),
        // Safety: AVX2 checked.
        None => aot(
            "avx2 madd",
            "intrinsics",
            "x86-64-v3",
            Box::new(|q, c, d, o| unsafe { i8_avx2_madd(q, c, d, o) }),
        ),
    });
    m.push(match missing(&["avx512f", "avx512bw"]) {
        Some(w) => skip("avx512 madd", w),
        // Safety: AVX-512 F and BW checked.
        None => aot(
            "avx512 madd",
            "intrinsics",
            "x86-64-v4",
            Box::new(|q, c, d, o| unsafe { i8_avx512_madd(q, c, d, o) }),
        ),
    });
    m.push(match missing(&["avx512f", "avx512bw", "avx512vnni"]) {
        Some(w) => skip("avx512-vnni dpbusd", w),
        // Safety: AVX-512 F, BW and VNNI checked.
        None => aot(
            "avx512-vnni dpbusd",
            "intrinsics",
            "v4+vnni",
            Box::new(|q, c, d, o| unsafe { i8_avx512_vnni(q, c, d, o) }),
        ),
    });
    m.push(match missing(&["avx2", "avxvnni"]) {
        Some(w) => skip("avx-vnni dpbusd", w),
        // Safety: AVX2 and AVX-VNNI checked.
        None => aot(
            "avx-vnni dpbusd",
            "intrinsics",
            "avx-vnni",
            Box::new(|q, c, d, o| unsafe { i8_avx_vnni(q, c, d, o) }),
        ),
    });
    m.push(interp());
    m.push(clif_i8("cranelift scalar", Clif::I8Scalar, true));
    m.push(clif_i8("cranelift simd", Clif::I8Simd, true));
    m.push(clif_i8("cranelift simd@sse2", Clif::I8Simd, false));
    for (i, lib) in clang_libs(dir) {
        let name = CLANG[i].0;
        if CLANG[i].1.contains("fast-math") || CLANG[i].1.contains("fp-contract") {
            continue; // float flags: the same integer code
        }
        m.push(match lib {
            Err(why) => skip(name, why),
            Ok((so, ms)) => {
                let f: CI8 = sym(&so, "score_i8");
                Ok(Mode {
                    name: name.into(),
                    family: "clang",
                    isa: clang_isa(CLANG[i].1),
                    compile_ms: Some(ms),
                    run: Box::new(move |q, c, d, o| unsafe {
                        f(
                            q.as_ptr(),
                            c.as_ptr(),
                            o.as_mut_ptr(),
                            o.len() as i64,
                            d as i64,
                        )
                    }),
                })
            }
        });
    }
    match mlir_libs(dir) {
        Err(why) => {
            m.push(skip("mlir+llc", why.clone()));
            m.push(skip("mlir+opt", why));
        }
        Ok([(llc, a), (opt, b)]) => {
            for (tag, lib, ms) in [("mlir+llc", llc, a), ("mlir+opt", opt, b)] {
                let g: MI8 = sym(&lib, "_mlir_ciface_score_i8");
                m.push(Ok(Mode {
                    name: tag.into(),
                    family: "mlir",
                    isa: "host",
                    compile_ms: Some(ms),
                    run: Box::new(move |q, c, d, o| unsafe {
                        g(&desc(q), &desc(c), &desc(o), o.len() as i64, d as i64)
                    }),
                }));
            }
        }
    }
    m.push(skip("neon (Apple silicon)", "on the Mac after Nov 29"));
    m
}

/// Every mode of the `f32` function, the reference (`rust@sse2`) first.
pub fn f32_modes(dir: &Path) -> Vec<Built<f32, f32>> {
    let mut m = rust_rows("rust", "rust", leveled!(f32_scalar, f32_scalar_at));
    m.extend(rust_rows(
        "lanes",
        "lanes",
        leveled!(f32_lanes, f32_lanes_at),
    ));
    m.push(aot(
        "kernel code_dot_lanes",
        "kernel",
        "dispatched",
        Box::new(|q, c, d, out| {
            for (o, code) in out.iter_mut().zip(c.chunks_exact(d)) {
                // Safety: i8 and u8 have the same size and alignment.
                let code: &[u8] = unsafe { std::slice::from_raw_parts(code.as_ptr().cast(), d) };
                *o = ikam_kernel::vectors::code_dot_lanes(q, code);
            }
        }),
    ));
    m.push(skip(
        "interp (kernel wasmi)",
        "floats off on main (E441's Floats::Pinned not merged)",
    ));
    m.push(clif_f32("cranelift scalar", true));
    m.push(clif_f32("cranelift scalar@sse2", false));
    for (i, lib) in clang_libs(dir) {
        let name = CLANG[i].0;
        m.push(match lib {
            Err(why) => skip(name, why),
            Ok((so, ms)) => {
                let f: CF32 = sym(&so, "score_f32");
                Ok(Mode {
                    name: name.into(),
                    family: "clang",
                    isa: clang_isa(CLANG[i].1),
                    compile_ms: Some(ms),
                    run: Box::new(move |q, c, d, o| unsafe {
                        f(
                            q.as_ptr(),
                            c.as_ptr(),
                            o.as_mut_ptr(),
                            o.len() as i64,
                            d as i64,
                        )
                    }),
                })
            }
        });
    }
    match mlir_libs(dir) {
        Err(why) => {
            m.push(skip("mlir+llc", why.clone()));
            m.push(skip("mlir+opt", why));
        }
        Ok([(llc, a), (opt, b)]) => {
            for (tag, lib, ms) in [("mlir+llc", llc, a), ("mlir+opt", opt, b)] {
                let g: MF32 = sym(&lib, "_mlir_ciface_score_f32");
                m.push(Ok(Mode {
                    name: tag.into(),
                    family: "mlir",
                    isa: "host",
                    compile_ms: Some(ms),
                    run: Box::new(move |q, c, d, o| unsafe {
                        g(&desc(q), &desc(c), &desc(o), o.len() as i64, d as i64)
                    }),
                }));
            }
        }
    }
    m.push(skip("neon (Apple silicon)", "on the Mac after Nov 29"));
    m
}

// ------------------------------------------------------------ the run ---

/// Fastest of 5 runs, each repeating `f` for at least 200 ms; ns a call.
fn time(mut f: impl FnMut()) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..5 {
        let (t, mut reps) = (Instant::now(), 0u64);
        while t.elapsed().as_millis() < 200 || reps == 0 {
            f();
            reps += 1;
        }
        best = best.min(t.elapsed().as_secs_f64() * 1e9 / reps as f64);
    }
    best
}

/// Distance in representable f32s (0 for equal bits).
fn ulps(a: f32, b: f32) -> u64 {
    let key = |x: f32| {
        let i = x.to_bits() as i32;
        i64::from(if i < 0 { i32::MIN - i } else { i })
    };
    key(a).abs_diff(key(b))
}

fn json_skip(func: &str, s: &Skip) {
    println!(
        "{{\"fn\":\"{func}\",\"mode\":\"{}\",\"skipped\":\"{}\"}}",
        s.name,
        s.reason.replace('"', "'")
    );
}

fn report<Q: Copy, O: Copy + Default + PartialEq>(
    func: &str,
    modes: &[Built<Q, O>],
    q: &[Q],
    codes: &[i8],
    n: usize,
    d: usize,
    diff: impl Fn(&[O], &[O]) -> (bool, u64),
) {
    // Outputs seen so far, each under the first mode that gave those bits.
    let mut classes: Vec<(&str, Vec<O>)> = Vec::new();
    for m in modes {
        let m = match m {
            Ok(m) => m,
            Err(s) => {
                if n == SIZES[0] {
                    json_skip(func, s);
                }
                continue;
            }
        };
        let mut out = vec![O::default(); n];
        (m.run)(q, &codes[..n * d], d, &mut out);
        if classes.is_empty() {
            classes.push((&m.name, out.clone()));
        }
        let want = &classes[0].1;
        let (same, worst) = diff(want, &out);
        let rows = (0..n)
            .filter(|&i| !diff(&want[i..i + 1], &out[i..i + 1]).0)
            .count();
        let class = match classes.iter().find(|(_, o)| diff(o, &out).0) {
            Some((name, _)) => name.to_string(),
            None => {
                classes.push((&m.name, out.clone()));
                m.name.clone()
            }
        };
        let ns = time(|| (m.run)(q, &codes[..n * d], d, &mut out));
        let compile = m.compile_ms.map_or("null".into(), |c| format!("{c:.2}"));
        println!(
            "{{\"fn\":\"{func}\",\"mode\":\"{}\",\"family\":\"{}\",\"isa\":\"{}\",\"n\":{n},\"d\":{d},\"compile_ms\":{compile},\"ns_call\":{ns:.0},\"ns_code\":{:.2},\"same_bits\":{same},\"rows_differ\":{rows},\"max_ulp\":{worst},\"bits_of\":\"{class}\"}}",
            m.name,
            m.family,
            m.isa,
            ns / n as f64
        );
    }
}

pub const SIZES: [usize; 3] = [64, 1024, 16384];

pub fn main() {
    let d = 256;
    let dir = tempfile::tempdir().unwrap();
    let i8s = i8_modes(dir.path());
    let f32s = f32_modes(dir.path());
    let mut rng = Lcg(423);
    let nmax = SIZES[SIZES.len() - 1];
    let codes: Vec<i8> = (0..nmax).flat_map(|_| quantize(&rng.unit(d))).collect();
    let q = rng.unit(d);
    let qi = quantize(&q);
    for n in SIZES {
        report("i8", &i8s, &qi, &codes, n, d, |a, b| (a == b, 0));
        report("f32", &f32s, &q, &codes, n, d, |a, b| {
            let worst = a
                .iter()
                .zip(b)
                .map(|(&x, &y)| ulps(x, y))
                .max()
                .unwrap_or(0);
            (
                worst == 0 && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits()),
                worst,
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mode that runs here gives the reference's bits on the `i8`
    /// function, on lengths that are and are not multiples of 16 (the
    /// SIMD and intrinsics modes need multiples of 64 and get those only).
    #[test]
    fn every_i8_mode_matches_the_reference() {
        let dir = tempfile::tempdir().unwrap();
        let modes = i8_modes(dir.path());
        let mut rng = Lcg(7);
        for d in [1usize, 17, 64, 100, 256] {
            let n = 37;
            let codes: Vec<i8> = (0..n).flat_map(|_| quantize(&rng.unit(d))).collect();
            let q = quantize(&rng.unit(d));
            let mut want = vec![0i32; n];
            i8_scalar(&q, &codes, d, &mut want);
            for m in modes.iter().flatten() {
                let wide = matches!(m.family, "intrinsics") || m.name.contains("simd");
                if wide && d % 64 != 0 {
                    continue;
                }
                let mut out = vec![0i32; n];
                (m.run)(&q, &codes, d, &mut out);
                assert_eq!(out, want, "{} at d {d}", m.name);
            }
        }
    }

    /// The order-keeping float modes give `code_dot`'s bits; the lanes
    /// modes give the kernel's `code_dot_lanes` bits at every level.
    #[test]
    fn f32_modes_match_their_order() {
        let dir = tempfile::tempdir().unwrap();
        let modes = f32_modes(dir.path());
        let mut rng = Lcg(9);
        let (n, d) = (29, 256);
        let codes: Vec<i8> = (0..n).flat_map(|_| quantize(&rng.unit(d))).collect();
        let q = rng.unit(d);
        let (mut seq, mut lanes) = (vec![0f32; n], vec![0f32; n]);
        f32_scalar(&q, &codes, d, &mut seq);
        f32_lanes(&q, &codes, d, &mut lanes);
        for m in modes.iter().flatten() {
            let mut out = vec![0f32; n];
            (m.run)(&q, &codes, d, &mut out);
            let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
            match m.family {
                "rust" | "cranelift" => assert_eq!(bits(&out), bits(&seq), "{}", m.name),
                "lanes" | "kernel" => assert_eq!(bits(&out), bits(&lanes), "{}", m.name),
                _ => {}
            }
        }
    }
}
