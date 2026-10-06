"""Assembler for the ikam-interp stack machine (see interp/src/lib.rs).

    python3 asm.py program.asm program.bin

One instruction per line; `; comment`; `label:`; strings in double quotes with
\\n, \\t, \\\\ and \\" escapes; jump targets are labels (absolute u16).
"""

import re
import sys

OPS = {
    "halt": (0x00, ""), "push": (0x01, "z"), "add": (0x02, ""), "sub": (0x03, ""),
    "mul": (0x04, ""), "div": (0x05, ""), "mod": (0x06, ""), "and": (0x07, ""),
    "or": (0x08, ""), "xor": (0x09, ""), "shl": (0x0A, ""), "sar": (0x0B, ""),
    "lt": (0x0C, ""), "eq": (0x0D, ""), "not": (0x0E, ""), "shr": (0x0F, ""),
    "dup": (0x10, ""), "swap": (0x11, ""), "drop": (0x12, ""), "over": (0x13, ""),
    "get": (0x14, "b"), "set": (0x15, "b"), "load": (0x16, ""), "store": (0x17, ""),
    "jmp": (0x18, "t"), "jz": (0x19, "t"), "jnz": (0x1A, "t"),
    "emit_byte": (0x20, ""), "emit_dec": (0x21, ""), "emit_pad": (0x22, "bb"),
    "emit_str": (0x23, "s"), "input_len": (0x24, "b"), "input_byte": (0x25, "b"),
    "emit_sel": (0x27, "S"),
}


def varint(n):
    out = bytearray()
    while n >= 0x80:
        out.append(n & 0x7F | 0x80)
        n >>= 7
    out.append(n)
    return bytes(out)


def string(tok):
    body = tok[1:-1].encode().decode("unicode_escape").encode("latin-1")
    return varint(len(body)) + body


def number(tok):
    if tok.startswith("'") and tok.endswith("'"):
        return ord(tok[1:-1].encode().decode("unicode_escape"))
    return int(tok, 0)


def assemble(text):
    lines = []
    for raw in text.splitlines():
        line = re.sub(r";.*$", "", raw).strip() if '"' not in raw else raw.split(" ;")[0].strip()
        if line:
            lines.append(line)
    labels, code, fixups = {}, bytearray(), []
    for line in lines:
        if line.endswith(":"):
            labels[line[:-1]] = len(code)
            continue
        name, _, rest = line.partition(" ")
        op, kinds = OPS[name.lower()]
        toks = re.findall(r'"(?:[^"\\]|\\.)*"|\'(?:[^\'\\]|\\.)\'|\S+', rest)
        code.append(op)
        if kinds == "S":
            code.append(len(toks))
            for t in toks:
                code += string(t)
            continue
        for kind, tok in zip(kinds, toks):
            if kind == "z":
                n = number(tok)
                code += varint((n << 1) ^ (n >> 63) if n < 0 else n << 1)
            elif kind == "b":
                code.append(number(tok) & 0xFF)
            elif kind == "t":
                fixups.append((len(code), tok))
                code += b"\0\0"
            elif kind == "s":
                code += string(tok)
    for at, label in fixups:
        code[at : at + 2] = labels[label].to_bytes(2, "big")
    return bytes(code)


if __name__ == "__main__":
    src, dst = sys.argv[1], sys.argv[2]
    data = assemble(open(src).read())
    open(dst, "wb").write(data)
    print(f"{dst}: {len(data)} bytes")
