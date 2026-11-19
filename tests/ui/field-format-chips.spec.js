import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
const root = resolve('.test-dist');
test.beforeEach(async ({page}) => {
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try { await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'}); }
    catch { await route.fulfill({status:404,body:'Missing fixture'}); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async()=>{document.getElementById('root').value='/music';await window.tagrex.openLibrary();});
  await page.locator('#ribbon-select-all').click();
  await page.locator('#ribbon-format-values').click();
  await expect(page.locator('#ribbon-format-stage')).toBeEnabled();
});
const editor = page => page.locator('#ribbon-format-pattern');
async function choose(page, text, label) {
  await editor(page).pressSequentially(text);
  await expect(page.getByRole('option',{name:label,exact:true})).toBeVisible();
  await page.keyboard.press('Enter');
}
test('delayed suggestions highlight matches, commit title-case chips and leave literal words alone',async({page})=>{
  await editor(page).fill('');await editor(page).pressSequentially('a');
  await expect(page.getByRole('listbox',{name:'Tag suggestions'})).toHaveCount(0);
  await editor(page).pressSequentially('rt');
  await expect(page.getByRole('option',{name:'Artist',exact:true})).toBeVisible();
  await expect(page.getByRole('option',{name:'Artist',exact:true}).locator('mark')).toHaveText('Art');
  await expect(page.getByRole('option',{name:'Album Artist',exact:true})).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(editor(page).locator('.field-pattern-chip')).toHaveText('Artist');
  await expect(editor(page)).toBeFocused();
  await page.keyboard.type(' - ');await choose(page,'album','Album');
  await expect(editor(page).locator('.field-pattern-chip')).toHaveText(['Artist','Album']);
  await page.keyboard.press('Backspace');await expect(editor(page).locator('.field-pattern-chip')).toHaveText('Artist');
  await page.keyboard.press('ControlOrMeta+z');await expect(editor(page).locator('.field-pattern-chip')).toHaveText(['Artist','Album']);
  await editor(page).fill('artist');await expect(page.getByRole('listbox',{name:'Tag suggestions'})).toBeVisible();
  await page.keyboard.press('Escape');await expect(page.locator('#ribbon-format-dialog')).toBeVisible();
  await expect(editor(page).locator('.field-pattern-chip')).toHaveCount(0);
});
test('Build combines current values and stages once without writing stored tags',async({page})=>{
  const before=await page.evaluate(()=>structuredClone(window.tagrex.tracks));
  await page.locator('#ribbon-format-field').selectOption('comment');
  await editor(page).fill('');await choose(page,'album','Album');await page.keyboard.type(' - ');await choose(page,'artist','Artist');
  await expect(page.locator('.format-result-row').first()).toContainText('La Bush - The X Factor');
  await page.locator('#ribbon-format-stage').click();await expect(page.locator('#ribbon-format-dialog')).toBeHidden();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes[0]?.tag_changes.find(edit=>edit.field==='comment')?.new)).toBe('La Bush - The X Factor');
  expect(await page.evaluate(()=>window.tagrex.tracks)).toEqual(before);
});
test('Extract previews multiple destinations and reports unmatched tracks without changing them',async({page})=>{
  // Existing pending titles are read by the native-style preview, not disk only.
  await page.locator('#ribbon-format-field').selectOption('title');
  await editor(page).fill('Hixxy - Toy Town');await page.locator('#ribbon-format-stage').click();
  await expect(page.locator('#ribbon-format-dialog')).toBeHidden();
  await page.locator('#ribbon-format-values').click();await page.getByRole('button',{name:'Extract fields',exact:true}).click();
  await expect(page.locator('.format-result-row').first()).toContainText('Hixxy');
  await expect(page.locator('.format-result-row').first()).toContainText('Toy Town');
  await expect(page.locator('#ribbon-format-stage')).toBeEnabled();await page.locator('#ribbon-format-stage').click();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes[0]?.tag_changes.find(edit=>edit.field==='artist')?.new)).toBe('Hixxy');
  await page.locator('#ribbon-format-values').click();await page.getByRole('button',{name:'Extract fields',exact:true}).click();
  await editor(page).fill('%artist% / %title%');
  await expect(page.locator('#ribbon-format-dialog')).toContainText('3 of 3 files don’t match');
  await expect(page.locator('#ribbon-format-stage')).toBeDisabled();
});
test('paste stays plain, chips survive copy and deletion, and IME does not accept suggestions',async({page})=>{
  await editor(page).focus();await editor(page).press('ControlOrMeta+a');
  await editor(page).evaluate(node=>{
    const transfer=new DataTransfer();transfer.setData('text/plain','%album% - %artist%');transfer.setData('text/html','<img src=x onerror="window.badPaste=true">');
    node.dispatchEvent(new ClipboardEvent('paste',{clipboardData:transfer,bubbles:true,cancelable:true}));
  });
  await expect(editor(page).locator('.field-pattern-chip')).toHaveText(['Album','Artist']);
  expect(await page.evaluate(()=>window.badPaste)).toBeUndefined();
  await page.keyboard.press('End');await page.keyboard.press('Backspace');
  await expect(editor(page).locator('.field-pattern-chip')).toHaveText('Album');
  await editor(page).fill('art');await expect(page.getByRole('listbox',{name:'Tag suggestions'})).toBeVisible();
  await editor(page).evaluate(node=>node.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true})));
  await expect(page.getByRole('listbox',{name:'Tag suggestions'})).toHaveCount(0);
  await editor(page).evaluate(node=>node.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',isComposing:true,bubbles:true,cancelable:true})));
  await expect(editor(page).locator('.field-pattern-chip')).toHaveCount(0);
});
for(const theme of ['light','dark']) test(`format dialog is reachable in ${theme} at a narrow viewport`,async({page},info)=>{
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
  await page.setViewportSize({width:420,height:700});
  await expect(editor(page)).toBeVisible();await expect(page.locator('#ribbon-format-stage')).toBeInViewport();
  await editor(page).fill('art');await expect(page.getByRole('listbox',{name:'Tag suggestions'})).toBeVisible();
  const popup=await page.getByRole('listbox',{name:'Tag suggestions'}).boundingBox();
  expect(popup.x).toBeGreaterThanOrEqual(0);expect(popup.x+popup.width).toBeLessThanOrEqual(420);
  await info.attach(`format-fields-${theme}`,{body:await page.screenshot(),contentType:'image/png'});
});
