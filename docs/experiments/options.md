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
