from ikam_perf_report.db import schema


def test_schema_includes_benchmark_tables():
    sql = schema.load_schema_sql()
    assert "benchmark_runs" in sql
    assert "benchmark_stages" in sql
    assert "benchmark_decisions" in sql
