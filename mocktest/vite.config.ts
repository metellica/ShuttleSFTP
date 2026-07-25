import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// Vite config for the mock UI harness (no Tauri backend).
export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: [
      {
        find: '@/composables/useTauri',
        replacement: fileURLToPath(new URL('./useTauri.mock.ts', import.meta.url)),
      },
      {
        find: /^@tauri-apps\/.*/,
        replacement: fileURLToPath(new URL('./tauri-mocks.ts', import.meta.url)),
      },
      { find: '@', replacement: fileURLToPath(new URL('../src', import.meta.url)) },
    ],
  },
  server: { port: 5199, strictPort: true },
})
