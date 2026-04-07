import { useMemo } from 'react';

type PairwiseSimilarity = {
  fragment_ids: string[];
  matrix: number[][];
  threshold?: number | null;
};

type EmbeddingDebug = {
  expected_count?: number;
  embedded_count?: number;
  coverage_ratio?: number;
  singleton_clusters?: number;
  missing_fragment_ids?: string[];
  threshold?: number;
  embedding_mode?: string | null;
};

type FragmentListItem = {
  fragment_id: string;
  label: string;
};

type Props = {
  pairwise: PairwiseSimilarity;
  debug: EmbeddingDebug;
  fragmentItems: FragmentListItem[];
  selectedFragmentId: string | null;
  selectedPair: { sourceFragmentId: string; targetFragmentId: string } | null;
  onSelectFragment: (fragmentId: string) => void;
  onSelectPair: (sourceFragmentId: string, targetFragmentId: string, similarity: number) => void;
};

const similarityColor = (value: number): string => {
  const clamped = Math.max(-1, Math.min(1, value));
  const hue = Math.round(((clamped + 1) / 2) * 220);
  const light = 86 - Math.round(((clamped + 1) / 2) * 44);
  return `hsl(${hue}, 78%, ${light}%)`;
};

const EmbeddingShapePanel = ({ pairwise, debug, fragmentItems, selectedFragmentId, selectedPair, onSelectFragment, onSelectPair }: Props) => {
  const fragmentLabelById = useMemo(
    () => new Map(fragmentItems.map((item) => [item.fragment_id, item.label])),
    [fragmentItems]
  );

  const nearestNeighbors = useMemo(() => {
    if (!selectedFragmentId) return [] as Array<{ fragmentId: string; score: number }>;
    const index = pairwise.fragment_ids.indexOf(selectedFragmentId);
    if (index < 0) return [] as Array<{ fragmentId: string; score: number }>;
    const row = Array.isArray(pairwise.matrix[index]) ? pairwise.matrix[index] : [];
    return pairwise.fragment_ids
      .map((fragmentId, idx) => ({ fragmentId, score: Number(row[idx] ?? 0) }))
      .filter((item) => item.fragmentId !== selectedFragmentId)
      .sort((a, b) => b.score - a.score)
      .slice(0, 6);
  }, [pairwise.fragment_ids, pairwise.matrix, selectedFragmentId]);

  return (
    <section className="embedding-shape-panel" data-testid="embedding-shape-panel">
      <h5>Embedding Similarity</h5>
      <div className="embedding-shape-metrics">
        <span>Expected: {debug.expected_count ?? pairwise.fragment_ids.length}</span>
        <span>Embedded: {debug.embedded_count ?? pairwise.fragment_ids.length}</span>
        <span>Coverage: {typeof debug.coverage_ratio === 'number' ? `${(debug.coverage_ratio * 100).toFixed(1)}%` : 'n/a'}</span>
        <span>Singleton Clusters: {debug.singleton_clusters ?? 'n/a'}</span>
        <span>Mode: {debug.embedding_mode ?? 'n/a'}</span>
      </div>

      <div className="embedding-shape-grid">
        <div className="embedding-nearest embedding-nearest-card" data-testid="embedding-nearest-list">
          <h6>Closest Fragments</h6>
          {selectedFragmentId ? (
            <p className="embedding-nearest-context">
              Anchor: <strong>{fragmentLabelById.get(selectedFragmentId) ?? selectedFragmentId}</strong>
            </p>
          ) : (
            <p className="embedding-nearest-context">Select a fragment from Drill Through to inspect nearest neighbors.</p>
          )}
          {selectedFragmentId ? (
            <ul>
              {nearestNeighbors.map((item) => (
                <li key={`nn-${item.fragmentId}`}>
                  <button
                    type="button"
                    className={selectedPair && selectedPair.sourceFragmentId === selectedFragmentId && selectedPair.targetFragmentId === item.fragmentId ? 'embedding-nearest-active' : ''}
                    aria-label={`Closest fragment ${fragmentLabelById.get(item.fragmentId) ?? item.fragmentId}`}
                    onClick={() => {
                      const sourceId = selectedFragmentId ?? item.fragmentId;
                      onSelectFragment(sourceId);
                      onSelectPair(sourceId, item.fragmentId, item.score);
                    }}
                  >
                    {fragmentLabelById.get(item.fragmentId) ?? item.fragmentId}
                  </button>
                  <span>{item.score.toFixed(3)}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>

        <div className="embedding-heatmap" data-testid="embedding-heatmap">
          <h6>Pairwise Similarity Heatmap</h6>
          <div className="embedding-heatmap-wrap">
            <table>
              <thead>
                <tr>
                  <th />
                  {pairwise.fragment_ids.map((id) => (
                    <th key={`col-${id}`}>{id.slice(0, 6)}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {pairwise.fragment_ids.map((sourceId, rowIndex) => (
                  <tr key={`row-${sourceId}`}>
                    <th>{sourceId.slice(0, 6)}</th>
                    {pairwise.fragment_ids.map((targetId, colIndex) => {
                      const value = Number(pairwise.matrix?.[rowIndex]?.[colIndex] ?? 0);
                      const isSelectedPair = Boolean(
                        selectedPair
                        && selectedPair.sourceFragmentId === sourceId
                        && selectedPair.targetFragmentId === targetId
                      );
                      return (
                        <td key={`cell-${sourceId}-${targetId}`}>
                          <button
                            type="button"
                            className={`embedding-heat-cell ${isSelectedPair ? 'embedding-heat-cell-active' : ''}`}
                            style={{ background: similarityColor(value) }}
                            title={`${sourceId} -> ${targetId}: ${value.toFixed(3)}`}
                            onClick={() => {
                              onSelectFragment(sourceId);
                              onSelectPair(sourceId, targetId, value);
                            }}
                          >
                            {value.toFixed(2)}
                          </button>
                        </td>
                      );
                    })}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="embedding-heatmap-note">Threshold: {typeof pairwise.threshold === 'number' ? pairwise.threshold.toFixed(2) : 'n/a'}</p>
        </div>
      </div>

      {Array.isArray(debug.missing_fragment_ids) && debug.missing_fragment_ids.length > 0 ? (
        <p className="embedding-missing">Missing embeddings: {debug.missing_fragment_ids.slice(0, 8).join(', ')}</p>
      ) : null}
    </section>
  );
};

export default EmbeddingShapePanel;
