//! The kernels in MLIR, lowered with MLIR's and LLVM's tools (found in
//! `MLIR_BIN`); none of them is a build dependency.
//!
//! - `scan` and `assign`: the vector dialect, 16 lanes (`vector<16xf32>`)
//!   and a `vector.reduction` at the end, as `vectors::dot_lanes`.
//! - `scores`: k-means scores as one `linalg.generic` over (row, l, cent),
//!   so the loops run with the centroid innermost over the transposed
//!   centroids; each score is summed in order over l, as `vectors::dot`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::D;

/// A memref as MLIR's C interface passes it.
#[repr(C)]
pub struct Desc1<T> {
    pub allocated: *const T,
    pub aligned: *const T,
    pub offset: i64,
    pub size: i64,
    pub stride: i64,
}

#[repr(C)]
pub struct Desc2<T> {
    pub allocated: *const T,
    pub aligned: *const T,
    pub offset: i64,
    pub sizes: [i64; 2],
    pub strides: [i64; 2],
}

pub fn d1<T>(p: *const T, len: usize) -> Desc1<T> {
    Desc1 {
        allocated: p,
        aligned: p,
        offset: 0,
        size: len as i64,
        stride: 1,
    }
}

pub fn d2<T>(p: *const T, rows: usize, cols: usize) -> Desc2<T> {
    Desc2 {
        allocated: p,
        aligned: p,
        offset: 0,
        sizes: [rows as i64, cols as i64],
        strides: [cols as i64, 1],
    }
}

pub type MScan = unsafe extern "C" fn(*mut Desc1<f32>, *mut Desc1<f32>, *mut Desc1<f32>);
pub type MAssign = unsafe extern "C" fn(*mut Desc1<f32>, *mut Desc1<f32>, *mut Desc1<u32>);
pub type MScores = unsafe extern "C" fn(*mut Desc2<f32>, *mut Desc2<f32>, *mut Desc2<f32>);

pub fn emit() -> String {
    format!(
        r#"#ma = affine_map<(i, l, j) -> (i, l)>
#mb = affine_map<(i, l, j) -> (l, j)>
#mc = affine_map<(i, l, j) -> (i, j)>
module {{
  func.func @scan(%q: memref<?xf32>, %r: memref<?xf32>, %out: memref<?xf32>) attributes {{llvm.emit_c_interface}} {{
    %c0 = arith.constant 0 : index
    %c1 = arith.constant 1 : index
    %c16 = arith.constant 16 : index
    %cd = arith.constant {D} : index
    %z = arith.constant dense<0.0> : vector<16xf32>
    %n = memref.dim %out, %c0 : memref<?xf32>
    scf.for %i = %c0 to %n step %c1 {{
      %base = arith.muli %i, %cd : index
      %acc = scf.for %l = %c0 to %cd step %c16 iter_args(%a = %z) -> (vector<16xf32>) {{
        %qv = vector.load %q[%l] : memref<?xf32>, vector<16xf32>
        %o = arith.addi %base, %l : index
        %rv = vector.load %r[%o] : memref<?xf32>, vector<16xf32>
        %p = arith.mulf %qv, %rv : vector<16xf32>
        %s = arith.addf %a, %p : vector<16xf32>
        scf.yield %s : vector<16xf32>
      }}
      %t = vector.reduction <add>, %acc : vector<16xf32> into f32
      memref.store %t, %out[%i] : memref<?xf32>
    }}
    return
  }}
  func.func @assign(%r: memref<?xf32>, %c: memref<?xf32>, %out: memref<?xi32>) attributes {{llvm.emit_c_interface}} {{
    %c0 = arith.constant 0 : index
    %c1 = arith.constant 1 : index
    %c16 = arith.constant 16 : index
    %cd = arith.constant {D} : index
    %z = arith.constant dense<0.0> : vector<16xf32>
    %ninf = arith.constant 0xFF800000 : f32
    %i0 = arith.constant 0 : i32
    %n = memref.dim %out, %c0 : memref<?xi32>
    %cl = memref.dim %c, %c0 : memref<?xf32>
    %k = arith.divui %cl, %cd : index
    scf.for %i = %c0 to %n step %c1 {{
      %rb = arith.muli %i, %cd : index
      %res:2 = scf.for %j = %c0 to %k step %c1 iter_args(%b = %ninf, %bi = %i0) -> (f32, i32) {{
        %cb = arith.muli %j, %cd : index
        %acc = scf.for %l = %c0 to %cd step %c16 iter_args(%a = %z) -> (vector<16xf32>) {{
          %o1 = arith.addi %cb, %l : index
          %cv = vector.load %c[%o1] : memref<?xf32>, vector<16xf32>
          %o2 = arith.addi %rb, %l : index
          %rv = vector.load %r[%o2] : memref<?xf32>, vector<16xf32>
          %p = arith.mulf %cv, %rv : vector<16xf32>
          %s = arith.addf %a, %p : vector<16xf32>
          scf.yield %s : vector<16xf32>
        }}
        %t = vector.reduction <add>, %acc : vector<16xf32> into f32
        %ge = arith.cmpf oge, %t, %b : f32
        %jc = arith.index_cast %j : index to i32
        %nb = arith.select %ge, %t, %b : f32
        %nbi = arith.select %ge, %jc, %bi : i32
        scf.yield %nb, %nbi : f32, i32
      }}
      memref.store %res#1, %out[%i] : memref<?xi32>
    }}
    return
  }}
  func.func @scores(%a: memref<?x{D}xf32>, %b: memref<{D}x?xf32>, %o: memref<?x?xf32>) attributes {{llvm.emit_c_interface}} {{
    linalg.generic {{indexing_maps = [#ma, #mb, #mc], iterator_types = ["parallel", "reduction", "parallel"]}} ins(%a, %b : memref<?x{D}xf32>, memref<{D}x?xf32>) outs(%o : memref<?x?xf32>) {{
    ^bb0(%x: f32, %y: f32, %acc: f32):
      %p = arith.mulf %x, %y : f32
      %s = arith.addf %acc, %p : f32
      linalg.yield %s : f32
    }}
    return
  }}
}}
"#
    )
}

const PASSES: &[&str] = &[
    "--convert-linalg-to-loops",
    "--lower-affine",
    "--canonicalize",
    "--cse",
    "--convert-scf-to-cf",
    "--convert-vector-to-llvm",
    "--expand-strided-metadata",
    "--finalize-memref-to-llvm",
    "--convert-arith-to-llvm",
    "--convert-index-to-llvm",
    "--convert-func-to-llvm",
    "--convert-cf-to-llvm",
    "--reconcile-unrealized-casts",
];

pub fn tool(bin: &str, name: &str) -> PathBuf {
    PathBuf::from(bin).join(name)
}

fn run(cmd: &mut Command) -> f64 {
    let t = Instant::now();
    let out = cmd.output().expect("tool");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    t.elapsed().as_secs_f64() * 1e3
}

/// E342: the module lowered to LLVM IR at `out`, for any target's llc.
pub fn lower(bin: &str, out: &Path) {
    let dir = tempfile::tempdir().unwrap();
    let (src, low) = (dir.path().join("k.mlir"), dir.path().join("low.mlir"));
    std::fs::write(&src, emit()).unwrap();
    run(Command::new(tool(bin, "mlir-opt"))
        .args(PASSES)
        .arg(&src)
        .arg("-o")
        .arg(&low));
    run(Command::new(tool(bin, "mlir-translate"))
        .arg("--mlir-to-llvmir")
        .arg(&low)
        .arg("-o")
        .arg(out));
}

/// The module built into a shared library: (library, stage times in ms).
pub fn build(bin: &str, dir: &Path, llc_cpu: Option<&str>) -> (PathBuf, Vec<(String, f64)>) {
    let src = dir.join("k.mlir");
    std::fs::write(&src, emit()).unwrap();
    let (low, ll, opt_ll, obj, so) = (
        dir.join("low.mlir"),
        dir.join("k.ll"),
        dir.join("k.opt.ll"),
        dir.join("k.o"),
        dir.join("mlir.so"),
    );
    let mut st = Vec::new();
    st.push((
        "mlir_opt".to_string(),
        run(Command::new(tool(bin, "mlir-opt"))
            .args(PASSES)
            .arg(&src)
            .arg("-o")
            .arg(&low)),
    ));
    st.push((
        "translate".to_string(),
        run(Command::new(tool(bin, "mlir-translate"))
            .arg("--mlir-to-llvmir")
            .arg(&low)
            .arg("-o")
            .arg(&ll)),
    ));
    st.push((
        "opt_O3".to_string(),
        run(Command::new(tool(bin, "opt"))
            .args(["-O3", "-S"])
            .arg(&ll)
            .arg("-o")
            .arg(&opt_ll)),
    ));
    let mut llc = Command::new(tool(bin, "llc"));
    llc.args(["-O3", "-filetype=obj", "-relocation-model=pic"]);
    if let Some(cpu) = llc_cpu {
        llc.arg(format!("-mcpu={cpu}"));
    }
    st.push((
        "llc_O3".to_string(),
        run(llc.arg(&opt_ll).arg("-o").arg(&obj)),
    ));
    st.push((
        "link".to_string(),
        run(Command::new("cc")
            .arg("-shared")
            .arg("-o")
            .arg(&so)
            .arg(&obj)),
    ));
    (so, st)
}
