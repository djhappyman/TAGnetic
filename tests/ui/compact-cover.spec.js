import { openTagSource } from './navigation.js';
import { test, expect } from '@playwright/test';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
const root = resolve('.test-dist');
test.use({ viewport: { width: 1440, height: 900 }, reducedMotion: 'reduce' });

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const image = (name, color = '#138d75') => ({ mime: 'image/svg+xml', data_base64: btoa(`<svg xmlns="http://www.w3.org/2000/svg" width="600" height="600"><rect width="600" height="600" fill="${color}"/><text x="300" y="310" text-anchor="middle" fill="white" font-size="42">${name}</text></svg>`), kind: 'front', description: name });
    window.__cover = { mode: 'single', calls: [], summaryReads:0, image, summary(paths) {
      this.summaryReads++;
      const images = this.mode === 'empty' ? [] : this.mode === 'multiple' ? [image('Front'), { ...image('Back', '#435c8c'), kind: 'back' }] : [image(this.label || 'Front')];
      const result = { total: paths.length, with_cover: images.length ? paths.length : 0, distinct: this.mode === 'mixed', samples: images, shared_set: this.mode === 'mixed' ? [] : images };
      return this.hold ? new Promise(resolve => (this.pending ??= []).push(() => resolve(result))) : result;
    } };
    const decode = HTMLImageElement.prototype.decode;
    HTMLImageElement.prototype.decode = function() {
      const ready = decode.call(this);
      return window.__cover.holdDecode ? ready.then(() => new Promise(resolve => (window.__cover.pendingDecodes ??= []).push(resolve))) : ready;
    };
  });
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      let body = await readFile(path);
      if (path.endsWith('.js')) {
        body = body.toString().replace(/case\s*["'`]read_cover_summary["'`]\s*:\s*\{\s*let\s+\w+=(\w+)\.paths/, (match, args) => match.replace('{', `{ return Promise.resolve(window.__cover.summary(${args}.paths));`));
        body = body.replace(/case\s*["'`]read_external_cover["'`]\s*:/, '$& return Promise.resolve(null);');
        body = body.replace(/case\s*["'`]read_cover_image["'`]\s*:/, '$& window.__cover.calls.push("read_cover_image");');
      }
      await route.fulfill({ body, contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : 'text/html' });
    } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.__coverErrors = errors;
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
  await page.locator('#root-display').click();
  await page.locator('#root').fill('/music');
  await page.locator('#root').press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
  await openTagSource(page, 'editor');
  await expect(page.locator('.cover-main-image img')).toHaveAttribute('alt', 'Front');
});
test.afterEach(async ({ page }) => expect(page.__coverErrors).toEqual([]));
const nextRow = page => page.locator('#tracks-body tr[data-path] .file').nth(1).click();
const openMenu = page => page.locator('#cover-well .cover-edit').click();

test('another cell keeps the current cover mounted until its identical artwork is ready without progress text',async({page})=>{
  await page.locator('#editor-field-album').fill('Staged album');
  await expect(page.locator('#staged-tab-count')).toHaveText('1');
  await page.evaluate(()=>{
    window.__cover.hold=true;
    window.__cover.originalImage=document.querySelector('.cover-main-image img');
    window.__cover.rows=[...document.querySelectorAll('#tracks-body tr[data-path]')];
    window.__cover.inputs=[...document.querySelectorAll('#fields-body .field-input')];
    window.__cover.removedImages=0;
    window.__cover.coverObserver=new MutationObserver(records=>{for(const record of records)for(const node of record.removedNodes)if(node===window.__cover.originalImage||node.contains?.(window.__cover.originalImage))window.__cover.removedImages++;});
    window.__cover.coverObserver.observe(document.getElementById('cover-well'),{childList:true,subtree:true});
  });
  await page.locator('#tracks-body td[data-field="title"]').nth(1).click();
  await expect.poll(()=>page.evaluate(()=>window.__cover.pending?.length || 0)).toBe(1);
  expect(await page.evaluate(()=>window.__cover.originalImage.isConnected)).toBe(true);
  await expect(page.locator('#cover-well')).not.toContainText('Loading');
  await page.evaluate(()=>{window.__cover.hold=false;window.__cover.pending.shift()();});
  await expect(page.locator('#cover-well')).not.toHaveAttribute('aria-busy','true');
  const result=await page.evaluate(()=>{
    window.__cover.coverObserver.disconnect();
    const inputs=[...document.querySelectorAll('#fields-body .field-input')];
    return{sameImage:window.__cover.originalImage===document.querySelector('.cover-main-image img'),removedImages:window.__cover.removedImages,sameRows:window.__cover.rows.every(row=>row.isConnected),sameInputs:inputs.length===window.__cover.inputs.length&&inputs.every((node,i)=>node===window.__cover.inputs[i])};
  });
  expect(result).toEqual({sameImage:true,removedImages:0,sameRows:true,sameInputs:true});
});

test('selection across different custom tag layouts retains Inspector scroll, cover position and core inputs',async({page})=>{
  await page.evaluate(()=>{
    window.tagrex.tracks[0].tags['custom:TEXT']='Writer';
    window.tagrex.tracks[1].tags['custom:TEXT']='Writer';
    window.tagrex.tracks[1].tags['custom:TPE4']='Remixer';
  });
  await nextRow(page);
  await page.locator('#tracks-body td[data-field="title"]').first().click();
  await expect(page.locator('#editor-field-title')).toHaveValue('Desert Rain');
  await page.evaluate(()=>{
    const scroller=document.querySelector('.editor-scroll');scroller.scrollTop=80;
    window.__cover.scroll=scroller.scrollTop;
    window.__cover.top=document.getElementById('cover-well').getBoundingClientRect().top;
    window.__cover.titleInput=document.getElementById('editor-field-title');
  });
  for(const index of [1,0,1]){
    await page.locator('#tracks-body td[data-field="title"]').nth(index).click();
    await expect(page.locator('#editor-field-title')).toHaveValue(await page.evaluate(index=>window.tagrex.tracks[index].tags.title,index));
    await expect(page.locator('#cover-well')).not.toHaveAttribute('aria-busy','true');
    const current=await page.evaluate(()=>({scroll:document.querySelector('.editor-scroll').scrollTop,top:document.getElementById('cover-well').getBoundingClientRect().top,sameInput:window.__cover.titleInput===document.getElementById('editor-field-title')}));
    expect(current.scroll).toBe(await page.evaluate(()=>window.__cover.scroll));
    expect(current.top).toBeCloseTo(await page.evaluate(()=>window.__cover.top),0);
    expect(current.sameInput).toBe(true);
  }
  await page.locator('#editor-field-title').fill('Only the newly selected file');
  await expect(page.locator('#tracks-body td[data-field="title"]').nth(1)).toContainText('Only the newly selected file');
  await expect(page.locator('#tracks-body td[data-field="title"]').first()).toContainText('Desert Rain');
});

test('a superseded image decode cannot commit while the newly selected cover is still reading',async({page})=>{
  await page.evaluate(()=>{window.__cover.label='Superseded';window.__cover.holdDecode=true;});
  await nextRow(page);
  await expect.poll(()=>page.evaluate(()=>window.__cover.pendingDecodes?.length || 0)).toBe(1);
  await page.evaluate(()=>{window.__cover.label='Current';window.__cover.hold=true;});
  await page.locator('#tracks-body td[data-field="title"]').nth(2).click();
  await expect.poll(()=>page.evaluate(()=>window.__cover.pending?.length || 0)).toBe(1);
  await page.evaluate(async()=>{window.__cover.pendingDecodes.shift()();await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));});
  await expect(page.locator('.cover-main-image img')).toHaveAttribute('alt','Front');
  await page.evaluate(()=>{window.__cover.hold=false;window.__cover.holdDecode=false;window.__cover.pending.shift()();});
  await expect(page.locator('.cover-main-image img')).toHaveAttribute('alt','Current');
});

test('single and shared cover selections keep the cover footprint and scrolled viewport stable',async({page})=>{
  await page.evaluate(()=>{
    const scroll=document.querySelector('.editor-scroll');scroll.scrollTop=scroll.scrollHeight;
    window.__cover.geometry={scroll:scroll.scrollTop,height:document.getElementById('cover-well').getBoundingClientRect().height,top:document.getElementById('cover-well').getBoundingClientRect().top};
  });
  await page.locator('#tracks-body td[data-field="title"]').nth(1).click({modifiers:['Control']});
  await expect(page.locator('.cover-compact-note')).toContainText('shared');
  const shared=await page.evaluate(()=>({scroll:document.querySelector('.editor-scroll').scrollTop,height:document.getElementById('cover-well').getBoundingClientRect().height,top:document.getElementById('cover-well').getBoundingClientRect().top}));
  expect(shared).toEqual(await page.evaluate(()=>window.__cover.geometry));
  await page.locator('#tracks-body td[data-field="title"]').first().click();
  await expect(page.locator('#cover-well')).not.toHaveAttribute('aria-busy','true');
  const single=await page.evaluate(()=>({scroll:document.querySelector('.editor-scroll').scrollTop,height:document.getElementById('cover-well').getBoundingClientRect().height,top:document.getElementById('cover-well').getBoundingClientRect().top}));
  expect(single).toEqual(await page.evaluate(()=>window.__cover.geometry));
});

test('one centered cover replaces duplicated rows, with actions behind Edit', async ({ page }) => {
  await expect(page.locator('#cover-set')).toBeHidden();
  await expect(page.locator('#cover-well .cover-main-image')).toBeVisible();
  await expect(page.locator('#cover-well .cover-actions')).toHaveCount(0);
  await openMenu(page);
  for (const name of ['Replace', 'Export', 'Remove']) await expect(page.getByRole('menuitem', { name, exact: true })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Manage images', exact: true })).toHaveCount(0);
  await expect(page.locator('.cover-row, .cover-mini')).toHaveCount(0);
  expect(await page.evaluate(() => window.tagrex.previewPlan)).toBeNull();
});

for (const source of ['Inspector','inline Enter']) test(`scalar ${source} edits retain the artwork without rereading it`,async({page})=>{
  await page.evaluate(()=>{
    window.__cover.originalImage=document.querySelector('.cover-main-image img');
    window.__cover.initialReads=window.__cover.summaryReads;
    window.__cover.mutations=0;
    window.__cover.observer=new MutationObserver(records=>{window.__cover.mutations+=records.length;});
    window.__cover.observer.observe(document.getElementById('cover-well'),{subtree:true,childList:true,attributes:true,characterData:true});
  });
  if(source==='Inspector')await page.locator('#editor-field-title').fill('Just a text edit');
  else {
    await page.locator('#tracks-body td[data-field="title"]').first().dblclick();
    await page.keyboard.insertText('Just a text edit');
    await page.keyboard.press('Enter');
  }
  await expect(page.locator('#tracks-body')).toContainText('Just a text edit');
  await page.waitForTimeout(150);
  const result=await page.evaluate(()=>{
    window.__cover.observer.disconnect();
    return{reads:window.__cover.summaryReads-window.__cover.initialReads,mutations:window.__cover.mutations,sameImage:window.__cover.originalImage===document.querySelector('.cover-main-image img')};
  });
  expect(result).toEqual({reads:0,mutations:0,sameImage:true});
});

test('typing and Enter while a cover is loading retain its one in-flight read',async({page})=>{
  await page.evaluate(()=>{window.__cover.hold=true;window.__cover.initialReads=window.__cover.summaryReads;});
  await nextRow(page);
  await expect.poll(()=>page.evaluate(()=>window.__cover.pending?.length || 0)).toBe(1);
  await page.locator('#tracks-body td[data-field="title"]').nth(1).dblclick();
  await page.keyboard.insertText('Edited during cover load');
  await page.keyboard.press('Enter');
  await expect(page.locator('#editor-field-title')).toHaveValue('Edited during cover load');
  await page.waitForTimeout(150);
  expect(await page.evaluate(()=>window.__cover.summaryReads-window.__cover.initialReads)).toBe(1);
  await page.evaluate(()=>{window.__cover.hold=false;for(const finish of window.__cover.pending.splice(0))finish();});
  await expect(page.locator('.cover-main-image img')).toHaveAttribute('alt','Front');
  await expect(page.locator('#cover-well')).not.toHaveAttribute('aria-busy','true');
  await page.locator('#tracks-body tr[data-path] .file').nth(2).click();
  await expect(page.locator('.cover-main-image img')).toHaveAttribute('alt','Front');
});

test('reverse cover responses and a replaced queued selection never leave the latest file loading',async({page})=>{
  const originalTitle=await page.locator('#editor-field-title').inputValue();
  await page.evaluate(()=>{window.__cover.hold=true;window.__cover.label='Second';});
  await nextRow(page);
  await expect.poll(()=>page.evaluate(()=>window.__cover.pending?.length || 0)).toBe(1);
  await page.evaluate(()=>{window.__cover.label='Third';});
  await page.locator('#tracks-body tr[data-path] .file').nth(2).click();
  await expect.poll(()=>page.evaluate(()=>window.__cover.pending.length)).toBe(2);
  await page.evaluate(()=>{window.__cover.label='Latest';});
  await page.locator('#tracks-body tr[data-path] .file').first().click();
  await expect(page.locator('#editor-field-title')).toHaveValue(originalTitle);
  await page.evaluate(()=>window.__cover.pending.shift()());
  await expect.poll(()=>page.evaluate(()=>window.__cover.pending.length)).toBe(2);
  await page.evaluate(()=>window.__cover.pending.pop()());
  await expect(page.locator('.cover-main-image img')).toHaveAttribute('alt','Latest');
  await page.evaluate(()=>window.__cover.pending.shift()());
  await expect(page.locator('.cover-main-image img')).toHaveAttribute('alt','Latest');
  await expect(page.locator('#cover-well')).not.toHaveAttribute('aria-busy','true');
});

test('artwork removal and discard update only the cover projection without rereading saved artwork',async({page})=>{
  const title=await page.locator('#editor-field-title').inputValue();
  const reads=await page.evaluate(()=>window.__cover.summaryReads);
  await openMenu(page);
  await page.getByRole('menuitem',{name:'Remove',exact:true}).click();
  await expect(page.locator('.cover-main-image img')).toHaveCount(0);
  await expect(page.locator('.cover-empty-target')).toBeEnabled();
  await expect(page.locator('#editor-field-title')).toHaveValue(title);
  await page.locator('#diff-discard').click();
  await expect(page.locator('.cover-main-image img')).toHaveAttribute('alt','Front');
  expect(await page.evaluate(()=>window.__cover.summaryReads)).toBe(reads);
});

test('discard restores values while retaining every Inspector input and untouched field',async({page})=>{
  const saved=await page.locator('#editor-field-title').inputValue();
  await page.evaluate(()=>{window.__cover.inputs=[...document.querySelectorAll('#fields-body .field-input')];});
  await page.locator('#editor-field-title').fill('Temporary title');
  await expect(page.locator('#tracks-body')).toContainText('Temporary title');
  await page.evaluate(()=>{
    window.__cover.otherMutations=0;
    window.__cover.otherObserver=new MutationObserver(records=>window.__cover.otherMutations+=records.length);
    for(const input of window.__cover.inputs)if(input.dataset.key!=='title')window.__cover.otherObserver.observe(input,{attributes:true,childList:true,characterData:true});
  });
  await page.locator('#diff-discard').click();
  await expect(page.locator('#editor-field-title')).toHaveValue(saved);
  const result=await page.evaluate(()=>{
    window.__cover.otherObserver.disconnect();
    const next=[...document.querySelectorAll('#fields-body .field-input')];
    return{sameInputs:next.length===window.__cover.inputs.length&&next.every((node,i)=>node===window.__cover.inputs[i]),unrelatedMutations:window.__cover.otherMutations};
  });
  expect(result).toEqual({sameInputs:true,unrelatedMutations:0});
});

test('typing another value does not rewrite unchanged staging counters',async({page})=>{
  await page.locator('#editor-field-title').fill('First proposal');
  await expect(page.locator('#staged-tab-count')).toHaveText('1');
  await page.evaluate(()=>{
    window.__cover.countMutations=0;
    window.__cover.countObserver=new MutationObserver(records=>window.__cover.countMutations+=records.length);
    for(const id of ['staged-count','staged-tab-count'])window.__cover.countObserver.observe(document.getElementById(id),{childList:true,characterData:true,subtree:true});
  });
  await page.locator('#editor-field-title').fill('Second proposal');
  await expect(page.locator('#tracks-body')).toContainText('Second proposal');
  await page.waitForTimeout(150);
  expect(await page.evaluate(()=>{window.__cover.countObserver.disconnect();return window.__cover.countMutations;})).toBe(0);
});

test('lightbox reuses wheel zoom and direct pan, closes with Escape and restores focus', async ({ page }) => {
  await page.locator('.cover-main-image').click();
  const dialog = page.getByRole('dialog', { name: 'Cover art', exact: true });
  await expect(dialog).toBeVisible();
  const canvas = dialog.locator('.aw-viewer');
  await expect(canvas.locator('img')).toBeVisible();
  const old = Number(await canvas.getAttribute('data-scale'));
  await dialog.locator('.aw-surface').hover();
  await page.mouse.wheel(0, -180);
  await expect.poll(async () => Number(await canvas.getAttribute('data-scale'))).toBeGreaterThan(old);
  const box = await canvas.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down(); await page.mouse.move(box.x + box.width / 2 + 55, box.y + box.height / 2 + 25); await page.mouse.up();
  await expect.poll(async () => Number(await canvas.getAttribute('data-pan-x'))).toBeGreaterThan(40);
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(page.locator('.cover-main-image')).toBeFocused();
});

test('empty state opens native picker beside tracks and rejects a stale selected subject', async ({ page }) => {
  await page.evaluate(() => {
    window.__cover.mode = 'empty';
    window.__TAURI__ = { dialog: { open: options => { window.__cover.pickerOptions = options; return new Promise(resolve => window.__cover.pick = resolve); } } };
  });
  await nextRow(page);
  await expect(page.locator('.cover-empty-target')).toBeEnabled();
  expect(await page.locator('.cover-empty-target').evaluate(node => [getComputedStyle(node).borderRadius,getComputedStyle(node).borderTopStyle])).toEqual(['8px','dashed']);
  await page.locator('.cover-empty-target').click();
  await expect.poll(() => page.evaluate(() => window.__cover.pickerOptions?.defaultPath)).toBe('/music/');
  await page.locator('#tracks-body tr[data-path] .file').nth(2).click();
  await page.evaluate(() => window.__cover.pick('/music/cover.png'));
  await page.waitForTimeout(30);
  expect(await page.evaluate(() => window.__cover.calls)).toEqual([]);
  expect(await page.evaluate(() => window.tagrex.previewPlan)).toBeNull();
});

test('native cancellation is inert and a valid image only stages a preview', async ({ page }) => {
  await page.evaluate(() => {
    window.__cover.mode = 'empty';
    window.__TAURI__ = { dialog: { open: options => { window.__cover.pickerOptions = options; return Promise.resolve(window.__cover.picked ?? null); } } };
  });
  await nextRow(page);
  await page.locator('.cover-empty-target').click();
  await expect.poll(() => page.evaluate(() => window.__cover.pickerOptions?.defaultPath)).toBe('/music/');
  expect(await page.evaluate(() => window.tagrex.previewPlan)).toBeNull();
  await page.evaluate(() => window.__cover.picked = '/music/cover.png');
  await page.locator('.cover-empty-target').click();
  await expect.poll(() => page.evaluate(() => window.__cover.calls.length)).toBe(1);
  await expect.poll(() => page.evaluate(() => !!window.tagrex.previewPlan)).toBe(true);
  expect(await page.evaluate(() => window.tagrex.tracks[1].covers?.length || 0)).toBe(0);
});

test('extra embedded pictures stay out of the main cover UI and mixed selections remain explicit', async ({ page }) => {
  await page.evaluate(() => window.__cover.mode = 'multiple'); await nextRow(page);
  await expect(page.locator('.cover-mini')).toHaveCount(0);
  await expect(page.locator('.cover-main-image img')).toHaveAttribute('alt', 'Front');
  await expect(page.locator('#cover-set')).toBeHidden();
  await page.evaluate(() => window.__cover.mode = 'mixed');
  await page.locator('#tracks-body tr[data-path] .file').nth(2).click();
  await expect(page.locator('.cover-compact-note')).toContainText('Multiple covers');
  await openMenu(page);
  await expect(page.getByRole('menuitem', { name: 'Manage images', exact: true })).toHaveCount(0);
});

for (const [width, theme] of [[1440, 'dark'], [1440, 'light'], [375, 'dark']]) {
  test(`compact artwork capture ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(value => { document.documentElement.dataset.theme = value; }, theme);
    if (width < 600) await page.locator('.dock-module-tab').filter({ hasText: 'Inspector' }).click();
    await page.locator('.cover-main-image').scrollIntoViewIfNeeded();
    expect(await page.locator('.cover-main-image').evaluate(node => getComputedStyle(node).borderRadius)).toBe('8px');
    expect(await page.locator('.mode-col').evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
    if (process.env.ITERATION_CAPTURE_EVIDENCE === '1') { await mkdir('docs/ui-iteration', { recursive: true }); await page.screenshot({ path: `docs/ui-iteration/cover-${width}-${theme}.png` }); }
    await page.locator('.cover-main-image').click();
    await expect(page.locator('.cover-lightbox .aw-hero')).toBeVisible();
    if (process.env.ITERATION_CAPTURE_EVIDENCE === '1') await page.screenshot({ path: `docs/ui-iteration/cover-lightbox-${width}-${theme}.png` });
    const box = await page.locator('.cover-lightbox').boundingBox();
    expect(box.x).toBeGreaterThanOrEqual(0); expect(box.x + box.width).toBeLessThanOrEqual(width + 1);
  });
}

test('artwork hover and drop feedback remain visible for filled and empty covers', async ({ page }) => {
  const edit=page.locator('#cover-well .cover-edit');
  const before=await edit.evaluate(node=>getComputedStyle(node).backgroundColor);
  await edit.hover();
  await expect.poll(()=>edit.evaluate(node=>getComputedStyle(node).backgroundColor)).not.toBe(before);
  for(const empty of [false,true]) {
    if(empty){await page.evaluate(()=>{window.__cover.mode='empty';});await nextRow(page);}
    const target=page.locator(empty?'.cover-empty-target':'.cover-main-image');
    await expect(target).toBeVisible();
    await page.locator('#cover-well').evaluate(node=>node.classList.add('dragover'));
    expect(await target.evaluate(node=>{const s=getComputedStyle(node);return [s.outlineStyle,s.outlineWidth];})).toEqual(['dashed','2px']);
    await page.locator('#cover-well').evaluate(node=>node.classList.remove('dragover'));
  }
  expect(await page.evaluate(()=>window.tagrex.previewPlan)).toBeNull();
});

for (const [width,height] of [[600,600],[900,300],[300,900]]) {
  test(`artwork keeps its actual ${width}x${height} contour rounded and softly shaded`, async ({page}) => {
    const image=page.locator('.cover-main-image img');
    await image.evaluate((node,{width,height})=>{node.src='data:image/svg+xml;base64,'+btoa(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><rect width="100%" height="100%" fill="#2868ff"/></svg>`);},{width,height});
    await expect.poll(()=>image.evaluate(node=>node.naturalWidth)).toBe(width);
    const metrics=await image.evaluate(node=>{const r=node.getBoundingClientRect(),s=getComputedStyle(node),well=node.closest('#cover-well').getBoundingClientRect();return {width:r.width,height:r.height,radius:s.borderRadius,shadow:s.boxShadow,center:r.x+r.width/2,wellCenter:well.x+well.width/2};});
    expect(metrics.width/metrics.height).toBeCloseTo(width/height,2);
    expect(metrics.radius).toBe('8px');expect(metrics.shadow).toContain('0, 0, 0');
    expect(metrics.shadow).not.toBe('none');expect(metrics.center).toBeCloseTo(metrics.wellCenter,0);
  });
}
