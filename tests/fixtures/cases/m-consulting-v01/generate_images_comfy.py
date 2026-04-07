from __future__ import annotations

import json
import os
import time
import urllib.request
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
WORKFLOW_PATH = Path("/Users/Shared/p/ingenierias-lentas/narraciones-de-economicos/tools/ComfyUI/workflows/sdxl_basic_api.json")
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"
COMFY_URL = os.environ.get("COMFY_URL", "http://127.0.0.1:8188")


def comfy_post_json(path: str, payload: dict):
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        COMFY_URL + path,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read().decode("utf-8"))


def comfy_get_json(path: str):
    with urllib.request.urlopen(COMFY_URL + path, timeout=60) as resp:
        return json.loads(resp.read().decode("utf-8"))


def wait_for_prompt(prompt_id: str, poll_s: float = 1.0, timeout_s: int = 1800):
    t0 = time.time()
    while time.time() - t0 < timeout_s:
        h = comfy_get_json(f"/history/{prompt_id}")
        if h.get(prompt_id) and h[prompt_id].get("status") and h[prompt_id]["status"].get("completed"):
            return h[prompt_id]
        time.sleep(poll_s)
    raise TimeoutError(f"Timed out waiting for prompt {prompt_id}")


def load_workflow():
    return json.loads(WORKFLOW_PATH.read_text(encoding="utf-8"))


def main(limit: int | None = None):
    lines = [l for l in PROMPTS_PATH.read_text(encoding="utf-8").splitlines() if l.strip()]
    if limit is not None:
        lines = lines[:limit]

    wf_base = load_workflow()

    for i, line in enumerate(lines, start=1):
        spec = json.loads(line)
        wf = json.loads(json.dumps(wf_base))

        wf["6"]["inputs"]["text"] = spec["prompt"]
        wf["7"]["inputs"]["text"] = spec.get("negative_prompt", "")
        wf["3"]["inputs"]["seed"] = int(spec.get("seed", 0))
        wf["3"]["inputs"]["steps"] = int(spec.get("steps", 30))
        wf["3"]["inputs"]["cfg"] = float(spec.get("guidance", 6.0))
        wf["3"]["inputs"]["sampler_name"] = spec.get("sampler", "dpmpp_2m")
        wf["5"]["inputs"]["width"] = int(spec.get("width", 1024))
        wf["5"]["inputs"]["height"] = int(spec.get("height", 1024))
        wf["4"]["inputs"]["ckpt_name"] = spec.get("checkpoint", wf["4"]["inputs"]["ckpt_name"])

        out_path = spec["out_path"]
        prefix = str(Path(out_path).with_suffix(""))
        wf["9"]["inputs"]["filename_prefix"] = prefix

        res = comfy_post_json("/prompt", {"prompt": wf})
        prompt_id = res.get("prompt_id")
        if not prompt_id:
            raise RuntimeError(f"Unexpected response: {res}")

        print(f"[{i}/{len(lines)}] submitted {spec['id']} -> {prompt_id}")
        wait_for_prompt(prompt_id)


if __name__ == "__main__":
    lim = os.environ.get("LIMIT")
    main(int(lim) if lim else None)
