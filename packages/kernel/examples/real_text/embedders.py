"""The embedding models the real-text harnesses can use (E304):

- `wordllama`: WordLlama l2_supercat, 256 dimensions, a static model (E256
  to E259). `WORDLLAMA_DIR` names a folder holding the wheel's
  `weights/` and `tokenizers/` (links will do), loaded with downloads off.
- `minilm`: sentence-transformers all-MiniLM-L6-v2, 384 dimensions, a
  6-layer transformer, mean pooled; reads at most 256 tokens of a text.
- `bge-small`: BAAI bge-small-en-v1.5, 384 dimensions, CLS pooled; reads
  at most 512 tokens. Queries get the model's retrieval instruction.

The transformer models come from Hugging Face through `sentence-transformers`
(pip). They run on Apple's GPU (MPS) when there is one, else the CPU.

`load(name)` returns an object with `name`, `dim`, `docs(texts)` and
`queries(texts)` (unit f32 rows) and `one(text)` (a single call, to time).
"""
import os

import numpy as np

MODELS = ("wordllama", "minilm", "bge-small")


def unit(m):
    return m / np.maximum(np.linalg.norm(m, axis=1, keepdims=True), 1e-9)


class WordLlamaModel:
    def __init__(self):
        from wordllama import WordLlama

        self.wl = WordLlama.load(cache_dir=os.environ["WORDLLAMA_DIR"], disable_download=True)
        self.name = "wordllama l2_supercat 256"
        self.device = "cpu"
        self.dim = len(self.wl.embed(["warm up"], norm=False)[0])

    def docs(self, texts):
        # Batches pad to their longest text, so embed in length order.
        order = sorted(range(len(texts)), key=lambda i: len(texts[i]))
        out = np.zeros((len(texts), self.dim), dtype=np.float32)
        if texts:
            out[order] = unit(np.asarray(self.wl.embed([texts[i] for i in order], norm=False), dtype=np.float32))
        return out

    queries = docs

    def one(self, text):
        self.wl.embed([text], norm=False)


class SentenceModel:
    REPO = {
        "minilm": ("sentence-transformers/all-MiniLM-L6-v2", ""),
        "bge-small": ("BAAI/bge-small-en-v1.5", "Represent this sentence for searching relevant passages: "),
    }

    def __init__(self, key):
        import torch
        from sentence_transformers import SentenceTransformer

        repo, self.prefix = self.REPO[key]
        self.device = "mps" if torch.backends.mps.is_available() else "cpu"
        self.m = SentenceTransformer(repo, device=self.device)
        self.name = f"{repo} (max {self.m.max_seq_length} tokens, {self.device})"
        self.dim = self.m.get_embedding_dimension() if hasattr(self.m, "get_embedding_dimension") else self.m.get_sentence_embedding_dimension()

    def _enc(self, texts):
        if not texts:
            return np.zeros((0, self.dim), dtype=np.float32)
        # encode() sorts by length itself.
        v = self.m.encode(texts, batch_size=128, normalize_embeddings=True, convert_to_numpy=True)
        return np.asarray(v, dtype=np.float32)

    def docs(self, texts):
        return self._enc(texts)

    def queries(self, texts):
        return self._enc([self.prefix + t for t in texts])

    def one(self, text):
        self.m.encode([text], normalize_embeddings=True)


def load(name):
    if name == "wordllama":
        return WordLlamaModel()
    if name in SentenceModel.REPO:
        return SentenceModel(name)
    raise SystemExit(f"model must be one of {MODELS}")
