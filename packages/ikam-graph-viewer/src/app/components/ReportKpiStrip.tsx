import type { AnswerQualitySummary, GraphSummary } from '../api/client';

type ReportKpiStripProps = {
  answerQuality?: AnswerQualitySummary;
  summary: GraphSummary;
};

const formatPct = (value: number) => `${Math.round(value * 100)}%`;

const ReportKpiStrip = ({ answerQuality, summary }: ReportKpiStripProps) => {
  const aqs = answerQuality?.aqs ?? 0;
  const storageGain = Math.max(0, 1 - summary.edges / Math.max(summary.nodes * 2, 1));
  const reliability = Math.min(1, (summary.semantic_relations + 1) / Math.max(summary.edges + 1, 1));

  const items = [
    { label: 'Answer Quality', value: formatPct(aqs), hint: answerQuality?.review_mode ?? 'oracle-defaulted' },
    { label: 'Storage Gains', value: formatPct(storageGain), hint: `${summary.nodes} nodes / ${summary.edges} edges` },
    { label: 'Reliability', value: formatPct(reliability), hint: `${summary.semantic_relations} semantic links` },
  ];

  return (
    <div className="kpi-strip" data-testid="kpi-strip">
      {items.map((item) => (
        <article key={item.label} className="kpi-card glass-panel">
          <div className="kpi-label" data-testid="kpi-label">
            {item.label}
          </div>
          <div className="kpi-value">{item.value}</div>
          <div className="kpi-hint">{item.hint}</div>
        </article>
      ))}
    </div>
  );
};

export default ReportKpiStrip;
