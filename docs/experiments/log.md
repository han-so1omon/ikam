# Experiment log

Newest last. Format and rules: `README.md`.

### E000 baseline
- Branch / parent: feature/rust-kernel at c6cf926 (+ bench score line) / none
- Hypothesis: none; this is the reference point
- Change: none
- Result: score **0.8896**
  - Per corpus `vs_dict`: md 1.408, pdf 1.016, office 1.321, invoices 0.700, synthetic-history 0.308, repo-history 1.218
  - Bytes after repack: md 108,244; pdf 25,043; office 516,710; invoices 9,470; synthetic 18,680; repo 1,572,127
  - Total time 20.7 s
- Verdict: reference
- Open gaps, largest first:
  - md: 1.41. Many tiny files; tree metadata is part of it.
  - office: 1.32. The trained dictionary for the members is only 6.9 KB.
  - repo-history: 1.22.
