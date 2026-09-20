/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

// The page. In development /api is proxied to a running Bagholder server
// (BAGHOLDER_PORT), so the page renders real data with hot reload; the built bundle
// is served by the Rust server at /, its asset paths relative.
export default defineConfig(({ mode }) => ({
  base: './',
  plugins: [svelte()],
  // Vitest must resolve Svelte's browser build, not its server build, so
  // component mount() works under jsdom. Vitest reads top-level resolve.conditions.
  // Only override in test mode — clobbering conditions in dev/build drops Vite's
  // own 'browser' default and makes Svelte resolve its server build.
  ...(mode === 'test' ? { resolve: { conditions: ['browser'] } } : {}),
  server: {
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:' + (process.env.BAGHOLDER_PORT || '8788'),
        changeOrigin: true,
      },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
  },
}))
