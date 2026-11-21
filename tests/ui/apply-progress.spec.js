import { openTagSource } from './navigation.js';
import { waitForScrollIdle } from './scrolling.js';
import {test,expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
const root=resolve(process.env.TAGREX_UI_DIST || '.test-dist');
test.beforeEach(async({page})=>{
  page.errors=[];page.on('pageerror',error=>page.errors.push(error.message));
  await page.route('http://tagrex.test/**',async route=>{
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);if(!path.startsWith(root+sep))return route.abort();
    try{await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});}catch{await route.fulfill({status:404});}
  });
});
test.afterEach(async({page})=>expect(page.errors).toEqual([]));
async function open(page,large=false){
  if(large)await page.addInitScript(()=>{
    const original=Promise.resolve;
    Promise.resolve=function(value){
      if(Array.isArray(value)&&value.length===3&&value.every(track=>track.path&&track.tags)){
        const seed=structuredClone(value[0]);
        value.splice(0,value.length,...Array.from({length:400},(_,index)=>({...structuredClone(seed),path:`/music/${String(index).padStart(4,'0')}.mp3`,tags:{...seed.tags,artist:`Original ${index}`}})));
      }
      return original.call(this,value);
    };
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async()=>{document.getElementById('root').value='/music';await window.tagrex.openLibrary();window.tagrex.selection.clear();for(const track of window.tagrex.tracks)window.tagrex.selection.add(track.path);window.tagrex.renderTracks();});
}
async function stageTags(page){
  await openTagSource(page, 'editor');
  await page.locator('#fields-body [data-key="artist"]').fill('Written artist');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.locator('#staged-tab').click();
}
async function holdApply(page){
  await page.evaluate(()=>{
    const originalTracks=window.tagrex.tracks.map(track=>structuredClone(track));
    window.__TAURI__={event:{listen:async(_event,callback)=>{window.writeProgress=payload=>callback({payload});return ()=>{window.writeProgress=null;};}}};
    const original=Promise.resolve;
    Promise.resolve=function(value){
      if(value?.id&&value?.description&&'applied_at' in value)return new Promise((resolve,reject)=>{window.finishWrite=()=>resolve(value);window.failWrite=(code='error.plan.cancelled')=>{window.tagrex.tracks.forEach((track,index)=>Object.assign(track,originalTracks[index]));reject({message:{code}});};});
      return original.call(this,value);
    };
    window.beforeRows=[...document.querySelectorAll('#tracks-body tr')];
    window.beforeCells=[...document.querySelectorAll('#tracks-body td')];
    window.beforeHeaders=[...document.querySelectorAll('#tracks thead th')];
    window.structuralChanges=0;
    window.progressObserver=new MutationObserver(records=>{
      for(const record of records)if(record.type==='childList'&&[...record.addedNodes,...record.removedNodes].some(node=>node.nodeType===1&&['TR','TD','TH'].includes(node.nodeName)))window.structuralChanges++;
    });
    window.progressObserver.observe(document.getElementById('tracks'),{childList:true,subtree:true});
  });
  await page.locator('#diff-apply').click();
  await expect.poll(()=>page.evaluate(()=>typeof window.finishWrite)).toBe('function');
  await expect(page.locator('#diff-apply')).toHaveAttribute('aria-busy','true');
}
const report=(page,event)=>page.evaluate(event=>window.writeProgress(event),event);
async function stable(page){
  expect(await page.evaluate(()=>window.structuralChanges)).toBe(0);
  expect(await page.evaluate(()=>window.beforeRows.every(node=>node.isConnected)&&window.beforeCells.every(node=>node.isConnected)&&window.beforeHeaders.every(node=>node.isConnected))).toBe(true);
}
async function finish(page){
  await page.evaluate(()=>{window.progressObserver.disconnect();window.finishWrite();});
  await expect(page.locator('#diff-actionbar')).toBeVisible();await expect(page.locator('#diff-apply')).toBeDisabled();
  await expect(page.locator('#diff-apply')).not.toHaveAttribute('aria-busy');
  await expect(page.locator('#diff-apply')).toBeDisabled();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes?.length || 0)).toBe(0);
  expect(await page.evaluate(()=>window.tagrex.selectedPaths())).toEqual([]);
  await expect(page.locator('#tracks-body tr.selected')).toHaveCount(0);
  await expect(page.locator('#tracks-body .sheet-active,#tracks-body .sheet-range')).toHaveCount(0);
}

for(const kind of ['tags','transfer'])test(`${kind} Apply patches values in order, clears completed selection, and retains every table node`,async({page},testInfo)=>{
  await open(page);
  if(kind==='tags')await stageTags(page);
  else {await page.locator('.mode-tab[data-mode="renamer"]').click();await page.locator('#mask').fill('Saved %artist% - %title%');await page.locator('#preview').click();await expect(page.locator('#diff-apply')).toBeEnabled();await page.locator('#staged-tab').click();}
  const changes=await page.evaluate(()=>window.tagrex.previewPlan.changes);
  await holdApply(page);
  const rows=page.locator('#tracks-body tr[data-path]');
  for(let index=0;index<changes.length;index++){
    const change=changes[index],row=rows.nth(index);
    const cell=kind==='tags'?row.locator('td[data-field="artist"]'):row.locator('.fname');
    await expect(cell).toContainText(kind==='tags'?change.tag_changes.find(field=>field.field==='artist').old:change.path.split('/').pop());
    await report(page,{path:change.path,done:changes.length+index,total:changes.length*2});
    await expect(page.locator('#activity-meter')).toHaveAttribute('role','progressbar');
    await expect(page.locator('#activity-meter')).toHaveAttribute('aria-valuenow',String(Math.round(index / changes.length * 100)));
    await expect(row).toHaveClass(/applying-current/);
    await report(page,{path:change.path,done:changes.length+index+1,total:changes.length*2,completed:kind});
    await expect(cell).toContainText(kind==='tags'?'Written artist':change.rename_to.split('/').pop());
    await expect(row).not.toHaveClass(/selected/);await expect(row).toHaveAttribute('aria-disabled','true');
    if(changes.length-index-1>1) await expect(page.locator('#files-selected-count')).toHaveText(String(changes.length-index-1));
    else await expect(page.locator('#selection-summary')).toBeHidden();
    await report(page,{path:change.path,done:changes.length+index+1,total:changes.length*2,completed:kind});
    if(changes.length-index-1>1) await expect(page.locator('#files-selected-count')).toHaveText(String(changes.length-index-1));
    else await expect(page.locator('#selection-summary')).toBeHidden();
    await expect(row).not.toHaveClass(/applying-current/);
    await stable(page);
    if(index===0&&kind==='tags')for(const theme of ['light','dark']){
      await page.emulateMedia({colorScheme:theme});
      await page.screenshot({path:testInfo.outputPath(`progress-${theme}.png`),animations:'disabled'});
    }
  }
  await finish(page);
});

test('a cancelled transaction restores the same cells and preview selection',async({page})=>{
  await open(page);await stageTags(page);
  const first=await page.evaluate(()=>window.tagrex.previewPlan.changes[0].path);
  await holdApply(page);await report(page,{path:first,done:4,total:6,completed:'tags'});
  await expect(page.locator('#tracks-body tr[data-path]').first()).toHaveAttribute('aria-disabled','true');
  await page.evaluate(()=>window.failWrite());
  await expect(page.locator('#mode-col')).not.toHaveAttribute('inert','');
  await expect(page.locator('#tracks-body td[data-field="artist"]').first()).toContainText('Written artist');
  await expect(page.locator('#tracks-body tr[data-path]').first()).toHaveClass(/selected/);
  await expect(page.locator('#tracks-body tr[data-path]').first()).not.toHaveAttribute('aria-disabled','true');
  await expect(page.locator('#files-selected-count')).toHaveText('3');
  await stable(page);
});

test('a combined tag and rename row completes only after both writes, without replacing its cells',async({page})=>{
  await open(page);
  await page.locator('.mode-tab[data-mode="renamer"]').click();await page.locator('#mask').fill('Saved %title%');await page.locator('#preview').click();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const first=await page.evaluate(()=>{
    const change=window.tagrex.previewPlan.changes[0];
    change.tag_changes.push({field:'artist',old:window.tagrex.tracks[0].tags.artist,new:'Written artist'});
    window.tagrex.renderTracks();return change;
  });
  await holdApply(page);
  const row=page.locator('#tracks-body tr[data-path]').first();
  await report(page,{path:first.path,done:4,total:7,completed:'tags'});
  await expect(page.locator('#activity-detail')).toContainText('0 of 3 files');
  await expect(row.locator('td').nth(3)).toContainText('Written artist');
  await expect(row.locator('.fname')).toHaveText(first.path.split('/').pop());
  await expect(row).not.toHaveAttribute('aria-disabled','true');
  await expect(page.locator('#files-selected-count')).toHaveText('3');
  await report(page,{path:first.path,done:5,total:7,completed:'transfer'});
  await expect(page.locator('#activity-detail')).toContainText('1 of 3 files');
  await expect(row.locator('.fname')).toHaveText(first.rename_to.split('/').pop());
  await expect(row).toHaveAttribute('aria-disabled','true');
  await expect(page.locator('#files-selected-count')).toHaveText('2');await stable(page);await finish(page);
});

test('artwork changes update the existing icon only after its write completes',async({page})=>{
  await open(page);
  await page.evaluate(()=>{window.tagrex.selection.clear();window.tagrex.selection.add(window.tagrex.tracks[0].path);window.tagrex.renderTracks();});
  await openTagSource(page, 'editor');
  await page.locator('#cover-well .cover-edit').click();await page.getByRole('menuitem',{name:'Remove',exact:true}).click();
  await expect(page.locator('#diff-apply')).toBeEnabled();await page.locator('#staged-tab').click();
  const first=await page.evaluate(()=>window.tagrex.previewPlan.changes[0]);
  await holdApply(page);
  const art=page.locator('#tracks-body .track-artwork').first();
  await expect(art).toHaveAttribute('data-cover-state','embedded');
  await report(page,{path:first.path,done:2,total:2,completed:'tags'});
  await expect(art).toHaveAttribute('data-cover-state','missing');
  await expect(art).not.toHaveClass(/artwork-changing/);await stable(page);await finish(page);
});

test('large batches allow manual scrolling and remounted rows retain their completed values',async({page})=>{
  await open(page,true);await stageTags(page);
  const changes=await page.evaluate(()=>window.tagrex.previewPlan.changes);
  expect(changes).toHaveLength(400);
  await holdApply(page);
  await page.evaluate(changes=>{for(const change of changes.slice(0,350))window.writeProgress({path:change.path,done:401,total:800,completed:'tags'});},changes);
  await expect(page.locator('#files-selected-count')).toHaveText('50');
  await stable(page);
  const before=await page.locator('#files-view').evaluate(node=>node.scrollTop);
  expect(before).toBe(0);
  await page.locator('#files-view').hover();await page.mouse.wheel(0,4200);
  await expect.poll(()=>page.locator('#files-view').evaluate(node=>node.scrollTop)).toBeGreaterThan(3000);
  await expect(page.locator('#tracks-body tr[data-path]').first()).toHaveAttribute('aria-disabled','true');
  await expect(page.locator('#tracks-body td[data-field="artist"]').first()).toContainText('Written artist');
  await waitForScrollIdle(page.locator('#files-view'));
  const scroll=await page.locator('#files-view').evaluate(node=>node.scrollTop);
  await report(page,{path:changes[399].path,done:799,total:800});
  expect(await page.locator('#files-view').evaluate(node=>node.scrollTop)).toBe(scroll);
  await finish(page);
});

test('Apply highlight transitions honour reduced motion',async({page})=>{
  await open(page);await stageTags(page);await holdApply(page);
  const cell=page.locator('#tracks-body td[data-field="artist"]').first();
  expect(await cell.evaluate(node=>getComputedStyle(node).transitionDuration)).not.toBe('0s');
  await page.emulateMedia({reducedMotion:'reduce'});
  expect(await cell.evaluate(node=>getComputedStyle(node).transitionDuration)).toBe('0s');
  await finish(page);
});

test('Files owns immediate counts and grows for large numbers without clipping',async({page})=>{
  await open(page);
  await expect(page.locator('.command-toolbar #status-sel')).toHaveCount(0);
  await expect(page.locator('#files-total-count')).toHaveText('3');
  await expect(page.locator('#files-selected-count')).toHaveText('3');
  await page.locator('#tracks-body tr[data-path]').first().click();
  await expect(page.locator('#selection-summary')).toBeHidden();
  await page.locator('#tracks-body tr[data-path]').first().click({modifiers:['Control']});
  await expect(page.locator('#selection-summary')).toBeHidden();
  await expect(page.locator('#selection-summary')).toBeHidden();
  await page.locator('#filter').fill('Radio');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(1);
  await expect(page.locator('#files-total-count')).toHaveText('3');
  await expect(page.locator('#files-tab')).toHaveAccessibleDescription(/1\/3/);
  await page.evaluate(()=>{document.getElementById('files-total-count').textContent='999,999,999';document.getElementById('files-selected-count').textContent='123,456,789';});
  const size=await page.locator('#files-tab').evaluate(node=>{const box=node.getBoundingClientRect(),counts=node.querySelector('.files-tab-counts'),content=counts.getBoundingClientRect();return {width:box.width,right:box.right,contentRight:content.right,client:counts.clientWidth,scroll:counts.scrollWidth};});
  expect(size.width).toBeGreaterThan(116);expect(size.contentRight).toBeLessThanOrEqual(size.right-12);expect(size.scroll).toBeLessThanOrEqual(size.client);
  await expect(page.getByRole('tab',{name:'Files',exact:true})).toBeVisible();
});

test('a same-frame rename burst visibly completes top to bottom before the preview closes',async({page})=>{
  await open(page);await page.locator('.mode-tab[data-mode="renamer"]').click();await page.locator('#mask').fill('Fast %title%');await page.locator('#preview').click();
  await expect(page.locator('#diff-apply')).toBeEnabled();const changes=await page.evaluate(()=>window.tagrex.previewPlan.changes);await holdApply(page);
  await page.evaluate(changes=>{
    window.burstFrames=[];window.recordBurst=true;
    const sample=()=>{if(!window.recordBurst)return;window.burstFrames.push([...document.querySelectorAll('#tracks-body tr.apply-complete')].map(row=>row.dataset.path));requestAnimationFrame(sample);};
    requestAnimationFrame(sample);
    for(const change of changes)window.writeProgress({path:change.path,done:changes.length*2,total:changes.length*2,completed:'transfer'});
    window.finishWrite();
  },changes);
  await expect(page.locator('#diff-actionbar')).toBeVisible();await expect(page.locator('#diff-apply')).toBeDisabled();
  await expect(page.locator('#diff-apply')).not.toHaveAttribute('aria-busy');
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes?.length || 0)).toBe(0);
  const frames=await page.evaluate(()=>{window.recordBurst=false;window.progressObserver.disconnect();return window.burstFrames;});
  expect(frames.some(paths=>paths.length>0&&paths.length<changes.length)).toBe(true);
  for(const paths of frames)expect(paths).toEqual(changes.slice(0,paths.length).map(change=>change.path));
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes?.length || 0)).toBe(0);
  expect(await page.evaluate(()=>window.tagrex.selectedPaths())).toEqual([]);
});

for (const theme of ['light', 'dark']) test(`staged surface grows into activity, holds confirmed success then collapses (${theme})`, async ({ page }, info) => {
  await open(page); await stageTags(page);
  await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
  const card = page.locator('#staged-edits');
  const before = await card.boundingBox(), action = await page.locator('#diff-apply').boundingBox();
  await holdApply(page);
  await expect(page.locator('#activity-title')).toHaveText('Checking files…');
  await expect(page.locator('#activity-meter')).not.toHaveAttribute('aria-valuenow');
  // Stop must remain reachable even though all editable inspector panels freeze.
  await page.locator('#activity-stop').focus(); await expect(page.locator('#activity-stop')).toBeFocused();
  await expect.poll(() => card.evaluate(node => node.getAnimations().length)).toBe(0);
  expect((await card.boundingBox()).height).toBeGreaterThan(before.height);
  expect(await page.locator('#diff-apply').boundingBox()).toEqual(action);
  const changes = await page.evaluate(() => window.tagrex.previewPlan.changes);
  await report(page, {path: changes[0].path, done:4, total:6, completed:'tags'});
  await expect(page.locator('#activity-title')).toHaveText('Applying changes…');
  await expect(page.locator('#activity-percent')).toHaveText('33%');
  await expect(page.locator('#diff-actionbar [role="progressbar"], #diff-actionbar #diff-cancel-apply')).toHaveCount(0);
  const meter = await page.locator('#activity-meter').boundingBox();
  const header = await page.locator('.activity-head').boundingBox();
  expect(meter.x).toBeCloseTo(header.x, 0);
  expect(meter.width).toBeCloseTo(header.width, 0);
  expect((await page.locator('#activity-detail').boundingBox()).y).toBeGreaterThan(meter.y + meter.height);
  await expect(page.locator('#activity-detail')).toContainText('1 of 3 files');
  await card.screenshot({path:info.outputPath(`activity-${theme}.png`)});
  await finish(page);
  await expect(page.locator('#activity-title')).toHaveText('Changes saved');
  await expect(page.locator('#activity-percent')).toHaveText('100%');
  // Capture the hold with actual motion, then verify that it exits autonomously.
  await page.clock.install(); await page.clock.runFor(900);
  await expect(card).toBeVisible();
  await page.clock.resume();
  await expect(card).toBeHidden();
});

test('activity coalesces bursts, counts combined writes once and never reports premature success', async ({ page }) => {
  await open(page); await stageTags(page); await holdApply(page);
  const first = await page.evaluate(() => window.tagrex.previewPlan.changes[0].path);
  await page.evaluate(first => {
    window.activityPaints = 0;
    const observer = new MutationObserver(records => window.activityPaints += records.length);
    observer.observe(document.getElementById('activity-fill'), {attributes:true});
    window.activityObserver = observer;
    for (let i=0; i<200; i++) window.writeProgress({path:first, done:4, total:6, completed:'tags'});
  }, first);
  await expect(page.locator('#activity-detail')).toContainText('1 of 3 files');
  expect(await page.evaluate(() => window.activityPaints)).toBeLessThan(5);
  await report(page, {path:first, done:6, total:6, completed:'tags'});
  await expect(page.locator('#activity-percent')).toHaveText('99%');
  await expect(page.locator('#activity-title')).toHaveText('Applying changes…');
  await stable(page); await finish(page);
});

test('failed activity persists and dismissal restores pending work; the next Apply supersedes it', async ({ page }) => {
  await open(page); await stageTags(page); await holdApply(page);
  await page.evaluate(() => window.failWrite('error.io'));
  await expect(page.locator('#staged-edits')).toHaveAttribute('data-activity','error');
  await expect(page.locator('#activity-dismiss')).toBeVisible();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.clock.install(); await page.clock.runFor(4000); await page.clock.resume();
  await expect(page.locator('#staged-edits')).toBeVisible();
  await page.locator('#activity-dismiss').click();
  await expect(page.locator('#staged-draft')).toBeVisible();
  await expect(page.locator('#staged-count')).toHaveText('3 files');
  await holdApply(page); await finish(page);
  await expect(page.locator('#activity-title')).toHaveText('Changes saved');
});

test('reduced motion preserves the completion hold without spatial animations', async ({ page }) => {
  await page.emulateMedia({reducedMotion:'reduce'});
  await open(page); await stageTags(page); await holdApply(page);
  const activeAnimations = await page.locator('#staged-edits').evaluate(node => node.getAnimations({subtree:true}).map(animation => ({
    type: animation.constructor.name,
    state: animation.playState,
    name: animation.animationName,
    property: animation.transitionProperty,
    target: animation.effect?.target?.id,
  })));
  expect(activeAnimations).toEqual([]);
  await finish(page); await expect(page.locator('#activity-title')).toHaveText('Changes saved');
  await expect(page.locator('#staged-edits')).toBeHidden();
});

test('Stop remains keyboard usable and progress cannot overwrite rollback feedback', async ({ page }) => {
  await open(page); await stageTags(page); await holdApply(page);
  await page.locator('#activity-stop').focus(); await page.keyboard.press('Enter');
  await expect(page.locator('#activity-title')).toHaveText('Cancelling…');
  await expect(page.locator('#activity-stop')).toBeDisabled();
  const first = await page.evaluate(() => window.tagrex.previewPlan.changes[0].path);
  await report(page, {path:first, done:4, total:6, completed:'tags'});
  await expect(page.locator('#activity-title')).toHaveText('Cancelling…');
  await page.evaluate(() => window.failWrite());
  await expect(page.locator('#activity-title')).toHaveText('Apply cancelled');
  await page.locator('#activity-dismiss').click();
  await expect(page.locator('#staged-draft')).toBeVisible();
  await expect(page.locator('#diff-apply')).toBeEnabled();
});

test('newly staged work survives the completion hold and returns as a draft', async ({ page }) => {
  await open(page); await stageTags(page); await holdApply(page); await finish(page);
  await expect(page.locator('#activity-title')).toHaveText('Changes saved');
  await page.evaluate(() => {
    window.tagrex.selection.clear();
    window.tagrex.selection.add(window.tagrex.tracks[0].path);
    window.tagrex.renderTracks();
  });
  await openTagSource(page, 'editor');
  await page.locator('#fields-body [data-key="artist"]').fill('A fresh draft');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await expect(page.locator('#staged-edits')).not.toHaveAttribute('data-activity');
  await expect(page.locator('#staged-draft')).toBeVisible();
  await expect(page.locator('#staged-count')).toHaveText('1 file');
});

test('a new Apply supersedes the previous success timer without losing the activity surface', async ({ page }) => {
  await open(page); await stageTags(page); await holdApply(page);
  await page.clock.install(); await finish(page);
  await page.evaluate(() => {
    window.tagrex.selection.clear();
    for (const track of window.tagrex.tracks) window.tagrex.selection.add(track.path);
    window.tagrex.renderTracks();
  });
  await openTagSource(page, 'editor');
  await page.locator('#fields-body [data-key="artist"]').fill('Another artist');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await holdApply(page);
  await page.clock.runFor(1800); await page.clock.resume();
  await expect(page.locator('#staged-edits')).toBeVisible();
  await expect(page.locator('#activity-title')).toHaveText('Checking files…');
  await finish(page);
});

test('compact layouts reveal the activity and keep Stop within reach', async ({ page }) => {
  await page.setViewportSize({width:640,height:700});
  await open(page);
  await openTagSource(page, 'editor');
  await page.locator('#fields-body [data-key="artist"]').fill('Written artist');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await holdApply(page);
  await expect(page.locator('#staged-edits')).toBeVisible();
  const stop = await page.locator('#activity-stop').boundingBox();
  expect(stop.x).toBeGreaterThanOrEqual(0); expect(stop.x + stop.width).toBeLessThanOrEqual(640);
  expect(stop.y + stop.height).toBeLessThanOrEqual(700);
  await page.locator('#activity-stop').focus(); await expect(page.locator('#activity-stop')).toBeFocused();
  await finish(page);
});

test('a rename, its Undo, and its Redo all use the same activity card', async ({ page }) => {
  await open(page);
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await page.locator('#mask').fill('Renamed %title%');
  await page.locator('#preview').click();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.locator('#diff-apply').click();
  await expect(page.locator('#activity-title')).toHaveText('Changes saved');
  await expect(page.locator('#undo')).toBeEnabled();

  await page.evaluate(() => {
    window.historyCard = document.getElementById('staged-edits');
    window.__TAURI__ = { event: { listen: async (_event, callback) => {
      window.writeProgress = payload => callback({ payload });
      return () => { window.writeProgress = null; };
    } } };
    const resolve = Promise.resolve;
    Promise.resolve = function (value) {
      if ((window.holdHistory === 'undo' && Array.isArray(value?.kept)) ||
          (window.holdHistory === 'redo' && value?.id && 'applied_at' in value)) {
        return new Promise(done => { window.finishHistory = () => done(value); });
      }
      return resolve.call(this, value);
    };
  });

  for (const [operation, title, success] of [
    ['undo', 'Undoing…', 'Undid the last batch.'],
    ['redo', 'Redoing…', 'Redid last batch'],
  ]) {
    await page.evaluate(operation => { window.holdHistory = operation; window.finishHistory = null; }, operation);
    await page.locator(`#${operation}`).click();
    await expect(page.locator('#activity-title')).toHaveText(title);
    await expect(page.locator('#activity-stop')).toBeHidden();
    await expect(page.locator('#busy-pill')).toBeHidden();
    await expect(page.locator('#activity-meter')).not.toHaveAttribute('aria-valuenow');
    await expect.poll(() => page.evaluate(() => typeof window.finishHistory)).toBe('function');
    await report(page, { path: '/music/Renamed fixture.mp3', done: 1, total: 2 });
    await expect(page.locator('#activity-percent')).toHaveText('50%');
    await expect(page.locator('#activity-detail')).toHaveText('Renamed fixture.mp3');
    await report(page, { path: '/music/Renamed fixture.mp3', done: 2, total: 2 });
    await expect(page.locator('#activity-percent')).toHaveText('99%');
    await page.evaluate(() => window.finishHistory());
    await expect(page.locator('#activity-title')).toHaveText(success);
    await expect(page.locator('#activity-percent')).toHaveText('100%');
    expect(await page.locator('#staged-edits').evaluate(card => card === window.historyCard)).toBe(true);
  }
  await expect(page.locator('#staged-edits')).toBeHidden();
});

test('the meter stays animated between file writes without moving the confirmed percentage', async ({ page }) => {
  await open(page); await stageTags(page); await holdApply(page);
  const path = await page.evaluate(() => window.tagrex.previewPlan.changes[0].path);
  const sample = () => page.locator('#activity-meter').evaluate(node => {
    const animation = node.getAnimations({subtree:true}).find(player => player.animationName === 'activity-glide');
    if (!animation) return null;
    return {time:animation.currentTime, progress:node.getAttribute('aria-valuenow')};
  });
  await expect.poll(sample).toMatchObject({progress:null});
  await report(page, {path, done:4, total:6, completed:'tags'});
  await expect(page.locator('#activity-meter')).toHaveAttribute('aria-valuenow','33');
  const start = await sample();
  expect(start).toMatchObject({progress:'33'});
  await expect.poll(async () => (await sample())?.time).toBeGreaterThan(start.time);
  expect((await sample()).progress).toBe('33');
  await finish(page);
  await expect.poll(sample).toBeNull();
});
