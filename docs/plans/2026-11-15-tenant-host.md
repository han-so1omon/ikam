# One ikam service for several tenants, kept apart

The owner, Nov 15: plan how several services can use the same ikam service,
with isolation. This note is the generic design for the kernel's server side.
Which services are tenants, and where the service runs, are deployment
choices kept outside this repository. Nothing here is built yet.

## Where it starts

- `ikam live` serves one store, one ref and one user (`IKAM_AUTH`), with
  `--max-body`, `--max-store`, `--max-conns`, `--public` and `--llm`.
- `ikam db serve` (E229) serves one store's databases (`db/NAME` refs) with a
  write user and a read user (`IKAM_READ_AUTH`).
- A deployment that wants several private stores runs one process per store,
  each with its own password and port. That is strong isolation, but every
  process holds its own index and cache, and each needs its own supervisor,
  port and backup job.
- Section grants (E140) already seal a scoped permission with a keyed hash.

## What it is

`ikam host --config FILE`: one process serving many **tenants**.

- **Tenant = one store folder.** Its own objects, refs, ledger, index and
  settings. No object is shared between tenants by default.
- **Routing.** Every route of `live` and `db serve` under `/t/<tenant>/`
  (`/t/<tenant>/dav/...`, `/t/<tenant>/api/...`, `/t/<tenant>/db/NAME/...`),
  or a host name mapped to a tenant in the config. An unknown tenant answers
  404 whether or not it exists elsewhere.
- **Roles per tenant:** owner (repack, gc, fsck, settings), write, read. Each
  is one or more `ikam passwd` lines in the config.
- **Scoped tokens:** issued by an owner for a ref prefix and/or path prefix,
  a role and an expiry, sealed with a per-tenant key the way grants are.
  Loop workers get `exp/` and `beat/` only; a review page gets read.
- **Limits per tenant:** body size, store size, connections, requests per
  second. Heavy jobs (repack, gc, fsck, reindex, ask) from every tenant go
  through one queue that runs one at a time.
- **Failure isolation:** a request runs under `catch_unwind`; a panic or a
  failed fsck marks that tenant degraded (503 for it) and the rest serve on.
  Stores open on first use and close after an idle time.
- **Settings per tenant:** deferred sync, packs, hot/cold, fusion, `--llm`,
  each as today and off unless set. `--public` is refused in host mode: a
  no-password copy stays its own process.
- **Config file:** readable only by its owner; holds tenants, folders,
  passwd lines, token keys and limits. Never in the repository.

## Choices made here (options for later)

- **Shared object pool** (off): tenants marked as one trust group share one
  object store, so near-copies across them are stored once (the north
  star). Off by default because a shared store leaks: `X-Ikam-Cost`
  new_bytes tells a writer whether another tenant already holds the bytes.
  With the pool on, that header is withheld.
- **Path vs host routing:** both; path is what a single private name needs,
  host names suit a proxy in front.
- **One process vs one per tenant:** one process is the new option; one
  process per store stays supported.

## What else the server needs (from the HTTP gaps still open)

1. Read at a commit: `GET .../dav/<path>?rev=<commit>`, `.../api/log?path=`.
2. Commit with a message and author, answering the commit id.
3. Write to a named ref, per request.
4. `live` and `db` routes in one process.
5. Net routes (fire, lease, beat, journal) so a worker can use a tenant over
   HTTP instead of opening a store folder.
6. A snapshot hook per tenant (flush, then a consistent copy) for backups.

## Experiments to prove it

In the repository's experiments format; numbers checked against the registry
when each starts.

| Id | Question | Measure |
|---|---|---|
| E313 | What does one host cost against one process per store? | p50/p99 per route, memory per idle and busy tenant, start time, 5 and 50 tenants |
| E314 | Can a tenant reach another's data? | cross-tenant paths, `..` and encoded paths, Host spoofing, expired or out-of-scope tokens, read role writing: all refused |
| E315 | Does one tenant slow another? | tenant A ingest, repack and a large db import; tenant B p99 with and without the job queue |
| E316 | Does one tenant's failure stay its own? | panic and damaged objects in A; B keeps serving; A recovers on restart |
| E317 | What does the shared pool save, and is the leak closed? | bytes vs separate stores on pairs of corpora; cost-header oracle shown, then gone with the header withheld |
| E318 | Are limits enforced per tenant? | 413, 507 and 429 at the set sizes and rates, other tenants unaffected |
| E319 | What does firing over HTTP cost? | net firing remote vs in-process (E293), with leases and beats (E303, E304) |
| E320 | Can a tenant be backed up while it serves? | snapshot time, byte-exact restore, write latency during it |

Gates as for every experiment: byte-exact reads, search equal to a scan,
clean fsck, in every tenant.
