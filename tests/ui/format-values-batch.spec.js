import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
test.beforeEach(async ({ page }) => {
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async () => {
    document.getElementById('root').value = '/music';
    await window.tagrex.openLibrary();
  });
});

test('Format Values announces the complete staged batch once and preserves selection and stored tags', async ({ page }) => {
  await page.locator('#ribbon-select-all').click();
  const before = await page.evaluate(() => ({
    selected: window.tagrex.selectedPaths(),
    tracks: structuredClone(window.tagrex.tracks),
  }));
  await page.locator('#ribbon-format-values').focus();
  await page.keyboard.press('Enter');
  await page.locator('#ribbon-format-field').selectOption('albumartist');
  await page.locator('#ribbon-format-pattern').fill('%title%');
  await page.evaluate(() => {
    window.__formatBatchEvents = [];
    document.addEventListener('tagrex:live-input', () => {
      window.__formatBatchEvents.push(structuredClone(window.tagrex.previewPlan));
    });
  });
  await expect(page.locator('#ribbon-format-stage')).toBeEnabled();
  await page.locator('#ribbon-format-stage').focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#ribbon-format-dialog')).toBeHidden();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const after = await page.evaluate(() => ({
    selected: window.tagrex.selectedPaths(),
    tracks: window.tagrex.tracks,
    plan: window.tagrex.previewPlan,
    events: window.__formatBatchEvents,
  }));
  const expected = before.selected.map(path => ({
    path,
    value: before.tracks.find(track => track.path === path).tags.title,
  }));
  const staged = plan => plan.changes.map(change => ({
    path: change.path,
    value: change.tag_changes.find(field => field.field === 'albumartist')?.new,
  }));
  expect(after.events).toHaveLength(1);
  expect(staged(after.events[0])).toEqual(expected);
  expect(staged(after.plan)).toEqual(expected);
  expect(after.selected).toEqual(before.selected);
  expect(after.tracks).toEqual(before.tracks);
});
