from __future__ import annotations

import argparse
import json
import os
import time
import urllib.request
from pathlib import Path

# Defaults match the rest of this repo
DEFAULT_COMFY_DIR = Path(
    "/Users/Shared/p/ingenierias-lentas/narraciones-de-economicos/tools/ComfyUI"
)
DEFAULT_WORKFLOW_PATH = DEFAULT_COMFY_DIR / "workflows/sdxl_basic_api.json"


def comfy_post_json(comfy_url: str, path: str, payload: dict, timeout: int | None = None):
    """POST JSON to ComfyUI.

    Note: ComfyUI can occasionally stall under load; allow a longer socket timeout
    (configurable via COMFY_HTTP_TIMEOUT_S).
    """
    if timeout is None:
        timeout = int(os.environ.get("COMFY_HTTP_TIMEOUT_S", "300"))

    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        comfy_url + path,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def comfy_get_json(comfy_url: str, path: str, timeout: int | None = None):
    """GET JSON from ComfyUI with a configurable socket timeout."""
    if timeout is None:
        timeout = int(os.environ.get("COMFY_HTTP_TIMEOUT_S", "300"))

    with urllib.request.urlopen(comfy_url + path, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def wait_for_prompt(
    comfy_url: str,
    prompt_id: str,
    poll_s: float = 1.0,
    timeout_s: int = 3600,
    verbose: bool = False,
):
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        try:
            h = comfy_get_json(comfy_url, f"/history/{prompt_id}")
        except TimeoutError:
            # Socket read timeout from urllib; ComfyUI may be busy. Treat as transient.
            time.sleep(poll_s)
            continue
        if h.get(prompt_id) and h[prompt_id].get("status") and h[prompt_id]["status"].get(
            "completed"
        ):
            return h[prompt_id]
        if verbose and int(time.time() - t0) % 30 == 0:
            print(f"  ...waiting {int(time.time()-t0)}s for {prompt_id}")
        time.sleep(poll_s)
    raise TimeoutError(f"Timed out waiting for prompt {prompt_id} after {timeout_s}s")


def load_workflow(workflow_path: Path) -> dict:
    return json.loads(workflow_path.read_text(encoding="utf-8"))


def main():
    ap = argparse.ArgumentParser(
        description="Run ComfyUI image generation for a case prompts.jsonl (resumable)."
    )
    ap.add_argument(
        "--case-dir",
        required=True,
        help="Case folder containing assets/images/prompts.jsonl",
    )
    ap.add_argument(
        "--workflow",
        default=str(DEFAULT_WORKFLOW_PATH),
        help="ComfyUI workflow JSON path",
    )
    ap.add_argument(
        "--comfy-url",
        default=os.environ.get("COMFY_URL", "http://127.0.0.1:8188"),
        help="ComfyUI base URL (default from COMFY_URL env)",
    )
    ap.add_argument("--limit", type=int, default=0, help="If set, run only first N prompts")
    ap.add_argument(
        "--timeout-s",
        type=int,
        default=int(os.environ.get("COMFY_TIMEOUT_S", "3600")),
        help="Per-image timeout for /history polling",
    )
    ap.add_argument(
        "--poll-s",
        type=float,
        default=float(os.environ.get("COMFY_POLL_S", "1.0")),
        help="Polling interval seconds",
    )
    ap.add_argument(
        "--no-skip-existing",
        action="store_true",
        help="Do not skip prompts whose target output file already exists in the case folder",
    )
    ap.add_argument(
        "--verbose",
        action="store_true",
        help="Print periodic wait messages",
    )

    args = ap.parse_args()

    case_dir = Path(args.case_dir).expanduser().resolve()
    prompts_path = case_dir / "assets/images/prompts.jsonl"
    if not prompts_path.exists():
        raise SystemExit(f"Missing prompts.jsonl: {prompts_path}")

    workflow_path = Path(args.workflow).expanduser().resolve()
    if not workflow_path.exists():
        raise SystemExit(f"Missing workflow: {workflow_path}")

    comfy_url = args.comfy_url.rstrip("/")

    lines = [l for l in prompts_path.read_text(encoding="utf-8").splitlines() if l.strip()]
    if args.limit and args.limit > 0:
        lines = lines[: args.limit]

    wf_base = load_workflow(workflow_path)

    # Track progress
    total = len(lines)
    submitted = 0
    skipped = 0

    for i, line in enumerate(lines, start=1):
        spec = json.loads(line)

        target_rel = Path(spec["out_path"])
        target_abs = case_dir / target_rel

        if not args.no_skip_existing and target_abs.exists():
            skipped += 1
            print(f"[{i}/{total}] skip existing {spec.get('id')} -> {target_rel}")
            continue

        wf = json.loads(json.dumps(wf_base))  # deep copy

        # Map from prompts.jsonl -> workflow nodes (matches sdxl_basic_api.json)
        wf["6"]["inputs"]["text"] = spec["prompt"]
        wf["7"]["inputs"]["text"] = spec.get("negative_prompt", "")

        wf["3"]["inputs"]["seed"] = int(spec.get("seed", 0))
        wf["3"]["inputs"]["steps"] = int(spec.get("steps", 30))
        wf["3"]["inputs"]["cfg"] = float(spec.get("guidance", 6.0))
        wf["3"]["inputs"]["sampler_name"] = spec.get("sampler", "dpmpp_2m")

        wf["5"]["inputs"]["width"] = int(spec.get("width", 1024))
        wf["5"]["inputs"]["height"] = int(spec.get("height", 1024))

        wf["4"]["inputs"]["ckpt_name"] = spec.get(
            "checkpoint", wf["4"]["inputs"].get("ckpt_name")
        )

        # Output naming: ComfyUI saves under ComfyUI/output/<filename_prefix>_00001_.png
        prefix = str(target_rel.with_suffix(""))
        wf["9"]["inputs"]["filename_prefix"] = prefix

        payload = {"prompt": wf}
        try:
            res = comfy_post_json(comfy_url, "/prompt", payload)
        except Exception as e:
            raise RuntimeError(
                f"Failed to submit to ComfyUI at {comfy_url}. Is the server running? ({e})"
            )

        prompt_id = res.get("prompt_id")
        if not prompt_id:
            raise RuntimeError(f"Unexpected response: {res}")

        submitted += 1
        print(
            f"[{i}/{total}] submitted {spec.get('id')} -> {prompt_id} ({target_rel})"
        )
        wait_for_prompt(
            comfy_url,
            prompt_id,
            poll_s=args.poll_s,
            timeout_s=args.timeout_s,
            verbose=args.verbose,
        )

    print(
        f"Done. total={total} submitted={submitted} skipped={skipped} case={case_dir.name}"
    )


if __name__ == "__main__":
    main()
