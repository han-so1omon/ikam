"""Tests for ReconstructionProgram model and render_program execution."""
import base64


def test_basic_program():
    from ikam.ir.reconstruction import ReconstructionProgram, CompositionStep
    prog = ReconstructionProgram(
        steps=[
            CompositionStep(strategy="overlay", inputs={"base": "abc", "delta": "def"}),
            CompositionStep(strategy="format", inputs={"format_spec": "ghi"}),
        ],
        renderer_version="1.0.0",
        output_mime_type="text/markdown",
    )
    assert len(prog.steps) == 2
    assert prog.steps[0].strategy == "overlay"
    assert prog.renderer_version == "1.0.0"


def test_roundtrip_json():
    from ikam.ir.reconstruction import ReconstructionProgram, CompositionStep
    prog = ReconstructionProgram(
        steps=[CompositionStep(strategy="concatenate", inputs={"parts": ["a", "b"]})],
        renderer_version="1.0.0",
        output_mime_type="text/markdown",
    )
    data = prog.model_dump(mode="json")
    restored = ReconstructionProgram.model_validate(data)
    assert restored == prog


def test_as_fragment():
    from ikam.ir.reconstruction import ReconstructionProgram, CompositionStep, program_to_fragment
    from ikam.ir.mime_types import RECONSTRUCTION_PROGRAM
    prog = ReconstructionProgram(
        steps=[CompositionStep(strategy="overlay", inputs={"base": "abc", "delta": "def"})],
        renderer_version="1.0.0",
        output_mime_type="text/markdown",
    )
    frag = program_to_fragment(prog)
    assert frag.mime_type == RECONSTRUCTION_PROGRAM
    assert frag.cas_id is not None
    assert frag.value["renderer_version"] == "1.0.0"


# ── render_program tests ──


def test_render_program_concatenate():
    """render_program executes a concatenate step to reconstruct bytes."""
    from ikam.ir.reconstruction import (
        ReconstructionProgram,
        CompositionStep,
        render_program,
    )
    from ikam.forja.cas import cas_fragment

    chunk_a = b"First chunk. "
    chunk_b = b"Second chunk."
    frag_a = cas_fragment(
        {"text": "First chunk.", "index": 0, "bytes_b64": base64.b64encode(chunk_a).decode()},
        "text/ikam-paragraph",
    )
    frag_b = cas_fragment(
        {"text": "Second chunk.", "index": 1, "bytes_b64": base64.b64encode(chunk_b).decode()},
        "text/ikam-paragraph",
    )

    program = ReconstructionProgram(
        steps=[
            CompositionStep(
                strategy="concatenate",
                inputs={"fragment_ids": [frag_a.cas_id, frag_b.cas_id]},
            ),
        ],
        output_mime_type="text/markdown",
    )
    store = {frag_a.cas_id: frag_a, frag_b.cas_id: frag_b}
    result = render_program(program, store)
    assert result == chunk_a + chunk_b


def test_render_program_empty_steps_returns_empty_bytes():
    """Program with no steps returns empty bytes."""
    from ikam.ir.reconstruction import ReconstructionProgram, render_program

    program = ReconstructionProgram(steps=[], output_mime_type="text/markdown")
    result = render_program(program, {})
    assert result == b""


def test_render_program_unknown_strategy_raises():
    """render_program raises ValueError for unknown strategy."""
    from ikam.ir.reconstruction import (
        ReconstructionProgram,
        CompositionStep,
        render_program,
    )

    program = ReconstructionProgram(
        steps=[CompositionStep(strategy="invented_nonsense", inputs={})],
        output_mime_type="text/markdown",
    )
    try:
        render_program(program, {})
        assert False, "Should have raised ValueError"
    except ValueError as e:
        assert "invented_nonsense" in str(e)
