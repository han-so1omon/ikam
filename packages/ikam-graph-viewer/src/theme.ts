export type SemanticPalette = {
  base: string;
  hover: string;
  selected: string;
};

export type SoftGlassGroupBubbleStyle = {
  color: string;
  opacity: number;
};

export type SemanticLegendEntry = {
  kind: string;
  label: string;
  description: string;
  color: string;
};

export type StateLegendEntry = {
  key: string;
  label: string;
  description: string;
  color: string;
};

export type SoftGlassGraphTheme = {
  boardBackground: string;
  dimmedScalarFloor: number;
  groupBubble: SoftGlassGroupBubbleStyle;
};

export const semanticNodePalettes: Record<string, SemanticPalette> = {
  artifact: { base: '#2f8f6f', hover: '#329d7a', selected: '#23785e' },
  fragment: { base: '#ca8f3b', hover: '#d39e4f', selected: '#ae7425' },
  interaction: { base: '#6f87a5', hover: '#7b97b8', selected: '#5d7290' },
  project: { base: '#3f86ea', hover: '#5a9af0', selected: '#2f6fd1' },
  text: { base: '#3aa8d6', hover: '#4db8e2', selected: '#2b8fb9' },
  binary: { base: '#d6953a', hover: '#e1a850', selected: '#bc7e26' },
  entity: { base: '#d0677f', hover: '#de7f95', selected: '#b95269' },
  relation: { base: '#4aa77c', hover: '#63b893', selected: '#368f66' },
  concept: { base: '#8a76d6', hover: '#9b89e2', selected: '#7662bd' },
  evidence: { base: '#3ea7b6', hover: '#55b8c6', selected: '#2b8e9c' },
  default: { base: '#8292a3', hover: '#97a5b3', selected: '#6f7f90' },
};

export const dimmedColorFloor = '#586676';

export const softGlassGraphTheme: SoftGlassGraphTheme = {
  boardBackground: '#edf3fb',
  dimmedScalarFloor: 0.38,
  groupBubble: {
    color: '#dce8f8',
    opacity: 0.24,
  },
};

export const defaultColors: Record<string, string> = Object.fromEntries(
  Object.entries(semanticNodePalettes).map(([key, palette]) => [key, palette.base]),
);

export const semanticLegendEntries: SemanticLegendEntry[] = [
  { kind: 'text', label: 'Text Fragments', description: 'Narrative and prose fragments', color: semanticNodePalettes.text.base },
  {
    kind: 'binary',
    label: 'Binary Fragments',
    description: 'Structured or encoded artifact fragments',
    color: semanticNodePalettes.binary.base,
  },
  { kind: 'artifact', label: 'Artifacts', description: 'Top-level source artifacts', color: semanticNodePalettes.artifact.base },
  {
    kind: 'interaction',
    label: 'Interactions',
    description: 'Interaction and operation events',
    color: semanticNodePalettes.interaction.base,
  },
];

export const stateLegendEntries: StateLegendEntry[] = [
  { key: 'selected', label: 'Selected Node', description: 'Currently selected focus node', color: '#7662bd' },
  { key: 'hover', label: 'Hover', description: 'Node under current pointer', color: '#fbbf24' },
  { key: 'highlighted', label: 'Search Highlight', description: 'Semantic query match node', color: '#2f8f6f' },
  { key: 'dimmed', label: 'Context', description: 'Unmatched nodes and edges kept visible', color: '#6d8198' },
];

export const getNodeColor = (type?: string) => {
  if (!type) return defaultColors.default;
  return defaultColors[type] ?? defaultColors.default;
};
