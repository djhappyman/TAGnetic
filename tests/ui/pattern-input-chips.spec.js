import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
import { openTagSource } from './navigation.js';
const root = resolve('.test-dist');
test.beforeEach(async ({page}) => {
  page.issues=[]; page.on('pageerror',e=>page.issues.push(String(e)));
  await page.route('http://tagrex.test/**',async route=>{
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);
    if(!path.startsWith(root+sep))return route.abort();
    try { await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'}); }
    catch { await route.fulfill({status:404,body:'Missing fixture'}); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async()=>{document.getElementById('root').value='/music';await window.tagrex.openLibrary();});
});
test.afterEach(async({page})=>expect(page.issues).toEqual([]));
async function raw(page,id,value) {await page.locator('#'+id).evaluate((node,value)=>{node.value=value;node.dispatchEvent(new Event('input',{bubbles:true}));},value);}
async function choose(page,id,word,label) {
  await page.locator('#'+id).fill(word);
  await expect(page.locator('#'+id+'-suggestions').getByRole('option',{name:label,exact:true})).toBeVisible();
  await page.locator('#'+id).press('Enter');
  await expect(page.locator('#'+id+' [data-pattern]')).toHaveText(label);
}
test('all eight controller inputs round-trip advanced patterns and pasted tokens as chips',async({page})=>{
  const ids=['mask','from-name-mask','from-text-mask','export-mask','colmask-pattern','mb-expr','artwork-import-pattern'];
  for(const id of ids) {
    const value='$upper(%artist%) / [%track:3%] - %unknown%';
    await raw(page,id,value);
    await expect(page.locator('#'+id+' [data-pattern]')).toHaveCount(2);
    expect(await page.locator('#'+id).evaluate(n=>n.value)).toBe(value);
  }
  await page.locator('.mode-tab[data-mode="exporter"]').click();
  await page.locator('#export-split').selectOption('album');
  await expect(page.locator('#export-name [data-pattern]')).toHaveText('Album');
  expect(await page.locator('#export-name').evaluate(n=>n.value)).toBe('%album%.m3u');
  await page.locator('#export-split').selectOption('');
  await raw(page,'export-name','%artist%.m3u');
  await expect(page.locator('#export-name [data-pattern]')).toHaveCount(0);
});
test('rename suggestions, atomic Delete and local Undo preserve the library and staged state',async({page})=>{
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  const before=await page.evaluate(()=>({tracks:structuredClone(window.tagrex.tracks),selected:window.tagrex.selectedPaths(),plan:window.tagrex.previewPlan}));
  await choose(page,'mask','artist','Artist');
  await page.locator('#mask').press('Backspace');
  await expect(page.locator('#mask [data-pattern]')).toHaveCount(0);
  await page.locator('#mask').press('ControlOrMeta+z');
  await expect(page.locator('#mask [data-pattern]')).toHaveText('Artist');
  await page.locator('#mask').evaluate(node=>{node.focus();node.select();node.dispatchEvent(new ClipboardEvent('paste',{bubbles:true,cancelable:true,clipboardData:(()=>{const d=new DataTransfer();d.setData('text/plain','%artist% - %title%');return d;})()}));});
  await expect(page.locator('#mask [data-pattern]')).toHaveCount(2);
  const after=await page.evaluate(()=>({tracks:window.tagrex.tracks,selected:window.tagrex.selectedPaths(),plan:window.tagrex.previewPlan}));
  expect(after).toEqual(before);
});
test('From Name and From Text offer extraction fields and detected patterns render as chips',async({page})=>{
  for(const [source,id] of [['filename','from-name-mask'],['fromtext','from-text-mask']]) {
    await openTagSource(page,source);
    await choose(page,id,'skip','Skip');
    await page.locator('#'+id).fill('filename');
    await expect(page.locator('#'+id+'-suggestions')).toHaveCount(0);
    await page.locator('#'+id).fill('upper');
    await expect(page.locator('#'+id+'-suggestions')).toHaveCount(0);
  }
  await page.locator('#from-text-lines').fill('01. Artist - Title');
  await page.locator('#from-text-suggest').click();
  await expect(page.locator('#from-text-mask [data-pattern]')).toHaveCount(3);
});
test('builder suggestion Enter never accepts and reference inserts at the raw caret',async({page})=>{
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await page.locator('#mask-builder-open').click();
  await choose(page,'mb-expr','artist','Artist');
  await expect(page.locator('#mask-builder')).toBeVisible();
  await raw(page,'mb-expr','%artist% - %title%');
  await page.locator('#mb-expr').evaluate(n=>{n.focus();n.setSelectionRange(11,11);});
  await page.locator('#mb-ref-body .ph-row[data-token="%track%"] ').click();
  expect(await page.locator('#mb-expr').evaluate(n=>n.value)).toBe('%artist% - %track%%title%');
  await page.locator('#mb-cancel').click();
  await expect(page.locator('#mask-builder')).toBeHidden();
});
test('focused chips and field tools remain reachable in both themes at narrow width',async({page})=>{
  await page.setViewportSize({width:720,height:800});
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  for(const theme of ['light','dark']) {
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    await raw(page,'mask','%artist% - %title%');
    await page.locator('#mask').focus();
    const box=await page.locator('#mask').boundingBox();
    expect(box.width).toBeGreaterThan(90);expect(box.x+box.width).toBeLessThanOrEqual(721);
    await expect(page.locator('#mask-builder-open')).toBeVisible();
  }
});
test('presets refresh chips and builder Clear can be undone without changing the source pattern',async({page})=>{
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await page.locator('[data-preset-target="mask"]').click();
  await page.locator('#preset-name').fill('Chip regression');
  await page.locator('#preset-save').click();
  await page.locator('#preset-menu .preset-apply').first().click();
  const original=await page.locator('#mask').evaluate(n=>n.value);
  await expect(page.locator('#mask [data-pattern]')).not.toHaveCount(0);
  await page.locator('#mask-builder-open').click();
  await page.locator('#mb-clear').click();
  await expect.poll(()=>page.locator('#mb-expr').evaluate(n=>n.value)).toBe('');
  await expect(page.locator('#mb-clear')).toBeHidden();
  await page.locator('#mb-expr').press('ControlOrMeta+z');
  await expect.poll(()=>page.locator('#mb-expr').evaluate(n=>n.value)).toBe(original);
  await page.locator('#mb-cancel').click();
  expect(await page.locator('#mask').evaluate(n=>n.value)).toBe(original);
});
