//! Summary of one fragment (E186): the summary E182 keeps in memory, as a
//! pure function, so it can be recorded as a derivation `summary = f(frag)`
//! and stored beside the fragment. Kernel ABI as in `quote/`: one argument,
//! the fragment's bytes. No floats, no imports.
//!
//! Output (numbers are LEB128 varints):
//!   "S1" money_n money_cents dates headings items nwords { len word count }
//! Words are lowercased runs of ASCII letters and digits of three or more
//! bytes, sorted bytewise. Dollar figures and dates as in E182
//! (`examples/fragment_insights.rs`), saturating the same way.
#![no_std]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};

/// Bump allocator over linear memory: one run per instance, never frees.
struct Bump;

static mut NEXT: usize = 0;

unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe {
            if NEXT == 0 {
                NEXT = core::arch::wasm32::memory_size(0) * 65536;
            }
            let start = (NEXT + layout.align() - 1) & !(layout.align() - 1);
            let end = start + layout.size();
            let have = core::arch::wasm32::memory_size(0) * 65536;
            if end > have {
                let pages = (end - have).div_ceil(65536);
                if core::arch::wasm32::memory_grow(0, pages) == usize::MAX {
                    return core::ptr::null_mut();
                }
            }
            NEXT = end;
            start as *mut u8
        }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}

#[global_allocator]
static ALLOC: Bump = Bump;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: i32) -> i32 {
    let v: Vec<u8> = Vec::with_capacity(len as usize);
    let p = v.as_ptr() as i32;
    core::mem::forget(v);
    p
}


#[unsafe(no_mangle)]
pub extern "C" fn run(ptr: i32, len: i32) -> i64 {
    let input = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let args = parse_args(input);
    if args.len() != 1 {
        trap()
    }
    let out = summary(args[0]);
    let (p, n) = (out.as_ptr() as i64, out.len() as i64);
    core::mem::forget(out);
    (p << 32) | n
}

fn trap() -> ! {
    core::arch::wasm32::unreachable()
}

fn parse_args(mut b: &[u8]) -> Vec<&[u8]> {
    let mut take = |n: usize| {
        if b.len() < n {
            trap()
        }
        let (h, t) = b.split_at(n);
        b = t;
        h
    };
    let u32_of = |s: &[u8]| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize;
    let count = u32_of(take(4));
    let mut args = Vec::new();
    for _ in 0..count {
        let n = u32_of(take(4));
        args.push(take(n));
    }
    args
}

fn put(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn summary(b: &[u8]) -> Vec<u8> {
    let mut words: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
    for w in b.split(|c| !c.is_ascii_alphanumeric()).filter(|w| w.len() >= 3) {
        *words.entry(w.to_ascii_lowercase()).or_default() += 1;
    }
    let (n, cents) = money(b);
    let (mut headings, mut items) = (0u64, 0u64);
    for line in b.split(|&c| c == b'\n') {
        let t = line.trim_ascii_start();
        if t.starts_with(b"#") {
            headings += 1;
        } else if t.starts_with(b"- ")
            || t.starts_with(b"* ")
            || (t.first().is_some_and(u8::is_ascii_digit)
                && t.iter()
                    .find(|c| !c.is_ascii_digit())
                    .is_some_and(|&c| c == b'.' || c == b')'))
        {
            items += 1;
        }
    }
    let mut out = b"S1".to_vec();
    for v in [n as u64, cents as u64, dates(b), headings, items, words.len() as u64] {
        put(&mut out, v);
    }
    for (w, c) in words {
        put(&mut out, w.len() as u64);
        out.extend_from_slice(&w);
        put(&mut out, c as u64);
    }
    out
}

fn money(b: &[u8]) -> (u32, i64) {
    let (mut n, mut total) = (0u32, 0i64);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
            let mut j = i + 1;
            let mut v: i64 = 0;
            while j < b.len()
                && (b[j].is_ascii_digit()
                    || (b[j] == b',' && b.get(j + 1).is_some_and(u8::is_ascii_digit)))
            {
                if b[j] != b',' {
                    v = v.saturating_mul(10).saturating_add((b[j] - b'0') as i64);
                }
                j += 1;
            }
            let mut cents = v.saturating_mul(100);
            if b.get(j) == Some(&b'.') && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
                let d1 = (b[j + 1] - b'0') as i64;
                let d2 = b
                    .get(j + 2)
                    .filter(|c| c.is_ascii_digit())
                    .map_or(0, |c| (c - b'0') as i64);
                cents += d1 * 10 + d2;
                j += 2;
            }
            n += 1;
            total += cents;
            i = j;
        } else {
            i += 1;
        }
    }
    (n, total)
}

fn dates(b: &[u8]) -> u64 {
    b.windows(10)
        .filter(|w| {
            w[..4].iter().all(u8::is_ascii_digit)
                && w[4] == b'-'
                && w[5..7].iter().all(u8::is_ascii_digit)
                && w[7] == b'-'
                && w[8..].iter().all(u8::is_ascii_digit)
        })
        .count() as u64
}
