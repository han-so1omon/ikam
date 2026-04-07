from __future__ import annotations

from datetime import datetime
from pathlib import Path

from ikam_perf_report.benchmarks.case_suite_ingestion import write_case_suite_report


def main() -> None:
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    out_dir = Path("packages/test/ikam-perf-report/reports/case-suite") / stamp
    summary = write_case_suite_report(out_dir)
    print(f"Generated case-suite report: {out_dir}")
    print(f"Cases processed: {summary['case_count']}")


if __name__ == "__main__":
    main()
