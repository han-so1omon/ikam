from ikam_perf_report.benchmarks.case_fixtures import (
    available_case_ids,
    load_case_fixture,
    parse_case_ids,
    validate_case_ids,
)


def test_available_case_ids_reads_registry(case_fixtures_root):
    assert available_case_ids() == ["s-construction-v01"]


def test_load_case_fixture_collects_assets(case_fixtures_root):
    fixture = load_case_fixture("s-construction-v01")
    assert fixture.case_id == "s-construction-v01"
    assert fixture.assets


def test_load_case_fixture_excludes_idea_md(case_fixtures_root):
    fixture = load_case_fixture("s-construction-v01")
    names = {asset.file_name for asset in fixture.assets}
    assert "idea.md" not in names


def test_load_case_fixture_excludes_venv_tree(case_fixtures_root):
    case_dir = case_fixtures_root / "s-construction-v01"
    site_packages = case_dir / ".venv" / "lib" / "python3.11" / "site-packages"
    site_packages.mkdir(parents=True)
    (site_packages / "irrelevant.py").write_text("print('not project content')", encoding="utf-8")

    fixture = load_case_fixture("s-construction-v01")
    names = {asset.file_name for asset in fixture.assets}
    assert not any(name.startswith(".venv/") for name in names)


def test_load_case_fixture_includes_all_non_ignored_files(case_fixtures_root):
    case_dir = case_fixtures_root / "s-construction-v01"
    (case_dir / "deck.pptx").write_bytes(b"pptx")
    (case_dir / "sheet.xlsx").write_bytes(b"xlsx")
    (case_dir / "diagram.svg").write_text("<svg></svg>", encoding="utf-8")
    (case_dir / "notes.txt").write_text("ignore", encoding="utf-8")
    (case_dir / "native.cpython-311-darwin.so").write_bytes(b"\x00\x01")
    (case_dir / "bytecode.cpython-311.pyc").write_bytes(b"\x00\x00")

    fixture = load_case_fixture("s-construction-v01")
    names = {asset.file_name for asset in fixture.assets}

    assert "deck.pptx" in names
    assert "sheet.xlsx" in names
    assert "diagram.svg" in names
    assert "notes.txt" in names
    assert "native.cpython-311-darwin.so" in names
    assert "bytecode.cpython-311.pyc" not in names


def test_load_case_fixture_applies_ikamignore_entries(case_fixtures_root):
    case_dir = case_fixtures_root / "s-construction-v01"
    (case_dir / ".ikamignore").write_text("metrics.json\nassets/\n", encoding="utf-8")

    fixture = load_case_fixture("s-construction-v01")
    names = {asset.file_name for asset in fixture.assets}

    assert "metrics.json" not in names
    assert not any(name.startswith("assets/") for name in names)


def test_case_local_ikamignore_overrides_root_with_negation(case_fixtures_root):
    case_dir = case_fixtures_root / "s-construction-v01"
    (case_dir / "script.py").write_text("print('hello')", encoding="utf-8")
    (case_dir / ".ikamignore").write_text("!script.py\n", encoding="utf-8")

    fixture = load_case_fixture("s-construction-v01")
    names = {asset.file_name for asset in fixture.assets}

    assert "script.py" in names


def test_case_local_ikamignore_supports_wildcard_and_dir_patterns(case_fixtures_root):
    case_dir = case_fixtures_root / "s-construction-v01"
    (case_dir / "assets" / "keep.md").write_text("note", encoding="utf-8")
    (case_dir / "assets" / "ignore.tmp").write_text("tmp", encoding="utf-8")
    (case_dir / "report.tmp").write_text("tmp", encoding="utf-8")
    (case_dir / ".ikamignore").write_text("*.tmp\nassets/\n!assets/keep.md\n", encoding="utf-8")

    fixture = load_case_fixture("s-construction-v01")
    names = {asset.file_name for asset in fixture.assets}

    assert "report.tmp" not in names
    assert "assets/ignore.tmp" not in names
    assert "assets/keep.md" in names


def test_parse_and_validate_case_ids(case_fixtures_root):
    parsed = parse_case_ids("s-construction-v01,missing")
    valid, missing = validate_case_ids(parsed)
    assert valid == ["s-construction-v01"]
    assert missing == ["missing"]


def test_load_case_fixture_detects_stable_mime_types(case_fixtures_root):
    case_dir = case_fixtures_root / "s-construction-v01"
    (case_dir / "readme.md").write_text("# heading", encoding="utf-8")
    (case_dir / "model.xlsx").write_bytes(b"xlsx")
    (case_dir / "slides.pptx").write_bytes(b"pptx")

    fixture = load_case_fixture("s-construction-v01")
    mime_by_name = {asset.file_name: asset.mime_type for asset in fixture.assets}

    assert mime_by_name["readme.md"] == "text/markdown"
    assert mime_by_name["model.xlsx"] == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    assert mime_by_name["slides.pptx"] == "application/vnd.openxmlformats-officedocument.presentationml.presentation"


def test_load_case_fixture_uses_extension_fallback_when_mimetypes_unknown(case_fixtures_root, monkeypatch):
    import mimetypes

    case_dir = case_fixtures_root / "s-construction-v01"
    (case_dir / "fallback.md").write_text("hello", encoding="utf-8")
    (case_dir / "fallback.xlsx").write_bytes(b"xlsx")

    monkeypatch.setattr(mimetypes, "guess_type", lambda _name: (None, None))

    fixture = load_case_fixture("s-construction-v01")
    mime_by_name = {asset.file_name: asset.mime_type for asset in fixture.assets}

    assert mime_by_name["fallback.md"] == "text/markdown"
    assert mime_by_name["fallback.xlsx"] == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
