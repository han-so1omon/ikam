"""Compile an ikam-interp program (bytecode) into its own WASM module (WAT).

    python3 compile.py program.bin program.wat

The interpreter (interp/) runs any program but costs ~1,300 wasmi fuel per op.
A compiled module runs the same ops as native WASM. It is an ordinary
reconstruction function: the kernel verifies its exact output, so it only has
to reproduce the bytes, not mirror the interpreter's every edge case (e.g.
i64::MIN / -1 traps here). Programs whose operand stack is not empty at a
jump target are not compiled (exit status 2); they stay interpreted.
Supported: every op except INPUT_LEN / INPUT_BYTE (no data arguments).
"""

import sys

SCRATCH = 32 << 20  # scratch i64 array base; output must stay below it

FMT = {  # op -> (mnemonic, operand kinds); see interp/src/lib.rs
    0x00: "halt", 0x01: "push", 0x02: "add", 0x03: "sub", 0x04: "mul", 0x05: "div",
    0x06: "mod", 0x07: "and", 0x08: "or", 0x09: "xor", 0x0A: "shl", 0x0B: "sar",
    0x0C: "lt", 0x0D: "eq", 0x0E: "not", 0x0F: "shr", 0x10: "dup", 0x11: "swap",
    0x12: "drop", 0x13: "over", 0x14: "get", 0x15: "set", 0x16: "load", 0x17: "store",
    0x18: "jmp", 0x19: "jz", 0x1A: "jnz", 0x20: "emit_byte", 0x21: "emit_dec",
    0x22: "emit_pad", 0x23: "emit_str", 0x27: "emit_sel",
}
EFFECT = {  # stack (pops, pushes)
    "push": (0, 1), "dup": (1, 2), "swap": (2, 2), "drop": (1, 0), "over": (2, 3),
    "get": (0, 1), "set": (1, 0), "load": (1, 1), "store": (2, 0), "not": (1, 1),
    "jmp": (0, 0), "jz": (1, 0), "jnz": (1, 0), "halt": (0, 0), "emit_byte": (1, 0),
    "emit_dec": (1, 0), "emit_pad": (1, 0), "emit_str": (0, 0), "emit_sel": (1, 0),
}
BINOP = {"add": "i64.add", "sub": "i64.sub", "mul": "i64.mul", "div": "i64.div_s",
         "mod": "i64.rem_s", "and": "i64.and", "or": "i64.or", "xor": "i64.xor",
         "shl": "i64.shl", "sar": "i64.shr_s", "shr": "i64.shr_u"}


def decode(code):
    """[(offset, mnemonic, operands)]"""
    out, pc = [], 0

    def varint():
        nonlocal pc
        n = shift = 0
        while True:
            b = code[pc]; pc += 1
            n |= (b & 0x7F) << shift; shift += 7
            if b < 0x80:
                return n

    def blob():
        nonlocal pc
        n = varint(); s = code[pc:pc + n]; pc += n
        return s

    while pc < len(code):
        at, op = pc, FMT[code[pc]]; pc += 1
        if op == "push":
            z = varint(); args = [(z >> 1) ^ -(z & 1)]
        elif op in ("get", "set"):
            args = [code[pc]]; pc += 1
        elif op in ("jmp", "jz", "jnz"):
            args = [code[pc] << 8 | code[pc + 1]]; pc += 2
        elif op == "emit_pad":
            args = [code[pc], code[pc + 1]]; pc += 2
        elif op == "emit_str":
            args = [blob()]
        elif op == "emit_sel":
            n = code[pc]; pc += 1; args = [blob() for _ in range(n)]
        else:
            args = []
        out.append((at, op, args))
    return out


def blocks(ops):
    """Start offsets of basic blocks; checks the stack is empty at each."""
    starts = {0}
    for i, (at, op, args) in enumerate(ops):
        if op in ("jmp", "jz", "jnz"):
            starts.add(args[0])
        if op in ("jmp", "jz", "jnz", "halt") and i + 1 < len(ops):
            starts.add(ops[i + 1][0])
    depth = 0
    for at, op, args in ops:
        if at in starts and depth != 0:
            raise ValueError(f"stack not empty at block {at}")
        pops, pushes = EFFECT.get(op, (2, 1))
        if depth < pops:
            raise ValueError(f"stack underflow at {at}")
        depth += pushes - pops
        if op in ("jmp", "halt"):
            depth = 0
    return sorted(starts)


def compile_program(code):
    ops, data, strings = decode(code), bytearray(), {}
    starts = blocks(ops)
    index = {s: i for i, s in enumerate(starts)}

    def string(s):
        if s not in strings:
            strings[s] = DATA_BASE + len(data); data.extend(s)
        return strings[s], len(s)

    body = []
    for at, op, args in ops:
        if at in index:
            body.append(f"end ;; block {index[at]}")
        if op == "push":
            body.append(f"i64.const {args[0]}")
        elif op in BINOP:
            body.append(BINOP[op])
        elif op in ("lt", "eq"):
            body += [f"i64.{'lt_s' if op == 'lt' else 'eq'}", "i64.extend_i32_u"]
        elif op == "not":
            body += ["i64.eqz", "i64.extend_i32_u"]
        elif op == "dup":
            body += ["local.tee $t0", "local.get $t0"]
        elif op == "swap":
            body += ["local.set $t0", "local.set $t1", "local.get $t0", "local.get $t1"]
        elif op == "drop":
            body.append("drop")
        elif op == "over":
            body += ["local.set $t0", "local.tee $t1", "local.get $t0", "local.get $t1"]
        elif op == "get":
            body.append(f"local.get $r{args[0]}")
        elif op == "set":
            body.append(f"local.set $r{args[0]}")
        elif op == "load":
            body.append("call $load")
        elif op == "store":
            body += ["local.set $t0", "local.get $t0", "call $store"]
        elif op == "jmp":
            body += [f"i32.const {index[args[0]]}", "local.set $pc", "br $dispatch"]
        elif op in ("jz", "jnz"):
            test = "i64.eqz" if op == "jz" else "i64.const 0\ni64.ne"
            body += [test, f"if", f"i32.const {index[args[0]]}", "local.set $pc",
                     "br $dispatch", "end"]
        elif op == "halt":
            body.append("br $exit")
        elif op == "emit_byte":
            body.append("call $emit_byte")
        elif op == "emit_dec":
            body += ["i32.const 0", "i32.const 32", "call $emit_pad"]
        elif op == "emit_pad":
            body += [f"i32.const {args[0]}", f"i32.const {args[1]}", "call $emit_pad"]
        elif op == "emit_str":
            p, n = string(args[0])
            body += [f"i32.const {p}", f"i32.const {n}", "call $emit_bytes"]
        elif op == "emit_sel":
            body.append("local.set $t0")
            for k, s in enumerate(args):
                p, n = string(s)
                body += ["local.get $t0", f"i64.const {k}", "i64.eq", "if",
                         f"i32.const {p}", f"i32.const {n}", "call $emit_bytes", "end"]
    body.append("br $exit")
    nblocks = len(starts)
    opening = [f"block $b{i}" for i in reversed(range(nblocks))]
    table = " ".join(f"$b{i}" for i in range(nblocks))
    dispatch = (["block $exit", "loop $dispatch"] + opening
                + [f"local.get $pc", f"br_table {table} $b0", "end ;; dispatch table"])
    # The first "end ;; block 0" closes block $b0 (its code follows);
    # drop the one emitted at offset 0, which the table's end replaces.
    body.remove("end ;; block 0")
    regs = " ".join(f"(local $r{i} i64)" for i in range(256))
    data_wat = "".join(f"\\{b:02x}" for b in data)
    return RUNTIME.format(regs=regs, body="\n    ".join(dispatch + body + ["end", "end"]),
                          data=data_wat, data_base=DATA_BASE, scratch=SCRATCH)


DATA_BASE = 1024
RUNTIME = """(module
  (memory (export "memory") 1)
  (data (i32.const {data_base}) "{data}")
  (global $heap (mut i32) (i32.const 65536))
  (global $out (mut i32) (i32.const 0))
  (global $out0 (mut i32) (i32.const 0))
  (func $ensure (param $end i32)
    (local $have i32)
    (local.set $have (i32.shl (memory.size) (i32.const 16)))
    (if (i32.gt_u (local.get $end) (local.get $have))
      (then (if (i32.lt_s (memory.grow (i32.shr_u (i32.add (i32.sub (local.get $end) (local.get $have)) (i32.const 65535)) (i32.const 16))) (i32.const 0))
        (then unreachable)))))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (local.get $p) (local.get $n)))
    (call $ensure (global.get $heap))
    (local.get $p))
  (func $emit_byte (param $v i64)
    (if (i32.ge_u (global.get $out) (i32.const {scratch})) (then unreachable))
    (call $ensure (i32.add (global.get $out) (i32.const 1)))
    (i64.store8 (global.get $out) (local.get $v))
    (global.set $out (i32.add (global.get $out) (i32.const 1))))
  (func $emit_bytes (param $p i32) (param $n i32)
    (if (i32.ge_u (i32.add (global.get $out) (local.get $n)) (i32.const {scratch})) (then unreachable))
    (call $ensure (i32.add (global.get $out) (local.get $n)))
    (memory.copy (global.get $out) (local.get $p) (local.get $n))
    (global.set $out (i32.add (global.get $out) (local.get $n))))
  (func $emit_pad (param $v i64) (param $w i32) (param $f i32)
    (local $u i64) (local $i i32) (local $len i32)
    ;; digits of |v| into 512..532, right to left
    (local.set $u (select (i64.sub (i64.const 0) (local.get $v)) (local.get $v) (i64.lt_s (local.get $v) (i64.const 0))))
    (local.set $i (i32.const 532))
    (loop $d
      (local.set $i (i32.sub (local.get $i) (i32.const 1)))
      (i64.store8 (local.get $i) (i64.add (i64.const 48) (i64.rem_u (local.get $u) (i64.const 10))))
      (local.set $u (i64.div_u (local.get $u) (i64.const 10)))
      (br_if $d (i64.ne (local.get $u) (i64.const 0))))
    (local.set $len (i32.add (i32.sub (i32.const 532) (local.get $i)) (i64.lt_s (local.get $v) (i64.const 0))))
    (block $done (loop $p
      (br_if $done (i32.ge_s (local.get $len) (local.get $w)))
      (call $emit_byte (i64.extend_i32_u (local.get $f)))
      (local.set $w (i32.sub (local.get $w) (i32.const 1)))
      (br $p)))
    (if (i64.lt_s (local.get $v) (i64.const 0)) (then (call $emit_byte (i64.const 45))))
    (call $emit_bytes (local.get $i) (i32.sub (i32.const 532) (local.get $i))))
  (func $addr (param $i i64) (result i32)
    (if (i64.ge_u (local.get $i) (i64.const 4194304)) (then unreachable))
    (i32.add (i32.const {scratch}) (i32.wrap_i64 (i64.shl (local.get $i) (i64.const 3)))))
  (func $load (param $i i64) (result i64)
    (local $a i32)
    (local.set $a (call $addr (local.get $i)))
    (if (result i64) (i32.lt_u (local.get $a) (i32.shl (memory.size) (i32.const 16)))
      (then (i64.load (local.get $a))) (else (i64.const 0))))
  (func $store (param $v i64) (param $i i64)
    (local $a i32)
    (local.set $a (call $addr (local.get $i)))
    (call $ensure (i32.add (local.get $a) (i32.const 8)))
    (i64.store (local.get $a) (local.get $v)))
  (func (export "run") (param $ptr i32) (param $len i32) (result i64)
    (local $pc i32) (local $t0 i64) (local $t1 i64) {regs}
    (global.set $out0 (global.get $heap))
    (global.set $out (global.get $heap))
    {body}
    (i64.or (i64.shl (i64.extend_i32_u (global.get $out0)) (i64.const 32))
            (i64.extend_i32_u (i32.sub (global.get $out) (global.get $out0))))))
"""


if __name__ == "__main__":
    try:
        wat = compile_program(open(sys.argv[1], "rb").read())
    except (ValueError, KeyError) as e:
        print(f"{sys.argv[1]}: not compiled ({e})")
        sys.exit(2)
    open(sys.argv[2], "w").write(wat)
    print(f"{sys.argv[2]}: {len(wat)} chars of WAT")
