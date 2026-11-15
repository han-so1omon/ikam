//! E282: a net's step compiled with Cranelift, the code generator used by
//! wasmtime and as a rustc backend, next to the kernel's `net::Table` and
//! clang -O2 (E277's native mode).
//!
//! The same seeded random walk as E277 (list the enabled transitions in
//! order, pick one with a shared xorshift generator, fire it) on seeded
//! random conservative nets of 10 x 20, 100 x 200 and 1,000 x 2,000. Three
//! modes, each checked to end in the same marking after the same steps:
//!
//! - `table`: the kernel's `Table::walk`.
//! - `cranelift`: one function per net, built with cranelift-frontend:
//!   a branch-free enabling pass (each transition's test computed and its
//!   index stored at the next slot, the count advanced by the test), the
//!   generator, then a jump table to one firing block per transition.
//!   Compiled in memory with cranelift-jit at `opt_level=speed`.
//! - `clang`: the same structure emitted as C, clang -O2, dlopen.
//!
//! Reported: ns per step, compile ms, and how many steps repay a compile
//! against the table.
//!
//!   cargo run --release   (in packages/kernel/experiments/net-jit)
//!
//! With `-- inc`, E288 instead: the incremental step compiled (see
//! `inc.rs`).

mod inc;

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
use ikam_kernel::net::{Arc, Kind, Net, Place, Table, Transition};

#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A conservative random net: each transition moves as many tokens out as
/// it takes in, so the walk neither explodes nor drains, and a cycle of
/// single moves keeps it live.
fn random_net(r: &mut Rng, places: usize, transitions: usize) -> Net {
    let pick = |r: &mut Rng, k: usize| {
        let mut v: Vec<usize> = Vec::new();
        while v.len() < k {
            let p = r.below(places);
            if !v.contains(&p) {
                v.push(p);
            }
        }
        v
    };
    let ts = (0..transitions)
        .map(|t| {
            // The first `places` transitions move one token round a cycle,
            // so some transition is always enabled and the walk never
            // deadlocks.
            if t < places {
                let arc = |p: usize| {
                    vec![Arc {
                        place: format!("p{p}"),
                        weight: 1,
                    }]
                };
                return Transition {
                    name: format!("t{t}"),
                    inputs: arc(t),
                    outputs: arc((t + 1) % places),
                    kind: Kind::Pure,
                    capability: None,
                };
            }
            let k = 1 + r.below(3);
            let ins = pick(r, k);
            let inputs: Vec<Arc> = (ins.iter())
                .map(|&p| Arc {
                    place: format!("p{p}"),
                    weight: 1 + r.below(2) as u32,
                })
                .collect();
            let total: u32 = inputs.iter().map(|a| a.weight).sum();
            let k = 1 + r.below(3).min(total as usize - 1);
            let outs = pick(r, k);
            let mut weights = vec![1u32; outs.len()];
            for _ in 0..(total as usize - outs.len()) {
                weights[r.below(outs.len())] += 1;
            }
            Transition {
                name: format!("t{t}"),
                inputs,
                outputs: (outs.iter().zip(weights))
                    .map(|(&p, w)| Arc {
                        place: format!("p{p}"),
                        weight: w,
                    })
                    .collect(),
                kind: Kind::Pure,
                capability: None,
            }
        })
        .collect();
    Net {
        name: "walk".into(),
        version: 1,
        places: (0..places)
            .map(|p| Place {
                name: format!("p{p}"),
                colored: false,
                initial: 2 + r.below(5) as u32,
            })
            .collect(),
        transitions: ts,
    }
}

type Walk = extern "C" fn(*mut u64, *mut u32, u64, u64) -> u64;

/// walk(m, e, steps, x) -> steps taken; m: place counts, e: scratch of
/// nt + 1 slots.
fn compile_cranelift(t: &Table) -> (JITModule, Walk) {
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
    for _ in 0..4 {
        ctx.func.signature.params.push(AbiParam::new(i64t));
    }
    ctx.func.signature.returns.push(AbiParam::new(i64t));
    ctx.func.name = UserFuncName::user(0, 0);
    let tcfg = module.target_config();
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let flags = MemFlagsData::trusted();
        let entry = b.create_block();
        let head = b.create_block();
        let body = b.create_block();
        let pick = b.create_block();
        let exit = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.append_block_param(head, i64t); // s
        b.append_block_param(head, i64t); // x
        b.append_block_param(exit, i64t);
        b.append_block_param(pick, i64t); // n
        b.switch_to_block(entry);
        let ps = b.block_params(entry).to_vec();
        let (m, e, steps, x0) = (ps[0], ps[1], ps[2], ps[3]);
        let zero = b.ins().iconst(i64t, 0);
        b.ins().jump(head, &[zero.into(), x0.into()]);

        b.switch_to_block(head);
        let s = b.block_params(head)[0];
        let x = b.block_params(head)[1];
        let done = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, s, steps);
        b.ins().brif(done, exit, &[s.into()], body, &[]);

        // Branch-free enabling pass.
        b.switch_to_block(body);
        let mut n = b.ins().iconst(i64t, 0);
        for (i, ins) in t.inputs.iter().enumerate() {
            let mut ok = b.ins().iconst(types::I8, 1);
            for &(p, w) in ins {
                let v = b.ins().load(i64t, flags, m, (p * 8) as i32);
                let c = b
                    .ins()
                    .icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, v, w as i64);
                ok = b.ins().band(ok, c);
            }
            let off = b.ins().ishl_imm_u(n, 2);
            let addr = b.ins().iadd(e, off);
            let idx = b.ins().iconst(types::I32, i as i64);
            b.ins().store(flags, idx, addr, 0);
            let inc = b.ins().uextend(i64t, ok);
            n = b.ins().iadd(n, inc);
        }
        let none = b.ins().icmp_imm_u(IntCC::Equal, n, 0);
        b.ins().brif(none, exit, &[s.into()], pick, &[n.into()]);

        // Generator, pick, dispatch.
        b.switch_to_block(pick);
        let n = b.block_params(pick)[0];
        let a = b.ins().ushr_imm_u(x, 12);
        let x1 = b.ins().bxor(x, a);
        let a = b.ins().ishl_imm_u(x1, 25);
        let x2 = b.ins().bxor(x1, a);
        let a = b.ins().ushr_imm_u(x2, 27);
        let x3 = b.ins().bxor(x2, a);
        let r = b.ins().imul_imm_u(x3, 0x2545_F491_4F6C_DD1Du64 as i64);
        let k = b.ins().urem(r, n);
        let off = b.ins().ishl_imm_u(k, 2);
        let addr = b.ins().iadd(e, off);
        let ti = b.ins().load(types::I32, flags, addr, 0);
        let ti = b.ins().uextend(i64t, ti);
        let s1 = b.ins().iadd_imm_s(s, 1);
        let fires: Vec<_> = (0..t.inputs.len()).map(|_| b.create_block()).collect();
        let trap = b.create_block();
        let mut sw = Switch::new();
        for (i, blk) in fires.iter().enumerate() {
            sw.set_entry(i as u128, *blk);
        }
        sw.emit(&mut b, ti, trap);
        for (i, blk) in fires.iter().enumerate() {
            b.switch_to_block(*blk);
            let mut delta = std::collections::BTreeMap::<usize, i64>::new();
            for &(p, w) in &t.inputs[i] {
                *delta.entry(p).or_default() -= w as i64;
            }
            for &(p, w) in &t.outputs[i] {
                *delta.entry(p).or_default() += w as i64;
            }
            for (p, d) in delta.into_iter().filter(|(_, d)| *d != 0) {
                let v = b.ins().load(i64t, flags, m, (p * 8) as i32);
                let v = b.ins().iadd_imm_s(v, d);
                b.ins().store(flags, v, m, (p * 8) as i32);
            }
            b.ins().jump(head, &[s1.into(), x3.into()]);
        }
        b.switch_to_block(trap);
        b.ins().jump(exit, &[s.into()]);
        b.switch_to_block(exit);
        let r = b.block_params(exit)[0];
        b.ins().return_(&[r]);
        b.seal_all_blocks();
        b.finalize(tcfg);
    }
    let id = module
        .declare_function("walk", Linkage::Export, &ctx.func.signature)
        .unwrap();
    module.define_function(id, &mut ctx).unwrap();
    module.clear_context(&mut ctx);
    module.finalize_definitions().unwrap();
    let ptr = module.get_finalized_function(id);
    // SAFETY: the function was just compiled with the Walk signature.
    let f = unsafe { std::mem::transmute::<*const u8, Walk>(ptr) };
    (module, f)
}

/// The same walk as C, for clang -O2.
fn emit_c(t: &Table) -> String {
    let nt = t.inputs.len();
    let mut c = String::from("typedef unsigned long long u64; typedef unsigned int u32;\n");
    writeln!(c, "u64 walk(u64* M, u32* E, u64 steps, u64 x) {{").unwrap();
    writeln!(c, "  for (u64 s = 0; s < steps; s++) {{ u64 n = 0;").unwrap();
    for (i, ins) in t.inputs.iter().enumerate() {
        let cond: Vec<String> = ins.iter().map(|(p, w)| format!("(M[{p}]>={w})")).collect();
        writeln!(c, "    E[n] = {i}; n += {};", cond.join("&")).unwrap();
    }
    writeln!(c, "    if (n == 0) return s;").unwrap();
    writeln!(
        c,
        "    x ^= x >> 12; x ^= x << 25; x ^= x >> 27; u64 r = x * 0x2545F4914F6CDD1DULL;"
    )
    .unwrap();
    writeln!(c, "    switch (E[r % n]) {{").unwrap();
    for i in 0..nt {
        let mut body = String::new();
        for (p, w) in &t.inputs[i] {
            write!(body, "M[{p}]-={w}; ").unwrap();
        }
        for (p, w) in &t.outputs[i] {
            write!(body, "M[{p}]+={w}; ").unwrap();
        }
        writeln!(c, "      case {i}: {body}break;").unwrap();
    }
    writeln!(c, "    }}\n  }}\n  return steps;\n}}").unwrap();
    c
}

fn clang_native(dir: &std::path::Path, i: usize, t: &Table) -> (Walk, f64) {
    let c = dir.join(format!("net{i}.c"));
    std::fs::write(&c, emit_c(t)).unwrap();
    let so = dir.join(format!("net{i}.so"));
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
        let name = CString::new("walk").unwrap();
        let f = libc::dlsym(h, name.as_ptr());
        assert!(!f.is_null());
        (std::mem::transmute::<*mut libc::c_void, Walk>(f), ms)
    }
}

fn run(f: Walk, t: &Table, steps: u64, seed: u64) -> (Vec<u64>, u64, f64) {
    let mut m = t.initial.clone();
    let mut e = vec![0u32; t.inputs.len() + 1];
    let start = Instant::now();
    let done = f(m.as_mut_ptr(), e.as_mut_ptr(), steps, seed);
    (
        m,
        done,
        start.elapsed().as_nanos() as f64 / done.max(1) as f64,
    )
}

const LOOP: &str = "\
net loop 1
place cpu 1
place queued colored
place running colored
place measured colored
place waiting colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
transition ask pure: measured -> waiting
transition merge human: waiting ->
";

/// E289: places with more readers than this are rechecked by a loop.
const HUB: usize = 16;

/// E288: table, incremental (E286), and the incremental step compiled with
/// Cranelift and with clang -O2, on random nets and the loop net unfolded.
fn main_inc() {
    let dir = tempfile::tempdir().unwrap();
    let seed = 0x0123_4567_89AB_CDEF;
    println!(
        "net | places x transitions | rechecks per transition | table ns | incremental ns | cranelift inc ns (compile ms) | clang inc ns (compile ms) | hybrid cranelift ns (compile ms) | hybrid clang ns (compile ms) | hybrid unrolled rechecks"
    );
    let mut nets: Vec<(String, Net)> = Vec::new();
    for (i, &(np, nt)) in [(10usize, 20usize), (100, 200), (1000, 2000)]
        .iter()
        .enumerate()
    {
        nets.push((
            format!("random {np} x {nt}"),
            random_net(&mut Rng(0xA5A5_0000 + i as u64), np, nt),
        ));
    }
    for c in [10usize, 100, 1000] {
        let names: Vec<String> = (0..c).map(|i| format!("c{i}")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let net = Net::parse(LOOP).unwrap().unfold(&refs).unwrap();
        nets.push((format!("loop net, {c} candidates"), net));
    }
    for (i, (label, net)) in nets.iter().enumerate() {
        net.check().unwrap();
        let t = net.lower().unwrap();
        let nt = t.inputs.len();
        let steps = (200_000_000 / nt as u64).clamp(200_000, 5_000_000);
        let per = inc::recheck_count(&t, usize::MAX) as f64 / nt as f64;

        let mut tm = t.initial.clone();
        let mut rng = Rng(seed);
        let start = Instant::now();
        let tdone = t.walk(&mut tm, steps, |n| (rng.next() % n as u64) as usize);
        let table_ns = start.elapsed().as_nanos() as f64 / tdone.max(1) as f64;

        let mut im = t.initial.clone();
        let mut rng = Rng(seed);
        let start = Instant::now();
        let idone = t.walk_incremental(&mut im, steps, |n| (rng.next() % n as u64) as usize);
        let inc_ns = start.elapsed().as_nanos() as f64 / idone.max(1) as f64;
        assert_eq!((&im, idone), (&tm, tdone), "incremental differs");

        let hub = inc::Hub::new(&t);
        let mut cols = Vec::new();
        for h in [usize::MAX, HUB] {
            let rechecks = inc::recheck_count(&t, h);
            if rechecks > 1_000_000 {
                // Unrolled, the rechecks alone would be this many enabling
                // tests in the code; the first try ran out of memory here.
                cols.push(format!(
                    "not compiled: {rechecks} unrolled rechecks | not compiled"
                ));
                continue;
            }
            let start = Instant::now();
            let (_module, cf) = inc::compile_cranelift(&t, h);
            let cl_ms = start.elapsed().as_secs_f64() * 1000.0;
            let (cm, cdone, cl_ns) = inc::run(cf, &t, &hub, steps, seed);
            assert_eq!((&cm, cdone), (&tm, tdone), "cranelift inc differs");

            let (nf, cc_ms) = inc::clang_native(dir.path(), i, &t, h);
            let (nm, ndone, cc_ns) = inc::run(nf, &t, &hub, steps, seed);
            assert_eq!((&nm, ndone), (&tm, tdone), "clang inc differs");
            let tail = if h == HUB {
                format!(" | {rechecks} unrolled")
            } else {
                String::new()
            };
            cols.push(format!(
                "{cl_ns:.1} ({cl_ms:.1}) | {cc_ns:.1} ({cc_ms:.0}){tail}"
            ));
        }
        println!(
            "{label} | {} x {nt} | {per:.1} | {table_ns:.1} | {inc_ns:.1} | {}",
            t.initial.len(),
            cols.join(" | ")
        );
    }
}

fn main() {
    if std::env::args().any(|a| a == "inc") {
        return main_inc();
    }
    let dir = tempfile::tempdir().unwrap();
    let seed = 0x0123_4567_89AB_CDEF;
    println!("net | mode | ns per step | compile ms | repays after (steps, vs table)");
    for (i, &(np, nt)) in [(10usize, 20usize), (100, 200), (1000, 2000)]
        .iter()
        .enumerate()
    {
        let net = random_net(&mut Rng(0xA5A5_0000 + i as u64), np, nt);
        net.check().unwrap();
        let t = net.lower().unwrap();
        let steps = (200_000_000 / nt as u64).clamp(100_000, 10_000_000);

        let mut tm = t.initial.clone();
        let mut rng = Rng(seed);
        let start = Instant::now();
        let tdone = t.walk(&mut tm, steps, |n| (rng.next() % n as u64) as usize);
        let table_ns = start.elapsed().as_nanos() as f64 / tdone as f64;
        assert_eq!(tdone, steps, "deadlock");

        let start = Instant::now();
        let (_module, cf) = compile_cranelift(&t);
        let cl_ms = start.elapsed().as_secs_f64() * 1000.0;
        let (cm, cdone, cl_ns) = run(cf, &t, steps, seed);
        assert_eq!((&cm, cdone), (&tm, tdone), "cranelift differs");

        let (nf, cc_ms) = clang_native(dir.path(), i, &t);
        let (nm, ndone, cc_ns) = run(nf, &t, steps, seed);
        assert_eq!((&nm, ndone), (&tm, tdone), "clang differs");

        let repay = |ns: f64, ms: f64| {
            if ns >= table_ns {
                "never".to_string()
            } else {
                format!("{:.0}", ms * 1e6 / (table_ns - ns))
            }
        };
        println!("{np} x {nt} | table | {table_ns:.1} | - | -");
        println!(
            "{np} x {nt} | cranelift | {cl_ns:.1} | {cl_ms:.1} | {}",
            repay(cl_ns, cl_ms)
        );
        println!(
            "{np} x {nt} | clang -O2 | {cc_ns:.1} | {cc_ms:.0} | {}",
            repay(cc_ns, cc_ms)
        );
    }
}
