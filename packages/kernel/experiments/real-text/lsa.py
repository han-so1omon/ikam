"""LSA vectors for the real-text benchmark: tf-idf (sublinear tf) over the
30,000 most common terms in 3+ docs, randomized SVD to 256 dims; documents
are U*S rows, queries folded in through V. Writes doc_whole.f32 and
queries.f32 (unit rows) next to docs.tsv and queries.tsv in DIR."""
import re, sys, numpy as np
D = sys.argv[1]; K = 256
tok = lambda s: re.findall(r"[a-z0-9]+", s.lower())
docs = [l.rstrip("\n").split("\t", 1)[1].replace("\x1e", "\n") for l in open(f"{D}/docs.tsv")]
qs = [l.rstrip("\n").split("\t")[4] for l in open(f"{D}/queries.tsv")]
dt = [tok(d) for d in docs]
df = {}
for t in dt:
    for w in set(t): df[w] = df.get(w, 0) + 1
vocab = [w for w, c in sorted(df.items(), key=lambda x: -x[1]) if c >= 3][:30000]
ix = {w: i for i, w in enumerate(vocab)}
n = len(docs)
idf = np.array([np.log(n / df[w]) for w in vocab], dtype=np.float32)
def vec(tokens):
    v = np.zeros(len(vocab), dtype=np.float32)
    for w in tokens:
        j = ix.get(w)
        if j is not None: v[j] += 1
    nz = v > 0; v[nz] = 1 + np.log(v[nz]); v *= idf
    return v / max(np.linalg.norm(v), 1e-9)
X = np.stack([vec(t) for t in dt])
rng = np.random.default_rng(0)
Y = X @ rng.standard_normal((X.shape[1], K + 20), dtype=np.float32)
for _ in range(2): Y = X @ (X.T @ Y)
Q, _ = np.linalg.qr(Y)
B = Q.T @ X
Ub, S, Vt = np.linalg.svd(B, full_matrices=False)
U = (Q @ Ub)[:, :K]; S = S[:K]; Vt = Vt[:K]
unit = lambda m: (m / np.maximum(np.linalg.norm(m, axis=1, keepdims=True), 1e-9)).astype("<f4")
unit(U * S).tofile(f"{D}/doc_whole.f32")
unit(np.stack([vec(tok(q)) for q in qs]) @ Vt.T).tofile(f"{D}/queries.f32")
print("docs", n, "vocab", len(vocab), "dim", K, "top singular", float(S[0]), "256th", float(S[-1]))
