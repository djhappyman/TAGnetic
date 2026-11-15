import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve(process.env.TAGREX_UI_TEST_DIST || '.test-dist');
test.beforeEach(async ({ page }) => {
  page.errors = [];
  page.on('pageerror', error => page.errors.push(error.message));
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch { await route.fulfill({ status: 404 }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async () => {
    document.getElementById('root').value = '/music';
    await window.tagrex.openLibrary();
  });
});
test.afterEach(async ({ page }) => expect(page.errors).toEqual([]));

const settle = page => page.evaluate(async () => {
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  await Promise.all(document.getAnimations().map(animation => animation.finished.catch(() => {})));
  await new Promise(resolve => requestAnimationFrame(resolve));
});

test('permanent filter and preset interactions do not rewrite inherited file-column styles', async ({ page }) => {
  await settle(page);
  await page.evaluate(() => {
    window.fileColumnStyleChanges = 0;
    const observer = new MutationObserver(records => { window.fileColumnStyleChanges += records.length; });
    observer.observe(document.querySelector('.files-col'), { attributes: true, attributeFilter: ['style'] });
  });
  for (let iteration = 0; iteration < 2; iteration++) {
    await page.keyboard.press('Control+f');
    await expect(page.locator('#filter')).toBeFocused();
    await page.locator('#presets-btn').click();
    await expect(page.locator('#presets-menu')).toBeVisible();
    await page.keyboard.press('Escape');
    await settle(page);
  }
  expect(await page.evaluate(() => window.fileColumnStyleChanges)).toBe(0);
  await expect(page.locator('#filter')).toBeVisible();
  await expect(page.locator('#filter-panel-toggle')).toHaveCount(0);
});

for (const theme of ['light', 'dark']) test(`${theme}: status text updates preserve player allocation until geometry changes`, async ({ page }) => {
  await page.evaluate(theme => {
    document.documentElement.dataset.theme = theme;
    document.getElementById('diff-actionbar').hidden = false;
  }, theme);
  await settle(page);
  const before = await page.locator('#player').boundingBox();
  const changes = await page.evaluate(async () => {
    const footer = document.querySelector('.statusbar');
    let mutations = 0;
    const observer = new MutationObserver(records => { mutations += records.length; });
    observer.observe(footer, { attributes: true, attributeFilter: ['style', 'class'] });
    for (let i = 0; i < 10; i++) {
      document.getElementById('ab-state').textContent = `Status ${i}`;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }
    observer.disconnect();
    return mutations;
  });
  expect(changes).toBe(0);
  await page.locator('#panel-toggle').click();
  await settle(page);
  const after = await page.locator('#player').boundingBox();
  expect(after.width).toBeGreaterThan(before.width);
  expect(after.x).toBeCloseTo(before.x, 0);
});
