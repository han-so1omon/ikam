//! The two kernels built with Cranelift, the dimension (256) fixed at
//! compile time: each dot product is 16 lanes held as four f32x4 sums,
//! fully unrolled, the lanes added in order at the end, so the result is
//! the same bits as `vectors::dot_lanes`.

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, UserFuncName, Value, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

use crate::{AssignFn, D, ScanFn};

fn module() -> JITModule {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    flags.set("use_colocated_libcalls", "false").unwrap();
    flags.set("is_pic", "false").unwrap();
    let isa = cranelift_native::builder()
        .unwrap()
        .finish(settings::Flags::new(flags))
        .unwrap();
    JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()))
}

/// The 16-lane dot product of the `D` f32s at `x` and at `y`.
fn dot16(b: &mut FunctionBuilder, x: Value, y: Value) -> Value {
    let fl = MemFlagsData::new().with_notrap();
    let z = b.ins().f32const(0.0);
    let zv = b.ins().splat(types::F32X4, z);
    let mut acc = [zv; 4];
    for c in 0..D / 16 {
        for (v, a) in acc.iter_mut().enumerate() {
            let off = (c * 64 + v * 16) as i32;
            let xa = b.ins().load(types::F32X4, fl, x, off);
            let ya = b.ins().load(types::F32X4, fl, y, off);
            let p = b.ins().fmul(xa, ya);
            *a = b.ins().fadd(*a, p);
        }
    }
    let mut s = b.ins().extractlane(acc[0], 0u8);
    for l in 1..16 {
        let e = b.ins().extractlane(acc[l / 4], (l % 4) as u8);
        s = b.ins().fadd(s, e);
    }
    s
}

fn finish<F: Copy>(
    mut module: JITModule,
    ctx: &mut cranelift_codegen::Context,
    name: &str,
) -> (JITModule, F) {
    let id = module
        .declare_function(name, Linkage::Export, &ctx.func.signature)
        .unwrap();
    module.define_function(id, ctx).unwrap();
    module.clear_context(ctx);
    module.finalize_definitions().unwrap();
    let ptr = module.get_finalized_function(id);
    assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*const u8>());
    // SAFETY: the function was just compiled with the signature F names.
    let f = unsafe { std::mem::transmute_copy::<*const u8, F>(&ptr) };
    (module, f)
}

/// scan(q, rows, n, out): out[i] = dot(q, row i).
pub fn scan() -> (JITModule, ScanFn) {
    let module = module();
    let mut ctx = module.make_context();
    let i64t = types::I64;
    for _ in 0..4 {
        ctx.func.signature.params.push(AbiParam::new(i64t));
    }
    ctx.func.name = UserFuncName::user(0, 0);
    let tcfg = module.target_config();
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let fl = MemFlagsData::new().with_notrap();
        let (entry, head, body, exit) = (
            b.create_block(),
            b.create_block(),
            b.create_block(),
            b.create_block(),
        );
        b.append_block_params_for_function_params(entry);
        b.append_block_param(head, i64t);
        b.switch_to_block(entry);
        let ps = b.block_params(entry).to_vec();
        let (q, rows, n, out) = (ps[0], ps[1], ps[2], ps[3]);
        let zero = b.ins().iconst(i64t, 0);
        b.ins().jump(head, &[zero.into()]);
        b.switch_to_block(head);
        let i = b.block_params(head)[0];
        let done = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, n);
        b.ins().brif(done, exit, &[], body, &[]);
        b.switch_to_block(body);
        let off = b.ins().imul_imm_u(i, (D * 4) as i64);
        let rp = b.ins().iadd(rows, off);
        let s = dot16(&mut b, q, rp);
        let oo = b.ins().ishl_imm_u(i, 2);
        let op = b.ins().iadd(out, oo);
        b.ins().store(fl, s, op, 0);
        let i1 = b.ins().iadd_imm_s(i, 1);
        b.ins().jump(head, &[i1.into()]);
        b.switch_to_block(exit);
        b.ins().return_(&[]);
        b.seal_all_blocks();
        b.finalize(tcfg);
    }
    finish(module, &mut ctx, "scan")
}

/// assign(rows, n, cents, k, out): out[i] = the last j of the largest
/// dot(cent j, row i).
pub fn assign() -> (JITModule, AssignFn) {
    let module = module();
    let mut ctx = module.make_context();
    let i64t = types::I64;
    for _ in 0..5 {
        ctx.func.signature.params.push(AbiParam::new(i64t));
    }
    ctx.func.name = UserFuncName::user(0, 1);
    let tcfg = module.target_config();
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let fl = MemFlagsData::new().with_notrap();
        let entry = b.create_block();
        let head_i = b.create_block();
        let body_i = b.create_block();
        let head_j = b.create_block();
        let body_j = b.create_block();
        let done_i = b.create_block();
        let exit = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.append_block_param(head_i, i64t);
        b.append_block_param(head_j, i64t);
        b.append_block_param(head_j, types::F32);
        b.append_block_param(head_j, types::I32);
        b.append_block_param(done_i, types::I32);
        b.switch_to_block(entry);
        let ps = b.block_params(entry).to_vec();
        let (rows, n, cents, k, out) = (ps[0], ps[1], ps[2], ps[3], ps[4]);
        let zero = b.ins().iconst(i64t, 0);
        b.ins().jump(head_i, &[zero.into()]);

        b.switch_to_block(head_i);
        let i = b.block_params(head_i)[0];
        let done = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i, n);
        b.ins().brif(done, exit, &[], body_i, &[]);

        b.switch_to_block(body_i);
        let off = b.ins().imul_imm_u(i, (D * 4) as i64);
        let rp = b.ins().iadd(rows, off);
        let ninf = b.ins().f32const(f32::NEG_INFINITY);
        let z32 = b.ins().iconst(types::I32, 0);
        b.ins()
            .jump(head_j, &[zero.into(), ninf.into(), z32.into()]);

        b.switch_to_block(head_j);
        let hp = b.block_params(head_j).to_vec();
        let (j, best, bi) = (hp[0], hp[1], hp[2]);
        let jd = b.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, j, k);
        b.ins().brif(jd, done_i, &[bi.into()], body_j, &[]);

        b.switch_to_block(body_j);
        let co = b.ins().imul_imm_u(j, (D * 4) as i64);
        let cp = b.ins().iadd(cents, co);
        let s = dot16(&mut b, cp, rp);
        let ge = b.ins().fcmp(FloatCC::GreaterThanOrEqual, s, best);
        let nb = b.ins().select(ge, s, best);
        let j32 = b.ins().ireduce(types::I32, j);
        let nbi = b.ins().select(ge, j32, bi);
        let j1 = b.ins().iadd_imm_s(j, 1);
        b.ins().jump(head_j, &[j1.into(), nb.into(), nbi.into()]);

        b.switch_to_block(done_i);
        let r = b.block_params(done_i)[0];
        let oo = b.ins().ishl_imm_u(i, 2);
        let op = b.ins().iadd(out, oo);
        b.ins().store(fl, r, op, 0);
        let i1 = b.ins().iadd_imm_s(i, 1);
        b.ins().jump(head_i, &[i1.into()]);

        b.switch_to_block(exit);
        b.ins().return_(&[]);
        b.seal_all_blocks();
        b.finalize(tcfg);
    }
    finish(module, &mut ctx, "assign")
}
