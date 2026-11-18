import { openTagSource } from './navigation.js';
import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
const errors = new WeakMap();
test.beforeEach(async ({ page }) => {
  const issues = []; errors.set(page, issues);
  page.on('pageerror', error => issues.push(error.message));
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try { await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' }); }
    catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  await page.goto('http://tagrex.test/tests/ui/mcp-native-fixture/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
  await page.locator('#root-display').click();
  await page.locator('#root').fill('/music'); await page.locator('#root').press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
  await openTagSource(page, 'editor');
});
test.afterEach(async ({ page }) => expect(errors.get(page)).toEqual([]));

async function staged(page, title) {
  await page.locator('#editor-field-title').fill(title);
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.manual?.changes[0]?.tag_changes.find(field => field.field === 'title')?.new)).toBe(title);
}

async function shared(page) {
  await page.evaluate(() => {
    window.mcpFixture.state.shared = true;
    window.mcpFixture.state.stage = structuredClone(window.mcpFixture.state.manual);
    window.mcpFixture.emit('service_changed', {});
  });
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.calls.includes('mcp_shared_stage'))).toBe(true);
}

async function nativeTransforms(page) {
  await page.evaluate(async()=>{
    window.mcpFixture.state.shared=true;window.mcpFixture.state.liveFields=true;window.mcpFixture.state.liveRecompute=true;
    window.mcpFixture.emit('service_changed',{});
    const settings=await window.__TAURI__.core.invoke('load_settings',{});
    document.dispatchEvent(new CustomEvent('tagrex:settings-saved',{detail:{...settings,action_groups:[{name:'Append once',scope:'title',rules:[{kind:'replace',scope:'title',from:'$',to:'!',regex:true,style:''}]}]}}));
  });
  await expect.poll(()=>page.evaluate(()=>window.mcpFixture.state.calls.includes('mcp_shared_stage_delta'))).toBe(true);
  const preset=page.locator('#groups-menu .preset-row').filter({hasText:'Append once'}).getByRole('checkbox');
  return {async click(){
    const previous=await page.evaluate(()=>document.querySelector('.subtab.active')?.dataset.subtab || 'editor');
    await openTagSource(page,'transformations');await preset.click();
    if(previous!=='transformations')await openTagSource(page,previous);
  }};
}

test('a human edit made during an agent write remains desired while saved bases refresh', async ({ page }) => {
  await staged(page, 'Human before agent');
  await page.evaluate(() => window.mcpFixture.holdAgentWrite(window.tagrex.tracks[0].path, 'Saved by agent'));
  await staged(page, 'Human after agent started');
  await page.evaluate(() => window.mcpFixture.state.complete());
  await expect.poll(() => page.evaluate(() => window.tagrex.previewPlan?.changes[0]?.tag_changes.find(field => field.field === 'title'))).toMatchObject({ old: 'Saved by agent', new: 'Human after agent started' });
  await expect(page.locator('#editor-field-title')).toHaveValue('Human after agent started');
  expect(await page.evaluate(() => window.tagrex.tracks[0].tags.title)).toBe('Saved by agent');
  expect(await page.evaluate(() => window.mcpFixture.state.calls.filter(command => command === 'apply_plan').length)).toBe(0);
  await page.locator('#diff-apply').click();
  await expect.poll(() => page.evaluate(() => window.tagrex.tracks[0].tags.title)).toBe('Human after agent started');
});

test('an unchanged human draft rebases without being applied by an agent event', async ({ page }) => {
  await staged(page, 'Keep this human title');
  await page.evaluate(async () => {
    await window.mcpFixture.holdAgentWrite(window.tagrex.tracks[0].path, 'Agent baseline');
    window.mcpFixture.state.complete();
  });
  await expect.poll(() => page.evaluate(() => window.tagrex.previewPlan?.changes[0]?.tag_changes.find(field => field.field === 'title'))).toMatchObject({ old: 'Agent baseline', new: 'Keep this human title' });
  await expect(page.locator('#editor-field-title')).toHaveValue('Keep this human title');
  expect(await page.evaluate(() => window.tagrex.tracks[0].tags.title)).toBe('Agent baseline');
});

test('trusted recovery displays the server scope and submits its exact proof', async ({ page }) => {
  await page.evaluate(() => {
    window.mcpFixture.state.recovery = { recovery_required: true, operations: [{ operation: 'operation_fixture', phase: 'completed', proof: 'verified-fixture-proof', action: 'accept_completed', guidance: 'These changes finished before the connection was interrupted.', batches: [{ id: 1, description: 'Agent fixture changes', paths: ['/music/example.flac'] }] }] };
    window.mcpFixture.emit('service_changed', {});
  });
  await page.locator('#agent-activity').click();
  await expect(page.getByRole('region', { name: 'Interrupted agent work' })).toBeVisible();
  await expect(page.getByText('/music/example.flac', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Keep completed changes', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.recovered)).toBe(true);
  await expect(page.getByRole('region', { name: 'Interrupted agent work' })).toHaveCount(0);
});

test('a stale recovery proof refreshes the scope before a person retries', async ({ page }) => {
  await page.evaluate(() => {
    window.mcpFixture.state.recovery = { recovery_required: true, operations: [{ operation: 'operation_fixture', phase: 'completed', proof: 'stale-proof', action: 'accept_completed', guidance: 'Review the interrupted work.' }] };
    window.mcpFixture.emit('service_changed', {});
  });
  await page.locator('#agent-activity').click();
  const restore = page.getByRole('button', { name: 'Keep completed changes', exact: true });
  await restore.click();
  await expect(page.locator('#mcp-error')).toContainText('recovery evidence changed');
  expect(await page.evaluate(() => window.mcpFixture.state.recovered)).toBe(false);
  await restore.click();
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.recovered)).toBe(true);
});

test('trusted pairing uses the opened folder and exposes only a credential file path', async ({ page }) => {
  await page.locator('#agent-activity').click();
  await expect(page.locator('#mcp-agent-roots')).toHaveValue('/music');
  await page.locator('#mcp-agent-label').fill('Codex fixture');
  await page.locator('#mcp-pair').click();
  await expect(page.locator('#mcp-connection-config')).toHaveValue(/--mcp-stdio/);
  await expect(page.locator('#mcp-connection-config')).toHaveValue(/client_fixture\.token/);
  expect(await page.evaluate(() => window.mcpFixture.state.clients[0].grant)).toMatchObject({ roots: ['/music'], actions: ['read', 'stage', 'providers'] });
  await page.getByRole('button', { name: 'Revoke', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Revoke', exact: true })).toBeDisabled();
});

test('unavailable recovery evidence keeps connection controls and guidance available', async ({ page }) => {
  await page.evaluate(() => {
    window.mcpFixture.state.recovery = { recovery_required: true, operations: [], guidance: 'Open the original folder to inspect this interrupted operation.' };
    window.mcpFixture.emit('service_changed', {});
  });
  await page.locator('#agent-activity').click();
  await expect(page.locator('#mcp-enable')).toBeEnabled();
  await expect(page.locator('#mcp-agent-label')).toBeVisible();
  await expect(page.getByText('Open the original folder to inspect this interrupted operation.', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Keep completed changes', exact: true })).toHaveCount(0);
});

test('a mask save conflict preserves the typed draft and shows success only after a real retry', async ({ page }) => {
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await page.locator('#mask').fill('%artist% - %title%');
  await page.locator('.preset-btn[data-preset-target="mask"]').click();
  await page.locator('#preset-name').fill('Human mask');
  await page.evaluate(() => { window.mcpFixture.state.failNextSettingsSave = true; });
  await page.locator('#preset-save').click();
  await expect(page.locator('#toast')).toContainText('Settings changed elsewhere');
  await expect(page.locator('#preset-name')).toHaveValue('Human mask');
  await expect(page.locator('#mask')).toHaveValue('%artist% - %title%');
  await expect(page.locator('#preset-list')).toContainText('Agent mask');
  await expect(page.locator('#preset-list')).not.toContainText('Human mask');
  await page.locator('#preset-save').click();
  await expect(page.locator('#preset-list')).toContainText('Human mask');
  await expect(page.locator('#preset-list')).toContainText('Agent mask');
  await expect(page.locator('#preset-name')).toHaveValue('');
});

test('agent fields appear live in Files, Staged and Inspector without changing focus or selection', async ({ page }) => {
  await shared(page);
  await page.locator('#editor-field-title').focus();
  await page.evaluate(() => { window.mcpFixture.titleNode = document.getElementById('editor-field-title'); });
  const selected = await page.evaluate(() => window.tagrex.selectedPaths());
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[0].path, { artist: 'Agent artist', title: 'Agent title' }));
  await expect(page.locator('#staged-tab-count')).toHaveText('1');
  await expect(page.locator('#editor-field-artist')).toHaveValue('Agent artist');
  await expect(page.locator('#editor-field-title')).toHaveValue('Agent title');
  await expect(page.locator('#editor-field-title')).toBeFocused();
  expect(await page.evaluate(() => document.getElementById('editor-field-title') === window.mcpFixture.titleNode)).toBe(true);
  expect(await page.evaluate(() => window.tagrex.selectedPaths())).toEqual(selected);
  await expect(page.locator('#files-tab')).toHaveAttribute('aria-selected', 'true');
  expect(await page.evaluate(() => window.mcpFixture.state.mediaWrites)).toBe(0);
  await page.locator('#staged-tab').click();
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(1);
  await expect(page.locator('#tracks-body')).toContainText('Agent artist');
  await expect(page.locator('#tracks-body')).toContainText('Agent title');
});

test('human title wins in the same stage while later agent album and artwork edits coexist', async ({ page }) => {
  await shared(page);
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[0].path, { artist: 'Agent artist', title: 'Initial agent title' }));
  await expect(page.locator('#editor-field-title')).toHaveValue('Initial agent title');
  await staged(page, 'Human title');
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[0].path, { album: 'Agent album' }, { old: [], new: [] }));
  await expect(page.locator('#editor-field-title')).toHaveValue('Human title');
  await expect(page.locator('#editor-field-artist')).toHaveValue('Agent artist');
  await expect(page.locator('#editor-field-album')).toHaveValue('Agent album');
  await expect.poll(() => page.evaluate(() => {
    const change = window.tagrex.previewPlan?.changes[0];
    return { title: change?.tag_changes.find(tag => tag.field === 'title')?.new, artist: change?.tag_changes.find(tag => tag.field === 'artist')?.new, artwork: Boolean(change?.cover_change) };
  })).toEqual({ title: 'Human title', artist: 'Agent artist', artwork: true });
  await page.locator('#diff-apply').click();
  await expect.poll(() => page.evaluate(() => window.tagrex.tracks[0].tags)).toMatchObject({ title: 'Human title', artist: 'Agent artist', album: 'Agent album' });
  expect(await page.evaluate(() => window.mcpFixture.state.mediaWrites)).toBe(1);
});

test('human-first title and agent artist remain editable and Apply uses current shared values', async ({ page }) => {
  await shared(page);
  await staged(page, 'Human first');
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[0].path, { artist: 'Agent later' }));
  await expect(page.locator('#editor-field-title')).toHaveValue('Human first');
  await expect(page.locator('#editor-field-artist')).toHaveValue('Agent later');
  await page.locator('#staged-tab').click();
  await page.locator('#diff-apply').click();
  await expect.poll(() => page.evaluate(() => window.tagrex.tracks[0].tags)).toMatchObject({ title: 'Human first', artist: 'Agent later' });
  await expect(page.locator('#staged-tab-count')).toHaveText('');
});

test('returning an agent title to its saved value removes only that field', async ({ page }) => {
  await shared(page);
  const saved = await page.evaluate(() => window.tagrex.tracks[0].tags.title);
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[0].path, { artist: 'Retain agent artist', title: 'Agent title' }));
  await expect(page.locator('#editor-field-title')).toHaveValue('Agent title');
  await page.locator('#editor-field-title').fill(saved);
  await expect.poll(() => page.evaluate(() => window.tagrex.previewPlan?.changes[0]?.tag_changes.map(tag => [tag.field, tag.new]))).toEqual([['artist', 'Retain agent artist']]);
  await expect(page.locator('#editor-field-title')).toHaveValue(saved);
  await expect(page.locator('#editor-field-artist')).toHaveValue('Retain agent artist');
  expect(await page.evaluate(() => window.mcpFixture.state.mediaWrites)).toBe(0);
});

test('late shared hydration cannot replace a raw title or detach its input', async ({ page }) => {
  await shared(page);
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[0].path, { artist: 'Agent artist', title: 'Older title' }));
  await expect(page.locator('#editor-field-title')).toHaveValue('Older title');
  await page.evaluate(() => {
    window.mcpFixture.titleNode = document.getElementById('editor-field-title');
    window.mcpFixture.state.holdNextStageRead = true;
    window.mcpFixture.emit('shared_stage_changed', {});
  });
  await expect.poll(() => page.evaluate(() => typeof window.mcpFixture.state.releaseStageRead)).toBe('function');
  await page.locator('#editor-field-title').fill('Typing while hydration waits');
  await expect(page.locator('#editor-field-title')).toHaveValue('Typing while hydration waits');
  await page.evaluate(() => window.mcpFixture.state.releaseStageRead());
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.stage?.changes[0]?.tag_changes.find(tag => tag.field === 'title')?.new)).toBe('Typing while hydration waits');
  await expect(page.locator('#editor-field-title')).toHaveValue('Typing while hydration waits');
  expect(await page.evaluate(() => document.getElementById('editor-field-title') === window.mcpFixture.titleNode)).toBe(true);
  await expect(page.locator('#editor-field-artist')).toHaveValue('Agent artist');
});

test('scoped agent Apply removes its file while unrelated human staging remains visible', async ({ page }) => {
  await shared(page);
  await staged(page, 'Keep human pending');
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[1].path, { album: 'Agent second file' }));
  await expect(page.locator('#staged-tab-count')).toHaveText('2');
  await page.evaluate(() => window.mcpFixture.commitAgent([window.tagrex.tracks[1].path]));
  await expect(page.locator('#staged-tab-count')).toHaveText('1');
  await expect(page.locator('#editor-field-title')).toHaveValue('Keep human pending');
  await expect.poll(() => page.evaluate(() => window.tagrex.tracks[1].tags.album)).toBe('Agent second file');
  expect(await page.evaluate(() => window.tagrex.tracks[0].tags.title)).not.toBe('Keep human pending');
  expect(await page.evaluate(() => window.mcpFixture.state.mediaWrites)).toBe(1);
});

test('Discard clears the one shared staging area without writing files', async ({ page }) => {
  await shared(page);
  await staged(page, 'Discard human');
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[1].path, { title: 'Discard agent' }));
  await expect(page.locator('#staged-tab-count')).toHaveText('2');
  await page.locator('#diff-discard').click();
  await expect(page.locator('#staged-tab-count')).toHaveText('');
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.stage)).toBe(null);
  expect(await page.evaluate(() => window.tagrex.previewPlan)).toBe(null);
  expect(await page.evaluate(() => window.mcpFixture.state.mediaWrites)).toBe(0);
});

test('selecting files keeps staged rename rows mounted and Inspector values live', async ({ page }) => {
  await shared(page);
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[0].path, { title: 'First staged title' }));
  await expect(page.locator('#staged-tab-count')).toHaveText('1');
  await page.evaluate(() => {
    const [first] = window.tagrex.tracks;
    window.mcpFixture.state.stage.changes[0].rename_to = first.path.replace(/\.[^.]+$/, '-clean.mp3');
    window.mcpFixture.state.revision++;
    window.mcpFixture.emit('shared_stage_changed', { revision: window.mcpFixture.state.revision });
  });
  await expect(page.locator('#staged-tab-count')).toHaveText('1');
  await page.evaluate(() => { window.mcpFixture.selectionRows = [...document.querySelectorAll('#tracks-body tr[data-path]')]; });
  await page.locator('#tracks-body tr[data-path]').nth(1).locator('td').nth(1).click();
  await page.locator('#tracks-body tr[data-path]').first().locator('td').nth(1).click();
  await expect(page.locator('#editor-field-title')).toHaveValue('First staged title');
  // Include the deferred automatic-preview/native-publication turn that used
  // to replace every row twice even though no staged value had changed.
  await page.waitForTimeout(140);
  expect(await page.evaluate(() => window.mcpFixture.selectionRows.every(row => row.isConnected))).toBe(true);
  expect(await page.evaluate(() => window.tagrex.previewPlan.changes.length)).toBe(1);
  await expect(page.locator('#staged-tab-count')).toHaveText('1');
});

test('identical shared acknowledgements cause zero table value mutations',async({page})=>{
  await shared(page);
  await page.evaluate(()=>window.mcpFixture.stageAgent(window.tagrex.tracks[0].path,{title:'Agent title'}));
  await expect(page.locator('#tracks-body')).toContainText('Agent title');
  await page.waitForTimeout(150);
  const result=await page.evaluate(async()=>{
    const body=document.getElementById('tracks-body');
    const nodes=[...body.querySelectorAll('td')].map(cell=>[cell,cell.firstChild]);
    let mutations=0; const detail=[];
    const observer=new MutationObserver(records=>{mutations+=records.length;detail.push(...records.map(record=>({tag:record.target.tagName,field:record.target.dataset?.field,name:record.attributeName,old:record.oldValue,value:record.target.getAttribute?.(record.attributeName)})));});
    observer.observe(body,{subtree:true,childList:true,characterData:true,attributes:true,attributeOldValue:true});
    window.mcpFixture.state.revision++;
    window.mcpFixture.emit('shared_stage_changed',{revision:window.mcpFixture.state.revision});
    await new Promise(resolve=>setTimeout(resolve,150));observer.disconnect();
    return {mutations,detail,retained:nodes.every(([cell,text])=>cell.isConnected&&cell.firstChild===text)};
  });
  if (result.mutations) console.log(JSON.stringify(result));
  expect({mutations:result.mutations,retained:result.retained}).toEqual({mutations:0,retained:true});
});

test('advertised live fields use small intents while preserving concurrent agent fields',async({page})=>{
  await page.evaluate(()=>{window.mcpFixture.state.shared=true;window.mcpFixture.state.liveFields=true;window.mcpFixture.emit('service_changed',{});});
  await expect.poll(()=>page.evaluate(()=>window.mcpFixture.state.calls.includes('mcp_shared_stage_delta'))).toBe(true);
  await page.evaluate(()=>window.mcpFixture.stageAgent(window.tagrex.tracks[0].path,{artist:'Agent artist'}));
  await expect(page.locator('#editor-field-artist')).toHaveValue('Agent artist');
  await page.locator('#editor-field-title').fill('Intent title');
  await expect.poll(()=>page.evaluate(()=>window.mcpFixture.state.calls.includes('mcp_publish_shared_intents'))).toBe(true);
  await expect(page.locator('#tracks-body')).toContainText('Intent title');
  await expect(page.locator('#editor-field-artist')).toHaveValue('Agent artist');
  expect(await page.evaluate(()=>window.mcpFixture.state.mediaWrites)).toBe(0);
});

test('native preset composition retains earlier raw fields and the final toggle-off recomputes without new edits',async({page})=>{
  const preset=await nativeTransforms(page);
  await preset.click();
  await page.locator('#editor-field-title').fill('Raw human title');
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toContainText('Raw human title!');
  await page.locator('#editor-field-artist').fill('Another human field');
  await expect(page.locator('#tracks-body td[data-field="artist"]').first()).toContainText('Another human field');
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toHaveText('Raw human title!');
  await expect(page.locator('#staged-transformations')).toHaveText('1 transformation');
  await expect(page.locator('#staged-tags')).toHaveText('1 tag change');
  await preset.click();
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toHaveText('Raw human title');
  await expect(page.locator('#staged-transformations')).toBeHidden();
  await expect(page.locator('#staged-tags')).toHaveText('2 tag changes');
  const requests=await page.evaluate(()=>window.mcpFixture.state.intentRequests);
  expect(requests.some(request=>request.recompute && request.groups.length===0)).toBe(true);
  expect(requests.every(request=>request.session==='fixture')).toBe(true);
  expect(await page.evaluate(()=>window.mcpFixture.state.mediaWrites)).toBe(0);
});

test('native preset toggle-off retires derived changes from earlier selected scopes',async({page})=>{
  const preset=await nativeTransforms(page);
  await preset.click();
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toHaveText('Desert Rain!');
  await page.locator('#tracks-body td[data-field="title"]').nth(1).click();
  await expect(page.locator('#tracks-body td[data-field="title"]').nth(1)).toHaveText('Radio!');
  await preset.click();
  await expect(page.locator('#staged-tab-count')).toHaveText('');
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toHaveText('Desert Rain');
  await expect(page.locator('#tracks-body td[data-field="title"]').nth(1)).toHaveText('Radio');
});

test('native preset recomputation preserves artwork edits without compounding title output',async({page})=>{
  const preset=await nativeTransforms(page);await preset.click();
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toHaveText('Desert Rain!');
  await page.locator('#cover-well .cover-edit').click();
  await page.getByRole('menuitem',{name:'Remove',exact:true}).click();
  await expect.poll(()=>page.evaluate(()=>window.mcpFixture.state.stage?.changes[0]?.cover_change?.new)).toEqual([]);
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toHaveText('Desert Rain!');
  await preset.click();
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toHaveText('Desert Rain');
  expect(await page.evaluate(()=>window.mcpFixture.state.stage?.changes[0]?.cover_change?.new)).toEqual([]);
  expect(await page.evaluate(()=>window.mcpFixture.state.mediaWrites)).toBe(0);
});

test('switching a native tag preset to filename rules still retires earlier derived scopes on toggle-off',async({page})=>{
  const preset=await nativeTransforms(page);
  await preset.click();
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toHaveText('Desert Rain!');
  await page.locator('#tracks-body td[data-field="title"]').nth(1).click();
  await expect(page.locator('#tracks-body td[data-field="title"]').nth(1)).toHaveText('Radio!');
  await page.evaluate(async()=>{
    const settings=await window.__TAURI__.core.invoke('load_settings',{});
    document.dispatchEvent(new CustomEvent('tagrex:settings-saved',{detail:{...settings,action_groups:[{name:'Append once',scope:'title',rules:[{kind:'replace',scope:'title',from:'$',to:'!',regex:true,style:''},{kind:'replace',scope:'filename',from:'$',to:'-copy',regex:true,style:''}]}]}}));
  });
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes.some(row=>row.rename_to))).toBe(true);
  await preset.click();
  await expect(page.locator('#staged-tab-count')).toHaveText('');
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toHaveText('Desert Rain');
  await expect(page.locator('#tracks-body td[data-field="title"]').nth(1)).toHaveText('Radio');
  expect(await page.evaluate(()=>window.mcpFixture.state.mediaWrites)).toBe(0);
});

test('native scalar staging also retains a later artwork removal and the human title', async ({page}) => {
  await page.evaluate(() => { window.mcpFixture.state.shared=true; window.mcpFixture.state.liveFields=true; window.mcpFixture.emit('service_changed',{}); });
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.calls.includes('mcp_shared_stage_delta'))).toBe(true);
  await staged(page, 'Keep this human title');
  await page.locator('#cover-well .cover-edit').click();
  await page.getByRole('menuitem', {name:'Remove',exact:true}).click();
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.stage?.changes[0]?.cover_change?.new)).toEqual([]);
  await expect(page.locator('#editor-field-title')).toHaveValue('Keep this human title');
  expect(await page.evaluate(() => window.mcpFixture.state.stage.changes[0].tag_changes.find(tag => tag.field==='title')?.new)).toBe('Keep this human title');
  expect(await page.evaluate(() => window.mcpFixture.state.mediaWrites)).toBe(0);
});

test('native scalar staging also accepts online imports without losing an existing human field', async ({page}) => {
  await page.evaluate(() => { window.mcpFixture.state.shared=true; window.mcpFixture.state.liveFields=true; window.mcpFixture.emit('service_changed',{}); });
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.calls.includes('mcp_shared_stage_delta'))).toBe(true);
  await page.locator('#editor-field-comment').fill('Keep this human comment');
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.stage?.changes[0]?.tag_changes.find(tag => tag.field==='comment')?.new)).toBe('Keep this human comment');
  await openTagSource(page, 'online');
  await page.evaluate(() => { document.getElementById('discogs-token').value='fixture'; });
  await page.locator('#discogs-query').fill('La Bush');
  await page.locator('#discogs-search').click();
  const card=page.locator('.release-card').first();
  await expect(card).toBeVisible();
  if(await card.getAttribute('aria-expanded')!=='true')await card.locator('.release-head').click();
  await expect(card).toHaveClass(/tracklist-loaded/);
  await card.locator('.release-import').click();
  await expect.poll(() => page.evaluate(() => window.mcpFixture.state.stage?.changes.some(change => change.tag_changes.some(tag => tag.field==='album')))).toBe(true);
  expect(await page.evaluate(() => window.mcpFixture.state.stage.changes[0].tag_changes.find(tag => tag.field==='comment')?.new)).toBe('Keep this human comment');
  expect(await page.evaluate(() => window.mcpFixture.state.mediaWrites)).toBe(0);
});

test('agent compilation changes mirror into the existing Inspector checkbox', async ({page}) => {
  await shared(page);
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[0].path, {[window.tagrex.tracks[0].compilation_field]:'1'}));
  await expect(page.locator('#editor-compilation-control')).toBeChecked();
  await page.evaluate(() => window.mcpFixture.stageAgent(window.tagrex.tracks[0].path, {[window.tagrex.tracks[0].compilation_field]:'0'}));
  await expect(page.locator('#editor-compilation-control')).not.toBeChecked();
  expect(await page.evaluate(() => window.mcpFixture.state.mediaWrites)).toBe(0);
});

test('first scalar staging leaves unrelated Files rows entirely untouched',async({page})=>{
  await shared(page);
  await page.evaluate(()=>{
    window.unrelatedRowMutations=[];
    const rows=[...document.querySelectorAll('#tracks-body tr[data-path]')];
    window.unrelatedRows=rows.slice(1);
    window.unrelatedObserver=new MutationObserver(records=>{
      for(const record of records){
        const target=record.target.nodeType===1?record.target:record.target.parentElement;
        if(window.unrelatedRows.includes(target?.closest('tr[data-path]')))window.unrelatedRowMutations.push({type:record.type,attribute:record.attributeName});
      }
    });
    window.unrelatedObserver.observe(document.getElementById('tracks-body'),{subtree:true,childList:true,attributes:true,characterData:true});
  });
  await staged(page,'Only this title changes');
  await page.waitForTimeout(150);
  const mutations=await page.evaluate(()=>{window.unrelatedObserver.disconnect();return window.unrelatedRowMutations;});
  expect(mutations).toEqual([]);
});
