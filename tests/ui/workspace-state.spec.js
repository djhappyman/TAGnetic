import { toggleDuplicates } from './navigation.js';
import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
const errors = new WeakMap();
test.beforeEach(async ({ page }) => {
  const observed = [];
  errors.set(page, observed);
  page.on('pageerror', error => observed.push(error.message));
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.setMode)).toBe('function');
});
test.afterEach(async ({ page }) => { expect(errors.get(page)).toEqual([]); });

function mode(page, value) { return page.locator(`.mode-tab[data-mode="${value}"]`); }
function subtab(page, value) { return page.locator(`.subtab[data-subtab="${value}"]`); }

test('controller changes update tab state without synthesizing clicks and preserve draft nodes', async ({ page }) => {
  await page.evaluate(async () => {
    document.getElementById('root').value = '/music';
    await window.tagrex.openLibrary();
    window.tagrex.setSubtab('editor');
  });
  const title = page.locator('#fields-body input[data-key="title"]');
  await title.fill('Keep this pending title');
  await page.evaluate(() => {
    window.tabClicks = 0;
    window.pendingTitleNode = document.querySelector('#fields-body input[data-key="title"]');
    document.addEventListener('click', event => { if (event.target.closest('.mode-tab, .subtab')) window.tabClicks++; });
    window.tagrex.setMode('renamer');
  });
  await expect(mode(page, 'renamer')).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#panel-renamer')).toBeVisible();
  await page.evaluate(() => { window.tagrex.setMode('tagger'); window.tagrex.setSubtab('filename'); });
  await expect(subtab(page, 'filename')).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#subtab-filename')).toBeVisible();
  await page.evaluate(() => window.tagrex.setSubtab('editor'));
  await expect(title).toHaveValue('Keep this pending title');
  expect(await page.evaluate(() => window.pendingTitleNode === document.querySelector('#fields-body input[data-key="title"]'))).toBe(true);
  expect(await page.evaluate(() => window.tabClicks)).toBe(0);
});

test('tool arrows browse before activation and source arrows keep mounted panels in sync', async ({ page }) => {
  const online = mode(page, 'online');
  const tags = mode(page, 'tagger');
  const rename = mode(page, 'renamer');
  const duplicates = mode(page, 'deduplicator');
  await expect(online).toHaveAttribute('aria-selected', 'true');
  const filter = page.locator('#filter');
  await filter.fill('current library search');
  await online.focus();
  await page.keyboard.press('ArrowRight');
  await expect(rename).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(duplicates).toBeFocused();
  await expect(duplicates).toHaveAttribute('aria-pressed', 'false');
  await expect(online).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#subtab-online')).toBeVisible();
  await expect(filter).toHaveValue('current library search');

  await page.keyboard.press('ArrowLeft');
  await expect(rename).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(rename).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#panel-renamer')).toBeVisible();
  await page.keyboard.press('Home');
  await expect(tags).toBeFocused();
  await page.keyboard.press('Space');
  await expect(tags).toHaveAttribute('aria-selected', 'true');
  await expect(tags).toHaveAttribute('aria-controls', 'panel-tagger');

  const editor = subtab(page, 'editor');
  const fromName = subtab(page, 'filename');
  const editorPanel = page.locator('#subtab-editor');
  const initialEditorPanel = await editorPanel.elementHandle();
  await editor.focus();
  await page.keyboard.press('ArrowRight');
  await expect(fromName).toBeFocused();
  await expect(fromName).toHaveAttribute('aria-selected', 'true');
  await expect(fromName).toHaveAttribute('aria-controls', 'subtab-filename');
  await expect(page.getByRole('tabpanel', { name: 'From name', exact: true })).toBeVisible();
  await expect(editorPanel).toBeHidden();
  await page.keyboard.press('Home');
  await expect(editor).toBeFocused();
  await expect(editorPanel).toBeVisible();
  expect(await editorPanel.evaluate((panel, initialPanel) => panel === initialPanel, initialEditorPanel)).toBe(true);

  await page.evaluate(() => { window.tagrex.setMode('renamer'); window.tagrex.setMode('tagger'); });
  await expect(editor).toHaveAttribute('aria-selected', 'true');
  await expect(editorPanel).toBeVisible();
});

test('generator jobs support roving focus and numbering opens its shared wizard', async ({ page }) => {
  await page.evaluate(() => window.tagrex.setMode('generator'));
  const transform = page.locator('#gen-job-transform');
  const number = page.locator('#gen-job-number');
  const sides = page.locator('#gen-job-sides');
  await transform.focus();
  await page.keyboard.press('ArrowRight');
  await expect(number).toBeFocused();
  await expect(number).toHaveAttribute('aria-selected', 'true');
  await expect(transform).toHaveAttribute('tabindex', '-1');
  const panel = page.locator(`#${await number.getAttribute('aria-controls')}`);
  await expect(panel).toHaveAttribute('role', 'tabpanel');
  await expect(panel).toBeVisible();
  await panel.getByRole('button',{name:'Open Autonumber',exact:true}).click();
  await expect(page.locator('#ribbon-number-dialog')).toBeVisible();
  await page.locator('#autonum-start').fill('17');
  await page.locator('#ribbon-number-dialog').getByRole('button',{name:'Cancel',exact:true}).click();
  await number.focus();
  await page.keyboard.press('End');
  await expect(sides).toBeFocused();
  await expect(sides).toHaveAttribute('aria-selected', 'true');
  await expect(panel).toBeHidden();
  await page.keyboard.press('Home');
  await expect(transform).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(number).toBeFocused();
  await expect(panel.getByRole('button',{name:'Open Autonumber',exact:true})).toBeVisible();
});


test('numbering and formatting keep their draft inputs through keyboard tab changes', async ({ page }) => {
  await page.evaluate(async () => {
    document.getElementById('root').value = '/music';
    await window.tagrex.openLibrary();
  });
  await page.locator('#ribbon-autonumber').click();
  const numbering = page.getByRole('dialog', { name: /Numbering/ });
  const tracks = numbering.getByRole('tab', { name: 'Autonumber' });
  const disc = numbering.getByRole('tab', { name: 'Disc' });
  const firstTrack = numbering.locator('#autonum-start');
  await firstTrack.fill('17');
  await tracks.focus();
  await page.keyboard.press('ArrowRight');
  await expect(disc).toBeFocused();
  await expect(numbering.getByRole('tabpanel', { name: 'Disc' })).toBeVisible();
  await expect(firstTrack).toBeHidden();
  await page.keyboard.press('Home');
  await expect(tracks).toBeFocused();
  await expect(firstTrack).toHaveValue('17');
  await numbering.getByRole('button', { name: 'Cancel', exact: true }).click();

  await page.locator('#ribbon-format-values').click();
  const formatting = page.getByRole('dialog', { name: 'Format fields' });
  const build = formatting.getByRole('tab', { name: 'Build a field' });
  const extract = formatting.getByRole('tab', { name: 'Extract fields' });
  await formatting.locator('#ribbon-format-pattern').fill('A draft value');
  await build.focus();
  await page.keyboard.press('End');
  await expect(extract).toBeFocused();
  await expect(formatting.getByRole('tabpanel', { name: 'Extract fields' })).toBeVisible();
  await page.keyboard.press('Home');
  await expect(build).toBeFocused();
  await expect(formatting.locator('#ribbon-format-pattern')).toHaveText('A draft value');
});

test('Tags and Online are separate segments and preserve a manual draft and selection', async ({page}) => {
  await page.evaluate(async () => { document.getElementById('root').value = '/music'; await window.tagrex.openLibrary(); });
  expect(await page.locator('.mode-tabs .mode-tab').evaluateAll(nodes => nodes.map(n => n.dataset.mode))).toEqual(['tagger','online','renamer','deduplicator','exporter']);
  await mode(page, 'tagger').click();
  await expect(page.locator('#subtab-editor')).toBeVisible();
  await expect(page.locator('.subtab[data-subtab="online"]')).toHaveCount(0);
  const title = page.locator('#fields-body input[data-key="title"]');
  await title.fill('Keep my manual draft');
  const selected = await page.evaluate(() => [...window.tagrex.selection]);
  await mode(page, 'online').click();
  await expect(mode(page, 'online')).toHaveAttribute('aria-selected','true');
  await expect(page.locator('#subtab-online')).toBeVisible();
  await expect(page.locator('.subtabs')).toBeHidden();
  await mode(page, 'tagger').click();
  await expect(title).toHaveValue('Keep my manual draft');
  expect(await page.evaluate(() => [...window.tagrex.selection])).toEqual(selected);
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await toggleDuplicates(page);
  await expect(page.locator('#query-duplicate-scan')).toBeVisible();
});
