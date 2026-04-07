import sys
import types


def test_loader_uses_json_reader_for_json_bytes() -> None:
    from modelado.executors.loaders import run

    result = None
    for item in run({"fragment": {}, "params": {"raw_bytes": b'{"revenue": 42}', "file_name": "data.json"}}, {}):
        if item.get("type") == "result":
            result = item["result"]
            break

    assert result is not None
    assert result["reader_key"] == "json_reader"
    assert result["reader_method"] == "JSONReader.load_data"
    assert len(result["documents"]) == 1
    assert '"revenue": 42' in result["documents"][0]["text"]


def test_loader_preserves_filename_suffix_for_simple_directory_reader(monkeypatch) -> None:
    from modelado.executors.loaders import run

    observed_paths: list[str] = []

    class FakeDoc:
        def __init__(self) -> None:
            self.doc_id = "doc-1"
            self.text = "hello"
            self.metadata = {"file_name": "brief.md"}

    class FakeReader:
        def __init__(self, input_files: list[str]) -> None:
            observed_paths.extend(input_files)

        def load_data(self) -> list[FakeDoc]:
            return [FakeDoc()]

    fake_llama_index = types.ModuleType("llama_index")
    fake_core = types.ModuleType("llama_index.core")
    fake_core.SimpleDirectoryReader = FakeReader
    monkeypatch.setitem(sys.modules, "llama_index", fake_llama_index)
    monkeypatch.setitem(sys.modules, "llama_index.core", fake_core)

    result = None
    for item in run({"fragment": {}, "params": {"raw_bytes": b"# Hello", "file_name": "brief.md"}}, {}):
        if item.get("type") == "result":
            result = item["result"]
            break

    assert result is not None
    assert result["reader_key"] == "simple_directory_reader"
    assert observed_paths
    assert observed_paths[0].endswith(".md")
