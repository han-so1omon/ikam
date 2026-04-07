# Conexiones — IKAM Graph Viewer

Reusable Three.js graph renderer for IKAM fragments/artifacts.

## Install

Local workspace package.

## Usage

```ts
import { createIKAMGraph, mapIkamFragmentsToGraph } from '@narraciones/ikam-graph-viewer';

const data = mapIkamFragmentsToGraph(fragments);
const handle = createIKAMGraph(containerEl, data, {
  onNodeClick: (node) => console.log(node.id),
});

handle.update(data);
handle.destroy();
```

## API
- `createIKAMGraph(container, data, options)`
- `GraphHandle.update(data)`
- `GraphHandle.setOptions(options)`
- `GraphHandle.focusNode(nodeId)`
- `GraphHandle.destroy()`
