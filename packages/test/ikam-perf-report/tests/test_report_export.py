from ikam_perf_report.reports.exporter import export_report


def test_export_report_creates_markdown():
    md = export_report(run_id="demo")
    assert "IKAM Performance Report" in md
