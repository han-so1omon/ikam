//! Pinned float arithmetic: the rules E409 measured as giving the same bits
//! on every platform it tried (x86-64 at three instruction-set levels,
//! ARM64, macOS, Windows):
//!
//! - IEEE 754 basic operations and square root, which are correctly
//!   rounded everywhere;
//! - no fused multiply-add (Rust never contracts `a * b + c` on its own);
//! - reductions in index order, one running sum, never reordered lanes;
//! - transcendental functions from `libm`, pure Rust compiled into the
//!   program (its only hardware paths are sqrt and fma, both correctly
//!   rounded), never the host's C library;
//! - one canonical NaN, since NaN payloads differ between x86 and ARM.
//!
//! Every function here returns a canonical value, so a result's bits, and
//! the content id of a tensor of results, depend only on the inputs.

pub const CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;

#[inline]
pub fn canon(x: f64) -> f64 {
    if x.is_nan() { f64::from_bits(CANONICAL_NAN) } else { x }
}

macro_rules! unary {
    ($($name:ident => $f:expr),* $(,)?) => {
        $(#[inline] pub fn $name(x: f64) -> f64 { canon($f(x)) })*
    };
}

unary! {
    exp => libm::exp,
    ln => libm::log,
    sin => libm::sin,
    cos => libm::cos,
    tanh => libm::tanh,
    sqrt => libm::sqrt,
}

#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    canon(libm::atan2(y, x))
}

/// Sum in index order.
pub fn sum(xs: &[f64]) -> f64 {
    let mut s = 0.0;
    for x in xs {
        s += x;
    }
    canon(s)
}
