---
name: ikam-store
description: How to work in an ikam store through its MCP tools - reading at a revision, paging long files, finding text, editing, committing and drafts. Load it before editing files in an ikam store.
---

# Working in an ikam store

An ikam store is a versioned file store. The tools read and change one
working tree on one ref (usually `main`).

## Revisions

- Tools that read take a revision: `at` for `list` and `read`, `from` and
  `to` for `diff`.
- A revision is a commit id or a prefix of one, a ref name, or `work`, the
  working tree with its pending edits. `work` is the default.
- `history` gives commit ids; `read` with `at` set to one shows a file as it
  was then.

## Reading

- `read` returns lines numbered from 1. Ask for a range with `start_line` and
  `end_line` rather than a whole large file.
- An answer is at most 64 KiB and says where to read on. For a single line
  longer than that, continue with `start_char`.
- Binary files (docx, xlsx, pdf, tar members) are read as their extracted
  text.

## Finding

- `search`: files whose text contains a string, case-insensitive, with the
  matching lines. Use it for a name, an identifier or an exact phrase.
- `passages`: paragraphs that share the most words with a question. Use it
  for a question in plain words.
- `rank`: whole files best first, each with its best passages. Use it to
  find which files are about something. Its first call builds the ranker,
  which takes a while on a large store.

## Changing

- `write` replaces a whole file; `edit` replaces text that must occur
  exactly once (or every occurrence with `all`). Read the lines first so that
  `old` is exact.
- `remove` and `move` take files or folders.
- Every change waits in the working tree until `commit`. Nothing commits on
  a timer. Check with `status`, and with `diff` and a `path` for one file.
- Commit with a message that says what changed and why, in one line. Changes
  still pending when the session ends are committed with a generic message,
  so commit your own work yourself.

## Drafts

- `draft_open NAME` moves the session onto a draft ref; edits and commits
  there stay off the ref you started on.
- `draft_promote` commits pending edits and merges the draft back;
  `draft_discard` deletes the draft and its pending edits. Discarding loses
  work: do it only when that is what the person wants.

## Errors

A failed tool call comes back as a tool result marked as an error, with the
reason. Read it and fix the call; do not retry it unchanged.
