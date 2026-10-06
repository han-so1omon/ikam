"""LLM program writer (offline): ask Claude for a program that reproduces a file.

For each target file, Claude sees the stack machine's instruction set, one
worked example (not a benchmark target), and samples of the file. It answers
with an assembly program. The program is assembled, run by the real kernel
(`ikam apply` with `interp.wasm`) and compared byte for byte; on a mismatch,
Claude gets the error or the first differing bytes and tries again. Programs
that reproduce their file exactly are saved as `programs/llm_<name>.asm/.bin`
(and compiled, when possible); `cargo run --release --example programs` then
prices them like any other proposal. Every API call is appended to
`llm_runs.jsonl` with its token counts and cost, and a run stops at its
dollar budget. Nothing here runs during the benchmark.

    export IKAM_ANTHROPIC_API_KEY=...   # a key with a workspace spend limit
    cargo build --release               # builds target/release/ikam
    cargo run --release --example bench -- --dump-generated /tmp/generated
    python3 functions/write_programs.py /tmp/generated/*.txt --budget-usd 3
"""

import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import anthropic

import wasmtime

from asm import OPS, assemble
from compile import compile_program

HERE = Path(__file__).resolve().parent
IKAM = HERE.parent / "target/release/ikam"
MODEL = "claude-opus-5-5"
PRICE = {"input": 4.00, "output": 20.00}  # USD per million tokens, claude-opus-5-5

EXAMPLE = '''; Example: prints "1\\n2\\n...\\n10\\n"
  push 1
  set 0
loop:
  get 0
  emit_dec
  emit_str "\\n"
  get 0
  push 1
  add
  dup
  set 0
  push 11
  lt
  jnz loop
  halt'''

SYSTEM = """You write programs for a small deterministic integer stack machine. A program must \
reproduce a given file exactly, byte for byte, and should be as short as possible: it replaces \
the file in a content-addressed store, so its size is the storage cost.

Assembly syntax: one instruction per line; `; comment`; `label:`; strings in double quotes with \\n \\t \\\\ \\" \
escapes; chars like ' ' or '0' where a byte is expected. Mnemonics and operands:
{syntax}

What each instruction does:
{isa}

Rules:
- Values are signed 64-bit integers; arithmetic wraps. div/mod by zero aborts.
- There is no floating point. Use integer or fixed-point arithmetic.
- Registers r0..r255 start at 0 (`get r`, `set r`). A scratch array is read with `load` \
(pops index) and written with `store` (stack: value then index on top); unset cells read 0.
- Output only through the emit_* instructions. End with `halt`.
- Execution is bounded (roughly 10^8 instructions); prefer efficient algorithms.
- jmp/jz/jnz targets are labels. Keep the operand stack empty at every label and after every \
jump (the program is compiled to WebAssembly, which requires it).

Example program:
{example}

Answer with the program in one ```asm code block and nothing else."""


def isa_text():
    src = (HERE / "interp/src/lib.rs").read_text()
    return "\n".join(l[4:] for l in src.splitlines() if l.startswith("//!   "))


def syntax_text():
    kinds = {"": "", "z": " <integer>", "b": " <byte>", "bb": " <width> <fill byte>",
             "t": " <label>", "s": ' "<string>"', "S": ' "<string 0>" "<string 1>" ... (pops i, emits string i)'}
    return "\n".join(f"  {name}{kinds[k]}" for name, (_, k) in OPS.items()
                     if name not in ("input_len", "input_byte"))


def sample(data: bytes) -> str:
    """What Claude sees of the file: its size and its first and last bytes."""
    text = data.decode("utf-8", errors="strict") if is_text(data) else None
    if text is not None:
        head, tail = text[:3000], text[-600:]
        return (f"The file has {len(data)} bytes (UTF-8 text, {text.count(chr(10))} lines).\n"
                f"First 3000 characters:\n<head>\n{head}\n</head>\n"
                f"Last 600 characters:\n<tail>\n{tail}\n</tail>")
    return (f"The file has {len(data)} bytes (binary). First 512 bytes as hex:\n"
            f"{data[:512].hex(' ')}\nLast 128 bytes as hex:\n{data[-128:].hex(' ')}")


def is_text(data: bytes) -> bool:
    try:
        data.decode("utf-8")
        return True
    except UnicodeDecodeError:
        return False


def run_program(program: bytes) -> bytes:
    """Run `program` through the kernel exactly as a stored derivation would:
    compiled to its own module when possible (fast), else interpreted."""
    try:
        module = wasmtime.wat2wasm(compile_program(program))
    except (ValueError, KeyError, IndexError):
        module = None
    with tempfile.TemporaryDirectory() as store:
        def ikam(*args):
            r = subprocess.run([str(IKAM), "--store", store, *args], capture_output=True, text=False)
            if r.returncode != 0:
                raise RuntimeError(r.stderr.decode(errors="replace").strip() or "failed")
            return r.stdout
        with tempfile.NamedTemporaryFile(suffix=".wasm") as m, tempfile.NamedTemporaryFile(suffix=".bin") as f:
            f.write(program); f.flush()
            if module is not None:
                m.write(module); m.flush()
                out = ikam("apply", ikam("put", m.name).split()[0].decode())
            else:
                interp = ikam("put", str(HERE / "interp.wasm")).split()[0].decode()
                out = ikam("apply", interp, ikam("put", f.name).split()[0].decode())
        return ikam("cat", out.split()[0].decode())


def mismatch(got: bytes, want: bytes) -> str:
    i = next((k for k in range(min(len(got), len(want))) if got[k] != want[k]), min(len(got), len(want)))
    return (f"Your output has {len(got)} bytes; the file has {len(want)}. First difference at byte {i}.\n"
            f"Expected around it: {want[max(0, i - 60):i + 60]!r}\n"
            f"You produced:      {got[max(0, i - 60):i + 60]!r}")


def cost(usage) -> float:
    inp = usage.input_tokens + (usage.cache_creation_input_tokens or 0) + (usage.cache_read_input_tokens or 0)
    return (inp * PRICE["input"] + usage.output_tokens * PRICE["output"]) / 1e6


def write_program(client, path: Path, attempts: int, budget: dict, log) -> bool:
    data = path.read_bytes()
    system = SYSTEM.format(syntax=syntax_text(), isa=isa_text(), example=EXAMPLE)
    messages = [{"role": "user", "content": f"Write a program that reproduces this file exactly.\n\n{sample(data)}"}]
    for attempt in range(1, attempts + 1):
        if budget["spent"] >= budget["limit"]:
            print(f"  budget reached (${budget['spent']:.2f})")
            return False
        try:
            response = client.beta.messages.create(
                model=MODEL,
                max_tokens=16000,
                system=system,
                messages=messages,
                output_config={"effort": "high"},
                betas=["server-side-fallback-2026-07-01"],
                fallbacks="default",
            )
        except anthropic.BadRequestError as e:
            # Includes a workspace spend limit being reached.
            print(f"  request rejected: {e.message}")
            return False
        except anthropic.RateLimitError as e:
            print(f"  rate limited or spend cap reached: {e.message}")
            return False
        spent = cost(response.usage)
        budget["spent"] += spent
        record = {"time": datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds"),
                  "file": path.name, "attempt": attempt, "model": response.model,
                  "input_tokens": response.usage.input_tokens, "output_tokens": response.usage.output_tokens,
                  "cost_usd": round(spent, 4), "stop_reason": response.stop_reason}
        if response.stop_reason == "refusal":
            log({**record, "outcome": "refused"})
            print(f"  attempt {attempt}: declined")
            return False
        text = "".join(b.text for b in response.content if b.type == "text")
        m = re.search(r"```(?:asm)?\n(.*?)```", text, re.S)
        source = m.group(1) if m else text
        try:
            program = assemble(source)
            got = run_program(program)
            feedback = None if got == data else mismatch(got, data)
        except Exception as e:  # assembler or kernel rejected it
            program, feedback = b"", f"The program failed: {e}"
        log({**record, "outcome": "exact" if feedback is None else "mismatch", "program_bytes": len(program)})
        print(f"  attempt {attempt}: {'exact' if feedback is None else 'no'} "
              f"({len(program)} B program, ${spent:.3f}, total ${budget['spent']:.2f})")
        if feedback is None:
            stem = "llm_" + re.sub(r"\W", "_", path.stem)
            (HERE / "programs" / f"{stem}.asm").write_text(source)
            (HERE / "programs" / f"{stem}.bin").write_bytes(program)
            subprocess.run([sys.executable, str(HERE / "compile.py"), str(HERE / "programs" / f"{stem}.bin"),
                            str(HERE / "programs" / f"{stem}.wat")], check=False)
            return True
        messages += [{"role": "assistant", "content": response.content},
                     {"role": "user", "content": f"{feedback}\nFix the program; answer with the whole program again."}]
    return False


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="+", type=Path)
    ap.add_argument("--attempts", type=int, default=3)
    ap.add_argument("--budget-usd", type=float, default=3.0, help="stop this run after spending this much")
    args = ap.parse_args()
    if not IKAM.exists():
        sys.exit("build the kernel first: cargo build --release")
    # The public endpoint, explicitly: an ANTHROPIC_BASE_URL meant for another
    # tool must not receive this key.
    # Its own variable name: ANTHROPIC_API_KEY has a meaning to Claude Code.
    key = os.environ.get("IKAM_ANTHROPIC_API_KEY") or os.environ.get("ANTHROPIC_API_KEY")
    if not key:
        sys.exit("set IKAM_ANTHROPIC_API_KEY (a key from a workspace with a spend limit)")
    client = anthropic.Anthropic(api_key=key, base_url="https://api.anthropic.com")
    budget = {"spent": 0.0, "limit": args.budget_usd}
    with open(HERE / "llm_runs.jsonl", "a") as runs:
        def log(record):
            runs.write(json.dumps(record) + "\n"); runs.flush()
        for path in args.files:
            print(f"{path.name}:")
            write_program(client, path, args.attempts, budget, log)
    print(f"spent ${budget['spent']:.2f} of ${budget['limit']:.2f}")


if __name__ == "__main__":
    main()
