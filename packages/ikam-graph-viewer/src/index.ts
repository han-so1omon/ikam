import type { GraphData, GraphHandle, GraphOptions } from './types';
import { GraphView } from './GraphView';

export const createIKAMGraph = (
  container: HTMLElement,
  data: GraphData,
  options: GraphOptions = {}
): GraphHandle => {
  const view = new GraphView(container, data, options);
  return {
    update: (next) => view.update(next),
    setOptions: (opts) => view.setOptions(opts),
    focusNode: (nodeId) => view.focusNode(nodeId),
    focusGroup: (groupId) => view.focusGroup(groupId),
    fitToNodes: (nodeIds) => view.fitToNodes(nodeIds),
    fitToGroup: (groupId) => view.fitToGroup(groupId),
    fitToData: () => view.fitToData(),
    destroy: () => view.destroy(),
  };
};

export * from './types';
export * from './adapter/ikam';
export * from './explainability/types';
export * from './explainability/adapters';
export * from './explainability/inspector';
