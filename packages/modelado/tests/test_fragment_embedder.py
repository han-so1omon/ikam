"""Tests for LocalFragmentEmbedder."""
import pytest

try:
    from sentence_transformers import SentenceTransformer  # noqa: F401
    HAS_SENTENCE_TRANSFORMERS = True
except ImportError:
    HAS_SENTENCE_TRANSFORMERS = False

skip_no_st = pytest.mark.skipif(
    not HAS_SENTENCE_TRANSFORMERS,
    reason="sentence-transformers not installed (Python 3.14 incompatible)",
)


@skip_no_st
def test_embedder_implements_protocol():
    from ikam.ir.protocols import FragmentEmbedder
    from modelado.fragment_embedder import LocalFragmentEmbedder
    embedder = LocalFragmentEmbedder()
    assert isinstance(embedder, FragmentEmbedder)
    assert embedder.dimensions > 0
    assert isinstance(embedder.model_name, str)


@skip_no_st
@pytest.mark.asyncio
async def test_embed_text_fragment():
    from ikam.fragments import Fragment
    from modelado.fragment_embedder import LocalFragmentEmbedder
    embedder = LocalFragmentEmbedder()
    frag = Fragment(value={"text": "Revenue growth assumptions"}, mime_type="text/ikam-heading")
    vec = await embedder.embed(frag)
    assert isinstance(vec, list)
    assert len(vec) == embedder.dimensions
    assert all(isinstance(v, float) for v in vec)


@skip_no_st
@pytest.mark.asyncio
async def test_embed_deterministic():
    from ikam.fragments import Fragment
    from modelado.fragment_embedder import LocalFragmentEmbedder
    embedder = LocalFragmentEmbedder()
    frag = Fragment(value={"text": "test"}, mime_type="text/ikam-paragraph")
    v1 = await embedder.embed(frag)
    v2 = await embedder.embed(frag)
    assert v1 == v2
