import { openTagSource } from './navigation.js';
import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
test.beforeEach(async ({ page }) => {
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try { await route.fulfill({ body: await readFile(path), contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html' }); }
    catch { await route.fulfill({ status:404, body:'Missing fixture' }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.setMode)).toBe('function');
});
async function openLibrary(page) {
  await page.locator('#root-display').click();
  await page.locator('#root').fill('/music');
  await page.locator('#root').press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
}

test('table-owned folder controls retain opening and refreshing while commands open the real preset editor', async ({ page }) => {
  await openLibrary(page);
  const header = page.locator('.files-col .library-header');
  await expect(header.locator('#lib-action')).toBeVisible();
  await expect(header.locator('#lib-refresh')).toBeVisible();
  const location = await header.locator('#root-field').boundingBox();
  const bounds = await header.boundingBox();
  expect(Math.abs(location.width - bounds.width)).toBeLessThan(2);
  await page.locator('#lib-refresh').click();
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
  await openTagSource(page,'transformations');await page.locator('#presets-edit').click();
  await expect(page.locator('#pe-editor')).toBeVisible();
  await expect(page.locator('.pe-list-row')).not.toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(page.locator('#pe-editor')).toBeHidden();
});

test('fixed workspace keeps drafts and exposes columns from the table header', async ({ page }) => {
  await openLibrary(page);
  await openTagSource(page, 'editor');
  const title = page.locator('#fields-body input[data-key="title"]');
  await title.fill('Draft survives header actions');
  await expect(page.locator('#view-btn, #health-btn, #arrange-layout, #arrange-done')).toHaveCount(0);
  await page.locator('#tracks th.sortable').first().click({button:'right'});
  await expect(page.locator('#columns-menu')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(title).toHaveValue('Draft survives header actions');
  await page.locator('.subtab').first().click({button:'right'});
  await expect(page.locator('.dock-layout-menu')).toHaveCount(0);
});

test('quiet row fills keep selection distinct and contain no row divider borders', async ({ page }) => {
  await openLibrary(page);
  await page.locator('#tracks-body tr[data-path]').first().click();
  const rowState = await page.locator('#tracks-body tr[data-path]').evaluateAll(rows => rows.map(row => {
    const cell = row.querySelector('td:not(.sel):not(.artwork-col)');
    const style = getComputedStyle(cell);
    return { selected:row.classList.contains('selected'), fill:style.backgroundColor, top:style.borderTopWidth, bottom:style.borderBottomWidth };
  }));
  expect(rowState).toHaveLength(3);
  expect(rowState.every(row => row.top === '0px' && row.bottom === '0px')).toBe(true);
  const plain = rowState.filter(row => !row.selected);
  expect(plain).toHaveLength(2);
  expect(plain[0].fill).not.toBe(plain[1].fill);
  expect(rowState.find(row => row.selected).fill).not.toBe(plain[0].fill);
  const headers = await page.locator('#tracks thead th:visible').evaluateAll(cells => cells.map(cell => getComputedStyle(cell).backgroundColor));
  expect(new Set(headers).size).toBe(1);
  expect(await page.locator('#tracks thead tr').evaluate(row => getComputedStyle(row).borderBottomWidth)).toBe('0px');
  const footer = await page.locator('#filter-panel').evaluate(panel => {
    const style = getComputedStyle(panel);
    const pane = getComputedStyle(document.querySelector('.files-col'));
    const table = getComputedStyle(document.getElementById('files-view'));
    return { fill:style.backgroundColor, pane:pane.backgroundColor, table:table.backgroundColor, border:style.borderTopWidth };
  });
  expect(footer.fill).toBe('rgba(0, 0, 0, 0)');
  expect(footer.pane).toBe(footer.table);
  expect(footer.border).toBe('0px');
  await expect(page.locator('#filter-panel-toggle, .filter-panel-head')).toHaveCount(0);
  await expect(page.locator('#filter')).toBeVisible();
  for (const id of ['presets-btn']) {
    const control = page.locator(`#${id}`);
    expect(await control.evaluate(node => getComputedStyle(node).borderTopWidth)).toBe('0px');
    await control.focus();
    expect(await control.evaluate(node => getComputedStyle(node).outlineStyle)).toBe('solid');
  }
  await page.locator('#filter').fill('x'.repeat(2049));
  await expect(page.locator('#filter')).toHaveAttribute('aria-invalid', 'true');
  await page.locator('#presets-btn').focus();
  expect(await page.locator('.filter-ctl').evaluate(node => {
    const style = getComputedStyle(node);
    return (style.borderTopWidth !== '0px' && style.borderTopColor !== 'rgba(0, 0, 0, 0)') || (style.outlineStyle === 'solid' && style.outlineColor !== 'rgba(0, 0, 0, 0)');
  })).toBe(true);
});

test('icons and all tool actions remain available at narrow widths', async ({ page }) => {
  await page.setViewportSize({width:375,height:720});
  for (const tab of await page.locator('.mode-tab').all()) {
    await expect(tab.locator('.ico')).toBeVisible();
    await expect(tab).toBeInViewport();
    await tab.focus();
    await expect(tab).toBeFocused();
  }
  await expect(page.locator('#ribbon-autonumber')).toBeInViewport();
  await expect(page.locator('#ribbon-remove-tags')).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
});

test('permanent playback follows the table width and stays reachable in the compact inspector', async ({ page }) => {
  await page.setViewportSize({ width:1397, height:900 });
  const alignment = () => page.evaluate(() => {
    const pane = document.querySelector('.files-col').getBoundingClientRect();
    const player = document.getElementById('player').getBoundingClientRect();
    return Math.abs(pane.left + 3 - player.left) + Math.abs(pane.right - 3 - player.right);
  });
  await expect.poll(alignment).toBeLessThan(2);
  await expect(page.getByRole('separator', {name:'Resize workspace columns',exact:true})).toHaveCount(0);
  await page.setViewportSize({width:1240,height:900});
  await expect.poll(alignment).toBeLessThan(2);
  expect(await page.locator('#pl-toggle svg').evaluate(node => parseFloat(getComputedStyle(node).width))).toBeGreaterThanOrEqual(16);
  await page.setViewportSize({ width:375, height:720 });
  await page.locator('.mode-tab[data-mode="tagger"]').click();
  await expect(page.locator('#mode-col')).toBeVisible();
  await expect(page.locator('#player')).toBeVisible();
  await expect(page.locator('#pl-volume')).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
});

test('Apply footer keeps a stable table with Preview beside the scoped primary action', async ({ page }) => {
  await page.setViewportSize({width:1397,height:900});
  await openLibrary(page);
  await openTagSource(page, 'editor');
  const before = await page.locator('.statusbar').boundingBox();
  const title = page.locator('#fields-body input[data-key="title"]');
  await title.fill('Footer preview');
  const apply = page.locator('#diff-apply');
  await expect(apply).toBeEnabled();
  await expect(apply).toHaveAccessibleName('Apply to 1 file');
  await expect(page.locator('.statusbar .sb-right #diff-actionbar')).toBeVisible();
  const after = await page.locator('.statusbar').boundingBox();
  expect(Math.abs(before.height - after.height)).toBeLessThan(1);
  await page.evaluate(() => {
    const note = document.getElementById('ab-locked');
    note.textContent = 'Artist kept unchanged because it is locked';
    note.hidden = false;
  });
  await expect(page.locator('#ab-locked')).toBeVisible();
  expect(Math.abs((await page.locator('.statusbar').boundingBox()).height - after.height)).toBeLessThan(1);
  const style = await page.locator('#diff-actionbar').evaluate(node => ({position:getComputedStyle(node).position,background:getComputedStyle(node).backgroundColor,shadow:getComputedStyle(node).boxShadow}));
  expect(style).toEqual({position:'static',background:'rgba(0, 0, 0, 0)',shadow:'none'});
  const sidebar = await page.locator('#mode-col').boundingBox();
  const button = await apply.boundingBox();
  expect(button.x).toBeGreaterThan(sidebar.x);
  expect(button.x + button.width).toBeLessThanOrEqual(sidebar.x + sidebar.width + 1);
  await page.locator('#diff-preview').click();
  await expect(page.locator('#tracks')).toHaveClass(/show-old/);
  expect(await page.evaluate(() => window.tagrex.tracks[0].tags.title)).toBe('Desert Rain');
  const actionNode = await page.locator('#diff-actionbar').evaluate(node => { window.footerActionNode = node; return node.id; });
  expect(actionNode).toBe('diff-actionbar');
  await page.setViewportSize({width:375,height:667});
  for (const id of ['diff-apply','diff-preview']) await expect(page.locator(`#${id}`)).toBeInViewport({ratio:1});
  expect(await page.evaluate(() => window.footerActionNode === document.getElementById('diff-actionbar'))).toBe(true);
  await page.setViewportSize({width:1280,height:800});
  await page.locator('#diff-discard').click();
  await expect(page.locator('#diff-actionbar')).toBeVisible();await expect(page.locator('#diff-apply')).toBeDisabled();
  await expect(page.locator('#ab-locked')).toBeHidden();
  expect(await page.evaluate(() => window.tagrex.tracks[0].tags.title)).toBe('Desert Rain');
});
