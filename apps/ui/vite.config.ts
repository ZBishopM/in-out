import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  // Tauri expects a fixed port and its own console.
  clearScreen: false,
  server: { port: 5173, strictPort: true }
});
