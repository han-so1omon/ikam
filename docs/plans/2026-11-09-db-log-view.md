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
