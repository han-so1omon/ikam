# Skills, tasks and elicitation for `ikam mcp`

The owner, Nov 24: "Plan it for ikam", for the MCP features Anthropic announced that day: skills over MCP, tasks, elicitation, and file uploads soon.

This is a plan; nothing here is built. Each entry below becomes one experiment, numbered with the next free E number when it runs. Each gets its own `exp/<id>-<slug>` branch, its log entry and its registry row (`docs/experiments/README.md`). What was read is in `docs/research/mcp-2026-07-28.md`.

None of M1 to M5 changes how the kernel stores data, so the fixed bench cannot move. Each still runs `cargo test --release` and clippy, and says that no score changed. A change with a trade-off is an option, off by default. Nothing merges without the owner.

## Where it starts (main at 437aef6)

- **`ikam mcp`:** a stdio server (E359, E378, E388) that reads one JSON-RPC message a line and answers each before reading the next.
  - Methods: `initialize` (versions 2024-11-05 to 2025-11-25, else 2025-06-18), `ping`, `tools/list`, `tools/call`.
  - Capabilities: `tools` only, with 17 tools.
  - Guidance lives in the `instructions` string and the tool descriptions, which every session takes in at start.
- **Missing:** resources, `server/discover`, request `_meta`, `resultType`, any way to ask the client something during a call. One long call holds up every other.
- **`ikam live` and `ikam host`:** they serve the same handlers as REST (`/api/tools`, `/api/tool/NAME`, E360), not as MCP. An MCP client reaches a running server's store only by starting its own `ikam mcp` process on it, and two processes on one store is not measured for `live`.
- **Work that takes a while:**
  - opening and indexing the store (384 ms at 111 MB; a store the size of enwik9 not measured through `mcp`);
  - building the file ranker (26.5 s at 1,656 paths, E388) and the passage index (18.3 s to ready, E378);
  - fsck, repack and gc (`host` queues them, E362);
  - embedding a corpus (about 0.6 s a long document on 4 CPUs, E387).
- **Steps that lose work or cross a project's edge:**
  - `draft_discard`, which drops a draft and its pending edits;
  - gc, which drops unreachable versions;
  - in `host`, a share's issue, accept and merge (E315 to E318, E365).
  - The owner's rule for the last group: projects are "separate by default but able to have scope access for granular data fragments and mergable on specific scope when authorized".

## M1 dual-era (2026-07-28 requests beside `initialize`)

- **Question:** can `ikam mcp` serve 2026-07-28 clients and keep every client of today unchanged? Which protocol era does Claude Code choose with it?
- **Hypothesis:** one dispatcher over the same `Session` handlers can do both (H1). A request whose `_meta` carries 2026-07-28 is served on its own, with no handshake. `initialize` keeps today's behaviour byte for byte.
- **Change (`mcp.rs`):**
  - `server/discover`: supported versions, capabilities with an `extensions` map, server info in `_meta`, instructions, `ttlMs` and `cacheScope`.
  - Each modern request is read with its `_meta` (version, client capabilities).
  - Errors: -32022 with the supported versions for an unknown version, -32602 for missing required `_meta`.
  - `resultType: "complete"` on modern results.
  - The legacy `initialize` result lists `capabilities.extensions` only when an extension's option is on.
- **Run:**
  - E359's `mcp_check.py`, unchanged (SDK 2.3.0, a 2025-11-25 client).
  - A 2026-07-28 client: the official Python SDK if PyPI has one that speaks it, else a client scripted from the specification's examples.
  - Claude Code on the Mac (after Nov 29): the era it uses by default, and with its setting that keeps stdio servers on the older protocol.
- **Pass bar:**
  - Legacy answers are byte-equal to main's for every call in E359's script; modern answers carry the same content.
  - Legacy and modern requests both work in one process, in either order.
  - An unknown version gets -32022.
  - No new tool and no new default behaviour.

## M2 skills (ikam's conventions, and a project's own)

- **Question:** E359's small task took in 4.7k to 14.8k characters, and every session also takes in the instructions and 17 tool descriptions. Can ikam serve its how-to as skills that a client loads only when needed? Can a project keep its own skills as versioned files in its store?
- **Hypothesis (H2):**
  - (a) Moving the usage guidance out of the instructions and descriptions into one built-in skill, `ikam-store`, cuts the bytes every session pays at start. The guidance covers revisions, drafts, paging with `start_char`, when to use `search`, `passages` or `rank`, and commit messages.
  - (b) Skills read from the store change with a commit and read back at an old commit through `read at=`. They reach another project only through a share of their path.
- **Change, `--skills [PATH]` (off):**
  - Declare `resources` and `io.modelcontextprotocol/skills` with `directoryRead`.
  - Methods: `skills/list` (with a cursor), `skills/get`, `resources/read` and `resources/directory/read`, over `skill://NAME/FILE`.
  - The built-in skill is a `SKILL.md` kept beside the code and compiled in, so it is reviewed like code.
  - With PATH, each folder under PATH in the session's ref head that has a `SKILL.md` is a skill. The ref head is used, not pending edits, so a half-written skill is never listed.
  - Each manifest lists every file with its `sha256:` digest and size. The spec names SHA-256, not BLAKE3, so this needs a small pure-Rust SHA-256 (the `sha2` crate, or a short local implementation).
  - A skill over 512 files or 16 MiB is refused and never cut. A folder whose name differs from its frontmatter `name` is not listed.
  - With the option on, the instructions shrink to two lines and point to `skill://ikam-store/SKILL.md`, so a client without skills can still read it as a resource.
  - In `host`, a project lists only its own skills.
- **Run:**
  - A scripted client lists the skills, gets each one and reads every file, then compares each digest with the SHA-256 of the bytes served.
  - Edit a store skill, commit, and list again.
  - Measure the bytes of `initialize` plus `tools/list` with the option off and on. They are deterministic.
  - On the Mac: whether Claude Code lists `ikam-store` and loads it when asked to edit a file. Then E359's rename task with and without the skill: calls, failed calls and characters taken in. The sample is small, so these are observations, not a score.
- **Pass bar:**
  - Every digest and size listed equals the bytes served.
  - A changed skill shows its new digest at once, and its old bytes still read at the old commit.
  - A skill over a limit is refused with the reason.
  - With the option off, `initialize` and `tools/list` are byte-equal to main's.
  - Report the bytes saved at start.

## M3 tasks (long jobs past a client's timeouts)

- **Question:** Claude Code's documentation says it moves a call still running after 2 minutes into the background, and aborts a stdio call with no answer for 30 minutes. `ikam mcp` answers one request at a time, so a long job also stalls every other call. Can long jobs run as MCP tasks, with the other tools still answering, past those timeouts, and with nothing acknowledged lost on a kill?
- **Hypothesis (H3):**
  - Each task gets a job thread, with the `Live` behind a lock as `ui.rs` already holds it.
  - Task state is a Petri net in the store. A task is a token; `working`, `input_required`, `completed`, `failed` and `cancelled` are places.
  - Each status change is a firing committed before it is acknowledged, as E386's `loop_drive` does (0 of 26 acknowledged lost in 600 kills, replay equal). That gives the spec's durability, and its rule that a finished task never changes, by replay.
  - It is measured against (A), a task table in memory: cheaper, but lost when the process exits.
- **Change, `--tasks memory|store` (off):**
  - The server decides per call. Tasks are only for calls that can run long:
    - a new read tool `fsck`;
    - `rank` and `passages` when their index is not built yet;
    - `commit` over a stated number of pending bytes;
    - later, a net transition's `cmd` run.
  - Only a client that declared the extension on that request gets a task. Every other client gets today's blocking answer.
  - Methods: `tasks/get`, `tasks/update` and `tasks/cancel`. Cancel is cooperative: jobs check a flag between files.
  - Timing: `ttlMs` of one hour, with finished results kept that long, and a `pollIntervalMs`.
  - Ids are 128 random bits from the system's source, as `tokens.rs` makes tokens.
  - With an authorization context (M5), a task is bound to its caller. Anyone else gets "not found".
  - With `store`, a task found `working` after a restart becomes `failed`, with a message that its process ended.
  - Nothing here knows about the self-improvement loop or the night net. A net may start a task like any caller, and the loop stays configured, not coupled.
- **Run:**
  - A store the size of enwik9: E390's data on the Mac, or a generated corpus of the same size in the cloud.
  - Run fsck and a first `rank` as tasks. While each runs, make 1,000 reads and searches.
  - Kill the process at random points 200 times, restart, and `tasks/get` every acknowledged id.
- **Pass bar:**
  - During a job, other tools answer with a p50 within 2 times of idle, and `tasks/get` answers in under 5 ms.
  - A client that did not declare tasks never gets one.
  - A finished task never changes: net replay equal, fsck clean.
  - With `store`, 0 acknowledged tasks are lost.
  - Cancel stops a job within 1 s of its next check.
  - An expired task answers -32602.
  - Report bytes and ms per task for both (A) and (B). Which becomes the default is the owner's decision.
- **Not covered:** `notifications/tasks`, which the spec makes optional. Polling comes first.

## M4 elicitation (a person confirms steps that lose work or cross a project)

- **Question:** which steps should stop and ask the person, and can an answer be made hard to forge or replay?
- **Hypothesis (H4):**
  - A gated call answers `input_required` with one `elicitation/create` and a `requestState`. The state is a keyed BLAKE3 hash (already a dependency) over the principal, the tool, a digest of its arguments, an expiry and a nonce.
  - The retry is accepted once: the nonce is recorded as a firing.
  - Two strengths:
    - **Form mode confirms intent.** For example, "type the draft's name to discard it", as GitHub asks for a repository's name before deleting it.
    - **URL mode can carry an authorization.** It sends the owner to the host's own sign-in page to fire the human transition that authorizes a merge across projects: Thursday night's authorization step on E365's scoped merge. The server checks that the person who signs in is the one who asked.
  - Form mode alone is not authorization, because a client could answer for the person.
- **Change, `--confirm` (off, and off means today's behaviour):**
  - Gated steps: `draft_discard` with pending edits, `remove` of a folder over a stated number of files, gc, and in `host` a share's issue, accept and merge.
  - A client that cannot elicit gets a tool error that names the step and the owner page where it can be confirmed. The step is never done silently.
  - Inside a task (M3), the same request is asked through `input_required` and answered with `tasks/update`.
- **Run:**
  - A scripted client tries each case: accept, decline, cancel, the wrong name, a tampered state, a replayed state, an expired state, and a URL confirmation by a different user.
  - On the Mac: what Claude Code shows the person for each mode.
- **Pass bar:**
  - Every gated step is refused without a valid, fresh answer.
  - Decline and cancel leave the store byte-equal, with fsck clean.
  - A replayed or tampered state is refused.
  - The URL step is refused for anyone but the person who asked.
  - Ungated tools are unchanged.

## M5 MCP over HTTP on `live` and `host`

- **Question:** E360 left the agent tools of a running server as REST, so an MCP client still starts its own process on the store. Can `live` and `host` answer MCP itself, over 2026-07-28's stateless HTTP POST?
  - Claude Code would then use the running server.
  - `host`'s users and tokens (E363) become the authorization context that tasks and confirmations bind to.
- **Change:** POST `/mcp` on `live` and `host`, through M1's dispatcher, with `/api/tool`'s roles: a reader gets the read tools, only an owner promotes.
- **Pass bar:**
  - Same answers as stdio and `/api/tool`, with reads near E360's 1 ms p50.
  - A reader cannot write.
  - A task or a request state made under one token is refused under another.
  - One project's skills and tasks never show to another project.
- **Note:** Claude's hosted connectors connect from Anthropic's servers, so a server on a private network needs a way in. Anthropic's 2026-07-28 article mentions MCP tunnels in research preview, not read. M5 is first for clients on the same machine or network as the server.

## M6 file uploads (waits on the spec)

- **Why:** `write` takes text, so an agent cannot store a docx, pdf or image through `ikam mcp` (E359's not-measured line). That is the owner's case for ikam: "the service that supports projects that use ai to do file generation / manipulation".
- **Change:** when the file-uploads extension is published, `write` takes an uploaded file.
- **Pass bar:**
  - A byte-exact round trip for each type.
  - `read` gives the file's extracted text.
  - Two formats of one text share storage, as `north/multiformat` measures.
- No work before the spec is published.

## Not planned

- **Tool search:** it happens in the client. ikam only needs specific tool descriptions.
- **Prompts, sampling and roots:** not asked for.

## Order

1. **M1:** M2, M3 and M4 need `server/discover` or request `_meta`, and M1 changes nothing for today's clients.
2. **M2:** the smallest change that every agent session gains from.
3. **M3**, then **M4:** M4's in-task half needs M3.
4. **M5**, then **M6**, which waits on the spec.

All of M1 to M4 can be checked in the cloud with a scripted client. Only the "what Claude Code does" lines wait for the Mac.
