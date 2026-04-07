import { useEffect, useState } from 'react';
import { JsonView, allExpanded, darkStyles, defaultStyles } from 'react-json-view-lite';
import 'react-json-view-lite/dist/index.css';

import {
  type RegistryEntry,
  type SubgraphResponse,
  getRegistryNamespaces,
  getRegistry,
  getSubgraph,
} from '../api/client';
import PetriNetViewer from './PetriNetViewer';

export default function RegistryWorkspace() {
  const [namespaces, setNamespaces] = useState<string[]>([]);
  const [selectedNamespace, setSelectedNamespace] = useState<string>('');

  const [entries, setEntries] = useState<RegistryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [selectedEntry, setSelectedEntry] = useState<RegistryEntry | null>(null);
  const [subgraph, setSubgraph] = useState<SubgraphResponse | null>(null);
  const [loadingSubgraph, setLoadingSubgraph] = useState(false);
  const [subgraphError, setSubgraphError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<'visual' | 'json'>('visual');

  const isPetriRenderable = Boolean(
    selectedEntry && subgraph && (
      subgraph.head?.profile === 'petri_net' ||
      subgraph.head?.data?.schema_id === 'modelado/petri-net-envelope@1' ||
      (subgraph.head?.profile === 'ikam_executable_graph' && subgraph.head?.data?.rich_petri_snapshot)
    )
  );

  useEffect(() => {
    const fetchNamespaces = async () => {
      try {
        const nsList = await getRegistryNamespaces();
        setNamespaces(nsList);
        if (nsList.length > 0 && !selectedNamespace) {
          setSelectedNamespace(nsList[0]);
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to fetch registry namespaces');
      }
    };
    void fetchNamespaces();
  }, []);

  const fetchRegistry = async (namespace: string) => {
    if (!namespace) return;
    setLoading(true);
    setError(null);
    try {
      const data = await getRegistry(namespace);
      setEntries(data.entries);
      setSelectedEntry(null);
      setSubgraph(null);
      setSubgraphError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Unknown error fetching registry');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (selectedNamespace) {
      void fetchRegistry(selectedNamespace);
    }
  }, [selectedNamespace]);

  const handleSelect = async (entry: RegistryEntry) => {
    setSelectedEntry(entry);
    const subgraphId = entry.fragment_id || entry.head_fragment_id;
    if (!subgraphId) {
      setSubgraphError('No fragment_id or head_fragment_id on this entry');
      setSubgraph(null);
      return;
    }

    setLoadingSubgraph(true);
    setSubgraphError(null);
    setSubgraph(null);
    try {
      const data = await getSubgraph(subgraphId);
      setSubgraph(data);
    } catch (err) {
      setSubgraphError(err instanceof Error ? err.message : 'Failed to fetch subgraph');
    } finally {
      setLoadingSubgraph(false);
    }
  };

  return (
    <div className="workspace split-workspace" style={{ display: 'flex', height: '100%', gap: '1rem' }}>
      <div className="pane left-pane" style={{ width: '300px', borderRight: '1px solid var(--border-color)', display: 'flex', flexDirection: 'column' }}>
        <header className="pane-header" style={{ flexDirection: 'column', alignItems: 'stretch', gap: '0.5rem' }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <h2>Registries</h2>
            <button onClick={() => fetchRegistry(selectedNamespace)} disabled={loading || !selectedNamespace}>
              {loading ? 'Refreshing...' : 'Refresh'}
            </button>
          </div>
          {namespaces.length > 0 && (
            <select 
              value={selectedNamespace} 
              onChange={(e) => setSelectedNamespace(e.target.value)}
              style={{ width: '100%', padding: '0.25rem', marginTop: '0.25rem' }}
            >
              {namespaces.map(ns => (
                <option key={ns} value={ns}>{ns}</option>
              ))}
            </select>
          )}
        </header>

        {error && <div className="error-banner">{error}</div>}

        <div className="pane-content" style={{ overflowY: 'auto', flex: 1, padding: '1rem' }}>
          {entries.length === 0 && !loading && !error ? (
            <p className="empty-state">No entries in this registry.</p>
          ) : (
            <ul className="entry-list" style={{ listStyle: 'none', padding: 0, margin: 0 }}>
              {entries.map((entry) => (
                <li
                  key={entry.key}
                  style={{
                    padding: '0.75rem',
                    marginBottom: '0.5rem',
                    borderRadius: '4px',
                    cursor: 'pointer',
                    background: selectedEntry?.key === entry.key ? 'var(--bg-active)' : 'var(--bg-surface)',
                    border: '1px solid var(--border-color)'
                  }}
                  onClick={() => handleSelect(entry)}
                >
                  <div style={{ fontWeight: 600 }}>{entry.title || entry.key || 'Untitled'}</div>
                  <div style={{ fontSize: '0.8rem', opacity: 0.7, marginTop: '0.25rem' }}>
                    {entry.registered_at ? new Date(entry.registered_at).toLocaleString() : 'Unknown date'}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      <div className="pane right-pane" style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <header className="pane-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
            <h2>{selectedEntry ? selectedEntry.title || selectedEntry.key || 'Artifact Details' : 'Select an entry'}</h2>
            {selectedEntry && isPetriRenderable && (
              <select value={viewMode} onChange={(e) => setViewMode(e.target.value as 'visual' | 'json')} style={{ padding: '2px 4px' }}>
                <option value="visual">Visual</option>
                <option value="json">JSON</option>
              </select>
            )}
          </div>
          {(selectedEntry?.fragment_id || selectedEntry?.head_fragment_id) && (
            <span style={{ opacity: 0.6, fontSize: '0.85rem', fontFamily: 'monospace' }}>
              {selectedEntry.fragment_id || selectedEntry.head_fragment_id}
            </span>
          )}
        </header>
        <div className="pane-content" style={{ overflowY: 'auto', flex: 1, padding: '1rem' }}>
          {!selectedEntry && (
            <div className="empty-state">Select an entry from the list to view its subgraph details.</div>
          )}
          {loadingSubgraph && <div className="loading-state">Loading subgraph...</div>}
          {subgraphError && <div className="error-banner">{subgraphError}</div>}
          
          {subgraph && (
            <>
              {isPetriRenderable && viewMode === 'visual' ? (
                <div style={{ height: 'calc(100vh - 120px)', border: '1px solid var(--border-color)', borderRadius: '4px' }}>
                  <PetriNetViewer head={subgraph.head} childrenFragments={subgraph.children} />
                </div>
              ) : (
                <div className="json-container" style={{ fontFamily: 'monospace', fontSize: '13px' }}>
                  <JsonView 
                    data={subgraph} 
                    shouldExpandNode={allExpanded} 
                    style={defaultStyles} 
                  />
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
