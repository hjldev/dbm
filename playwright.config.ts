import { defineConfig } from '@playwright/test'
export default defineConfig({
  testDir: './tests/ui', testMatch: '**/*.ui.ts', fullyParallel: true, workers: 2,
  use: { baseURL: 'http://127.0.0.1:1421', viewport: { width: 1320, height: 860 }, channel: 'chrome', screenshot: 'only-on-failure', trace: 'retain-on-failure' },
  webServer: { command: 'node_modules/.bin/vite --host 127.0.0.1 --port 1421', url: 'http://127.0.0.1:1421', reuseExistingServer: !process.env.CI },
})
