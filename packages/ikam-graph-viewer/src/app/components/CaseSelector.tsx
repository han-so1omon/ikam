import { useMemo, useState } from 'react';

import type { CaseMeta } from '../api/client';

type CaseSelectorProps = {
  cases: CaseMeta[];
  selectedCaseIds: string[];
  onToggleCase: (caseId: string) => void;
  disabled?: boolean;
};

const CaseSelector = ({ cases, selectedCaseIds, onToggleCase, disabled = false }: CaseSelectorProps) => {
  const [query, setQuery] = useState('');
  const searchInputId = 'case-selector-search';

  if (!cases.length) {
    return <div className="panel-placeholder" data-testid="case-selector-empty">No cases available</div>;
  }

  const normalizedQuery = query.trim().toLowerCase();
  const visibleCases = useMemo(() => {
    if (!normalizedQuery) {
      return cases;
    }
    return cases.filter((item) => {
      const haystack = `${item.case_id} ${item.domain} ${item.size_tier} chaos:${item.chaos_level ?? ''}`.toLowerCase();
      return haystack.includes(normalizedQuery);
    });
  }, [cases, normalizedQuery]);

  return (
    <div className="case-selector-shell" data-testid="run-debug-case-selector-shell">
      <input
        id={searchInputId}
        name={searchInputId}
        type="search"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder="Search by case, domain, or size"
        aria-label="Search cases"
        className="case-selector-search"
        disabled={disabled}
      />
      <div className="case-selector case-selector-compact" role="group" aria-label="Case selector" data-testid="run-debug-case-selector">
        {visibleCases.map((item) => {
        const checked = selectedCaseIds.includes(item.case_id);
        return (
          <label key={item.case_id} className="case-option">
            <input
              name={`case-selector-${item.case_id}`}
              type="checkbox"
              checked={checked}
              onChange={() => onToggleCase(item.case_id)}
              disabled={disabled}
            />
            <span>{item.case_id}</span>
            <span className="decision-meta">
              {item.domain} / {item.size_tier}
              {typeof item.chaos_level === 'number' ? ` / chaos ${item.chaos_level}` : ''}
              {item.deliberate_contradictions ? ' / contradictions' : ''}
            </span>
          </label>
        );
        })}
        {visibleCases.length === 0 ? <div className="panel-placeholder">No matching cases</div> : null}
      </div>
    </div>
  );
};

export default CaseSelector;
