"""Fetch the real-world corpora into a cache directory (default ~/ikam-corpora).

Every corpus is real files, not generated ones: test documents that Python
office/PDF libraries ship in their source releases on PyPI, real data sets,
real release series of a package, and local toolchain docs and sources.
Downloads come from PyPI's JSON API and are checked against its sha256.
Nothing downloaded is executed: archives are only unpacked (data filter).

Usage: python3 -I fetch.py [CACHE_DIR] [--large]
(--large adds a scale corpus: four pandas source releases, about 90 MB once
expanded.)
"""

import hashlib
import io
import json
import shutil
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
CACHE = Path(ARGS[0] if ARGS else Path.home() / "ikam-corpora")

# (corpus, package, version, archive kind, extensions to keep)
DOCUMENTS = [
    ("docx", "python-docx", "1.2.0", "sdist", (".docx",)),
    ("pptx", "python-pptx", "1.0.2", "sdist", (".pptx",)),
    ("pdf", "pypdf", "7.0.0", "sdist", (".pdf",)),
    ("sheets", "pyexcel-xlsx", "0.6.1", "sdist", (".xlsx", ".xls")),
    ("sheets", "python-calamine", "0.9.1", "sdist", (".xlsx", ".xls", ".ods")),
    ("sheets", "odfpy", "1.4.1", "sdist", (".ods", ".odt")),
    ("tables", "vega_datasets", "0.9.0", "bdist_wheel", (".csv", ".json", ".tsv")),
]
# Release series: one snapshot per version, files kept as released.
SERIES = [
    ("requests-wheels", "requests", "bdist_wheel",
     ["2.25.1", "2.26.0", "2.27.1", "2.28.2", "2.29.0", "2.30.0", "2.31.0", "2.32.3"]),
    ("click-sdists", "click", "sdist",
     ["8.0.4", "8.1.0", "8.1.3", "8.1.5", "8.1.7", "8.1.8", "8.2.0", "8.2.1"]),
]

LARGE = [
    ("pandas-sdists", "pandas", "sdist", ["2.2.0", "2.2.1", "2.2.2", "2.2.3"]),
]


def release_file(pkg: str, version: str, kind: str) -> tuple[str, bytes]:
    meta = json.load(urllib.request.urlopen(f"https://pypi.org/pypi/{pkg}/{version}/json", timeout=60))
    files = [f for f in meta["urls"] if f["packagetype"] == kind]
    # Prefer a pure-python wheel when there are several.
    files.sort(key=lambda f: ("none-any" not in f["filename"], f["filename"]))
    f = files[0]
    data = urllib.request.urlopen(f["url"], timeout=300).read()
    if hashlib.sha256(data).hexdigest() != f["digests"]["sha256"]:
        raise SystemExit(f"sha256 mismatch for {f['filename']}")
    return f["filename"], data


def members(name: str, data: bytes):
    if name.endswith(".whl") or name.endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(data)) as z:
            for info in z.infolist():
                if not info.is_dir():
                    yield info.filename, z.read(info)
    else:
        with tarfile.open(fileobj=io.BytesIO(data)) as t:
            for m in t.getmembers():
                m = tarfile.data_filter(m, "/")
                if m.isfile():
                    yield m.name, t.extractfile(m).read()


def safe(path: str) -> str:
    parts = [p for p in Path(path).parts if p not in ("..", "/", "")]
    return "/".join(parts)


def main() -> None:
    raw = CACHE / "raw"
    raw.mkdir(parents=True, exist_ok=True)
    manifest = []
    for corpus, pkg, version, kind, exts in DOCUMENTS:
        name, data = release_file(pkg, version, kind)
        kept = 0
        for path, body in members(name, data):
            if path.lower().endswith(exts) and body:
                out = CACHE / corpus / pkg / safe(path).split("/", 1)[-1]
                out.parent.mkdir(parents=True, exist_ok=True)
                out.write_bytes(body)
                kept += 1
        manifest.append(f"{corpus}\t{pkg}=={version}\t{name}\t{kept} files")
        print(manifest[-1])
    for corpus, pkg, kind, versions in SERIES + (LARGE if "--large" in sys.argv else []):
        for version in versions:
            name, data = release_file(pkg, version, kind)
            out = CACHE / corpus / version / name
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(data)
            manifest.append(f"{corpus}\t{pkg}=={version}\t{name}\t1 file")
            print(manifest[-1])
    (CACHE / "MANIFEST.tsv").write_text("\n".join(manifest) + "\n")
    shutil.rmtree(raw, ignore_errors=True)


if __name__ == "__main__":
    main()
