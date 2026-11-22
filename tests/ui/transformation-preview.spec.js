import { openTagSource } from './navigation.js';
import { waitForScrollIdle } from './scrolling.js';
import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
const issues = new WeakMap();
const groups = [
  { name: 'All fields cleanup', scope: 'tags', rules: [{kind:'case',style:'upper',scope:'tags'}] },
  { name: 'Upper titles', scope: 'title', rules: [{ kind: 'case', style: 'upper', scope: 'title', from: '', to: '', regex: false }] },
  { name: 'Append title mark', scope: 'title', rules: [{ kind: 'replace', scope: 'title', from: '$', to: '!', regex: true, style: '' }] },
  { name: 'Upper filenames', scope: 'filename', rules: [{ kind: 'case', style: 'upper', scope: 'filename', from: '', to: '', regex: false }] },
];

test.beforeEach(async ({ page }) => {
  const errors = []; issues.set(page, errors);
  page.on('pageerror', error => errors.push(error.stack || error.message));
  await page.addInitScript(presets => {
    localStorage.setItem('tagrex.activePresetsByJob', JSON.stringify({ generator: [], online: [], fromname: [], renamer: [] }));
    window.transformCalls = {}; window.transformRequests = []; window.transformUndo = [];
    window.transformLocked = []; window.transformHeld = []; window.transformHold = false;
    const originalResolve = Promise.resolve;
    Promise.resolve = function (value) {
      // The fixture supplies real configurable presets and models native IPC
      // ownership. No native app or real library is ever accessed.
      if (value && typeof value === 'object' && 'proxy' in value && 'portable_names' in value) value = { ...value, action_groups: structuredClone(presets) };
      if (value?.description === 'Transform' && Array.isArray(value.changes)) {
        value = structuredClone(value);
        // Model the native backend's field-lock gate (the browser mock has no
        // transform lock gate). Keep its actual transform computation intact.
        const skipped = new Map();
        value.changes = value.changes.filter(change => {
          change.tag_changes = change.tag_changes.filter(field => {
            if (!window.transformLocked.includes(field.field)) return true;
            skipped.set(field.field, (skipped.get(field.field) || 0) + 1); return false;
          });
          return change.rename_to || change.tag_changes.length;
        });
        value.locked_skipped = [...skipped].map(([field, files]) => ({ field, files }));
        if (window.transformHold) return new Promise(resolve => window.transformHeld.push({ plan: value, resolve }));
      }
      return originalResolve.call(this, value);
    };
    window.releaseTransform = index => window.transformHeld[index].resolve(window.transformHeld[index].plan);
  }, groups);
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      let body = await readFile(path);
      if (path.endsWith('.js')) {
        let source = body.toString();
        const state = source.match(/case([`"'])list_tracks\1:\s*return Promise\.resolve\(([\w$]+)\.tracks\)/)?.[2];
        const transform = source.match(/case([`"'])preview_live_edits\1:/);
        const args = transform && source.slice(transform.index, transform.index + 1200).match(/([\w$]+)\.paths/)?.[1];
        if (transform && (!state || !args)) throw new Error('Could not instrument transformation mock');
        if (transform) {
          source = source.replace(/case([`"'])(preview_live_edits|preview_transform_over_plan|preview_tag_edits|transform_sample|apply_plan|undo|set_locked_fields)\1:/g, (match, _quote, command) => {
            let trace = `window.transformCalls['${command}']=(window.transformCalls['${command}']||0)+1;`;
            if (command === 'preview_live_edits') trace += `window.transformRequests.push(structuredClone(${args}));`;
            if (command === 'apply_plan') trace += `window.transformUndo.push(structuredClone(${state}.tracks));`;
            if (command === 'undo') trace += `if(window.transformUndo.length)${state}.tracks=window.transformUndo.pop();`;
            if (command === 'set_locked_fields') trace += `window.transformLocked=[...${args}.fields];`;
            return match + trace;
          });
          source = source.replace(/(case([`"'])list_tracks\2:\s*return Promise\.resolve\()([\w$]+)\.tracks(\))/g, '$1structuredClone($3.tracks)$4');
          body = Buffer.from(source);
        }
      }
      await route.fulfill({ body, contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch (error) {
      if (!String(error).includes('ENOENT')) throw error;
      await route.fulfill({ status: 404, body: 'Missing fixture' });
    }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
  expect(await page.evaluate(() => !!window.__TAURI__)).toBe(false);
  await page.locator('#root-display').click(); await page.locator('#root').fill('/music'); await page.locator('#root').press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
  await openTagSource(page, 'editor');
  await expect(page.locator('#groups-menu')).toContainText('Upper titles');
});
test.afterEach(async ({ page }) => expect(issues.get(page)).toEqual([]));
const calls = (page, command) => page.evaluate(command => window.transformCalls[command] || 0, command);
const plan = page => page.evaluate(() => structuredClone(window.tagrex.previewPlan));
async function tick(page, name, on = true) {
  const previous=await page.evaluate(()=>document.querySelector('.subtab.active')?.dataset.subtab || 'online');
  await openTagSource(page,'transformations');
  const row = page.locator('#groups-menu .preset-check').filter({ has: page.locator('.preset-apply', { hasText: name }) });
  const input = row.locator('input.group-tick');
  if (await input.isChecked() !== on) await row.getByRole('checkbox').click();
  await expect(input).toBeChecked({ checked: on });
  if(previous!=='transformations')await openTagSource(page,previous);
}
async function changed(page, count = 1) {
  await expect.poll(async () => (await plan(page))?.changes.length || 0).toBe(count);
  await expect(page.locator('#status-transform-scope')).toBeHidden();
  await expect(page.locator('#diff-apply')).toBeEnabled();
}

test('ticking a visible preset automatically previews the selection in Tags', async ({ page }) => {
  await tick(page, 'Upper titles');
  await changed(page);
  await expect(page.locator('#files-view')).toBeVisible();
  await expect(page.locator('#diff-actionbar')).toBeVisible();
  expect((await plan(page)).changes[0].tag_changes.find(change => change.field === 'title').new).toBe('DESERT RAIN');
  expect(await calls(page, 'preview_tag_edits')).toBe(0);
  expect(await calls(page, 'apply_plan')).toBe(0);
});

test('the same active filename rules run after a chosen Online import', async ({ page }) => {
  await openTagSource(page, 'online');
  await tick(page, 'Upper filenames'); await changed(page);
  expect((await plan(page)).changes[0].rename_to).toContain('THE X FACTOR');
  await page.evaluate(() => { document.getElementById('discogs-token').value = 'fixture'; });
  await page.locator('#discogs-query').fill('La Bush'); await page.locator('#discogs-search').click();
  const card = page.locator('.release-card').first(); await card.locator('.release-head').click();
  await expect(card).toHaveClass(/tracklist-loaded/); await card.locator('.release-import').click();
  await expect.poll(() => calls(page, 'preview_live_edits')).toBeGreaterThan(0);
  await expect.poll(async () => (await plan(page))?.changes.some(change => change.tag_changes.length > 0)).toBe(true);
  await expect.poll(async()=> (await plan(page))?.changes.some(change=>!!change.rename_to)).toBe(true);
  expect(await calls(page, 'preview_transform_over_plan')).toBe(0);
  expect(await calls(page, 'apply_plan')).toBe(0);
});

test('automatic filename Apply and Undo keep paths correct without reselecting files in Online', async ({ page }) => {
  const original = await page.evaluate(() => window.tagrex.selectedPaths()[0]);
  await openTagSource(page, 'online');
  await tick(page, 'Upper filenames'); await changed(page);
  const renamed = (await plan(page)).changes[0].rename_to;
  await page.locator('#diff-apply').click();
  await expect.poll(() => page.evaluate(path => window.tagrex.tracks.some(track=>track.path===path),renamed)).toBe(true);
  expect(await page.evaluate(() => window.tagrex.selectedPaths())).toEqual([]);
  await expect.poll(() => plan(page)).toBe(null);
  await expect(page.locator('#tracks-body tr.selected')).toHaveCount(0);
  expect(await calls(page, 'apply_plan')).toBe(1);
  await page.locator('#undo').click();
  await expect.poll(() => page.evaluate(path => window.tagrex.tracks.some(track=>track.path===path),original)).toBe(true);
  expect(await page.evaluate(() => window.tagrex.selectedPaths())).toEqual([]);
  await expect.poll(() => plan(page)).toBe(null);
  await expect(page.locator('#tracks-body tr.selected')).toHaveCount(0);
  expect(await calls(page, 'undo')).toBe(1);
  await expect(page.locator('#groups-menu .preset-check').filter({ hasText: 'Upper filenames' }).locator('input.group-tick')).toBeChecked();
});

test('toggling a non-idempotent preset is A/B and selecting another diff row retargets the preview', async ({ page }) => {
  await tick(page, 'Append title mark'); await changed(page);
  expect((await plan(page)).changes[0].tag_changes[0].new).toBe('Desert Rain!');
  await tick(page, 'Append title mark', false);
  await expect.poll(() => plan(page)).toBe(null);
  await expect(page.locator('#diff-actionbar')).toBeVisible();await expect(page.locator('#diff-apply')).toBeDisabled();
  await tick(page, 'Append title mark'); await changed(page);
  expect((await plan(page)).changes[0].tag_changes[0].new).toBe('Desert Rain!');
  await page.locator('#tracks-body tr[data-path]').nth(1).locator('td.file').click();
  await expect.poll(async () => (await plan(page))?.changes[0]?.tag_changes[0]?.new).toBe('Radio!');
  expect((await plan(page)).changes[0].path).toContain('02 -');
  expect(await page.evaluate(() => window.tagrex.selectedPaths())).toEqual([(await plan(page)).changes[0].path]);
  expect(await page.evaluate(() => window.tagrex.tracks[0].tags.title)).toBe('Desert Rain');
  expect(await calls(page, 'apply_plan')).toBe(0);
});

test('Apply and Undo each use one batch and leave saved values visible with presets still checked', async ({ page }) => {
  await tick(page, 'Append title mark'); await changed(page);
  await page.locator('#diff-apply').click();
  await expect.poll(() => page.evaluate(() => window.tagrex.tracks[0].tags.title)).toBe('Desert Rain!');
  await expect.poll(() => plan(page)).toBe(null);
  await expect(page.locator('#diff-actionbar')).toBeVisible();await expect(page.locator('#diff-apply')).toBeDisabled();
  expect(await calls(page, 'apply_plan')).toBe(1);
  await expect(page.locator('#undo')).toBeEnabled(); await page.locator('#undo').click();
  await expect.poll(() => page.evaluate(() => window.tagrex.tracks[0].tags.title)).toBe('Desert Rain');
  await expect.poll(() => plan(page)).toBe(null);
  await expect(page.locator('#diff-actionbar')).toBeVisible();await expect(page.locator('#diff-apply')).toBeDisabled();
  expect(await calls(page, 'undo')).toBe(1);
  await expect(page.locator('#groups-menu .preset-check').filter({ hasText: 'Append title mark' }).locator('input.group-tick')).toBeChecked();
  await page.locator('#tracks-body tr[data-path] td.file').first().click();
  await tick(page, 'Append title mark', false); await tick(page, 'Append title mark'); await changed(page);
  expect((await plan(page)).changes[0].tag_changes[0].new).toBe('Desert Rain!');
});

test('slow previews coalesce to the newest selection and never enable a stale Apply', async ({ page }) => {
  await page.evaluate(() => { window.transformHold = true; });
  await tick(page, 'Append title mark');
  await expect.poll(() => page.evaluate(() => window.transformHeld.length)).toBe(1);
  await page.locator('#tracks-body tr[data-path]').nth(1).locator('td.file').click();
  // Selection B queues behind the single active native job. Its obsolete A
  // response must never become an applicable plan while B is waiting.
  await page.waitForTimeout(100);
  expect(await page.evaluate(() => window.transformHeld.length)).toBe(1);
  await page.evaluate(() => window.releaseTransform(0));
  await expect.poll(() => page.evaluate(() => window.transformHeld.length)).toBe(2);
  expect(await plan(page)).toBeNull();
  await page.evaluate(() => window.releaseTransform(1)); await changed(page);
  const newest = await plan(page);
  expect(newest.changes[0].path).toContain('02 -');
  await page.locator('#tracks-body tr[data-path]').nth(2).locator('td.file').click();
  await expect(page.locator('#diff-apply')).toBeDisabled();
  await page.keyboard.press('Control+Enter');
  expect(await calls(page, 'apply_plan')).toBe(0);
  await expect.poll(() => page.evaluate(() => window.transformHeld.length)).toBe(3);
  await page.evaluate(() => window.releaseTransform(2)); await changed(page);
  expect((await plan(page)).changes[0].path).toContain('03 -');
});

test('manual values and active rules compose after late transform responses', async ({ page }) => {
  await page.evaluate(() => { window.transformHold = true; });
  await tick(page, 'Upper titles');
  await expect.poll(() => page.evaluate(() => window.transformHeld.length)).toBe(1);
  await page.locator('#fields-body input[data-key="artist"]').fill('Pending manual artist');
  await expect(page.locator('#diff-apply')).toBeDisabled();
  await page.evaluate(() => { window.transformHold = false; window.releaseTransform(0); });
  await expect(page.locator('#diff-apply')).toBeEnabled();
  expect((await plan(page)).changes[0].tag_changes.find(change => change.field === 'artist').new).toBe('Pending manual artist');
  expect((await plan(page)).changes[0].tag_changes.find(change => change.field === 'title').new).toBe('DESERT RAIN');
  await tick(page, 'Upper titles', false);
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await expect.poll(async()=> (await plan(page)).changes[0].tag_changes.some(change=>change.field==='title')).toBe(false);
  expect(await calls(page, 'apply_plan')).toBe(0);
});

test('typing while rules are active retains raw input and previews transformations after it', async ({ page }) => {
  await tick(page, 'Append title mark'); await changed(page);
  const title = page.locator('#fields-body input[data-key="title"]');
  await title.fill('Keep my typing');
  await expect(title).toHaveValue('Keep my typing');
  await expect(title).toHaveClass(/dirty/);
  await expect.poll(async()=> (await plan(page))?.changes[0]?.tag_changes.find(field=>field.field==='title')?.new).toBe('Keep my typing!');
  await tick(page, 'Append title mark', false);
  await expect.poll(async()=> (await plan(page))?.changes[0]?.tag_changes.find(field=>field.field==='title')?.new).toBe('Keep my typing');
  await expect(title).toHaveValue('Keep my typing');
  expect(await calls(page, 'apply_plan')).toBe(0);
});

test('field locks are honored and changing a lock recomputes the checked preset', async ({ page }) => {
  const lock = page.locator('#fields-body .fe-row').filter({ has: page.locator('input[data-key="title"]') }).locator('.fe-lock');
  await lock.click(); await tick(page, 'Upper titles');
  await expect.poll(() => calls(page, 'preview_live_edits')).toBe(1);
  await expect.poll(() => page.evaluate(() => document.getElementById('status-transform-scope').getAttribute('aria-busy'))).toBe('false');
  expect(await plan(page)).toBe(null);
  expect(await page.evaluate(() => window.tagrex.tracks[0].tags.title)).toBe('Desert Rain');
  await lock.click(); await changed(page);
  expect((await plan(page)).changes[0].tag_changes[0].new).toBe('DESERT RAIN');
  expect(await calls(page, 'apply_plan')).toBe(0);
});

test('selection preview covers virtual rows while filters keep hidden selections explicit', async ({ page }) => {
  await page.evaluate(async () => {
    await window.tagrex.invoke('open_drop', { paths: Array.from({ length: 1200 }, (_, i) => `track-${String(i).padStart(3, '0')}.mp3`) });
    document.getElementById('root').value = '/dropped'; await window.tagrex.openLibrary();
  });
  await tick(page, 'Append title mark'); await changed(page);
  await page.locator('#tracks-body tr[data-path]').first().focus(); await page.keyboard.press('Control+a');
  await changed(page, 1200);
  expect(await page.locator('#tracks-body tr[data-path]').count()).toBeLessThan(100);
  await page.locator('#filter').fill('track-000');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(1);
  await expect(page.locator('#status-transform-scope')).toBeHidden();
  await expect(page.locator('#staged-count')).toContainText(/1,?200/);
  expect((await plan(page)).changes).toHaveLength(1200);
  expect(await calls(page, 'apply_plan')).toBe(0);
});

test('the compact375px layout keeps scope and explicit Apply reachable after selecting a transformation', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 667 });
  await page.getByRole('tab', { name: 'Editor', exact: true }).click();
  await tick(page, 'Upper titles'); await changed(page);
  await page.getByRole('tab', { name: 'Files', exact: true }).click();
  await expect(page.locator('#diff-apply')).toBeInViewport({ ratio: 1 });
  await expect(page.locator('#diff-apply')).toHaveAccessibleName('Apply to 1 file');
  await expect(page.locator('#status-transform-scope')).toBeHidden();
  await page.locator('#diff-apply').focus(); await expect(page.locator('#diff-apply')).toBeFocused();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
  expect(await calls(page, 'apply_plan')).toBe(0);
});


test('clearing mixed titles stages only the selected files and Apply/Undo round-trips them',async({page})=>{
  const before=await page.evaluate(()=>window.tagrex.tracks.map(track=>({path:track.path,tags:structuredClone(track.tags)})));
  await page.locator('#tracks-body tr[data-path]').first().click();
  await page.locator('#tracks-body tr[data-path]').nth(1).click({modifiers:['Shift']});
  const input=page.locator('#editor-field-title');await expect(input).toHaveClass(/multiple/);await input.focus();
  await page.locator('#editor-field-title-clear').click();await changed(page,2);
  const removal=(await plan(page)).changes;
  expect(removal.map(change=>change.path)).toEqual(before.slice(0,2).map(track=>track.path));
  for(const change of removal)expect(change.tag_changes).toEqual([{field:'title',old:before.find(track=>track.path===change.path).tags.title,new:null,invalid:false}]);
  expect(await page.evaluate(()=>window.tagrex.tracks.map(track=>track.tags))).toEqual(before.map(track=>track.tags));
  expect(await calls(page,'apply_plan')).toBe(0);
  await page.locator('#diff-apply').click();await expect(page.locator('#staged-edits')).toBeHidden();
  expect(await page.evaluate(()=>window.tagrex.tracks.map(track=>track.tags.title||''))).toEqual(['','',before[2].tags.title]);
  await page.locator('#undo').click();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.tracks.map(track=>track.tags))).toEqual(before.map(track=>track.tags));
});

for (const theme of ['dark','light']) test(`quiet chain expands rules in place without running a sample (${theme})`,async({page},info)=>{
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
  await openTagSource(page,'transformations');
  const row=page.locator('#groups-menu .preset-row').filter({hasText:'Upper titles'});
  await row.getByRole('button',{name:'Upper titles',exact:true}).click();
  await expect(row.getByRole('button')).toHaveAttribute('aria-expanded','true');
  await expect(row.getByRole('checkbox')).not.toBeChecked();
  await expect(page.locator('.transform-rule-details:visible li')).toHaveCount(1);
  await expect(page.locator('#transform-sample, #transform-inspect, #transform-enabled, #transform-manage')).toHaveCount(0);
  await expect(page.locator('#staged-edits')).toBeHidden();
  expect(await calls(page,'apply_plan')).toBe(0);
  expect(await calls(page,'transform_sample')).toBe(0);
  const second=page.locator('#groups-menu .preset-row').filter({hasText:'Append title mark'});
  await second.getByRole('button').click();
  await expect(row.getByRole('button')).toHaveAttribute('aria-expanded','false');
  await expect(page.locator('.transform-rule-details:visible')).toHaveCount(1);
  await page.locator('#transform-add').click();
  await expect(page.locator('#transform-search')).toBeFocused();
  await page.locator('#transform-search').fill('Append');
  await expect(page.locator('.transform-library-preset')).toHaveCount(1);
  await page.locator('#transform-search').press('Escape');
  await expect(page.locator('#transform-library')).toBeHidden();
  await expect(page.locator('#transform-add')).toBeFocused();
  await page.setViewportSize({width:390,height:760});
  await openTagSource(page,'transformations');
  expect(await page.locator('#transformation-pane').evaluate(node=>node.scrollWidth-node.clientWidth)).toBeLessThanOrEqual(1);
  await page.locator('.mode-col').screenshot({path:info.outputPath(`quiet-transformations-${theme}.png`)});
});

test('sidebar New opens a draft preset without changing the saved library',async({page})=>{
  await openTagSource(page,'transformations');
  await page.locator('#transform-add').click();
  await page.locator('#transform-new').click();
  await expect(page.locator('#pe-editor')).toBeVisible();
  await expect(page.locator('#pe-rules .rule-card')).toHaveCount(0);
  expect(await calls(page,'apply_plan')).toBe(0);
});

test('Add enables a built-in in the chain without opening the editor or writing files',async({page})=>{
  await openTagSource(page,'transformations');
  await page.locator('#transform-add').click();
  const builtin=page.locator('.transform-library-preset[data-builtin="true"]').first();
  const name=await builtin.innerText();
  await builtin.click();
  await expect(page.locator('#transform-library')).toBeHidden();
  const row=page.locator('#groups-menu .preset-row').filter({has:page.getByRole('button',{name,exact:true})});
  await expect(row.getByRole('checkbox')).toBeChecked();
  await expect(row.getByRole('button')).toHaveAttribute('aria-expanded','true');
  await expect(page.locator('#pe-editor')).toBeHidden();
  expect(await calls(page,'apply_plan')).toBe(0);
  await row.getByRole('checkbox').uncheck();
  await expect(row.getByRole('checkbox')).not.toBeChecked();
});

test('transformation table sits above fields, resizes without reflowing the library, and remembers its height',async({page})=>{
  await openTagSource(page,'editor');
  await expect(page.getByRole('tab',{name:'Transformations',exact:true})).toHaveCount(0);
  const table=page.locator('#transform-block'),handle=page.locator('#transform-resize');
  const before=await table.boundingBox(),field=await page.locator('#editor-field-title').boundingBox();
  expect(before.y+before.height).toBeLessThan(field.y);
  await handle.focus();await handle.press('Home');
  const small=await table.boundingBox();
  const library=await page.locator('#files-view').boundingBox();
  await page.locator('#tracks-body tr[data-path]').first().evaluate(n=>window.keptTableRow=n);
  const start=await handle.boundingBox();
  await page.mouse.move(start.x+start.width/2,start.y+start.height/2);await page.mouse.down();
  await page.mouse.move(start.x+start.width/2,start.y+start.height/2+72,{steps:6});await page.mouse.up();
  const large=await table.boundingBox();expect(large.height-small.height).toBeCloseTo(72,0);
  expect(await page.locator('#files-view').boundingBox()).toEqual(library);
  expect(await page.evaluate(()=>window.keptTableRow.isConnected)).toBe(true);
  const saved=await page.evaluate(()=>Number(localStorage.getItem('tagrex.transformationsHeight')));
  expect(saved).toBeCloseTo(large.height,0);
  await page.reload();await openTagSource(page,'editor');
  expect((await table.boundingBox()).height).toBeCloseTo(saved,0);
});

test('internal fades and overlay scrollbar track overflow without moving content or losing drafts',async({page})=>{
  await openTagSource(page,'editor');
  await page.locator('#editor-field-title').fill('Keep my raw draft');await changed(page);
  const before=await plan(page);
  await page.locator('#transform-resize').focus();await page.keyboard.press('Home');
  const port=page.locator('#transform-scroll');
  await expect(port).toHaveAttribute('data-fade-top','false');
  await expect(port).toHaveAttribute('data-fade-bottom','true');
  const widths=await port.evaluate(n=>({port:n.clientWidth,rows:document.getElementById('groups-menu').clientWidth}));
  await port.evaluate(n=>n.scrollTop=(n.scrollHeight-n.clientHeight)/2);
  await expect(port).toHaveAttribute('data-fade-top','true');
  await expect(port).toHaveAttribute('data-fade-bottom','true');
  const rail=page.locator('#transform-block > .inspector-scrollbar.is-vertical');
  await expect(rail).toHaveClass(/is-active/);
  expect(await port.evaluate(n=>({port:n.clientWidth,rows:document.getElementById('groups-menu').clientWidth}))).toEqual(widths);
  await port.evaluate(n=>n.scrollTop=n.scrollHeight);
  await expect(port).toHaveAttribute('data-fade-bottom','false');
  await expect(rail).not.toHaveClass(/is-active/);
  expect(await rail.evaluate(n=>getComputedStyle(n).position)).toBe('absolute');
  expect(await port.evaluate(n=>getComputedStyle(n).scrollbarWidth)).toBe('none');
  await expect(page.locator('#editor-field-title')).toHaveValue('Keep my raw draft');
  expect(await plan(page)).toEqual(before);
});

test('the combined editor scrolls under fixed tabs with fades on both edges',async({page})=>{
  await openTagSource(page,'editor');
  const port=page.locator('#subtab-editor .editor-scroll');
  await port.evaluate(async node => {
    node.scrollTop = 0;
    // Both reveal animations translate the port while its scroll offset stays
    // fixed. Measure the scroll displacement once those animations finish.
    await new Promise(requestAnimationFrame);
    const panels = [node.closest('.mode-panel'), node.closest('.subtab-panel')];
    await Promise.allSettled(panels.flatMap(panel => panel.getAnimations().map(animation => animation.finished)));
  });
  await waitForScrollIdle(port);
  const tabs=await page.locator('.subtabs').boundingBox(),table=await page.locator('#transform-block').boundingBox();
  expect(table.y-tabs.y-tabs.height).toBeLessThanOrEqual(8);
  const before=await page.locator('#editor-field-title').boundingBox();
  await port.evaluate(n=>n.scrollTop=60);
  await expect(port).toHaveClass(/can-scroll-up/);
  await waitForScrollIdle(port);
  expect(await page.locator('.subtabs').boundingBox()).toEqual(tabs);
  expect(table.y-(await page.locator('#transform-block').boundingBox()).y).toBeCloseTo(60,0);
  expect(before.y-(await page.locator('#editor-field-title').boundingBox()).y).toBeCloseTo(60,0);
  expect(await port.evaluate(n=>getComputedStyle(n).getPropertyValue('--editor-fade-top'))).toBe('8px');
  await expect(port).toHaveClass(/can-scroll-down/);
  expect(await port.evaluate(n=>getComputedStyle(n).getPropertyValue('--editor-fade-bottom'))).toBe('8px');
});

for(const theme of ['light','dark'])test(`${theme}: target arrows describe scopes and labels stay centred in their existing columns`,async({page})=>{
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
  await openTagSource(page,'editor');
  for(const [name,target] of [['Upper titles','TAG'],['All fields cleanup','ALL'],['Upper filenames','FILENAME']]) {
    const row=page.locator('#groups-menu .preset-row').filter({has:page.getByRole('button',{name,exact:true})});
    await expect(row.locator('.preset-target')).toHaveText(target);
    await expect(row.locator('.preset-target use')).toHaveAttribute('href','#i-arrow-right');
  }
  const labels=await page.locator('#fields-body .fe-label,#fields-body .fe-duo-label').evaluateAll(nodes=>nodes.map(n=>({text:n.textContent,align:getComputedStyle(n).textAlign,width:n.getBoundingClientRect().width})));
  expect(labels.every(label=>label.align==='center')).toBe(true);
  expect(labels.find(label=>label.text==='Title').width).toBe(74);
  expect(labels.find(label=>label.text==='Disc').width).toBe(32);
});

test('header collapse animates, reverses, and restores the chosen height without changing drafts',async({page})=>{
  await openTagSource(page,'editor');
  await page.locator('#editor-field-title').fill('Keep this draft');await changed(page);
  const before=await plan(page),table=page.locator('#transform-block'),toggle=page.locator('#transform-collapse');
  const expanded=(await table.boundingBox()).height;
  await page.locator('#groups-menu .group-tick').first().evaluate(n=>window.keptTransformTick=n);
  await toggle.focus();
  const intermediate=await table.evaluate(async n=>{
    document.getElementById('transform-collapse').click();
    await new Promise(requestAnimationFrame);
    const animation=n.getAnimations().find(a=>a.transitionProperty==='height');
    if(!animation)return {height:n.getBoundingClientRect().height,animations:0};
    // Inspect the actual transition midpoint rather than racing real frame
    // timing across headless workers. Native smoothness is checked in-app.
    animation.pause();animation.currentTime=Number(animation.effect.getComputedTiming().duration)/2;
    const height=n.getBoundingClientRect().height;animation.play();
    return {height,animations:1};
  });
  await expect(toggle).toHaveAttribute('aria-expanded','false');
  expect(intermediate.height).toBeLessThan(expanded);expect(intermediate.height).toBeGreaterThan(34);expect(intermediate.animations).toBeGreaterThan(0);
  await expect(page.locator('#transform-scroll')).toHaveJSProperty('inert',true);
  expect((await table.boundingBox()).height).toBeLessThan(40);
  await toggle.press('Enter');await expect(table).not.toHaveAttribute('data-collapse-animating','true');
  expect((await table.boundingBox()).height).toBeCloseTo(expanded,0);
  await page.evaluate(async()=>{const toggle=document.getElementById('transform-collapse');toggle.click();await new Promise(requestAnimationFrame);toggle.click();});
  await expect(table).not.toHaveAttribute('data-collapse-animating','true');
  expect((await table.boundingBox()).height).toBeCloseTo(expanded,0);
  expect(await page.evaluate(()=>window.keptTransformTick.isConnected)).toBe(true);
  expect(await plan(page)).toEqual(before);
  await expect(page.locator('#editor-field-title')).toHaveValue('Keep this draft');
});

test('collapsed preference persists and reduced motion opens instantly at the saved height',async({page})=>{
  await openTagSource(page,'editor');await page.emulateMedia({reducedMotion:'reduce'});
  const table=page.locator('#transform-block'),toggle=page.locator('#transform-collapse');
  const expanded=(await table.boundingBox()).height;
  await toggle.click();await expect(toggle).toHaveAttribute('aria-expanded','false');
  expect((await table.boundingBox()).height).toBeLessThan(40);
  await page.reload();await openTagSource(page,'editor');
  await expect(toggle).toHaveAttribute('aria-expanded','false');
  expect((await table.boundingBox()).height).toBeLessThan(40);
  await page.locator('#transform-add').click();await expect(toggle).toHaveAttribute('aria-expanded','true');
  expect((await table.boundingBox()).height).toBeCloseTo(expanded,0);
  await expect(page.locator('#transform-search')).toBeFocused();
});
