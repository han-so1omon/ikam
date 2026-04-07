from modelado.ikam_graph_repository import integrate_parse_outputs


def test_parse_outputs_merge_into_existing_graph():
    integrate_parse_outputs(None, artifact_id="artifact-123", derived_edges=[("a", "b")])


def test_parse_outputs_adds_derivation_metadata(db_connection):
    integrate_parse_outputs(
        db_connection,
        artifact_id="artifact-123",
        derived_edges=[("source-1", "fragment-1")],
    )

    row = db_connection.execute(
        """
        SELECT properties
          FROM graph_edge_events
         WHERE edge_label = %s
           AND out_id = %s
           AND in_id = %s
        """,
        ("knowledge:parse", "source-1", "fragment-1"),
    ).fetchone()

    assert row is not None
    props = row["properties"] if isinstance(row, dict) else row[0]
    assert props.get("derivationId")
    assert props.get("derivationType") == "parse"
