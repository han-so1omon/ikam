# Real-world corpora

The bench (`examples/bench.rs`) scores the kernel on fixtures and generated
corpora. This directory measures it on real files that nobody made for it:

| Corpus | What it is |
|---|---|
| `docx` | the Word documents python-docx ships as test files (47) |
| `pptx` | the PowerPoint decks python-pptx ships as test files (68) |
| `pdf` | the PDFs pypdf ships as test resources (72) |
| `sheets` | spreadsheets and ODF files from pyexcel-xlsx, python-calamine and odfpy |
| `tables` | vega_datasets' bundled data sets (CSV, JSON, TSV) |
| `requests-wheels` | eight releases of the requests wheel (zip), one snapshot each |
| `click-sdists` | eight releases of the click source archive (tar.gz), one snapshot each |
| `pandas-sdists` | four releases of the pandas source archive (`fetch.py --large`, 17.6 MB) |

Versions are pinned in `fetch.py` and every download is checked against the
sha256 PyPI publishes. Nothing downloaded is run; archives are only
unpacked.

```
python3 -I realworld/fetch.py ~/ikam-corpora
cargo run --release --example realworld -- ~/ikam-corpora/{docx,pptx,pdf,sheets,tables,requests-wheels,click-sdists}
realworld/cli.sh ~/ikam-corpora/pdf      # the same through the ikam CLI
```

`realworld` ingests each corpus into a fresh store (one commit, or one per
version for a release series), repacks, reads every file back byte for byte,
runs fsck, and prints the stored bytes next to zstd 3 with a trained
dictionary (the bench's baseline), zstd 19 per file, and solid zstd 19 and
xz 9 over all distinct files (what `tar | zstd` or `tar | xz` gives, with no
random access). `vs_best` is the store against the smallest of the four.
`store_peak_kib` is the peak resident memory of the store's own run (ingest,
repack, reads, fsck), reset per corpus; `peak_kib` also counts the
baselines.
Results are recorded in `docs/experiments/log.md`.
