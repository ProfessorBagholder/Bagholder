import { defineConfig } from '@playwright/test'
export default defineConfig({ testDir: '.', testMatch: /.*\.spec\.mjs/, workers: 1, timeout: 900_000, reporter: [['line']], outputDir: '/private/tmp/claude-501/-Users-md-dev-Bagholder/4156ee24-aa1e-453b-9468-2ae062980e70/scratchpad/parity/results' })
