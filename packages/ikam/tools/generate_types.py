#!/usr/bin/env python3
"""
Generate TypeScript types for IKAM models into the IKAM graph viewer workspace.

This is a minimal template-based generator that mirrors the core Pydantic models
in ikam.models without pulling heavy dependencies.
"""
from __future__ import annotations

import os
from pathlib import Path

HEADER = """// AUTO-GENERATED FILE. Do not edit manually.
// Source: packages/ikam/tools/generate_types.py
// Generated: IKAM core types for frontend usage

/* eslint-disable */

"""

TYPES = """
// Enums as string unions
export type ArtifactType = 'document' | 'slide-deck' | 'sheet'
export type ModelKind = 'economic' | 'story'
export type MediaKind = 'image' | 'chart' | 'table' | 'asset'

export interface ArtifactRef {
  id: string
  type: ArtifactType
  title?: string | null
}

export type DocumentRef = ArtifactRef & { type: 'document' }
export type SlideDeckRef = ArtifactRef & { type: 'slide-deck' }
export type SheetRef = ArtifactRef & { type: 'sheet' }

export interface ModelRef {
  key: string
  kind: ModelKind
  version?: string | null
  config: Record<string, string | number | boolean>
}

export interface MediaRef {
  key: string
  kind: MediaKind
  uri: string
  meta: Record<string, string | number | boolean>
}

export interface Derivation {
  id?: string | null
  source_artifact_id: string
  derived_artifact_id: string
  operation: string
  params: Record<string, string | number | boolean>
}

export interface Snapshot {
  id?: string | null
  artifact_id: string
  label?: string | null
  data: Record<string, any>
}

export type InstructionActor = 'user' | 'system' | 'agent'

export interface Instruction {
  id?: string | null
  project_id: string
  actor: InstructionActor
  text: string
  intent?: string | null
  payload: Record<string, any>
}

export interface IKAMProject {
  id?: string | null
  name: string
  description?: string | null
  artifacts: ArtifactRef[]
  models: ModelRef[]
  media: MediaRef[]
  derivations: Derivation[]
  artifact_index: Record<string, ArtifactRef>
  model_index: Record<string, ModelRef>
  media_index: Record<string, MediaRef>
}
"""


def main() -> None:
    repo_root = Path(__file__).resolve().parents[3]
    default_out_dir = repo_root / 'packages' / 'ikam-graph-viewer' / 'src' / 'generated'
    out_dir = Path(os.getenv('IKAM_TS_TYPES_OUT', str(default_out_dir)))
    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / 'ikam.ts'
    out_file.write_text(HEADER + TYPES, encoding='utf-8')
    print(f"Wrote {out_file}")


if __name__ == "__main__":
    main()
