import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
import { openTagSource } from './navigation.js';
const root = resolve('.test-dist');
test.beforeEach(async ({page}) => {
  page.issues=[];
  page.on('pageerror',error=>page.issues.push(String(error)));
  await page.addInitScript(()=>{window.searchCalls=[];window.captureSearch=args=>{window.searchCalls.push(args);return Promise.resolve([]);};});
  await page.route('http://tagrex.test/**', async route => {
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);
    if(!path.startsWith(root+sep))return route.abort();
    try {
      let body=await readFile(path);
      if(path.endsWith('.js'))body=Buffer.from(body.toString().replace(/case\s*["'\x60]provider_search["'\x60]:/g,'case "provider_search": return window.captureSearch(arguments[1]);'));
      await route.fulfill({body,contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});
    } catch { await route.fulfill({status:404,body:'Missing fixture'}); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await openTagSource(page,'online');
  await page.locator('#discogs-token').evaluate(node=>{node.value='fixture';});
});
test.afterEach(async({page})=>expect(page.issues).toEqual([]));
async function addFilter(page, field='Label', value='React', keyboard=false) {
  await page.locator('#discogs-query').fill(field);
  const option=page.getByRole('option',{name:field,exact:true});
  await expect(option).toBeVisible();
  if(keyboard)await page.locator('#discogs-query').press('Enter');else await option.click();
  await expect(page.getByRole('dialog',{name:`${field} filter`})).toBeVisible();
  await expect(page.locator('#discogs-search')).toBeDisabled();
  await page.locator('#online-filter-value').fill(value);
  if(keyboard)await page.locator('#online-filter-value').press('Enter');else await page.getByRole('button',{name:'OK',exact:true}).click();
  await expect(page.getByRole('dialog',{name:`${field} filter`})).toHaveCount(0);
  await expect(page.locator('#online-filter-chip')).toHaveText(value);
}

test('label value becomes the chip, border persists on blur and search keeps the filter separate',async({page})=>{
  await addFilter(page);
  const colours=await page.locator('#query-box').evaluate(node=>({border:getComputedStyle(node).borderColor,ink:getComputedStyle(node.querySelector('#online-filter-chip')).color}));
  expect(colours.border).toBe(colours.ink);
  await page.locator('#discogs-query').fill('Bonkers');
  await page.locator('#discogs-search').click();
  await expect.poll(()=>page.evaluate(()=>window.searchCalls.length)).toBe(1);
  expect(await page.evaluate(()=>window.searchCalls[0].query)).toMatchObject({album:'Bonkers',format:null,filters:{label:'React'},page:1});
  await expect(page.locator('#query-box')).toHaveClass(/search-filtered/);
  expect(await page.locator('#query-box').evaluate(node=>getComputedStyle(node).borderColor)).toBe(colours.border);
});
test('keyboard confirmation replaces one filter and Escape leaves the original intact',async({page})=>{
  await addFilter(page,'Label','React',true);
  await page.locator('#online-filter-chip').click();
  await page.locator('#online-filter-value').fill('Wrong label');
  await page.locator('#online-filter-value').press('Escape');
  await expect(page.locator('#online-filter-chip')).toHaveText('React');
  await addFilter(page,'Artist','Hixxy',true);
  await expect(page.locator('#online-filter-chip')).toHaveCount(1);
  await page.locator('#discogs-query').fill('Bonkers');await page.locator('#discogs-query').press('Enter');
  await expect.poll(()=>page.evaluate(()=>window.searchCalls.length)).toBe(1);
  expect(await page.evaluate(()=>window.searchCalls[0].query.filters)).toEqual({artist:'Hixxy'});
});
test('sources restore independent drafts and hide unsupported field suggestions',async({page})=>{
  await addFilter(page);await page.locator('#discogs-query').fill('Bonkers');
  await page.locator('#online-source').selectOption('bandcamp');
  await expect(page.locator('#online-filter-chip')).toHaveCount(0);
  await expect(page.locator('#discogs-query')).toHaveValue('');
  await page.locator('#discogs-query').fill('label');
  await expect(page.getByRole('listbox',{name:'Search field suggestions'})).toHaveCount(0);
  await page.locator('#online-source').selectOption('discogs');
  await expect(page.locator('#discogs-query')).toHaveValue('Bonkers');
  await expect(page.locator('#online-filter-chip')).toHaveText('React');
});
test('empty values cannot confirm and clearing removes both the chip and its border',async({page})=>{
  await page.locator('#discogs-query').fill('label');await page.getByRole('option',{name:'Label',exact:true}).click();
  await expect(page.getByRole('button',{name:'OK',exact:true})).toBeDisabled();
  await page.locator('#online-filter-value').fill('   ');await expect(page.getByRole('button',{name:'OK',exact:true})).toBeDisabled();
  await page.locator('#online-filter-value').fill('React');await page.locator('#online-filter-value').press('Enter');
  await page.locator('#query-box').hover();await page.locator('#discogs-clear').click();
  await expect(page.locator('#online-filter-chip')).toHaveCount(0);await expect(page.locator('#query-box')).not.toHaveClass(/search-filtered/);
  await expect(page.locator('#discogs-query')).toHaveValue('');
});
test('chip Backspace and IME remain local editing operations',async({page})=>{
  await addFilter(page);await page.locator('#discogs-query').press('Backspace');
  await expect(page.locator('#online-filter-chip')).toHaveCount(0);
  await page.locator('#discogs-query').evaluate(node=>{node.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}));node.value='label';node.dispatchEvent(new Event('input',{bubbles:true}));node.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',isComposing:true,bubbles:true}));});
  expect(await page.evaluate(()=>window.searchCalls.length)).toBe(0);
  await expect(page.getByRole('dialog')).toHaveCount(0);
});
test('active filter remains reachable in both themes and narrow layout',async({page})=>{
  await page.setViewportSize({width:740,height:780});await addFilter(page);
  for(const theme of ['light','dark']){
    await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;document.documentElement.style.colorScheme=theme;},theme);
    await expect(page.locator('#discogs-query')).toBeVisible();await expect(page.locator('#online-filter-chip')).toBeVisible();
    await page.screenshot({path:test.info().outputPath(`online-filter-${theme}.png`)});
  }
});

test('chip shortcuts never invoke library Undo or trash and pending input can be dismissed from the search field',async({page})=>{
  await addFilter(page);
  await page.evaluate(()=>{window.libraryActions=[];for(const id of ['undo','diff-discard'])document.getElementById(id)?.addEventListener('click',()=>window.libraryActions.push(id));});
  await page.locator('#online-filter-chip').focus();await page.keyboard.press('Delete');
  await expect(page.locator('#online-filter-chip')).toHaveCount(0);
  await page.locator('#discogs-query').press('ControlOrMeta+z');
  await expect(page.locator('#online-filter-chip')).toHaveText('React');
  expect(await page.evaluate(()=>window.libraryActions)).toEqual([]);
  await page.locator('#online-filter-chip').click();await page.locator('#discogs-query').click();
  await expect(page.getByRole('dialog',{name:'Label filter'})).toHaveCount(0);
  await expect(page.locator('#discogs-search')).toBeEnabled();
});
