"""E224: raw text of blocks compressed after the keyframe block.

python3 -I packages/kernel/scripts/ir_raw_keyframe.py tests/fixtures/cases

Files kept as they are (E198's raw part) in blocks of `per` files: each
block compressed with xz -9 on its own, after block 0 (as a keyframe's raw
text would serve as a dictionary; estimated as xz(block 0 + block) minus
xz(block 0)), or after a sample of every block's first file. Against xz -9
over every file at once.
"""

import lzma
import os
import sys

EXTS = (".md", ".json", ".csv", ".log", ".py", ".txt")


def xz(b):
    return len(lzma.compress(b, preset=9)) if b else 0


def blob(files):
    return b"".join(n.encode() + b"\0" + b for n, b in files)


def main(root):
    files = []
    for d, _, names in os.walk(root):
        for f in names:
            if f.endswith(EXTS):
                p = os.path.join(d, f)
                files.append((os.path.relpath(p, root), open(p, "rb").read()))
    files.sort()
    whole = xz(blob(files))
    print(f"{len(files)} files, {sum(len(b) for _, b in files)} B; xz over all {whole} B")
    for per in (20, 50, 100):
        blocks = [files[i : i + per] for i in range(0, len(files), per)]
        alone = sum(xz(blob(b)) for b in blocks)
        first = blob(blocks[0])
        x0 = xz(first)
        keyed = x0 + sum(xz(first + blob(b)) - x0 for b in blocks[1:])
        spread = b"".join(blob(b[:1]) for b in blocks)[:400_000]
        xs = xz(spread)
        sampled = xs + sum(xz(spread + blob(b)) - xs for b in blocks)
        print(
            f"blocks of {per} ({len(blocks)}): alone {alone / whole:.3f}, "
            f"after block 0 {keyed / whole:.3f}, after a sample {sampled / whole:.3f}"
        )


if __name__ == "__main__":
    main(sys.argv[1])
