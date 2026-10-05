# Research notes

Literature that informs the kernel, read before experiments are designed (`docs/experiments/README.md`, step 2).

## How to read

- **Find:** `orx discover keyword "<query>"` and `orx discover embedding "<query>"` search arXiv full text through alphaXiv. `orx discover openalex "<query>"` covers all venues, but is rate-limited, so use it sparingly.
- **Read:** `orx paper <arxiv-id> [--full]`, `https://arxiv.org/html/<id>`, or the venue's page or PDF.
- **Cache** anything fetched locally rather than re-querying.

## Note format (`<topic>.md`)

- The question, tied to a benchmark gap or a design decision.
- **Papers read.** For each: citation and id/URL; whether you read the abstract only or the full text; the specific claim or technique, with the paper's numbers; how it maps onto the kernel.
- **Hypotheses.** Each names its source paper, the code change, the expected effect per corpus, and its status: `open`, `confirmed by E<nnn>`, `refuted by E<nnn>`, or `not applicable`.
- **Unknowns**, with inferences marked as such.

A paper's claim stays *unverified for this kernel* until an experiment reproduces it on our benchmark.

## Notes

- `office-gap.md`: why office, md and repo-history lose to zstd+dict (framing, dictionary sizing, deltas, solid regions, accounting).
- `kernel-direction.md`: library learning, program-synthesis compression and e-graph extraction, mapped onto templates, claims and repack.
- `tree-metadata.md`: how git, Mercurial and dedup systems (Meister et al., FAST 2013) store versioned listings and fingerprint lists compactly.
- `graph-storage.md`: graph stores compared (HugeGraph, Rama, TerminusDB, Dolt/Noms prolly trees, Datomic/XTDB, HDT, k²-trees, WebGraph, Kùzu, Neo4j) for a versioned, sliceable, deduplicating graph layer.
- `semantic-grouping.md`: which signal (byte order, MinHash/seeds, NCD, embeddings, LLM) should decide which small contents are compressed together.
