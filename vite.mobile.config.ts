/// <reference types="vitest/config" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';

// The Android and iOS apps (Capacitor): the same signed-in web app as Genius Web, bundled into the app with
// the app screen as the root page. Output: dist-mobile/ (Capacitor's webDir). It behaves like the web build
// (browser runtime, account, relay) and calls the web functions at an absolute address (VITE_API_BASE).
const p = (rel: string) => fileURLToPath(new URL(rel, import.meta.url));

export default defineConfig({
  root: 'web/app',
  base: './',
  publicDir: false,
  plugins: [react()],
  define: { __GENIUS_TARGET__: JSON.stringify('web'), __GENIUS_MOBILE__: JSON.stringify(true) },
  resolve: {
    alias: [
      { find: '@tauri-apps/api/core', replacement: p('./src/web/shims/tauri-core.ts') },
      { find: '@tauri-apps/api/event', replacement: p('./src/web/shims/tauri-event.ts') },
      { find: '@tauri-apps/plugin-opener', replacement: p('./src/web/shims/tauri-opener.ts') },
      { find: '@tauri-apps/plugin-dialog', replacement: p('./src/web/shims/tauri-dialog.ts') },
    ],
  },
  server: { port: 1431, strictPort: true, fs: { allow: ['../..'] } },
  envDir: p('./'),
  envPrefix: ['VITE_'],
  build: { outDir: p('./dist-mobile'), emptyOutDir: true, target: 'es2022', sourcemap: false, chunkSizeWarningLimit: 2500 },
});
