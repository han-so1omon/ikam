from pathlib import Path


def test_large_wiki_fixtures_are_gitignored_and_untracked():
    repo_root = Path(__file__).resolve().parents[3]
    gitignore_text = (repo_root / ".gitignore").read_text(encoding="utf-8")

    assert "tests/fixtures/wiki/enwik8" in gitignore_text
    assert "tests/fixtures/wiki/enwik9" in gitignore_text
