"""Tests that ikam_fragment_store DDL is syntactically present."""


def test_fragment_store_ddl_exists():
    from modelado.ikam_graph_schema import IKAM_FRAGMENT_STORE_DDL
    assert "ikam_fragment_store" in IKAM_FRAGMENT_STORE_DDL
    assert "cas_id" in IKAM_FRAGMENT_STORE_DDL
    assert "env" in IKAM_FRAGMENT_STORE_DDL
    assert "operation_id" in IKAM_FRAGMENT_STORE_DDL
    assert "project_id" in IKAM_FRAGMENT_STORE_DDL
    assert "embedding" in IKAM_FRAGMENT_STORE_DDL
    assert "VECTOR" in IKAM_FRAGMENT_STORE_DDL
    assert "uq_fragment_store_pk" in IKAM_FRAGMENT_STORE_DDL


def test_normalization_stats_ddl_exists():
    from modelado.ikam_graph_schema import IKAM_NORMALIZATION_STATS_DDL
    assert "ikam_normalization_stats" in IKAM_NORMALIZATION_STATS_DDL
    assert "family" in IKAM_NORMALIZATION_STATS_DDL
    assert "storage_saved_bytes" in IKAM_NORMALIZATION_STATS_DDL


def test_provenance_alter_ddl_exists():
    from modelado.ikam_graph_schema import IKAM_PROVENANCE_ALTER_DDL
    assert "fragment_id" in IKAM_PROVENANCE_ALTER_DDL
    assert "operation_id" in IKAM_PROVENANCE_ALTER_DDL
