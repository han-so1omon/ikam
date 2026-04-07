import { render, screen } from '@testing-library/react';
import DecisionTrace from '../components/DecisionTrace';

test('renders decision trace heading', () => {
  render(<DecisionTrace runId={null} graphId={null} decisions={[]} loading={false} error={null} />);
  expect(screen.getByText('Decision Trace')).toBeInTheDocument();
});

test('renders graph context when provided', () => {
  render(<DecisionTrace runId="run-1" graphId="graph-1" decisions={[]} loading={false} error={null} />);
  expect(screen.getByText('graph-1')).toBeInTheDocument();
});
