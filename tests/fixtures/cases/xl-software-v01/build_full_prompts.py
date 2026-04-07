from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(5120605)
    created_at = str(date.today())

    targets = {
        "logos": 8,
        "people": 30,
        "product-ui": 30,
        "diagrams": 25,
        "social": 12,
    }

    lines = []

    def add(kind: str, idx: int, *, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "logos":
            out_path = f"assets/images/logos/logo-{idx:03d}.png"
        elif kind == "product-ui":
            out_path = f"assets/images/product-ui/ui-{idx:03d}.png"
        elif kind == "diagrams":
            out_path = f"assets/images/diagrams/diagram-{idx:03d}.png"
        elif kind == "social":
            out_path = f"assets/images/social/social-{idx:03d}.png"

        lines.append(
            {
                "id": f"{kind}-{idx:03d}",
                "out_path": out_path,
                "kind": kind,
                "prompt": prompt,
                "negative_prompt": negative,
                "model": "sdxl",
                "checkpoint": "SSD-1B.safetensors",
                "sampler": "dpmpp_2m",
                "steps": steps,
                "guidance": guidance,
                "width": w,
                "height": h,
                "seed": rng.randrange(10_000, 9_999_999),
                "created_at": created_at,
                "style_tags": ["meridian", "b2b-saas", "navy-teal", "enterprise"],
                "source_case_id": "xl-software-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, celebrity, uncanny, extra fingers, distorted text"
    neg_ui = "watermark, real brand logos, personal data, readable email addresses, phone numbers, credit cards, copyrighted UI"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    # LOGOS
    logo_prompts = [
        "Flat vector logo for a B2B software company named 'Meridian SignalWorks'. Minimal wordmark with an abstract signal wave icon. Palette: deep navy, teal accent, off-white background. No mockups.",
        "Flat icon + wordmark: compass/meridian line motif, modern SaaS aesthetic, navy + teal.",
        "Circular seal logo: 'Meridian SignalWorks' around edge, small signal icon in center, flat, high contrast.",
        "Monogram logo: 'MSW' in a clean geometric style, navy on off-white.",
        "Wordmark only: 'Meridian SignalWorks' with teal underline and small dot grid accent.",
        "Minimal logo: abstract layered bars suggesting a dashboard signal, paired with wordmark.",
        "Alternate icon: hexagon with signal wave cutout + wordmark.",
        "Simple mark: meridian line intersecting a small circle, flat, navy/teal.",
    ]
    for i in range(1, targets["logos"] + 1):
        add("logos", i, prompt=logo_prompts[i - 1], negative=neg_flat, w=1024, h=1024, steps=30, guidance=5.5)

    # PEOPLE
    roles = ["CEO", "CTO", "VP Product", "Platform Eng Director", "Security lead", "CS lead", "Engineer", "Product manager", "Analyst"]
    for i in range(1, targets["people"] + 1):
        role = roles[(i - 1) % len(roles)]
        prompt = (
            f"Photoreal headshot of a {role} at a modern B2B SaaS company. Neutral background, soft studio lighting, calm confident expression, business casual. "
            "Looks like a team page photo, clearly synthetic."
        )
        add("people", i, prompt=prompt, negative=neg_photo, w=1024, h=1024, steps=32, guidance=6.0)

    # PRODUCT UI (synthetic screenshots)
    ui_scenes = [
        "dark-mode dashboard showing ingestion latency, queue depth, and workflow throughput charts",
        "settings page for KPI definitions registry with toggle switches and definition cards (no real personal data)",
        "incident timeline view with events and annotations, modern UI",
        "workflow builder canvas with nodes and connectors",
        "customer account page showing modules enabled and usage summary",
        "alerts configuration screen for queue-depth SLO",
    ]
    for i in range(1, targets["product-ui"] + 1):
        scene = ui_scenes[(i - 1) % len(ui_scenes)]
        prompt = (
            f"Synthetic UI screenshot/illustration for a product called Meridian SignalWorks: {scene}. "
            "Modern SaaS UI, navy/teal palette, clean typography, rounded cards, high contrast. "
            "Use placeholder text only; avoid readable emails/phones; no real brand marks."
        )
        add("product-ui", i, prompt=prompt, negative=neg_ui, w=1344, h=768, steps=34, guidance=6.0)

    # DIAGRAMS
    diagram_types = [
        "architecture diagram: ingestion API -> queue -> normalizer -> workflow engine -> metrics",
        "incident timeline diagram with key events and detection/mitigation",
        "KPI definition registry concept diagram",
        "data reconciliation flowchart",
        "NRR/ARR definition comparison diagram",
        "SLO dashboard layout",
        "security access review workflow diagram",
        "roadmap timeline with Q1/Q2/Q3 milestones",
        "queue backlog causal graph diagram",
        "RACI matrix layout for incident ownership",
    ]
    for i in range(1, targets["diagrams"] + 1):
        dt = diagram_types[(i - 1) % len(diagram_types)]
        prompt = (
            f"Clean illustrative tech diagram on off-white background with navy/teal accents: {dt}. "
            "Slide-ready, minimal text, high contrast, modern enterprise style."
        )
        add("diagrams", i, prompt=prompt, negative=neg_flat, w=1344, h=768, steps=30, guidance=5.5)

    # SOCIAL
    for i in range(1, targets["social"] + 1):
        if i % 3 == 1:
            prompt = "Illustrative product launch social template: navy background, teal accent line, space for UI screenshot, clean sans blocks (not readable text)."
        elif i % 3 == 2:
            prompt = "Illustrative hiring post template for a B2B SaaS company: off-white background, navy header, teal highlight, minimal icons."
        else:
            prompt = "Illustrative event/webinar promo template: modern tech aesthetic, navy/teal palette, abstract signal wave pattern."
        add("social", i, prompt=prompt, negative=neg_flat, w=1080, h=1080, steps=28, guidance=5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
