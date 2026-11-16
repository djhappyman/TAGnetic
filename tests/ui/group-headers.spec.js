import {test, expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {resolve, sep} from 'node:path';
import {openTagSource} from './navigation.js';
const root=resolve('.test-dist');
const common=['folder','album','albumartist','artist','disc','year','genre','media'];
test.beforeEach(async({page})=>{
  page.groupErrors=[];page.on('pageerror',e=>page.groupErrors.push(e.message));
  await page.route('http://tagrex.test/**',async route=>{
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);
    if(!path.startsWith(root+sep))return route.abort();
    try{await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});}catch{await route.fulfill({status:404,body:'Missing fixture'});}
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async()=>{document.getElementById('root').value='/music';await window.tagrex.openLibrary();});
});
test.afterEach(async({page})=>expect(page.groupErrors).toEqual([]));
async function group(page,key){await page.locator('#group-btn').click();await page.locator(`#group-menu [data-group="${key}"]`).click();}
async function folders(page,paths){
  await page.evaluate(async paths=>{await window.tagrex.invoke('open_drop',{paths});document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();},paths);
  await group(page,'folder');
}
test('eight icon choices lead the menu and More exposes secondary choices by keyboard',async({page})=>{
  await page.locator('#group-btn').click();
  expect(await page.locator('#group-menu .group-opt:visible').evaluateAll(nodes=>nodes.map(n=>n.dataset.group))).toEqual(['',...common]);
  await expect(page.locator('#group-more-options')).toBeHidden();
  const more=page.locator('#group-menu .group-more');await more.focus();await page.keyboard.press('Enter');
  await expect(more).toHaveAttribute('aria-expanded','true');
  await expect(page.locator('#group-more-options [data-group="release"]')).toBeVisible();
  await page.locator('#group-more-options [data-group="publisher"]').focus();await page.keyboard.press('Enter');
  await expect(page.locator('#group-menu')).toBeHidden();await expect(page.locator('#group-btn')).toBeFocused();
  expect(await page.evaluate(()=>localStorage.getItem('tagrex.groupBy'))).toBe('publisher');
});
test('menu and header icons match while regrouping preserves a pending manual draft',async({page})=>{
  await openTagSource(page,'editor');const title=page.locator('#fields-body input[data-key="title"]');await title.fill('Keep this group draft');
  const selected=await page.evaluate(()=>[...window.tagrex.selection]);const paths=await page.evaluate(()=>window.tagrex.tracks.map(t=>t.path));
  for(const key of common){
    await page.locator('#group-btn').click();
    const icon=await page.locator(`#group-menu [data-group="${key}"] .ico use`).getAttribute('href');
    await page.locator(`#group-menu [data-group="${key}"]`).click();
    await expect(page.locator('#tracks-body .group-type-icon use').first()).toHaveAttribute('href',icon);
    await expect(title).toHaveValue('Keep this group draft');
    expect(await page.evaluate(()=>[...window.tagrex.selection])).toEqual(selected);
    expect(await page.evaluate(()=>window.tagrex.tracks.map(t=>t.path))).toEqual(paths);
  }
});
test('folder headers trim the full-library prefix and remain stable when filtering',async({page})=>{
  await folders(page,['Music/Music Testing/Bonkers/Pack/CD1/one.mp3','Music/Music Testing/Bonkers/Pack/CD1/two.mp3','Music/Music Testing/Bonkers/Pack/CD2/three.mp3','Music/Music Testing/HTID/Set/CD1/four.mp3']);
  const labels=page.locator('#tracks-body .group-label');
  expect(await labels.allTextContents()).toEqual(['Bonkers/Pack/CD1','Bonkers/Pack/CD2','HTID/Set/CD1']);
  const first=labels.first();const details=await first.evaluate(n=>n.title||n.getAttribute('aria-description'));
  expect(details).toContain('Music/Music Testing/Bonkers/Pack/CD1');
  const key=await page.locator('#tracks-body .group-head').first().getAttribute('data-group');
  expect(key).toContain('Music/Music Testing/Bonkers/Pack/CD1');
  await page.locator('#filter').fill('one.mp3');await expect(labels).toHaveCount(1);await expect(labels).toHaveText('Bonkers/Pack/CD1');
  await page.locator('#filter-clear').click();await expect(labels).toHaveCount(3);
  await folders(page,['Elsewhere/New Library/CD1/a.mp3','Elsewhere/New Library/CD2/b.mp3']);
  expect(await labels.allTextContents()).toEqual(['CD1','CD2']);
});
for(const theme of ['dark','light'])test(`count chips follow the group name and grouping icons have contrast in ${theme}`,async({page})=>{
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
  for(const width of [1280,375]){
    await page.setViewportSize({width,height:800});
    const libraryTab=page.locator('.dock-module-tab[aria-controls="dock-module-files"]');
    if(await libraryTab.isVisible())await libraryTab.click();
    await expect(page.locator('#files-view')).toBeVisible();
    await expect.poll(()=>page.locator('#files-view').evaluate(n=>n.clientWidth)).toBeGreaterThan(20);
    await expect.poll(()=>page.locator('.group-count').first().evaluate(n=>n.getBoundingClientRect().right<=document.getElementById('files-view').getBoundingClientRect().right)).toBe(true);
    const pane=await page.locator('#files-view').boundingBox();const badge=await page.locator('.group-count').first().boundingBox();
    expect(badge.x+badge.width).toBeLessThanOrEqual(pane.x+pane.width);
    const label=await page.locator('.group-label').first().boundingBox();
    expect(badge.x-label.x-label.width).toBeCloseTo(8,0);
    const style=await page.locator('.group-count').first().evaluate(n=>({background:getComputedStyle(n).backgroundColor,color:getComputedStyle(n).color}));
    expect(style.background).not.toBe('rgba(0, 0, 0, 0)');expect(style.background).not.toBe(style.color);
    const ink=await page.locator('.group-type-icon').first().evaluate(n=>getComputedStyle(n).color);
    expect(ink).not.toBe('rgb(255, 255, 255)');expect(ink).not.toBe('rgba(0, 0, 0, 0)');
    await page.locator('#files-view').evaluate(n=>{n.scrollLeft=200;});
    await expect(page.locator('.group-count').first()).toBeInViewport();
  }
});
