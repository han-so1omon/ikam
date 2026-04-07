import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls';
import type { GraphData, GraphEdge, GraphNode, GraphOptions } from './types';
import { layoutNeighborhoods } from './graph/neighborhood_layout';
import { buildSemanticGroups } from './graph/semantic_groups';
import {
  buildCameraTweenFrames,
  computeBoundsForNodeIds,
  computeCameraDistance,
  computeCenterFromBounds,
  type Bounds3D,
  type CameraFrame,
} from './graph/camera_fit';
import { getNodeColor, softGlassGraphTheme } from './theme';

const DEFAULTS: Required<Pick<GraphOptions, 'background' | 'nodeSize' | 'edgeOpacity' | 'orbitControls'>> = {
  background: softGlassGraphTheme.boardBackground,
  nodeSize: 6,
  edgeOpacity: 0.72,
  orbitControls: true,
};

const EDGE_BASE_COLOR = new THREE.Color('#2f4358');
const EDGE_HOVER_COLOR = new THREE.Color('#2f7cd8');
const EDGE_SELECTED_COLOR = new THREE.Color('#2f8f6f');

const NODE_HOVER_COLOR = new THREE.Color('#fbbf24');
const NODE_BRIGHTEN_SCALAR = 1.25;
const NODE_SELECTED_COLOR = new THREE.Color('#7662bd');
const NODE_MULTI_SELECTED_COLOR = new THREE.Color('#2f8f6f');
const NODE_DIMMED_VISIBILITY_FLOOR = softGlassGraphTheme.dimmedScalarFloor;

const EDGE_HIGHLIGHT_OPACITY = 0.9;

export function shouldEnableNodeHalo(meta?: Record<string, unknown>) {
  if (!meta) return false;
  return Boolean(meta.pulse || meta.highlighted || meta.selected);
}

export function getNodeDimmedScalar() {
  return NODE_DIMMED_VISIBILITY_FLOOR;
}

export function getSoftGlassGroupBubbleStyle() {
  return softGlassGraphTheme.groupBubble;
}

/** Resolve which hit target takes priority when both node and edge are under cursor. Nodes win. */
export function resolveHitPriority(nodeHit: boolean, edgeHit: boolean): 'node' | 'edge' | null {
  if (nodeHit) return 'node';
  if (edgeHit) return 'edge';
  return null;
}

/** Sinusoidal pulse intensity for render-chain edge animation. Returns 0–1. */
export function computeEdgePulseIntensity(timeMs: number): number {
  const period = 2000; // 2 second full cycle
  return (Math.sin((timeMs / period) * Math.PI * 2) + 1) / 2;
}

function createSoftNodeSpriteTexture() {
  const size = 64;
  const canvas = document.createElement('canvas');
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext('2d');
  if (!context) return null;

  const center = size / 2;
  const radius = size * 0.38;
  const gradient = context.createRadialGradient(center, center, radius * 0.05, center, center, radius);
  gradient.addColorStop(0, 'rgba(255,255,255,1)');
  gradient.addColorStop(0.74, 'rgba(255,255,255,0.97)');
  gradient.addColorStop(1, 'rgba(255,255,255,0)');

  context.clearRect(0, 0, size, size);
  context.fillStyle = gradient;
  context.beginPath();
  context.arc(center, center, radius, 0, Math.PI * 2);
  context.fill();

  const texture = new THREE.CanvasTexture(canvas);
  texture.needsUpdate = true;
  texture.generateMipmaps = true;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  texture.magFilter = THREE.LinearFilter;
  return texture;
}

function createSelfLoopRingTexture() {
  const size = 96;
  const canvas = document.createElement('canvas');
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext('2d');
  if (!context) return null;

  const center = size / 2;
  const radius = size * 0.32;
  const ringWidth = size * 0.13;
  context.clearRect(0, 0, size, size);
  context.beginPath();
  context.arc(center, center, radius, 0, Math.PI * 2);
  context.strokeStyle = 'rgba(42, 63, 85, 0.96)';
  context.lineWidth = ringWidth;
  context.stroke();

  const texture = new THREE.CanvasTexture(canvas);
  texture.needsUpdate = true;
  texture.generateMipmaps = true;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  texture.magFilter = THREE.LinearFilter;
  return texture;
}

export class GraphView {
  private container: HTMLElement;
  private renderer: any;
  private scene: any;
  private camera: any;
  private controls?: any;
  private resizeObserver?: ResizeObserver;
  private raycaster: any;
  private mouse: any;
  private animationId: number | null = null;
  private nodeMesh?: any;
  private nodeOutlineMesh?: any;
  private nodeHaloMesh?: any;
  private nodeSpriteTexture?: THREE.CanvasTexture;
  private selfLoopMesh?: any;
  private selfLoopTexture?: THREE.CanvasTexture;
  private nodeColors?: any;
  private nodeColorArray?: Float32Array;
  private nodeBaseColorArray?: Float32Array;
  private edgeMesh?: any;
  private edgeColors?: any;
  private edgeColorArray?: Float32Array;
  private edgeHoverMesh?: any;
  private edgeSelectedMesh?: any;
  private edgeGroupMesh?: any;
  private edgeGroupPositions?: Float32Array;
  private edgeOutlineMesh?: any;
  private edgeSegmentToEdgeIndex: number[] = [];
  private edgeVertexRangeByEdgeIndex: Array<{ start: number; count: number }> = [];
  private groupBubbleMeshes: THREE.Mesh[] = [];
  private groupLabelSprites: THREE.Sprite[] = [];
  private groupByNodeId: Map<string, string> = new Map();
  private groupCenters: Map<
    string,
    { x: number; y: number; z: number; radius: number; label: string; count: number }
  > = new Map();
  private groupBuild?: ReturnType<typeof buildSemanticGroups>;
  private pointerDirty = false;
  private nodes: GraphNode[] = [];
  private edges: GraphEdge[] = [];
  private options: GraphOptions;
  private nodeIndex = new Map<string, number>();
  private lastPositions?: Float32Array;
  private hoveredEdgeIndex: number | null = null;
  private selectedEdgeIndex: number | null = null;
  private hoveredNodeIndex: number | null = null;
  private selectedNodeIndex: number | null = null;
  private selectedNodeIds = new Set<string>();
  private hasHighlightedNodes = false;
  private hasEdgePulse = false;
  private lastPointer: { x: number; y: number } | null = null;
  private selectionBoxEl: HTMLDivElement | null = null;
  private isBoxSelecting = false;
  private isBoxSelectArmed = false;
  private boxSelectStart: { x: number; y: number } | null = null;
  private ignoreNextClick = false;
  private cameraTweenId: number | null = null;

  constructor(container: HTMLElement, data: GraphData, options: GraphOptions = {}) {
    this.container = container;
    this.options = { ...DEFAULTS, ...options };

    this.scene = new THREE.Scene();
    this.scene.background = new THREE.Color(this.options.background ?? DEFAULTS.background);

    const width = Math.max(1, container.clientWidth);
    const height = Math.max(1, container.clientHeight);
    this.camera = new THREE.PerspectiveCamera(60, width / height, 0.1, 5000);
    this.camera.position.set(0, 0, 300);

    this.renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
    // Cap DPR to reduce framebuffer memory on high-DPI displays.
    this.renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
    this.renderer.setSize(width, height);
    container.appendChild(this.renderer.domElement);
    // Avoid browser gesture handling (esp. trackpad/touch) interfering with controls.
    (this.renderer.domElement.style as any).touchAction = 'none';

    this.raycaster = new THREE.Raycaster();
    this.raycaster.params.Points.threshold = 6;
    this.raycaster.params.Line = { threshold: 4 };
    this.mouse = new THREE.Vector2();

    if (this.options.orbitControls) {
      this.controls = new OrbitControls(this.camera, this.renderer.domElement);
      this.controls.enableDamping = true;
      this.controls.dampingFactor = 0.08;
      this.controls.enableRotate = false;
      this.controls.enablePan = true;
      this.controls.enableZoom = true;
      this.controls.zoomSpeed = 1.15;
      this.controls.panSpeed = 0.9;
      this.controls.screenSpacePanning = true;
      this.controls.minDistance = 10;
      this.controls.maxDistance = 5000;
      this.controls.zoomToCursor = true;
      this.controls.mouseButtons = {
        LEFT: THREE.MOUSE.PAN,
        MIDDLE: THREE.MOUSE.DOLLY,
        RIGHT: THREE.MOUSE.ROTATE,
      };
    }

    this.ensureSelectionOverlay();

    this.attachListeners();
    this.update(data);
    this.animate();
  }

  update(data: GraphData) {
    this.nodes = data.nodes ?? [];
    this.edges = data.edges ?? [];
    this.nodeIndex = new Map(this.nodes.map((n, idx) => [n.id, idx]));
    this.hasHighlightedNodes = this.nodes.some((n) => Boolean((n as any)?.meta && (n as any).meta.highlighted));
    this.hasEdgePulse = this.edges.some((e) => Boolean((e as any)?.meta?.pulse));
    this.hoveredEdgeIndex = null;
    this.selectedEdgeIndex = null;
    this.hoveredNodeIndex = null;
    this.selectedNodeIndex = null;
    this.selectedNodeIds = new Set();

    const fallbackPositions = this.layoutNodes(this.nodes);
    const groupBuild = buildSemanticGroups(this.nodes, []);
    this.groupBuild = groupBuild;
    this.groupByNodeId = groupBuild.nodeGroup;
    const groupNodes = new Map(groupBuild.groupOrder.map((groupId) => [groupId, groupBuild.groupsById.get(groupId)?.nodeIds ?? []]));
    const neighborhood = layoutNeighborhoods({
      groupOrder: groupBuild.groupOrder,
      groupNodes,
    });
    this.groupCenters = new Map(
      Array.from(neighborhood.groupCenters.entries()).map(([groupId, center]) => {
        const info = groupBuild.groupsById.get(groupId);
        return [
          groupId,
          {
            ...center,
            label: info?.label ?? groupId,
            count: info?.nodeIds.length ?? 0,
          },
        ];
      })
    );
    const positions = this.buildNeighborhoodPositions(fallbackPositions, neighborhood.nodePositions);
    this.lastPositions = positions;
    this.nodeColorArray = new Float32Array(this.nodes.length * 3);

    for (let i = 0; i < this.nodes.length; i += 1) {
      const node = this.nodes[i];
      const meta: any = (node as any)?.meta ?? {};
      const pulse = Boolean(meta?.pulse);
      const highlighted = Boolean(meta?.highlighted);
      const dimmed = Boolean(meta?.dimmed);
      const selected = Boolean(meta?.selected);

      let color = new THREE.Color(getNodeColor(node.type));
      color.multiplyScalar(NODE_BRIGHTEN_SCALAR);
      if (pulse || highlighted || selected) color = new THREE.Color('#2f8f6f');
      else if (dimmed) color.multiplyScalar(NODE_DIMMED_VISIBILITY_FLOOR);

      this.nodeColorArray[i * 3] = color.r;
      this.nodeColorArray[i * 3 + 1] = color.g;
      this.nodeColorArray[i * 3 + 2] = color.b;
    }

    this.nodeBaseColorArray = new Float32Array(this.nodeColorArray);

    const nodeGeometry = new THREE.BufferGeometry();
    nodeGeometry.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    this.nodeColors = new THREE.BufferAttribute(this.nodeColorArray, 3);
    nodeGeometry.setAttribute('color', this.nodeColors);

    if (!this.nodeSpriteTexture) {
      this.nodeSpriteTexture = createSoftNodeSpriteTexture() ?? undefined;
    }

    const nodeMaterial = new THREE.PointsMaterial({
      size: this.options.nodeSize ?? DEFAULTS.nodeSize,
      vertexColors: true,
      map: this.nodeSpriteTexture,
      alphaTest: 0.2,
      transparent: true,
      sizeAttenuation: false,
      depthTest: false,
    });

    if (this.nodeMesh) {
      this.scene.remove(this.nodeMesh);
      this.nodeMesh.geometry.dispose();
      (this.nodeMesh.material as any).dispose();
    }

    this.nodeMesh = new THREE.Points(nodeGeometry, nodeMaterial);
    this.nodeMesh.renderOrder = 2;
    this.scene.add(this.nodeMesh);

    const outlineMaterial = new THREE.PointsMaterial({
      size: (this.options.nodeSize ?? DEFAULTS.nodeSize) * 1.7,
      color: '#9eb0c5',
      opacity: 0.2,
      map: this.nodeSpriteTexture,
      alphaTest: 0.18,
      transparent: true,
      depthWrite: false,
      sizeAttenuation: false,
      depthTest: false,
    });

    if (this.nodeOutlineMesh) {
      this.scene.remove(this.nodeOutlineMesh);
      this.nodeOutlineMesh.geometry.dispose();
      (this.nodeOutlineMesh.material as any).dispose();
    }

    this.nodeOutlineMesh = new THREE.Points(nodeGeometry, outlineMaterial);
    this.nodeOutlineMesh.renderOrder = 1;
    this.scene.add(this.nodeOutlineMesh);

    const haloMaterial = new THREE.PointsMaterial({
      size: (this.options.nodeSize ?? DEFAULTS.nodeSize) * 2.7,
      color: '#a9d2ff',
      opacity: 0,
      map: this.nodeSpriteTexture,
      alphaTest: 0.1,
      transparent: true,
      depthWrite: false,
      sizeAttenuation: false,
      depthTest: false,
      blending: THREE.AdditiveBlending,
    });

    if (this.nodeHaloMesh) {
      this.scene.remove(this.nodeHaloMesh);
      this.nodeHaloMesh.geometry.dispose();
      (this.nodeHaloMesh.material as any).dispose();
    }

    this.nodeHaloMesh = new THREE.Points(nodeGeometry, haloMaterial);
    this.nodeHaloMesh.renderOrder = 0;
    this.scene.add(this.nodeHaloMesh);

    const selfLoopNodeIndices = new Set<number>();
    for (const edge of this.edges) {
      if (edge.source !== edge.target) continue;
      const idx = this.nodeIndex.get(edge.source);
      if (idx != null) selfLoopNodeIndices.add(idx);
    }

    if (!this.selfLoopTexture) {
      this.selfLoopTexture = createSelfLoopRingTexture() ?? undefined;
    }

    if (this.selfLoopMesh) {
      this.scene.remove(this.selfLoopMesh);
      this.selfLoopMesh.geometry.dispose();
      (this.selfLoopMesh.material as any).dispose();
      this.selfLoopMesh = undefined;
    }

    if (selfLoopNodeIndices.size && this.selfLoopTexture) {
      const selfLoopPositions = new Float32Array(selfLoopNodeIndices.size * 3);
      const selfLoopColors = new Float32Array(selfLoopNodeIndices.size * 3);
      let pointer = 0;
      for (const idx of selfLoopNodeIndices) {
        selfLoopPositions[pointer * 3] = positions[idx * 3];
        selfLoopPositions[pointer * 3 + 1] = positions[idx * 3 + 1];
        selfLoopPositions[pointer * 3 + 2] = positions[idx * 3 + 2];
        selfLoopColors[pointer * 3] = EDGE_BASE_COLOR.r;
        selfLoopColors[pointer * 3 + 1] = EDGE_BASE_COLOR.g;
        selfLoopColors[pointer * 3 + 2] = EDGE_BASE_COLOR.b;
        pointer += 1;
      }

      const loopGeometry = new THREE.BufferGeometry();
      loopGeometry.setAttribute('position', new THREE.BufferAttribute(selfLoopPositions, 3));
      loopGeometry.setAttribute('color', new THREE.BufferAttribute(selfLoopColors, 3));
      const loopMaterial = new THREE.PointsMaterial({
        size: (this.options.nodeSize ?? DEFAULTS.nodeSize) * 6.2,
        vertexColors: true,
        map: this.selfLoopTexture,
        alphaTest: 0.2,
        transparent: true,
        opacity: 1,
        sizeAttenuation: false,
        depthTest: false,
      });
      this.selfLoopMesh = new THREE.Points(loopGeometry, loopMaterial);
      this.selfLoopMesh.renderOrder = 1;
      this.scene.add(this.selfLoopMesh);
    }

    const edgeLayout = this.layoutEdges(positions);
    const edgePositions = edgeLayout.positions;
    this.edgeSegmentToEdgeIndex = edgeLayout.segmentToEdgeIndex;
    this.edgeVertexRangeByEdgeIndex = edgeLayout.vertexRanges;
    const edgeGeometry = new THREE.BufferGeometry();
    edgeGeometry.setAttribute('position', new THREE.BufferAttribute(edgePositions, 3));

    this.edgeColorArray = new Float32Array(edgePositions.length);
    this.edgeColors = new THREE.BufferAttribute(this.edgeColorArray, 3);
    edgeGeometry.setAttribute('color', this.edgeColors);
    this.updateEdgeColors();

    const edgeMaterial = new THREE.LineBasicMaterial({
      transparent: true,
      opacity: this.options.edgeOpacity ?? DEFAULTS.edgeOpacity,
      vertexColors: true,
    });

    const edgeOutlineMaterial = new THREE.LineBasicMaterial({
      transparent: true,
      opacity: 0.5,
      color: '#0b1015',
    });

    if (this.edgeMesh) {
      this.scene.remove(this.edgeMesh);
      this.edgeMesh.geometry.dispose();
      (this.edgeMesh.material as any).dispose();
    }

    this.edgeMesh = new THREE.LineSegments(edgeGeometry, edgeMaterial);
    this.scene.add(this.edgeMesh);

    if (this.edgeOutlineMesh) {
      this.scene.remove(this.edgeOutlineMesh);
      this.edgeOutlineMesh.geometry.dispose();
      (this.edgeOutlineMesh.material as any).dispose();
    }

    this.edgeOutlineMesh = new THREE.LineSegments(edgeGeometry, edgeOutlineMaterial);
    this.edgeOutlineMesh.renderOrder = -1;
    this.scene.add(this.edgeOutlineMesh);

    this.ensureGroupEdgeMesh();

    this.updateGroupDecorations();

    this.fitCameraToData(positions);
    this.ensureEdgeHighlightMeshes();
    this.updateNodeColors();
    this.updateEdgeHighlightMeshes();
    this.updateNodeHaloVisibility();
  }

  fitToData() {
    if (!this.lastPositions) return;
    const bounds = this.computeBoundsFromPositions(this.lastPositions);
    this.fitCameraToBounds(bounds);
  }

  fitToNodes(nodeIds: string[]) {
    if (!this.lastPositions || !nodeIds.length) return;
    const bounds = computeBoundsForNodeIds(nodeIds, this.nodeIndex, this.lastPositions);
    this.fitCameraToBounds(bounds);
  }

  fitToGroup(groupId: string) {
    const group = this.groupBuild?.groupsById.get(groupId);
    if (!group?.nodeIds.length) return;
    this.fitToNodes(group.nodeIds);
  }

  private fitCameraToData(positions: Float32Array) {
    if (!positions.length) return;
    this.fitCameraToBounds(this.computeBoundsFromPositions(positions));
  }

  private fitCameraToBounds(bounds: Bounds3D) {
    const center = computeCenterFromBounds(bounds);
    const distance = computeCameraDistance(bounds, this.camera.fov);

    this.container.setAttribute('data-camera-distance', String(distance));
    this.options.onViewportChange?.({
      width: this.container.clientWidth,
      height: this.container.clientHeight,
      cameraDistance: distance,
    });

    this.animateCameraTo(center, distance);
    this.camera.near = 0.1;
    this.camera.far = Math.max(5000, distance * 10);
    this.camera.updateProjectionMatrix();
  }

  private animateCameraTo(center: { x: number; y: number; z: number }, distance: number) {
    if (this.cameraTweenId != null) {
      cancelAnimationFrame(this.cameraTweenId);
      this.cameraTweenId = null;
    }

    const start: CameraFrame = {
      x: this.camera.position.x,
      y: this.camera.position.y,
      z: this.camera.position.z,
      targetX: this.controls?.target.x ?? center.x,
      targetY: this.controls?.target.y ?? center.y,
      targetZ: this.controls?.target.z ?? center.z,
    };
    const end: CameraFrame = {
      x: center.x,
      y: center.y,
      z: center.z + distance,
      targetX: center.x,
      targetY: center.y,
      targetZ: center.z,
    };
    const frames = buildCameraTweenFrames(start, end, 7);
    let index = 0;

    const step = () => {
      const frame = frames[index];
      if (!frame) return;
      this.camera.position.set(frame.x, frame.y, frame.z);
      if (this.controls) {
        this.controls.target.set(frame.targetX, frame.targetY, frame.targetZ);
        this.controls.update();
      }
      index += 1;
      if (index < frames.length) {
        this.cameraTweenId = requestAnimationFrame(step);
      } else {
        this.cameraTweenId = null;
      }
    };

    this.cameraTweenId = requestAnimationFrame(step);
  }

  private computeBoundsFromPositions(positions: Float32Array): Bounds3D {
    let minX = Number.POSITIVE_INFINITY;
    let minY = Number.POSITIVE_INFINITY;
    let minZ = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let maxY = Number.NEGATIVE_INFINITY;
    let maxZ = Number.NEGATIVE_INFINITY;

    for (let i = 0; i < positions.length; i += 3) {
      const x = positions[i];
      const y = positions[i + 1];
      const z = positions[i + 2];
      if (x < minX) minX = x;
      if (y < minY) minY = y;
      if (z < minZ) minZ = z;
      if (x > maxX) maxX = x;
      if (y > maxY) maxY = y;
      if (z > maxZ) maxZ = z;
    }

    return { minX, minY, minZ, maxX, maxY, maxZ };
  }

  setOptions(options: Partial<GraphOptions>) {
    this.options = { ...this.options, ...options };
    if (options.background) {
      this.scene.background = new THREE.Color(options.background);
    }
    if (this.nodeMesh && options.nodeSize) {
      const material = this.nodeMesh.material as any;
      material.size = options.nodeSize;
    }
    if (this.nodeOutlineMesh && options.nodeSize) {
      const material = this.nodeOutlineMesh.material as any;
      material.size = options.nodeSize * 1.7;
    }
    if (this.edgeMesh && options.edgeOpacity != null) {
      const material = this.edgeMesh.material as any;
      material.opacity = options.edgeOpacity;
    }
    if (options.showGroups != null || options.activeGroupId != null) {
      this.updateGroupDecorations();
    }
  }

  focusNode(nodeId: string) {
    this.fitToNodes([nodeId]);
  }

  focusGroup(groupId: string) {
    this.fitToGroup(groupId);
  }

  destroy() {
    if (this.animationId != null) cancelAnimationFrame(this.animationId);
    this.animationId = null;
    if (this.cameraTweenId != null) cancelAnimationFrame(this.cameraTweenId);
    this.cameraTweenId = null;
    this.controls?.dispose();
    this.resizeObserver?.disconnect();
    this.resizeObserver = undefined;

    if (this.nodeMesh) {
      this.scene.remove(this.nodeMesh);
      this.nodeMesh.geometry?.dispose?.();
      this.nodeMesh.material?.dispose?.();
      this.nodeMesh = undefined;
    }
    if (this.nodeOutlineMesh) {
      this.scene.remove(this.nodeOutlineMesh);
      this.nodeOutlineMesh.geometry?.dispose?.();
      this.nodeOutlineMesh.material?.dispose?.();
      this.nodeOutlineMesh = undefined;
    }
    if (this.nodeHaloMesh) {
      this.scene.remove(this.nodeHaloMesh);
      this.nodeHaloMesh.geometry?.dispose?.();
      this.nodeHaloMesh.material?.dispose?.();
      this.nodeHaloMesh = undefined;
    }
    if (this.selfLoopMesh) {
      this.scene.remove(this.selfLoopMesh);
      this.selfLoopMesh.geometry?.dispose?.();
      this.selfLoopMesh.material?.dispose?.();
      this.selfLoopMesh = undefined;
    }

    if (this.edgeMesh) {
      this.scene.remove(this.edgeMesh);
      this.edgeMesh.geometry?.dispose?.();
      this.edgeMesh.material?.dispose?.();
      this.edgeMesh = undefined;
    }
    if (this.edgeOutlineMesh) {
      this.scene.remove(this.edgeOutlineMesh);
      this.edgeOutlineMesh.geometry?.dispose?.();
      this.edgeOutlineMesh.material?.dispose?.();
      this.edgeOutlineMesh = undefined;
    }
    if (this.edgeGroupMesh) {
      this.scene.remove(this.edgeGroupMesh);
      this.edgeGroupMesh.geometry?.dispose?.();
      this.edgeGroupMesh.material?.dispose?.();
      this.edgeGroupMesh = undefined;
    }

    if (this.edgeHoverMesh) {
      this.scene.remove(this.edgeHoverMesh);
      this.edgeHoverMesh.geometry?.dispose?.();
      this.edgeHoverMesh.material?.dispose?.();
      this.edgeHoverMesh = undefined;
    }
    if (this.edgeSelectedMesh) {
      this.scene.remove(this.edgeSelectedMesh);
      this.edgeSelectedMesh.geometry?.dispose?.();
      this.edgeSelectedMesh.material?.dispose?.();
      this.edgeSelectedMesh = undefined;
    }

    this.clearGroupDecorations();

    this.renderer.dispose();
    try {
      // Encourage GPU memory release when the view is torn down.
      this.renderer.forceContextLoss?.();
    } catch {
      // ignore
    }
    this.nodeSpriteTexture?.dispose();
    this.nodeSpriteTexture = undefined;
    this.selfLoopTexture?.dispose();
    this.selfLoopTexture = undefined;

    if (this.selectionBoxEl?.parentElement) this.selectionBoxEl.parentElement.removeChild(this.selectionBoxEl);
    if (this.renderer?.domElement?.parentElement === this.container) {
      this.container.removeChild(this.renderer.domElement);
    }
    this.detachListeners();
  }

  private ensureSelectionOverlay() {
    try {
      const style = window.getComputedStyle(this.container);
      if (style.position === 'static') {
        this.container.style.position = 'relative';
      }
    } catch {
      // ignore
    }

    if (this.selectionBoxEl) return;
    const el = document.createElement('div');
    el.style.position = 'absolute';
    el.style.left = '0px';
    el.style.top = '0px';
    el.style.width = '0px';
    el.style.height = '0px';
    el.style.border = '1px solid rgba(125, 211, 252, 0.9)';
    el.style.background = 'rgba(125, 211, 252, 0.10)';
    el.style.pointerEvents = 'none';
    el.style.display = 'none';
    el.style.zIndex = '10';
    this.container.appendChild(el);
    this.selectionBoxEl = el;
  }

  private ensureEdgeHighlightMeshes() {
    if (this.edgeHoverMesh && this.edgeSelectedMesh) return;

    const makeLine = (color: any) => {
      const geom = new THREE.BufferGeometry();
      const attr = new THREE.BufferAttribute(new Float32Array(0), 3);
      geom.setAttribute('position', attr);
      geom.setDrawRange(0, 0);
      const mat = new THREE.LineBasicMaterial({
        transparent: true,
        opacity: EDGE_HIGHLIGHT_OPACITY,
        color,
      });
      const line = new THREE.LineSegments(geom, mat);
      line.renderOrder = 5;
      return line;
    };

    this.edgeHoverMesh = makeLine(EDGE_HOVER_COLOR);
    this.edgeSelectedMesh = makeLine(EDGE_SELECTED_COLOR);
    this.scene.add(this.edgeHoverMesh);
    this.scene.add(this.edgeSelectedMesh);
  }

  private ensureGroupEdgeMesh() {
    if (this.edgeGroupMesh) return;
    const geom = new THREE.BufferGeometry();
    this.edgeGroupPositions = new Float32Array(0);
    geom.setAttribute('position', new THREE.BufferAttribute(this.edgeGroupPositions, 3));
    geom.setDrawRange(0, 0);
    const mat = new THREE.LineBasicMaterial({
      transparent: true,
      opacity: 0.8,
      color: EDGE_HOVER_COLOR,
    });
    const line = new THREE.LineSegments(geom, mat);
    line.renderOrder = 4;
    this.edgeGroupMesh = line;
    this.scene.add(line);
  }

  private updateEdgeHighlightMeshes() {
    if (!this.edgeMesh || !this.edgeHoverMesh || !this.edgeSelectedMesh) return;
    const base = (this.edgeMesh.geometry as any).getAttribute('position');

    const setEdge = (targetMesh: any, edgeIndex: number | null) => {
      if (edgeIndex == null || edgeIndex < 0 || edgeIndex >= this.edges.length) {
        (targetMesh.geometry as any).setDrawRange(0, 0);
        return;
      }
      const range = this.edgeVertexRangeByEdgeIndex[edgeIndex];
      if (!range || range.count < 2) {
        (targetMesh.geometry as any).setDrawRange(0, 0);
        return;
      }
      const source = base.array as Float32Array;
      const start = range.start * 3;
      const end = (range.start + range.count) * 3;
      const highlight = source.slice(start, end);
      const geometry = targetMesh.geometry as any;
      geometry.setAttribute('position', new THREE.BufferAttribute(highlight, 3));
      geometry.setDrawRange(0, range.count);
      geometry.attributes.position.needsUpdate = true;
    };

    setEdge(this.edgeHoverMesh, this.hoveredEdgeIndex);
    setEdge(this.edgeSelectedMesh, this.selectedEdgeIndex);
  }

  private updateNodeColors() {
    if (!this.nodeColors || !this.nodeColorArray || !this.nodeBaseColorArray) return;
    this.nodeColorArray.set(this.nodeBaseColorArray);

    // Multi-select (box) takes precedence for color.
    if (this.selectedNodeIds.size > 0) {
      for (const nodeId of this.selectedNodeIds) {
        const idx = this.nodeIndex.get(nodeId);
        if (idx == null) continue;
        this.nodeColorArray[idx * 3] = NODE_MULTI_SELECTED_COLOR.r;
        this.nodeColorArray[idx * 3 + 1] = NODE_MULTI_SELECTED_COLOR.g;
        this.nodeColorArray[idx * 3 + 2] = NODE_MULTI_SELECTED_COLOR.b;
      }
    }

    if (this.selectedNodeIndex != null && this.selectedNodeIndex >= 0) {
      this.nodeColorArray[this.selectedNodeIndex * 3] = NODE_SELECTED_COLOR.r;
      this.nodeColorArray[this.selectedNodeIndex * 3 + 1] = NODE_SELECTED_COLOR.g;
      this.nodeColorArray[this.selectedNodeIndex * 3 + 2] = NODE_SELECTED_COLOR.b;
    }

    if (this.hoveredNodeIndex != null && this.hoveredNodeIndex >= 0) {
      this.nodeColorArray[this.hoveredNodeIndex * 3] = NODE_HOVER_COLOR.r;
      this.nodeColorArray[this.hoveredNodeIndex * 3 + 1] = NODE_HOVER_COLOR.g;
      this.nodeColorArray[this.hoveredNodeIndex * 3 + 2] = NODE_HOVER_COLOR.b;
    }

    this.nodeColors.needsUpdate = true;
    this.updateNodeHaloVisibility();
  }

  private updateNodeHaloVisibility() {
    if (!this.nodeHaloMesh) return;
    const material = this.nodeHaloMesh.material as THREE.PointsMaterial;
    const hasInteractiveSelection =
      (this.selectedNodeIndex != null && this.selectedNodeIndex >= 0) || this.selectedNodeIds.size > 0;
    const hasSemanticHighlight = this.nodes.some((node) => shouldEnableNodeHalo((node as any)?.meta));
    material.opacity = hasInteractiveSelection || hasSemanticHighlight ? 0.22 : 0;
    material.needsUpdate = true;
  }

  private updateEdgeColors() {
    if (!this.edgeColorArray || !this.edgeColors) return;
    let activeGroupId: string | null = this.options.activeGroupId ?? null;
    if (this.selectedNodeIndex != null && this.selectedNodeIndex >= 0) {
      const nodeId = this.nodes[this.selectedNodeIndex]?.id;
      activeGroupId = nodeId ? this.groupByNodeId.get(nodeId) ?? null : null;
    } else if (this.selectedNodeIds.size > 0) {
      const ids = Array.from(this.selectedNodeIds);
      const firstGroup = ids.length ? this.groupByNodeId.get(ids[0]) : null;
      if (firstGroup && ids.every((id) => this.groupByNodeId.get(id) === firstGroup)) {
        activeGroupId = firstGroup;
      }
    }
    const edgeColors = new Array<THREE.Color>(this.edges.length);
    for (let i = 0; i < this.edges.length; i += 1) {
      const isSelected = this.selectedEdgeIndex === i;
      const isHovered = this.hoveredEdgeIndex === i;
      let color = isSelected ? EDGE_SELECTED_COLOR : isHovered ? EDGE_HOVER_COLOR : EDGE_BASE_COLOR;

      if (!isSelected && !isHovered && activeGroupId) {
        const edge = this.edges[i];
        const sourceGroup = this.groupByNodeId.get(edge.source);
        const targetGroup = this.groupByNodeId.get(edge.target);
        if (sourceGroup && sourceGroup === targetGroup && sourceGroup === activeGroupId) {
          color = EDGE_HOVER_COLOR;
        }
      }

      if (!isSelected && !isHovered && this.hasHighlightedNodes) {
        const edge = this.edges[i];
        const sIdx = this.nodeIndex.get(edge.source);
        const tIdx = this.nodeIndex.get(edge.target);
        const s = sIdx != null ? this.nodes[sIdx] : undefined;
        const t = tIdx != null ? this.nodes[tIdx] : undefined;
        const sHighlighted = Boolean((s as any)?.meta && (s as any).meta.highlighted);
        const tHighlighted = Boolean((t as any)?.meta && (t as any).meta.highlighted);
        if (!sHighlighted && !tHighlighted) {
          color = color.clone().multiplyScalar(Math.max(NODE_DIMMED_VISIBILITY_FLOOR, 0.72));
        }
      }
      edgeColors[i] = color;
    }

    // Pulse modulation for render-chain edges
    if (this.hasEdgePulse) {
      const intensity = computeEdgePulseIntensity(performance.now());
      const pulseScale = 0.6 + 0.4 * intensity; // subtle range: 0.6–1.0
      for (let i = 0; i < this.edges.length; i += 1) {
        if ((this.edges[i] as any)?.meta?.pulse) {
          edgeColors[i] = edgeColors[i].clone().multiplyScalar(pulseScale);
        }
      }
    }

    for (let segmentIndex = 0; segmentIndex < this.edgeSegmentToEdgeIndex.length; segmentIndex += 1) {
      const edgeIndex = this.edgeSegmentToEdgeIndex[segmentIndex] ?? -1;
      const color = edgeColors[edgeIndex] ?? EDGE_BASE_COLOR;
      const offset = segmentIndex * 6;
      this.edgeColorArray[offset] = color.r;
      this.edgeColorArray[offset + 1] = color.g;
      this.edgeColorArray[offset + 2] = color.b;
      this.edgeColorArray[offset + 3] = color.r;
      this.edgeColorArray[offset + 4] = color.g;
      this.edgeColorArray[offset + 5] = color.b;
    }
    this.edgeColors.needsUpdate = true;
    this.updateGroupEdgeMesh(activeGroupId);
  }

  private updateGroupEdgeMesh(activeGroupId: string | null) {
    if (!this.edgeGroupMesh || !this.edgeMesh) return;
    const base = (this.edgeMesh.geometry as any).getAttribute('position');
    if (!activeGroupId) {
      (this.edgeGroupMesh.geometry as any).setDrawRange(0, 0);
      return;
    }

    const positions: number[] = [];
    for (let i = 0; i < this.edges.length; i += 1) {
      const edge = this.edges[i];
      const sourceGroup = this.groupByNodeId.get(edge.source);
      const targetGroup = this.groupByNodeId.get(edge.target);
      if (sourceGroup && sourceGroup === targetGroup && sourceGroup === activeGroupId) {
        const range = this.edgeVertexRangeByEdgeIndex[i];
        if (!range || range.count < 2) continue;
        for (let vertex = 0; vertex < range.count; vertex += 1) {
          const index = range.start + vertex;
          positions.push(base.getX(index), base.getY(index), base.getZ(index));
        }
      }
    }

    const next = new Float32Array(positions);
    this.edgeGroupPositions = next;
    const geom = this.edgeGroupMesh.geometry as any;
    geom.setAttribute('position', new THREE.BufferAttribute(next, 3));
    geom.setDrawRange(0, positions.length / 3);
    geom.attributes.position.needsUpdate = true;
  }

  private buildNeighborhoodPositions(fallbackPositions: Float32Array, nodePositions: Map<string, { x: number; y: number; z: number }>) {
    if (!nodePositions.size) return fallbackPositions;
    const positions = new Float32Array(this.nodes.length * 3);
    for (let i = 0; i < this.nodes.length; i += 1) {
      const node = this.nodes[i];
      const pos = nodePositions.get(node.id);
      if (pos) {
        positions[i * 3] = pos.x;
        positions[i * 3 + 1] = pos.y;
        positions[i * 3 + 2] = pos.z;
      } else {
        positions[i * 3] = fallbackPositions[i * 3];
        positions[i * 3 + 1] = fallbackPositions[i * 3 + 1];
        positions[i * 3 + 2] = fallbackPositions[i * 3 + 2];
      }
    }
    return positions;
  }

  private updateGroupDecorations() {
    this.clearGroupDecorations();
    if (!this.groupCenters.size) return;

    const groupsToRender = getGroupDecorations(this.groupCenters, {
      showGroups: this.options.showGroups,
      activeGroupId: this.options.activeGroupId ?? null,
    });

    for (const [groupId, center] of groupsToRender) {
      const geometry = new THREE.CircleGeometry(center.radius, 64);
      const bubbleStyle = getSoftGlassGroupBubbleStyle();
      const bubbleMaterial = new THREE.MeshBasicMaterial({
        color: bubbleStyle.color,
        transparent: true,
        opacity: bubbleStyle.opacity,
        depthWrite: false,
      });
      const bubble = new THREE.Mesh(geometry, bubbleMaterial);
      bubble.position.set(center.x, center.y, center.z - 2);
      bubble.renderOrder = -2;
      this.scene.add(bubble);
      this.groupBubbleMeshes.push(bubble);

      const labelText = `${center.label} · ${center.count}`;
      const sprite = this.createLabelSprite(labelText);
      sprite.position.set(center.x, center.y + center.radius + 16, center.z + 4);
      sprite.renderOrder = 6;
      this.scene.add(sprite);
      this.groupLabelSprites.push(sprite);
    }
  }

  private createLabelSprite(text: string) {
    const canvas = document.createElement('canvas');
    const context = canvas.getContext('2d');
    if (!context) return new THREE.Sprite();

    const fontSize = 14;
    const paddingX = 10;
    const paddingY = 6;
    const font = `600 ${fontSize}px Inter, system-ui, sans-serif`;
    context.font = font;
    const metrics = context.measureText(text);
    const width = Math.ceil(metrics.width + paddingX * 2);
    const height = fontSize + paddingY * 2;
    const dpr = Math.max(1, window.devicePixelRatio || 1);
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    context.scale(dpr, dpr);

    context.clearRect(0, 0, width, height);
    context.fillStyle = 'rgba(15, 23, 42, 0.7)';
    context.strokeStyle = 'rgba(148, 163, 184, 0.6)';
    context.lineWidth = 1;
    const radius = 6;
    context.beginPath();
    context.moveTo(radius, 0);
    context.lineTo(width - radius, 0);
    context.quadraticCurveTo(width, 0, width, radius);
    context.lineTo(width, height - radius);
    context.quadraticCurveTo(width, height, width - radius, height);
    context.lineTo(radius, height);
    context.quadraticCurveTo(0, height, 0, height - radius);
    context.lineTo(0, radius);
    context.quadraticCurveTo(0, 0, radius, 0);
    context.closePath();
    context.fill();
    context.stroke();

    context.font = font;
    context.fillStyle = '#e2e8f0';
    context.textBaseline = 'middle';
    context.fillText(text, paddingX, height / 2);

    const texture = new THREE.CanvasTexture(canvas);
    texture.needsUpdate = true;
    const material = new THREE.SpriteMaterial({
      map: texture,
      transparent: true,
      depthWrite: false,
    });
    const sprite = new THREE.Sprite(material);
    const scaleFactor = 0.6;
    sprite.scale.set(width * scaleFactor, height * scaleFactor, 1);
    return sprite;
  }

  private clearGroupDecorations() {
    for (const bubble of this.groupBubbleMeshes) {
      this.scene.remove(bubble);
      bubble.geometry?.dispose?.();
      (bubble.material as THREE.Material | undefined)?.dispose?.();
    }
    this.groupBubbleMeshes = [];

    for (const sprite of this.groupLabelSprites) {
      this.scene.remove(sprite);
      const material = sprite.material as THREE.SpriteMaterial | undefined;
      material?.map?.dispose?.();
      material?.dispose?.();
    }
    this.groupLabelSprites = [];
  }

  private layoutNodes(nodes: GraphNode[]): Float32Array {
    const positions = new Float32Array(nodes.length * 3);
    const spacing = 45;
    const radiusBase = 80;

    nodes.forEach((node, idx) => {
      const angle = (idx / Math.max(nodes.length, 1)) * Math.PI * 2;
      const level = node.level ?? 0;
      const radius = radiusBase + Math.max(0, level) * spacing;
      positions[idx * 3] = Math.cos(angle) * radius;
      positions[idx * 3 + 1] = Math.sin(angle) * radius;
      positions[idx * 3 + 2] = level * 20;
    });

    return positions;
  }

  private layoutEdges(nodePositions: Float32Array): {
    positions: Float32Array;
    segmentToEdgeIndex: number[];
    vertexRanges: Array<{ start: number; count: number }>;
  } {
    const segments: number[] = [];
    const segmentToEdgeIndex: number[] = [];
    const vertexRanges: Array<{ start: number; count: number }> = new Array(this.edges.length);

    this.edges.forEach((edge, edgeIndex) => {
      const sourceIdx = this.nodeIndex.get(edge.source) ?? 0;
      const targetIdx = this.nodeIndex.get(edge.target) ?? 0;
      const sx = nodePositions[sourceIdx * 3];
      const sy = nodePositions[sourceIdx * 3 + 1];
      const sz = nodePositions[sourceIdx * 3 + 2];
      const tx = nodePositions[targetIdx * 3];
      const ty = nodePositions[targetIdx * 3 + 1];
      const tz = nodePositions[targetIdx * 3 + 2];
      const startVertex = segments.length / 3;

      if (sourceIdx === targetIdx) {
        const loopSegments = 28;
        const loopRadius = Math.max(36, (this.options.nodeSize ?? DEFAULTS.nodeSize) * 4.5);
        const offsetRadius = loopRadius * 1.8;
        const centerX = sx + offsetRadius;
        const centerY = sy;
        for (let i = 0; i < loopSegments; i += 1) {
          const a0 = (i / loopSegments) * Math.PI * 2;
          const a1 = ((i + 1) / loopSegments) * Math.PI * 2;
          segments.push(
            centerX + Math.cos(a0) * loopRadius,
            centerY + Math.sin(a0) * loopRadius,
            sz,
            centerX + Math.cos(a1) * loopRadius,
            centerY + Math.sin(a1) * loopRadius,
            sz
          );
          segmentToEdgeIndex.push(edgeIndex);
        }
        vertexRanges[edgeIndex] = { start: startVertex, count: loopSegments * 2 };
        return;
      }

      segments.push(sx, sy, sz, tx, ty, tz);
      segmentToEdgeIndex.push(edgeIndex);
      vertexRanges[edgeIndex] = { start: startVertex, count: 2 };
    });

    return {
      positions: new Float32Array(segments),
      segmentToEdgeIndex,
      vertexRanges,
    };
  }

  private animate() {
    this.animationId = requestAnimationFrame(() => this.animate());
    this.controls?.update();
    if (this.pointerDirty) {
      this.pointerDirty = false;
      this.processHover();
    }
    if (this.hasEdgePulse) {
      this.updateEdgeColors();
    }
    this.renderer.render(this.scene, this.camera);
  }

  private processHover() {
    if (!this.nodeMesh && !this.edgeMesh) return;
    if (this.isBoxSelecting) return;

    this.raycaster.setFromCamera(this.mouse, this.camera);

    // Check nodes FIRST — nodes have priority over edges when both are under cursor.
    if (this.nodeMesh) {
      const hits = this.raycaster.intersectObject(this.nodeMesh);
      if (hits.length > 0) {
        const idx = hits[0].index ?? -1;
        const node = this.nodes[idx];
        if (idx !== this.hoveredNodeIndex) {
          this.hoveredNodeIndex = idx >= 0 ? idx : null;
          this.updateNodeColors();
        }
        if (node) this.options.onNodeHover?.(node);
        // Clear edge hover since node takes priority
        if (this.hoveredEdgeIndex != null) {
          this.hoveredEdgeIndex = null;
          this.updateEdgeColors();
          this.updateEdgeHighlightMeshes();
          this.options.onEdgeHover?.(null);
        }
        return;
      }
    }

    // No node hit — check edges.
    let hoveredEdgeIndex: number | null = null;
    if (this.edgeMesh) {
      const edgeHits = this.raycaster.intersectObject(this.edgeMesh);
      if (edgeHits.length > 0) {
        const idx = edgeHits[0].index ?? -1;
        const segmentIndex = idx >= 0 ? Math.floor(idx / 2) : -1;
        hoveredEdgeIndex = segmentIndex >= 0 ? this.edgeSegmentToEdgeIndex[segmentIndex] ?? null : null;
      }
    }
    if (hoveredEdgeIndex !== this.hoveredEdgeIndex) {
      this.hoveredEdgeIndex = hoveredEdgeIndex;
      this.updateEdgeColors();
      this.updateEdgeHighlightMeshes();
      this.options.onEdgeHover?.(hoveredEdgeIndex != null ? this.edges[hoveredEdgeIndex] ?? null : null);
    }

    // No node and no edge — clear hover state.
    this.options.onNodeHover?.(null);
    if (this.hoveredNodeIndex != null) {
      this.hoveredNodeIndex = null;
      this.updateNodeColors();
    }
  }

  private attachListeners() {
    this.renderer.domElement.addEventListener('pointermove', this.onPointerMove);
    // Capture to intercept Shift+drag before OrbitControls (which listens in bubble phase).
    this.renderer.domElement.addEventListener('pointerdown', this.onPointerDown, { capture: true });
    this.renderer.domElement.addEventListener('click', this.onClick);
    window.addEventListener('resize', this.onResize);

    if (typeof ResizeObserver !== 'undefined') {
      this.resizeObserver = new ResizeObserver(() => {
        this.onResize();
      });
      this.resizeObserver.observe(this.container);
    }
  }

  private detachListeners() {
    this.renderer.domElement.removeEventListener('pointermove', this.onPointerMove);
    this.renderer.domElement.removeEventListener('pointerdown', this.onPointerDown, { capture: true } as any);
    this.renderer.domElement.removeEventListener('click', this.onClick);
    window.removeEventListener('resize', this.onResize);
    this.resizeObserver?.disconnect();
    this.resizeObserver = undefined;
  }

  private onPointerDown = (event: PointerEvent) => {
    // Shift + left-drag: box-select nodes (do NOT steal Shift+click; arm first, activate on movement)
    if (!event.shiftKey || event.button !== 0 || !this.selectionBoxEl) return;
    // Do not preventDefault here: Firefox may suppress the subsequent click if we do,
    // breaking Shift+click toggle. Stopping propagation keeps OrbitControls from starting.
    event.stopPropagation();
    if (this.controls) this.controls.enabled = false;

    const rect = this.renderer.domElement.getBoundingClientRect();
    this.isBoxSelectArmed = true;
    this.isBoxSelecting = false;
    this.boxSelectStart = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    this.selectionBoxEl.style.display = 'none';

    window.addEventListener('pointermove', this.onBoxSelectMove);
    window.addEventListener('pointerup', this.onBoxSelectUp, { once: true });
    window.addEventListener('pointercancel', this.onBoxSelectUp, { once: true });
  };

  private onBoxSelectMove = (event: PointerEvent) => {
    if (!this.isBoxSelectArmed || !this.boxSelectStart || !this.selectionBoxEl) return;
    const rect = this.renderer.domElement.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;

    if (!this.isBoxSelecting) {
      const dx = x - this.boxSelectStart.x;
      const dy = y - this.boxSelectStart.y;
      // Only activate box selection if the pointer actually moves.
      if (Math.hypot(dx, dy) < 5) return;
      this.isBoxSelecting = true;
      this.selectionBoxEl.style.display = 'block';
      this.selectionBoxEl.style.left = `${this.boxSelectStart.x}px`;
      this.selectionBoxEl.style.top = `${this.boxSelectStart.y}px`;
      this.selectionBoxEl.style.width = '0px';
      this.selectionBoxEl.style.height = '0px';
      if (this.controls) this.controls.enabled = false;
    }

    const left = Math.min(this.boxSelectStart.x, x);
    const top = Math.min(this.boxSelectStart.y, y);
    const width = Math.abs(x - this.boxSelectStart.x);
    const height = Math.abs(y - this.boxSelectStart.y);
    this.selectionBoxEl.style.left = `${left}px`;
    this.selectionBoxEl.style.top = `${top}px`;
    this.selectionBoxEl.style.width = `${width}px`;
    this.selectionBoxEl.style.height = `${height}px`;
  };

  private onBoxSelectUp = (event: PointerEvent) => {
    window.removeEventListener('pointermove', this.onBoxSelectMove);

    if (!this.isBoxSelectArmed || !this.boxSelectStart || !this.selectionBoxEl) {
      this.isBoxSelectArmed = false;
      this.isBoxSelecting = false;
      this.boxSelectStart = null;
      if (this.controls) this.controls.enabled = true;
      return;
    }

    // If we never activated (i.e., Shift+click), don't treat it as a box selection.
    if (!this.isBoxSelecting) {
      this.isBoxSelectArmed = false;
      this.boxSelectStart = null;
      if (this.controls) this.controls.enabled = true;
      return;
    }

    const rect = this.renderer.domElement.getBoundingClientRect();
    const end = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    const left = Math.min(this.boxSelectStart.x, end.x);
    const top = Math.min(this.boxSelectStart.y, end.y);
    const right = Math.max(this.boxSelectStart.x, end.x);
    const bottom = Math.max(this.boxSelectStart.y, end.y);

    this.isBoxSelectArmed = false;
    this.isBoxSelecting = false;
    this.boxSelectStart = null;
    this.selectionBoxEl.style.display = 'none';
    if (this.controls) this.controls.enabled = true;

    const selected: string[] = [];
    if (this.nodeMesh) {
      const pos = (this.nodeMesh.geometry as any).getAttribute('position');
      const v = new THREE.Vector3();
      for (let i = 0; i < this.nodes.length; i += 1) {
        v.set(pos.getX(i), pos.getY(i), pos.getZ(i));
        v.project(this.camera);
        const sx = ((v.x + 1) / 2) * rect.width;
        const sy = ((-v.y + 1) / 2) * rect.height;
        if (sx >= left && sx <= right && sy >= top && sy <= bottom) {
          selected.push(this.nodes[i].id);
        }
      }
    }

    this.selectedNodeIds = new Set(selected);
    this.selectedNodeIndex = null;
    this.hoveredNodeIndex = null;
    this.ignoreNextClick = true;
    this.updateNodeColors();
    this.options.onNodesSelected?.(selected);
    this.options.onSelectionChange?.({
      selectedNodeId: undefined,
      selectedEdgeId: undefined,
      selectedNodeIds: selected,
    });
  };

  private onResize = () => {
    const width = Math.max(1, this.container.clientWidth);
    const height = Math.max(1, this.container.clientHeight);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(width, height);
    this.options.onViewportChange?.({
      width,
      height,
      cameraDistance: this.camera.position.distanceTo(this.controls?.target ?? new THREE.Vector3(0, 0, 0)),
    });
  };

  private onPointerMove = (event: MouseEvent) => {
    if (!this.nodeMesh && !this.edgeMesh) return;
    if (this.isBoxSelecting) return;

    const rect = this.renderer.domElement.getBoundingClientRect();
    this.lastPointer = {
      x: event.clientX - rect.left,
      y: event.clientY - rect.top,
    };
    this.mouse.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
    this.mouse.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;
    this.pointerDirty = true;
    if (this.lastPointer) {
      this.options.onPointerMove?.(this.lastPointer);
    }
  };

  private onClick = (event: MouseEvent) => {
    if (!this.nodeMesh && !this.edgeMesh) return;
    if (this.ignoreNextClick) {
      this.ignoreNextClick = false;
      return;
    }

    const rect = this.renderer.domElement.getBoundingClientRect();
    this.mouse.x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
    this.mouse.y = -((event.clientY - rect.top) / rect.height) * 2 + 1;
    this.raycaster.setFromCamera(this.mouse, this.camera);

    // Check nodes FIRST — nodes have priority over edges when both are under cursor.
    if (this.nodeMesh) {
      const hits = this.raycaster.intersectObject(this.nodeMesh);
      if (hits.length > 0) {
        const idx = hits[0].index ?? -1;
        const node = this.nodes[idx];
        if (!node) return;

        // Shift+click toggles membership in multi-selection.
        if (event.shiftKey) {
          const next = new Set(this.selectedNodeIds);
          if (next.has(node.id)) next.delete(node.id);
          else next.add(node.id);
          this.selectedNodeIds = next;
          this.selectedNodeIndex = null;
          this.selectedEdgeIndex = null;
          this.updateEdgeColors();
          this.updateEdgeHighlightMeshes();
          this.updateNodeColors();
          this.options.onNodesSelected?.(Array.from(next));
          this.options.onSelectionChange?.({
            selectedNodeId: undefined,
            selectedEdgeId: undefined,
            selectedNodeIds: Array.from(next),
          });
          return;
        }

        this.selectedNodeIndex = idx;
        this.selectedNodeIds = new Set();
        this.selectedEdgeIndex = null;
        this.updateEdgeColors();
        this.updateEdgeHighlightMeshes();
        this.updateNodeColors();
        const groupId = this.groupByNodeId.get(node.id);
        if (groupId) this.focusGroup(groupId);
        this.options.onNodeClick?.(node);
        this.options.onSelectionChange?.({
          selectedNodeId: node.id,
          selectedEdgeId: undefined,
          selectedNodeIds: [],
        });
        return;
      }
    }

    // No node hit — check edges.
    if (this.edgeMesh && this.options.onEdgeClick) {
      const edgeHits = this.raycaster.intersectObject(this.edgeMesh);
      if (edgeHits.length > 0) {
        const idx = edgeHits[0].index ?? -1;
        const segmentIndex = idx >= 0 ? Math.floor(idx / 2) : -1;
        const edgeIndex = segmentIndex >= 0 ? this.edgeSegmentToEdgeIndex[segmentIndex] ?? -1 : -1;
        const edge = this.edges[edgeIndex];
        if (edge) {
          this.selectedEdgeIndex = edgeIndex;
          this.updateEdgeColors();
          this.updateEdgeHighlightMeshes();
          this.selectedNodeIndex = null;
          this.selectedNodeIds = new Set();
          this.updateNodeColors();
          this.options.onEdgeClick(edge);
          this.options.onSelectionChange?.({
            selectedNodeId: undefined,
            selectedEdgeId: edge.id,
            selectedNodeIds: [],
          });
          return;
        }
      }
    }

    // Nothing hit — clear all selection.
    this.selectedNodeIndex = null;
    this.selectedNodeIds = new Set();
    this.selectedEdgeIndex = null;
    this.updateEdgeColors();
    this.updateEdgeHighlightMeshes();
    this.updateNodeColors();
    this.options.onSelectionChange?.({
      selectedNodeId: undefined,
      selectedEdgeId: undefined,
      selectedNodeIds: [],
    });
  };
}

type GroupDecoration = [
  string,
  { x: number; y: number; z: number; radius: number; label: string; count: number }
];

export function getGroupDecorations(
  groupCenters: Map<string, { x: number; y: number; z: number; radius: number; label: string; count: number }>,
  options: Pick<GraphOptions, 'showGroups' | 'activeGroupId'>
): GroupDecoration[] {
  if (!groupCenters.size) return [];
  const activeGroupId = options.activeGroupId ?? null;
  const shouldRenderAll = Boolean(options.showGroups) && !activeGroupId;
  if (shouldRenderAll) return Array.from(groupCenters.entries());
  if (activeGroupId && groupCenters.has(activeGroupId)) {
    return [[activeGroupId, groupCenters.get(activeGroupId)!]];
  }
  return [];
}
