from __future__ import annotations

import json
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write("usage: python stagehand_report_md.py /path/to/outputs.json\n")
        return 2

    outputs_path = Path(sys.argv[1])
    data = json.loads(outputs_path.read_text())

    run_id = data.get("decision_trace", {}).get("run_id")
    viewer_url = data.get("viewer_url")
    sources = data.get("sources", {})
    graph = data.get("graph", {})
    benchmarks = data.get("benchmarks", {})

    lines: list[str] = []
    lines.append("# IKAM Perf Tester Stagehand Report")
    lines.append("")
    lines.append(f"Outputs: `{outputs_path}`")
    lines.append("")
    lines.append("## Run")
    lines.append(f"- Viewer: {viewer_url}")
    lines.append(f"- Run ID: `{run_id}`")
    lines.append(f"- Size: `{data.get('run_size')}`")
    lines.append("")
    lines.append("## Endpoints")
    lines.append(f"- Graph: `{sources.get('graph')}`")
    lines.append(f"- Benchmarks: `{sources.get('benchmarks')}`")
    lines.append("")
    lines.append("## Expected Features")
    lines.append(f"- Graph Nodes: `{graph.get('nodes')}`")
    lines.append(f"- Graph Edges: `{graph.get('edges')}`")
    lines.append(f"- Semantic Entities: `{graph.get('semantic_entities')}`")
    lines.append(f"- Semantic Relations: `{graph.get('semantic_relations')}`")
    lines.append(f"- Decisions: `{data.get('decision_trace', {}).get('decisions')}`")
    lines.append(f"- Runs Counter: `{benchmarks.get('runs')}`")
    lines.append(f"- Stage Latency: `{benchmarks.get('stage_latency')}`")
    lines.append("")

    error = data.get("error")
    if error:
        lines.append("## Error")
        lines.append(f"- `{error}`")
        lines.append("")

    failures = data.get("request_failures") or []
    if failures:
        lines.append("## Network Failures")
        for item in failures:
            lines.append(f"- `{item}`")
        lines.append("")

    responses = data.get("responses") or []
    if responses:
        lines.append("## API Responses")
        for item in responses:
            lines.append(f"- `{item}`")
        lines.append("")

    sys.stdout.write("\n".join(lines) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
