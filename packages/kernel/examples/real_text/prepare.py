"""E256-E259: turn a sample of PyPI project pages into a real-text search
benchmark for the vector and hybrid examples.

    python3 prepare.py IN.jsonl OUT_DIR

IN.jsonl has one project a line, from PyPI's JSON API (`fetch.py`): name,
summary, description (the README), classifiers and up to six earlier
releases' descriptions. Only projects with an OSI-approved license
classifier are kept. Nothing from the data is committed; OUT_DIR is
scratch.

The model is WordLlama (`wordllama` on PyPI, config l2_supercat, 256
dimensions): a static embedding model, token embeddings taken from
Llama 2 and trained for sentence similarity, mean pooled. Its weights and
tokenizer ship inside the wheel; `WORDLLAMA_DIR` must point at a folder
holding them as `weights/` and `tokenizer/` (see the experiment log).

Writes (little-endian f32, one unit vector a row):
- docs.tsv       key, then the README's paragraphs joined by 0x1e
- doc_whole.f32  each README embedded whole
- doc_mean.f32   the mean of its paragraphs' vectors, renormalized
- frags.tsv / frags.f32  every distinct paragraph and its vector
- queries.tsv    task, qid, split (dev/test), relevant keys (comma), text
- queries.f32
- hist.tsv       key, version index, paragraphs joined by 0x1e
- model.json     the model's measured cost a fragment
"""
import hashlib, json, os, random, re, sys, time

import numpy as np
from wordllama import WordLlama

SRC, OUT = sys.argv[1], sys.argv[2]
RS = "\x1e"
os.makedirs(OUT, exist_ok=True)


def norm(s):
    return re.sub(r"\s+", " ", s).strip()


def paragraphs(desc, drop=None):
    out = []
    for p in re.split(r"\n\s*\n", desc.replace("\r\n", "\n")):
        p = norm(p)
        if p and not (drop and drop in p.lower()):
            out.append(p[:4000])
    return out[:300]


def unit(m):
    return m / np.maximum(np.linalg.norm(m, axis=1, keepdims=True), 1e-9)


projects = [json.loads(l) for l in open(SRC)]
docs, seen = [], set()
for p in projects:
    key = "d/" + re.sub(r"[^a-z0-9]+", "-", p["name"].lower())
    paras = paragraphs(p["description"], norm(p["summary"]).lower())
    if key in seen or len(paras) < 2:
        continue
    seen.add(key)
    docs.append((key, p, paras))
docs.sort(key=lambda d: d[0])
print("docs", len(docs))

wl = WordLlama.load(cache_dir=os.environ["WORDLLAMA_DIR"], disable_download=True)


DIM = len(wl.embed(["warm up"], norm=False)[0])


def emb(texts):
    # Batches pad to their longest text, so embed in length order.
    order = sorted(range(len(texts)), key=lambda i: len(texts[i]))
    out = np.zeros((len(texts), DIM), dtype=np.float32)
    if texts:
        out[order] = unit(np.asarray(wl.embed([texts[i] for i in order], norm=False), dtype=np.float32))
    return out


frag_list = sorted({x for _, _, ps in docs for x in ps})
t = time.perf_counter()
fv = emb(frag_list)
batch_s = time.perf_counter() - t
sample = random.Random(1).sample(frag_list, min(2000, len(frag_list)))
t = time.perf_counter()
for s in sample:
    wl.embed([s], norm=False)
single_s = (time.perf_counter() - t) / len(sample)
fidx = {s: i for i, s in enumerate(frag_list)}

whole = emb(["\n\n".join(ps) for _, _, ps in docs])
mean = unit(np.stack([fv[[fidx[x] for x in ps]].mean(0) for _, _, ps in docs]))

with open(f"{OUT}/docs.tsv", "w") as f:
    for key, _, ps in docs:
        f.write(f"{key}\t{RS.join(ps)}\n")
whole.tofile(f"{OUT}/doc_whole.f32")
mean.tofile(f"{OUT}/doc_mean.f32")
with open(f"{OUT}/frags.tsv", "w") as f:
    for s in frag_list:
        f.write(s + "\n")
fv.tofile(f"{OUT}/frags.f32")

# Task A (known item): a project's summary finds its README.
rng = random.Random(2)
qs = []
for key, p, _ in rng.sample(docs, min(1000, len(docs))):
    qs.append(("known", key, [key], norm(p["summary"])))
# Task B (topic): a Trove topic finds the projects tagged with it.
topics = {}
for key, p, _ in docs:
    for c in p["classifiers"]:
        if c.startswith("Topic :: "):
            topics.setdefault(c, []).append(key)
for c, keys in sorted(topics.items()):
    if 5 <= len(keys) <= 300:
        text = norm(re.sub(r"[:/]+", " ", c[len("Topic :: "):]))
        qs.append(("topic", "t/" + hashlib.sha1(c.encode()).hexdigest()[:10], keys, text))
qv = emb([q[3] for q in qs])
with open(f"{OUT}/queries.tsv", "w") as f:
    for i, (task, qid, rel, text) in enumerate(qs):
        split = "dev" if random.Random(qid).random() < 0.5 else "test"
        f.write(f"{task}\t{qid}\t{split}\t{','.join(rel)}\t{text}\n")
qv.tofile(f"{OUT}/queries.f32")

# Real README histories: each earlier release's description, oldest first.
with open(f"{OUT}/hist.tsv", "w") as f:
    for key, p, _ in docs:
        h = p.get("history") or []
        if len(h) >= 3:
            for i, v in enumerate(h):
                f.write(f"{key}\t{i}\t{RS.join(paragraphs(v['description']))}\n")

json.dump({
    "model": "wordllama l2_supercat 256",
    "dim": int(fv.shape[1]),
    "fragments": len(frag_list),
    "batch_us_per_fragment": batch_s / len(frag_list) * 1e6,
    "single_call_us": single_s * 1e6,
    "queries": {t: sum(1 for q in qs if q[0] == t) for t in ("known", "topic")},
}, open(f"{OUT}/model.json", "w"), indent=1)
print(open(f"{OUT}/model.json").read())
