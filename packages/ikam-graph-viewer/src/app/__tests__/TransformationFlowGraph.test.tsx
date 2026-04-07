import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { vi } from 'vitest';

import TransformationFlowGraph from '../components/debug/TransformationFlowGraph';

test('renders svg flow nodes and connecting edges', async () => {
  render(
    <div style={{ width: '1000px', height: '1000px' }}>
      <TransformationFlowGraph
        ariaLabel="Transformation flow for artifact"
        nodes={[
          { id: 'a1', label: 'artifact:run-1', kind: 'artifact', stage: 'artifact', summary: 'Artifact root', state: 'complete', linkedNodeId: 'artifact:run-1' },
          { id: 's1', label: 'surface-1', kind: 'surface', stage: 'surface', summary: 'Surface fragment', state: 'complete', linkedNodeId: 'fragment:surface-1' },
          { id: 'i1', label: 'ir-1', kind: 'ir', stage: 'ir', summary: 'Lifted claim', state: 'attention', linkedNodeId: 'fragment:ir-1' },
          { id: 'n1', label: 'norm-1', kind: 'normalized', stage: 'normalized', summary: 'Normalized claim', state: 'complete', linkedNodeId: 'fragment:norm-1' },
        ]}
        edges={[
          { id: 'e1', from: 'a1', to: 's1' },
          { id: 'e2', from: 's1', to: 'i1' },
          { id: 'e3', from: 'i1', to: 'n1' },
        ]}
      />
    </div>
  );

  expect(screen.getByTestId('transformation-flow-graph')).toBeInTheDocument();
  expect(screen.getByRole('figure', { name: 'Transformation flow for artifact' })).toBeInTheDocument();
  
  await waitFor(() => {
    expect(screen.getByText('artifact:run-1')).toBeInTheDocument();
  });
  expect(screen.getByText('surface-1')).toBeInTheDocument();
  expect(screen.getByText('ir-1')).toBeInTheDocument();
  expect(screen.getByText('norm-1')).toBeInTheDocument();
});

test('does not warn about missing React Flow parent dimensions in its default container', async () => {
  const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

  render(
    <TransformationFlowGraph
      ariaLabel="Transformation flow for artifact"
      nodes={[
        { id: 'a1', label: 'artifact:run-1', kind: 'artifact', stage: 'artifact', summary: 'Artifact root', state: 'complete', linkedNodeId: 'artifact:run-1' },
        { id: 's1', label: 'surface-1', kind: 'surface', stage: 'surface', summary: 'Surface fragment', state: 'complete', linkedNodeId: 'fragment:surface-1' },
      ]}
      edges={[
        { id: 'e1', from: 'a1', to: 's1' },
      ]}
    />
  );

  await waitFor(() => {
    expect(screen.getByRole('figure', { name: 'Transformation flow for artifact' })).toBeInTheDocument();
  });

  expect(screen.getByTestId('transformation-flow-graph')).toHaveStyle({
    width: '100%',
    height: '330px',
  });

  const warnOutput = warnSpy.mock.calls.flatMap((call) => call.map(String)).join('\n');
  expect(warnOutput).not.toContain('The React Flow parent container needs a width and a height');

  warnSpy.mockRestore();
});

test('uses the shared flow shell controls and interaction states', async () => {
  const onNodeClick = vi.fn();

  render(
      <TransformationFlowGraph
        ariaLabel="Transformation flow shell"
        onNodeClick={onNodeClick}
        nodes={[
        { id: 'a1', label: 'artifact:run-1', nodeType: 'data', kind: 'artifact', stage: 'artifact', summary: 'Artifact root', state: 'complete', linkedNodeId: 'artifact:run-1' },
        { id: 'i1', label: 'Lift Claims', nodeType: 'step', kind: 'operation', stage: 'ir', summary: 'Lifted claim', state: 'attention' },
        ]}
        edges={[{ id: 'e1', from: 'a1', to: 'i1' }]}
      />
  );

  expect(screen.getByTestId('mock-react-flow')).toBeInTheDocument();
  expect(screen.getByTestId('mock-controls')).toBeInTheDocument();
  expect(screen.getByTestId('mock-background')).toBeInTheDocument();

  const artifactNode = await screen.findByTestId('transformation-flow-node-a1');
  const irNode = screen.getByTestId('transformation-flow-node-i1');

  expect(artifactNode).toHaveAttribute('data-kind', 'artifact');
  expect(irNode).toHaveAttribute('data-kind', 'operation');
  expect(artifactNode).toHaveAttribute('data-state', 'complete');
  expect(irNode).toHaveAttribute('data-state', 'attention');

  fireEvent.click(irNode);
  expect(onNodeClick).toHaveBeenCalledWith('i1');
});
