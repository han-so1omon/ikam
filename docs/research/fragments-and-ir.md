# Research note: fragments, edit propagation and intermediate representations (E153 to E161, 2026-11-10)

**[read]** means I read the text myself. **[inf]** means my own inference. OpenResearch (`orx`) cannot be installed in the cloud environment, so the experiments follow the README's git-branch loop by hand. The sources here are this repository's own history: the Python packages removed in b1ce98f ("Remove the Python packages"), read at its parent `b1ce98f^`.

## Question

Eric asked (Nov 9) for experiments on efficient edits that propagate to many render endpoints (files) in a corpus, using the fragment concept from the Python version; on efficient fragmentation; on insight generation and querying; and on intermediate representations, all from the repo's history. What did the Python version define, and which parts earn a place in the Rust kernel?

## What the Python version defined [read]

- **Fragments (V3):** `fragment_id`, `cas_id`, `value`, `mime`. An artifact is the root fragment of a DAG of fragments; relations are fragments too.
- **Surface fragments** by MIME type: heading, paragraph, code block, list item, formula and value cell, table region, slide shape, pdf page. `boundary_policy.py` set per-MIME cutting rules (markdown: at most 1,800 characters, full coverage of the source).
- **IR fragments** (`ikam/ir`): `PropositionIR`, `StructuredDataIR` / `TableIR`, `ExpressionIR` (an operator AST; `OpShape` deduplicated by a canonical AST hash, `OpInstance` holding parameters), `ClaimIR`, `StyleIR`.
- **Lifting and rendering:** a `Lifter` lifts surface fragments to IR (non-deterministic, may call a model). A deterministic `Renderer` executes a `ReconstructionProgram` whose steps are concatenate, overlay, format, instantiate, transform and generate. A `Canonicalizer` normalises; a `Verifier` checks the render against a `DriftSpec` (byte identity, among others).
- **Edits:** graph deltas (an atomic upsert at an anchor, removal of a region); `delta_chain.py` kept delta chains of length 3 or less, then rebased. Variants were a base plus a delta ("radicals" were rendering instructions).

## What carried over (the experiments)

- **Paragraph blocks are the edit unit** (E153, E154). Of the cutters measured (whole files, lines, paragraph blocks, CDC at 256 to 16,384 bytes, line-aligned CDC), blocks make the smallest new bytes per edit with few references (115 to 176 bytes and 5.6 to 7.6 references, against 740 to 860 bytes whole). Storage is not the reason: zstd over each cutter's distinct fragments is about 157 KB for every cutter, since the matcher and zstd already find what is shared. This matches the Python boundary policy (structure-aware, full coverage) without its 1,800-character cap [inf].
- **A file as a render over its fragments** (E155 to E157). `render/1` concatenates like `concat` but is a fact, so repack keeps it. With only shared blocks standing alone (E157, "render-runs"), the store costs 24% more at rest than plain files, and one edit reaches every file that renders the fragment, writing 26 to 40% fewer bytes than splicing files found through a side index. This is the Python `concatenate` step of a reconstruction program, without the program [inf].
- **Renders of other formats are local only when the renderer is** (E158). pandoc's html of a paragraph is a substring of the whole document's html (571 of 576 blocks), and splicing the new block's html over the old one reproduces a full re-render byte for byte (72 of 72 edits). docx is not local, because pandoc numbers bookmarks and lists across the whole document (0 of 72 exact). With those ids normalised, 206 of 236 blocks are local, so a renumbering renderer would make docx a splice target; pdf is regenerated.
- **Goal-scoped edits** (E159). A grant (E140) over the files a fragment reaches is checked from fragment lengths, without diffing any file, at no measured cost; a refusal writes nothing. The grant lists every file, so it grows by about 140 bytes per file; a fragment-scoped grant would stay at about 57 bytes.
- **Queries over fragments** (E160). Indexes over distinct fragments and their seams answer search exactly, update after a propagated edit by reading only the new fragment (101 bytes, 1.5 ms against a 42 ms rebuild), and per-fragment word counts sum to each file's counts exactly. Paragraphs that hold a term are 3.7 times fewer bytes than the files that hold it.
- **TableIR** (E161). Lifting the csv into a table with **cell types** (the spreadsheet's number or text, which the csv's quoting carries) and two residuals (the md's title and source line; the csv's empty rows) renders csv, json and md byte for byte for all 24 workbooks. Without cell types only 13 csv files reproduce. The IR alone is 51% of the kernel's bytes for the three formats. A one-cell edit changes one contiguous range of each rendered file.

## Not carried over

- Non-deterministic lifting (a model in the loop): every lift here is deterministic and checked by a byte-identity drift spec.
- Delta chains: the kernel's matcher and repack already store a new version as references into the old one [inf].
- `ExpressionIR`, `PropositionIR`, `ClaimIR`, `StyleIR`: no corpus here exercises them; they are left for a corpus that does (formulas in workbooks would be the first).
