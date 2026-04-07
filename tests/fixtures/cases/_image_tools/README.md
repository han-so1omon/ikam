# Case Image Tools (ComfyUI)

Shared, resumable image generation + syncing for all cases.

## 1) Start ComfyUI (in separate terminal)
Use the repo's ComfyUI run script (recommended) and keep it running.

If you hit BrokenPipe issues, run with nohup + log redirect:

```bash
cd /Users/Shared/p/ingenierias-lentas/narraciones-de-economicos/tools/ComfyUI
nohup python main.py --listen 127.0.0.1 --port 8188 --disable-all-custom-nodes > comfy.log 2>&1 &
```

## 2) Generate images for a case (resumable)

```bash
cd /Users/Shared/p/ingenierias-lentas/narraciones-de-economicos/tests/fixtures/cases
python3 _image_tools/run_images_comfy.py --case-dir ./xl-manufacturing-v01
```

Smoke test:

```bash
python3 _image_tools/run_images_comfy.py --case-dir ./xl-manufacturing-v01 --limit 5
```

Notes:
- By default it **skips** images that already exist in the case folder.
- Set `COMFY_URL` to point at a different ComfyUI server.
- Set `COMFY_TIMEOUT_S=7200` if your renders are slow.

## 3) Sync images from ComfyUI output into the case folder

```bash
python3 _image_tools/sync_images_from_comfy.py --case-dir ./xl-manufacturing-v01
```

Add `--overwrite` to replace existing images in the case.

## Recommended workflow
One case at a time:
1) run generator (resume safe)
2) sync
3) count-check images vs targets
