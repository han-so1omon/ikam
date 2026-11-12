# Option space: what each optimization trades

The kernel makes most storage choices itself, by one rule: a plan costs its
stored bytes plus `read_weight` times the decode work its reads take (E036,
E069). Most "optimizations" are therefore not switches but plans the
planner may take when they pay at the caller's read weight. This page lists
what a caller can set, what is built in, and what was measured and could
become a setting. Numbers come from the experiment log; each row names its
entry.

## The one dial: `Repo::read_weight`

Stored bytes worth one decoded byte. It decides between every codec, group,
template, slice, derivation and container plan below.

| Workload | read_weight | Where |
|---|---|---|
| Heavy (1,000 read passes and 100 searches each a month, the cost score) | about 49 | bench `READ_WEIGHT` (E064, E069) |
| Default (`DEFAULT_READ_WEIGHT`) | 0.001 | `repo.rs` |
| Archive (2 reads a month, the north score) | about 0.00097 | bench `ARCHIVE_READ_WEIGHT` (E099) |

Low weights let slow codecs (xz, brotli, PPMd: E031, E033, E038), deep
derivations (programs, quotes, templates) and container rebuilds (E080) win;
high weights favour raw or zstd content that reads fast (E074). A
**profile** (heavy / balanced / archive) that sets the dial and the settings
below together is the simplest interface; today callers set the fields.

## Settings a caller has

| Setting | Default | Trade | Measured |
|---|---|---|---|
| `read_weight` | 0.001 | bytes against decode CPU, for every plan | E069, E099 |
| `search_rate`, `search_sample` | 0 (no index) | a trigram index is stored only if it pays for that many searches per read pass | E065, E089, E094 |
| `proposed_order` | empty | an external content order (embeddings, an LLM) for grouping; used only when it saves bytes | E019; no effect in E051 |
| `slow_codec_max` (new, E109) | off (`usize::MAX`) | contents larger than this many bytes skip brotli when repack picks their encoding: repack time against the few percent brotli wins on large content | pandas repack 130 -> 82 s at 4 MiB, same bytes (brotli lost on its 23.6 MB tar); peak 791 -> 849 MiB; see below |
| `short_records` (new, E104) | off | derivation records store 8-byte id prefixes and a builtin index: about a third less record bytes; reading resolves prefixes (a two-pass ledger build) | north 0.3035 -> 0.2793 (8%); cost score up about 4.5% on four runs with identical instruction counts (timing, not work); see below |
| `markup_layers` (new, E112) | off | a zip's text-heavy XML parts (docx body, sheets, slides) stored as one skeleton member and one text member, so their words compress with the same text in other formats; repack CPU to split and rebuild them | north 0.3035 -> 0.3005; cases one graph -4.0%, multiformat +1.4%, tables -0.2%; office corpus cost up (0.33-0.35 -> 0.40-0.46); single-type real office corpora -0.1% to +0.9% |
| `joins_build`, `joins_update`, `join_rows`; `Arrangement`, `Join` (new, E243) | not used unless called | joins kept up to date from a batch of changes through indexes by join key, shared between joins; an update can cost more than recomputing when its changes reach many join rows | 20k rows: 10 changes 46 ms vs 372 ms recomputed; 1,000 changes 862 vs 395 ms |

Reading accepts every stored form whatever the settings, so a setting can be
changed on an existing store; the next repack re-plans under it.

## Built in, chosen by the planner

| Optimization | What it buys | What it costs | Log |
|---|---|---|---|
| zstd at level 19 for stored objects | smaller objects | slower repack (plans are measured at level 3, stored at 19) | E030 |
| Shared trained dictionary (one) | small objects compress against common content | dictionary bytes; a 1 MiB cap and two clustered dictionaries did not pay | E003, E078, E079 |
| Small contents grouped (64 KiB solid groups, blocked for range reads) | context across small files | reading a member decodes the group up to it; 4 MiB groups were rejected by read pricing | E016-E020, E037, E082 |
| Related content as zstd context | near-duplicates stored as deltas | decode needs the context | E057, E084 |
| Slices and templates | shared spans and filled templates stored once | search work; skipped when read work alone loses | E092 |
| zlib/deflate streams expanded (zip, docx, xlsx, pptx, PDF, PNG) | inner bytes compress and share; multiformat -38%, -31% more with memLevel 9 | re-deflating on read; ingest CPU | E080, E083 |
| PDF text layer | PDF text stored as UTF-8 next to the other formats' text; multiformat -12% | a skeleton/text rebuild through the font's ToUnicode map on read | E102 |
| Programs (WASM) and quotes | generated or quoted contents derived instead of stored | fsck and read CPU; quotes no longer pay under the heavy workload | E022-E026, E056, E095, E096 |
| Trigram search index (per distinct chunk) | search without decoding everything | index bytes | E058, E089, E094 |
| Checked raw content | hot content read without decoding (CRC-32 on read, BLAKE3 in fsck) | bytes | E074 |

## Measured, not kept, could be a setting

| Candidate | Result | As an option |
|---|---|---|
| Range-read blocks (verified blocks per leaf) | pays only if range reads weigh about 10x whole reads | a `range_read_rate`, like `search_rate` (E047) |
| zstd checksum per frame | neutral on the score | an integrity setting for untrusted media (E076) |
| Dissolve groups where contexts cost less | bytes-neutral, trades bytes for reads | covered by `read_weight` (E091) |
| One record set for csv/json/md/sheet | records would cost more than the bytes saved | no (E103) |

## Not measured yet

- **Font glyph dedup across PDFs**: fonts are 48 KB of multiformat's 332 KB one-store (E101 roles); subset fonts of the same face share most glyph programs.
- **A profile setting** (heavy / balanced / archive) instead of separate fields.
- **Real-world corpora**: measured in E105-E108 (`packages/kernel/realworld`); the kernel is below the best archiver on 7 of 8 after E106. Scale was the open limit: four pandas releases repacked in 7 minutes at 842 MiB peak (E108); since E109 in 2 minutes 10 s (82 s with `slow_codec_max` at 4 MiB), same bytes, peak 865 -> 791 MiB. Peak memory (about 0.9 GB for 17.6 MB of archives, 112 MB expanded) is the next limit.

## E104 as an option

E104 stored derivation records with short ids. Its bytes win was clear on
every run (tables 135,551 -> 122,678; multiformat 332,403 -> 310,415; cases
391,496 -> 361,242), but the cost score rose about 4.5% on all four runs while
callgrind counted the same instructions for the office ingest as main
(10,039,456,397 against 10,037,569,872). Rather than accept or refuse it on a
timing difference it is now `Repo::short_records`, off by default, and
`bench --short-records` measures with it on. Off, every corpus is
byte-identical to main (two runs; cost 0.2484 and 0.2608, the second with
other work beside it, a 5% spread with identical bytes); on, it is E104
(north 0.2793, cost 0.2646).

**When to turn it on**: archives and cold stores, where bytes are the cost
(north -8%). **When to leave it off**: hot stores, until a run shows the extra work
costs nothing there; building the ledger resolves every short id against
the store's ids (a second pass over the records).

## `slow_codec_max` (E109)

Brotli (the one slow codec still tried) cannot use more than one core, so
on one large content it bounds the whole recompress. On four pandas source
releases it took 44 s of the repack for one 23.6 MB tar and then lost: 3%
smaller than zstd 19, but that tar is read four times over (each later
release is spliced from it), and four times zstd's decode work outweighed
the bytes. On the real-world corpora, the largest content brotli won was
3.3 MB (8% smaller than zstd 19); it lost on the two larger ones it was
tried on (8.0 MB, 23.6 MB). With the cap the pandas peak was 849 MiB
against 791 without it (still below 865 before E109): zstd's work on the
large tars overlaps instead of waiting behind brotli.

**When to set it** (`Repo::slow_codec_max`, `realworld --slow-codec-max`,
`ikam --slow-codec-max`): stores with large contents where repack time
matters; 4 MiB keeps every brotli win measured so far. **When to leave it
off**: archives where every byte counts and repack time does not, since
brotli can win a few percent on a large content read rarely.

## `ikam live` (E110)

The live filesystem stores each edit as it comes and leaves re-planning to
`repack`. Its settings are about when work happens, not what is stored.

| Setting | Default | Trade | Measured |
|---|---|---|---|
| `--commit-after MS` | 1500 | edits are readable at once but reach the ref only after this much quiet: fewer, larger commits (a Finder copy of a folder is one) against edits that a crash in that window leaves out of the tree (their objects stay in the store, unreachable until a later commit or `gc`) | one commit per burst in the live bench |
| when to repack | by hand (page button, `ikam repack`) | stored bytes against minutes of CPU: between repacks, edits are stored as `put_content` plans them; repack re-plans everything | see E110: about 17% more bytes before the first repack on the real-world corpora, repack 92 s |
| `--watch DIR[:PREFIX]` scan interval | 500 ms (fixed) | a change on disk is readable after about this long; each scan stats every file in the folder | lag p50 505 ms in the live bench |
| `--llm` | off | questions get a written answer from Claude (the question and the retrieved passages are sent to the Claude API) against sources only, locally | not scored |

| `--allow-host NAME` (E111) | none | another name the server answers to (a proxy such as Tailscale Serve) against a wider set of pages that could reach it if that name ever pointed somewhere else; it still listens on 127.0.0.1 only | not scored |
| `--log-requests` (E113) | off | a line per request (what a client such as Finder sends) against a log that grows with use | not scored |
| `--public` (E114) | off | anyone without the password can read every file, search and mount read-only; writes, admin calls and Claude answers still need the password; needs `IKAM_AUTH` | not scored |
| `--max-body BYTES` (E114) | none (25M with `--public`) | one upload can't fill memory (the body is held whole) against larger single files refused with 413 | not scored |
| `--max-store BYTES` (E114) | none (1G with `--public`) | the disk can't fill against new content refused with 507 once reached; history is kept, so deletes don't free it | not scored |
| `--max-conns N` (E114) | none (32 with `--public`) | threads stay bounded against a busy client (Finder opens several) waiting on 503 | not scored |
| `--gc-every N` (E116) | off | the store keeps only what commits need (edits leave a working tree per edit and files replaced before a commit) against a pause after every N commits while gc walks the whole store; `POST /api/gc` does it once | test copy: about 70% of each self-test run's bytes freed |
| `IKAM_AUTH` (E111) | unset | a password on every request against one more thing to keep secret; the hash is fast (BLAKE3), so the line belongs in a file only its owner reads | not scored |

**Not yet a setting**: an incremental repack that re-plans only what was
added since the last one (the live drift is in new content; a full repack
re-plans all of it).

## The database layer (E118 to E120)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| Batch size of `db_append` (E118) | the caller's | fewer graph edits and objects per event against waiting to fill a batch | 10,000 events: one at a time 48 s, in batches of 1,000 1.9 s (E120) |
| `Keys::Any` in `put_graph_with` / `graph_edit_with` (E120) | `Keys::Unique` (`put_graph`, `graph_edit`); the db's `ids` and `kv` views use `Any` | labels chosen by clients may share an 8-byte key without refusing the graph or blocking a rebuild, against edges that could name two nodes: only for graphs no edge points into | a real clashing pair is kept, read back and rebuilt (tests) |

**Not yet a setting**: storing new view chunks plain on the append path and
leaving slices to repack or a background integrator (E119, measured with
read weight 1.0: 1,000 single appends 0.9 s and 7.9 MB stored against
3.1 s and 3.3 MB). A key index for the log, so in-place edits check its
label keys across the whole graph (needed only near 100M events: chance
clashes are about 3 in 10,000 there).

## Store backends (E122)

Each backend sits behind the `Store` trait; `FsStore` (one file per object) stays the default until the Mac numbers are in. The others build with the cargo feature named (`stores` for all).

| Backend | Feature | Trade | Measured (Linux, ext4) |
|---|---|---|---|
| `FsStore` | default | simplest, readable by any tool; one file and one 4 KiB block per object, 66 flushes and 25 files changed per commit | 4.1 GB allocated for 1M small objects (288 MB of data); 19.2 ms a single append |
| `PackStore` | default | append-only packs and a checkpointed index; a pack cleaner is not built yet, so deleted objects stay until one is | 769 MB/s writes, 2 flushes and 2 files per commit, 372 MB for 1M objects, open 279 ms and cold read p50 190 us at 10M |
| SQLite (WAL) | `sqlite` | one file, `synchronous` FULL (or NORMAL: 0.3 flushes a commit, may lose the last commits on power loss, never corrupts) | 9 ms open at 10M; cold p50 155 us, warm 92 |
| LMDB | `lmdb` | fastest warm reads (1.6 us at 1M); a fixed map size and 1.6 to 2.8 times the bytes | 6.1 GB at 10M |
| redb | `redb` | pure Rust, one file; the largest on disk at 1M | cold p50 85 us at 10M |
| fjall, with or without key-value separation | `fjall` | LSM; fast writes, slow open (640 ms to 1.8 s) | 330 MB/s writes separated |
| RocksDB, with or without blobs | `rocksdb` | LSM; needs a C++ toolchain to build; 3.7 s open at 10M | cold p50 142 us at 10M |
| PackStore, `lazy_checkpoint` (E171) | default, off | open searches the open pack's checkpoint in place; cold reads one read slower until cached | 1M open 205 ms to 1.1 ms; cold read p50 38 to 79 us |
| PackStore, `ref_log` (E172) | default, off | refs as records in the pack: one flush and no rename a commit; a format addition (ref records, `refs.snap`) | swap p50 495 to 143 us; 100 kills clean |
| `hybrid::RefsHybrid` (E170) | `store-rocksdb` | objects in packs, refs in RocksDB; still two flushes a commit | 1M open 252 ms; swap p50 189 us |
| `hybrid::IndexHybrid` (E170) | `store-rocksdb` | own packs, object places and refs in RocksDB; one flush for a swap, two for new objects | 1M open 663 ms; 10k batch 7.5 s |
| PackStore, `filters` (E175) | default, off | a Bloom filter beside each pack index (`.flt`, `.cflt`): lookups skip packs that cannot hold the id; derived files, rebuilt when missing or damaged, but a format addition | 10M cold read p50 about 160 to 80 us; missing id about 140 to under 1 us; 0.3% more disk |
| `PackStore::compact` (E176) | not run unless called | rewrites live records into new packs and deletes the old; one writer, so a server runs it between requests | 10M churned store 4.33 to 2.61 GB in 55 s with filters, 124 s without |
| `backends::GroupCommit` (E177) | not used unless wrapped | many threads share one store and one flush covers every waiting commit; needs `ref_log`; p50 latency rises a little as a commit waits for the flush in progress | 16 writers 3x the commits a second, p99 about 80 to 2 ms |

## Where new content looks for repeats (E131)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `Repo::seed_scope` (E131) | `None`: every stored content | `Some(roots)`: the first write reads only those contents and what they are made of, and near-match reuse stays inside them, against missing repeats of content outside them | one-file edit's first write 238 MB / 1.7 s to 0.69 MB / 8.8 ms, same bytes stored; a 36-file folder copy with source and destination folders in scope 8.5 KB written against 21.8 KB on the whole store; an empty scope 1.09 MB against 2.9 KB |
| `Repo::set_seed_scope` (E133) | not called | a new scope for each request (a goal's folders), the seed index rebuilt for it, against that rebuild's reads (2.7 MB for the second goal in E133) | name the trees and view chunks a write replaces too: a view update stored 237 KB with them, 404 KB without |
| What to put in a goal's scope (E137) | (a choice of the caller) | the files the goal reads and edits, plus the trees and view chunks it replaces, against their whole folders: misses repeats of files the goal never touched | E133's first goal 94 MB / 797 ms to 7.7 MB / 318 ms, 250,379 bytes written against 250,630 |

**Not yet a setting**: a ledger cache file (E130: 159 KB and 0.1 ms to load
in one read; since E135 a fresh handle's first lookup reads 192 KB of headers
in 1,906 reads instead of 229 MB). It would save round trips, mainly on R2;
it is an on-disk addition and waits on a decision.

## Reads within one request (E124 to E128)

| Setting | Default | Trade | Measured |
|---|---|---|---|
| `Repo::session(limit)` | plain reads (no session) | memory up to `limit` decoded bytes, against refetching a group, dictionary or container for each read | all text files 7.31 -> 5.30 MB fetched; a goal's windows 257 -> 166 KB ("invoice", files whole 184 KB); 238 Office files 18.9 -> 2.68 MB in one session (E128) |
| Reverse-link view (not built) | none: incoming links by scan | 33.9 MB and a write per link change, against a scan of every object | incoming links 113k reads and 340 ms by scan, 8 reads and about 1 ms by view (E125) |
| Path index (not kept) | tree walk | an index graph, 62 KB here | worse: 4 reads a lookup against 2 by walking trees (E125) |
| Per-tenant stores (not built) | one shared store | isolation of every stored object, against dedup across tenants | 644 MB against 229 MB for the bench corpus's 26 folders (E126) |
| Short-term index for terms under 3 characters (not built) | trigram index only | index bytes, against a scan of every text file for "Q3" (7.3 MB, 68 ms) | E127 |

## What one edit reads (E138)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `Repo::read_memo` (E138; `--read-memo`) | off | each tree, view or content write keeps the stored contents it reads (up to 64 MB) until it returns, so the old chunks a new chunk is matched against are read once, against holding those bytes in memory for the edit; the same bytes are stored either way | E133's first goal (E137's scope) 7.7 MB to 4.5 MB fetched, its view update 3.85 MB / 1,337 reads to 1.01 MB / 446 reads; next goal 5.7 MB to 3.3 MB; identical objects stored |

## Which store, for a goal (E139)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `PackStore` (E122) against `FsStore` | `FsStore` | two syncs per ref move against one per object written, and a writer lock, against one file per object that any tool can list | E133's first goal with the recommended options 735 ms on files, 351 ms on packs (291 ms in memory), the same 4.5 MB fetched and bytes written; the view update syncs 425 objects on files |

## What a goal may touch (E140)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| Section grants (`Grant`, `windows_around`) against file grants (E126) | none in the kernel; the server checks paths | a grant names file versions and byte ranges, so a stale version or a change outside the goal's lines is refused; the server must place the windows (it reads the matched files) and an edit check diffs the old and new bytes | "Q3": 16.7 KB of 347 KB exposed (4.8%); read check 14 to 22 ns, edit check 14 to 33 us, seal or verify 1.5 to 3.2 us; 0 of 70 out-of-window edits allowed against 70 of 70; same bytes fetched as whole files (groups) |

## What a goal's first write indexes (E141)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `replace_hints` | off | with a scope, old trees and chunks an edit reads join the seed index, so the scope can name only the files; repeats of content outside the scope and outside what the edit reads are not found | E133's first goal 4.21 MB to 2.50 MB, next 3.26 MB to 1.23 MB (heads apart); first write 1.78 MB to 189 KB; bytes written 250,171 against 250,379 |

## Search and read in one session (E142)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `ReadSession::search` | `Repo::search` (no shared decodes) | the scan keeps decodes up to the session's limit, so memory is bounded by that limit; reads of the hits then cost nothing | E133's first goal 2.50 MB to 2.04 MB, next 1.23 MB to 1.14 MB (heads apart); read stage to zero |

`read_memo` (E143) also covers template candidates: a similar chunk is read once per edit, not twice. E133's first goal on the recommended setup 2.04 MB to 1.59 MB, next 1.14 MB to 0.78 MB; bytes written unchanged.

## Template plans for trees and view chunks (E144)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `object_templates` | on | off: a tree or view chunk is stored as slices of earlier content or plain, never as a template fill that also rewrites the old chunk; edits only add objects | E133's first goal on the recommended setup writes 250 KB to 86 KB, the store grows 72 KB against 76 KB, 332 ms to 159 ms on packs; the next goal reads 2% more |

## A goal's writes as one batch (E145)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `begin_batch` / `end_batch`, `keep_read` | each write is its own edit | with `read_memo` on, what a goal's writes read (and the files it hands over) is kept until the batch ends, up to the memo's 64 MB | E133's first goal on the recommended setup 1.48 MB to 1.13 MB, edit stage 414 KB to 70 KB; bytes written unchanged |

## Object lengths from the store (E147)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `Store::stored_len` | provided; reads the object | a backend that answers from its index saves one read per old chunk whose bytes a batch already holds; a backend that keeps the default behaves as before | E133's first goal in a batch: edit stage 70 KB to 8 KB, the goal 1.13 MB to 1.07 MB; bytes written unchanged |

## A view's edits kept pending (E148)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `graph_prefix_pending` / `graph_node_pending`, fold with `graph_edit_with` | the caller edits the view in every goal | lookups merge the pending edits in memory; the view on disk lags until the next fold | E133's first goal: view update 405 KB to 22 KB, 150 ms to 37 ms on packs; one fold for two goals 445 KB against 744 KB, same root |

## Short searches from the postings alone (E149)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `exact_short_search` | off | on: patterns of one to three bytes are answered from the trigram postings without decoding the matches, so the answer rests on the index | E133's searches 655 KB to 269 KB and 351 KB to 269 KB (the index alone), same matches; a goal that then reads the files fetches the same total |

## Group size (E151)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `group_size`, `--group-size` | 64 KiB | smaller groups fetch less per member read and store more where small contents share context | 16 KiB: E133's file reads 385 KB to 208 KB and 82 KB to 53 KB; north score 0.3035 to 0.3100 (multiformat +4.6%, tables +1.8%); 8 KiB: 157 KB and 25 KB |

## Held search index (E152)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| a caller holds the decoded `search_index()` between goals (no kernel switch) | each caller loads it | memory for the decoded index against reloading it per goal; contents written since are scanned, so results stay exact | E133's next goal 358 KB to 89 KB (60 KB with 16 KiB groups), same matches and writes |

## Files as renders over fragments (E155 to E159)

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `put_fragmented(bytes, None)`, files as `render/1` over every paragraph block | files stored by `put_content` | one edit to a shared block reaches every file that renders it without re-planning them; +145% stored at rest (+74% with `short_records`) | 343 md files: 169,623 against 69,332 bytes after repack |
| `put_fragmented(bytes, Some(&shared))`, only shared blocks stand alone (render-runs) | off | +24% stored at rest for 26 to 40% fewer bytes per propagated edit, and faster edits at large fan-out | fan-out 256: 85.6 KB and 85 ms against 115.3 KB and 117 ms (index-rewrite); fan-out 64: 22.0 against 30.0 KB |
| `propagate_granted` with a goal's grant | `propagate` (no grant) | the grant is checked from fragment lengths, so a scoped edit costs no more than an unscoped one; the grant lists every file it reaches (about 140 bytes each) | n = 256: 81.9 ms granted against 88.7 ms ungranted; the diff check it replaces would add 17.5 ms |

## Not built, measured as worth building

| Option | Trade | Measured |
| --- | --- | --- |
| a fragment-scoped grant (the goal may change one fragment wherever it is rendered) | constant size against a per-file grant; needs a grant form that names a fragment | about 57 bytes against 35,772 for 256 files (E159) |
| a docx renderer that renumbers document-wide ids, so docx renders splice per fragment like html | a renderer to maintain against re-running pandoc | 206 of 236 blocks local once bookmark and list ids are normalised; 0 of 72 splices exact without it (E158) |
| rows as fragments for large tables | per-row references against one plain file | a one-cell edit of a 10,000-row table writes 12.7 to 16.1 KB today, against 164 to 363 bytes at 100 to 1,000 rows (E161) |
| a typed table IR (TableIR) stored instead of its csv, json and md renders | json and md rendered on read (json 99 ms at 10,000 rows) against storing them | 6,515 against 12,725 bytes for the 24 workbooks, all renders byte-exact (E161) |

## Fragment options built (E162 to E169)

These are the options E158 to E161 listed above as not built; each is now built and measured.

| Setting | Default | Trade | Measured |
| --- | --- | --- | --- |
| `propagate_fragment_granted` with a `FragmentGrant` (the goal may change one fragment wherever it is rendered, under a path prefix) | `propagate_granted` with a per-file grant | constant grant size against one entry per file; a new sealed grant form | 85 bytes at any fan-out against 35,772 for 256 files, same edit time (E162) |
| `put_cut` over groups of 64 rows (optionally as a tree) for very large tables | `put_content` | +45% stored and larger after repack, for faster one-cell edits; plain puts already write about one row | 100,000 rows: edit 60 ms (tree 115 ms) against 353 ms; plain adds about 168 bytes (E164) |
| a typed table IR (csv plus a residual sidecar) stored instead of csv, json and md | store each format | 43% fewer bytes for the text formats, for rendering json and md on every read | 241 ms to render at 10,000 rows against 6 to 11 ms to read (E165) |

Renderers and indexes outside the kernel (scripts and examples; no switch):

| Option | Trade | Measured |
| --- | --- | --- |
| docx renders spliced per edit (`scripts/docx_splice.py`, `scripts/docx_restructure.py`) instead of re-running pandoc | a renderer to keep in step with pandoc's numbering, against a full render per edit | 96/96 same-shape edits and 168/168 inserts and deletes byte-identical; deletes 2.8 ms, inserts 45 to 49 ms, against 65 to 77 ms (E163, E169) |
| an md edit carried to its html and docx by splices, stored as plain files | storing the html as fragments costs more for one file (the render names every piece) | 58 against 134 ms; 2,340 bytes per edit as files, 3,210 with html fragments (E167) |
| a compact fragment trigram index (positions, implied seams, crossing trigrams) | the fragment list must be held to map positions | 185,690 against 280,615 bytes, about the stored index's size, exact for any pattern length (E168) |
| a computed-table IR (cell values, number formats, formulas as shapes) lifted from the workbook (`scripts/formula_ir.py`) | a formula evaluator and format renderer to maintain, against re-running LibreOffice | 24/24 renders and 22/22 input edits exact; 0.12 ms against 872 ms (E180) |
| a shared paragraph's renders made once and spliced into every file that holds it (`scripts/fanout_prep.py`) | the per-file block layout kept from ingest | 64 files' html and docx: 194 ms against 8,477 ms, 128/128 exact (E181) |
| summaries per fragment, with corpus aggregates updated as edits propagate (`examples/fragment_insights.rs`) | a summary cache keyed by fragment id, and a word-to-fragment index | 22 edits 7.5 ms reading 5.7 KB against 481 ms reading 4.8 MB, always exact; goal queries read 4% of the bytes (E182, E183) |
| table aggregates kept fresh from the computed-table IR (`scripts/table_aggregates.py`) | per-workbook aggregates kept, and the IR's evaluator | exact 22/22 from one workbook's evaluation (E184) |
| summaries stored as derivations of a WASM function beside the fragments, in a `summaries` graph committed with the files (`examples/stored_summaries.rs`, `functions/summary.wasm`); off unless an application commits the view | about the text's size again in storage and 2 to 7 ms per edit, for summaries that survive restarts, gc and kills and can be re-verified | exact through 22 edits, gc, restart and 60 kills; no query gain on its own (E186, E188, E189) |
| a word index (postings lists) in the same view | 35 to 60 times the bytes written per edit and 3.5 times the storage, for a fast first query after a restart | 29 ms reading 30 KB against 548 ms reading 5.2 MB at 18,000 fragments (E187, E189); bucketed lists do not write less (E191) |


## IR codec options (E192 to E205)

`ikam_kernel::ir::Options`, all off by default; nothing in put or repack uses the codec. `decode(encode(files, o))` is byte-exact for any bytes and options (`tests/ir.rs`).

| Setting | Trade | Measured (against xz -9 over all files) |
| --- | --- | --- |
| templates (the codec with every option off): cells lexed out, each distinct line skeleton stored once | a full decode to read one file; worse on prose | reports 0.88 to 1.03; md+csv+json exports 0.69; prose 1.25 (E192) |
| `columns`: one cell stream per (line skeleton, slot) | stream overhead on small tables | reports 0.72 to 0.93; real tables 1.08 (E193) |
| `numbers`: numeric cells (`-$1,650,000.50`, `12.5%`) as deltas in their stream | none measured beyond encode time | reports 0.67 to 0.84 (E193) |
| `rows`: streams keyed by the line's first cell, so a report row lines up across runs | more streams; no gain when values do not repeat | reruns on the same data 0.591; random values 0.677 against 0.669 without (E193) |
| `siblings`: a file with the same stem stores its cells as a diff against the first one (canonical form across csv, json, md) | a diff per sibling at encode | exports 0.483; real tables 0.927 (E194) |
| IR parts stored in the kernel (four contents) | one-file reads decode everything (12 ms for 300 reports against 0.03 ms) | drift reports 10,362 B against 27,657 for the files, ingest 15 times faster (E195) |
| `encode_blocks(per)`: blocks of `per` files, one shared skeleton table, each block one kernel content | bytes grow as blocks shrink; a read decodes one block (about 0.03 ms per file in it) | drift, per 20: 1.227 of xz, 0.84 ms read; per 100: 0.756, 3.3 ms (E197) |
| `encode_blocks(.., keyframe)`: blocks after the first go on from the first block's streams | a cold read decodes the first block too | drift per 50: 0.858 against 0.904; exports per 60: 0.630 against 0.729 (E197) |
| `prose`: a file under a tenth of whose bytes are cells is stored as it is (raw part) | none measured | texts 1.000 against 1.249 without; mixed reports and prose 0.826 against 1.013 (E198) |
| `nearest: n`: cells diffed against whichever of the last n files with the extension holds the most of them | encode 2 to 4 times slower at 16; worse at n = 1 | drift blocks of 20 in the kernel 0.759 against 0.984; shuffled reports 0.770 against 0.906 (E199) |
| `predict` (with `columns`): each column learns which of its line's first 4 cells determines it; a right guess is one byte | decode about 2 times slower; no gain in small blocks | drift 0.543 against 0.591; exports 0.448 against 0.483 (E200) |
| `step` (with `predict`): a number column predicts its last value beside a key plus the step it took then | decode about a third slower (28 to 37 ms for 300 reports with xz) | drift 0.505 against 0.543; exports 0.436 against 0.448 (E204) |
| `group`: the streams an encode adds are sorted by key | none measured (encode sorts once) | with row keys 4 to 8% smaller; best overall only on reports-random, 0.638 against 0.654 (E201) |
| `ir::schema` thresholds (E202, read side, not a codec option): `min_rows` and `min_strength` for a dependency; blips on every dependency or only on those holding 97%+ | stricter: fewer columns covered | every dependency: drift 91% of covered errors found, 93% precision; 97%+: 98% and 94%, but covers 42 of 180 errors instead of 154 |
| Query-friendly layout (E203): columns and numbers only (no `siblings`, `nearest`, `predict`), each part one kernel content | drift 0.819 of xz against 0.543 for the smallest IR | one column of 300 reports in 0.6 to 1.1 ms against 18 to 25 ms reading the files; random reports smaller than the solid pack (0.643) |
| Query-friendly layout with `predict` and `step` (E205): siblings and nearest off, each part one kernel content | column queries about 13 ms instead of 1 ms (the predictors learn on every cell) | drift 0.548 of xz against 0.819; random 0.619 against 0.643 |
| `route` (E208): each kind of file (extension and first-line template) goes through the IR or is stored plain, whichever xz -6 judges smaller | encode 0.5 to 1.8 s for a few hundred files, 13 s for a 10.8 MB table (xz over each group twice, then up to three judgments of the whole set) | never worse than xz over the files on any corpus tried (worst 1.000, was 1.115); real cases text 0.995, cases with reports 0.940 |
| `tune` (E209): each stream chooses whether it uses number deltas, predicted cells and stepped numbers, judged by xz -6 per stream; kept only if the whole pack is smaller | encode 2 to 4 times longer (47 s for a 10.8 MB table); tuned on the whole set, so blocks can lose a little (random reports in blocks 0.712 to 0.722) | real registry 0.961 of xz (from 1.000 routed plain, best untuned 0.966); random reports 0.633 |
| `literal` (E210, with `tune`): a stream may write every string out instead of as a dictionary index | tune's encode 1.5 to 1.7 times longer | registry 0.961 to 0.960; nothing else moves (not worth turning on) |
| `sort` (E211): a CSV table's rows sorted by its best key column, that column also kept in file order | encode about 3 s more per 10 MB table; column queries refused on a pack with sorted tables | real registry 0.961 to 0.942 of xz |
| `transpose` (E212, E213): a CSV table stored plain is laid out by column, without the quotes of a column quoted in every row | a table so stored is not in column queries | real registry with `sort` 0.942 to 0.911 of xz; alone 0.961 to 0.954 |
| `sample` (E214): `route` and `tune` judge on large files cut to about this many bytes | a choice could differ from judging everything (none did on 6 corpora at 1 MB or 256 KB) | registry encode with `tune` 43 to 7 s, columns 13 to 5 s |
| `raw_keyframe` (E225, blocks with a keyframe): later blocks' raw parts as zstd frames with the first block's raw part as dictionary | a frame the store cannot compress again | worse in the kernel wherever used (cases blocks of 50 0.819 to 0.869 of the files); not worth turning on |
| `Repo::put_ir`, `put_ir_blocks` (E223, not an option: called explicitly): files stored as members of one IR pack or of blocks | one derivation record per file (30 KB for 549 files with `short_records`); the planner stores the pack with zstd, since it charges a pack decode per member read; first read 1 to 35 ms whole, 0.1 to 4 ms in blocks | north judged 0.914 against 0.920 for `short_records` alone; fixture cases 0.982, reports and exports worse than their files |
| `bases` (E216): `route` also judges per kind of file whether its files diff against a base | encode about 1.5x longer (a third judged way per kind) | with siblings and nearest: reports-drift 0.537 to 0.504, random 0.673 to 0.630, exports and tables kept at 0.458 and 0.936 |

One setting for every corpus tried (E216): `columns`, `numbers`, `siblings`, `nearest: 16`, `predict`, `step`, `prose`, `route`, `sort`, `transpose`, `bases`, `sample: 1 << 20`, no `rows`; `tune` on for the last 0.015. Against xz -9 over the files it is best or tied on all nine corpora and never worse than xz (drift reports 0.504, exports 0.458, real cases text 0.995, real registry 0.911). In the kernel (E217) it stores 0.23 to 0.92 of the files; blocks of 50 with a keyframe keep most of that with 1 to 10 ms reads. With `sample`'s runs (E219) 18 MB of reports encode in about a second untuned. As a per-file storage plan (E223, `Repo::put_ir`, files kept readable as members of the pack) it adds only 0.007 to the north score on top of `short_records`, so it is not a put or repack default; it pays where whole folders are written and read together. Not worth it: `literal` (E210), large tables cut into row groups (E220), block raw parts as their own entries (E221), `raw_keyframe` (E225).
