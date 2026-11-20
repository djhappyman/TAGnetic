import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
import { openTagSource } from './navigation.js';
const root=resolve('.test-dist');
test.beforeEach(async({page})=>{
  page.issues=[];page.on('pageerror',error=>page.issues.push(String(error)));
  await page.addInitScript(()=>{
    localStorage.removeItem('tagrex.maskBuilderSize');
    window.builderTracks=Array.from({length:371},(_,i)=>({path:`/music/${i+1} - Artist - Title.mp3`,format:'Mp3',tags:{track:String(i+1),artist:'Artist',title:`Track ${i+1} ${'Long filename '.repeat(10)}`,album:'Album'},has_cover:false,tag_blocks:[]}));
  });
  await page.route('http://tagrex.test/**',async route=>{
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);
    if(!path.startsWith(root+sep))return route.abort();
    try{
      let body=await readFile(path);
      if(path.endsWith('.js'))body=Buffer.from(body.toString().replace(/(case([`"'])list_tracks\2:\s*)return Promise\.resolve\(([\w$]+)\.tracks\)/g,'$1$3.tracks=window.builderTracks;return Promise.resolve($3.tracks)'));
      await route.fulfill({body,contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});
    }catch {await route.fulfill({status:404,body:'Missing fixture'});}
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async()=>{document.getElementById('root').value='/music';await window.tagrex.openLibrary();});
  await page.locator('#ribbon-select-all').click();
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await page.locator('#mask-builder-open').click();
  await expect(page.locator('#mb-list tbody tr')).toHaveCount(300);
  await page.locator('.mask-builder').evaluate(n=>Promise.all(n.closest('.controller-dialog-content').getAnimations().map(a=>a.finished)));
});
test.afterEach(async({page})=>expect(page.issues).toEqual([]));
test('batch preview has sticky headers and consistent surfaces without summary counts',async({page})=>{
  expect(await page.locator('#mb-list th').allTextContents()).toEqual(['#','Current name','Proposed name','Status']);
  await expect(page.locator('#mb-summary')).toHaveCount(0);
  await expect(page.locator('.mb-field')).toHaveCSS('border-top-width','0px');
  await expect(page.locator('#mb-list .mb-result-status').first()).toHaveText('Changed');
  await expect(page.locator('#mb-list thead th').first()).toHaveCSS('position','sticky');
  await expect(page.locator('#mb-list thead th').first()).toHaveCSS('text-transform','none');
  expect(await page.locator('#mb-list .mb-new').first().evaluate(n=>n.dataset.tooltip)).toContain('Long filename');
  for(const theme of ['light','dark']){
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    const colours=await page.evaluate(()=>{
      const css=n=>getComputedStyle(n).backgroundColor;
      const dialog=document.querySelector('.mask-builder');
      const sidebar=document.querySelector('.mb-ref');
      const cells=[...document.querySelectorAll('#mb-list tbody tr')].slice(0,2).map(n=>css(n.querySelector('td')));
      return {shell:css(dialog),sidebar:css(sidebar),preview:css(document.querySelector('.mb-preview')),body:css(document.querySelector('.mb-ref-body')),cells};
    });
    expect(colours.shell).not.toBe(colours.preview);expect(colours.sidebar).toBe(colours.preview);expect(colours.body).toBe('rgba(0, 0, 0, 0)');
    expect(colours.cells[0]).not.toBe(colours.cells[1]);
  }
});
test('supported reference values become chips; filtering and caret insertion preserve native syntax',async({page})=>{
  await expect(page.locator('.mb-ref #mb-ref-filter')).toBeVisible();
  await expect(page.locator('#mb-ref-body [data-token="%artist%"] .ph-tag-chip')).toHaveText('Artist');
  await expect(page.locator('#mb-ref-body [data-token="%filename%"] .ph-tag-chip')).toHaveText('Filename');
  await expect(page.locator('#mb-ref-body [data-token="$upper()"] .ph-tag-chip')).toHaveText('Uppercase');
  await expect(page.locator('#mb-ref-body [data-token="[]"] .ph-tag-chip')).toHaveCount(0);
  const hue=await page.locator('#mb-ref-body [data-token="%artist%"] .ph-tag-chip').evaluate(n=>n.style.getPropertyValue('--field-hue'));
  expect(hue).toBe('255');
  const hues=await page.locator('#mb-ref-body .ph-tag-chip').evaluateAll(nodes=>[...new Set(nodes.map(n=>n.style.getPropertyValue('--field-hue')))]);
  expect(hues.length).toBeGreaterThanOrEqual(4);
  await page.locator('#mb-ref-filter').fill('album artist');
  await expect(page.locator('#mb-ref-body .ph-tag-chip')).toHaveText('Album Artist');
  await page.locator('#mb-expr').evaluate(n=>{n.value='%title%';n.focus();n.setSelectionRange(0,0);});
  await page.locator('#mb-ref-body [data-token="%albumartist%"] ').click();
  expect(await page.locator('#mb-expr').evaluate(n=>n.value)).toBe('%albumartist%%title%');
  await page.locator('#mb-ref-clear').click();
  await expect(page.locator('#mb-ref-body .ph-grammar')).toBeVisible();
  await page.locator('#mb-cancel').click();
  expect(await page.locator('#mask').evaluate(n=>n.value)).toBe('%artist% - %title%');
  expect(await page.evaluate(()=>window.tagrex.previewPlan)).toBe(null);
});
test('both established overlay rails scroll, fade and survive reopening without duplicates',async({page})=>{
  const vertical=page.locator('.mb-preview [role="scrollbar"][aria-orientation="vertical"]');
  const horizontal=page.locator('.mb-preview [role="scrollbar"][aria-orientation="horizontal"]');
  await expect(vertical).toBeVisible();await expect(horizontal).toBeVisible();
  await expect(page.locator('#mb-list')).toHaveCSS('scrollbar-width','none');
  await page.locator('#mb-list').evaluate(n=>{n.scrollTop=100;n.scrollLeft=80;});
  await expect(vertical).toHaveClass(/is-active/);await expect(horizontal).toHaveClass(/is-active/);
  await expect(vertical).not.toHaveClass(/is-active/);await expect(horizontal).not.toHaveClass(/is-active/);
  await horizontal.focus();await page.keyboard.press('End');
  await expect.poll(()=>page.locator('#mb-list').evaluate(n=>n.scrollLeft)).toBeGreaterThan(100);
  await vertical.focus();await page.keyboard.press('End');
  await expect.poll(()=>page.locator('#mb-list').evaluate(n=>n.scrollTop)).toBeGreaterThan(1000);
  const raw=await page.locator('#mb-expr').evaluate(n=>n.value);
  await page.locator('#mb-list').evaluate(n=>{n.scrollTop=0;});
  const thumb=await vertical.locator('.table-scrollbar-thumb').boundingBox();
  await page.mouse.move(thumb.x+thumb.width/2,thumb.y+thumb.height/2);
  await page.mouse.down();await page.mouse.move(thumb.x+thumb.width/2,thumb.y+thumb.height/2+80);await page.mouse.up();
  await expect.poll(()=>page.locator('#mb-list').evaluate(n=>n.scrollTop)).toBeGreaterThan(100);
  expect(await page.locator('#mb-expr').evaluate(n=>n.value)).toBe(raw);
  await page.locator('#mb-cancel').click();await page.locator('#mask-builder-open').click();
  await expect(page.locator('.mb-preview [role="scrollbar"]')).toHaveCount(2);
});
test('content fades follow both scroll boundaries and clear when filtering removes overflow',async({page})=>{
  for(const id of ['mb-list','mb-ref-body']) {
    const port=page.locator('#'+id);
    await expect(port).toHaveAttribute('data-fade-top','false');
    await expect(port).toHaveAttribute('data-fade-bottom','true');
    await expect(port).not.toHaveCSS('mask-image','none');
    await port.evaluate(node=>node.scrollTop=100);
    await expect(port).toHaveAttribute('data-fade-top','true');
    await expect(port).toHaveAttribute('data-fade-bottom','true');
    await port.evaluate(node=>node.scrollTop=node.scrollHeight);
    await expect(port).toHaveAttribute('data-fade-bottom','false');
    await expect(port).toHaveAttribute('data-fade-top','true');
    await port.evaluate(node=>node.scrollTop=0);
    await expect(port).toHaveAttribute('data-fade-top','false');
  }
  expect(await page.locator('#mb-list').evaluate(node=>parseFloat(node.style.getPropertyValue('--scroll-fade-head')))).toBeGreaterThan(0);
  expect(await page.locator('#mb-ref-body').evaluate(node=>parseFloat(node.style.getPropertyValue('--scroll-fade-head')))).toBe(0);
  await page.locator('#mb-ref-filter').fill('no-matching-placeholders');
  await expect(page.locator('#mb-ref-body')).toHaveAttribute('data-fade-bottom','false');
  await expect(page.locator('#mb-ref-body')).toHaveAttribute('data-fade-top','false');
  await page.locator('#mb-ref-filter').fill('');
  await expect(page.locator('#mb-ref-body')).toHaveAttribute('data-fade-bottom','true');
  // Filtering and fading never mask the search field, footer or scrollbar rails.
  await expect(page.locator('.mb-ref-search')).toHaveCSS('mask-image','none');
  await expect(page.locator('.mb-ref .ph-foot')).toHaveCSS('mask-image','none');
  await expect(page.locator('.mb-preview .inspector-scrollbar.is-vertical')).toHaveCSS('mask-image','none');
});
test('extraction retains native columns and reference restrictions',async({page})=>{
  await page.locator('#mb-cancel').click();await openTagSource(page,'filename');
  await page.locator('#from-name-mask').fill('%track% - %artist% - %title%');
  await page.locator('#from-name-builder-open').click();
  await expect(page.locator('#mb-list th')).toHaveCount(4);
  await expect(page.locator('#mb-ref-body [data-token="$upper()"]')).toHaveCount(0);
  await expect(page.locator('#mb-ref-body [data-token="%skip%"] .ph-tag-chip')).toHaveText('Skip Text');
});
test('preview, integrated filter and footer remain reachable in a narrow window',async({page})=>{
  await page.setViewportSize({width:520,height:760});
  await expect(page.locator('#mb-ref-filter')).toBeInViewport();
  await expect(page.locator('#mb-ok')).toBeInViewport();
  const box=await page.locator('.mask-builder').boundingBox();
  expect(box.x).toBeGreaterThanOrEqual(0);expect(box.x+box.width).toBeLessThanOrEqual(520);
  await expect(page.locator('#mb-list')).toBeInViewport();
});

test('unchanged names, collisions, missing values and invalid syntax retain clear preview states',async({page})=>{
  await page.locator('#mb-expr').fill('%track% - Artist - Title');
  await expect(page.locator('#mb-list .mb-unchanged')).toHaveCount(300);
  await expect(page.locator('#mb-summary')).toHaveCount(0);
  await page.locator('#mb-expr').fill('Same name');
  await expect(page.locator('#mb-list .mb-clash')).toHaveCount(300);
  await expect(page.locator('#mb-list .mb-result-status').first()).toHaveText('Conflict');
  await page.locator('#mb-expr').fill('%genre% - %title%');
  await expect(page.locator('#mb-list .mb-unrendered')).toHaveCount(300);
  await expect(page.locator('#mb-list .mb-result-status').first()).toHaveText('Missing tag');
  await page.locator('#mb-expr').fill('%notreal%');
  await expect(page.locator('#mb-error')).toBeVisible();await expect(page.locator('#mb-ok')).toBeDisabled();
  await expect(page.locator('#mb-list .mb-unrendered')).toHaveCount(300);
  expect(await page.evaluate(()=>window.tagrex.previewPlan)).toBe(null);
});

async function widths(page) {
  return page.evaluate(()=>({dialog:document.querySelector('.mask-builder').offsetWidth,
    sidebar:document.querySelector('.mb-ref').offsetWidth,
    preview:document.querySelector('.mb-preview').offsetWidth,
    table:document.querySelector('#mb-list table').offsetWidth,
    columns:[...document.querySelectorAll('#mb-list th')].map(n=>n.offsetWidth),
    raw:document.getElementById('mb-expr').value}));
}
async function dragX(page,selector,delta) {
  const box=await page.locator(selector).boundingBox();
  const x=box.x+box.width/2,y=box.y+box.height/2;
  await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x+delta,y,{steps:6});await page.mouse.up();
}
test('horizontal dialog edges resize the viewport without resizing preview columns',async({page})=>{
  await page.locator('.mask-builder').evaluate(n=>{n.style.width='1000px';});
  const before=await widths(page);
  await dragX(page,'#mb-resize-right',70);
  await expect.poll(async()=>(await widths(page)).dialog).toBeGreaterThan(before.dialog+100);
  const expanded=await widths(page);
  expect(expanded.table).toBeCloseTo(before.table,0);expect(expanded.columns).toEqual(before.columns);expect(expanded.raw).toBe(before.raw);
  await dragX(page,'#mb-resize-left',60);
  await expect.poll(async()=>(await widths(page)).dialog).toBeLessThan(expanded.dialog-80);
  const final=await widths(page);
  await page.locator('#mb-cancel').click();await page.locator('#mask-builder-open').click();
  expect((await widths(page)).dialog).toBeCloseTo(final.dialog,0);
});
test('pane grip reflows the sidebar and persists its width without scaling the table',async({page})=>{
  await page.locator('#mb-pane-divider').focus();await page.keyboard.press('End');
  const before=await widths(page);
  const description=page.locator('#mb-ref-body [data-token="%catalognumber%"] .ph-desc');
  const oldHeight=await description.evaluate(n=>n.getBoundingClientRect().height);
  await dragX(page,'#mb-pane-divider',-240);
  await expect.poll(async()=>(await widths(page)).sidebar).toBeGreaterThan(before.sidebar+100);
  const after=await widths(page);
  expect(after.preview).toBeLessThan(before.preview-100);
  expect(after.table).toBeCloseTo(before.table,0);expect(after.columns).toEqual(before.columns);expect(after.raw).toBe(before.raw);
  expect(await description.evaluate(n=>n.getBoundingClientRect().height)).toBeLessThan(oldHeight);
  await page.locator('#mb-cancel').click();await page.locator('#mask-builder-open').click();
  expect((await widths(page)).sidebar).toBeCloseTo(after.sidebar,0);
  await page.locator('#mb-pane-divider').dblclick();
  expect((await widths(page)).sidebar).toBeLessThan(after.sidebar-50);
});
test('resize grips support keyboard bounds and Escape cancels an active drag',async({page})=>{
  const divider=page.locator('#mb-pane-divider');
  const before=await widths(page);
  await divider.focus();await page.keyboard.press('ArrowRight');
  expect((await widths(page)).sidebar).toBeCloseTo(before.sidebar-10,0);
  await page.keyboard.press('Home');expect((await widths(page)).preview).toBeGreaterThanOrEqual(280);
  await page.keyboard.press('End');expect((await widths(page)).sidebar).toBeCloseTo(200,0);
  const reset=await widths(page);
  const box=await divider.boundingBox();
  await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.down();
  await page.mouse.move(box.x-80,box.y+box.height/2);
  await page.keyboard.press('Escape');await page.mouse.up();
  await expect(page.locator('#mask-builder')).toBeVisible();expect((await widths(page)).sidebar).toBeCloseTo(reset.sidebar,0);
  await page.locator('#mb-resize-right').focus();await page.keyboard.press('Home');
  expect((await widths(page)).dialog).toBeCloseTo(720,0);
  await page.keyboard.press('End');expect((await widths(page)).dialog).toBeLessThanOrEqual(1280-24);
  expect((await widths(page)).raw).toBe(before.raw);
});
