import { defineConfig } from '@playwright/test';
import base from './playwright.config.js';

// WebKit must not inherit a local/system Chromium executable override.
export default defineConfig({
  ...base,
  use: {...base.use, browserName:'webkit', launchOptions:{}},
});
