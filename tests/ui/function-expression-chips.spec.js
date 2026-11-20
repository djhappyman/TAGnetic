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

const composer=page=>page.locator('[data-function-composer]').last();
async function openBuilder(page){await page.locator('.mode-tab[data-mode="renamer"]').click();await page.locator('#mask-builder-open').click();}
async function choose(page,editor,text,label){await editor.fill(text);await expect(page.getByRole('option',{name:label,exact:true})).toBeVisible();await editor.press('Enter');}
async function raw(page,value){await page.locator('#mb-expr').evaluate((n,value)=>{n.value=value;n.dispatchEvent(new Event('input',{bubbles:true}));},value);}
async function value(page){return page.locator('#mb-expr').evaluate(n=>n.value);}
test('typing offers function descriptions and Replace uses one inline argument row',async({page})=>{
 await openBuilder(page);await choose(page,page.locator('#mb-expr'),'replace','Replace');
 await expect(composer(page)).toBeVisible();await expect(composer(page).getByRole('combobox',{name:'Operation'})).toHaveText('Replace');
 await expect(composer(page).getByRole('combobox',{name:'Value',exact:true})).toBeFocused();
 await choose(page,composer(page).getByRole('combobox',{name:'Value',exact:true}),'title','Title');
 await composer(page).getByRole('combobox',{name:'Find',exact:true}).fill('_');
 await composer(page).getByRole('combobox',{name:'Replace with',exact:true}).fill(' ');
 await composer(page).getByRole('button',{name:'OK',exact:true}).click();
 await expect(page.locator('#mb-expr [data-function]')).toHaveText('Replace · Title');
 expect(await value(page)).toBe('$replace(%title%,_, )');
 await page.locator('#mb-expr').press('Backspace');await expect(page.locator('#mb-expr [data-pattern]')).toHaveCount(0);
 await page.locator('#mb-expr').press('ControlOrMeta+z');await expect(page.locator('#mb-expr [data-function]')).toHaveText('Replace · Title');
 expect(await value(page)).toBe('$replace(%title%,_, )');
});
test('click editing retains raw spelling, empty arguments and cached operation values',async({page})=>{
 await openBuilder(page);const original="$REPLACE($upper(%artist%),'a,b',)";await raw(page,original);
 await expect(page.locator('#mb-expr [data-function]')).toHaveCount(1);await page.locator('#mb-expr [data-function]').click();
 await expect(composer(page).getByRole('combobox',{name:'Find',exact:true})).toHaveText('a,b');
 await composer(page).getByRole('combobox',{name:'Operation'}).click();await page.getByRole('option',{name:'Uppercase',exact:true}).click();
 await expect(composer(page).getByRole('combobox',{name:'Find',exact:true})).toHaveCount(0);
 await composer(page).getByRole('combobox',{name:'Operation'}).click();await page.getByRole('option',{name:'Replace',exact:true}).click();
 await expect(composer(page).getByRole('combobox',{name:'Find',exact:true})).toHaveText('a,b');
 await composer(page).getByRole('button',{name:'OK',exact:true}).click();await expect(page.locator('[data-function-composer]')).toHaveCount(0);expect(await value(page)).toBe(original);
});
test('argument typing supports nested text functions without an input-type selector',async({page})=>{
 await openBuilder(page);await choose(page,page.locator('#mb-expr'),'replace','Replace');
 await choose(page,composer(page).getByRole('combobox',{name:'Value',exact:true}),'title','Title');
 await choose(page,composer(page).getByRole('combobox',{name:'Find',exact:true}),'trim','Trim');
 await expect(page.locator('[data-function-composer]')).toHaveCount(2);
 await choose(page,composer(page).getByRole('combobox',{name:'Value',exact:true}),'artist','Artist');
 await composer(page).getByRole('button',{name:'OK',exact:true}).click();
 await expect(page.locator('[data-function-composer]')).toHaveCount(1);
 await expect(composer(page).getByRole('combobox',{name:'Find',exact:true}).locator('[data-function]')).toHaveText('Trim · Artist');
 await composer(page).getByRole('combobox',{name:'Replace with',exact:true}).fill('x');
 await composer(page).getByRole('button',{name:'OK',exact:true}).click();
 await expect(page.locator('[data-function-composer]')).toHaveCount(0);expect(await value(page)).toBe('$replace(%title%,$trim(%artist%),x)');
 await expect(page.locator('#mb-expr [data-function]')).toHaveCount(1);
});
test('typing a function after a tag wraps that tag; cancellation never stages a partial expression',async({page})=>{
 await openBuilder(page);await raw(page,'%artist%');
 await page.locator('#mb-expr').focus();await page.locator('#mb-expr').press('End');await page.keyboard.type(' replace');
 await expect(page.getByRole('option',{name:'Replace',exact:true})).toBeVisible();await page.keyboard.press('Enter');
 await expect(composer(page).getByRole('combobox',{name:'Value',exact:true}).locator('[data-pattern]')).toHaveText('Artist');
 await expect(page.locator('#mb-ok')).toHaveAttribute('inert','');
 await page.locator('#mb-ok').evaluate(n=>n.click());expect(await page.evaluate(()=>window.tagrex.previewPlan)).toBe(null);
 await composer(page).getByRole('combobox',{name:'Find',exact:true}).press('Escape');
 await expect(page.locator('[data-function-composer]')).toHaveCount(0);await expect(page.locator('#mask-builder')).toBeVisible();
 expect(await value(page)).toBe('%artist% replace');await expect(page.locator('#mb-ok')).not.toHaveAttribute('inert','');
});
test('native quote characters and commas remain editable text and serialize safely',async({page})=>{
 await openBuilder(page);await choose(page,page.locator('#mb-expr'),'replace','Replace');
 await choose(page,composer(page).getByRole('combobox',{name:'Value',exact:true}),'title','Title');
 await composer(page).getByRole('combobox',{name:'Find',exact:true}).fill(',');
 await composer(page).getByRole('combobox',{name:'Replace with',exact:true}).fill("it's John's");
 await composer(page).getByRole('button',{name:'OK',exact:true}).click();
 await expect(page.locator('[data-function-composer]')).toHaveCount(0);expect(await value(page)).toBe("$replace(%title%,',',it''s John''s)");
});
test('logic and extraction stay raw; function composer remains reachable in both themes',async({page})=>{
 await openBuilder(page);await raw(page,'$if2(%artist%,%title%)');await expect(page.locator('#mb-expr [data-function]')).toHaveCount(0);
 for(const theme of ['light','dark']){
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
  await choose(page,page.locator('#mb-expr'),'upper','Uppercase');
  const box=await composer(page).boundingBox();expect(box.x).toBeGreaterThanOrEqual(0);expect(box.x+box.width).toBeLessThanOrEqual(1280);
  await composer(page).getByRole('button',{name:'Cancel'}).click();
 }
 await page.locator('#mb-cancel').click();await openTagSource(page,'filename');await page.locator('#from-name-mask').fill('upper');
 await expect(page.getByRole('option',{name:'Uppercase',exact:true})).toHaveCount(0);
});

test('sidebar chips open supported function composers and headers use the app hierarchy',async({page})=>{
 await openBuilder(page);
 await expect(page.locator('#mb-ref-body [data-token="$upper()"] .ph-tag-chip')).toHaveText('Uppercase');
 await expect(page.locator('#mb-ref-body [data-token="%filename%"] .ph-tag-chip')).toHaveText('Filename');
 await expect(page.locator('#mb-ref-body [data-token="%_bitrate%"] .ph-tag-chip')).toHaveText('Bitrate');
 expect(await page.locator('#mb-ref-body .ph-dir').allTextContents()).not.toContain('name only');
 await expect(page.locator('#mb-ref-body .ph-group-name').first()).toHaveCSS('text-transform','none');
 await page.locator('#mb-ref-body [data-token="$upper()"] ').click();
 await expect(composer(page)).toBeVisible();
 await expect(composer(page).getByRole('combobox',{name:'Operation'})).toHaveText('Uppercase');
 await composer(page).getByRole('button',{name:'Cancel'}).click();
});

test('closing the parent cancels unfinished functions and releases blocked actions',async({page})=>{
 await openBuilder(page);await choose(page,page.locator('#mb-expr'),'replace','Replace');
 await expect(composer(page)).toBeVisible();await page.locator('#mb-cancel').click();
 await expect(page.locator('[data-function-composer]')).toHaveCount(0);
 await expect(page.locator('#mb-ok')).not.toHaveAttribute('inert','');
 await page.locator('#mask-builder-open').click();await expect(page.locator('#mb-expr')).toBeEditable();
});
test('function arguments remain one row and reachable on a narrow viewport',async({page})=>{
 await page.setViewportSize({width:520,height:760});await openBuilder(page);
 await choose(page,page.locator('#mb-expr'),'replace','Replace');
 const positions=await composer(page).locator('.function-argument').evaluateAll(nodes=>nodes.map(n=>n.getBoundingClientRect().top));
 expect(Math.max(...positions)-Math.min(...positions)).toBeLessThan(2);
 await composer(page).evaluate(n=>{n.scrollLeft=n.scrollWidth;});
 await expect(composer(page).getByRole('button',{name:'Cancel'})).toBeInViewport();
 await composer(page).getByRole('button',{name:'Cancel'}).click();await expect(page.locator('[data-function-composer]')).toHaveCount(0);
});
