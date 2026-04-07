import type { GraphData, GraphNode, GraphEdge } from '../types';

const labelFromContent = (content: any, fallback: string) => {
  if (content && typeof content === 'object') {
    const candidate = content.title ?? content.text ?? content.name ?? content.value;
    if (typeof candidate === 'string' && candidate.trim()) return candidate;
    if (typeof candidate === 'number') return String(candidate);
  }
  return fallback;
};

export const mapIkamFragmentsToGraph = (fragments: any[]): GraphData => {
  const nodes: GraphNode[] = [];
  const edges: GraphEdge[] = [];
  const idSet = new Set<string>();

  for (const frag of fragments) {
    const id = String(frag?.id ?? '');
    if (!id || idSet.has(id)) continue;
    idSet.add(id);
    const artifactId = typeof frag?.artifact_id === 'string' ? frag.artifact_id : undefined;
    const artifactIds = Array.isArray(frag?.artifact_ids)
      ? frag.artifact_ids.map((v: any) => String(v)).filter(Boolean)
      : artifactId
        ? [artifactId]
        : [];
    nodes.push({
      id,
      type: String(frag?.type ?? 'fragment'),
      label: labelFromContent(frag?.content, id.slice(0, 8)),
      level: typeof frag?.level === 'number' ? frag.level : undefined,
      salience: typeof frag?.salience === 'number' ? frag.salience : undefined,
      meta: {
        artifact_id: artifactId,
        artifact_ids: artifactIds,
      },
    });
  }

  for (const frag of fragments) {
    const source = String(frag?.id ?? '');
    const radicals = Array.isArray(frag?.radicals) ? frag.radicals : [];
    for (const child of radicals) {
      const target = String(child ?? '');
      if (!source || !target) continue;
      edges.push({ id: `${source}:${target}`, source, target, kind: 'composition' });
    }
  }

  return { nodes, edges };
};
