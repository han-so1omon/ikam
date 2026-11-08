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
| `IKAM_AUTH` (E111) | unset | a password on every request against one more thing to keep secret; the hash is fast (BLAKE3), so the line belongs in a file only its owner reads | not scored |

**Not yet a setting**: an incremental repack that re-plans only what was
added since the last one (the live drift is in new content; a full repack
re-plans all of it).
