import type { RunEntry } from '../api/client';

type RunTableProps = {
  runs: RunEntry[];
  activeRunId: string | null;
  onSelectRun: (runId: string) => void;
};

const getRunStatus = (run: RunEntry): 'succeeded' | 'failed' | 'pending' => {
  if (run.evaluation?.report?.passed === true) return 'succeeded';
  if (run.evaluation?.report?.passed === false) return 'failed';
  return 'pending';
};

const RunTable = ({ runs, activeRunId, onSelectRun }: RunTableProps) => {
  if (!runs.length) {
    return <div className="panel-placeholder">No runs yet</div>;
  }

  return (
    <div className="run-table run-table-compact" aria-label="Latest runs navigator">
      {[...runs].reverse().map((run) => {
        const lastStage = Array.isArray(run.stages) ? run.stages[run.stages.length - 1] : null;
        const lastStepLabel = lastStage?.stage_name ?? 'No stages yet';
        const durationLabel = typeof lastStage?.duration_ms === 'number' ? `${lastStage.duration_ms} ms` : 'n/a';
        const status = getRunStatus(run);
        const isActive = activeRunId === run.run_id;

        return (
          <button
            type="button"
            key={run.run_id}
            className={`run-nav-row ${isActive ? 'run-nav-row-active' : ''}`}
            onClick={() => onSelectRun(run.run_id)}
          >
            <span className={`run-nav-status run-nav-status-${status}`} aria-hidden="true" />
            <span className="run-nav-copy">
              <strong>{run.run_id}</strong>
              <span>{run.case_id}</span>
              <small>{lastStepLabel} - {durationLabel}</small>
            </span>
          </button>
        );
      })}
    </div>
  );
};

export default RunTable;
