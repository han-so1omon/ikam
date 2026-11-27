//! Float functions as stored WASM modules (E441), from E409's measurement:
//! a dot product, a mean and variance, a softmax with an `exp` shipped
//! inside the module, that `exp` alone, and a square matrix multiply. Each
//! module follows the kernel's ABI (see `wasm.rs`): its arguments are
//! little-endian f64 arrays, its output one too. Shared by the tests and
//! the E441 example.
#![allow(dead_code)]

/// The functions, by name.
pub const FUNCS: [&str; 5] = ["dot", "mean_var", "softmax", "exp", "matmul"];

const INV_LN2: f64 = std::f64::consts::LOG2_E;
const LN2_HI: u64 = 0x3fe6_2e42_fee0_0000;
const LN2_LO: u64 = 0x3dea_39ef_3579_3c76;
const EXP_MAX: f64 = 709.782712893384;
const EXP_MIN: f64 = -745.1332191019412;

/// Taylor coefficients 1/k! for the shipped `exp`.
pub fn coeffs() -> [f64; 14] {
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

/// The shipped `exp` in Rust, operation for operation as in the module
/// (with `fused`, Horner's steps use fused multiply-add instead).
pub fn shipped_exp(x: f64, fused: bool) -> f64 {
    let c = coeffs();
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

/// How a native run computes, to stand in for other builds and platforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arith {
    /// One operation at a time, sums in index order, the shipped `exp`:
    /// the module's own arithmetic.
    Pinned,
    /// Fused multiply-add in products and in `exp`, sums over eight lanes,
    /// and the host's `exp`.
    Host,
}

fn sum(v: impl Iterator<Item = f64>, a: Arith) -> f64 {
    match a {
        Arith::Pinned => v.fold(0.0, |s, x| s + x),
        Arith::Host => {
            let mut l = [0.0f64; 8];
            for (i, x) in v.enumerate() {
                l[i % 8] += x;
            }
            ((l[0] + l[1]) + (l[2] + l[3])) + ((l[4] + l[5]) + (l[6] + l[7]))
        }
    }
}

fn dotp(a: &[f64], b: &[f64], m: Arith) -> f64 {
    match m {
        Arith::Pinned => sum(a.iter().zip(b).map(|(x, y)| x * y), m),
        Arith::Host => a.iter().zip(b).fold(0.0, |s, (x, y)| x.mul_add(*y, s)),
    }
}

fn exp(x: f64, m: Arith) -> f64 {
    match m {
        Arith::Pinned => shipped_exp(x, false),
        Arith::Host => x.exp(),
    }
}

/// Native evaluation of `func` (one of `FUNCS`).
pub fn native(func: &str, a: &[f64], b: &[f64], m: Arith) -> Vec<f64> {
    match func {
        "dot" => vec![dotp(a, b, m)],
        "mean_var" => {
            let n = a.len() as f64;
            let mean = sum(a.iter().copied(), m) / n;
            let var = sum(a.iter().map(|x| (x - mean) * (x - mean)), m) / n;
            vec![mean, var]
        }
        "softmax" => {
            let mx = a[1..].iter().fold(a[0], |m, &x| if x > m { x } else { m });
            let e: Vec<f64> = a.iter().map(|x| exp(x - mx, m)).collect();
            let s = sum(e.iter().copied(), m);
            e.iter().map(|x| x / s).collect()
        }
        "exp" => a.iter().map(|x| exp(*x, m)).collect(),
        _ => {
            let d = (a.len() as f64).sqrt() as usize;
            let mut o = vec![0.0; d * d];
            for i in 0..d {
                for j in 0..d {
                    let row = &a[i * d..i * d + d];
                    let col: Vec<f64> = (0..d).map(|k| b[k * d + j]).collect();
                    o[i * d + j] = dotp(row, &col, m);
                }
            }
            o
        }
    }
}

pub fn f64s(b: &[u8]) -> Vec<f64> {
    b.chunks_exact(8)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

pub fn bytes(v: &[f64]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// The module for `func`, in the kernel's ABI.
pub fn wat(func: &str) -> String {
    let c = coeffs();
    let mut horner = format!("(local.set $p (f64.const {:?}))\n", c[13]);
    for ci in c[..13].iter().rev() {
        horner.push_str(&format!(
            "(local.set $p (f64.add (f64.mul (local.get $p) (local.get $r)) (f64.const {ci:?})))\n"
        ));
    }
    let at = |p: &str, i: &str| {
        format!("(i32.add (local.get {p}) (i32.shl (local.get {i}) (i32.const 3)))")
    };
    let (ai, bi, oi) = (at("$a", "$i"), at("$b", "$i"), at("$o", "$i"));
    // Output length in bytes from the input lengths (na, nb in values).
    let (out_len, call) = match func {
        "dot" => (
            "(i32.const 8)",
            "(f64.store (local.get $o) (call $dot (local.get $a) (local.get $b) (local.get $na)))",
        ),
        "mean_var" => (
            "(i32.const 16)",
            "(call $mean_var (local.get $a) (local.get $na) (local.get $o))",
        ),
        "softmax" => (
            "(i32.shl (local.get $na) (i32.const 3))",
            "(call $softmax (local.get $a) (local.get $na) (local.get $o))",
        ),
        "exp" => (
            "(i32.shl (local.get $na) (i32.const 3))",
            "(call $exps (local.get $a) (local.get $na) (local.get $o))",
        ),
        _ => (
            "(i32.shl (local.get $na) (i32.const 3))",
            "(call $matmul (local.get $a) (local.get $b) (local.get $o) (i32.trunc_f64_u (f64.sqrt (f64.convert_i32_u (local.get $na)))))",
        ),
    };
    format!(
        r#"(module
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 1024))
  (func $alloc (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (if (i32.gt_u (i32.add (local.get $p) (local.get $n)) (i32.mul (memory.size) (i32.const 65536)))
      (then (drop (memory.grow (i32.add (i32.shr_u (local.get $n) (i32.const 16)) (i32.const 1))))))
    (global.set $heap (i32.add (local.get $p) (local.get $n)))
    (local.get $p))
  (func $be32 (param $p i32) (result i32)
    (i32.or (i32.or
      (i32.shl (i32.load8_u (local.get $p)) (i32.const 24))
      (i32.shl (i32.load8_u (i32.add (local.get $p) (i32.const 1))) (i32.const 16)))
      (i32.or
        (i32.shl (i32.load8_u (i32.add (local.get $p) (i32.const 2))) (i32.const 8))
        (i32.load8_u (i32.add (local.get $p) (i32.const 3))))))
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
  (func $dot (param $a i32) (param $b i32) (param $n i32) (result f64)
    (local $i i32) (local $s f64)
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (local.set $s (f64.add (local.get $s) (f64.mul (f64.load {ai}) (f64.load {bi}))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (local.get $s))
  (func $mean_var (param $a i32) (param $n i32) (param $o i32)
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
  (func $softmax (param $a i32) (param $n i32) (param $o i32)
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
  (func $exps (param $a i32) (param $n i32) (param $o i32)
    (local $i i32)
    (block $done (loop $l
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (f64.store {oi} (call $exp (f64.load {ai})))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l))))
  (func $matmul (param $a i32) (param $b i32) (param $o i32) (param $dim i32)
    (local $i i32) (local $j i32) (local $k i32) (local $s f64)
    (block $di (loop $li
      (br_if $di (i32.ge_u (local.get $i) (local.get $dim)))
      (local.set $j (i32.const 0))
      (block $dj (loop $lj
        (br_if $dj (i32.ge_u (local.get $j) (local.get $dim)))
        (local.set $s (f64.const 0))
        (local.set $k (i32.const 0))
        (block $dk (loop $lk
          (br_if $dk (i32.ge_u (local.get $k) (local.get $dim)))
          (local.set $s (f64.add (local.get $s) (f64.mul
            (f64.load (i32.add (local.get $a) (i32.shl
              (i32.add (i32.mul (local.get $i) (local.get $dim)) (local.get $k)) (i32.const 3))))
            (f64.load (i32.add (local.get $b) (i32.shl
              (i32.add (i32.mul (local.get $k) (local.get $dim)) (local.get $j)) (i32.const 3)))))))
          (local.set $k (i32.add (local.get $k) (i32.const 1)))
          (br $lk)))
        (f64.store (i32.add (local.get $o) (i32.shl
          (i32.add (i32.mul (local.get $i) (local.get $dim)) (local.get $j)) (i32.const 3)))
          (local.get $s))
        (local.set $j (i32.add (local.get $j) (i32.const 1)))
        (br $lj)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $li))))
  (func (export "run") (param $p i32) (param $len i32) (result i64)
    (local $a i32) (local $b i32) (local $la i32) (local $na i32) (local $o i32) (local $out i32)
    (local.set $la (call $be32 (i32.add (local.get $p) (i32.const 4))))
    (local.set $a (i32.add (local.get $p) (i32.const 8)))
    (local.set $na (i32.shr_u (local.get $la) (i32.const 3)))
    (if (i32.ge_u (call $be32 (local.get $p)) (i32.const 2))
      (then (local.set $b (i32.add (i32.add (local.get $a) (local.get $la)) (i32.const 4)))))
    (local.set $out {out_len})
    (local.set $o (call $alloc (local.get $out)))
    {call}
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $o)) (i64.const 32))
      (i64.extend_i32_u (local.get $out))))
)"#,
        max = EXP_MAX,
        min = EXP_MIN,
        inv = INV_LN2,
        hi = f64::from_bits(LN2_HI),
        lo = f64::from_bits(LN2_LO),
    )
}

struct Lcg(u64);
impl Lcg {
    fn unit(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 52) as f64) - 1.0
    }
}

/// E409's inputs by name: `plain`, `cancel`, `subnormal`, `wide`, `edges`
/// and `special` (NaN, infinities and other edges), `n` values each.
pub const INPUTS: [&str; 6] = ["plain", "cancel", "subnormal", "wide", "edges", "special"];

pub fn inputs(name: &str, n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut r = Lcg(409);
    let mut a: Vec<f64> = (0..n).map(|_| 10.0 * r.unit()).collect();
    let b: Vec<f64> = (0..n).map(|_| 10.0 * r.unit()).collect();
    match name {
        "plain" => {}
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
        "wide" => a.iter_mut().for_each(|x| *x *= 80.0),
        "edges" => {
            let k = n.min(64);
            for (i, x) in a.iter_mut().take(k).enumerate() {
                *x = match i {
                    0..40 => -745.2 + 36.9 * i as f64 / 39.0,
                    40..48 => 709.5 + 0.5 * (i - 40) as f64 / 7.0,
                    _ => *x * 1e-3,
                };
            }
        }
        _ => {
            for (i, v) in [
                f64::from_bits(0x7ff8_0000_0000_1234),
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::from_bits(1),
                -0.0,
                1.7e308,
                -1.7e308,
            ]
            .into_iter()
            .enumerate()
            {
                if 3 + 10 * i < n {
                    a[3 + 10 * i] = v;
                }
            }
        }
    }
    (a, b)
}
