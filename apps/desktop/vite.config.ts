import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Tauri v2 expects the web assets in `../dist` relative to `src-tauri`, i.e. `apps/desktop/dist`.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // The Rust side is rebuilt by cargo, not by Vite.
      ignored: ['**/src-tauri/**', '**/target/**'],
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'chrome110',
    sourcemap: false,
  },
});
