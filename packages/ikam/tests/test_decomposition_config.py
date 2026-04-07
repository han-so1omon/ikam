from ikam.config import DecompositionConfig


def test_effective_max_depth_defaults_to_target_levels():
	cfg = DecompositionConfig()
	assert cfg.max_depth is None
	assert cfg.target_levels == 3
	assert cfg.effective_max_depth() == 3


def test_effective_max_depth_prefers_max_depth_when_set():
	cfg = DecompositionConfig(max_depth=2, target_levels=7)
	assert cfg.effective_max_depth() == 2
from ikam.config import DecompositionConfig


def test_effective_max_depth_default_is_target_levels_default() -> None:
    config = DecompositionConfig()
    assert config.effective_max_depth() == 3


def test_effective_max_depth_prefers_max_depth_when_set() -> None:
    config = DecompositionConfig(max_depth=1)
    assert config.effective_max_depth() == 1


def test_effective_max_depth_uses_target_levels_when_max_depth_unset() -> None:
    config = DecompositionConfig(target_levels=2)
    assert config.max_depth is None
    assert config.effective_max_depth() == 2
