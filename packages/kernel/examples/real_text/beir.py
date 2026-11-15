"""E304: turn a BEIR dataset (human relevance judgments) into the files
`prepare.py` writes, so the real-text harnesses run on it unchanged.

    python3 beir.py BEIR_DIR OUT_DIR [MODEL]

BEIR_DIR is one unzipped BEIR dataset (`corpus.jsonl`, `queries.jsonl`,
`qrels/*.tsv`), downloaded by hand; nothing from it is committed. MODEL as
in `prepare.py`.

- docs.tsv: key `d/<corpus id>`, then the title and the text's
  paragraphs joined by 0x1e.
- doc_whole.f32: title and text embedded together (BEIR's usual input).
- doc_mean.f32: the mean of the paragraphs' vectors, renormalized.
- queries.tsv: task (the dataset's name), qid, split, judged keys as
  `key=grade` (grades above 0 only), text. Split `test` is BEIR's test
  qrels; `dev` is its dev qrels, or train where there is no dev (SciFact),
  at most 1,000 queries (seeded sample), for tuning.
- frags.tsv / frags.f32 are left empty and hist.tsv is not written: BEIR
  has no versions.
"""
import csv, json, os, random, re, sys, time

import numpy as np

import embedders

SRC, OUT = sys.argv[1], sys.argv[2]
MODEL = embedders.load(sys.argv[3] if len(sys.argv) > 3 else "wordllama")
NAME = os.path.basename(os.path.normpath(SRC))
RS = "\x1e"
os.makedirs(OUT, exist_ok=True)


def norm(s):
    return re.sub(r"\s+", " ", s).strip()


corpus = [json.loads(l) for l in open(f"{SRC}/corpus.jsonl")]
corpus.sort(key=lambda d: d["_id"])
docs = []
for d in corpus:
    title, text = norm(d.get("title") or ""), d.get("text") or ""
    paras = [norm(p) for p in re.split(r"\n\s*\n", text) if norm(p)]
    docs.append(("d/" + d["_id"], ([title] if title else []) + paras, (title + " " + norm(text)).strip()))
print("docs", len(docs), flush=True)

t = time.perf_counter()
whole = MODEL.docs([w for _, _, w in docs])
batch_s = time.perf_counter() - t
frag_list = sorted({p for _, ps, _ in docs for p in ps})
fv = MODEL.docs(frag_list)
fidx = {s: i for i, s in enumerate(frag_list)}
mean = embedders.unit(np.stack([
    fv[[fidx[p] for p in ps]].mean(0) if ps else np.zeros(MODEL.dim, dtype=np.float32) for _, ps, _ in docs
]))
sample = random.Random(1).sample([w for _, _, w in docs], min(500, len(docs)))
t = time.perf_counter()
for s in sample:
    MODEL.one(s)
single_s = (time.perf_counter() - t) / len(sample)

with open(f"{OUT}/docs.tsv", "w") as f:
    for key, ps, _ in docs:
        f.write(f"{key}\t{RS.join(ps)}\n")
whole.astype("<f4").tofile(f"{OUT}/doc_whole.f32")
mean.astype("<f4").tofile(f"{OUT}/doc_mean.f32")
open(f"{OUT}/frags.tsv", "w").close()
open(f"{OUT}/frags.f32", "wb").close()

text = {}
for l in open(f"{SRC}/queries.jsonl"):
    q = json.loads(l)
    text[q["_id"]] = norm(q["text"])


def qrels(split):
    p = f"{SRC}/qrels/{split}.tsv"
    if not os.path.exists(p):
        return None
    out = {}
    with open(p) as f:
        r = csv.reader(f, delimiter="\t")
        next(r)
        for qid, did, score in r:
            if int(score) > 0:
                out.setdefault(qid, {})[did] = int(score)
    return out


test = qrels("test")
tune_from = "dev" if qrels("dev") is not None else "train"
tune = qrels(tune_from) or {}
tune_ids = sorted(q for q in tune if q not in test)
tune_ids = sorted(random.Random(3).sample(tune_ids, min(1000, len(tune_ids))))
qs = [(qid, "test", test[qid]) for qid in sorted(test)] + [(qid, "dev", tune[qid]) for qid in tune_ids]
qs = [q for q in qs if q[0] in text]
qv = MODEL.queries([text[q[0]] for q in qs])
with open(f"{OUT}/queries.tsv", "w") as f:
    for qid, split, rel in qs:
        judged = ",".join(f"d/{d}={g}" for d, g in sorted(rel.items()))
        f.write(f"{NAME}\t{qid}\t{split}\t{judged}\t{text[qid]}\n")
qv.astype("<f4").tofile(f"{OUT}/queries.f32")

json.dump({
    "model": MODEL.name,
    "dim": int(whole.shape[1]),
    "dataset": NAME,
    "docs": len(docs),
    "fragments": len(frag_list),
    "batch_us_per_doc": batch_s / len(docs) * 1e6,
    "single_call_us": single_s * 1e6,
    "queries": {"test": sum(1 for q in qs if q[1] == "test"), "dev": sum(1 for q in qs if q[1] == "dev")},
    "dev_from": tune_from,
}, open(f"{OUT}/model.json", "w"), indent=1)
print(open(f"{OUT}/model.json").read())
