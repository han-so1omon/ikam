//! E288: the incremental step (E286) compiled, with Cranelift and with
//! clang -O2.
//!
//! E286's `Table::walk_incremental` rechecks, after a firing, only the
//! transitions that read a place the firing changed, and picks the k-th
//! enabled transition from a Fenwick tree. Compiled, both parts become
//! straight-line code known at compile time: each transition's firing
//! block updates the places it changes, then recomputes the enabling test
//! of each of its readers (a fixed list) and flips that reader's bit in an
//! enabled bitset, keeping the count. The pick scans the bitset's words by
//! popcount, then clears low bits to reach the k-th, and dispatches through
//! a jump table. The same seeded generator and pick as the kernel's walks,
//! so the result must equal `Table::walk_incremental` (and `Table::walk`).

use std::ffi::CString;
use std::fmt::Write as _;
use std::process::Command;
use std::time::Instant;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, UserFuncName, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};
use ikam_kernel::net::Table;

/// walk(m, bits, steps, x, n) -> steps taken; m: place counts, bits: the
/// enabled bitset (nt / 64 + 1 words), n: how many bits are set.
pub type WalkInc = extern "C" fn(*mut u64, *mut u64, u64, u64, u64) -> u64;

/// For each transition, the places it changes (with the net change) and
/// the distinct transitions that read any of them.
/// Per transition: the changed places with their net change.
type Deltas = Vec<Vec<(usize, i64)>>;

fn plan(t: &Table) -> (Deltas, Vec<Vec<usize>>) {
    let nt = t.inputs.len();
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); t.initial.len()];
    for (i, ins) in t.inputs.iter().enumerate() {
        for &(p, _) in ins {
            if readers[p].last() != Some(&i) {
                readers[p].push(i);
            }
        }
    }
    let mut deltas = Vec::with_capacity(nt);
    let mut rechecks = Vec::with_capacity(nt);
    for i in 0..nt {
        let mut d = std::collections::BTreeMap::<usize, i64>::new();
        for &(p, w) in &t.inputs[i] {
            *d.entry(p).or_default() -= w as i64;
        }
        for &(p, w) in &t.outputs[i] {
            *d.entry(p).or_default() += w as i64;
        }
        let d: Vec<(usize, i64)> = d.into_iter().filter(|(_, x)| *x != 0).collect();
        let mut r: Vec<usize> = d.iter().flat_map(|(p, _)| readers[*p].clone()).collect();
        r.sort_unstable();
        r.dedup();
        deltas.push(d);
        rechecks.push(r);
    }
    (deltas, rechecks)
}

/// The initial bitset and count for `m`.
pub fn bits(t: &Table, m: &[u64]) -> (Vec<u64>, u64) {
    let nt = t.inputs.len();
    let mut b = vec![0u64; nt / 64 + 1];
    let mut n = 0;
    for i in 0..nt {
        if t.is_enabled(m, i) {
            b[i / 64] |= 1 << (i % 64);
            n += 1;
        }
    }
    (b, n)
}

/// The total of rechecks over transitions, for the report.
pub fn recheck_count(t: &Table) -> usize {
    plan(t).1.iter().map(|r| r.len()).sum()
}

pub fn compile_cranelift(t: &Table) -> (JITModule, WalkInc) {
    let (deltas, rechecks) = plan(t);
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    flags.set("use_colocated_libcalls", "false").unwrap();
    flags.set("is_pic", "false").unwrap();
    let isa = cranelift_native::builder()
        .unwrap()
        .finish(settings::Flags::new(flags))
        .unwrap();
    let mut module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));
    let mut ctx = module.make_context();
    let i64t = types::I64;
    for _ in 0..5 {
        ctx.func.signature.params.push(AbiParam::new(i64t));
    }
    ctx.func.signature.returns.push(AbiParam::new(i64t));
    ctx.func.name = UserFuncName::user(0, 1);
    let tcfg = module.target_config();
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let fl = MemFlagsData::trusted();
        let [vs, vx, vn, vk, vw, vv] = [(); 6].map(|_| b.declare_var(i64t));
        let entry = b.create_block();
        let head = b.create_block();
        let live = b.create_block();
        let body = b.create_block();
        let wl = b.create_block();
        let wl_next = b.create_block();
        let bl_init = b.create_block();
        let bl = b.create_block();
        let bl_step = b.create_block();
        let sel = b.create_block();
        let exit = b.create_block();
        b.append_block_params_for_function_params(entry);

        b.switch_to_block(entry);
        let ps = b.block_params(entry).to_vec();
        let (m, e, steps) = (ps[0], ps[1], ps[2]);
        let zero = b.ins().iconst(i64t, 0);
        b.def_var(vs, zero);
        b.def_var(vx, ps[3]);
        b.def_var(vn, ps[4]);
        b.ins().jump(head, &[]);

        b.switch_to_block(head);
        let s = b.use_var(vs);
        let done = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, s, steps);
        b.ins().brif(done, exit, &[], live, &[]);

        b.switch_to_block(live);
        let n = b.use_var(vn);
        let none = b.ins().icmp_imm_u(IntCC::Equal, n, 0);
        b.ins().brif(none, exit, &[], body, &[]);

        // Generator and k.
        b.switch_to_block(body);
        let x = b.use_var(vx);
        let a = b.ins().ushr_imm_u(x, 12);
        let x1 = b.ins().bxor(x, a);
        let a = b.ins().ishl_imm_u(x1, 25);
        let x2 = b.ins().bxor(x1, a);
        let a = b.ins().ushr_imm_u(x2, 27);
        let x3 = b.ins().bxor(x2, a);
        b.def_var(vx, x3);
        let r = b.ins().imul_imm_u(x3, 0x2545_F491_4F6C_DD1Du64 as i64);
        let n = b.use_var(vn);
        let k = b.ins().urem(r, n);
        b.def_var(vk, k);
        let zero = b.ins().iconst(i64t, 0);
        b.def_var(vw, zero);
        b.ins().jump(wl, &[]);

        // The word that holds the k-th set bit.
        b.switch_to_block(wl);
        let w = b.use_var(vw);
        let off = b.ins().ishl_imm_u(w, 3);
        let addr = b.ins().iadd(e, off);
        let v = b.ins().load(i64t, fl, addr, 0);
        let c = b.ins().popcnt(v);
        let k = b.use_var(vk);
        let here = b.ins().icmp(IntCC::UnsignedLessThan, k, c);
        b.ins().brif(here, bl_init, &[], wl_next, &[]);

        b.switch_to_block(wl_next);
        let k = b.use_var(vk);
        let k1 = b.ins().isub(k, c);
        b.def_var(vk, k1);
        let w = b.use_var(vw);
        let w1 = b.ins().iadd_imm_s(w, 1);
        b.def_var(vw, w1);
        b.ins().jump(wl, &[]);

        b.switch_to_block(bl_init);
        b.def_var(vv, v);
        b.ins().jump(bl, &[]);

        // Clear k low set bits.
        b.switch_to_block(bl);
        let k = b.use_var(vk);
        let at = b.ins().icmp_imm_u(IntCC::Equal, k, 0);
        b.ins().brif(at, sel, &[], bl_step, &[]);

        b.switch_to_block(bl_step);
        let v = b.use_var(vv);
        let v1 = b.ins().iadd_imm_s(v, -1);
        let v2 = b.ins().band(v, v1);
        b.def_var(vv, v2);
        let k = b.use_var(vk);
        let k1 = b.ins().iadd_imm_s(k, -1);
        b.def_var(vk, k1);
        b.ins().jump(bl, &[]);

        b.switch_to_block(sel);
        let v = b.use_var(vv);
        let tz = b.ins().ctz(v);
        let w = b.use_var(vw);
        let base = b.ins().ishl_imm_u(w, 6);
        let ti = b.ins().iadd(base, tz);
        let fires: Vec<_> = (0..t.inputs.len()).map(|_| b.create_block()).collect();
        let trap = b.create_block();
        let mut sw = Switch::new();
        for (i, blk) in fires.iter().enumerate() {
            sw.set_entry(i as u128, *blk);
        }
        sw.emit(&mut b, ti, trap);

        for (i, blk) in fires.iter().enumerate() {
            b.switch_to_block(*blk);
            for &(p, d) in &deltas[i] {
                let v = b.ins().load(i64t, fl, m, (p * 8) as i32);
                let v = b.ins().iadd_imm_s(v, d);
                b.ins().store(fl, v, m, (p * 8) as i32);
            }
            for &r in &rechecks[i] {
                let mut ok = b.ins().iconst(types::I8, 1);
                for &(p, w) in &t.inputs[r] {
                    let v = b.ins().load(i64t, fl, m, (p * 8) as i32);
                    let c = b
                        .ins()
                        .icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, v, w as i64);
                    ok = b.ins().band(ok, c);
                }
                let now = b.ins().uextend(i64t, ok);
                let woff = ((r / 64) * 8) as i32;
                let word = b.ins().load(i64t, fl, e, woff);
                let sh = b.ins().ushr_imm_u(word, (r % 64) as i64);
                let was = b.ins().band_imm_u(sh, 1);
                let diff = b.ins().bxor(now, was);
                let flip = b.ins().ishl_imm_u(diff, (r % 64) as i64);
                let word = b.ins().bxor(word, flip);
                b.ins().store(fl, word, e, woff);
                let n = b.use_var(vn);
                let n = b.ins().iadd(n, now);
                let n = b.ins().isub(n, was);
                b.def_var(vn, n);
            }
            let s = b.use_var(vs);
            let s1 = b.ins().iadd_imm_s(s, 1);
            b.def_var(vs, s1);
            b.ins().jump(head, &[]);
        }
        b.switch_to_block(trap);
        b.ins().jump(exit, &[]);
        b.switch_to_block(exit);
        let s = b.use_var(vs);
        b.ins().return_(&[s]);
        b.seal_all_blocks();
        b.finalize(tcfg);
    }
    let id = module
        .declare_function("walk_inc", Linkage::Export, &ctx.func.signature)
        .unwrap();
    module.define_function(id, &mut ctx).unwrap();
    module.clear_context(&mut ctx);
    module.finalize_definitions().unwrap();
    let ptr = module.get_finalized_function(id);
    // SAFETY: the function was just compiled with the WalkInc signature.
    let f = unsafe { std::mem::transmute::<*const u8, WalkInc>(ptr) };
    (module, f)
}

/// The same incremental walk as C, for clang -O2.
fn emit_c(t: &Table) -> String {
    let (deltas, rechecks) = plan(t);
    let mut c = String::from("typedef unsigned long long u64;\n");
    writeln!(
        c,
        "u64 walk_inc(u64* M, u64* E, u64 steps, u64 x, u64 n) {{"
    )
    .unwrap();
    writeln!(c, "  for (u64 s = 0; s < steps; s++) {{").unwrap();
    writeln!(c, "    if (n == 0) return s;").unwrap();
    writeln!(
        c,
        "    x ^= x >> 12; x ^= x << 25; x ^= x >> 27; u64 k = (x * 0x2545F4914F6CDD1DULL) % n;"
    )
    .unwrap();
    writeln!(c, "    u64 w = 0;").unwrap();
    writeln!(
        c,
        "    for (;; w++) {{ u64 c = __builtin_popcountll(E[w]); if (k < c) break; k -= c; }}"
    )
    .unwrap();
    writeln!(c, "    u64 v = E[w]; while (k) {{ v &= v - 1; k--; }}").unwrap();
    writeln!(c, "    switch (w * 64 + __builtin_ctzll(v)) {{").unwrap();
    for i in 0..t.inputs.len() {
        let mut body = String::new();
        for (p, d) in &deltas[i] {
            write!(body, "M[{p}]+={d}LL; ").unwrap();
        }
        for &r in &rechecks[i] {
            let cond: Vec<String> = t.inputs[r]
                .iter()
                .map(|(p, w)| format!("(M[{p}]>={w}ULL)"))
                .collect();
            let cond = if cond.is_empty() {
                "1".to_string()
            } else {
                cond.join("&")
            };
            let (wd, bt) = (r / 64, r % 64);
            write!(
                body,
                "{{ u64 a = (u64)({cond}); u64 o = (E[{wd}] >> {bt}) & 1; E[{wd}] ^= (a ^ o) << {bt}; n += a - o; }} "
            )
            .unwrap();
        }
        writeln!(c, "      case {i}: {body}break;").unwrap();
    }
    writeln!(c, "    }}\n  }}\n  return steps;\n}}").unwrap();
    c
}

pub fn clang_native(dir: &std::path::Path, i: usize, t: &Table) -> (WalkInc, f64) {
    let c = dir.join(format!("inc{i}.c"));
    std::fs::write(&c, emit_c(t)).unwrap();
    let so = dir.join(format!("inc{i}.so"));
    let start = Instant::now();
    let out = Command::new("clang")
        .args([
            "-O2",
            "-shared",
            "-fPIC",
            "-o",
            so.to_str().unwrap(),
            c.to_str().unwrap(),
        ])
        .output()
        .expect("clang");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let path = CString::new(so.to_str().unwrap()).unwrap();
    // SAFETY: the library was just built from the C above.
    unsafe {
        let h = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        assert!(!h.is_null());
        let name = CString::new("walk_inc").unwrap();
        let f = libc::dlsym(h, name.as_ptr());
        assert!(!f.is_null());
        (std::mem::transmute::<*mut libc::c_void, WalkInc>(f), ms)
    }
}

/// Run a compiled incremental walk from the net's initial marking.
pub fn run(f: WalkInc, t: &Table, steps: u64, seed: u64) -> (Vec<u64>, u64, f64) {
    let mut m = t.initial.clone();
    let (mut e, n) = bits(t, &m);
    let start = Instant::now();
    let done = f(m.as_mut_ptr(), e.as_mut_ptr(), steps, seed, n);
    (
        m,
        done,
        start.elapsed().as_nanos() as f64 / done.max(1) as f64,
    )
}
