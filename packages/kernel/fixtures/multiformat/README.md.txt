The 24 fixture cases' idea.md (tests/fixtures/cases/*/idea.md), each also
rendered to:
  html: pandoc 3.1.3, `pandoc X.md -s -o X.html`
  docx: pandoc 3.1.3, `pandoc X.md -o X.docx`
  pdf:  LibreOffice 24.2.7.2, `soffice --headless --convert-to pdf X.docx`
Committed as generated, so the corpus does not depend on tool versions.
Used by examples/cross.rs (E077).
