import {test, expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
const root=resolve('.test-dist');
test.beforeEach(async({page})=>{
  await page.addInitScript(()=>{
    localStorage.setItem('tagrex.groupBy','');
    localStorage.setItem('tagrex.columns',JSON.stringify(['file','track','artist','title','album','year','length']));
    localStorage.setItem('tagrex.colWidths',JSON.stringify({file:80,track:40,artist:120,title:120,album:100,year:40,length:50}));
  });
  await page.route('http://tagrex.test/**',async route=>{
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);
    if(!path.startsWith(root+sep))return route.abort();
    try{await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});}catch{await route.fulfill({status:404});}
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async()=>{
    await window.tagrex.invoke('open_drop',{paths:Array.from({length:300},(_,i)=>`track-${String(i).padStart(3,'0')}.mp3`)});
    document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();
    await document.fonts.ready;
  });
});

test('overlay scrollbar fades after scrolling without reserving width and supports keys',async({page})=>{
  const bar=page.getByRole('scrollbar',{name:'Scroll tracks'}),viewport=page.locator('#files-view');
  await expect(bar).toHaveCount(1);
  expect(await viewport.evaluate(n=>n.offsetWidth-n.clientWidth)).toBe(0);
  const width=await viewport.evaluate(n=>n.clientWidth);
  await viewport.evaluate(n=>n.scrollTop=300);
  await expect.poll(()=>bar.evaluate(n=>Number(getComputedStyle(n).opacity))).toBe(1);
  await expect.poll(()=>bar.evaluate(n=>Number(getComputedStyle(n).opacity)),{timeout:2500}).toBe(0);
  expect(await viewport.evaluate(n=>n.clientWidth)).toBe(width);
  await bar.focus();await page.keyboard.press('End');
  await expect.poll(()=>viewport.evaluate(n=>Math.abs(n.scrollHeight-n.clientHeight-n.scrollTop))).toBeLessThan(2);
  await page.keyboard.press('Home');await expect.poll(()=>viewport.evaluate(n=>n.scrollTop)).toBe(0);
});

test('rows fill spare viewport width while data columns retain their chosen widths',async({page})=>{
  await page.locator('#panel-toggle').click();
  await page.evaluate(async()=>{await Promise.all(document.getAnimations().map(a=>a.finished.catch(()=>{})));});
  const fill=page.locator('#tracks-body tr[data-path] td.table-fill').first();
  await expect(fill).toHaveCount(1);
  expect((await fill.boundingBox()).width).toBeGreaterThan(50);
  for(const [key,width] of Object.entries({file:80,track:40,artist:120,title:120,album:100,year:40,length:50}))
    expect((await page.locator(`#tracks th[data-sort="${key}"]`).boundingBox()).width,key).toBeCloseTo(width,0);
  const table=await page.locator('#tracks').boundingBox(),row=await fill.locator('..').boundingBox(),filler=await fill.boundingBox();
  expect(row.width).toBeCloseTo(table.width,0);expect(filler.x+filler.width).toBeCloseTo(table.x+table.width,0);
  await fill.click();await expect(fill.locator('..')).toHaveClass(/selected/);
});

test('manual columns can be very narrow or wider than the old auto-fit cap',async({page})=>{
  const header=page.locator('#tracks th[data-sort="file"]');
  let grip=await header.locator('.col-resize').boundingBox();
  await page.mouse.move(grip.x+grip.width/2,grip.y+grip.height/2);await page.mouse.down();
  await page.mouse.move(grip.x-200,grip.y+grip.height/2);await page.mouse.up();
  expect((await header.boundingBox()).width).toBeLessThanOrEqual(24);
  grip=await header.locator('.col-resize').boundingBox();
  await page.mouse.move(grip.x+grip.width/2,grip.y+grip.height/2);await page.mouse.down();
  await page.mouse.move(grip.x+650,grip.y+grip.height/2);await page.mouse.up();
  expect((await header.boundingBox()).width).toBeGreaterThan(600);
});

test('horizontal overlay fades too, keeps its six-pixel thumb and reaches the final column',async({page})=>{
  await page.evaluate(async()=>{await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));});
  await expect(page.locator('.table-scrollbar.is-horizontal')).toBeHidden();
  const header=page.locator('#tracks th[data-sort="file"]'),grip=await header.locator('.col-resize').boundingBox();
  await page.mouse.move(grip.x+4,grip.y+10);await page.mouse.down();await page.mouse.move(grip.x+700,grip.y+10);await page.mouse.up();
  const bar=page.getByRole('scrollbar',{name:'Scroll columns'}),viewport=page.locator('#files-view');
  await expect(bar).toHaveCount(1);
  expect(await viewport.evaluate(n=>n.offsetHeight-n.clientHeight)).toBe(0);
  expect((await bar.locator('.table-scrollbar-thumb').boundingBox()).height).toBe(6);
  await page.mouse.move(20,20);
  await viewport.evaluate(n=>n.scrollLeft=200);
  await expect.poll(()=>bar.evaluate(n=>Number(getComputedStyle(n).opacity))).toBe(1);
  await expect.poll(()=>bar.evaluate(n=>Number(getComputedStyle(n).opacity)),{timeout:2500}).toBe(0);
  await bar.focus();await page.keyboard.press('End');
  await expect.poll(()=>viewport.evaluate(n=>Math.abs(n.scrollWidth-n.clientWidth-n.scrollLeft))).toBeLessThan(2);
});

test('auto-fit uses the full value and a manual drag turns off automatic refitting',async({page})=>{
  const cell=page.locator('#tracks-body td[data-field="title"]').first();
  await cell.dblclick();await cell.fill('A very long title '.repeat(15));await cell.press('Enter');
  await page.locator('#tracks th[data-sort="title"]').click({button:'right'});
  await page.locator('#columns-menu .column-fit-action').click();
  expect(await page.locator('#tracks th[data-sort="title"]').evaluate(n=>parseFloat(n.style.width))).toBeGreaterThan(480);
  await page.evaluate(()=>localStorage.setItem('tagrex.autofitColumns','1'));
  const header=page.locator('#tracks th[data-sort="file"]'),grip=await header.locator('.col-resize').boundingBox();
  await page.mouse.move(grip.x+4,grip.y+10);await page.mouse.down();await page.mouse.move(grip.x+40,grip.y+10);await page.mouse.up();
  expect(await page.evaluate(()=>localStorage.getItem('tagrex.autofitColumns'))).toBe('0');
});

test('auto-fit measures contents rather than distributing spare viewport width',async({page})=>{
  await page.setViewportSize({width:1600,height:800});
  await page.locator('#panel-toggle').click();
  await page.locator('#tracks th[data-sort="file"]').click({button:'right'});
  await page.locator('#columns-menu .column-fit-action').click();
  expect(await page.locator('#tracks th[data-sort="file"]').evaluate(n=>parseFloat(n.style.width))).toBeLessThan(160);
  expect((await page.locator('#tracks-body .table-fill').first().boundingBox()).width).toBeGreaterThan(100);
});

test('thumb dragging keeps the current raw cell mounted and editable',async({page})=>{
  await page.evaluate(async()=>{await window.tagrex.invoke('open_drop',{paths:Array.from({length:53},(_,i)=>`album-${i}.mp3`)});document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();});
  const cell=page.locator('#tracks-body td[data-field="title"]').first();await cell.dblclick();
  await page.evaluate(()=>{window.rawCell=document.querySelector('#tracks-body [contenteditable="true"]');window.rawCell.textContent='Uncommitted raw text';});
  const bar=page.getByRole('scrollbar',{name:'Scroll tracks'});
  await bar.hover();const thumb=await bar.locator('.table-scrollbar-thumb').boundingBox();
  await page.mouse.move(thumb.x+3,thumb.y+thumb.height/2);await page.mouse.down();await page.mouse.move(thumb.x+3,thumb.y+thumb.height/2+30);await page.mouse.up();
  // Album rows remain mounted while the native scroll position changes.
  expect(await page.evaluate(()=>window.rawCell.isConnected&&document.activeElement===window.rawCell&&window.rawCell.isContentEditable&&window.rawCell.textContent==='Uncommitted raw text')).toBe(true);
  expect(await page.locator('#files-view').evaluate(n=>n.scrollTop)).toBeGreaterThan(0);
});
