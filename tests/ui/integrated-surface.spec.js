import { openTagSource } from './navigation.js';
import { test, expect } from '@playwright/test';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
const evidence = resolve('docs/ui-integration');
const pageErrors = new WeakMap();
test.use({ viewport: { width: 1440, height: 900 }, reducedMotion: 'reduce' });

test.beforeEach(async ({ page }) => {
  const errors = []; pageErrors.set(page, errors);
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => { window.integrationCalls = []; });
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      let body = await readFile(path);
      if (path.endsWith('.js')) body = body.toString().replace(/case([`"'])(read_cover_summary|read_external_cover|preview_transform_groups|preview_tag_edits|apply_plan|render_columns|render_column|read_track_rows|list_tracks|get_settings|save_settings)\1:/g,
        (match, _quote, command) => `${match}window.integrationCalls.push('${command}');`);
      await route.fulfill({ body, contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch (error) {
      if (path.endsWith('.js') && !String(error).includes('ENOENT')) throw error;
      await route.fulfill({ status: 404, body: 'Missing fixture' });
    }
  });
});
test.afterEach(async ({ page }) => expect(pageErrors.get(page)).toEqual([]));

async function open(page, theme = 'dark') {
  await page.addInitScript(value => localStorage.setItem('tagrex.theme', value), theme);
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
  await page.locator('#root-display').click();
  await page.locator('#root').fill('/music');
  await page.locator('#root').press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
}
const transforms = page => page.locator('#transformation-pane');

async function editor(page) {
  const compact = page.getByRole('tablist', { name: 'Workspace module' });
  if (await compact.isVisible()) await page.getByRole('tab', { name: 'Inspector', exact: true }).click();
  await openTagSource(page, 'editor');
}
async function screenshot(page, file) {
  await mkdir(evidence, { recursive: true });
  await page.screenshot({ path: resolve(evidence, file), fullPage: true, animations: 'disabled' });
}

// These initial assertions were run against checkpoint719c96c's exact172-test
// fixture build before the implementation build lease was released.
test('Transformations table sits above the editor fields without its own tab', async ({page}) => {
 await open(page);await openTagSource(page,'transformations');
 await expect(page.locator('#dock-module-transformations')).toHaveCount(0);
 await expect(transforms(page)).toBeVisible();
 await expect(page.locator('.manual-inspector-heading')).toHaveCount(0);
 const tab=await page.locator('.subtab[data-subtab="editor"]').boundingBox(),pane=await page.locator('.mode-col').boundingBox();expect(tab.y-pane.y).toBeLessThan(16);
 await expect(page.locator('.mode-col .inspector-help')).toHaveCount(0);
 await expect(page.locator('.transform-heading h3')).toHaveText('Transformations');
 await expect(page.locator('#transform-add')).toBeVisible();
 await expect(page.locator('#presets-edit')).toBeHidden();
});

test('everyday fields stay open while More tags stays an accessible collapsible', async ({ page }) => {
  await open(page);
  await editor(page);
  await expect(page.locator('#fields-body .fe-group[data-group="core"] > button.fe-group-head')).toHaveCount(0);
  await expect(page.locator('#subtab-editor [data-i18n="editor.tagFields"], #subtab-editor [data-i18n="editor.fieldsShort"]')).toHaveCount(0);
  await expect(page.locator('#fields-add-toggle, #fields-add-row, #fields-new-name, #fields-new-value, #fields-add, #fields-add-cancel, #fields-known')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Add tags$/i })).toHaveCount(0);
  for (const kind of ['standard']) {
    const head = page.locator(`.fe-group[data-group="${kind}"] > button.fe-group-head`);
    await expect(head).toHaveAttribute('aria-controls', `editor-group-${kind}`);
    const labelOffset = await head.evaluate((node, name) => {
      const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
      let text; while ((text = walker.nextNode())) if (text.textContent.trim().toLowerCase() === name) break;
      if (!text) return Infinity;
      const range = document.createRange(); range.selectNodeContents(text);
      return range.getBoundingClientRect().left - node.getBoundingClientRect().left;
    }, kind === 'standard' ? 'more tags' : kind);
    expect(labelOffset).toBeLessThan(32);
    if (await head.getAttribute('aria-expanded') === 'false') await head.click();
    await head.focus();
    await page.keyboard.press('Space');
    await expect(head).toHaveAttribute('aria-expanded', 'false');
    await expect(page.locator(`#editor-group-${kind}`)).toHaveAttribute('inert', '');
    await page.keyboard.press('Tab');
    expect(await page.evaluate(id => document.activeElement?.closest(`#editor-group-${id}`) === null, kind)).toBe(true);
    await head.focus();
    await page.keyboard.press('Enter');
    await expect(head).toHaveAttribute('aria-expanded', 'true');
  }
});

test('dark table well stays distinct while sidebar and chrome share a quiet continuous surface', async ({ page }) => {
  await open(page);
  const surfaces = await page.evaluate(() => {
    const rgb = value => { const parts = value.match(/[\d.]+/g)?.map(Number) || []; const scale = value.startsWith('color(srgb ') ? 255 : 1; return { rgb: parts.slice(0, 3).map(channel => channel * scale), alpha: parts[3] ?? 1 }; };
    const surface = selector => {
      let node = document.querySelector(selector), color = [0, 0, 0];
      const ancestors = []; while (node) { ancestors.unshift(node); node = node.parentElement; }
      for (const node of ancestors) { const layer = rgb(getComputedStyle(node).backgroundColor); color = color.map((c, i) => layer.rgb[i] * layer.alpha + c * (1 - layer.alpha)); }
      return color;
    };
    return { table: surface('#table-wrap'), sidebar: surface('#dock-module-inspector'), toolbar: surface('.topbar'), frame: surface('.workarea') };
  });
  const luminance = color => color.reduce((total, value, i) => total + value * [0.2126, 0.7152, 0.0722][i], 0);
  expect(luminance(surfaces.sidebar) - luminance(surfaces.table)).toBeGreaterThan(2);
  expect(Math.abs(luminance(surfaces.sidebar) - luminance(surfaces.toolbar))).toBeLessThan(3);
  expect(Math.abs(luminance(surfaces.sidebar) - luminance(surfaces.frame))).toBeLessThan(3);
  await expect(page.locator('.dock-column-splitter')).toHaveCount(0);
});

test('source tab changes retain the exact checkbox, manual draft and plan', async ({page}) => {
 await open(page);await editor(page);const title=page.locator('#fields-body input[data-key="title"]');await title.fill('KEEP MANUAL DRAFT');await expect(page.locator('#diff-apply')).toBeEnabled();
 await page.evaluate(()=>{window.integrationOriginal={plan:window.tagrex.previewPlan,title:document.querySelector('#fields-body input[data-key="title"]'),checkbox:document.querySelector('#groups-menu input.group-tick')};window.integrationCalls=[];});
 for(let i=0;i<3;i++){await openTagSource(page,'transformations');await openTagSource(page,'editor');}
 await expect(title).toHaveValue('KEEP MANUAL DRAFT');
 expect(await page.evaluate(()=>({title:window.integrationOriginal.title===document.querySelector('#fields-body input[data-key="title"]'),checkbox:window.integrationOriginal.checkbox===document.querySelector('#groups-menu input.group-tick'),writes:window.integrationCalls.filter(c=>c==='apply_plan').length}))).toEqual({title:true,checkbox:true,writes:0});
});



test('editor group folding retains drafts and metadata while preferences follow new selections', async ({ page }) => {
  await open(page);
  await editor(page);
  await page.evaluate(() => {
    const track = window.tagrex.tracks[0];
    track.tags = { ...track.tags, 'custom:QA_RAW_TAG': 'Existing custom metadata' };
    window.tagrex.renderTracks();
  });
  const advanced = page.locator('.fe-group[data-group="standard"] > button.fe-group-head');
  await advanced.click();
  const custom = page.locator('input[data-key="custom:QA_RAW_TAG"]');
  await expect(custom).toHaveValue('Existing custom metadata');
  await custom.fill('Unstaged custom draft');
  await custom.evaluate(node => window.integrationCustom = node);
  await advanced.click();
  await expect(advanced).toHaveAttribute('aria-expanded', 'false');
  await advanced.click();
  await expect(custom).toHaveValue('Unstaged custom draft');
  expect(await custom.evaluate(node => node === window.integrationCustom)).toBe(true);
  const title = page.locator('input[data-key="title"]').first();
  await title.fill('Subject A draft');
  await page.locator('#tracks-body tr[data-path]').nth(1).locator('.file').click();
  await expect(title).toHaveValue('Radio');
  await expect(title).not.toHaveClass(/dirty/);
  expect(await page.evaluate(() => window.tagrex.tracks[0].tags['custom:QA_RAW_TAG'])).toBe('Existing custom metadata');
});



for (const theme of ['dark', 'light']) for (const size of [{ width: 375, height: 667 }, { width: 844, height: 390 }, { width: 1440, height: 900 }]) {
  test(`${theme} ${size.width}×${size.height} integrated surfaces stay within the viewport and expose their controls`, async ({ page }) => {
    await page.setViewportSize(size);
    await open(page, theme);
    const compact = page.getByRole('tablist', { name: 'Workspace module' });
    await openTagSource(page,'transformations');
    await expect(page.locator('.subtab[data-subtab="editor"]')).toBeInViewport({ratio:1});
    await expect(page.locator('.mode-col .inspector-help')).toHaveCount(0);
    await expect(page.locator('#transform-add')).toBeInViewport({ ratio: 1 });
    await editor(page);
    await page.locator('#editor-field-title').scrollIntoViewIfNeeded();
    await expect(page.locator('#editor-field-title')).toBeInViewport({ ratio: 1 });
    await expect(page.locator('#staged-edits')).toBeHidden();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const panel = await page.locator('#dock-module-inspector').evaluate(node => ({ scroll: node.scrollWidth, client: node.clientWidth }));
    expect(panel.scroll).toBeLessThanOrEqual(panel.client);
    if (process.env.INTEGRATION_CAPTURE_EVIDENCE === '1') await screenshot(page, `${theme}-${size.width}x${size.height}.png`);
  });
}
