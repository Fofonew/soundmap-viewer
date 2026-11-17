import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Tauri attend le serveur de dev sur un port fixe.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: 'es2022', chunkSizeWarningLimit: 2000 },
});
