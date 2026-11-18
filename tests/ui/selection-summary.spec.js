import {test,expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
const root=resolve('.test-dist');
test.use({reducedMotion:'no-preference'});
test.beforeEach(async({page})=>{
 page.summaryErrors=[];page.on('pageerror',e=>page.summaryErrors.push(e.message));
 await page.route('http://tagrex.test/**',async route=>{
  const path=resolve(root,'.'+new URL(route.request().url()).pathname);if(!path.startsWith(root+sep))return route.abort();
  try{await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});}catch{await route.fulfill({status:404});}
 });
 await page.goto('http://tagrex.test/app/ui/index.html');await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
 await page.evaluate(async()=>{document.getElementById('root').value='/music';await window.tagrex.openLibrary();});
});
test.afterEach(async({page})=>expect(page.summaryErrors).toEqual([]));
async function select(page,count){await page.evaluate(count=>{window.tagrex.selection.clear();for(const track of window.tagrex.tracks.slice(0,count))window.tagrex.selection.add(track.path);window.tagrex.renderTracks();},count);}
const summary=page=>page.locator('#selection-summary');
async function settled(page){await expect.poll(()=>summary(page).evaluate(n=>n.getAnimations().length)).toBe(0);}
test('tabs show file counts only and the multi-selection pill does not resize the table',async({page})=>{
 await expect(page.locator('.files-tab-counts')).toHaveText('3');await expect(summary(page)).toBeHidden();
 const table=await page.locator('#files-view').boundingBox();await select(page,2);
 await expect(summary(page)).toBeVisible();await expect(page.locator('#files-selected-count')).toHaveText('2');await settled(page);
 const after=await page.locator('#files-view').boundingBox();expect(after).toEqual(table);
 const pane=await page.locator('.files-col').boundingBox(),pill=await summary(page).boundingBox();expect(pill.x+pill.width/2).toBeCloseTo(pane.x+pane.width/2,0);
 await page.locator('#toggle-groups').click();await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(0);await expect(summary(page)).toBeVisible();await expect(summary(page)).not.toContainText('hidden');
 await page.locator('#selection-summary-clear').click();await expect(summary(page)).toBeHidden();await expect(page.locator('#files-total-count')).toHaveText('3');
 expect(await page.evaluate(()=>window.tagrex.selection.size)).toBe(0);await expect(page.locator('#tracks')).toBeFocused();
});
test('filtering preserves selection without hidden-file notices',async({page})=>{
 await page.locator('#filter').fill('Wish Mountain');await expect(summary(page)).toBeHidden();
 await expect(page.locator('#filter-feedback')).toBeHidden();
 expect(await page.evaluate(()=>window.tagrex.selection.size)).toBe(1);
 await page.locator('#filter-clear').click();await expect(summary(page)).toBeHidden();
 await select(page,2);await page.locator('#filter').fill('zz-no-match');await expect(summary(page)).toContainText('2 selected');await expect(summary(page)).not.toContainText('hidden');
 await expect(page.locator('#selection-summary-detail')).toBeEmpty();
 await expect(page.locator('#filter-feedback')).toContainText('No files match');
 await expect(page.locator('.query-hidden-selection')).toHaveCount(0);
 await page.locator('#selection-summary-clear').click();await expect(summary(page)).toBeHidden();
});
test('Staged distinguishes included scope and clearing selection never discards edits',async({page})=>{
 await select(page,2);const original=await page.evaluate(()=>window.tagrex.tracks.map(t=>({...t.tags})));
 await page.locator('.mode-tab[data-mode="renamer"]').click();await page.locator('#mask').fill('New %title%');await page.locator('#preview').click();
 await expect(page.locator('#staged-tab-count')).toHaveText('2');
 await page.locator('#staged-tab').click();await expect(summary(page)).toContainText('included for Apply');await expect(page.locator('#files-selected-count')).toHaveText('2');
 await page.locator('#selection-summary-clear').click();await expect(summary(page)).toBeHidden();await expect(page.locator('#staged-tab-count')).toHaveText('2');await expect(page.locator('#diff-apply')).toBeDisabled();
 expect(await page.evaluate(()=>window.tagrex.tracks.map(t=>({...t.tags})))).toEqual(original);
 expect(await page.evaluate(()=>window.tagrex.selection.size)).toBe(2);
});
test('rapid reversals settle without stale hiding or animation replay',async({page})=>{
 await select(page,2);await expect(summary(page)).toBeVisible();
 await select(page,0);await page.evaluate(()=>new Promise(requestAnimationFrame));await select(page,3);
 await expect(page.locator('#files-selected-count')).toHaveText('3');await settled(page);await expect(summary(page)).toBeVisible();
 await page.evaluate(()=>window.tagrex.renderTracks());await page.evaluate(()=>new Promise(requestAnimationFrame));expect(await summary(page).evaluate(n=>n.getAnimations().length)).toBe(0);
});
test('width growth is centred and spring entry changes only the overlay',async({page})=>{
 await select(page,2);await page.locator('.mode-tab[data-mode="renamer"]').click();await page.locator('#mask').fill('New %title%');await page.locator('#preview').click();
 await expect(page.locator('#staged-tab-count')).toHaveText('2');await page.locator('#files-tab').click();
 await settled(page);const before=await page.locator('#files-view').boundingBox();
 await page.locator('#staged-tab').click();await expect(summary(page)).toContainText('included for Apply');
 const frames=await page.evaluate(async()=>{
  const samples=[];for(let i=0;i<18;i++){await new Promise(requestAnimationFrame);const p=document.querySelector('.files-col').getBoundingClientRect(),n=document.getElementById('selection-summary'),r=n.getBoundingClientRect(),t=document.getElementById('files-view').getBoundingClientRect();samples.push({width:r.width,offset:r.left+r.width/2-p.left-p.width/2,table:{x:t.x,y:t.y,width:t.width,height:t.height},effects:n.getAnimations().map(a=>Object.keys(a.effect.getKeyframes()[0]))});}return samples;
 });
 expect(frames.every(f=>Math.abs(f.offset)<1)).toBe(true);expect(frames.every(f=>Math.abs(f.table.height-before.height)<1&&Math.abs(f.table.y-before.y)<1)).toBe(true);
 expect(new Set(frames.map(f=>Math.round(f.width))).size).toBeGreaterThan(1);await settled(page);
});
test('reduced motion skips spring and width animation',async({page})=>{
 await page.emulateMedia({reducedMotion:'reduce'});await select(page,2);await expect(summary(page)).toBeVisible();expect(await summary(page).evaluate(n=>n.getAnimations().length)).toBe(0);
 await page.locator('#selection-summary-clear').click();await expect(summary(page)).toBeHidden();
});
