import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/three')) return 'vendor-three';
          if (id.includes('node_modules/react')) return 'vendor-react';
          if (id.includes('node_modules')) return 'vendor';
          return undefined;
        },
      },
    },
  },
  server: {
    allowedHosts: ['ikam-graph-viewer', 'localhost', '127.0.0.1'],
  },
  test: {
    environment: 'jsdom',
    setupFiles: './src/app/test-setup.ts',
    globals: true,
  },
});
