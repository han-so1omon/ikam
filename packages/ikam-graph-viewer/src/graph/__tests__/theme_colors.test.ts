import { describe, expect, it } from 'vitest';

import {
  defaultColors,
  dimmedColorFloor,
  getNodeColor,
  semanticNodePalettes,
} from '../../theme';

describe('theme node colors', () => {
  it('maps text and binary nodes to explicit non-default colors', () => {
    expect(getNodeColor('text')).not.toBe(defaultColors.default);
    expect(getNodeColor('binary')).not.toBe(defaultColors.default);
  });

  it('keeps default fallback for unknown node types', () => {
    expect(getNodeColor('unknown-kind')).toBe(defaultColors.default);
  });

  it('provides semantic palettes with base, hover, and selected variants', () => {
    const paletteEntries = Object.entries(semanticNodePalettes);
    expect(paletteEntries.length).toBeGreaterThan(0);
    for (const [kind, palette] of paletteEntries) {
      expect(kind.length).toBeGreaterThan(0);
      expect(palette.base).toMatch(/^#[0-9a-fA-F]{6}$/);
      expect(palette.hover).toMatch(/^#[0-9a-fA-F]{6}$/);
      expect(palette.selected).toMatch(/^#[0-9a-fA-F]{6}$/);
    }
  });

  it('keeps dimmed color floor readable', () => {
    expect(dimmedColorFloor).toMatch(/^#[0-9a-fA-F]{6}$/);
    expect(dimmedColorFloor.toLowerCase()).not.toBe('#000000');
  });
});
