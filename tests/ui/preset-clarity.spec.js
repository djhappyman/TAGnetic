import { openTagSource } from './navigation.js';
import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('.test-dist');
test.beforeEach(async ({ page }) => {
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      await route.fulfill({ body: path.endsWith('.js') ? (await readFile(path, 'utf8')).replace(/case\s*["'\x60]save_settings["'\x60]:/g, match => match + 'window.presetSaveCalls=(window.presetSaveCalls||0)+1;if(window.failPresetSave)return Promise.reject("Preset save failed");') : await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' });
    } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.setMode)).toBe('function');
  await page.locator('#root-display').click();
  await page.locator('#root').fill('/music');
  await page.locator('#root').press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
  await openTagSource(page,'transformations');await page.locator('#presets-edit').click();
  await expect(page.locator('#pe-editor')).toBeVisible();
  await page.locator('#pe-editor').evaluate(async e=>{await Promise.all(e.getAnimations({subtree:true}).map(a=>a.finished.catch(()=>{})));});
});

async function create(page, name = 'My rules') {
  await page.locator('#pe-new').click();
  await page.locator('#pe-name').fill(name);
  await page.locator('#pe-name').press('Enter');
}
async function add(page, kind) {
  const empty = page.locator('#pe-empty-add');
  await (await empty.isVisible() ? empty : page.locator('#pe-open-add')).click();
  await page.locator(`#pe-rule-picker [data-cmd="${kind}"]`).click();
  await expect(page.locator('#pe-rules .is-expanded')).toHaveCount(1);
}
async function menu(page, cmd) {
  await page.locator('.pe-list-row.active').click({ button:'right' });
  await page.locator(`#pe-preset-menu [data-cmd="${cmd}"]`).click();
}

test('built-in rules explain themselves, expand inline and remain immutable', async ({ page }) => {
  const before = await page.evaluate(() => JSON.stringify(window.tagrex.tracks));
  await expect(page.locator('.pe-list-note')).toHaveCount(0);
  await expect(page.locator('#pe-group-builtin .pe-preset-icon')).toHaveCount(2);
  await expect(page.locator('#pe-rules .is-expanded')).toHaveCount(0);
  await expect(page.locator('#pe-rules .rule-grip, #pe-rules .rule-toggle')).toHaveCount(0);
  await expect(page.locator('#pe-rules-scope')).toHaveText('All tags');
  await expect(page.getByRole('group',{name:'Rules',exact:true})).toHaveAccessibleDescription('All tags');
  for(const target of await page.locator('.pe-rule-target').all()) await expect(target).toBeHidden();
  await page.locator('.pe-rule-open').first().click();
  await expect(page.locator('.pe-rule-open').first()).toHaveAttribute('aria-expanded','true');
  await expect(page.locator('.is-expanded .pe-rule-fields-set')).toHaveJSProperty('disabled',true);
  for (const input of await page.locator('.is-expanded input').all()) await expect(input).toBeDisabled();
  await page.locator('#pe-sample-input').fill('  Blue   Moon  ');
  await expect(page.locator('#pe-sample-out')).toHaveText('Blue Moon');
  // Both trim and collapse run; the final result is trimmed as well.
  expect(await page.evaluate(() => JSON.stringify(window.tagrex.tracks))).toBe(before);
});

test('selecting each built-in retains its compact name, icon and actual rules',async({page})=>{
  const rows=page.locator('#pe-group-builtin .pe-list-row');
  for(let i=0;i<await rows.count();i++){
    const before=await rows.nth(i).locator('.pe-list-name').textContent();
    await rows.nth(i).click(); await expect(page.locator('#pe-sample-out')).not.toHaveAttribute('aria-busy','true');
    await expect(page.locator('.pe-list-row.active .pe-list-name')).toHaveText(before);
    expect(await page.locator('#pe-rules .rule-card').count()).toBeGreaterThan(0);
    await expect(page.locator('#pe-empty')).toBeHidden();
  }
});

test('built-in menu offers Copy only, copy is editable and rename stays in sidebar', async ({ page }) => {
  await page.locator('.pe-list-row.active').click({button:'right'});
  await expect(page.locator('#pe-preset-menu [data-cmd="delete"]')).toBeHidden();
  await expect(page.locator('#pe-preset-menu [data-cmd="rename"]')).toBeHidden();
  await page.locator('#pe-preset-menu [data-cmd="copy"]').click();
  await page.locator('#pe-name').fill('My cleanup'); await page.locator('#pe-name').press('Enter');
  await expect(page.locator('.pe-list-row.active .pe-list-name')).toHaveText('My cleanup');
  await expect(page.locator('#pe-rules .rule-toggle')).toHaveCount(2);
  await page.locator('.pe-rule-open').first().click();
  await page.locator('.is-expanded .pe-rule-field input').nth(0).fill('Moon');
  await page.locator('.is-expanded .pe-rule-field input').nth(1).fill('Sun');
  await page.locator('.is-expanded').getByRole('checkbox',{name:'Pattern matching',exact:true}).uncheck();
  await page.locator('#pe-sample-input').fill('Moon Moon');
  await expect(page.locator('#pe-sample-out')).toHaveText('Sun Sun');
  await page.locator('#pe-save-close').click(); await expect(page.locator('#pe-editor')).toBeHidden(); await openTagSource(page,'transformations');await page.locator('#presets-edit').click();
  await expect(page.locator('.pe-list-row.active .pe-list-name')).toHaveText('My cleanup');
  await page.locator('.pe-list-row').nth(1).click();
  await expect(page.locator('.pe-rule-title').first()).toHaveText('Remove repeated spaces');
});

test('empty state adds a rule in one flow and focuses its input', async ({ page }) => {
  await create(page);
  await expect(page.locator('#pe-empty')).toContainText('No rules yet.');
  await expect(page.locator('.pe-preset-heading, #pe-rule-details, .rule-chain-empty')).toHaveCount(0);
  await add(page,'replace');
  await expect(page.locator('.is-expanded input[type="text"]').first()).toBeFocused();
  await expect(page.locator('#pe-empty')).toBeHidden();
  await expect(page.locator('#pe-open-add')).toBeVisible();
});

test('case controls are labelled and enable switches preserve output semantics', async ({ page }) => {
  await create(page); await add(page,'case');
  await page.locator('.pe-case-style').selectOption('lower');
  await page.locator('#pe-sample-input').fill('BLUE MOON');
  await expect(page.locator('#pe-sample-out')).toHaveText('blue moon');
  await expect(page.locator('#pe-rules .seg')).toHaveCount(0);
  await page.locator('#pe-rules .rule-toggle + .app-native-control [role="checkbox"]').click();
  await expect(page.locator('#pe-sample-out')).toHaveText('BLUE MOON');
  await expect(page.locator('#pe-rules .rule-card')).toHaveClass(/rule-disabled/);
});

test('one inline editor expands at a time and typing retains focus and node identity', async ({ page }) => {
  await create(page); await add(page,'replace'); await add(page,'case');
  await page.locator('.pe-rule-open').first().click();
  await expect(page.locator('#pe-rules .is-expanded')).toHaveCount(1);
  const input = page.locator('.is-expanded input[type="text"]').first();
  await input.evaluate(e=>{window._editedInput=e;});
  await input.fill('Moon'); await input.press('End'); await input.press('!');
  await expect(input).toBeFocused();
  expect(await input.evaluate(e=>e===window._editedInput)).toBe(true);
  await page.locator('.pe-rule-open').first().click();
  await expect(page.locator('#pe-rules .is-expanded')).toHaveCount(0);
});

test('new-rule autofocus does not steal focus after the user moves to the sample',async({page})=>{
  await create(page); await add(page,'replace');
  const input=page.locator('#pe-sample-input'); await input.fill('My own sample');
  await input.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  await expect(input).toBeFocused(); await expect(input).toHaveValue('My own sample');
});

test('focus-only navigation cancels new-rule autofocus before its deferred frames run',async({page})=>{
  await create(page);
  await page.evaluate(async()=>{
    const raf=window.requestAnimationFrame,caf=window.cancelAnimationFrame,pending=new Map();let id=1000000;
    window.requestAnimationFrame=callback=>{pending.set(++id,callback);return id;};
    window.cancelAnimationFrame=handle=>{if(!pending.delete(handle))caf(handle);};
    try {
      document.querySelector('#pe-empty-add').click();await Promise.resolve();
      document.querySelector('#pe-rule-picker [data-cmd="replace"]').click();await Promise.resolve();
      // Assistive/programmatic navigation need not emit pointer, key or input events.
      document.querySelector('#pe-sample-input').focus();
      for(let turn=0;turn<4;turn++){
        const batch=[...pending.values()];pending.clear();
        for(const callback of batch)callback(performance.now());
        await Promise.resolve();
      }
    } finally {
      window.requestAnimationFrame=raf;window.cancelAnimationFrame=caf;
      for(const callback of pending.values())raf(callback);
    }
  });
  await expect(page.locator('#pe-sample-input')).toBeFocused();
});

test('matching options are explicit and change the actual result', async ({ page }) => {
  await create(page); await add(page,'replace');
  const fields=page.locator('.is-expanded .pe-rule-field input');
  await fields.nth(0).fill('moon'); await fields.nth(1).fill('sun');
  await page.locator('#pe-sample-input').fill('Moon moon');
  await expect(page.locator('#pe-sample-out')).toHaveText('sun sun');
  await page.getByRole('checkbox',{name:'Match case',exact:true}).click();
  await expect(page.locator('#pe-sample-out')).toHaveText('Moon sun');
  await fields.nth(0).fill('cat'); await fields.nth(1).fill('dog');
  await page.locator('#pe-sample-input').fill('cat catalog');
  await page.getByRole('checkbox',{name:'Whole words',exact:true}).click();
  await expect(page.locator('#pe-sample-out')).toHaveText('dog catalog');
  await fields.nth(0).fill('\\d+'); await fields.nth(1).fill('X');
  await page.locator('#pe-sample-input').fill('Mix 123');
  await page.locator('.pe-pattern-options > summary').click();
  await page.getByRole('checkbox',{name:'Pattern matching',exact:true}).click();
  await expect(page.locator('#pe-sample-out')).toHaveText('Mix X');
});

test('mixed scopes preview the corresponding field rather than combining unrelated rules', async ({ page }) => {
  await create(page); await add(page,'case');
  await page.locator('.is-expanded select.rule-scope').selectOption('artist');
  await page.locator('.is-expanded .pe-case-style').selectOption('upper');
  await add(page,'case');
  await page.locator('.is-expanded select.rule-scope').selectOption('title');
  await page.locator('.is-expanded .pe-case-style').selectOption('lower');
  await expect(page.locator('#pe-rules-scope')).toBeHidden();
  await expect(page.locator('.pe-rule-target').nth(0)).toHaveText('Artist');
  await expect(page.locator('.pe-rule-target').nth(1)).toHaveText('Title');
  for(const target of await page.locator('.pe-rule-target').all()) await expect(target).toBeVisible();
  await page.locator('#pe-preview-field').selectOption('artist');
  await page.locator('#pe-sample-input').fill('Blue Moon');
  await expect(page.locator('#pe-sample-out')).toHaveText('BLUE MOON');
  await page.locator('#pe-preview-field').selectOption('title');
  await expect(page.locator('#pe-sample-out')).toHaveText('blue moon');
  await page.locator('#pe-preview-field').selectOption('artist'); await page.locator('#pe-selection').click();
  const tags=await page.evaluate(()=>window.tagrex.tracks[0].tags);
  await expect(page.locator('#pe-sample-input')).toHaveValue(tags.artist);
  await page.locator('#pe-preview-field').selectOption('title'); await expect(page.locator('#pe-sample-input')).toHaveValue(tags.title);
});

test('extension-only preview reaches rules through the filename API adapter', async ({ page }) => {
  await create(page); await add(page,'case');
  await page.locator('.pe-case-style').selectOption('lower'); await page.locator('select.rule-scope').selectOption('fileext');
  await page.locator('#pe-sample-input').fill('FLAC'); await expect(page.locator('#pe-sample-out')).toHaveText('flac');
});

test('rename confirms or cancels locally; menus close before the dialog', async ({ page }) => {
  await create(page,'Original');
  await test.step('cancel rename without closing the dialog', async()=>{
    await page.locator('.pe-list-row.active').press('F2'); await page.locator('#pe-name').fill('Unwanted'); await page.locator('#pe-name').press('Escape');
    await expect(page.locator('.pe-list-row.active .pe-list-name')).toHaveText('Original'); await expect(page.locator('#pe-editor')).toBeVisible();
  });
  await test.step('keyboard context menu closes first', async()=>{
    await page.locator('.pe-list-row.active').press('Shift+F10'); await expect(page.locator('#pe-preset-menu')).toBeVisible();
    await page.keyboard.press('Escape'); await expect(page.locator('#pe-preset-menu')).toBeHidden(); await expect(page.locator('#pe-editor')).toBeVisible();
  });
  await test.step('rule picker closes first', async()=>{
    await page.locator('#pe-empty-add').click(); await expect(page.locator('#pe-rule-picker small')).toHaveCount(6);
    await page.keyboard.press('Escape'); await expect(page.locator('#pe-editor')).toBeVisible();
  });
  await test.step('field picker closes first', async()=>{
    await add(page,'case'); await page.getByRole('combobox',{name:'Letter case',exact:true}).click();
    await page.keyboard.press('Escape'); await expect(page.locator('#pe-editor')).toBeVisible();
  });
});

test('sidebar groups collapse accessibly and new presets reveal themselves',async({page})=>{
  const toggle=page.getByRole('button',{name:'Built-in',exact:true});
  await toggle.click(); await expect(toggle).toHaveAttribute('aria-expanded','false');
  await expect(page.locator('#pe-group-builtin')).toHaveAttribute('aria-hidden','true');
  await toggle.press('Enter'); await expect(toggle).toHaveAttribute('aria-expanded','true');
  const personal=page.getByRole('button',{name:'Your presets',exact:true}); await personal.click();
  await create(page); await expect(page.getByRole('button',{name:'Your presets',exact:true})).toHaveAttribute('aria-expanded','true');
});

test('field dropdowns stay attached to their control for pointer and keyboard',async({page})=>{
  await create(page); await add(page,'case');
  const trigger=page.getByRole('combobox',{name:'Letter case',exact:true});
  const r=await trigger.boundingBox(); const x=r.x+12,y=r.y+r.height/2;
  await page.mouse.click(x,y); const menu=page.locator('.app-select-menu[data-state="open"]'); await expect(menu).toBeVisible();
  await expect.poll(async()=>{const bounds=await menu.boundingBox();return Math.abs(bounds.x-r.x)+Math.abs(bounds.y-r.y-r.height-4);}).toBeLessThan(2);
  await page.keyboard.press('Escape'); await trigger.focus(); await trigger.press('ArrowDown');
  await expect(menu).toBeVisible(); await page.keyboard.press('Escape'); await expect(trigger).toBeFocused();
});

test('keyboard activation copies a built-in and keeps its original untouched',async({page})=>{
  await page.locator('.pe-list-row.active').press('Shift+F10');
  await expect(page.locator('#pe-preset-menu [data-cmd="copy"]')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('#pe-name')).toBeVisible();
  await page.locator('#pe-name').press('Escape');
  await expect(page.locator('#pe-group-personal .pe-list-row')).toHaveCount(1);
  await expect(page.locator('#pe-group-builtin .pe-list-row')).toHaveCount(2);
});

test('renaming to an existing name stays editable and Escape restores the original',async({page})=>{
  await create(page,'Mine'); await page.locator('.pe-list-row.active').press('F2');
  await page.locator('#pe-name').fill('Standard values'); await page.locator('#pe-name').press('Enter');
  await expect(page.locator('#pe-name')).toHaveAttribute('aria-invalid','true');
  await page.locator('#pe-name').press('Escape'); await expect(page.locator('.pe-list-row.active .pe-list-name')).toHaveText('Mine');
});

test('clicking another preset cancels an unconfirmed rename without swallowing the click',async({page})=>{
  await create(page,'First'); await create(page,'Second');
  await page.locator('#pe-list').getByText('First',{exact:true}).click();
  await page.locator('.pe-list-row.active').press('F2'); await page.locator('#pe-name').fill('Renamed');
  await page.locator('#pe-list').getByText('Second',{exact:true}).click();
  await expect(page.locator('.pe-list-row.active .pe-list-name')).toHaveText('Second');
  await expect(page.locator('#pe-list').getByText('First',{exact:true})).toBeVisible();
  await expect(page.locator('#pe-list').getByText('Renamed',{exact:true})).toHaveCount(0);
});

test('rule action menus attach to the button and fit inside narrow windows',async({page})=>{
  await page.setViewportSize({width:375,height:667}); await create(page);
  const trigger=page.locator('#pe-empty-add'); await trigger.scrollIntoViewIfNeeded();
  const r=await trigger.boundingBox(),x=r.x+10,y=r.y+r.height/2;
  await page.mouse.click(x,y); const menu=page.locator('#pe-rule-picker'); await expect(menu).toBeVisible();
  const box=await menu.boundingBox(); expect(box.x).toBeGreaterThanOrEqual(0); expect(box.x+box.width).toBeLessThanOrEqual(375); expect(box.y+box.height).toBeLessThanOrEqual(667);
  await page.keyboard.press('Escape');
  await page.locator('.pe-list-row.active').click({button:'right'}); await expect(page.locator('#pe-preset-menu')).toBeVisible();
  const preset=await page.locator('#pe-preset-menu').boundingBox(); expect(preset.x+preset.width).toBeLessThanOrEqual(375); expect(preset.y+preset.height).toBeLessThanOrEqual(667);
});

test('preset three-dot menu attaches to its button inside a centred desktop dialog',async({page},testInfo)=>{
  await create(page); const trigger=page.locator('.pe-list-item:has(.pe-list-row.active) .pe-preset-more');
  await trigger.click({timeout:5000}); await expect(page.locator('#pe-preset-menu')).toBeVisible();
  const owner=await trigger.boundingBox();
  const menu=await page.locator('#pe-preset-menu').boundingBox();
  expect(Math.abs(menu.x+menu.width-owner.x-owner.width)).toBeLessThan(2); expect(Math.abs(menu.y-owner.y-owner.height-4)).toBeLessThan(2);
  expect(menu.width).toBeLessThanOrEqual(160);
  const leadingGap=await page.locator('#pe-preset-menu [data-cmd=rename]').evaluate(async button=>{
    await Promise.all(button.closest('[data-controller-popup]').getAnimations({subtree:true}).map(animation=>animation.finished.catch(()=>{})));
    const style=getComputedStyle(button),box=button.getBoundingClientRect();
    return button.firstElementChild.getBoundingClientRect().left-box.left-parseFloat(style.borderLeftWidth)-parseFloat(style.paddingLeft);
  });
  expect(Math.abs(leadingGap)).toBeLessThan(1);
  await page.screenshot({path:testInfo.outputPath('preset-menu.png'),animations:'disabled',timeout:5000});
});

test('delete is reversible and Undo restores saved rules', async ({ page }) => {
  await create(page,'Recover me'); await add(page,'case'); await page.locator('.pe-case-style').selectOption('upper');
  await menu(page,'delete'); await expect(page.locator('.pe-list-name').filter({hasText:'Recover me'})).toHaveCount(0);
  await page.getByRole('button',{name:'Undo',exact:true}).click(); await expect(page.locator('.pe-list-row.active .pe-list-name')).toHaveText('Recover me');
  await page.locator('#pe-sample-input').fill('Blue Moon'); await expect(page.locator('#pe-sample-out')).toHaveText('BLUE MOON');
});

test('invalid patterns identify the Find field and recover after correction', async ({ page }) => {
  await create(page); await add(page,'replace');
  await page.locator('.is-expanded .pe-rule-field input').first().fill('[');
  await page.locator('.pe-pattern-options > summary').click(); await page.getByRole('checkbox',{name:'Pattern matching',exact:true}).click();
  await expect(page.locator('.pe-rule-error')).toBeVisible(); await expect(page.locator('.is-expanded .pe-rule-field input').first()).toHaveAttribute('aria-invalid','true');
  await expect(page.locator('#pe-sample-out')).toHaveText('Check the rule settings.');
  await page.locator('.is-expanded .pe-rule-field input').first().fill('Moon'); await expect(page.locator('.pe-rule-error')).toBeHidden();
  await expect(page.locator('#pe-sample-out')).not.toHaveAttribute('aria-busy','true');
});

test('reordering keeps the focused action attached to the same rule', async ({ page }) => {
  await create(page); await add(page,'replace'); await add(page,'case');
  const card=page.locator('.is-expanded'); const key=await card.getAttribute('data-key');
  await card.getByRole('button',{name:'Move up',exact:true}).click();
  await expect(page.locator('.rule-card').first()).toHaveAttribute('data-key',key);
  await expect(page.locator('.is-expanded .pe-rule-open')).toBeFocused();
});

for (const theme of ['light','dark']) for (const [width,height] of [[1280,800],[375,667],[844,390]]) test(`aligned pinned preview and themed inline rules: ${theme} ${width}x${height}`, async ({ page }, testInfo) => {
  await page.setViewportSize({width,height}); await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;},theme);
  await page.locator('#pe-editor').evaluate(async e=>{await Promise.all(e.getAnimations({subtree:true}).map(a=>a.finished.catch(()=>{})));});
  const before=await page.locator('.pe-sample').boundingBox();
  await page.locator('.pe-list-row').last().click(); await page.locator('.pe-rule-open').first().click();
  await page.locator('.pe-rules-panel').evaluate(e=>{e.scrollTop=e.scrollHeight;});
  const after=await page.locator('.pe-sample').boundingBox(); expect(before.y).toBe(after.y); expect(before.height).toBe(after.height);
  await expect(page.locator('#pe-sample-input')).toBeInViewport({ratio:1}); await expect(page.locator('#pe-sample-out')).toBeInViewport({ratio:1});
  const metrics=await page.locator('.pe-sample').evaluate(e=>['#pe-sample-input','#pe-sample-out'].map(id=>{const n=e.querySelector(id);const s=getComputedStyle(n);return {height:n.getBoundingClientRect().height,padding:s.padding,line:s.lineHeight,font:s.fontSize};}));
  expect(metrics[0]).toEqual(metrics[1]);
  await page.locator('.pe-rules-panel').evaluate(e=>{e.scrollTop=0;});
  await expect(page.locator('#pe-sample-out')).not.toHaveAttribute('aria-busy','true');
  await page.locator('#pe-editor').evaluate(async e=>{await Promise.all(e.getAnimations({subtree:true}).map(a=>a.finished.catch(()=>{})));});
  await page.screenshot({path:testInfo.outputPath(`preset-${theme}-${width}.png`)});
});

test('preset right-click stays at the pointer while its three-dot menu follows the button',async({page})=>{
  await page.emulateMedia({reducedMotion:'reduce'});await create(page);
  const row=page.locator('.pe-list-row.active');const r=await row.boundingBox();const x=r.x+40,y=r.y+16;
  await row.click({button:'right',position:{x:40,y:16}});const menu=page.locator('#pe-preset-menu');await expect(menu).toBeVisible();let box=await menu.boundingBox();expect(Math.abs(box.x-x)).toBeLessThan(2);expect(Math.abs(box.y-y)).toBeLessThan(2);
  await page.keyboard.press('Escape');await expect(row).toBeFocused();
  const more=page.locator('.pe-list-item:has(.pe-list-row.active) .pe-preset-more');await more.click();await expect(menu).toBeVisible();const owner=await more.boundingBox();box=await menu.boundingBox();expect(Math.abs(box.x+box.width-owner.x-owner.width)).toBeLessThan(2);expect(Math.abs(box.y-owner.y-owner.height-4)).toBeLessThan(2);
  await page.keyboard.press('Escape');await expect(more).toBeFocused();
});


test('preset changes are local until Save, including selection and search', async ({ page }) => {
  const saved = () => page.evaluate(async () => (await window.tagrex.invoke('load_settings', {})).action_groups || []);
  await create(page, 'Draft only'); await add(page, 'case');
  await page.locator('.pe-case-style').selectOption('upper');
  await page.locator('#pe-group-builtin .pe-list-row').first().click();
  await page.locator('#pe-search').fill('Draft');
  await expect(page.locator('#pe-group-personal .pe-list-row')).toHaveCount(1);
  expect(await saved()).toEqual([]);
  expect(await page.evaluate(() => window.presetSaveCalls || 0)).toBe(0);
  await page.locator('#pe-save').click();
  await expect(page.locator('#pe-save')).toBeDisabled();
  await expect(page.locator('#pe-editor')).toBeVisible();
  expect((await saved())[0].name).toBe('Draft only');
  expect(await page.evaluate(() => window.presetSaveCalls)).toBe(1);
  await page.locator('#pe-search').fill('');
  await page.locator('#pe-group-personal .pe-list-row').click();
  await page.locator('.pe-rule-open').click();
  await page.locator('.pe-case-style').selectOption('lower');
  await page.locator('#pe-cancel').click();
  await expect(page.locator('#pe-editor')).toBeHidden();
  await openTagSource(page,'transformations');await page.locator('#presets-edit').click();
  await page.locator('.pe-rule-open').click();
  await expect(page.locator('.pe-case-style')).toHaveValue('upper');
});

test('failed Save preserves the draft and supports retry without closing', async ({ page }) => {
  await create(page, 'Retry me');
  await page.evaluate(() => window.failPresetSave = true);
  await page.locator('#pe-save-close').click();
  await expect(page.locator('#toast')).toContainText('Could not save presets');
  await expect(page.locator('#pe-editor')).toBeVisible();
  await expect(page.locator('#pe-save')).toBeEnabled();
  expect(await page.evaluate(async () => (await window.tagrex.invoke('load_settings', {})).action_groups || [])).toEqual([]);
  await page.evaluate(() => window.failPresetSave = false);
  await page.locator('#pe-save-close').click();
  await expect(page.locator('#pe-editor')).toBeHidden();
  await openTagSource(page,'transformations');await page.locator('#presets-edit').click();
  await expect(page.locator('.pe-list-row.active')).toContainText('Retry me');
});

for (const close of ['button', 'escape', 'overlay']) test(`unsaved ${close} dismissal asks before discarding`, async ({ page }) => {
  await create(page, 'Keep this draft');
  if (close === 'button') await page.locator('#pe-editor-close').click();
  else if (close === 'escape') await page.keyboard.press('Escape');
  else await page.mouse.click(4, 4);
  await expect(page.locator('#confirm-modal')).toBeVisible();
  await page.locator('#confirm-cancel').click();
  await expect(page.locator('#pe-editor')).toBeVisible();
  await expect(page.locator('.pe-list-row.active')).toContainText('Keep this draft');
  await page.locator('#pe-cancel').click();
  await expect(page.locator('#pe-editor')).toBeHidden();
  await openTagSource(page,'transformations');await page.locator('#presets-edit').click();
  await expect(page.locator('#pe-group-personal .pe-list-row')).toHaveCount(0);
});

test('search and header actions keep compact groups usable', async ({ page }) => {
  await expect(page.locator('.pe-group-caret, .pe-list-note, .pe-list-actions')).toHaveCount(0);
  await expect(page.locator('.pe-sidebar-group > h3 #pe-new')).toBeVisible();
  await page.locator('#pe-search').fill('does not exist');
  await expect(page.locator('.pe-list-row')).toHaveCount(0);
  await expect(page.locator('#pe-list')).toContainText('No matching presets.');
  await create(page, 'Search clears for new');
  await expect(page.locator('#pe-search')).toHaveValue('');
  await expect(page.locator('.pe-list-row.active')).toContainText('Search clears for new');
});


test('keyboard search/save and fixed footer match the Settings surface', async ({ page }) => {
  await create(page, 'Keyboard saved');
  await page.keyboard.press('Control+f'); await expect(page.locator('#pe-search')).toBeFocused();
  await page.keyboard.press('Control+s');
  await expect(page.locator('#pe-save')).toBeDisabled();
  expect((await page.evaluate(async () => (await window.tagrex.invoke('load_settings', {})).action_groups))[0].name).toBe('Keyboard saved');
  const surfaces = await page.evaluate(() => ['.pe-edit-col', '.settings-body'].map(selector => getComputedStyle(document.querySelector(selector)).backgroundColor));
  expect(surfaces[0]).toBe(surfaces[1]);
  const inset=await page.locator('.pe-edit-col').boundingBox(), footer=await page.locator('.pe-footer').boundingBox();
  expect(footer.y-inset.y-inset.height).toBeGreaterThanOrEqual(7);
  const dialog=await page.locator('.modal.pe-editor').boundingBox(), save=await page.locator('#pe-save-close').boundingBox();
  expect(dialog.x+dialog.width-save.x-save.width).toBeLessThanOrEqual(18);
  expect(dialog.y+dialog.height-save.y-save.height).toBeLessThanOrEqual(16);
});


test('renaming survives blur and commits only with Enter', async ({ page }) => {
  await create(page, 'Original name');
  await page.locator('#pe-save').click(); await expect(page.locator('#pe-save')).toBeDisabled();
  await page.locator('.pe-list-row.active').press('F2');
  const name=page.locator('#pe-name'); await name.fill('Only after Enter');
  await page.locator('#pe-sample-input').click();
  await expect(name).toBeVisible(); await expect(name).toHaveValue('Only after Enter');
  await expect(page.locator('#pe-save')).toBeDisabled();
  await expect(page.locator('#pe-save-close')).toBeDisabled();
  await name.focus(); await expect(name).toBeFocused();
  await name.press('Enter');
  await expect(name).toHaveCount(0);
  await expect(page.locator('.pe-list-row.active .pe-list-name')).toHaveText('Only after Enter');
  await expect(page.locator('#pe-save')).toBeEnabled();
});
