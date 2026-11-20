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

async function mode(page,value,label) {
  await page.locator('[data-proxy-id="rename-destination-mode"]').click();
  await page.getByRole('option',{name:label,exact:true}).click();
  await expect(page.locator('#rename-destination-mode')).toHaveValue(value);
}
async function pattern(page,value) {
  await page.locator('#mask').evaluate((node,value)=>{node.value=value;node.dispatchEvent(new Event('input',{bubbles:true}));},value);
}
test('filename comparison and one fixed Preview preserve the stage and Apply boundary',async({page})=>{
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  expect(await page.locator('.rename-section h3').allTextContents()).toEqual(['Destination','Related files']);
  await expect(page.locator('#panel-renamer #preview')).toHaveCount(0);
  await expect(page.locator('#diff-actionbar #preview')).toBeVisible();
  await expect(page.locator('#diff-preview')).toBeHidden();
  await expect(page.locator('#preview')).toBeEnabled();
  await expect(page.locator('#diff-apply')).toBeDisabled();
  const before=await page.evaluate(()=>structuredClone(window.tagrex.tracks));
  await pattern(page,'New %artist% - %title%');
  await expect(page.locator('#mask-example')).toContainText('New ');
  await expect(page.locator('#rename-filename-heading')).toHaveText('Rename filenames');
  await expect(page.locator('#mask-current')).toHaveCount(0);
  await expect(page.locator('.rename-example-label')).toHaveCount(0);
  expect(await page.evaluate(()=>window.tagrex.previewPlan)).toBeNull();
  await page.locator('#preview').click();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  expect(await page.evaluate(()=>window.tagrex.previewPlan.changes[0].rename_to)).toContain('New ');
  expect(await page.evaluate(()=>window.tagrex.tracks)).toEqual(before);
  await page.locator('.mode-tab[data-mode="exporter"]').click();
  await expect(page.locator('#preview')).toBeHidden();
  await expect(page.locator('#diff-preview')).toBeVisible();
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  expect(await page.locator('#mask').evaluate(node=>node.value)).toBe('New %artist% - %title%');
  await page.locator('#mask-builder-open').click();
  await expect(page.locator('#mb-expr')).toBeVisible();
  await page.locator('#mb-cancel').click();
});
test('keep, move and copy retain destination and cleanup choices with exact native plan flags',async({page})=>{
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await expect(page.locator('#rename-destination-mode')).toHaveValue('keep');
  await expect(page.locator('#reorganize-options')).toBeHidden();
  await expect(page.locator('#carry-extras-option')).toBeHidden();
  await mode(page,'move','Move to folder');
  await expect(page.locator('#move-prune-option')).toBeVisible();
  await expect(page.locator('#carry-extras-option')).toBeVisible();
  page.once('dialog',dialog=>dialog.accept('/chosen/music'));
  await page.locator('#move-dest-pick').click();
  await expect(page.locator('#move-dest')).toHaveValue('/chosen/music');
  await page.locator('#move-prune').evaluate(node=>{node.checked=true;node.dispatchEvent(new Event('change',{bubbles:true}));});
  await page.locator('#preview').click();
  expect(await page.evaluate(()=>window.tagrex.previewPlan.prune_empty_dirs)).toBe(true);
  expect(await page.evaluate(()=>window.tagrex.previewPlan.changes[0].rename_to)).toContain('/chosen/music/');
  await mode(page,'copy','Copy to folder');
  await expect(page.locator('#move-prune-option')).toBeHidden();
  await expect(page.locator('#move-prune')).toBeDisabled();
  await page.locator('#preview').click();
  expect(await page.evaluate(()=>window.tagrex.previewPlan.changes.every(change=>change.copy))).toBe(true);
  expect(await page.evaluate(()=>window.tagrex.previewPlan.prune_empty_dirs)).toBe(false);
  await mode(page,'keep','Keep current folders');
  await expect(page.locator('#reorganize-options')).toBeHidden();
  await mode(page,'move','Move to folder');
  await expect(page.locator('#move-dest')).toHaveValue('/chosen/music');
  await expect(page.locator('#move-prune')).toBeChecked();
  await page.reload();
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await expect(page.locator('#rename-destination-mode')).toHaveValue('move');
  await expect(page.locator('[data-proxy-id="rename-destination-mode"]')).toContainText('Move to folder');
  await expect(page.locator('#move-dest')).toHaveValue('/chosen/music');
});
test('selection changes and errors keep filename comparisons honest without staging',async({page})=>{
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await page.locator('#ribbon-clear-selection').click();
  await expect(page.locator('#preview')).toBeDisabled();
  await expect(page.locator('#mask-example')).toContainText('Select a file');
  const row=page.locator('#tracks-body tr[data-path]').nth(1);
  const path=await row.getAttribute('data-path');
  await row.click();
  await expect(page.locator('#preview')).toBeEnabled();
  const title=await page.evaluate(path=>window.tagrex.tracks.find(track=>track.path===path).tags.title,path);
  await expect(page.locator('#mask-example')).toContainText(title);
  await pattern(page,'%notreal%');
  await expect(page.locator('.rename-example')).toHaveAttribute('data-state','error');
  expect(await page.evaluate(()=>window.tagrex.previewPlan)).toBeNull();
  await pattern(page,'%artist% - %title%');
  await expect(page.locator('.rename-example')).not.toHaveAttribute('data-state','error');
});
test('sidebar and single footer action remain reachable in both themes and a short narrow window',async({page})=>{
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  for(const theme of ['light','dark']) {
    await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
    await page.setViewportSize({width:1100,height:580});
    await mode(page,'move','Move to folder');
    await page.locator('#carry-extras-option').scrollIntoViewIfNeeded();
    await expect(page.locator('#preview')).toBeInViewport();
    await expect(page.locator('#diff-apply')).toBeInViewport();
    const geometry=await page.locator('#panel-renamer').evaluate(node=>({width:node.clientWidth,content:node.scrollWidth}));
    expect(geometry.content).toBeLessThanOrEqual(geometry.width);
    const surfaces=await page.evaluate(()=>({outer:getComputedStyle(document.getElementById('panel-renamer')).backgroundColor,result:getComputedStyle(document.querySelector('.rename-example')).backgroundColor}));
    expect(surfaces.result).not.toBe('rgba(0, 0, 0, 0)');
    await page.locator('#mask').focus();
    await page.locator('#mask').press('Tab');
    expect(await page.evaluate(()=>document.activeElement.getAttribute('aria-label'))).toBe('Placeholder reference');
  }
});

test('placeholder picker mirrors builder chips and opens supported functions at the saved caret',async({page})=>{
  await page.locator('.mode-tab[data-mode="renamer"]').click();
  await pattern(page,'A B');
  await page.locator('#mask').evaluate(node=>{node.focus();node.setSelectionRange(2,2);});
  const trigger=page.locator('.ph-btn[data-ph-target="mask"]');
  await trigger.click();
  await expect(page.locator('#ph-body .ph-tag-chip')).not.toHaveCount(0);
  await expect(page.locator('#ph-body [data-text-function]')).toHaveCount(1);
  expect(await page.locator('#ph-body .ph-dir').allTextContents()).not.toContain('name only');
  await expect(page.locator('#ph-body .ph-group-name').first()).toHaveCSS('text-transform','none');
  await expect(page.locator('#ph-body .ph-group-name').first()).toHaveCSS('font-weight','600');
  await expect(page.locator('#ph-menu .ph-head .field')).toHaveCSS('background-color',await page.locator('.mb-ref-search').evaluate(node=>getComputedStyle(node).backgroundColor));
  await page.locator('#ph-filter').fill('artist');
  await page.locator('#ph-body [data-token="%artist%"] .ph-tag-chip').click();
  expect(await page.locator('#mask').evaluate(node=>node.value)).toBe('A %artist%B');
  await expect(page.locator('#ph-menu')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.locator('#ph-menu')).toBeHidden();
  await expect(trigger).toBeFocused();
  await pattern(page,'%title%');
  await page.locator('#mask').evaluate(node=>{node.focus();node.setSelectionRange(node.value.length,node.value.length);});
  await trigger.click();
  await page.locator('#ph-filter').fill('upper');
  await page.locator('#ph-body [data-text-function="upper"]').click();
  await expect(page.locator('#ph-menu')).toBeHidden();
  await expect(page.locator('[data-function-composer]')).toBeVisible();
  await expect(page.locator('#mask-function-arg-0 [data-pattern]')).toHaveText('Title');
  await page.locator('[data-function-composer]').getByRole('button',{name:'Cancel',exact:true}).click();
  expect(await page.locator('#mask').evaluate(node=>node.value)).toBe('%title%');
  expect(await page.evaluate(()=>window.tagrex.previewPlan)).toBeNull();
});
test('extracting placeholders retain chips and grammar without offering render functions',async({page})=>{
  await openTagSource(page,'filename');
  await page.locator('.ph-btn[data-ph-target="from-name-mask"]').click();
  await expect(page.locator('#ph-body .ph-tag-chip')).not.toHaveCount(0);
  await expect(page.locator('#ph-body [data-text-function]')).toHaveCount(0);
  await expect(page.locator('#ph-body [data-token="%disc:1%%track:2%"] .field-value-chip')).toHaveText(['Disc · 1','Track · 2']);
  await page.locator('#ph-filter').fill('title');
  await expect(page.locator('#ph-body [data-token="%title%"]')).toBeVisible();
  await expect(page.locator('#ph-body .ph-grammar')).toHaveCount(0);
  await page.locator('#ph-filter-clear').click();
  await expect(page.locator('#ph-body .ph-grammar')).toBeVisible();
  await expect(page.locator('.ph-reference-wrap [role="scrollbar"]')).toHaveCount(2);
});
