declare module 'three' {
  // TypeScript moduleResolution=Bundler can fail to resolve `three`'s typings under certain
  // `exports` configurations. This shim unblocks builds; runtime bundling still uses real `three`.
  const THREE: any;
  export = THREE;
}

declare module 'three/examples/jsm/controls/OrbitControls' {
  export const OrbitControls: any;
}
