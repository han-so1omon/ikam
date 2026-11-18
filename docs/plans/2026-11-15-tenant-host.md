# Separate ikam projects, shared by scope

The owner, Nov 15: several services should use ikam with isolation. Revised
the same day, in the owner's words: projects "separate by default but able to
have scope access for granular data fragments and mergable on specific scope
when authorized. Else graphs are completely separate."

This note is the generic design for the kernel and its server. Which
services are projects, and where they run, are deployment choices kept
outside this repository. Shares (E315 to E318) and the host (E313, E314) are built and opt-in; E348 to E351 are not.

## Where it starts

- `ikam live` serves one store, one ref and one user (`IKAM_AUTH`), with
  `--max-body`, `--max-store`, `--max-conns`, `--public` and `--llm`.
- `ikam db serve` (E229) serves one store's databases (`db/NAME` refs) with a
  write user and a read user (`IKAM_READ_AUTH`).
- Section grants (E140) seal byte ranges of file versions with a keyed hash;
  fragment grants (E155) do the same for one fragment within a path.
- `Repo::merge` (E265) merges branches of one store: files three-way by path,
  database events since the fork, conflicts reported and never resolved.
- Nothing crosses stores today.

## Separate by default

- **Project = one store folder:** its own objects, refs, ledger, index and
  graphs. No object is shared or deduplicated across projects, so no project
  can learn from sizes or the `X-Ikam-Cost` header whether another holds the
  same bytes.
- **Project host (`ikam host --config FILE`):** serves one or many projects.
  A host name maps to a project (built in E313: the names in the config and
  `NAME.localhost`; a path prefix such as `/p/<project>/` was dropped, since
  the pages and WebDAV answers of `live` use absolute links). A project the
  caller may not see answers exactly like one that does not exist.
- **Roles per project:** owner, write, read, each from `ikam passwd` lines.
  Owners issue tokens for a ref or path prefix with an expiry, sealed with the
  project's key.
- **Limits per project** (body, store size, connections, request rate), and
  one queue that runs heavy jobs (repack, gc, fsck, reindex, ask) one at a
  time across projects.
- **Fault isolation:** a request runs under `catch_unwind`; a panic or failed
  fsck marks that project degraded and the rest serve on. Stores open on
  first use and close when idle.
- **Settings per project**, each off unless set. `--public` is refused in the
  host: a no-password copy stays its own process and never takes part in a
  share.
- One process per project stays supported; isolation never rests on the
  process boundary alone.

## Scopes

A scope names exactly what a project exposes, at one of five grains:

| Grain | What | Builds on |
|---|---|---|
| fragment | fragment ids, or the fragments rendered under a path | `FragmentGrant` (E155) |
| section | byte ranges of one file version | `Grant` (E140) |
| path | the files under a prefix | trees |
| records | a key prefix or range in one database | `db_range`, `db_prefix` (E229) |
| graph | the nodes under a prefix in one graph | `graph_prefix`, `graph_range` |

A scope is pinned to a commit by default; following a ref is an option.

## Shares

A share is `(source project, scope, receiving project, right, expiry)`, with
right one of:

- **read:** the receiver may read the scope through the host;
- **propose:** the receiver may write edits inside the scope, which land as a
  branch in the source project (write ranges as in grants);
- **merge:** the receiver may take the scope into its own graph (below).

A share is sealed with the source project's key and recorded as an event in
the source project's own log (`shares` database), so it is listed, audited,
backed up and revoked there. A read through a share returns only the scoped
bytes and never a storage object: a group, dictionary or delta object can
hold bytes of several files (E126), so checks stay at the file, fragment,
record and node layer. Anything outside the scope answers as if it did not
exist.

Search and ask run over a project's own graph. With a read share they may
also run over the shared scope; every hit is marked with its source project
and none comes from outside the scope.

## Scoped merge

- A merge copies the scope's content into the receiving project's store as
  one commit, with provenance: source project, scope and source commit. The
  graphs stay separate; the receiver deduplicates only within itself.
- It needs a share with the merge right **and** an authorization of that
  merge by the owner (per merge by default; standing authorization per pair
  of projects and scope is an option).
- A later merge of the same scope is incremental: three-way against the last
  merged source commit, as `Repo::merge` does within one store. A change on
  both sides is a conflict, reported, nothing written.
- Revoking a share stops reads, proposals and future merges at once. Content
  already merged stays, because it is a copy.

## What else the server needs (HTTP gaps still open)

1. Read at a commit: `GET .../dav/<path>?rev=<commit>`, `.../api/log?path=`.
2. Commit with a message and author, answering the commit id.
3. Write to a named ref, per request.
4. `live` and `db` routes in one process.
5. Net routes (fire, lease, beat, journal) so a worker can use a project over
   HTTP instead of opening a store folder.
6. A snapshot hook per project (flush, then a consistent copy) for backups.

## Experiments to prove it

In the repository's experiments format; numbers checked against the registry
when each starts.

| Id | Question | Measure |
|---|---|---|
| E313 | What does one host cost against one process per project? | p50/p99 per route, memory per idle and busy project, start time, 5 and 50 projects |
| E314 | Can a project reach another's data without a share? | cross-project paths, `..` and encoded paths, Host spoofing, expired or out-of-scope tokens, read role writing: all refused; hidden and missing projects answer alike |
| E315 | Can a share be stretched past its scope? | one byte past a section, a stale version, a fragment outside the scope, a key past the prefix, a node outside the graph prefix: all refused; cost per scoped read |
| E316 | Does a scoped merge copy exactly and merge again cleanly? | byte-exact copy with provenance, incremental re-merge, conflicts reported, proposal branches |
| E317 | Do revoke and expiry take effect at once? | reads, proposals and merges refused from the next request; merged copies read back and pass fsck |
| E318 | Can search through a share leak? | no hit outside the scope, with near-identical text just outside it |
| E348 | Does one project slow or break another? | import, repack and damage in A; B's p99 and uptime, with and without the job queue |
| E349 | Are limits enforced per project? | 413, 507 and 429 at the set sizes and rates, other projects unaffected |
| E350 | What does firing over HTTP cost? | net firing remote vs in-process (E293), with leases and beats (E303, E304) |
| E351 | Can a project be backed up while it serves? | snapshot time, byte-exact restore, write latency during it |

Gates as for every experiment: byte-exact reads, search equal to a scan,
clean fsck, in every project.
