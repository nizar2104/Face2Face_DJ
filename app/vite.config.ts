import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { devBridge } from '../tools/mock-sender/dev-bridge.ts';

// Port 1420 is what src-tauri/tauri.conf.json points `devUrl` at.
export default defineConfig({
  // devBridge: in a plain browser, stands in for the Rust side (dev server only).
  plugins: [react(), devBridge()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // A cloud workspace or a forwarded port reaches the dev server under its own
    // host name, which Vite refuses unless told otherwise. Off by default.
    allowedHosts: process.env['FACE2FACE_DEV_ANY_HOST'] === '1' ? true : undefined,
  },
  build: { outDir: 'dist', target: 'es2022' },
});
