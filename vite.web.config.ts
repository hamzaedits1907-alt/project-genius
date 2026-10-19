/// <reference types="vitest/config" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';

// Genius Web: the browser build (landing page + the signed-in app), independent of the Tauri desktop
// packaging. The Tauri APIs the shared interface imports are replaced by browser implementations that
// talk to the web runtime (src/web). Output: dist-web/ (what Netlify publishes).
const p = (rel: string) => fileURLToPath(new URL(rel, import.meta.url));

export default defineConfig({
  root: 'web',
  publicDir: 'public',
  plugins: [react()],
  define: { __GENIUS_TARGET__: JSON.stringify('web') },
  resolve: {
    alias: [
      { find: '@tauri-apps/api/core', replacement: p('./src/web/shims/tauri-core.ts') },
      { find: '@tauri-apps/api/event', replacement: p('./src/web/shims/tauri-event.ts') },
      { find: '@tauri-apps/plugin-opener', replacement: p('./src/web/shims/tauri-opener.ts') },
      { find: '@tauri-apps/plugin-dialog', replacement: p('./src/web/shims/tauri-dialog.ts') },
    ],
  },
  server: { port: 1430, strictPort: true, fs: { allow: ['..'] } },
  envDir: '..',
  envPrefix: ['VITE_'],
  build: {
    outDir: '../dist-web',
    emptyOutDir: true,
    target: 'es2022',
    sourcemap: false,
    chunkSizeWarningLimit: 2500,
    rollupOptions: { input: { landing: p('./web/index.html'), app: p('./web/app/index.html') } },
  },
});
