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

## Second round (E162 to E169)

- **A grant that names a fragment** (E162) is 85 bytes whatever the fan-out (against 35,772 for a per-file grant over 256 files), with the same edit time, and refuses a stale fragment or a file outside its path prefix without writing.
- **docx becomes a splice target** (E163, E169). A renderer that maps pandoc's document-wide numbering reproduces a full re-render byte for byte: one-block edits 96 of 96, and inserts and deletes of paragraphs, headings and lists 168 of 168, with bookmarks, list ids and numbering.xml rebuilt. Deletes take 2.8 ms against 77 ms. This is the Python `Renderer` executing a reconstruction program with a byte-identity `DriftSpec`, for a renderer that numbers across the document [inf].
- **For one file and its renders, splice the renders and store them as plain files** (E167). The kernel's matcher already writes about one paragraph for a changed html (196 bytes); storing the html as a render over its pieces costs more (1,071 bytes, since the render names every piece). Fragments as storage pay where blocks are shared across files (E157).
- **Large tables do not need rows as fragments** (E164). A plain put of a one-cell edit adds about 168 bytes at 1,000 to 100,000 rows; E161's 13 to 16 KB was the file store rewriting its files, not the kernel. Groups of rows only make edits faster on very large tables.
- **A stored TableIR is an archive form** (E165): 43% fewer bytes than the three text formats, but rendering on read is slow for large tables (241 ms at 10,000 rows).
- **A compact fragment index** (E168) is the stored index's size (186 KB) with fragment-index speed (0.09 ms per query) and exact answers for patterns of any length.
- **A bug in range reads** (E166): a range read of a render rebuilt each whole argument to learn its length. Lengths now come from the derivation (decode work for a 100-byte range of a 4,096-line render: 99,064 to 400).
- **ExpressionIR for workbook formulas** (E180). Lifting each workbook's cells (values, number formats, and formulas as relative-reference shapes: 25 formulas, 12 shapes) renders every committed csv, json and md exactly, and an input edit reaches every computed cell in all three formats exactly (22 of 22 against LibreOffice). This is the Python version's `OpShape` and `OpInstance`, with a deterministic evaluator in place of the reconstruction program's `transform` step [inf].
- **One edit, 192 endpoints** (E181). A paragraph shared by 64 md files, edited once: rendering it once and splicing it into the 64 html and 64 docx renders takes 194 ms against 8.5 s to re-render them, every file byte-identical; with the md stored as render-runs the kernel writes 25 KB for all 192 files.
