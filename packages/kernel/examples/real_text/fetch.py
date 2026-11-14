"""Fetch a random sample of PyPI projects' metadata (summary, README,
classifiers) from the PyPI JSON API, keeping OSI-licensed projects with a
real summary and README. Then fetch earlier releases' READMEs for a subset,
to get real document version histories. Writes JSONL to argv[1].
"""
import json, random, sys, re, concurrent.futures as cf, urllib.request, time

OUT = sys.argv[1]
N_SAMPLE = int(sys.argv[2])
N_HIST = int(sys.argv[3])
UA = {"User-Agent": "research-script/0.1 (embedding benchmark)"}


def get(url):
    for i in range(3):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=20) as r:
                return json.load(r)
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            time.sleep(1 + i)
        except Exception:
            time.sleep(1 + i)
    return None


def keep(info):
    s = (info.get("summary") or "").strip()
    d = info.get("description") or ""
    cls = info.get("classifiers") or []
    if not any(c.startswith("License :: OSI Approved") for c in cls):
        return False
    if not (5 <= len(s.split()) <= 30) or len(d) < 600 or len(d) > 60000:
        return False
    if sum(ch.isascii() for ch in d) < 0.97 * len(d):
        return False
    return True


def one(name):
    j = get(f"https://pypi.org/pypi/{name}/json")
    if not j or not keep(j["info"]):
        return None
    i = j["info"]
    rel = sorted(
        (v for v, files in j.get("releases", {}).items() if files),
        key=lambda v: min(f["upload_time"] for f in j["releases"][v]),
    )
    return {"name": i["name"], "summary": i["summary"].strip(),
            "description": i["description"], "classifiers": i["classifiers"],
            "releases": rel[-8:]}


def history(p):
    out = []
    for v in p["releases"][-6:]:
        j = get(f"https://pypi.org/pypi/{p['name']}/{v}/json")
        if j and j["info"].get("description"):
            out.append({"version": v, "description": j["info"]["description"]})
    return out


def main():
    names = [x["name"] for x in json.load(open(sys.argv[4]))["projects"]]
    random.Random(256).shuffle(names)
    names = names[:N_SAMPLE]
    kept = []
    with cf.ThreadPoolExecutor(8) as ex:
        for k, r in enumerate(ex.map(one, names)):
            if r:
                kept.append(r)
            if k % 2000 == 0:
                print(k, len(kept), flush=True)
    hist_c = [p for p in kept if len(p["releases"]) >= 4]
    random.Random(257).shuffle(hist_c)
    hist_c = hist_c[:N_HIST]
    with cf.ThreadPoolExecutor(8) as ex:
        hs = list(ex.map(history, hist_c))
    by = {p["name"]: h for p, h in zip(hist_c, hs)}
    with open(OUT, "w") as f:
        for p in kept:
            p["history"] = by.get(p["name"], [])
            f.write(json.dumps(p) + "\n")
    print("kept", len(kept), "with history", sum(1 for h in hs if len(h) >= 3))


main()
