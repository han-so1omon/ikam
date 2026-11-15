//! E277: execution modes for a net's step, and whether they matter.
//!
//! The same random walk (at each step, list the enabled transitions in
//! order, pick one with a shared xorshift generator, fire it) runs in four
//! modes on seeded random place/transition nets (plain places; a colored
//! net unfolds to one copy per color):
//!
//! - `interp`: the `net` module's in-memory `Net::is_enabled` and
//!   `Net::fire`, on string-keyed markings.
//! - `table`: the net lowered once to dense arrays (place indices, each
//!   transition's input thresholds and its delta), a table-driven loop.
//! - `native`: the net compiled to specialized C (one enabling test and one
//!   firing per transition, unrolled), built with clang -O2 (LLVM) into a
//!   shared library and loaded with dlopen. The closest available stand-in
//!   for an MLIR-to-LLVM pipeline, and the shape of Fronc and Pommereau's
//!   compiled backends.
//! - `wasm`: the same C built for wasm32 and run in wasmi, the kernel's
//!   deterministic interpreter (how guards would run under the kernel's
//!   reconstruction rules).
//!
//! Each mode must end in the same marking after the same steps. Reported:
//! ns per step, and for the compiled modes the compile time. E269's
//! database firing (about 8.6 ms) is the comparison for the live loop.
//!
//!   cargo run --release --example net_modes
//!
//! Needs clang with the wasm32 target and lld on PATH; deterministic
//! results, machine-dependent times. Writes only to a temp directory.

use std::ffi::CString;
use std::fmt::Write as _;
use std::process::Command;
use std::time::Instant;

use ikam_kernel::net::{Arc, Kind, Marking, Net, Place, Transition};

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

/// Dense lowering: place index, thresholds and delta per transition.
struct Table {
    initial: Vec<u32>,
    inputs: Vec<Vec<(usize, u32)>>,
    outputs: Vec<Vec<(usize, u32)>>,
}

fn lower(net: &Net) -> Table {
    let idx = |name: &str| net.places.iter().position(|p| p.name == name).unwrap();
    let side = |arcs: &[Arc]| arcs.iter().map(|a| (idx(&a.place), a.weight)).collect();
    Table {
        initial: net.places.iter().map(|p| p.initial).collect(),
        inputs: net.transitions.iter().map(|t| side(&t.inputs)).collect(),
        outputs: net.transitions.iter().map(|t| side(&t.outputs)).collect(),
    }
}

fn walk_table(t: &Table, steps: u64, seed: u64) -> (Vec<u32>, u64) {
    let mut m = t.initial.clone();
    let mut rng = Rng(seed);
    let mut enabled = Vec::with_capacity(t.inputs.len());
    for step in 0..steps {
        enabled.clear();
        for (i, ins) in t.inputs.iter().enumerate() {
            if ins.iter().all(|&(p, w)| m[p] >= w) {
                enabled.push(i);
            }
        }
        if enabled.is_empty() {
            return (m, step);
        }
        let i = enabled[(rng.next() % enabled.len() as u64) as usize];
        for &(p, w) in &t.inputs[i] {
            m[p] -= w;
        }
        for &(p, w) in &t.outputs[i] {
            m[p] += w;
        }
    }
    (m, steps)
}

fn walk_interp(net: &Net, steps: u64, seed: u64) -> (Marking, u64) {
    let mut m = net.initial_marking();
    let mut rng = Rng(seed);
    for step in 0..steps {
        let enabled: Vec<&Transition> = (net.transitions.iter())
            .filter(|t| net.is_enabled(&m, t, None))
            .collect();
        if enabled.is_empty() {
            return (m, step);
        }
        let t = enabled[(rng.next() % enabled.len() as u64) as usize];
        m = net.fire(&m, t, None).unwrap();
    }
    (m, steps)
}

fn dense_of(net: &Net, m: &Marking) -> Vec<u32> {
    (net.places.iter())
        .map(|p| {
            m.get(&p.name)
                .and_then(|t| t.get("_"))
                .copied()
                .unwrap_or(0) as u32
        })
        .collect()
}

/// Specialized C: one test and one firing per transition.
fn emit_c(t: &Table) -> String {
    let (np, nt) = (t.initial.len(), t.inputs.len());
    let mut c = String::new();
    writeln!(
        c,
        "typedef unsigned int u32; typedef unsigned long long u64;"
    )
    .unwrap();
    writeln!(c, "static u32 M[{np}]; static u32 E[{nt}];").unwrap();
    let init: Vec<String> = t.initial.iter().map(|x| x.to_string()).collect();
    writeln!(c, "static const u32 I[{np}] = {{{}}};", init.join(",")).unwrap();
    // A freestanding wasm build has no libc; the loop below may become a
    // memcpy call.
    writeln!(c, "#ifdef __wasm__\nvoid* memcpy(void* d, const void* s, unsigned long n) {{ char* a = d; const char* b = s; while (n--) *a++ = *b++; return d; }}\n#endif").unwrap();
    writeln!(c, "u32* marking(void) {{ return M; }}").unwrap();
    writeln!(c, "u64 walk(u64 steps, u64 x) {{").unwrap();
    writeln!(c, "  for (int p = 0; p < {np}; p++) M[p] = I[p];").unwrap();
    writeln!(c, "  for (u64 s = 0; s < steps; s++) {{ u32 n = 0;").unwrap();
    for (i, ins) in t.inputs.iter().enumerate() {
        let cond: Vec<String> = ins.iter().map(|(p, w)| format!("M[{p}]>={w}")).collect();
        writeln!(c, "    if ({}) E[n++] = {i};", cond.join(" && ")).unwrap();
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

fn clang(args: &[&str]) {
    let out = Command::new("clang").args(args).output().expect("clang");
    assert!(
        out.status.success(),
        "clang failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Native {
    walk: extern "C" fn(u64, u64) -> u64,
    marking: extern "C" fn() -> *const u32,
}

fn load_native(so: &std::path::Path) -> Native {
    let path = CString::new(so.to_str().unwrap()).unwrap();
    // SAFETY: the library was just built from code emitted above, which
    // only reads and writes its own static arrays.
    unsafe {
        let h = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        assert!(!h.is_null(), "dlopen failed");
        let sym = |name: &str| {
            let s = CString::new(name).unwrap();
            let f = libc::dlsym(h, s.as_ptr());
            assert!(!f.is_null(), "dlsym {name}");
            f
        };
        Native {
            walk: std::mem::transmute::<*mut libc::c_void, extern "C" fn(u64, u64) -> u64>(sym(
                "walk",
            )),
            marking: std::mem::transmute::<*mut libc::c_void, extern "C" fn() -> *const u32>(sym(
                "marking",
            )),
        }
    }
}

fn walk_wasm(bytes: &[u8], np: usize, steps: u64, seed: u64) -> (Vec<u32>, u64, f64) {
    use wasmi::{Config, Engine, Linker, Module, Store};
    let mut config = Config::default();
    config.floats(false);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, bytes).unwrap();
    let mut store = Store::new(&engine, ());
    let inst = Linker::<()>::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .unwrap();
    let walk = inst
        .get_typed_func::<(i64, i64), i64>(&store, "walk")
        .unwrap();
    let marking = inst.get_typed_func::<(), i32>(&store, "marking").unwrap();
    let mem = inst.get_memory(&store, "memory").unwrap();
    let start = Instant::now();
    let done = walk.call(&mut store, (steps as i64, seed as i64)).unwrap() as u64;
    let secs = start.elapsed().as_secs_f64();
    let ptr = marking.call(&mut store, ()).unwrap() as u32 as usize;
    let mut raw = vec![0u8; np * 4];
    mem.read(&store, ptr, &mut raw).unwrap();
    let m = raw
        .chunks(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    (m, done, secs)
}

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let seed = 0x0123_4567_89AB_CDEF;
    println!("net (places x transitions) | mode | ns per step | compile ms | steps run");
    for (i, &(np, nt)) in [(10usize, 20usize), (100, 200), (1000, 2000)]
        .iter()
        .enumerate()
    {
        let net = random_net(&mut Rng(0xA5A5_0000 + i as u64), np, nt);
        net.check().unwrap();
        let table = lower(&net);
        // Interpreter: fewer steps, and the check that table agrees.
        let isteps = (2_000_000 / nt as u64).clamp(1000, 200_000);
        let start = Instant::now();
        let (im, idone) = walk_interp(&net, isteps, seed);
        let interp_ns = start.elapsed().as_nanos() as f64 / idone.max(1) as f64;
        let (tm, tdone) = walk_table(&table, isteps, seed);
        assert_eq!((dense_of(&net, &im), idone), (tm, tdone), "table differs");

        let steps = (200_000_000 / nt as u64).clamp(100_000, 10_000_000);
        let start = Instant::now();
        let (tm, tdone) = walk_table(&table, steps, seed);
        let table_ns = start.elapsed().as_nanos() as f64 / tdone.max(1) as f64;

        let c = dir.path().join(format!("net{i}.c"));
        std::fs::write(&c, emit_c(&table)).unwrap();
        let so = dir.path().join(format!("net{i}.so"));
        let start = Instant::now();
        clang(&[
            "-O2",
            "-shared",
            "-fPIC",
            "-o",
            so.to_str().unwrap(),
            c.to_str().unwrap(),
        ]);
        let native_ms = start.elapsed().as_secs_f64() * 1000.0;
        let n = load_native(&so);
        let start = Instant::now();
        let ndone = (n.walk)(steps, seed);
        let native_ns = start.elapsed().as_nanos() as f64 / ndone.max(1) as f64;
        // SAFETY: `marking` points at the library's static array of np u32s.
        let nm = unsafe { std::slice::from_raw_parts((n.marking)(), np) }.to_vec();
        assert_eq!((nm, ndone), (tm.clone(), tdone), "native differs");

        let wasm = dir.path().join(format!("net{i}.wasm"));
        let start = Instant::now();
        clang(&[
            "--target=wasm32",
            "-O2",
            "-nostdlib",
            "-fno-builtin",
            "-Wl,--no-entry",
            "-Wl,--export=walk",
            "-Wl,--export=marking",
            "-o",
            wasm.to_str().unwrap(),
            c.to_str().unwrap(),
        ]);
        let wasm_ms = start.elapsed().as_secs_f64() * 1000.0;
        let wsteps = steps / 10;
        let (wm, wdone, wsecs) = walk_wasm(&std::fs::read(&wasm).unwrap(), np, wsteps, seed);
        let (tm10, t10) = walk_table(&table, wsteps, seed);
        assert_eq!((wm, wdone), (tm10, t10), "wasm differs");
        assert_eq!(tdone, steps, "the walk deadlocked");
        let wasm_ns = wsecs * 1e9 / wdone.max(1) as f64;

        let row = |mode: &str, ns: f64, ms: Option<f64>, done: u64| {
            println!(
                "{np} x {nt} | {mode} | {ns:.1} | {} | {done}",
                ms.map(|m| format!("{m:.0}")).unwrap_or("-".into())
            )
        };
        row("interp", interp_ns, None, idone);
        row("table", table_ns, None, tdone);
        row("native", native_ns, Some(native_ms), ndone);
        row("wasm", wasm_ns, Some(wasm_ms), wdone);
    }
}
