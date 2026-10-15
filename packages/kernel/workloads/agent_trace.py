"""Whole versus range reads in a coding agent's transcript (E050).

Counts only; no file content or command text is printed. Usage:
    python3 -I agent_trace.py <transcript.jsonl> <repo root> [<repo root> ...]

A read is classified from the agent's tool calls:
- whole: `cat FILE`, or a Read call without offset/limit;
- range: `sed -n A,Bp FILE`, `head`/`tail FILE`, or a Read call with offset/limit;
- scan: grep/rg over files or directories (a scan decodes every file it covers).
Files are sized as they are now under the given roots.
"""
import collections, glob, json, os, re, shlex, sys

SRC = re.compile(r'[\w./~*-]+\.(rs|md|py|toml|json|txt|jsonl|sh|wat|csv|ya?ml)$')
reads = collections.defaultdict(list)  # kind -> paths


def resolve(arg, roots):
    for root in roots:
        hits = glob.glob(arg if arg.startswith('/') else os.path.join(root, arg))
        out = []
        for h in hits:
            if os.path.isdir(h):
                out += [os.path.join(d, f) for d, _, fs in os.walk(h)
                        if 'target' not in d and '.git' not in d for f in fs]
            elif os.path.isfile(h):
                out.append(h)
        if out:
            return out
    return []


def bash(command, roots):
    for seg in re.split(r'\|\||&&|;|\||\n', command):
        try:
            t = shlex.split(seg.strip())
        except ValueError:
            continue
        if not t:
            continue
        cmd, args = t[0], [a for a in t[1:] if not a.startswith('-')]
        if cmd == 'cat' and '<<' not in seg and '>' not in seg:
            kind = 'whole'
        elif cmd == 'sed' and '-n' in t or cmd in ('head', 'tail'):
            kind = 'range'
        elif cmd in ('grep', 'rg'):
            kind, args = 'scan', args[1:]  # the first non-flag argument is the pattern
        else:
            continue
        for a in args:
            if kind == 'scan' or SRC.search(a):
                reads[kind] += resolve(a, roots)


def main(path, roots):
    for line in open(path):
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        content = (ev.get('message') or {}).get('content')
        if ev.get('type') != 'assistant' or not isinstance(content, list):
            continue
        for b in content:
            if b.get('type') != 'tool_use':
                continue
            inp = b.get('input') or {}
            if b.get('name') == 'Read':
                kind = 'range' if 'offset' in inp or 'limit' in inp else 'whole'
                reads[kind] += resolve(inp.get('file_path', ''), roots)
            elif b.get('name') == 'Bash':
                bash(inp.get('command', ''), roots)
    for lim in (0, 16 * 1024, 64 * 1024):
        big = {k: [p for p in v if os.path.getsize(p) > lim] for k, v in reads.items()}
        files = len({p for v in big.values() for p in v})
        counts = {k: len(big.get(k, [])) for k in ('whole', 'range', 'scan')}
        print(f'files > {lim // 1024} KiB: {files} distinct; reads {counts}')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2:])
