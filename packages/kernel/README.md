# ikam-kernel

A content-addressed store whose files are guaranteed to reconstruct byte-for-byte. It provides:
- dedup at data-driven boundaries
- compression
- zip-container unpacking
- a ledger of verified derivations (`output = func(args)`, with byte-range selectors). The ledger is storage, provenance and the object graph at once.
- pure (WASM) functions, memoized through the ledger
- git-like versioning with store-wide repacking

The design, laws and measurements are in `docs/plans/2026-10-02-rust-kernel.md`.

```sh
cargo test                                         # property tests for every law
cargo run --release -- put FILE                    # prints the file's content id
cargo run --release -- cat ID > out                # reconstructs FILE exactly
cargo run --release -- commit DIR -m "msg"         # snapshot DIR onto ref main
cargo run --release -- log                         # history of main
cargo run --release -- checkout main DEST          # restore a snapshot
cargo run --release -- apply FUNC ARG...           # run a stored WASM function (memoized)
cargo run --release -- links ID                    # outgoing graph edges
cargo run --release -- used-by ID                  # incoming graph edges
cargo run --release -- claim A "revision of" B     # semantic claim, weight measured in bits
cargo run --release -- relate ID 5                 # contents most informative about ID
cargo run --release -- promote CLAIM               # turn a claim into verified storage
cargo run --release -- fsck                        # reconstruct and verify everything
cargo run --release -- gc                          # drop objects no ref reaches
cargo run --release -- repack                      # re-plan all storage (only if smaller and verified)
```

The store lives in `.ikam` by default; override it with `--store DIR`.

Working with a corpus:

```sh
ikam --store S ingest DIR -m "msg"   # commit DIR, repack, verify every file; prints sizes and times
ikam --store S ls main:sub/dir       # list a directory (-r for every file)
ikam --store S grep "text"           # files containing text, also inside docx XML and PDF text
ikam --store S stats                 # objects and bytes by kind and encoding
ikam --store S cat main:path/file    # REV:PATH, a hex id or a unique prefix work everywhere
ikam --store S serve                 # browse, read and search at http://127.0.0.1:8737/
```

A live filesystem (E110): one process owns the store and you change it while
it runs.

```sh
ikam --store S live                          # http://127.0.0.1:8737/ (page) and /dav/ (WebDAV)
ikam --store S live --watch ~/Drop:inbox     # also mirror a folder into inbox/ as it changes
ikam --store S live --llm                    # let questions be answered by Claude (ANTHROPIC_API_KEY)
ikam --store S write notes/a.md FILE         # or: rm, mv, cp, mkdir; each one commit
ikam --store S search "text"                 # text of every file, also docx/xlsx/pptx XML, PDF, tar members
ikam --store S ask "which file says how to add a paragraph?" [--llm]
```

- **Mount it**: in Finder, Go > Connect to Server, `http://127.0.0.1:8737/dav/`
  (any user name and password). Copy files in, open, edit and save them,
  rename and delete them; on Linux, `mount -t davfs` or any WebDAV client.
- **An edit** stores the new content (`put_content`, deduplicated against
  what is already stored) and rewrites only the trees on its path
  (`Repo::edit_tree`); it does not repack. Edits show at once; they are
  committed to the ref after 1.5 s of quiet (`--commit-after MS`), so a
  Finder copy of a folder is one commit. `repack` (the page's button, or
  `ikam repack` with the server stopped) re-plans everything when you
  choose.
- **Search** is an exact, case-insensitive match over every file's text,
  held in memory and updated with each edit. **Questions** rank 12-line
  passages (BM25) and list them as numbered sources with file and line;
  with `--llm` and a key, Claude writes an answer from those passages only
  and cites them by number. Only the question and the passages are sent.
- The page shows the tree, versions of a file, what each commit changed,
  and what the store holds. `/api/status`, `/api/search?q=`, `/api/ask?q=`
  answer in JSON.
- It listens on 127.0.0.1 only, refuses requests whose Host is not
  127.0.0.1 or localhost, and serves files with a sandbox policy (an HTML
  file in the store cannot run script in the page). WebDAV locks are
  granted but not enforced (one user).
- **It stays in its own folder**: everything it writes is under `--store`.
  A `--watch` folder is only read, never written to, and the WebDAV mount
  is a separate network volume, so the machine's own files are untouched.
  Stopping it and deleting the store folder removes it.
- **From other machines** (E111): put a proxy that only your devices reach
  in front of it, such as Tailscale Serve, and name the proxy's host with
  `--allow-host NAME` (repeatable; it still listens on 127.0.0.1 only).
  With `IKAM_AUTH` set, every request, page and `/dav/` alike, needs that
  user and password (HTTP basic auth, which Finder sends):

  ```sh
  read -rs PW; printf '%s\n' "$PW" | ikam passwd eric   # prints IKAM_AUTH=eric:salt:hash
  IKAM_AUTH=... ikam --store S live --allow-host mac.example.ts.net
  ```

  Keep that line in a file only you can read (the hash is BLAKE3, which is
  fast to guess against). SIGTERM or Ctrl-C commits pending edits, then
  exits, so a service manager can restart it without losing an edit.
- **Seeing what clients send** (E113): `--log-requests` prints one line per
  request, with the time, method, path, Host, the client's first User-Agent
  word and whether it carried a password (never the password), then the
  status and how long it took. Every log line starts with the time (UTC).

`realworld/live_bench.py` measures it (edit cost and latency, exact
read-back against a copy of the tree, search freshness, restart).

`--short-records` stores derivation records with short ids (smaller, for
archives; see `docs/experiments/options.md`). Real-world corpora and how the
kernel does on them: `realworld/README.md`.

Benchmark (deterministic corpora from this repo, verified exact, with zstd baselines):

```sh
cargo run --release --example bench               # table
cargo run --release --example bench -- --json     # one JSON line per corpus
cargo run --release --example bench -- invoices pdf   # a subset while iterating
cargo run --release --example bench -- north     # only the north corpora (one graph across file types)
mise run experiment                               # the fixed experiment command (docs/experiments)
```

It prints two scores against zstd with a trained dictionary: a cost score (monthly cost under a heavy read workload) and a north score (bytes of mixed-type corpora stored as one graph under an archive workload).

A file's id is the BLAKE3 hash of its content and never depends on how it is stored. Its bytes may be stored (zstd-compressed when smaller), or it may exist only through derivations, for example:
- `concat` of byte ranges of other content, found by extending matches byte-by-byte
- `deflate-pack` rebuilding a zip from its uncompressed members
- `fill(template, fillers)`: similar documents share one template; each keeps only what differs
- a WASM function of other content

One file may have several derivations. A corrupt stored copy falls through to them.

Claims are the descriptive half of the graph: `subject predicate object` over any selections, with a weight the kernel measures (bits the object saves on the subject). They never affect reconstruction, but `promote` turns one into storage when that is verifiably smaller.

Stored bytes use the smallest of raw, zstd, or zstd against a shared dictionary trained by `repack` (the dictionary is content too). Plans are chosen by an evaluator: bytes written plus `read_weight` × decode work.

Every derivation is verified by reconstructing the exact bytes before it is recorded. Planners, including future AI ones, can therefore cost space but never correctness. Which bytes stay stored is a cost decision that `repack` re-makes store-wide.

WASM functions export `memory`, `alloc(len) -> ptr` and `run(ptr, len) -> (out_ptr << 32 | out_len)`. They may not import anything or use floats, and they run under fixed fuel and memory limits.
