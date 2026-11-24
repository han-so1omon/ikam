"""Candidates: one catalog constant changed by one line (E400)."""
import os

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def rows():
    out = []
    for line in open(os.path.join(HERE, "catalog.tsv")):
        if not line.startswith("#"):
            name, file, l, value, tries = line.rstrip("\n").split("\t")
            out.append(dict(name=name, file=file, line=l, value=value, tries=tries.split(",")))
    return out


def changes(cfg):
    """Every single change the catalog offers, in catalog order (the fixed
    ranking)."""
    return [dict(name=r["name"], base=r["value"], value=v, row=r) for r in rows() for v in r["tries"]]
