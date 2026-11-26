//! E409: what makes a float step give the same bits everywhere, and what
//! does it cost?
//!
//! Four float functions (a dot product, a mean and variance, a softmax
//! with exp, an 8x8 matrix multiply) run in several modes on inputs that
//! make NaN, infinities, subnormals and large cancellations:
//!
//! - `scalar`: native Rust, one sum in index order, no fused multiply-add,
//!   `exp` shipped with the program (below), not taken from the host.
//! - `fma`: the same with every multiply-add fused (`mul_add`), as a
//!   compiler allowed to contract would build it.
//! - `lanes8`: sums kept in 8 lanes and added pairwise at the end, as a
//!   vectorized build reorders them (E327's lanes).
//! - `std-exp` and `libm-exp`: the softmax with the host's `exp` (glibc on
//!   Linux, the system's elsewhere) or the pure-Rust `libm` crate's.
//! - `wasm`: the same functions written in WebAssembly, `exp` inside the
//!   module, run by wasmi 2.0.0 as the kernel runs stored functions but
//!   with floats on. With the `canonical` feature (the default, the
//!   kernel's `deterministic` feature) every float result that is NaN is
//!   the one canonical NaN; without it NaN bits are what the host makes.
//!
//! Each record is one JSON line: the platform, the function, the input,
//! the mode, an FNV-1a hash of the output's bits (equal hashes mean equal
//! bits, across machines), the NaN bit patterns in the output, the largest
//! difference in units in the last place from `scalar` on this machine,
//! and the time per call.
//!
//! `cargo run --release -- [LABEL]`

use wasmi::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

const N: usize = 1024;
const SOFT: usize = 64;
const DIM: usize = 8;

/// `exp` shipped with the program: Cody-Waite reduction by ln 2 (fdlibm's
/// split), then a degree-13 Taylor polynomial by Horner's rule, then a
/// scale by 2^k in two factors so subnormal results are reached. Every
/// operation is one IEEE 754 operation in a fixed order, so any machine
/// that rounds to nearest gives the same bits; the WebAssembly copy is
/// generated from these same constants.
const INV_LN2: f64 = std::f64::consts::LOG2_E;
const LN2_HI: u64 = 0x3fe6_2e42_fee0_0000;
const LN2_LO: u64 = 0x3dea_39ef_3579_3c76;
const EXP_MAX: f64 = 709.782712893384;
const EXP_MIN: f64 = -745.1332191019412;

fn coeffs() -> [f64; 14] {
    let mut c = [0.0; 14];
    let mut fact = 1.0f64;
    for (i, ci) in c.iter_mut().enumerate() {
        if i > 0 {
            fact *= i as f64;
        }
        *ci = 1.0 / fact;
    }
    c
}

fn shipped_exp(x: f64, c: &[f64; 14], fused: bool) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > EXP_MAX {
        return f64::INFINITY;
    }
    if x < EXP_MIN {
        return 0.0;
    }
    let k = (x * INV_LN2).round_ties_even();
    let (hi, lo) = (f64::from_bits(LN2_HI), f64::from_bits(LN2_LO));
    let r = (x - k * hi) - k * lo;
    let mut p = c[13];
    for &ci in c[..13].iter().rev() {
        p = if fused { p.mul_add(r, ci) } else { p * r + ci };
    }
    let ki = k as i64;
    let k1 = ki / 2;
    let s1 = f64::from_bits(((k1 + 1023) as u64) << 52);
    let s2 = f64::from_bits(((ki - k1 + 1023) as u64) << 52);
    p * s1 * s2
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Scalar,
    Fma,
    Lanes8,
    StdExp,
    LibmExp,
}

fn add_seq(v: impl Iterator<Item = f64>) -> f64 {
    let mut s = 0.0;
    for x in v {
        s += x;
    }
    s
}

fn add_lanes(v: impl Iterator<Item = f64>) -> f64 {
    let mut l = [0.0f64; 8];
    for (i, x) in v.enumerate() {
        l[i % 8] += x;
    }
    ((l[0] + l[1]) + (l[2] + l[3])) + ((l[4] + l[5]) + (l[6] + l[7]))
}

fn dot(a: &[f64], b: &[f64], m: Mode) -> Vec<f64> {
    vec![match m {
        Mode::Fma => {
            let mut s = 0.0f64;
            for i in 0..a.len() {
                s = a[i].mul_add(b[i], s);
            }
            s
        }
        Mode::Lanes8 => add_lanes(a.iter().zip(b).map(|(x, y)| x * y)),
        _ => add_seq(a.iter().zip(b).map(|(x, y)| x * y)),
    }]
}

fn mean_var(a: &[f64], m: Mode) -> Vec<f64> {
    let n = a.len() as f64;
    let sum = match m {
        Mode::Lanes8 => add_lanes(a.iter().copied()),
        _ => add_seq(a.iter().copied()),
    };
    let mean = sum / n;
    let acc = match m {
        Mode::Fma => {
            let mut s = 0.0f64;
            for &x in a {
                let d = x - mean;
                s = d.mul_add(d, s);
            }
            s
        }
        Mode::Lanes8 => add_lanes(a.iter().map(|x| (x - mean) * (x - mean))),
        _ => add_seq(a.iter().map(|x| (x - mean) * (x - mean))),
    };
    vec![mean, acc / n]
}

fn softmax(a: &[f64], m: Mode, c: &[f64; 14]) -> Vec<f64> {
    // The largest by comparison: a NaN is never larger, so it is passed
    // over unless it comes first (the same rule in the WebAssembly copy).
    let mut mx = a[0];
    for &x in &a[1..] {
        if x > mx {
            mx = x;
        }
    }
    let e: Vec<f64> = a
        .iter()
        .map(|x| {
            let d = x - mx;
            match m {
                Mode::StdExp => d.exp(),
                Mode::LibmExp => libm::exp(d),
                Mode::Fma => shipped_exp(d, c, true),
                _ => shipped_exp(d, c, false),
            }
        })
        .collect();
    let s = match m {
        Mode::Lanes8 => add_lanes(e.iter().copied()),
        _ => add_seq(e.iter().copied()),
    };
    e.iter().map(|x| x / s).collect()
}

fn matmul(a: &[f64], b: &[f64], m: Mode) -> Vec<f64> {
    let mut c = vec![0.0; DIM * DIM];
    for i in 0..DIM {
        for j in 0..DIM {
            let terms = (0..DIM).map(|k| a[i * DIM + k] * b[k * DIM + j]);
            c[i * DIM + j] = match m {
                Mode::Fma => {
                    let mut s = 0.0f64;
                    for k in 0..DIM {
                        s = a[i * DIM + k].mul_add(b[k * DIM + j], s);
                    }
                    s
                }
                Mode::Lanes8 => add_lanes(terms),
                _ => add_seq(terms),
            };
        }
    }
    c
}

/// The WebAssembly copy, its `exp` constants printed from the same values
/// (`{:?}` prints the shortest decimal that reads back as the same bits).
fn wat(c: &[f64; 14]) -> String {
    let mut horner = format!("(local.set $p (f64.const {:?}))\n", c[13]);
    for ci in c[..13].iter().rev() {
        horner.push_str(&format!(
            "(local.set $p (f64.add (f64.mul (local.get $p) (local.get $r)) (f64.const {ci:?})))\n"
        ));
    }
    let at = |p: &str, i: &str| {
        format!("(i32.add (local.get {p}) (i32.shl (local.get {i}) (i32.const 3)))")
    };
    format!(
        r#"(module
  (memory (export "mem") 1)
  (func $exp (param $x f64) (result f64)
    (local $k f64) (local $r f64) (local $p f64) (local $ki i64) (local $k1 i64)
    (if (f64.ne (local.get $x) (local.get $x)) (then (return (local.get $x))))
    (if (f64.gt (local.get $x) (f64.const {max:?})) (then (return (f64.const inf))))
    (if (f64.lt (local.get $x) (f64.const {min:?})) (then (return (f64.const 0))))
    (local.set $k (f64.nearest (f64.mul (local.get $x) (f64.const {inv:?}))))
    (local.set $r (f64.sub
      (f64.sub (local.get $x) (f64.mul (local.get $k) (f64.const {hi:?})))
      (f64.mul (local.get $k) (f64.const {lo:?}))))
    {horner}
    (local.set $ki (i64.trunc_f64_s (local.get $k)))
    (local.set $k1 (i64.div_s (local.get $ki) (i64.const 2)))
    (f64.mul
      (f64.mul (local.get $p)
        (f64.reinterpret_i64 (i64.shl (i64.add (local.get $k1) (i64.const 1023)) (i64.const 52))))
      (f64.reinterpret_i64 (i64.shl
        (i64.add (i64.sub (local.get $ki) (local.get $k1)) (i64.const 1023)) (i64.const 52)))))
  (func (export "dot") (param $a i32) (param $b i32) (param $n i32) (result f64)
    (local $i i32) (local $s f64)
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (local.set $s (f64.add (local.get $s)
        (f64.mul (f64.load {ai}) (f64.load {bi}))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (local.get $s))
  (func (export "mean_var") (param $a i32) (param $n i32) (param $o i32)
    (local $i i32) (local $s f64) (local $mean f64) (local $d f64)
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (local.set $s (f64.add (local.get $s) (f64.load {ai})))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (local.set $mean (f64.div (local.get $s) (f64.convert_i32_u (local.get $n))))
    (local.set $s (f64.const 0))
    (local.set $i (i32.const 0))
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (local.set $d (f64.sub (f64.load {ai}) (local.get $mean)))
      (local.set $s (f64.add (local.get $s) (f64.mul (local.get $d) (local.get $d))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (f64.store (local.get $o) (local.get $mean))
    (f64.store (i32.add (local.get $o) (i32.const 8))
      (f64.div (local.get $s) (f64.convert_i32_u (local.get $n)))))
  (func (export "softmax") (param $a i32) (param $n i32) (param $o i32)
    (local $i i32) (local $m f64) (local $x f64) (local $s f64)
    (local.set $m (f64.load (local.get $a)))
    (local.set $i (i32.const 1))
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (local.set $x (f64.load {ai}))
      (if (f64.gt (local.get $x) (local.get $m)) (then (local.set $m (local.get $x))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (local.set $i (i32.const 0))
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (local.set $x (call $exp (f64.sub (f64.load {ai}) (local.get $m))))
      (f64.store {oi} (local.get $x))
      (local.set $s (f64.add (local.get $s) (local.get $x)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (local.set $i (i32.const 0))
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (f64.store {oi} (f64.div (f64.load {oi}) (local.get $s)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l))))
  (func (export "matmul") (param $a i32) (param $b i32) (param $o i32)
    (local $i i32) (local $j i32) (local $k i32) (local $s f64)
    (block $di (loop $li
      (br_if $di (i32.ge_u (local.get $i) (i32.const {dim})))
      (local.set $j (i32.const 0))
      (block $dj (loop $lj
        (br_if $dj (i32.ge_u (local.get $j) (i32.const {dim})))
        (local.set $s (f64.const 0))
        (local.set $k (i32.const 0))
        (block $dk (loop $lk
          (br_if $dk (i32.ge_u (local.get $k) (i32.const {dim})))
          (local.set $s (f64.add (local.get $s) (f64.mul
            (f64.load (i32.add (local.get $a) (i32.shl
              (i32.add (i32.mul (local.get $i) (i32.const {dim})) (local.get $k)) (i32.const 3))))
            (f64.load (i32.add (local.get $b) (i32.shl
              (i32.add (i32.mul (local.get $k) (i32.const {dim})) (local.get $j)) (i32.const 3)))))))
          (local.set $k (i32.add (local.get $k) (i32.const 1)))
          (br $lk)))
        (f64.store (i32.add (local.get $o) (i32.shl
          (i32.add (i32.mul (local.get $i) (i32.const {dim})) (local.get $j)) (i32.const 3)))
          (local.get $s))
        (local.set $j (i32.add (local.get $j) (i32.const 1)))
        (br $lj)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $li))))
)"#,
        max = EXP_MAX,
        min = EXP_MIN,
        inv = INV_LN2,
        hi = f64::from_bits(LN2_HI),
        lo = f64::from_bits(LN2_LO),
        ai = at("$a", "$i"),
        bi = at("$b", "$i"),
        oi = at("$o", "$i"),
        dim = DIM,
    )
}

const PA: usize = 0;
const PB: usize = 8 * N;
const PO: usize = 16 * N;

struct Wasm {
    store: Store<()>,
    mem: Memory,
    dot: TypedFunc<(i32, i32, i32), f64>,
    mean_var: TypedFunc<(i32, i32, i32), ()>,
    softmax: TypedFunc<(i32, i32, i32), ()>,
    matmul: TypedFunc<(i32, i32, i32), ()>,
}

impl Wasm {
    fn new(c: &[f64; 14]) -> Wasm {
        let engine = Engine::default();
        let bytes = wat::parse_str(wat(c)).expect("wat");
        let module = Module::new(&engine, &bytes[..]).expect("module");
        let mut store = Store::new(&engine, ());
        let inst: Instance = Linker::new(&engine)
            .instantiate_and_start(&mut store, &module)
            .expect("instance");
        Wasm {
            mem: inst.get_memory(&store, "mem").unwrap(),
            dot: inst.get_typed_func(&store, "dot").unwrap(),
            mean_var: inst.get_typed_func(&store, "mean_var").unwrap(),
            softmax: inst.get_typed_func(&store, "softmax").unwrap(),
            matmul: inst.get_typed_func(&store, "matmul").unwrap(),
            store,
        }
    }
    fn put(&mut self, at: usize, v: &[f64]) {
        let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.mem.write(&mut self.store, at, &b).unwrap();
    }
    fn get(&self, at: usize, n: usize) -> Vec<f64> {
        let mut b = vec![0u8; 8 * n];
        self.mem.read(&self.store, at, &mut b).unwrap();
        b.chunks(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
            .collect()
    }
    fn run(&mut self, func: &str, a: &[f64], b: &[f64]) -> Vec<f64> {
        self.put(PA, a);
        self.put(PB, b);
        let (pa, pb, po) = (PA as i32, PB as i32, PO as i32);
        match func {
            "dot" => vec![self.dot.call(&mut self.store, (pa, pb, N as i32)).unwrap()],
            "mean_var" => {
                self.mean_var
                    .call(&mut self.store, (pa, N as i32, po))
                    .unwrap();
                self.get(PO, 2)
            }
            "softmax" => {
                self.softmax
                    .call(&mut self.store, (pa, SOFT as i32, po))
                    .unwrap();
                self.get(PO, SOFT)
            }
            _ => {
                self.matmul.call(&mut self.store, (pa, pb, po)).unwrap();
                self.get(PO, DIM * DIM)
            }
        }
    }
}

struct Lcg(u64);
impl Lcg {
    /// Uniform in [-1, 1), from integers only, so every machine draws the
    /// same inputs.
    fn unit(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 52) as f64) - 1.0
    }
}

fn inputs(name: &str) -> (Vec<f64>, Vec<f64>) {
    let mut r = Lcg(409);
    let mut a: Vec<f64> = (0..N).map(|_| 10.0 * r.unit()).collect();
    let b: Vec<f64> = (0..N).map(|_| 10.0 * r.unit()).collect();
    match name {
        "plain" => {}
        // Huge terms that cancel, with small ones between: the order of
        // the sum decides what survives.
        "cancel" => {
            for (i, x) in a.iter_mut().enumerate() {
                *x = match i % 4 {
                    0 => 1e16 * (1.0 + *x * 1e-4),
                    2 => -1e16 * (1.0 + *x * 1e-4),
                    _ => *x,
                };
            }
        }
        "subnormal" => a.iter_mut().for_each(|x| *x *= 1e-310),
        // Wide enough that exp overflows, underflows and lands on
        // subnormals.
        "wide" => a.iter_mut().for_each(|x| *x *= 80.0),
        _ => {
            a[3] = f64::from_bits(0x7ff8_0000_0000_1234); // a quiet NaN with a payload
            a[10] = f64::INFINITY;
            a[20] = f64::NEG_INFINITY;
            a[30] = f64::from_bits(1);
            a[40] = -0.0;
            a[50] = 1.7e308;
            a[60] = -1.7e308;
        }
    }
    (a, b)
}

fn fnv(v: &[f64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in v {
        for b in x.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// Units in the last place between two floats; NaN against NaN is 0
/// (their bits are reported apart), NaN against a number is `u64::MAX`.
fn ulps(a: f64, b: f64) -> u64 {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => 0,
        (true, false) | (false, true) => u64::MAX,
        _ => {
            let key = |x: f64| {
                let b = x.to_bits();
                if b >> 63 == 1 { !b } else { b | 1 << 63 }
            };
            key(a).abs_diff(key(b))
        }
    }
}

fn time(mut f: impl FnMut() -> Vec<f64>) -> f64 {
    let mut calls = 0u32;
    let t = std::time::Instant::now();
    while t.elapsed().as_millis() < 40 || calls < 20 {
        std::hint::black_box(f());
        calls += 1;
    }
    t.elapsed().as_secs_f64() * 1e9 / calls as f64
}

fn main() {
    let label = std::env::args().nth(1).unwrap_or_default();
    let canonical = cfg!(feature = "canonical");
    let platform = format!(
        "{}-{}{}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        if label.is_empty() {
            String::new()
        } else {
            format!("-{label}")
        }
    );
    let c = coeffs();
    let mut wasm = Wasm::new(&c);
    let wasm_mode = if canonical { "wasm" } else { "wasm-raw" };
    for input in ["plain", "cancel", "subnormal", "wide", "special"] {
        let (a, b) = inputs(input);
        for func in ["dot", "mean_var", "softmax", "matmul"] {
            let native = |m: Mode| match func {
                "dot" => dot(&a, &b, m),
                "mean_var" => mean_var(&a, m),
                "softmax" => softmax(&a[..SOFT], m, &c),
                _ => matmul(&a[..DIM * DIM], &b[..DIM * DIM], m),
            };
            let base = native(Mode::Scalar);
            let mut modes: Vec<(&str, Option<Mode>)> = vec![
                ("scalar", Some(Mode::Scalar)),
                ("fma", Some(Mode::Fma)),
                ("lanes8", Some(Mode::Lanes8)),
            ];
            if func == "softmax" {
                modes.push(("std-exp", Some(Mode::StdExp)));
                modes.push(("libm-exp", Some(Mode::LibmExp)));
            }
            modes.push((wasm_mode, None));
            for (name, m) in modes {
                let (out, ns) = match m {
                    Some(m) => (native(m), time(|| native(m))),
                    None => {
                        let out = wasm.run(func, &a, &b);
                        (out, time(|| wasm.run(func, &a, &b)))
                    }
                };
                let mut nan: Vec<String> = out
                    .iter()
                    .filter(|x| x.is_nan())
                    .map(|x| format!("{:016x}", x.to_bits()))
                    .collect();
                nan.sort();
                nan.dedup();
                let ulp = out
                    .iter()
                    .zip(&base)
                    .map(|(x, y)| ulps(*x, *y))
                    .max()
                    .unwrap_or(0);
                let ulp = if ulp == u64::MAX {
                    "-1".to_string()
                } else {
                    ulp.to_string()
                };
                println!(
                    "{{\"platform\":\"{platform}\",\"func\":\"{func}\",\"input\":\"{input}\",\"mode\":\"{name}\",\"bits\":\"{:016x}\",\"nan\":{nan:?},\"ulp_vs_scalar\":{ulp},\"ns\":{ns:.0}}}",
                    fnv(&out)
                );
            }
        }
    }
}
