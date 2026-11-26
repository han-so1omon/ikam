"""Split a MediaWiki dump into one file per <page> (in order) under OUTDIR; concatenating
the files in name order gives the input back byte for byte. Usage: splitwiki.py IN OUTDIR"""
import sys, os, re
data = open(sys.argv[1], 'rb').read(); out = sys.argv[2]
starts = [m.start() for m in re.finditer(rb'  <page>\n', data)]
cuts = [0] + starts + [len(data)]
for d in range(0, len(cuts) - 1, 1000): os.makedirs(f"{out}/{d // 1000:04d}", exist_ok=True)
for i in range(len(cuts) - 1):
    open(f"{out}/{i // 1000:04d}/{i:07d}.xml", 'wb').write(data[cuts[i]:cuts[i + 1]])
print(len(cuts) - 1, 'files', file=sys.stderr)
