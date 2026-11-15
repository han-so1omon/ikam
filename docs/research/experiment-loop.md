# An experiment loop on IKAM

The question: can the loop in `docs/experiments/README.md` (question, read, hypothesis, branch, run, record, keep or discard) run with little input, for example overnight, with IKAM itself holding its state? That needs a queue of candidates (gaps), a way to rank them, a scheduler that knows what may run unattended, and a representation of the loop that survives restarts and replays exactly.

## Sources read

- **The Python-era orchestration** (removed in a992b77, still in history at `a992b77^`). Read in full: `interacciones/schemas/petri.py` and `workflows.py` (an authored workflow of typed nodes, compiled to a Petri workflow of places, transitions and arcs, with validators and approval and checkpoint hints), `interacciones/workflow/compiler.py` (one transition per node, one place per link, start and end places), `modelado/graph/compiler.py` and `lowering_petri.py` (each transition lowered to an IR operator, the net to an executable graph), and `modelado/plans/engine.py` (`PetriNetEngine`: a transition is enabled by its input tokens, a retry budget and equality guards; each firing records content hashes of the marking before and after). The Rust kernel has none of this yet; `docs/plans/2026-10-02-rust-kernel.md` lists "Scheduling: Petri nets over derivations" as planned, and `docs/plans/2026-10-04-graph-objects.md` notes that a Petri net fits a graph object as a bipartite graph with markings as content targets.
- **The autoresearch family of loops**, from the index at github.com/webfuse-com/awesome-autoresearch (read the index, not each project). Common shape: a fixed task and scorer, an agent proposes one change, keep or revert by the score, an append-only ledger, isolated branches or worktrees; some add plan approval before a run, resumable state, a "director" that injects ideas when a loop stalls, and prioritized sequences of goals. IKAM's README loop already has the fixed scorer, the gates and the ledger; it lacks the queue and the scheduler.
- **OxyMake** (arXiv 2606.20989, read the abstract and most of the text): a Rust workflow engine whose cache key is a BLAKE3 hash of the rule source, inputs, parameters and environment, so a rerun with the same key is skipped. Maps onto recording effectful firings: a firing whose input hash matches a recorded one can reuse its result.

## Design (generic)

- **The net:** places and transitions as a graph object; tokens colored by candidate id; resource places (a nightly budget, CPU slots, a run window). Firings are database events with an idempotency key from the transition, the consumed tokens and the marking before, so a retried firing is a no-op; the marking is a view over them and must equal a replay from zero.
- **Four kinds of transition:** pure (ranking, the keep rule, guards: recomputed on replay), command (the fixed bench and tests, from an allowlist in the net: output recorded), agent (reading, proposing, writing code: output recorded, never regenerated, as the kernel plan says of effectful runs), human (merges and anything that needs an owner's yes: never fired by the engine).
- **A text form** compiled to a checked net (bipartite, ids unique, budgets finite, human transitions without effects), then lowered to the graph, an incidence table (only transitions next to changed places are rechecked), the event schema and the marking view.
- **Projects keep their own nets and capabilities** in their own repositories; IKAM holds the engine and its own loop.

## Hypotheses

- **H1. The records hold the queue.** Stale verdicts, "Not measured" lines, open hypotheses and ids mentioned ahead of time list the candidates. Status: confirmed in part by E275 (85 gaps; 8 of 10 ids named ahead of time were run within 20 experiments; matching "Not measured" lines by citation and shared words scores the same as its swap control).
- **H2. A value-per-hour score ranks them better than the order they were run in.** score = P(keep) x gain + 0.5 x info, over hours, with P(keep) a Beta prior per mechanism from the registry. Status: refuted by E276 (failures' mean queue position 0.40 against 0.50 for a random order and 0.35 for the run order; mechanism keep rates do not predict verdicts). Rank by dependencies, information, the owner's theme boosts and, once runs record them, gain and hours, with a seeded random tie-break.
- **H3. Execution modes for nets.** The same checked net run interpreted, compiled to the stack machine already stored as `functions/interp.wasm`, compiled through Cranelift, or lowered through MLIR (from Rust, the melior bindings), compared on firing cost, compile cost, determinism on replay and dependencies added. Status: `open` (E277; read MLIR's dialect and pass design and the Rust options first).
- **H4. The net core replays exactly.** Status: confirmed by E269 (10,000 firings replay to the stored marking; retries safe; fire p50 8.6 ms in memory, growing with the log and the tokens held).
- **H5. The text form round-trips and refuses malformed nets.** Status: `open` (E270).

## Unknowns

- Whether "Not measured" lines are rarely taken up or only hard to detect [inf: both; the word test cannot tell them apart].
- What an unattended run may write: branches only, under `exp/`, after the privacy check, and never to main; decided by the owner, not the loop.
