//! E311: the same walk as `compile_cranelift` and `emit_c`, emitted as MLIR
//! (func, arith, memref and cf dialects) and lowered with MLIR's own tools:
//!
//!   mlir-opt (canonicalize, cse, lower to the llvm dialect)
//!   -> mlir-translate --mlir-to-llvmir
//!   -> [opt -O2] -> llc -O2 -> cc -shared -> dlopen
//!
//! Two modes: `mlir+llc` (MLIR's passes, then LLVM's code generator only)
//! and `mlir+opt` (LLVM's -O2 middle end too, as clang -O2 runs). The tools
//! are found in `MLIR_BIN` (a folder) or on PATH; nothing is linked into
//! this crate, so only the `mlir` feature's subcommand needs them.

use std::ffi::CString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use ikam_kernel::net::Table;

/// A one-dimensional memref as MLIR's C interface passes it.
#[repr(C)]
pub struct Desc<T> {
    allocated: *mut T,
    aligned: *mut T,
    offset: i64,
    size: i64,
    stride: i64,
}

impl<T> Desc<T> {
    fn of(v: &mut [T]) -> Self {
        Desc {
            allocated: v.as_mut_ptr(),
            aligned: v.as_mut_ptr(),
            offset: 0,
            size: v.len() as i64,
            stride: 1,
        }
    }
}

pub type CWalk = extern "C" fn(*mut Desc<u64>, *mut Desc<u32>, u64, u64) -> u64;

fn tool(name: &str) -> PathBuf {
    match std::env::var_os("MLIR_BIN") {
        Some(d) => Path::new(&d).join(name),
        None => PathBuf::from(name),
    }
}

/// The LLVM version a tool reports ("23.1.5"), or None if it is missing.
pub fn version(name: &str) -> Option<String> {
    let out = Command::new(tool(name)).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("version"))?;
    let v = line.split_whitespace().last()?;
    Some(v.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect())
}

/// MLIR for walk(m, e, steps, x) -> steps taken. Same structure as the
/// Cranelift function: a branch-free enabling pass storing each index at
/// the next slot, the shared generator, a switch to one block per firing.
pub fn emit(t: &Table) -> String {
    let nt = t.inputs.len();
    let mut used: Vec<usize> = (t.inputs.iter().chain(&t.outputs))
        .flat_map(|a| a.iter().map(|&(p, _)| p))
        .collect();
    used.sort_unstable();
    used.dedup();
    let mut s = String::from("module {\n");
    s += "  func.func @walk(%m: memref<?xi64>, %e: memref<?xi32>, %steps: i64, %x0: i64) -> i64 attributes {llvm.emit_c_interface} {\n";
    s += "    %c0 = arith.constant 0 : i64\n    %c1 = arith.constant 1 : i64\n";
    s += "    %c12 = arith.constant 12 : i64\n    %c25 = arith.constant 25 : i64\n";
    s += "    %c27 = arith.constant 27 : i64\n";
    s += "    %k = arith.constant 2685821657736338717 : i64\n";
    s += "    %true = arith.constant true\n";
    for p in &used {
        writeln!(s, "    %p{p} = arith.constant {p} : index").unwrap();
    }
    s += "    cf.br ^head(%c0, %x0 : i64, i64)\n";
    s += "  ^head(%s: i64, %x: i64):\n";
    s += "    %done = arith.cmpi uge, %s, %steps : i64\n";
    s += "    cf.cond_br %done, ^exit(%s : i64), ^body\n";
    s += "  ^body:\n";
    let mut n = "%c0".to_string();
    for (i, ins) in t.inputs.iter().enumerate() {
        let mut ok = "%true".to_string();
        for (j, &(p, w)) in ins.iter().enumerate() {
            writeln!(s, "    %v{i}_{j} = memref.load %m[%p{p}] : memref<?xi64>").unwrap();
            writeln!(s, "    %w{i}_{j} = arith.constant {w} : i64").unwrap();
            writeln!(s, "    %t{i}_{j} = arith.cmpi uge, %v{i}_{j}, %w{i}_{j} : i64").unwrap();
            writeln!(s, "    %ok{i}_{j} = arith.andi {ok}, %t{i}_{j} : i1").unwrap();
            ok = format!("%ok{i}_{j}");
        }
        writeln!(s, "    %at{i} = arith.index_cast {n} : i64 to index").unwrap();
        writeln!(s, "    %id{i} = arith.constant {i} : i32").unwrap();
        writeln!(s, "    memref.store %id{i}, %e[%at{i}] : memref<?xi32>").unwrap();
        writeln!(s, "    %inc{i} = arith.extui {ok} : i1 to i64").unwrap();
        writeln!(s, "    %n{i} = arith.addi {n}, %inc{i} : i64").unwrap();
        n = format!("%n{i}");
    }
    writeln!(s, "    %none = arith.cmpi eq, {n}, %c0 : i64").unwrap();
    s += "    cf.cond_br %none, ^exit(%s : i64), ^pick\n";
    s += "  ^pick:\n";
    s += "    %a1 = arith.shrui %x, %c12 : i64\n    %x1 = arith.xori %x, %a1 : i64\n";
    s += "    %a2 = arith.shli %x1, %c25 : i64\n    %x2 = arith.xori %x1, %a2 : i64\n";
    s += "    %a3 = arith.shrui %x2, %c27 : i64\n    %x3 = arith.xori %x2, %a3 : i64\n";
    s += "    %r = arith.muli %x3, %k : i64\n";
    writeln!(s, "    %slot = arith.remui %r, {n} : i64").unwrap();
    s += "    %slotx = arith.index_cast %slot : i64 to index\n";
    s += "    %ti = memref.load %e[%slotx] : memref<?xi32>\n";
    s += "    %s1 = arith.addi %s, %c1 : i64\n";
    s += "    cf.switch %ti : i32, [\n      default: ^exit(%s : i64)";
    for i in 0..nt {
        write!(s, ",\n      {i}: ^f{i}").unwrap();
    }
    s += "\n    ]\n";
    for i in 0..nt {
        writeln!(s, "  ^f{i}:").unwrap();
        let mut delta = std::collections::BTreeMap::<usize, i64>::new();
        for &(p, w) in &t.inputs[i] {
            *delta.entry(p).or_default() -= w as i64;
        }
        for &(p, w) in &t.outputs[i] {
            *delta.entry(p).or_default() += w as i64;
        }
        for (p, d) in delta.into_iter().filter(|(_, d)| *d != 0) {
            writeln!(s, "    %g{i}_{p} = memref.load %m[%p{p}] : memref<?xi64>").unwrap();
            writeln!(s, "    %d{i}_{p} = arith.constant {d} : i64").unwrap();
            writeln!(s, "    %h{i}_{p} = arith.addi %g{i}_{p}, %d{i}_{p} : i64").unwrap();
            writeln!(s, "    memref.store %h{i}_{p}, %m[%p{p}] : memref<?xi64>").unwrap();
        }
        s += "    cf.br ^head(%s1, %x3 : i64, i64)\n";
    }
    s += "  ^exit(%ret: i64):\n    return %ret : i64\n  }\n}\n";
    s
}

const LOWER: &[&str] = &[
    "--canonicalize",
    "--cse",
    "--convert-cf-to-llvm",
    "--convert-arith-to-llvm",
    "--finalize-memref-to-llvm",
    "--convert-func-to-llvm",
    "--reconcile-unrealized-casts",
];

fn step(name: &str, args: &[&str]) -> f64 {
    let start = Instant::now();
    let out = Command::new(tool(name))
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(
        out.status.success(),
        "{name} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .take(20)
            .collect::<Vec<_>>()
            .join("\n")
    );
    start.elapsed().as_secs_f64() * 1000.0
}

/// Compile times in ms by stage, and the total.
pub struct Stages {
    pub mlir_opt: f64,
    pub translate: f64,
    pub llvm_opt: f64,
    pub llc: f64,
    pub link: f64,
}

impl Stages {
    pub fn total(&self) -> f64 {
        self.mlir_opt + self.translate + self.llvm_opt + self.llc + self.link
    }
}

/// Builds `t` through MLIR into a shared library in `dir` and loads it.
/// The emitted text is written before the clock starts.
pub fn compile(dir: &Path, tag: &str, t: &Table, llvm_o2: bool) -> (CWalk, Stages) {
    let f = |ext: &str| dir.join(format!("{tag}.{ext}"));
    let p = |b: &PathBuf| b.to_str().unwrap().to_string();
    std::fs::write(f("mlir"), emit(t)).unwrap();
    let (src, low, ll, opt, obj, so) = (
        p(&f("mlir")),
        p(&f("llvm.mlir")),
        p(&f("ll")),
        p(&f("opt.ll")),
        p(&f("o")),
        p(&f("so")),
    );
    let mut args: Vec<&str> = vec![src.as_str()];
    args.extend_from_slice(LOWER);
    args.extend_from_slice(&["-o", low.as_str()]);
    let mlir_opt = step("mlir-opt", &args);
    let translate = step("mlir-translate", &["--mlir-to-llvmir", &low, "-o", &ll]);
    let (llvm_opt, ir) = if llvm_o2 {
        (step("opt", &["-O2", &ll, "-S", "-o", &opt]), opt.as_str())
    } else {
        (0.0, ll.as_str())
    };
    let llc = step(
        "llc",
        &["-O2", "-filetype=obj", "-relocation-model=pic", ir, "-o", &obj],
    );
    let start = Instant::now();
    let out = Command::new("cc")
        .args(["-shared", "-o", &so, &obj])
        .output()
        .expect("cc");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let link = start.elapsed().as_secs_f64() * 1000.0;
    let path = CString::new(so).unwrap();
    // SAFETY: the library was just built from the MLIR above, whose C
    // interface wrapper has the CWalk signature.
    let walk = unsafe {
        let h = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        assert!(!h.is_null(), "dlopen failed");
        let name = CString::new("_mlir_ciface_walk").unwrap();
        let sym = libc::dlsym(h, name.as_ptr());
        assert!(!sym.is_null(), "no _mlir_ciface_walk");
        std::mem::transmute::<*mut libc::c_void, CWalk>(sym)
    };
    let stages = Stages {
        mlir_opt,
        translate,
        llvm_opt,
        llc,
        link,
    };
    (walk, stages)
}

pub fn run(f: CWalk, t: &Table, steps: u64, seed: u64) -> (Vec<u64>, u64, f64) {
    let mut m = t.initial.clone();
    let mut e = vec![0u32; t.inputs.len() + 1];
    let (mut dm, mut de) = (Desc::of(&mut m), Desc::of(&mut e));
    let start = Instant::now();
    let done = f(&mut dm, &mut de, steps, seed);
    let ns = start.elapsed().as_nanos() as f64 / done.max(1) as f64;
    (m, done, ns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikam_kernel::net::Net;

    /// Without MLIR installed: every value is defined once, and every block
    /// a branch names exists.
    #[test]
    fn emitted_text_is_well_formed() {
        let net = Net::parse(
            "net t 1\nplace a 2\nplace b 0\ntransition f pure: a -> b\ntransition g pure: b, b -> a\n",
        )
        .unwrap();
        let text = emit(&net.lower().unwrap());
        let mut defs = std::collections::HashSet::new();
        let mut blocks = std::collections::HashSet::new();
        for line in text.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix('^') {
                let (name, args) = rest.split_once(['(', ':']).unwrap();
                assert!(blocks.insert(name.to_string()), "block {name} twice");
                for a in args.split(',').filter_map(|a| a.split(':').next()) {
                    let a = a.trim().trim_start_matches('(');
                    if a.starts_with('%') {
                        assert!(defs.insert(a.to_string()), "{a} twice");
                    }
                }
            } else if let Some((lhs, _)) = line.split_once(" = ") {
                assert!(defs.insert(lhs.to_string()), "{lhs} twice");
            }
        }
        for word in text.split(|c: char| !(c.is_alphanumeric() || "^_%".contains(c))) {
            if let Some(b) = word.strip_prefix('^') {
                assert!(blocks.contains(b), "no block {b}");
            }
        }
        assert!(text.contains("default: ^exit(%s : i64),\n      0: ^f0,\n      1: ^f1\n"));
        println!("{text}");
    }
}
