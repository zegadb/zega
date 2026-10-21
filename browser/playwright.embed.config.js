import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  testMatch: 'embed.spec.js',
  workers: 1,
  retries: 0,
  timeout: 60_000,
  reporter: 'list',
  use: { baseURL: 'http://127.0.0.1:8790', browserName: 'chromium', headless: true, viewport: { width: 1440, height: 1000 } },
  webServer: { command: 'node scripts/embed-test-server.mjs', url: 'http://127.0.0.1:8790', reuseExistingServer: false, timeout: 10_000 },
});
