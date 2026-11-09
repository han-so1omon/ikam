# An event log and views in the kernel (step 1 of the database roadmap)

Eric, Nov 9: start step 1 of the five-step roadmap for ikam as a database as
well as a file store. Step 1 is the append-only log in the kernel, idempotent
appends and one last-write-wins view, with tests for exact read-back, rebuild
equality and reads at older commits, and the April fold tests as laws. Kernel
only; no on-disk format change.

## Where it comes from

The April Python layer (removed in b1ce98f, read at `b1ce98f^`) had a
project-scoped edge event log in Postgres with idempotency keys
(`graph_edge_event_log.py`, `ON CONFLICT ... DO NOTHING`), and a fold that
kept the last write per identity, independent of the event's idempotency key,
with subtree deletes (`graph_edge_event_folding.py`). Its tests are ported in
`packages/kernel/tests/db.rs`.

## What it is

`src/db.rs`, built only from objects the kernel has (content, `G` graphs,
`T` trees, `C` commits):

- **Event:** `key` (identity: `/`-joined segments, none empty), `op`
  (`Upsert(bytes)` or `Remove { subtree }`) and `idempotency_key`. Stored as
  content with a fixed, strictly decoded encoding; the value is its tail.
- **Log:** a graph whose labels are 20-digit sequence numbers and whose
  targets are events.
- **`ids` view:** idempotency key to event. The same event again is skipped
  (no root changes); a different event under a logged key is
  `Error::IdempotencyConflict` and nothing is appended.
- **`kv` view:** last write wins per key; a value is a byte range of the
  event that wrote it, so there is no second copy (an empty value is the
  empty content, since ranges may not be empty).
- **Commit layout:** the tree entry `db` holds `log` and `views/{ids,kv}`;
  `db_commit` keeps every other entry of the ref's tree.
- **API:** `db_empty`, `db_append`, `db_get`, `db_scan`, `db_events`,
  `db_rebuild`, `db_commit`, `db_at`.

## Choices made here (options for later)

- **Idempotency keys are required.** April computed one from the event when
  none was given, including its time. Without a time, a derived key would
  merge a repeated write (`a=1, a=2, a=1`) into the first and leave `a=2`.
- **A reused key with different bytes is an error.** April did nothing and
  returned the stored row. An error is stricter; silent skip is a one-line
  change if retries with changed payloads should be accepted.
- **No clock in events.** Time is nondeterministic; a caller that needs it
  puts it in the value. Recorded clock and model output is step 5.
- **Qualifiers live in the key** (`env:dev/...`), not in a separate field.
- **Every append rebuilds the three graphs** with `put_graph`, so an append
  costs time proportional to the database and writes new chunks along every
  changed path, plus intermediate versions that gc drops if uncommitted.
  In-place prolly edits are the fix, measured separately.

## Measured (append_cost, ignored test, release, bench running alongside)

| Events | Batch | Total | Last batch | Objects | Stored |
| --- | --- | --- | --- | --- | --- |
| 1,000 | 1 | 6.2 s | 8.5 ms | 16,068 | 3.3 MB |
| 1,000 | 100 | 0.21 s | 22.7 ms | 1,225 | 194 KB |
| 10,000 | 1,000 | 1.9 s | 202 ms | 10,710 | 1.6 MB |

Events are about 220 bytes over 500 keys; nothing was committed or gc'd, so
"stored" includes every intermediate graph version.

## In-place edits and view positions (E119, branch exp/db-inplace)

Eric, Nov 9: build in-place graph edits, with each view's log position
recorded, on a branch from exp/db-log-view.

- `Repo::graph_edit(root, edits)` re-cuts only the chunks the edits fall in,
  continuing until a new boundary lands on an old one, then does the same one
  level up. It reads every chunk above the leaves (small) and only the leaves
  it edits. Its root equals `put_graph` of the edited node set: tested on
  graphs of height 0, 1 and 2, growing, shrinking and emptied.
- `db_append` edits the three views in place, looks up a batch's idempotency
  keys in one pass (`graph_nodes_in`) and finds a subtree remove's keys with
  `graph_prefix`. `db_rebuild` is now an independent full fold with
  `put_graph`, so the rebuild law compares two implementations.
- Each view records its log position (`Db::ids_at`, `Db::kv_at`), committed
  as the files `db/views/ids.at` and `db/views/kv.at`; a commit without them
  reads as the log's end. Appending to views behind the log is refused.
- Label-key clashes are checked only within the chunks rewritten.

| Case | Before (E118, rerun) | After |
| --- | --- | --- |
| 1,000 single appends | 6.3 s, last 9.5 ms | 3.1 s, last 3.0 ms |
| 1,000 in batches of 100 | 0.25 s | 0.20 s |
| 10,000 in batches of 1,000 | 2.1 s, last 214 ms | 1.8 s, last 189 ms |
| 10,000 single appends | not run (about 10 min by extrapolation) | 48 s, last 4.7 ms |

Objects and bytes written are the same before and after: unchanged chunks
were never rewritten. Most of what is left is the write path, not the
graph: each new chunk is stored as a slice of its earlier version when that
is smaller, and searching for that slice is most of an append. With that
search off (read weight 1.0, measured, not committed), 1,000 single appends
take 0.9 s but store 7.9 MB instead of 3.3 MB, and 10,000 take 23 s and
130 MB instead of 47 MB. That trade is the background integration idea:
store new chunks plain on the fast path and let repack find the slices.

## Loose ends fixed (E120, branch exp/db-inplace)

Eric, Nov 9: do both fixes from the loose ends in the design doc.

- **Label clashes.** `Keys::Any` lets a graph's label keys repeat;
  `put_graph_with` and `graph_edit_with` take it, and agree on every graph,
  clashing keys included. The `ids` and `kv` views use it, since their
  labels are chosen by clients and no edge points into either; nodes are
  found by label, so a repeated key changes nothing there. The log keeps
  `Keys::Unique`: its labels are sequence numbers, so a clash can only be
  chance, and `db_rebuild` (`put_graph`) checks its whole key set.
  `graph_edit` with `Keys::Unique` still checks only the chunks it
  rewrites. No format change, and no existing root changes.
- **A real clashing pair.** `ke32051949435c933` and `kb4a1e63c25ad68d4`
  share their 8-byte key (`2402028764400e0e`), found by a distinguished-point
  collision search in about 10 minutes on 4 cores. Tests use it in a graph
  (refused by `put_graph`; kept by both functions with `Keys::Any`, in one
  chunk or two) and in a database (as keys and idempotency keys: read back
  exactly, retries are no-ops, views equal a rebuild). With the views on
  `Keys::Unique`, the database test fails.
- **Path-only reads.** `graph_edit` no longer loads every chunk above the
  leaves: it walks down from the root to each chunk an edit falls in and
  moves right through the tree only while the re-cut hasn't fallen back
  into step, reading each upper chunk at most once per edit. Its output is
  unchanged (every in-place equality test passes as before). A new test
  reads at most twice the height in upper chunks for one edit in 150,000
  nodes; the old code read all 11, some twice, and fails it.

One edit to a kv-shaped graph (`edit_cost`, ignored test; numbers from the
same code run as a scratch program, release, in memory):

| Keys | Upper chunks | Before: time, reads | After: time, reads | Upper chunks read |
| --- | --- | --- | --- | --- |
| 1M | 131 | 10 to 11 ms, about 310 | 2.1 to 2.3 ms, 45 to 55 | 3 |
| 10M | 1,321 | 94 to 105 ms, about 2,700 | 2.2 to 2.4 ms, 45 to 47 (one of three: 8.8 ms, 183) | 3 |

The reads left are the leaf and the slice search when new chunks are
stored, as before. Append timings are unchanged at these sizes (1,000 single
appends 3.3 s; 10,000, 48 s), since their views have few upper chunks.
