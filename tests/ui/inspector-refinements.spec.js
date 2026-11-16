import { openTagSource } from './navigation.js';
import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
const errors = new WeakMap();
test.use({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' });
test.beforeEach(async ({ page }) => {
  const issues = []; errors.set(page, issues); page.on('pageerror', error => issues.push(error.message));
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await page.waitForFunction(() => !!window.tagrex);
  await page.locator('#root-display').click(); await page.locator('#root').fill('/music'); await page.locator('#root').press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
});
test.afterEach(async ({ page }) => { expect(errors.get(page)).toEqual([]); });
async function selectAll(page) {
  await page.locator('#tracks-body tr[data-path]').first().click();
  await page.locator('#tracks-body tr[data-path]').last().click({ modifiers: ['Shift'] });
  await openTagSource(page, 'editor');
}


test('compilation returns absent and explicit false flags to their exact saved values',async({page})=>{
  await page.evaluate(()=>{const [a,b,c]=window.tagrex.tracks;a.tags[a.compilation_field]='';b.tags[b.compilation_field]='0';c.tags[c.compilation_field]='false';});
  await selectAll(page);const flag=page.locator('#editor-compilation-control');
  await flag.click();await expect(page.locator('#staged-edits')).toBeVisible();
  await flag.click();await expect(page.locator('#staged-edits')).toBeHidden();
  await expect(page.locator('#diff-apply')).toBeDisabled();await expect(page.locator('.fe-compilation')).not.toHaveClass(/dirty/);
  expect(await page.evaluate(()=>window.tagrex.previewPlan?.changes?.length||0)).toBe(0);
});
test('typing the saved value clears both staging and dirty input styling',async({page})=>{
  await page.locator('#tracks-body tr[data-path]').first().click();await openTagSource(page, 'editor');
  for(const key of ['title','year','bpm']){
    const input=page.locator(`#editor-field-${key}`),old=await input.inputValue();
    await input.fill(key==='title'?'Changed title':'2025');await expect(page.locator('#staged-edits')).toBeVisible();
    // Cover the validated state too, not only reverting before the debounce.
    await expect(page.locator('#diff-apply')).toBeEnabled();
    await input.fill(old);await expect(page.locator('#staged-edits')).toBeHidden();
    await expect(input).not.toHaveClass(/dirty/);await expect(page.locator('#diff-apply')).toBeDisabled();
  }
});

test('returning compilation to true restores each saved spelling; discard restores a mixed flag',async({page})=>{
  await page.evaluate(()=>{const values=['1','true','yes'];window.tagrex.tracks.forEach((track,i)=>track.tags[track.compilation_field]=values[i]);});
  await selectAll(page);const flag=page.locator('#editor-compilation-control');
  await flag.click();await expect(page.locator('#staged-edits')).toBeVisible();
  await flag.click();await expect(page.locator('#staged-edits')).toBeHidden();
  await page.locator('#tracks-body tr[data-path]').first().click();
  await page.evaluate(()=>window.tagrex.tracks[1].tags[window.tagrex.tracks[1].compilation_field]='0');
  await selectAll(page);await expect(flag).toHaveAttribute('aria-checked','mixed');
  await flag.click();await expect(page.locator('#staged-edits')).toBeVisible();
  await page.locator('#diff-discard').click();
  await expect(flag).toHaveAttribute('aria-checked','mixed');await expect(page.locator('#staged-edits')).toBeHidden();
});

test('discard restores number and text values without leaving phantom staging',async({page})=>{
  await page.locator('#tracks-body tr[data-path]').first().click();await openTagSource(page, 'editor');
  const keys=['title','year','track','disc','bpm'],saved={};
  for(const key of keys){saved[key]=await page.locator(`#editor-field-${key}`).inputValue();await page.locator(`#editor-field-${key}`).fill(key==='title'?'Pending':'12');}
  // Discard also works before validation (the short year here is invalid).
  await expect(page.locator('#staged-edits')).toBeVisible();await page.locator('#diff-discard').click();
  for(const key of keys)await expect(page.locator(`#editor-field-${key}`)).toHaveValue(saved[key]);
  await expect(page.locator('#diff-apply')).toBeDisabled();
  await expect(page.locator('#staged-edits')).toBeHidden();await expect(page.locator('.field-input.dirty')).toHaveCount(0);
});

test('help keeps the bottom fade and lock buttons sit inside complete field surfaces',async({page})=>{
  await selectAll(page);const port=page.locator('.editor-scroll');
  await port.evaluate(node=>{node.style.maxHeight='350px';node.scrollTop=0;});
  await expect(port).toHaveClass(/can-scroll-down/);
  await page.locator('#subtab-editor .inspector-help').click();await expect(page.locator('.inspector-help-content')).toBeVisible();
  expect(await port.evaluate(node=>getComputedStyle(node).maskImage)).toContain('linear-gradient');
  await page.keyboard.press('Escape');expect(await port.evaluate(node=>getComputedStyle(node).maskImage)).toContain('linear-gradient');
  for(const key of ['title','year','bpm','key','genre']){
    const input=page.locator(`#editor-field-${key}`),row=input.locator('xpath=ancestor::div[contains(@class,"fe-row")]');
    await input.scrollIntoViewIfNeeded();await row.hover();
    const field=await input.boundingBox(),lock=await row.locator('.fe-lock').boundingBox();
    expect(lock.x).toBeGreaterThanOrEqual(field.x);expect(lock.x+lock.width).toBeLessThanOrEqual(field.x+field.width+1);
    const cell=await row.locator('.fe-cell').boundingBox();expect(field.width).toBeCloseTo(cell.width,0);
  }
});

for(const theme of ['light','dark'])test(`mixed values and rounded disclosure hierarchy stay distinct (${theme})`,async({page},info)=>{
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);await selectAll(page);
  const input=page.locator('#editor-field-title');
  const colors=await input.evaluate(node=>{const s=getComputedStyle(node),p=getComputedStyle(node,'::placeholder'),probe=document.createElement('span');probe.style.color='var(--field-ghost)';node.parentElement.append(probe);const ghost=getComputedStyle(probe).color;probe.remove();return {value:s.color,placeholder:p.color,ghost};});
  expect(colors.placeholder).toBe(colors.ghost);expect(colors.placeholder).not.toBe(colors.value);
  const group=page.locator('[data-group="standard"]'),head=group.locator('.fe-group-head');
  await expect(group.locator('.fe-group-count')).toHaveCount(0);await head.click();
  const geometry=await group.evaluate(node=>{const h=node.querySelector('.fe-group-head'),grid=node.querySelector('.fe-grid');return {radius:getComputedStyle(h).borderRadius,background:getComputedStyle(h).backgroundColor,gap:grid.getBoundingClientRect().top-h.getBoundingClientRect().bottom};});
  expect(geometry.radius).toBe('10px');expect(geometry.background).not.toBe('rgba(0, 0, 0, 0)');expect(geometry.gap).toBeCloseTo(12,0);
  await page.locator('.mode-col').screenshot({path:info.outputPath(`inspector-${theme}.png`)});
});

for (const keys of [
  ['title','artist','genre','comment','catalognumber','isrc','custom:TEST_CLEAR'],
  ['track','tracktotal','disc','disctotal','year','bpm','key'],
]) test(`mixed fields can explicitly remove their tags: ${keys.join(', ')}`,async({page})=>{
  await page.evaluate(keys=>window.tagrex.tracks.forEach((track,i)=>{
    for(const key of keys)track.tags[key]=i===0?'':key==='key'?['','Am','Dm'][i]:key.match(/track|disc|year|bpm/)?String(2000+i):`Value ${i}`;
  }),keys);
  await selectAll(page);
  for(const group of ['standard','advanced']){
    const node=page.locator(`[data-group="${group}"]`);if(await node.count())await node.locator('.fe-group-head').click();
  }
  for(const key of keys){
    const input=page.locator(`[id="editor-field-${encodeURIComponent(key)}"]`),clear=page.locator(`[id="editor-field-${encodeURIComponent(key)}-clear"]`);
    await input.scrollIntoViewIfNeeded();await input.hover({position:{x:4,y:16}});await expect(input).toHaveClass(/multiple/);
    await expect(clear).toHaveCSS('opacity','1');await expect(clear).toBeEnabled();
    const before=await input.boundingBox();await clear.click();
    await expect(input).toHaveValue('');await expect(input).not.toHaveClass(/multiple/);await expect(input).toHaveAttribute('placeholder','');
    await expect(input).toBeFocused();await expect(clear).toBeHidden();
    await expect.poll(()=>page.evaluate(key=>window.tagrex.previewPlan?.changes.filter(change=>change.tag_changes.some(field=>field.field===key&&field.new==null)).length||0,key)).toBe(2);
    await expect(page.locator('#diff-apply')).toBeEnabled();
    const after=await input.boundingBox();expect(after.width).toBeCloseTo(before.width,0);
    if(key==='genre')await expect(page.locator('.genre-options')).toBeHidden();
    await page.locator('#diff-discard').click();
    await expect(input).toHaveClass(/multiple/);await expect(page.locator('#staged-edits')).toBeHidden();
  }
});

test('clear remains keyboard accessible and respects empty and locked fields',async({page})=>{
  await selectAll(page);const input=page.locator('#editor-field-title'),clear=page.locator('#editor-field-title-clear');
  await input.focus();await expect(clear).toHaveCSS('opacity','1');
  await page.keyboard.press('Tab');await expect(clear).toBeFocused();await page.keyboard.press('Enter');
  await expect(input).toBeFocused();await expect(clear).toBeHidden();await expect(page.locator('#staged-edits')).toBeVisible();
  await page.locator('#diff-discard').click();
  const row=input.locator('xpath=ancestor::div[contains(@class,"fe-row")]');await row.hover();await row.locator('.fe-lock').click();
  await expect(input).toBeDisabled();await expect(clear).toBeDisabled();await expect(clear).toBeHidden();
  await row.locator('.fe-lock').click();await expect(input).toBeEnabled();await input.focus();
  await input.press('Backspace');await expect(input).not.toHaveClass(/multiple/);await expect(page.locator('#staged-edits')).toBeVisible();
});

for(const theme of ['dark','light'])test(`clear and lock actions fit the fields without resizing (${theme})`,async({page},info)=>{
  await page.evaluate(theme=>{
    document.documentElement.dataset.theme=theme;
    window.tagrex.tracks.forEach((track,i)=>{for(const key of ['title','track','tracktotal','disc','disctotal','year','bpm','key','genre','comment'])track.tags[key]=/track|disc|year|bpm/.test(key)?String(2000+i):`Value ${i}`;});
  },theme);
  await selectAll(page);
  await page.locator('.mode-col').evaluate(node=>{node.style.width='360px';node.style.flex='0 0 360px';});
  for(const key of ['title','track','tracktotal','disc','disctotal','year','bpm','key','genre','comment']){
    const input=page.locator(`#editor-field-${key}`),clear=page.locator(`#editor-field-${key}-clear`);
    await input.scrollIntoViewIfNeeded();const before=await input.boundingBox();await input.hover({position:{x:4,y:16}});await expect(clear).toHaveCSS('opacity','1');
    const after=await input.boundingBox(),button=await clear.boundingBox();
    expect(after.width).toBeCloseTo(before.width,0);expect(button.width).toBeGreaterThanOrEqual(24);
    expect(button.x).toBeGreaterThanOrEqual(after.x);expect(button.x+button.width).toBeLessThanOrEqual(after.x+after.width+1);
    const lock=input.locator('xpath=ancestor::div[contains(@class,"fe-row")]').locator('.fe-lock');
    if(await lock.evaluate(node=>getComputedStyle(node).opacity==='1')){
      const bounds=await lock.boundingBox();expect(button.x+button.width<=bounds.x+1||bounds.x+bounds.width<=button.x+1).toBe(true);
    }
  }
  await page.locator('#editor-field-title').hover();
  await page.locator('.mode-col').screenshot({path:info.outputPath(`field-clear-${theme}.png`)});
});
