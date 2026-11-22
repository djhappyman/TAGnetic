import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/ui',
  fullyParallel: true,
  use: {
    browserName: 'webkit',
    headless: true,
    viewport: { width: 1280, height: 800 },
  },
});
