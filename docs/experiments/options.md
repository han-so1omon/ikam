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
| `short_records` (new, E104) | off | derivation records store 8-byte id prefixes and a builtin index: about a third less record bytes; reading resolves prefixes (a two-pass ledger build) | north 0.3035 -> 0.2793 (8%); cost score up about 4.5% on four runs with identical instruction counts (timing, not work); see below |

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
- **Real-world corpora**: every number above is on the bench's fixtures and generated corpora, not on files nobody made for the kernel (E105 measures those).

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
