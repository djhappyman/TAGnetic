import { openTagSource } from './navigation.js';
import {test,expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
const root=resolve('.test-dist');
test.beforeEach(async({page})=>{
  page.__selectionErrors=[];page.on('pageerror',error=>page.__selectionErrors.push(error.message));
  await page.route('http://tagrex.test/**',async route=>{
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);if(!path.startsWith(root+sep))return route.abort();
    try{await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});}catch{await route.fulfill({status:404,body:'Missing fixture'});}
  });
  await page.goto('http://tagrex.test/app/ui/index.html');await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.locator('#root-display').click();await page.locator('#root').fill('/music');await page.locator('#root').press('Enter');await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
});
test.afterEach(async({page})=>expect(page.__selectionErrors).toEqual([]));
const rows=page=>page.locator('#tracks-body tr[data-path]');
async function editor(page){await openTagSource(page, 'editor');}
async function preview(page,text){await page.locator('#fields-body [data-key="title"]').fill(text);await expect(page.locator('#diff-apply')).toBeEnabled();}

test('primary pointer down immediately selects and highlights a row without waiting for release',async({page})=>{
  const cell=rows(page).nth(1).locator('td.file');const box=await cell.boundingBox();await page.mouse.move(box.x+80,box.y+box.height/2);await page.mouse.down();
  expect(await rows(page).nth(1).evaluate(node=>node.classList.contains('selected')&&window.tagrex.selection.has(node.dataset.path))).toBe(true);
  await page.mouse.up();
});
test('highlight is the only row selection indicator including live previews',async({page})=>{
  await editor(page);await preview(page,'No checkboxes');
  expect(await rows(page).locator('input[type="checkbox"]').count()).toBe(0);
  await expect(rows(page).first()).toHaveClass(/selected/);
});
test('comparison belongs to Staged and stays enabled across drafts',async({page})=>{
  await editor(page);await preview(page,'First preview');await page.locator('#diff-preview').click();await expect(page.locator('#tracks')).toHaveClass(/show-old/);
  await page.locator('#diff-discard').click();await preview(page,'Second preview');await expect(page.locator('#tracks')).toHaveClass(/show-old/);
  await page.locator('#files-tab').click();await expect(page.locator('#tracks')).not.toHaveClass(/show-old/);
});
test('right-clicking a standard column header opens the column chooser',async({page})=>{
  await page.locator('#tracks thead th[data-sort="title"]').click({button:'right'});await expect(page.locator('#columns-menu')).toBeVisible();
});
test('clicking a group label collapses the group without changing track selection',async({page})=>{
  const before=await page.evaluate(()=>[...window.tagrex.selection]);const group=page.locator('#tracks-body tr.group-head').first();await group.locator('.group-label').click();await expect(group).toHaveClass(/collapsed/);expect(await page.evaluate(()=>[...window.tagrex.selection])).toEqual(before);
});
test('selection handler baseline records synchronous click work with Editor visible',async({page})=>{
  await editor(page);
  const result=await page.evaluate(()=>{const times=[];for(let i=0;i<30;i++){const row=document.querySelectorAll('#tracks-body tr[data-path]')[i%3];const start=performance.now();row.querySelector('td.file').dispatchEvent(new MouseEvent('click',{bubbles:true}));times.push(performance.now()-start);}times.sort((a,b)=>a-b);return{median:times[15],p95:times[28],max:times[29]};});
  console.log('SELECTION_HANDLER_MS',JSON.stringify(result));
});

test('column sorting keeps folder positions and sorts numeric filenames only inside each folder',async({page})=>{
  await page.evaluate(async()=>{
    await window.tagrex.invoke('open_drop',{paths:[
      'Z Folder/take10.mp3','Z Folder/take2.mp3','Z Folder/take9.mp3',
      'A Folder/take10.mp3','A Folder/take2.mp3','A Folder/take9.mp3',
    ]});
    document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();
  });
  const headers=page.locator('#tracks-body tr.group-head');
  const original=await headers.evaluateAll(nodes=>nodes.map(node=>node.dataset.group));
  expect(original).toHaveLength(2);
  for(const key of ['file','title','artist','file']){
    for(let direction=0;direction<2;direction++){
      await page.locator(`#tracks th[data-sort="${key}"]`).click();
      expect(await headers.evaluateAll(nodes=>nodes.map(node=>node.dataset.group))).toEqual(original);
      const groups=await rows(page).evaluateAll(nodes=>nodes.map(node=>node.dataset.group));
      expect(groups).toEqual(original.flatMap(group=>Array(3).fill(group)));
      if(key==='file'){
        const order=await rows(page).evaluateAll(nodes=>nodes.map(node=>node.dataset.path.split('/').pop()));
        const expected=direction===0?['take2.mp3','take9.mp3','take10.mp3']:['take10.mp3','take9.mp3','take2.mp3'];
        expect(order).toEqual([...expected,...expected]);
      }
    }
  }
  await headers.first().locator('.group-caret').click();
  await expect(rows(page)).toHaveCount(3);
  await page.locator('#tracks th[data-sort="title"]').click();
  await expect(rows(page)).toHaveCount(3);
  expect(await headers.evaluateAll(nodes=>nodes.map(node=>node.dataset.group))).toEqual(original);
  await page.locator('#filter').fill('title:A Folder');
  await expect(headers).toHaveCount(1);
  await page.locator('#tracks th[data-sort="file"]').click();
  await page.locator('#filter').fill('');
  await expect(headers).toHaveCount(2);
  expect(await headers.evaluateAll(nodes=>nodes.map(node=>node.dataset.group))).toEqual(original);
  await expect(rows(page)).toHaveCount(3); // collapse state survived filtering and sorting
});

for (const count of [371, 1248]) {
  test(`sorting ${count} expanded files animates only visible rows and preserves the group anchors`, async ({page}) => {
    await page.emulateMedia({reducedMotion:'no-preference'});
    await page.evaluate(async size => {
      const groupCount = size === 371 ? 23 : 2;
      const paths = Array.from({length:size}, (_, index) => `Folder ${index % groupCount}/take${index}.mp3`);
      await window.tagrex.invoke('open_drop', {paths});
      document.getElementById('root').value = '/dropped';
      await window.tagrex.openLibrary();
    }, count);
    const fileHeader = page.locator('#tracks th[data-sort="file"]');
    const probe = await fileHeader.evaluate(header => {
      const table = document.querySelector('#files-view');
      const body = document.querySelector('#tracks-body');
      const before = new Map([...body.querySelectorAll('tr[data-path]')].map(row => [row.dataset.path,row]));
      const groups = [...body.querySelectorAll('tr.group-head')].map(row => row.dataset.group);
      const first = body.querySelector('tr[data-path]');
      first.querySelector('td.file').click();
      const selection = [...window.tagrex.selection];
      const animate = Element.prototype.animate;
      const moving = [];
      let previous = new Map();
      Element.prototype.animate = function (frames, timing) {
        if (this.matches('#tracks-body tr[data-path]')) {
          const rect = this.getBoundingClientRect();
          const top = table.querySelector('thead').getBoundingClientRect().bottom;
          const bottom = table.getBoundingClientRect().bottom;
          const old = previous.get(this.dataset.path);
          moving.push({
            path:this.dataset.path, visible:rect.bottom > top && rect.top < bottom,
            actual:Number.parseFloat(frames[0].transform.slice('translateY('.length)),
            expected:old === undefined ? bottom - rect.top : old - rect.top,
            fade:frames.some(frame => frame.opacity !== undefined), duration:timing.duration,
          });
        }
        return animate.call(this, frames, timing);
      };
      try {
        header.click();
        table.dispatchEvent(new WheelEvent('wheel', {deltaY:10}));
        moving.length = 0;
        const upper = table.querySelector('thead').getBoundingClientRect().bottom;
        const lower = table.getBoundingClientRect().bottom;
        previous = new Map([...body.querySelectorAll('tr[data-path]')].flatMap(row => {
          const rect = row.getBoundingClientRect();
          return rect.bottom > upper && rect.top < lower ? [[row.dataset.path, rect.top]] : [];
        }));
        header.click();
        const rows = [...body.querySelectorAll('tr[data-path]')];
        const rowBottom = rows.at(-1).getBoundingClientRect().bottom;
        const running = rows.flatMap(row => row.getAnimations());
        table.dispatchEvent(new WheelEvent('wheel', {deltaY:10}));
        return {
          moving, mounted: rows.length,
          visible:rows.flatMap(row => {
            const rect = row.getBoundingClientRect();
            const old = previous.get(row.dataset.path);
            return rect.bottom > upper && rect.top < lower && (old === undefined || Math.abs(old - rect.top) > 1) ? [row.dataset.path] : [];
          }),
          fillsViewport: rowBottom >= table.getBoundingClientRect().bottom,
          columnState: header.getAttribute('aria-sort'),
          reused: rows.some(row => before.get(row.dataset.path) === row),
          scoped: selection.length === 1 && window.tagrex.selection.has(selection[0]),
          firstGroup: body.querySelector('tr.group-head').dataset.group,
          previousFirstGroup: groups[0],
          cancelled: running.length > 0 && running.every(animation => animation.playState === 'idle'),
        };
      } finally { Element.prototype.animate = animate; }
    });
    expect(probe.fillsViewport).toBe(true);
    if (count === 371) expect(probe.mounted).toBe(count);
    else expect(probe.mounted).toBeLessThan(96);
    expect(probe.columnState).toBe('descending');
    expect(probe.moving.length).toBeGreaterThan(5);
    expect(probe.moving.map(move => move.path)).toEqual(probe.visible);
    expect(probe.moving.every(move => move.visible && !move.fade && move.duration === 230)).toBe(true);
    expect(probe.moving.some(move => Math.abs(move.actual) > 160)).toBe(true);
    for (const move of probe.moving) expect(move.actual).toBeCloseTo(move.expected, 0);
    if (count === 371) expect(probe.reused).toBe(true);
    expect(probe.scoped).toBe(true);
    expect(probe.firstGroup).toBe(probe.previousFirstGroup);
    expect(probe.cancelled).toBe(true);

    await page.locator('#group-btn').click();
    await page.locator('#group-menu .group-opt[data-group=""]').click();
    await expect(page.locator('#tracks-body tr.group-head')).toHaveCount(0);
    await expect(fileHeader).toHaveAttribute('aria-sort','descending');
    const order = await page.evaluate(() => window.tagrex.tracks.map(track => Number(/take(\d+)\.mp3$/.exec(track.path)[1])));
    expect(order).toEqual(Array.from({length:count},(_, i) => count-1-i));

    await page.emulateMedia({reducedMotion:'reduce'});
    await fileHeader.evaluate(header => header.click());
    expect(await rows(page).evaluateAll(nodes => nodes.flatMap(node => node.getAnimations()).length)).toBe(0);
    await expect(fileHeader).toHaveAttribute('aria-sort','ascending');
  });
}
