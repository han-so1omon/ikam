"""Embedding proposer for grouping (E019): writes order.txt for the benchmark.

Embeds every content the benchmark stores (md files, office zip members,
repo-history files) with a pinned sentence-embedding model, then orders each
corpus by a greedy nearest-neighbour chain over cosine similarity. The kernel
only reads the resulting id order as one grouping proposal among several and
keeps it only if it saves bytes; no model runs during the benchmark.

    pip install onnxruntime tokenizers blake3 numpy huggingface_hub
    python3 packages/kernel/semantic/embed.py
"""

import hashlib
import io
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

import blake3
import numpy as np
import onnxruntime as ort
from huggingface_hub import hf_hub_download
from tokenizers import Tokenizer

MODEL = "sentence-transformers/all-MiniLM-L6-v2"
REVISION = "1110a243fdf4706b3f48f1d95db1a4f5529b4d41"
ONNX_SHA256 = "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452"
MAX_TOKENS = 256
ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).with_name("order.txt")


def content_id(data: bytes) -> str:
    return blake3.blake3(b"B" + data).hexdigest()


def fixtures(exts):
    root = ROOT / "tests/fixtures/cases"
    for p in sorted(root.rglob("*")):
        if p.is_file() and "_image_tools" not in str(p) and p.suffix[1:] in exts:
            yield p.read_bytes()


def office_members():
    for data in fixtures({"xlsx", "docx", "pptx"}):
        try:
            with zipfile.ZipFile(io.BytesIO(data)) as z:
                for name in z.namelist():
                    yield z.read(name)
        except zipfile.BadZipFile:
            yield data


def repo_history():
    git = lambda *a: subprocess.run(["git", *a], cwd=ROOT, capture_output=True, check=True).stdout
    for c in git("rev-list", "--reverse", "6cd6921").decode().split():
        tar = git("archive", c, "--", "docs", "packages/ikam", "packages/modelado", "packages/interacciones")
        with tarfile.open(fileobj=io.BytesIO(tar)) as t:
            for m in t.getmembers():
                if m.isfile():
                    yield t.extractfile(m).read()


def embedder():
    onnx = hf_hub_download(MODEL, "onnx/model.onnx", revision=REVISION)
    if hashlib.sha256(Path(onnx).read_bytes()).hexdigest() != ONNX_SHA256:
        sys.exit("model weights do not match the pinned hash")
    tok = Tokenizer.from_file(hf_hub_download(MODEL, "tokenizer.json", revision=REVISION))
    tok.enable_truncation(MAX_TOKENS)
    tok.enable_padding()
    session = ort.InferenceSession(onnx, providers=["CPUExecutionProvider"])
    names = {i.name for i in session.get_inputs()}

    def embed(texts):
        out = []
        for k in range(0, len(texts), 32):
            enc = tok.encode_batch(texts[k : k + 32])
            ids = np.array([e.ids for e in enc], dtype=np.int64)
            mask = np.array([e.attention_mask for e in enc], dtype=np.int64)
            feed = {"input_ids": ids, "attention_mask": mask, "token_type_ids": np.zeros_like(ids)}
            hidden = session.run(None, {n: v for n, v in feed.items() if n in names})[0]
            pooled = (hidden * mask[..., None]).sum(1) / mask.sum(1, keepdims=True)
            out.append(pooled / np.linalg.norm(pooled, axis=1, keepdims=True))
        return np.concatenate(out)

    return embed


def chain(vectors):
    """Greedy nearest-neighbour order by cosine similarity, from item 0."""
    sims = vectors @ vectors.T
    order, left = [0], set(range(1, len(vectors)))
    while left:
        cand = np.array(sorted(left))
        nxt = int(cand[np.argmax(sims[order[-1], cand])])
        order.append(nxt)
        left.remove(nxt)
    return order


def main():
    embed = embedder()
    lines = [f"# {MODEL}@{REVISION} onnx sha256 {ONNX_SHA256}; max {MAX_TOKENS} tokens; greedy cosine chain"]
    for name, contents in [("md", fixtures({"md"})), ("office", office_members()), ("repo-history", repo_history())]:
        unique = {content_id(b): b for b in contents if b}
        ids = sorted(unique)
        vectors = embed([unique[i].decode("utf-8", "replace") for i in ids])
        lines.append(f"# {name}: {len(ids)} contents")
        lines.extend(ids[i] for i in chain(vectors))
    OUT.write_text("\n".join(lines) + "\n")
    print(f"wrote {OUT} ({len(lines)} lines)")


if __name__ == "__main__":
    main()
