//! Quotes: rebuild text that quotes other content line by line, as agent
//! transcripts and logs do (E056). Run by the kernel as a WASM function.
//!
//! Kernel ABI (`wasm.rs`): `alloc(len) -> ptr`, `run(ptr, len) -> out_ptr << 32
//! | out_len`; input `count:u32 { len:u32 bytes }`. Argument 0 is the op
//! stream, argument 1 the sources' names (one per line), arguments 2.. the
//! sources. No floats, no imports; malformed input traps (no output).
//!
//! Ops (numbers are LEB128 varints; `s` is a source, `a` its first line,
//! `n` the line count; every quoted line is JSON-escaped and ends in `\n`):
//!   00 len bytes   literal bytes
//!   01 s a n       each line as `{number:>6}\t{line}`
//!   02 s a n       each line as `{name}:{number}:{line}`
#![no_std]

extern crate alloc;

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
    if args.len() < 2 {
        trap()
    }
    let names: Vec<&[u8]> = lines(args[1]);
    let sources: Vec<Vec<&[u8]>> = args[2..].iter().map(|s| lines(s)).collect();
    let out = execute(args[0], &names, &sources);
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

/// Lines as Rust's `str::lines` splits them: on `\n`, a final `\r` dropped,
/// no empty last line.
fn lines(text: &[u8]) -> Vec<&[u8]> {
    let mut out: Vec<&[u8]> = text.split(|&b| b == b'\n').collect();
    if out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.into_iter()
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .collect()
}

fn varint(ops: &[u8], at: &mut usize) -> usize {
    let (mut n, mut shift) = (0usize, 0);
    loop {
        let Some(&b) = ops.get(*at) else { trap() };
        *at += 1;
        n |= ((b & 0x7f) as usize) << shift;
        if b < 0x80 {
            return n;
        }
        shift += 7;
        if shift > 28 {
            trap()
        }
    }
}

fn escaped(out: &mut Vec<u8>, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in bytes {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0..0x20 => out.extend_from_slice(&[b'\\', b'u', b'0', b'0', HEX[(b >> 4) as usize], HEX[(b & 15) as usize]]),
            _ => out.push(b),
        }
    }
}

fn decimal(out: &mut Vec<u8>, mut n: usize, width: usize) {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let len = digits.len() - i;
    out.extend(core::iter::repeat_n(b' ', width.saturating_sub(len)));
    out.extend_from_slice(&digits[i..]);
}

fn execute(ops: &[u8], names: &[&[u8]], sources: &[Vec<&[u8]>]) -> Vec<u8> {
    let (mut out, mut at) = (Vec::new(), 0);
    while at < ops.len() {
        let op = ops[at];
        at += 1;
        if op == 0 {
            let n = varint(ops, &mut at);
            let Some(bytes) = ops.get(at..at + n) else { trap() };
            out.extend_from_slice(bytes);
            at += n;
            continue;
        }
        let (s, a, n) = (varint(ops, &mut at), varint(ops, &mut at), varint(ops, &mut at));
        let (Some(src), Some(name)) = (sources.get(s), names.get(s)) else { trap() };
        let Some(quoted) = src.get(a..a + n) else { trap() };
        for (i, line) in quoted.iter().enumerate() {
            match op {
                1 => {
                    decimal(&mut out, a + i + 1, 6);
                    out.extend_from_slice(b"\\t");
                }
                2 => {
                    escaped(&mut out, name);
                    out.push(b':');
                    decimal(&mut out, a + i + 1, 0);
                    out.push(b':');
                }
                _ => trap(),
            }
            escaped(&mut out, line);
            out.extend_from_slice(b"\\n");
        }
    }
    out
}
