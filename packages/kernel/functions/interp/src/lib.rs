//! A deterministic integer stack machine, compiled to WASM and stored in the
//! kernel as content: one shared reconstruction function whose programs are
//! short arguments (`docs/plans/2026-10-06-system-architecture.md`).
//!
//! Kernel ABI (`wasm.rs`): `alloc(len) -> ptr`, `run(ptr, len) -> out_ptr << 32
//! | out_len`; input `count:u32 { len:u32 bytes }`. Argument 0 is the program;
//! further arguments are data it may read. No floats, no imports; the
//! kernel's fuel bounds execution. Any malformed program traps (no output).
//!
//! Program: a byte string of ops. Jump targets are absolute u16 offsets.
//!   00 HALT
//!   01 PUSH zigzag-varint
//!   02 ADD 03 SUB 04 MUL 05 DIV 06 MOD   (wrapping; DIV/MOD by 0 trap)
//!   07 AND 08 OR 09 XOR 0A SHL 0B SAR 0F SHR (logical)
//!   0C LT 0D EQ 0E NOT                   (results 0 or 1)
//!   10 DUP 11 SWAP 12 DROP 13 OVER
//!   14 GET r:u8 15 SET r:u8              (256 registers, start at 0)
//!   16 LOAD 17 STORE                     (scratch array: LOAD i; STORE v i)
//!   18 JMP t:u16 19 JZ t:u16 1A JNZ t:u16
//!   20 EMIT_BYTE 21 EMIT_DEC
//!   22 EMIT_PAD width:u8 fill:u8         (decimal, right-aligned)
//!   23 EMIT_STR len:varint bytes
//!   24 INPUT_LEN a:u8 25 INPUT_BYTE a:u8 (pops index)
//!   27 EMIT_SEL n:u8 { len:varint bytes } (pops i, emits string i)
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
    let out = execute(args[0], &args[1..]);
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
    if args.is_empty() {
        trap()
    }
    args
}

struct Code<'a> {
    b: &'a [u8],
    pc: usize,
}

impl Code<'_> {
    fn byte(&mut self) -> u8 {
        let v = *self.b.get(self.pc).unwrap_or_else(|| trap());
        self.pc += 1;
        v
    }
    fn u16(&mut self) -> usize {
        (self.byte() as usize) << 8 | self.byte() as usize
    }
    fn varint(&mut self) -> u64 {
        let mut n = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.byte();
            n |= ((b & 0x7f) as u64) << shift;
            if b < 0x80 {
                return n;
            }
        }
        trap()
    }
    fn bytes(&mut self) -> &[u8] {
        let n = self.varint() as usize;
        let s = self.b.get(self.pc..self.pc + n).unwrap_or_else(|| trap());
        self.pc += n;
        s
    }
}

fn emit_dec(out: &mut Vec<u8>, v: i64, width: usize, fill: u8) {
    let (mut digits, mut n) = ([0u8; 20], v.unsigned_abs());
    let mut i = 20;
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let len = 20 - i + (v < 0) as usize;
    out.extend(core::iter::repeat_n(fill, width.saturating_sub(len)));
    if v < 0 {
        out.push(b'-');
    }
    out.extend_from_slice(&digits[i..]);
}

/// Operand stack: a fixed array (no allocation in the hot loop).
struct Stack {
    v: [i64; 1024],
    n: usize,
}

impl Stack {
    #[inline(always)]
    fn push(&mut self, x: i64) {
        if self.n == self.v.len() {
            trap()
        }
        self.v[self.n] = x;
        self.n += 1;
    }
    #[inline(always)]
    fn pop(&mut self) -> i64 {
        if self.n == 0 {
            trap()
        }
        self.n -= 1;
        self.v[self.n]
    }
    #[inline(always)]
    fn peek(&self, depth: usize) -> i64 {
        if depth >= self.n {
            trap()
        }
        self.v[self.n - 1 - depth]
    }
}

fn execute(program: &[u8], data: &[&[u8]]) -> Vec<u8> {
    let mut c = Code { b: program, pc: 0 };
    let (mut out, mut mem) = (Vec::new(), Vec::<i64>::new());
    let mut s = Stack { v: [0; 1024], n: 0 };
    let mut regs = [0i64; 256];
    loop {
        let op = c.byte();
        match op {
            0x00 => return out,
            0x01 => {
                let z = c.varint();
                s.push(((z >> 1) as i64) ^ -((z & 1) as i64));
            }
            0x02..=0x0F if op != 0x0E => {
                let (b, a) = (s.pop(), s.pop());
                s.push(match op {
                    0x02 => a.wrapping_add(b),
                    0x03 => a.wrapping_sub(b),
                    0x04 => a.wrapping_mul(b),
                    0x05 if b != 0 => a.wrapping_div(b),
                    0x06 if b != 0 => a.wrapping_rem(b),
                    0x07 => a & b,
                    0x08 => a | b,
                    0x09 => a ^ b,
                    0x0A => a.wrapping_shl(b as u32),
                    0x0B => a.wrapping_shr(b as u32),
                    0x0C => (a < b) as i64,
                    0x0D => (a == b) as i64,
                    0x0F => ((a as u64).wrapping_shr(b as u32)) as i64,
                    _ => trap(),
                });
            }
            0x0E => {
                let a = s.pop();
                s.push((a == 0) as i64);
            }
            0x10 => s.push(s.peek(0)),
            0x11 => {
                let (b, a) = (s.pop(), s.pop());
                s.push(b);
                s.push(a);
            }
            0x12 => drop(s.pop()),
            0x13 => s.push(s.peek(1)),
            0x14 => s.push(regs[c.byte() as usize]),
            0x15 => regs[c.byte() as usize] = s.pop(),
            0x16 => {
                let i = s.pop() as usize;
                s.push(mem.get(i).copied().unwrap_or(0));
            }
            0x17 => {
                let (i, v) = (s.pop() as usize, s.pop());
                if i >= 1 << 22 {
                    trap()
                }
                if mem.len() <= i {
                    mem.resize(i + 1, 0);
                }
                mem[i] = v;
            }
            0x18 => c.pc = c.u16(),
            0x19 | 0x1A => {
                let t = c.u16();
                if (s.pop() == 0) == (op == 0x19) {
                    c.pc = t;
                }
            }
            0x20 => out.push(s.pop() as u8),
            0x21 => emit_dec(&mut out, s.pop(), 0, b' '),
            0x22 => {
                let (w, f) = (c.byte() as usize, c.byte());
                emit_dec(&mut out, s.pop(), w, f);
            }
            0x23 => {
                let b = c.bytes();
                out.extend_from_slice(b);
            }
            0x24 => {
                let a = data.get(c.byte() as usize).unwrap_or_else(|| trap());
                s.push(a.len() as i64);
            }
            0x25 => {
                let a = data.get(c.byte() as usize).unwrap_or_else(|| trap());
                let i = s.pop() as usize;
                s.push(*a.get(i).unwrap_or_else(|| trap()) as i64);
            }
            0x27 => {
                let (n, i) = (c.byte() as usize, s.pop() as usize);
                if i >= n {
                    trap()
                }
                for k in 0..n {
                    let b = c.bytes();
                    if k == i {
                        out.extend_from_slice(b);
                    }
                }
            }
            _ => trap(),
        }
    }
}
