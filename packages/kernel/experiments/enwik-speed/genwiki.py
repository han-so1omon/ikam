"""Generate a MediaWiki-XML-shaped corpus (enwik-like structure) from seed prose.
Usage: genwiki.py SEED OUT TARGET_BYTES SEEDNUM"""
import sys, random, re, collections
seed, out, target, rs = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
rng = random.Random(rs)
words = re.findall(r"[A-Za-z][A-Za-z'-]{1,14}|[.,;:]", open(seed, errors='ignore').read())[:3_000_000]
chain = collections.defaultdict(list)
for a, b, c in zip(words, words[1:], words[2:]):
    chain[(a, b)].append(c)
keys = list(chain)
vocab = [w for w in set(words) if w[0].isalpha() and len(w) > 3]
rng.shuffle(vocab)
# titles: Zipf-popular so links concentrate like Wikipedia's
titles = []
seen = set()
while len(titles) < 400_000:
    t = ' '.join(w.capitalize() for w in rng.sample(vocab[:20000], rng.choice([1, 1, 2, 2, 3])))
    if t not in seen: seen.add(t); titles.append(t)
cats = [' '.join(w.capitalize() for w in rng.sample(vocab[:3000], 2)) for _ in range(3000)]
templates = ['Infobox country', 'Infobox person', 'Infobox city', 'Infobox album', 'Infobox company', 'Infobox species']
fields = {t: rng.sample(vocab[:2000], 10) for t in templates}
langs = ['de', 'fr', 'es', 'it', 'nl', 'pl', 'pt', 'ru', 'sv', 'ja', 'zh', 'fi', 'no', 'da', 'he']
def zipf_title(): return titles[min(int(rng.paretovariate(1.1)) - 1, len(titles) - 1) * 7 % len(titles)]
def sentence():
    a, b = rng.choice(keys); s = [a, b]
    for _ in range(rng.randint(8, 30)):
        nxt = chain.get((a, b));
        if not nxt: break
        c = rng.choice(nxt); s.append(c); a, b = b, c
    outw = []
    for w in s:
        r = rng.random()
        if r < 0.06: t = zipf_title(); outw.append(f"[[{t}]]")
        elif r < 0.08: t = zipf_title(); outw.append(f"[[{t}|{w}]]")
        elif r < 0.085: outw.append(f"'''{w}'''")
        else: outw.append(w)
    return ' '.join(outw).replace(' .', '.').replace(' ,', ',') + '.'
def body(n):
    parts = []
    if rng.random() < 0.4:
        t = rng.choice(templates); parts.append('{{' + t + '\n' + '\n'.join(f"| {f} = [[{zipf_title()}]]" if rng.random() < .5 else f"| {f} = {rng.randint(1, 9999)}" for f in fields[t]) + '\n}}')
    size = 0
    while size < n:
        if rng.random() < 0.15: p = f"\n== {rng.choice(vocab).capitalize()} ==\n"
        else: p = ' '.join(sentence() for _ in range(rng.randint(2, 6))) + '\n'
        parts.append(p); size += len(p)
    parts += [f"[[Category:{rng.choice(cats)}]]" for _ in range(rng.randint(1, 5))]
    parts += [f"[[{l}:{rng.choice(titles)}]]" for l in rng.sample(langs, rng.randint(0, 12))]
    return '\n'.join(parts)
def esc(s): return s.replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;').replace('"', '&quot;')
with open(out, 'w') as f:
    f.write('<mediawiki xmlns="http://www.mediawiki.org/xml/export-0.3/" version="0.3" xml:lang="en">\n  <siteinfo>\n    <sitename>Wikipedia</sitename>\n    <case>first-letter</case>\n  </siteinfo>\n')
    written, pid, rid = 0, 0, 1000
    while written < target:
        pid += 1; rid += rng.randint(1, 500)
        title = titles[pid - 1]
        if rng.random() < 0.25:  # redirects are a large share of enwik pages
            txt = f"#REDIRECT [[{zipf_title()}]]"
        else:
            txt = body(int(min(rng.lognormvariate(8.0, 1.2), 200_000)))
        page = (f"  <page>\n    <title>{esc(title)}</title>\n    <id>{pid}</id>\n    <revision>\n      <id>{rid}</id>\n"
                f"      <timestamp>2006-0{rng.randint(1,3)}-{rng.randint(10,28)}T{rng.randint(10,23)}:{rng.randint(10,59)}:{rng.randint(10,59)}Z</timestamp>\n"
                f"      <contributor>\n        <username>{rng.choice(vocab[:500]).capitalize()}</username>\n        <id>{rng.randint(1, 900000)}</id>\n      </contributor>\n"
                f"      <text xml:space=\"preserve\">{esc(txt)}</text>\n    </revision>\n  </page>\n")
        b = page.encode(); 
        if written + len(b) > target: break
        f.write(page); written += len(b)
    print(pid, 'pages', file=sys.stderr)
