# Recompression: expanding deflate streams inside files

Status: research note (2026-10-20). E080 is grounded in a measurement (below), not in a paper. The tools named here are known from prior knowledge. Their documentation was **not** re-read for this note, so any claim about them is unverified for this kernel.

## The problem

Many formats wrap their content in deflate: zip members (docx, xlsx, pptx, jar, epub), PDF FlateDecode streams (page content, fonts, images), PNG IDAT, gzip and HTTP bodies. Deflated bytes hide what they share. Two PDFs that embed the same font subset, or a docx and a markdown note with the same paragraphs, look unrelated until they are inflated. For the north star (one graph across arbitrary sources, with shared information stored once), every deflate layer is a wall between sources.

## The technique

Inflate the stream, then find deflate parameters that reproduce the original compressed bytes exactly. Store the inflated bytes, plus a recipe (parameters and the bytes around the stream). Rebuilding re-deflates with the same parameters. This is lossless only if the deflater is deterministic and pinned. The kernel pins zlib (`container::tests::deflate_output_is_pinned`).

- **precomp** (C. Schneider) applies this to many container formats by trying zlib parameter sets.
- **preflate** (D. Steinke) also reconstructs streams written by deflaters other than zlib, by recording the differences from a reference deflater.

Neither has been reproduced here beyond the zlib-level search below.

## Measured (E080 probe, Python zlib, `fixtures/multiformat`)

- 24 LibreOffice PDFs, 2,244,923 B. 275 FlateDecode streams, 843,494 B deflated, 1,821,562 B inflated.
- 255 of the 275 streams re-deflate exactly with zlib at some level (window 15, memLevel 8).
- The inflated streams compress solid (zstd 19) to 138,390 B, against 700,501 B for zstd 19 over the PDFs as they are.

## In the kernel

- E080 adds `container::plan_zlib`. It finds zlib streams anywhere: a valid header, a deflate stream that ends, and a matching Adler-32 after it. Each stream that re-deflates exactly at some zlib level is planned as a member of the existing `deflate-pack` function. No file type is named.
- Open: the 20 streams that zlib levels do not reproduce (a preflate-style diff), gzip framing, and PNG IDAT split across chunks.

## E083

The 20 streams the first probe could not reproduce were 19 streams written at zlib level 6 with **memLevel 9** (LibreOffice's fonts), plus a probe artifact. A wider probe (memLevel 1–9, strategies 0–4) reproduces all 275. The kernel now reproduces memLevel 9 through the pinned zlib.
