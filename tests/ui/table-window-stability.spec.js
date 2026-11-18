import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve(process.env.TAGREX_TABLE_DIST || '.test-dist');
const baseline = !!process.env.TAGREX_TABLE_BASELINE;
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('tagrex.groupBy', ''));
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try { await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' }); }
    catch { await route.fulfill({ status: 404 }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
});

async function open(page, size) {
  await page.evaluate(async size => {
    await window.tagrex.invoke('open_drop', { paths: Array.from({ length: size }, (_, i) => `/music/group-${Math.floor(i / 250)}/track-${String(i).padStart(6, '0')}.mp3`) });
    document.getElementById('root').value = '/dropped';
    await window.tagrex.openLibrary();
    await document.fonts.ready;
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  }, size);
}

test('filter row height changes retain loaded album rows and an active raw cell', async ({ page }, info) => {
  await open(page, 53);
  await page.locator('#tracks-body td[data-field="title"]').first().click();
  await page.keyboard.press('F2');
  const result = await page.evaluate(async () => {
    const body = document.getElementById('tracks-body');
    const panel = document.getElementById('filter-panel');
    const padding = panel.style.paddingBlock;
    const rows = [...body.querySelectorAll('tr[data-path]')];
    const cell = body.querySelector('[contenteditable="true"]');
    cell.textContent = 'An uncommitted raw value';
    let created = 0, mutations = 0;
    const create = document.createElement;
    document.createElement = function (tag, ...args) { if (tag === 'tr') created++; return create.call(this, tag, ...args); };
    const observer = new MutationObserver(records => mutations += records.length);
    observer.observe(body, { childList: true, subtree: true, characterData: true });
    const timings = [];
    try {
      for (let iteration = 0; iteration < 4; iteration++) {
        panel.style.paddingBlock = iteration % 2 ? '8px' : '12px';
        for (let frame = 0; frame < 15; frame++) {
          const start = performance.now(); await new Promise(requestAnimationFrame); timings.push(performance.now() - start);
        }
      }
      return { created, mutations, retained: rows.every(row => row.isConnected), editing: cell.isConnected && cell.isContentEditable,
        raw: cell.textContent, frames: timings };
    } finally { observer.disconnect(); document.createElement = create; panel.style.paddingBlock = padding; }
  });
  await info.attach('filter-resize.json', { body: JSON.stringify(result, null, 2), contentType: 'application/json' });
  if (!baseline) {
    expect(result.created).toBe(0); expect(result.mutations).toBe(0);
    expect(result.retained).toBe(true); expect(result.editing).toBe(true);
    expect(result.raw).toBe('An uncommitted raw value');
  }
});

test('viewport height and width changes preserve unchanged album cells and selection', async ({ page }) => {
  await open(page, 53);
  await page.locator('#tracks-body td.file').nth(2).click();
  await page.evaluate(() => { window.tableOriginalRows = [...document.querySelectorAll('#tracks-body tr[data-path]')]; });
  for (const viewport of [{ width: 1280, height: 600 }, { width: 1600, height: 900 }, { width: 1200, height: 700 }]) {
    await page.setViewportSize(viewport);
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(resolve)))));
  }
  if (!baseline) expect(await page.evaluate(() => window.tableOriginalRows.every(row => row.isConnected))).toBe(true);
  await expect(page.locator('#tracks-body tr.selected')).toHaveCount(1);
  await expect(page.locator('#tracks-body td.sheet-active')).toHaveCount(1);
});

for (const size of [20000, 50000, 100000]) test(`table-scale ${size} bounds scroll work after select-all`, async ({ page }, info) => {
  test.skip(!process.env.TAGREX_TABLE_SCALE, 'Opt-in serial scale measurements');
  test.setTimeout(120000);
  await open(page, size);
  await page.locator('#tracks-body td.file').first().click();
  await page.keyboard.press('Control+a');
  const result = await page.evaluate(async () => {
    const wrap = document.getElementById('files-view'), body = document.getElementById('tracks-body');
    let maxRows = 0, pathIterations = 0;
    const selection = window.tagrex.selection, iterate = selection[Symbol.iterator];
    selection[Symbol.iterator] = function* () { for (const path of iterate.call(this)) { pathIterations++; yield path; } };
    const samples = [];
    try {
      for (let index = 0; index < 30; index++) {
        const start = performance.now();
        wrap.scrollTop = index * (wrap.scrollHeight - wrap.clientHeight) / 29;
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        samples.push(performance.now() - start);
        maxRows = Math.max(maxRows, body.querySelectorAll('tr[data-path]').length);
      }
      return { samples, maxRows, pathIterations, selected: selection.size, finalMounted: !!body.querySelector(`tr[data-path="${window.tagrex.tracks.at(-1).path}"]`) };
    } finally { delete selection[Symbol.iterator]; }
  });
  await info.attach(`scroll-${size}.json`, { body: JSON.stringify(result, null, 2), contentType: 'application/json' });
  expect(result.maxRows).toBeLessThan(120); expect(result.selected).toBe(size); expect(result.finalMounted).toBe(true);
  expect(result.pathIterations).toBe(0);
});

for (const size of [371, 1200]) test(`bottom scrolling settles without bouncing or clipping the final row (${size} tracks)`, async ({page}) => {
  await open(page,size);
  const result=await page.locator('#files-view').evaluate(async wrap=>{
    const frames=async()=>{for(let i=0;i<5;i++)await new Promise(requestAnimationFrame);};
    // Leave the top, then request the bottom exactly as End/thumb dragging does.
    wrap.scrollTop=wrap.scrollHeight;await frames();
    const top=wrap.scrollTop;
    wrap.scrollTop=wrap.scrollHeight;const requested=wrap.scrollTop;await frames();
    const last=[...wrap.querySelectorAll('tr[data-path]')].at(-1), bounds=wrap.getBoundingClientRect();
    return {bounce:Math.abs(requested-wrap.scrollTop),top,finalTop:wrap.scrollTop,gap:bounds.bottom-last.getBoundingClientRect().bottom,lastPath:last.dataset.path,expected:window.tagrex.tracks.at(-1).path};
  });
  expect(result.bounce).toBeLessThanOrEqual(1);
  expect(result.lastPath).toBe(result.expected);
  expect(result.gap).toBeGreaterThanOrEqual(8);
});
