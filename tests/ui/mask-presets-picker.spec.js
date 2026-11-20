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

async function open(page) {
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await page.locator('[data-preset-target="mask"]').click();
  await expect(page.locator('#preset-filter')).toBeFocused();
}
async function save(page, name, pattern) {
  await page.locator('#mask').evaluate((node,value)=>{node.value=value;node.dispatchEvent(new Event('input',{bubbles:true}));},pattern);
  await page.locator('#preset-new').click();
  await page.locator('#preset-name').fill(name);
  await page.locator('#preset-name').press('Enter');
  await expect(page.locator('#preset-save-form')).toBeHidden();
}
test('saved masks show chips, search by name or raw mask, and apply exact syntax by keyboard',async({page})=>{
  await open(page);
  await expect(page.locator('#preset-save-form')).toBeHidden();
  await expect(page.locator('.preset-heading')).toHaveCount(0);
  await expect(page.locator('#preset-filter')).toHaveAttribute('placeholder','Search presets');
  await expect(page.locator('#preset-new')).toHaveText('Save preset');
  const pattern="$upper(%artist%) / [%track:3%] - %title% - %unknown%";
  await save(page,'Album folders',pattern);
  await save(page,'Short title','%title%');
  const short = page.locator('.preset-list .preset-row').filter({hasText:'Short title'});
  expect(await short.evaluate(node=>node.getBoundingClientRect().height)).toBeLessThanOrEqual(48);
  await expect(short.locator('.preset-del use')).toHaveAttribute('href','#i-close');
  const centre = await page.locator('#preset-new').evaluate(button=>{
    const b=button.getBoundingClientRect(),f=button.parentElement.getBoundingClientRect();
    return Math.abs((b.left+b.right-f.left-f.right)/2);
  });
  expect(centre).toBeLessThan(1);
  await page.locator('[data-preset-target="mask"]').click();
  await page.locator('[data-preset-target="mask"]').click();
  await page.locator('#preset-filter').fill('album folders');
  await expect(page.locator('.preset-list .preset-row')).toHaveCount(1);
  await expect(page.locator('.preset-list .field-value-chip')).toHaveText(['Uppercase · Artist','Track · 3','Title']);
  await expect(page.locator('.preset-list .preset-mask')).toContainText('%unknown%');
  await page.locator('#preset-filter').fill('%track:3%');
  await expect(page.locator('.preset-list .preset-row')).toHaveCount(1);
  await page.locator('#preset-filter').press('ArrowDown');
  await expect(page.locator('.preset-list .preset-apply')).toBeFocused();
  await page.locator('.preset-list .preset-apply').press('Enter');
  await expect(page.locator('#preset-menu')).toBeHidden();
  expect(await page.locator('#mask').evaluate(node=>node.value)).toBe(pattern);
  await page.locator('[data-preset-target="mask"]').click();
  await expect(page.locator('.preset-list [aria-current="true"] .preset-name')).toHaveText('Album folders');
  await page.locator('#preset-filter').fill('not found');
  await expect(page.locator('.preset-empty')).toHaveText('No matching saved masks.');
  await page.locator('#preset-filter').press('Escape');
  await expect(page.locator('#preset-menu')).toBeHidden();
  await expect(page.locator('[data-preset-target="mask"]')).toBeFocused();
});
test('save cancellation, update and deletion keep the draft and library untouched',async({page})=>{
  await open(page);
  const before=await page.evaluate(()=>({tracks:structuredClone(window.tagrex.tracks),selected:window.tagrex.selectedPaths(),plan:window.tagrex.previewPlan}));
  await save(page,'Keep this','%artist% - %title%');
  await page.locator('#mask').evaluate(node=>{node.value='%album% / %title%';node.dispatchEvent(new Event('input',{bubbles:true}));});
  await page.locator('#preset-new').click();
  await expect(page.locator('#preset-name')).toHaveValue('Keep this');
  await expect(page.locator('#preset-save')).toHaveText('Update');
  await page.locator('#preset-save-cancel').click();
  await expect(page.locator('#preset-new')).toBeFocused();
  await page.locator('#preset-new').click();
  await page.locator('#preset-save').click();
  await expect(page.locator('.preset-list .preset-row')).toHaveAttribute('data-mask','%album% / %title%');
  await page.locator('.preset-list .preset-del').click();
  await expect(page.locator('.preset-list .preset-row')).toHaveCount(0);
  await expect(page.locator('#preset-filter')).toBeFocused();
  expect(await page.locator('#mask').evaluate(node=>node.value)).toBe('%album% / %title%');
  expect(await page.evaluate(()=>({tracks:window.tagrex.tracks,selected:window.tagrex.selectedPaths(),plan:window.tagrex.previewPlan}))).toEqual(before);
});
test('long masks wrap, lists scroll with overlay rails, and save stays reachable in either theme',async({page})=>{
  await open(page);
  for(let i=0;i<7;i++)await save(page,`A very long saved mask name ${i}`, '%albumartist% / %album% / %disc:2% - %track:3% - %artist% - %title%');
  for(const theme of ['light','dark']) {
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    await page.setViewportSize({width:480,height:600});
    await expect(page.locator('.preset-search')).toHaveCSS('background-color',await page.locator('.mb-ref-search').evaluate(node=>getComputedStyle(node).backgroundColor));
    const bounds=await page.locator('#preset-menu').evaluate(menu=>{
      const list=document.getElementById('preset-list'),r=menu.getBoundingClientRect();
      return {left:r.left,right:r.right,bottom:r.bottom,width:innerWidth,height:innerHeight,listWidth:list.clientWidth,contentWidth:list.scrollWidth,scrolls:list.scrollHeight>list.clientHeight};
    });
    expect(bounds.left).toBeGreaterThanOrEqual(7);expect(bounds.right).toBeLessThanOrEqual(bounds.width-7);expect(bounds.bottom).toBeLessThanOrEqual(bounds.height-7);
    expect(bounds.contentWidth).toBeLessThanOrEqual(bounds.listWidth);expect(bounds.scrolls).toBe(true);
    await expect(page.locator('.preset-list-wrap .inspector-scrollbar.is-vertical')).toBeVisible();
    await expect(page.locator('#preset-new')).toBeInViewport();
    await page.locator('#preset-new').click();
    await expect(page.locator('#preset-save')).toBeInViewport();
    await page.locator('#preset-save-cancel').click();
  }
  await page.keyboard.press('Escape');
  await page.locator('#mask-builder-open').click();
  await expect(page.locator('#mb-transform')).toHaveCount(0);
  await expect(page.locator('#mb-list table')).toBeVisible();
});
