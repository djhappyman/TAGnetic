import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
const errors = new WeakMap();
test.beforeEach(async ({ page }) => {
  const issues = [];
  errors.set(page, issues);
  page.on('pageerror', error => issues.push(error.message));
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
});
test.afterEach(async ({ page }) => expect(errors.get(page)).toEqual([]));

async function library(page) {
  await page.locator('#root-display').click();
  await page.locator('#root').fill('/music');
  await page.locator('#root').press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
}

test('filter stays open in the header without a duplicate heading, summary or clear action', async ({ page }) => {
  await library(page);
  await expect(page.locator('#filter')).toBeVisible();
  await expect(page.locator('#filter-panel-toggle, #filter-panel-summary, #filter-panel-clear, .filter-panel-active')).toHaveCount(0);
  expect(await page.locator('#filter-panel-content').evaluate(node => ({ inert: node.inert, transition: getComputedStyle(node).transitionDuration }))).toEqual({ inert: false, transition: '0s' });
  expect(await page.evaluate(() => document.getElementById('filter-panel').getBoundingClientRect().bottom <= document.getElementById('files-view').getBoundingClientRect().top)).toBe(true);
  await expect(page.locator('.view-tabs #filter')).toHaveCount(0);
  await expect(page.locator('#filter-panel #presets-btn')).toHaveCount(1);
  await page.locator('#filter').fill('Wish Mountain');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(1);
  await page.locator('#filter-clear').click();
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
  await expect(page.locator('#filter')).toBeVisible();
});

test('filter shortcut focuses the permanent field and selects its existing query', async ({ page }) => {
  await library(page);
  await page.keyboard.press('Control+f');
  await expect(page.locator('#filter')).toBeFocused();
  await page.locator('#filter').fill('Radio');
  await page.locator('#presets-btn').focus();
  await page.keyboard.press('Control+f');
  await expect(page.locator('#filter')).toBeFocused();
  expect(await page.locator('#filter').evaluate(input => [input.selectionStart, input.selectionEnd])).toEqual([0, 5]);
});

test('obsolete closed preference cannot hide the permanent field; removed flags stay absent', async ({ page }) => {
  await page.evaluate(() => {
    localStorage.setItem('tagrex.filterPanelOpen', '0');
  });
  await page.reload();
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
  await expect(page.locator('#filter')).toBeVisible();
  await expect(page.locator('#filter-panel-toggle, #filter-regex, #filter-case')).toHaveCount(0);
  await expect(page.locator('#filter')).toHaveCount(1);
  await expect(page.locator('#presets-menu')).toHaveCount(1);
});

test('cover status aliases distinguish embedded, missing and unknown scan metadata', async ({ page }) => {
  await library(page);
  await page.evaluate(() => {
    const tracks = window.tagrex.tracks;
    tracks[0].has_cover = true;
    tracks[1].has_cover = false;
    delete tracks[2].has_cover;
    window.tagrex.renderTracks();
  });
  const rows = page.locator('#tracks-body tr[data-path]');
  for (const [query, title] of [
    ['cover:embedded', 'Desert Rain'], ['cover:missing', 'Radio'],
    ['cover:unknown', 'Feel It'], ['COVER:MISSING', 'Radio'],
    ['cover:no cover', 'Radio'], ['cover:No embedded artwork', 'Radio'],
  ]) {
    await page.locator('#filter').fill(query);
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText(title);
  }
  // A non-boolean or unreadable value must never be reported as missing art.
  for (const metadata of [{ has_cover: 'false' }, { has_cover: false, unreadable: true }]) {
    await page.evaluate(metadata => {
      Object.assign(window.tagrex.tracks[2], metadata);
      window.tagrex.renderTracks();
    }, metadata);
    await page.locator('#filter').fill('cover:unknown');
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toHaveAttribute('data-path', /u-hi/);
    await page.locator('#filter').fill('cover:missing');
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText('Radio');
  }
});

test('plain search is case-insensitive and treats regex punctuation literally', async ({ page }) => {
  await library(page);
  const filter = page.locator('#filter');
  const rows = page.locator('#tracks-body tr[data-path]');
  for (const query of ['Radio', 'RADIO', 'radio']) {
    await filter.fill(query);
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText('Radio');
  }
  for (const query of ['[', '^Radio$', '(embedded|missing)']) {
    await filter.fill(query);
    await expect(rows).toHaveCount(0);
    await expect(filter).not.toHaveAttribute('aria-invalid', 'true');
  }
  await expect(page.locator('#filter-regex, #filter-case')).toHaveCount(0);
  await page.locator('#filter-clear').click();
  await expect(rows).toHaveCount(3);
});

test('cover aliases survive language changes and translated labels are recompiled', async ({ page }) => {
  await library(page);
  const filter = page.locator('#filter');
  const rows = page.locator('#tracks-body tr[data-path]');
  const language = async value => page.evaluate(value => {
    const select = document.getElementById('set-lang');
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  }, value);
  await filter.fill('cover:missing');
  await expect(rows).toHaveCount(1);
  await language('fr');
  await expect(rows).toHaveCount(1);
  await filter.fill('cover:Aucune pochette intégrée');
  await expect(rows).toHaveCount(1);
  await language('de');
  await expect(rows).toHaveCount(0);
  await filter.fill('cover:missing');
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText('Radio');
});

test('cover filtering covers 20000 rows while keeping the table virtualized and the latest query current', async ({ page }) => {
  await library(page);
  await page.evaluate(() => {
    const tracks = window.tagrex.tracks;
    const seed = tracks[0];
    tracks.length = 0;
    for (let i = 0; i < 20000; i++) tracks.push({
      ...seed, path: `/music/scale-${String(i).padStart(5, '0')}.mp3`,
      tags: { ...seed.tags, album: 'Large library', title: `Track ${i}` },
      has_cover: i % 2 === 0,
    });
    window.tagrex.selection.clear();
    window.tagrex.renderTracks();
  });
  const filter = page.locator('#filter');
  const rows = page.locator('#tracks-body tr[data-path]');
  await filter.fill('cover:missing');
  await expect(page.locator('#status-sel')).toHaveText(/^10000\//);
  expect(await rows.count()).toBeLessThan(200);
  expect(await rows.locator('[data-cover-state]').evaluateAll(nodes => [...new Set(nodes.map(node => node.dataset.coverState))])).toEqual(['missing']);
  await page.locator('#files-view').evaluate(node => { node.scrollTop = node.scrollHeight; });
  await expect(rows.last()).toHaveAttribute('data-path', '/music/scale-19999.mp3');
  // Start a replacement during the old query's scan, then ensure the old
  // generation cannot overwrite the result after the newer one is published.
  await page.evaluate(() => {
    let replace = true;
    Object.defineProperty(window.tagrex.tracks[0], 'has_cover', { configurable: true, get() {
      if (replace) {
        replace = false;
        const filter = document.getElementById('filter');
        filter.value = 'cover:embedded';
        filter.dispatchEvent(new Event('input', { bubbles: true }));
      }
      return true;
    } });
  });
  await filter.fill('cover:unknown');
  await expect(filter).toHaveValue('cover:embedded');
  await expect(page.locator('#status-sel')).toHaveText(/^10000\//);
  await expect.poll(() => rows.locator('[data-cover-state]').evaluateAll(nodes => [...new Set(nodes.map(node => node.dataset.coverState))])).toEqual(['embedded']);
  expect(await rows.count()).toBeLessThan(200);
  await addCondition(page, 'cover', 'missing');
  await expect(page.locator('#status-sel')).toHaveText(/^0\//);
  await filter.fill('');
  await expect(page.locator('#status-sel')).toHaveText(/^10000\//);
  expect(await rows.count()).toBeLessThan(200);
});

for (const viewport of [{ width: 375, height: 667 }, { width: 844, height: 390 }, { width: 1280, height: 800 }]) {
  test(`filter and upward preset menu remain reachable at ${viewport.width}x${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await library(page);
      await expect(page.locator('#filter')).toBeInViewport({ ratio: 1 });
    const input = await page.locator('#filter').boundingBox();
    expect(input.width).toBeGreaterThanOrEqual(100);
    await page.locator('#presets-btn').click();
    await expect(page.locator('#presets-menu')).toBeInViewport({ ratio: 1 });
    const bounds = await page.locator('#presets-menu').boundingBox();
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.y).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(viewport.width + 1);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(viewport.height + 1);
    await page.keyboard.press('Escape');
    await expect(page.locator('#presets-menu')).toBeHidden();
  });
}

async function clearFilters(page) {
  const remove = page.locator('.filter-condition-chip > button:last-child');
  for (let count = await remove.count(); count > 0; count--) {
    await remove.last().click();
    await expect(remove).toHaveCount(count - 1);
  }
  if (await page.locator('#filter').inputValue()) await page.locator('#filter-clear').click();
}

async function addCondition(page, field, value, operator = 'contains') {
  await page.locator('#filter').press('Alt+Enter');
  await page.locator('#query-editor select[id^=field-]').selectOption(field,{force:true});
  await page.locator('#query-editor select[id^=operator-]').selectOption(field==='cover'?'equals':operator,{force:true});
  if(!['empty','present'].includes(operator)||field==='cover')await page.locator('#query-editor input:not([type=checkbox])').fill(value);
  await page.getByRole('button', { name: 'Save filter', exact: true }).click();
}

test('compact conditions combine with search and edit/remove without losing selection', async ({ page }) => {
  await library(page);
  const selected = await page.evaluate(() => [...window.tagrex.selection]);
  await page.keyboard.press('Control+f');
  await addCondition(page, 'cover', 'missing');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(1);
  await expect(page.locator('#tracks-body')).toContainText('Radio');
  await expect(page.locator('#filter-feedback')).toContainText('1 selected file is hidden');
  await page.locator('#filter').fill('wish');
  await expect(page.locator('#status-sel')).toContainText('1/');
  await page.getByRole('button', { name: 'Edit Artwork: missing', exact: true }).click();
  await page.locator('#query-editor input:not([type=checkbox])').fill('embedded');
  await page.getByRole('button', { name: 'Save filter', exact: true }).click();
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(0);
  await expect(page.locator('#filter-feedback')).toContainText('No files match');
  await expect(page.getByRole('button', { name: 'Edit Artwork: embedded', exact: true })).toBeVisible();
  await clearFilters(page);
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
  expect(await page.evaluate(() => [...window.tagrex.selection])).toEqual(selected);
});

test('all/any conditions and saved presets preserve structured queries', async ({ page }) => {
  await library(page);
  await page.keyboard.press('Control+f');
  await addCondition(page, 'cover', 'missing');
  await addCondition(page, 'title', 'Desert');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(0);
  await page.getByRole('combobox', { name: 'Combine filter conditions' }).click();
  await page.getByRole('option', { name: 'Match any', exact: true }).click();
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(2);
  await page.locator('#presets-btn').click();
  await page.locator('.preset-name').fill('Artwork or Desert');
  await page.locator('.preset-save button').click();
  expect(await page.evaluate(() => [...Object.keys(localStorage)].filter(key => !key.endsWith('.conditions.v2')).some(key => localStorage.getItem(key)?.includes('Artwork or Desert')))).toBe(false);
  await clearFilters(page);
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
  await page.locator('#presets-btn').click();
  await page.getByRole('button', { name: 'Artwork or Desert', exact: true }).click();
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(2);
  await expect(page.locator('.filter-condition-chip')).toHaveCount(2);
  await expect(page.getByRole('combobox', { name: 'Combine filter conditions' })).toContainText('any');
  await page.getByRole('button', { name: 'Remove Artwork: missing', exact: true }).click();
  await expect(page.locator('#filter')).toBeFocused();
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(1);
});

test('keyboard cancellation, narrow windows and themes keep the command popup reachable', async ({ page }) => {
  await library(page);await page.setViewportSize({width:375,height:667});await page.keyboard.press('Control+f');
  for(const theme of ['dark','light']){
    await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;},theme);
    await page.locator('#filter').press('Alt+Enter');
    await expect(page.locator('#query-editor')).toBeInViewport({ratio:1});
    await page.keyboard.press('Escape');await expect(page.locator('#filter')).toBeFocused();await expect(page.locator('.filter-condition-chip')).toHaveCount(0);
  }
});

test('pending and invalid filters cannot publish a staged Apply using stale results', async ({ page }) => {
  await library(page);
  await page.locator('#tracks-body tr[data-path] td[data-field="title"]').first().click();
  await page.evaluate(() => {
    const data = new DataTransfer(); data.setData('text/plain', 'Pending title');
    document.activeElement.dispatchEvent(new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: data }));
  });
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.keyboard.press('Control+f');
  const pendingDisabled = await page.evaluate(() => {
    const input = document.getElementById('filter'); input.value = 'Radio';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    return document.getElementById('diff-apply').disabled;
  });
  expect(pendingDisabled).toBe(true);
  await expect(page.locator('#filter-feedback')).not.toContainText('Updating');
  await page.locator('#filter').fill('x'.repeat(20000));
  await expect(page.locator('#filter')).toHaveAttribute('aria-invalid', 'true');
  await expect(page.locator('#diff-apply')).toBeDisabled();
  await expect(page.locator('#diff-actionbar')).toBeVisible();
  await page.locator('#presets-btn').click();
  await page.locator('.preset-name').fill('Invalid query');
  await page.locator('.preset-save button').click();
  expect(await page.evaluate(() => localStorage.getItem('tagrex.filterPresets.conditions.v2'))).toBeNull();
});
