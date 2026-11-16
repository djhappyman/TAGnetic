import { openTagSource } from './navigation.js';
import {test, expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
const root=resolve('.test-dist');
test.beforeEach(async({page})=>{
  page.errors=[];page.on('pageerror',error=>page.errors.push(error.message));
  await page.route('http://tagrex.test/**',async route=>{
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);
    if(!path.startsWith(root+sep))return route.abort();
    try{await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});}catch{await route.fulfill({status:404});}
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async()=>{document.getElementById('root').value='/music';await window.tagrex.openLibrary();});
});
test.afterEach(async({page})=>expect(page.errors).toEqual([]));
const box=(page,selector)=>page.locator(selector).boundingBox();
async function settled(page){await page.evaluate(async()=>{await Promise.all(document.getAnimations().map(animation=>animation.finished.catch(()=>{})));});}

for(const theme of ['light','dark'])test(`${theme}: drawer overlays a stable table and address bar animates independently`,async({page},info)=>{
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
  const toggle=page.locator('#panel-toggle');
  await expect(toggle).toHaveAttribute('aria-expanded','true');
  await expect(page.locator('.dock-frame')).toHaveAttribute('data-sidebar-drawer','true');
  const ribbon=await box(page,'.command-ribbon'), drawer=await box(page,'.dock-right'), table=await box(page,'#tracks'), address=await box(page,'.library-header');
  expect(drawer.y).toBe(ribbon.y);
  expect(address.x+address.width).toBeCloseTo(drawer.x,0);
  const columns=await page.locator('#tracks thead th:not(.table-fill)').evaluateAll(nodes=>nodes.map(node=>({x:node.getBoundingClientRect().x,width:node.getBoundingClientRect().width})));
  await page.evaluate(()=>{window.savedTable=document.getElementById('tracks');window.savedRows=[...document.querySelectorAll('#tracks-body tr[data-path]')];});
  await toggle.focus();await page.keyboard.press('Enter');
  await expect(toggle).toHaveAttribute('aria-expanded','false');
  const middle=await page.evaluate(async()=>{
    await new Promise(resolve=>setTimeout(resolve,60));
    const rect=document.querySelector('.library-header').getBoundingClientRect();
    return{width:rect.width, transition:getComputedStyle(document.querySelector('.dock-right')).transitionDuration};
  });
  expect(middle.width).toBeGreaterThanOrEqual(address.width);
  expect(middle.width).toBeLessThan(1280);
  expect(middle.transition).toContain('0.24s');
  await settled(page);
  await expect(page.locator('.dock-right')).toBeHidden();
  const closed=await box(page,'.library-header');
  expect(closed.width).toBeCloseTo((await box(page,'#tracks')).width,0);
  expect((await box(page,'#tracks')).width).toBeGreaterThanOrEqual(table.width);
  const closedRibbon=await box(page,'.command-ribbon');
  expect(closedRibbon.x).toBe(ribbon.x);expect(closedRibbon.y).toBe(ribbon.y);
  expect(await page.locator('#tracks thead th:not(.table-fill)').evaluateAll(nodes=>nodes.map(node=>({x:node.getBoundingClientRect().x,width:node.getBoundingClientRect().width})))).toEqual(columns);
  expect(await page.evaluate(()=>window.savedTable===document.getElementById('tracks'))).toBe(true);
  expect(await page.evaluate(()=>window.savedRows.every(row=>row.isConnected))).toBe(true);
  await toggle.press('Enter');await settled(page);
  expect((await box(page,'.library-header')).width).toBeCloseTo(address.width,0);
  await page.screenshot({path:info.outputPath(`drawer-${theme}.png`)});
});

test('folding preserves staged values, selected files, and the mounted editor',async({page})=>{
  await openTagSource(page, 'editor');
  await page.locator('#editor-field-title').fill('Keep this draft');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const before=await page.evaluate(()=>{window.savedField=document.getElementById('editor-field-title');return {plan:structuredClone(window.tagrex.previewPlan),selection:[...window.tagrex.selection]};});
  for(let i=0;i<2;i++) {await page.locator('#panel-toggle').click();await settled(page);}
  await expect(page.locator('#editor-field-title')).toHaveValue('Keep this draft');
  expect(await page.evaluate(()=>window.savedField===document.getElementById('editor-field-title'))).toBe(true);
  expect(await page.evaluate(()=>window.tagrex.previewPlan)).toEqual(before.plan);
  expect(await page.evaluate(()=>[...window.tagrex.selection])).toEqual(before.selection);
  await expect(page.locator('#diff-apply')).toBeEnabled();
});

test('Transformations belongs to Tags as a source tab and retains its native checklist',async({page})=>{
 await openTagSource(page,'transformations');const panel=page.locator('#subtab-transformations');await expect(panel).toBeVisible();await page.locator('#groups-menu').evaluate(n=>window.keptChecklist=n);
 for(const mode of ['renamer','exporter']){await page.locator(`.mode-tab[data-mode="${mode}"]`).click();await expect(panel).toBeHidden();}
 await page.locator('.mode-tab[data-mode="tagger"]').click();await expect(panel).toBeVisible();expect(await page.locator('#groups-menu').evaluate(n=>n===window.keptChecklist)).toBe(true);
});

test('compact switching keeps the ribbon visible and the existing toggle reveals the inspector',async({page})=>{
  await page.setViewportSize({width:390,height:760});
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await expect(page.locator('#mode-col')).toBeVisible();
  await expect(page.locator('.command-ribbon')).toBeVisible();
  const ribbon=await box(page,'.command-ribbon');
  await page.locator('#panel-toggle').click();
  await expect(page.locator('#mode-col')).toBeHidden();
  await expect(page.locator('#files-view')).toBeVisible();
  const closedRibbon=await box(page,'.command-ribbon');
  expect(closedRibbon.x).toBe(ribbon.x);expect(closedRibbon.y).toBe(ribbon.y);
  await page.locator('#panel-toggle').click();
  await expect(page.locator('#mode-col')).toBeVisible();
  await expect(page.locator('#mask')).toBeInViewport();
  await expect(page.getByRole('tab',{name:'Transformations',exact:true})).toHaveCount(0);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
});

test('collapse choice persists and reduced motion removes the slide',async({page})=>{
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.locator('#panel-toggle').click();
  await expect(page.locator('#panel-toggle')).toHaveAttribute('aria-expanded','false');
  expect(await page.locator('.dock-right').evaluate(node=>getComputedStyle(node).transitionDuration)).toBe('0s');
  await page.reload();
  await expect(page.locator('#panel-toggle')).toHaveAttribute('aria-expanded','false');
  await expect(page.locator('.dock-right')).toBeHidden();
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await expect(page.locator('#panel-toggle')).toHaveAttribute('aria-expanded','true');
  await expect(page.locator('#mask')).toBeVisible();
});

test('fixed sidebar leaves every ribbon action uncovered',async({page})=>{
  await expect(page.getByRole('separator',{name:'Resize workspace columns',exact:true})).toHaveCount(0);
  expect((await box(page,'.dock-right')).width).toBe(420);
  for(const button of await page.locator('.command-ribbon button.ribbon-command').all()) {
    await expect(button).toBeInViewport();
    expect(await button.evaluate(node=>{const r=node.getBoundingClientRect();return node.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));})).toBe(true);
  }
  const fileArea=await box(page,'.files-col'), filter=await box(page,'#filter-panel');
  expect(filter.y+filter.height).toBeLessThan(fileArea.y);
  expect(await page.locator('.files-col').evaluate(node=>getComputedStyle(node).borderBottomRightRadius)).toBe('14px');
});

test('horizontal scrolling reaches the last column with the drawer open and adjusts when closed',async({page})=>{
  const scroll=()=>page.evaluate(()=>{const n=document.getElementById('files-view');n.scrollLeft=n.scrollWidth;return{left:n.scrollLeft,max:n.scrollWidth-n.clientWidth,width:n.clientWidth,content:n.scrollWidth};});
  const open=await scroll();expect(open.max).toBeGreaterThan(0);expect(open.left).toBe(open.max);
  const last=page.locator('#tracks thead th[data-sort]').last();
  const view=await box(page,'#files-view'),end=await last.boundingBox();
  expect(end.x+end.width).toBeLessThanOrEqual((await box(page,'.dock-right')).x+1);
  await page.locator('#panel-toggle').click();await settled(page);
  const closed=await scroll();expect(closed.width).toBe(open.width);expect(closed.left).toBe(closed.max);
  expect(closed.content).toBeLessThanOrEqual(open.content);
  await page.locator('#panel-toggle').click();await settled(page);
  const reopened=await scroll();expect(reopened.max).toBe(open.max);expect(reopened.left).toBe(reopened.max);
});


test('routine info messages do not create a bottom strip, while actionable failures remain visible',async({page})=>{
  await expect(page.locator('#toast')).toBeHidden();
  await openTagSource(page, 'editor');
  await page.locator('#editor-field-title').fill('A pending title');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.locator('#diff-discard').click();
  await page.locator('#panel-toggle').click();await settled(page);
  await expect(page.locator('#toast')).toBeHidden();
  await expect(page.locator('#filter-panel')).toBeInViewport();
  await page.locator('#panel-toggle').click();await settled(page);
  await page.locator('#editor-field-title').fill('Another pending title');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.locator('#ribbon-add-folder').click();
  await expect(page.locator('#toast.error')).toBeVisible();
  await expect(page.locator('#toast.error')).toContainText('Apply or clear staged edits');
});

test('edit actions stay bottom right and the player yields space without making a second row',async({page})=>{
  const toggle=page.locator('#panel-toggle');
  await toggle.click();await settled(page);
  const full=await box(page,'#player'), initialFooter=await box(page,'.statusbar');
  await expect(page.locator('#diff-actionbar')).toBeVisible();await expect(page.locator('#diff-apply')).toBeDisabled();
  await toggle.click();await settled(page);
  await openTagSource(page, 'editor');
  await page.locator('#editor-field-title').fill('Footer draft');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  await toggle.click();await settled(page);
  const staged=await box(page,'#player'), apply=await box(page,'#diff-apply'), preview=await box(page,'#diff-preview'), footer=await box(page,'.statusbar');
  expect(staged.width).toBeCloseTo(full.width,0);
  expect(apply.x+apply.width).toBeCloseTo(footer.width-12,0);
  expect(staged.x+staged.width).toBeLessThan(preview.x-8);
  expect(Math.abs(staged.y-apply.y)).toBeLessThan(1);
  expect(footer.height).toBeCloseTo(initialFooter.height,0);
  await toggle.click();await settled(page);
  const openPlayer=await box(page,'#player'), openApply=await box(page,'#diff-apply'), filter=await box(page,'#filter-panel'), summary=await box(page,'#staged-edits');
  expect(openPlayer.width).toBeLessThan(staged.width);
  expect(openApply.x+openApply.width).toBeCloseTo(apply.x+apply.width,0);
  expect(summary.y).toBeGreaterThan((await box(page,'.source-navigation')).y);
  expect(summary.y+summary.height+12).toBeCloseTo((await box(page,'.mode-col')).y+(await box(page,'.mode-col')).height,0);
  expect(openPlayer.y-filter.y-filter.height).toBeGreaterThanOrEqual(12);
  expect(await page.locator('#files-view').evaluate(n=>getComputedStyle(n,'::-webkit-scrollbar-corner').backgroundColor)).toBe('rgba(0, 0, 0, 0)');
});

test('every visible column heading stays above scrolled staged rows on both axes',async({page})=>{
  await page.evaluate(async()=>{
    await window.tagrex.invoke('open_drop',{paths:Array.from({length:200},(_,i)=>`track-${i}.mp3`)});
    document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();
  });
  await page.locator('#ribbon-select-all').click();
  await openTagSource(page, 'editor');
  await page.locator('#editor-field-title').fill('Staged title');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  for(const x of [0,150,400]) {
    const headers=await page.evaluate(async x=>{
      const viewport=document.getElementById('files-view');viewport.scrollTop=600;viewport.scrollLeft=x;
      await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
      const v=viewport.getBoundingClientRect(),file=document.querySelector('#tracks th[data-sort="file"]').getBoundingClientRect();
      return [...document.querySelectorAll('#tracks thead th')].filter(n=>{const r=n.getBoundingClientRect();return r.width&&r.x+r.width/2>Math.max(v.x,file.right)&&r.x+r.width/2<Math.min(v.right,document.querySelector('.dock-right').getBoundingClientRect().left)-12;}).map(n=>{
        const r=n.getBoundingClientRect(),hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);
        return{text:n.textContent.trim(),ownsHit:n.contains(hit),atTop:Math.abs(r.y-v.y)<1};
      });
    },x);
    expect(headers.length).toBeGreaterThan(0);
    for(const header of headers){expect(header.ownsHit,header.text).toBe(true);expect(header.atTop,header.text).toBe(true);}
  }
});

test('ribbon controls stay comfortable and expand only modestly when the drawer folds',async({page})=>{
  const before=await box(page,'.command-ribbon'), hit=await box(page,'#ribbon-open-folder');
  expect(hit.width).toBeGreaterThanOrEqual(31);
  await page.locator('#panel-toggle').click();await settled(page);
  const after=await box(page,'.command-ribbon');
  expect(after.width-before.width).toBeLessThanOrEqual(130);
  expect(after.width).toBeLessThanOrEqual(760);
  for(const button of await page.locator('.ribbon-command').all()) await expect(button).toBeInViewport();
});

test('tag-only staged edits keep ordinary row selection and cell highlighting working in Files',async({page})=>{
  await page.locator('#ribbon-remove-tags').click();
  await page.locator('#ribbon-remove-stage').click();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const staged=await page.evaluate(()=>structuredClone(window.tagrex.previewPlan));
  const row=page.locator('#tracks-body tr[data-path]').nth(1);
  const path=await row.getAttribute('data-path');
  await row.locator('td.file').click();
  await expect(row).toHaveClass(/selected/);
  expect(await page.evaluate(()=>[...window.tagrex.selection])).toEqual([path]);
  await row.locator('td[data-field="artist"]').click();
  await expect(row.locator('td[data-field="artist"]')).toHaveClass(/sheet-active/);
  expect(await page.evaluate(()=>window.tagrex.previewPlan)).toEqual(staged);
  await page.locator('#staged-tab').click();
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(staged.changes.length);
});

test('desktop sidebar stays 420px wide without a resize handle and ignores old drag preferences',async({page})=>{
  await expect(page.locator('.dock-right')).toHaveCSS('width','420px');
  await expect(page.getByRole('separator',{name:'Resize workspace columns',exact:true})).toHaveCount(0);
  for(const width of [1024,1440,1920]){
    await page.setViewportSize({width,height:900});
    await expect(page.locator('.dock-right')).toHaveCSS('width','420px');
  }
  await page.evaluate(()=>{
    const saved=JSON.parse(localStorage.getItem('tagrex.workspace.v1') || '{"version":1,"modules":{}}');saved.left=20;localStorage.setItem('tagrex.workspace.v1',JSON.stringify(saved));
  });
  await page.reload();await expect.poll(()=>page.evaluate(()=>!!window.tagrex)).toBe(true);
  await expect(page.locator('.dock-right')).toHaveCSS('width','420px');
});

test('rounded boundary stretches while the native table viewport stays fixed',async({page})=>{
  await page.emulateMedia({reducedMotion:'no-preference'});
  await page.locator('#group-btn').click();await page.locator('#group-menu [data-group="folder"]').click();
  const result=await page.evaluate(async()=>{
    let broadcasts=0;
    const original=CSSStyleDeclaration.prototype.setProperty;
    CSSStyleDeclaration.prototype.setProperty=function(key,...args){if(key==='--table-viewport-width')broadcasts++;return original.call(this,key,...args);};
    const table=document.getElementById('files-view'),widths=[],clips=[],edges=[],players=[];
    try{
      document.getElementById('panel-toggle').click();
      for(let i=0;i<24;i++){
        await new Promise(requestAnimationFrame);
        widths.push(table.clientWidth);edges.push(document.querySelector('.files-col').clientWidth);players.push(document.getElementById('player').clientWidth);clips.push(getComputedStyle(table.closest('.files-col')).clipPath);
      }
    }finally{CSSStyleDeclaration.prototype.setProperty=original;}
    const band=document.querySelector('.group-heading-content');
    return {broadcasts,widths,edges,players,clips,container:getComputedStyle(table).containerType,band:band?.getBoundingClientRect().width,width:table.clientWidth};
  });
  expect(result.broadcasts).toBe(0);
  expect(result.container).toBe('inline-size');
  expect(new Set(result.widths).size).toBe(1);
  expect(new Set(result.edges).size).toBeGreaterThan(2);
  expect(new Set(result.players).size).toBeGreaterThan(2);
  expect(result.clips.every(value=>value==='none')).toBe(true);
  expect(result.band).toBeCloseTo(result.width-24,0);
});

test('keyboard navigation reveals the final cell to the left of the open sidebar',async({page})=>{
  await page.locator('#tracks-body tr[data-path]').first().locator('td[data-field="title"]').click();
  for(let i=0;i<12;i++)await page.keyboard.press('ArrowRight');
  const focused=await page.evaluate(()=>{
    const n=document.activeElement,r=n.getBoundingClientRect(),edge=document.querySelector('.dock-right').getBoundingClientRect().left;
    return {field:n.dataset.field || document.querySelector('#tracks thead tr').cells[n.cellIndex]?.dataset.sort,right:r.right,edge,visible:n.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2))};
  });
  expect(focused.field).toBe('length');expect(focused.right).toBeLessThanOrEqual(focused.edge+1);expect(focused.visible).toBe(true);
  expect(await page.locator('.files-col').evaluate(n=>getComputedStyle(n,'::after').content)).toBe('none');
});
