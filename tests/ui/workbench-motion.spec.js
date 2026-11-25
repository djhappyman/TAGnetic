import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
const root = resolve('.test-dist');
test.beforeEach(async ({ page }) => {
  page.motionErrors = [];
  page.on('pageerror', error => page.motionErrors.push(error.message));
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try { await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' }); }
    catch { await route.fulfill({ status: 404 }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
});
test.afterEach(async ({ page }) => expect(page.motionErrors).toEqual([]));

test('toolbar hints warm only after a hint appears and Escape restores the first delay', async ({ page }) => {
  await page.clock.install();
  const first = page.locator('#ribbon-open-folder'), next = page.locator('#ribbon-add-folder');
  await first.hover();
  await page.clock.runFor(400);
  await expect(page.locator('#controller-tooltip-content')).toBeHidden();
  await page.clock.runFor(80);
  await expect(page.locator('#controller-tooltip-content')).toBeVisible();
  await next.hover();
  await page.clock.runFor(20);
  await expect(page.locator('#controller-tooltip-content')).toHaveText(await next.getAttribute('aria-description'));
  await page.keyboard.press('Escape');
  await expect(page.locator('#controller-tooltip-content')).toBeHidden();
  await first.hover();
  await page.clock.runFor(100);
  await expect(page.locator('#controller-tooltip-content')).toBeHidden();
  await page.clock.runFor(400);
  await expect(page.locator('#controller-tooltip-content')).toBeVisible();
});

test('hints outside the ribbon use the same delay and follow the active theme', async ({ page }) => {
  const time = new Date('2026-01-01T00:00:00Z');
  await page.clock.install({ time });
  await page.clock.pauseAt(new Date(time.getTime() + 1000));
  await page.mouse.move(0, 0);
  await page.keyboard.press('Escape');
  const anchor = page.locator('#settings-open');
  const tip = page.locator('#controller-tooltip-content');
  await anchor.hover();
  await page.clock.runFor(400);
  await expect(tip).toBeHidden();
  await page.clock.runFor(80);
  await expect(tip).toHaveText('Settings');

  for (const theme of ['light', 'dark']) {
    await page.evaluate(theme => {
      document.documentElement.dataset.palette = 'navy';
      document.documentElement.dataset.theme = theme;
    }, theme);
    expect(await tip.evaluate(node => {
      const sample = document.createElement('span');
      sample.style.backgroundColor = 'var(--panel)';
      sample.style.color = 'var(--text)';
      document.body.append(sample);
      const expected = getComputedStyle(sample);
      const rendered = getComputedStyle(node);
      const matches = rendered.backgroundColor === expected.backgroundColor && rendered.color === expected.color;
      sample.remove();
      return matches;
    })).toBe(true);
  }
  await page.mouse.move(0, 0);
  await page.clock.runFor(200);
  await expect(tip).toBeHidden();
});

test('file tooltips show just the filename when the file column clips it', async ({ page }) => {
  await page.evaluate(async () => {
    document.getElementById('root').value = '/music';
    await window.tagrex.openLibrary();
  });
  const time = new Date('2026-01-01T00:00:00Z');
  await page.clock.install({ time });
  await page.clock.pauseAt(new Date(time.getTime() + 1000));
  const column = page.locator('#tracks th[data-sort="file"]');
  const file = page.locator('#tracks-body tr[data-path] td.file').first();
  const visibleName = (await file.textContent()).trim();
  const tip = page.locator('#controller-tooltip-content');
  const isClipped = () => file.evaluate(cell => cell.scrollWidth > cell.clientWidth);

  await column.evaluate(header => { header.style.width = '420px'; });
  await expect.poll(isClipped).toBe(false);
  await file.hover();
  await page.clock.runFor(600);
  await expect(tip).toBeHidden();

  await page.mouse.move(0, 0);
  await column.evaluate(header => { header.style.width = '84px'; });
  await expect.poll(isClipped).toBe(true);
  await file.hover();
  await page.clock.runFor(400);
  await expect(tip).toBeHidden();
  await page.clock.runFor(80);
  await expect(tip).toBeVisible();
  await expect(tip).toHaveText(visibleName);
  await expect(file).not.toHaveAttribute('aria-description');
});

test('activating and closing a dialog clears hints without showing one on return focus', async ({ page }) => {
  const time = new Date('2026-01-01T00:00:00Z');
  await page.clock.install({ time });
  await page.clock.pauseAt(new Date(time.getTime() + 1000));
  const opener = page.locator('#settings-open');
  const tip = page.locator('#controller-tooltip-content');
  await opener.hover();
  await page.clock.runFor(550);
  await expect(tip).toHaveText('Settings');
  await opener.click();
  await page.clock.runFor(650);
  await expect(page.getByRole('dialog', { name: 'Settings', exact: true })).toBeVisible();
  await expect(tip).toBeHidden();
  await page.keyboard.press('Escape');
  await page.clock.runFor(700);
  await expect(opener).toBeFocused();
  await expect(tip).toBeHidden();
  await page.mouse.move(0, 0);
  await opener.hover();
  await page.clock.runFor(650);
  await expect(tip).toBeVisible();
  await expect(tip).toHaveText('Settings');
});

for (const theme of ['light', 'dark']) test(`navigation settles on the latest tab without shifting the table (${theme})`, async ({ page }) => {
  await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
  const table = await page.locator('#files-view').boundingBox();
  for (const mode of ['renamer', 'exporter', 'online', 'tagger']) await page.locator(`.mode-tab[data-mode="${mode}"]`).click();
  const active = page.locator('.mode-tabs [aria-selected="true"]');
  await expect(active).toHaveAttribute('data-mode', 'tagger');
  const pill = page.locator('.mode-tabs .navigation-motion-pill');
  await expect.poll(() => pill.evaluate(node => node.getAnimations().length)).toBe(0);
  expect(await pill.boundingBox()).toEqual(await active.boundingBox());
  expect(await page.locator('#files-view').boundingBox()).toEqual(table);
  // Wrapping navigation must remeasure without stretching or clipping its fill.
  await page.setViewportSize({ width: 640, height: 800 });
  await expect.poll(async () => JSON.stringify(await pill.boundingBox())).toBe(JSON.stringify(await active.boundingBox()));
  await page.locator('#settings-open').click();
  await expect(page.getByRole('dialog', { name: 'Settings', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.locator('#settings-open')).toBeFocused();
});

test('reduced motion keeps selection and progress legible with no movement', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.locator('.mode-tab[data-mode="exporter"]').click();
  const pill = page.locator('.mode-tabs .navigation-motion-pill');
  expect(await pill.evaluate(node => node.getAnimations().length)).toBe(0);
  expect(await pill.boundingBox()).toEqual(await page.locator('.mode-tabs [aria-selected="true"]').boundingBox());
  expect(await page.locator('#activity-fill').evaluate(node => getComputedStyle(node).transitionDuration)).toBe('0s');
  const button = page.locator('.mode-tab[data-mode="exporter"]');
  await button.focus();
  await expect(button).toBeFocused();
  expect(await button.evaluate(node => getComputedStyle(node).transitionDuration)).toBe('0s');
});


test('main navigation never shows hover or keyboard tooltips', async ({ page }) => {
  await page.clock.install();
  for (const mode of ['tagger', 'online', 'renamer', 'deduplicator', 'exporter']) {
    const tab = page.locator(`.mode-tab[data-mode="${mode}"]`);
    await tab.hover();
    await page.clock.runFor(1200);
    await expect(page.locator('#controller-tooltip-content')).toBeHidden();
    await tab.focus();
    await page.clock.runFor(50);
    await expect(page.locator('#controller-tooltip-content')).toBeHidden();
    await expect(tab).not.toHaveAttribute('title');
  }
});
