from __future__ import annotations

import json
import sys

from mcp_ikam.agent_executor import run_parse_review


def main() -> None:
    payload = json.load(sys.stdin)
    json.dump(run_parse_review(payload), sys.stdout)


if __name__ == "__main__":
    main()
