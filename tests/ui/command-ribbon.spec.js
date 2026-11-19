import { openTagSource } from './navigation.js';
import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
const root = resolve('.test-dist');
test.beforeEach(async ({ page }) => {
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try { await route.fulfill({ body: await readFile(path), contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html' }); }
    catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.setMode)).toBe('function');
  await page.evaluate(async () => { document.getElementById('root').value = '/music'; await window.tagrex.openLibrary(); });
});
async function picker(page, path) {
  await page.evaluate(path => { window.__TAURI__ = { dialog: { open: async options => { window.lastFolderOptions = options; return path; } } }; }, path);
}
const paths = page => page.evaluate(() => window.tagrex.tracks.map(t => t.path));
const selected = page => page.evaluate(() => [...window.tagrex.selection]);

test('eighteen commands have eight group dividers and selection follows the filtered model', async ({ page }) => {
  await expect(page.locator('.command-ribbon button.ribbon-command')).toHaveCount(18);
  await expect(page.locator('.ribbon-divider')).toHaveCount(8);
  await expect(page.locator('#more-btn')).toBeHidden();
  await page.locator('#ribbon-clear-selection').click();
  await expect.poll(() => selected(page)).toEqual([]);
  await expect(page.locator('#ribbon-clear-selection')).toBeDisabled();
  await page.locator('#ribbon-select-all').focus();
  await page.keyboard.press('Enter');
  expect(new Set(await selected(page))).toEqual(new Set(await paths(page)));
  await page.locator('#ribbon-clear-selection').click();
  // Use a real substring from the fixture rather than relying on an artist name.
  await page.locator('#filter').fill('Wish');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(1);
  await page.locator('#ribbon-select-all').click();
  expect(await selected(page)).toHaveLength(1);
});

test('Open uses a directory chooser; cancelling Open or Add leaves the list intact', async ({ page }) => {
  const before = await paths(page);
  await picker(page, null);
  for (const id of ['ribbon-open-folder', 'ribbon-add-folder']) {
    await page.locator('#' + id).click();
    expect(await paths(page)).toEqual(before);
    expect(await page.evaluate(() => window.lastFolderOptions)).toEqual({ directory: true, multiple: false });
  }
  await picker(page, '/replacement');
  await page.locator('#ribbon-open-folder').click();
  await expect(page.locator('#root-display')).toContainText('replacement');
});

test('Add retains existing files, tags and selection while loading another folder', async ({ page }) => {
  const before = await page.evaluate(() => window.tagrex.tracks.map(t => structuredClone(t)));
  const selection = await selected(page);
  await picker(page, '/more-music');
  await page.locator('#ribbon-add-folder').click();
  await expect.poll(async () => (await paths(page)).length).toBe(before.length + 2);
  const after = await page.evaluate(() => window.tagrex.tracks);
  for (const track of before) expect(after.find(t => t.path === track.path).tags).toEqual(track.tags);
  expect(await selected(page)).toEqual(selection);
  await page.locator('#ribbon-add-folder').click();
  await expect.poll(async () => (await paths(page)).length).toBe(before.length + 2);
});

test('folder commands protect staged edits; Undo invokes the existing history flow', async ({ page }) => {
  await openTagSource(page, 'editor');

  await page.locator('#editor-field-title').fill('Ribbon test');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const before = await paths(page);
  await picker(page, '/other');
  for (const id of ['ribbon-open-folder', 'ribbon-add-folder']) {
    await page.locator('#' + id).click();
    expect(await paths(page)).toEqual(before);
    await expect(page.locator('#editor-field-title')).toHaveValue('Ribbon test');
    expect(await page.evaluate(() => window.lastFolderOptions)).toBeUndefined();
  }
  await page.locator('#diff-apply').click();
  await expect(page.locator('#undo')).toBeEnabled();
  const history = await page.evaluate(() => window.tagrex.invoke('history', {}));
  await page.locator('#undo').click();
  await expect.poll(() => page.evaluate(async () => (await window.tagrex.invoke('history', {})).length)).toBe(history.length - 1);
});

for (const mode of ['light', 'dark']) test(`ribbon fits a narrow ${mode} window`, async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 720 });
  await page.evaluate(mode => { document.documentElement.dataset.theme = mode; }, mode);
  for (const button of await page.locator('.command-ribbon button.ribbon-command').all()) await expect(button).toBeInViewport();
  const dividers = await page.locator('.ribbon-divider').evaluateAll(nodes => nodes.map(n => ({ width: n.getBoundingClientRect().width, height: n.getBoundingClientRect().height })));
  expect(dividers).toEqual(Array.from({ length: 8 }, () => ({ width: 1, height: 18 })));
});


test('every ribbon command has a themed hover hint, including disabled commands', async ({ page }) => {
  for (const id of ['ribbon-open-folder', 'ribbon-add-folder', 'undo', 'ribbon-select-all', 'ribbon-clear-selection', 'ribbon-move-down', 'ribbon-move-up', 'group-btn', 'ribbon-remove-tags', 'ribbon-rename-folder', 'ribbon-format-values', 'ribbon-autonumber', 'ribbon-import-artwork', 'ribbon-export-artwork', 'ribbon-artwork-options', 'ribbon-lyrics']) {
    const button = page.locator('#' + id);
    await button.hover();
    const text = await button.getAttribute('aria-description');
    expect(text).toBeTruthy();
    await expect(page.locator('#controller-tooltip-content')).toBeVisible();
    await expect(page.locator('#controller-tooltip-content')).toContainText(text);
    await page.mouse.move(500, 350);
    await expect(page.locator('#controller-tooltip-content')).toBeHidden();
  }
  await page.locator('#ribbon-add-folder').focus();
  await expect(page.locator('#controller-tooltip-content')).toContainText('Add folder');
  await page.keyboard.press('Escape');
  await expect(page.locator('#controller-tooltip-content')).toBeHidden();
});


test('move buttons reorder selected files in their group and retain the selection', async ({ page }) => {
  const original = await paths(page);
  const picked = await selected(page);
  await page.locator('#ribbon-move-down').click();
  await expect.poll(() => paths(page)).toEqual([original[1], original[0], original[2]]);
  expect(await selected(page)).toEqual(picked);
  await page.locator('#ribbon-move-up').click();
  await expect.poll(() => paths(page)).toEqual(original);
  await expect(page.locator('#ribbon-move-up')).toBeDisabled();
});

test('grouping menu is usable from a neutral ribbon button', async ({ page }) => {
  const button = page.locator('#group-btn');
  const neutral = await button.evaluate(n => getComputedStyle(n).color);
  await button.click();
  await expect(page.locator('#group-menu')).toBeVisible();
  await page.locator('#group-menu [data-group=""]').click();
  await expect(button).toBeFocused();
  await expect(page.locator('#group-menu')).toBeHidden();
  await button.click();
  await page.locator('#group-menu [data-group="album"]').click();
  await page.mouse.move(500,350);
  await button.blur();
  await expect.poll(() => button.evaluate(n => getComputedStyle(n).color)).toBe(neutral);
  await expect(page.locator('#ribbon-sort')).toHaveCount(0);
});


test('Autonumber owns the track and disc format choices and stages numbers in file order', async ({ page }) => {
  await page.locator('#ribbon-select-all').click();
  await page.locator('#ribbon-autonumber').click();
  await expect(page.getByRole('dialog', {name:/Numbering/})).toBeVisible();
  await expect(page.locator('#autonum-track-format-control')).toBeVisible();
  await expect(page.locator('#autonum-disc-format-control')).toBeHidden();
  await page.getByRole('tab',{name:'Disc',exact:true}).click();
  await expect(page.locator('#autonum-disc-format-control')).toBeVisible();
  await page.locator('#autonum-disc').fill('2');
  await page.getByRole('tab',{name:'Autonumber',exact:true}).click();
  await expect(page.locator('.command-ribbon > .ribbon-group #autonum-track-format')).toHaveCount(0);
  await page.locator('#autonum-start').fill('3');
  await page.locator('#autonum-run').click();
  await expect(page.locator('#ribbon-number-dialog')).toBeHidden();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const plan = await page.evaluate(() => window.tagrex.previewPlan);
  expect(plan.changes.map(change => change.tag_changes.find(field => field.field === 'track')?.new)).toEqual(['3','4','5']);
  expect(plan.changes.flatMap(change=>change.tag_changes).some(field=>field.field==='disc')).toBe(false);
});

test('artwork importer uses a string or first-image option and stages without writing', async ({ page }) => {
  await page.locator('#ribbon-select-all').click();
  await page.locator('#ribbon-import-artwork').click();
  await expect(page.getByRole('dialog', {name:'Import artwork',exact:true})).toBeVisible();
  await page.locator('#artwork-import-pattern').fill('%album%.jpg');
  await page.locator('#artwork-import-first-control').click();
  await expect(page.locator('#artwork-import-pattern')).toBeDisabled();
  await page.locator('#artwork-import-first-control').click();
  await expect(page.locator('#artwork-import-pattern')).toHaveValue('%album%.jpg');
  await page.locator('#ribbon-import-stage').click();
  await expect(page.locator('#ribbon-import-dialog')).toBeHidden();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const changed = await page.evaluate(() => window.tagrex.previewPlan.changes.filter(c=>c.cover_change).map(c=>c.path));
  expect(changed).not.toContain((await paths(page))[0]);
  expect(changed).toHaveLength(2);
  await expect(page.locator('#ribbon-lyrics')).toBeDisabled();
  await expect(page.locator('#ribbon-lyrics')).toHaveAttribute('aria-description',/coming later/);
});

test('Format values stages field patterns and closes without applying', async ({ page }) => {
  await page.locator('#ribbon-format-values').click();
  await page.locator('#ribbon-format-pattern').fill('%artist%');
  await page.locator('#ribbon-format-stage').click();
  await expect(page.locator('#ribbon-format-dialog')).toBeHidden();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  expect(await page.evaluate(()=>window.tagrex.previewPlan.changes[0].tag_changes.find(c=>c.field==='albumartist').new)).toBe('The X Factor');
});

for (const mode of ['light','dark']) test(`command dialogs have opaque themed surfaces in ${mode}`, async ({page}) => {
  await page.locator('#settings-open').click();
  await page.locator(`#set-theme [data-theme-mode="${mode}"]`).click();
  await page.locator('#settings-cancel').click();
  for (const [button,root] of [['ribbon-autonumber','ribbon-number-dialog'],['ribbon-import-artwork','ribbon-import-dialog'],['ribbon-remove-tags','ribbon-remove-dialog'],['ribbon-format-values','ribbon-format-dialog']]) {
    await page.locator('#'+button).click();
    const body = page.locator('#'+root+' .modal');
    await expect(body).toBeVisible();
    const style = await body.evaluate(n => {const s=getComputedStyle(n);return {background:s.backgroundColor,border:s.borderWidth,radius:s.borderRadius,padding:s.padding};});
    expect(style.background).not.toBe('rgba(0, 0, 0, 0)');
    expect(style.border).toBe('1px');
    expect(style.radius).toBe('22px');
    expect(style.padding).toBe('24px');
    await page.keyboard.press('Escape');
    await expect(body).toBeHidden();
  }
});

test('quick Write keeps staged values on the same file and leaves the Apply scope intact', async ({page}) => {
  await openTagSource(page, 'editor');
  await page.locator('#editor-field-albumartist').fill('Staged album artist');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const before = await page.evaluate(() => ({plan:structuredClone(window.tagrex.previewPlan), selected:[...window.tagrex.selection]}));
  await page.locator('#ribbon-format-values').click();
  await page.locator('#ribbon-format-pattern').fill('Quick album artist');
  await page.locator('#ribbon-format-write').click();
  await expect(page.locator('#ribbon-format-dialog')).toBeHidden();
  await expect.poll(() => page.evaluate(()=>window.tagrex.tracks[0].tags.albumartist)).toBe('Quick album artist');
  await expect(page.locator('#undo')).toBeEnabled();
  const after = await page.evaluate(() => ({plan:window.tagrex.previewPlan, selected:[...window.tagrex.selection], tags:window.tagrex.tracks[0].tags}));
  expect(after.tags.albumartist).toBe('Quick album artist');
  expect(after.selected).toEqual([]);
  expect(after.plan.changes.map(c=>c.path)).toEqual(before.plan.changes.map(c=>c.path));
  const staged = after.plan.changes[0].tag_changes.find(f=>f.field==='albumartist');
  expect(staged.new).toBe('Staged album artist');
  expect(staged.old).toBe('Quick album artist');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.locator('#diff-apply').click();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.tracks[0].tags.albumartist)).toBe('Staged album artist');
});

test('Cancel discards dialog values and number display preferences without touching staged edits', async ({page}) => {
  await page.locator('#ribbon-format-values').click();
  await page.locator('#ribbon-format-stage').click();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const before = await page.evaluate(()=>structuredClone(window.tagrex.previewPlan));
  await page.locator('#ribbon-format-values').click();
  await page.locator('#ribbon-format-pattern').fill('Discard me');
  await page.getByRole('button',{name:'Cancel',exact:true}).click();
  await page.locator('#ribbon-format-values').click();
  await expect(page.locator('#ribbon-format-pattern')).toHaveText('Artist');
  await page.keyboard.press('Escape');
  await page.locator('#ribbon-autonumber').click();
  await page.locator('#autonum-track-format-control').click();
  await page.getByRole('option',{name:'001',exact:true}).click();
  await page.getByRole('button',{name:'Cancel',exact:true}).click();
  expect(await page.evaluate(()=>localStorage.getItem('tagrex.trackDigits'))).toBeNull();
  expect(await page.evaluate(()=>window.tagrex.previewPlan)).toEqual(before);
});

for (const [command,root,write] of [['ribbon-autonumber','ribbon-number-dialog','autonum-write'],['ribbon-import-artwork','ribbon-import-dialog','ribbon-import-write'],['ribbon-remove-tags','ribbon-remove-dialog','ribbon-remove-write']]) test(`${command} has Cancel Stage Write and writes without creating staging`, async ({page}) => {
  await page.locator('#ribbon-clear-selection').click();
  const rows = page.locator('#tracks-body tr[data-path]');
  await rows.nth(1).click();
  await page.locator('#'+command).click();
  const dialog = page.locator('#'+root);
  for (const name of ['Cancel',command === 'ribbon-autonumber' ? 'Stage Edits' : 'Stage','Write']) await expect(dialog.getByRole('button',{name,exact:true})).toBeVisible();
  await page.locator('#'+write).click();
  await expect(dialog).toBeHidden();
  await expect(page.locator('#undo')).toBeEnabled();
  expect(await page.evaluate(()=>window.tagrex.previewPlan?.changes?.length || 0)).toBe(0);
});

async function holdQuickWrite(page) {
  await page.evaluate(()=>{
    const original = Promise.resolve;
    window.__TAURI__ = {event:{listen:async(_event, callback)=>{window.quickProgress=payload=>callback({payload});return ()=>{};}}};
    const disk = window.tagrex.tracks.map(track=>structuredClone(track));
    Promise.resolve = function(value) {
      if (value?.id && value?.description && 'applied_at' in value) return new Promise((resolve,reject)=>{
        window.finishQuick=()=>resolve(value);
        window.failQuick=()=>{window.tagrex.tracks.forEach((track,index)=>Object.assign(track,disk[index]));reject({message:{code:'error.plan.cancelled'}});};
      });
      return original.call(this,value);
    };
    window.quickRows=[...document.querySelectorAll('#tracks-body tr[data-path]')];
    window.quickCells=[...document.querySelectorAll('#tracks-body td')];
  });
}

test('Write closes immediately, paints confirmed rows sequentially, and deselects completed files', async ({page})=>{
  await page.locator('#ribbon-select-all').click();
  const targets = await paths(page);
  await page.locator('#ribbon-format-values').click();
  await page.locator('#ribbon-format-pattern').fill('Quick write');
  await holdQuickWrite(page);
  await page.locator('#ribbon-format-write').click();
  await expect(page.locator('#ribbon-format-dialog')).toBeHidden();
  await expect.poll(()=>page.evaluate(()=>typeof window.finishQuick)).toBe('function');
  const rows=page.locator('#tracks-body tr[data-path]');
  for (let index=0;index<targets.length;index++) {
    await page.evaluate(({path,done})=>window.quickProgress({path,done,total:6}), {path:targets[index],done:3+index});
    await expect(rows.nth(index)).toHaveClass(/applying-current/);
    await page.evaluate(({path,done})=>window.quickProgress({path,done,total:6,completed:'tags'}), {path:targets[index],done:4+index});
    await expect(rows.nth(index)).not.toHaveClass(/selected/);
    await expect(page.locator('#files-selected-count')).toHaveText(String(targets.length-index-1));
    expect(await page.evaluate(()=>[...window.quickRows,...window.quickCells].every(n=>n.isConnected))).toBe(true);
  }
  await page.evaluate(()=>window.finishQuick());
  await expect(page.locator('#undo')).toBeEnabled();
  await expect.poll(()=>selected(page)).toEqual([]);
  await expect(page.locator('#diff-actionbar')).toBeVisible();await expect(page.locator('#diff-apply')).toBeDisabled();
});

test('cancelled quick Write restores the row animation and keeps pending staged edits', async ({page})=>{
  await page.locator('#ribbon-select-all').click();
  await page.locator('#ribbon-format-values').click();
  await page.locator('#ribbon-format-pattern').fill('Staged value');
  await page.locator('#ribbon-format-stage').click();
  await expect(page.locator('#diff-apply')).toBeEnabled();
  const before=await page.evaluate(()=>structuredClone(window.tagrex.previewPlan));
  await page.locator('#ribbon-format-values').click();
  await page.locator('#ribbon-format-pattern').fill('Quick value');
  await holdQuickWrite(page);
  await page.locator('#ribbon-format-write').click();
  await expect.poll(()=>page.evaluate(()=>typeof window.finishQuick)).toBe('function');
  await page.evaluate(path=>window.quickProgress({path,done:4,total:6,completed:'tags'}),before.changes[0].path);
  await expect(page.locator('#tracks-body tr[data-path]').first()).not.toHaveClass(/selected/);
  await page.evaluate(()=>window.failQuick());
  await expect(page.locator('#mode-col')).not.toHaveAttribute('inert','');
  await expect(page.locator('#tracks-body tr[data-path]').first()).toHaveClass(/selected/);
  expect(await page.evaluate(()=>window.tagrex.previewPlan.changes)).toEqual(before.changes);
  await expect(page.locator('#diff-apply')).toBeEnabled();
});

test('Redo sits beside Undo and replays the last undone batch', async ({page}) => {
  const before=await page.evaluate(()=>window.tagrex.tracks[0].tags.title);
  await expect(page.locator('#redo')).toBeDisabled();
  await expect(page.locator('#undo + #redo')).toHaveCount(1);
  await openTagSource(page, 'editor');
  await page.locator('#editor-field-title').fill('Redo ribbon fixture');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.locator('#diff-apply').click();
  await expect(page.locator('#undo')).toBeEnabled();
  await page.locator('#undo').click();
  await expect(page.locator('#redo')).toBeEnabled();
  expect(await page.evaluate(()=>window.tagrex.tracks[0].tags.title)).toBe(before);
  await page.locator('#redo').focus();
  await page.keyboard.press('Control+Shift+z');
  await expect.poll(()=>page.evaluate(()=>window.tagrex.tracks[0].tags.title)).toBe('Redo ribbon fixture');
  await expect(page.locator('#redo')).toBeDisabled();
  await expect(page.locator('#undo')).toBeEnabled();
});

test('Redo preserves a pending editor value and revalidates its preview', async ({page}) => {
  await openTagSource(page, 'editor');
  await page.locator('#editor-field-title').fill('Saved title');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.locator('#diff-apply').click();
  await expect(page.locator('#undo')).toBeEnabled();
  await page.locator('#undo').click();
  await expect(page.locator('#redo')).toBeEnabled();
  await page.locator('#ribbon-select-all').click();
  await page.locator('#editor-field-artist').fill('Keep this pending');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  await page.locator('#redo').click();
  await expect(page.locator('#editor-field-artist')).toHaveValue('Keep this pending');
  await expect(page.locator('#diff-apply')).toBeEnabled();
  expect(await page.evaluate(()=>window.tagrex.tracks.some(track=>track.tags.artist==='Keep this pending'))).toBe(false);
  expect(await page.evaluate(()=>window.tagrex.previewPlan.changes.some(change=>change.tag_changes.some(field=>field.field==='artist'&&field.new==='Keep this pending')))).toBe(true);
});


test('a manually arranged order survives re-reading the opened files',async({page})=>{
  const first=page.locator('#tracks-body tr[data-path]').first();
  await first.click();
  await page.locator('#ribbon-move-down').click();
  const arranged=await paths(page);
  await page.locator('#lib-refresh').click();
  await expect.poll(()=>paths(page)).toEqual(arranged);
});

for (const theme of ['light', 'dark']) test(`header groups the tools beside the logo and grouping toggle works by keyboard in ${theme}`, async ({page}) => {
  await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
  await page.setViewportSize({width:1397,height:900});
  const tabs = await page.locator('.mode-tabs').boundingBox();
  const brand = await page.locator('.brand').boundingBox();
  expect(tabs.x - brand.x - brand.width).toBeGreaterThanOrEqual(12);
  expect(tabs.x - brand.x - brand.width).toBeLessThanOrEqual(24);
  await page.locator('#group-btn').click();
  await page.locator('#group-menu [data-group=""]').click();
  await expect(page.locator('#toggle-groups')).toBeHidden();
  await page.locator('#group-btn').click();
  await page.locator('#group-menu [data-group="album"]').click();
  const toggle = page.locator('#toggle-groups');
  await expect(toggle).toBeVisible();
  expect(await toggle.evaluate(node => node.previousElementSibling.id)).toBe('group-btn');
  await expect(toggle).toHaveAccessibleName('Collapse every group');
  await toggle.focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(0);
  await expect(toggle).toHaveAccessibleName('Expand every group');
  await page.keyboard.press('Enter');
  await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
  await expect(toggle).toHaveAccessibleName('Collapse every group');
});

test('Autonumber uses quiet controls, borderless actions and stable invalid input handling',async({page},info)=>{
  await page.locator('#ribbon-autonumber').click();
  const dialog=page.locator('#ribbon-number-dialog');
  await expect(dialog.getByRole('button',{name:'Stage Edits',exact:true})).toBeEnabled();
  await expect(dialog.getByRole('switch')).toHaveCount(2);
  await expect(dialog.locator('.ribbon-tool-description,.ribbon-field-help,[role=alert]')).toHaveCount(0);
  const before=await dialog.locator('.modal').evaluate(node=>({height:parseFloat(getComputedStyle(node).height)}));
  await expect.poll(async()=>(await dialog.locator('.modal').boundingBox()).height).toBeCloseTo(before.height,0);
  await page.locator('#autonum-start').fill('');
  await expect(page.locator('#autonum-run')).toBeDisabled();await expect(page.locator('#autonum-write')).toBeDisabled();
  await page.locator('#autonum-start').fill('3');
  await page.getByRole('tab',{name:'Disc',exact:true}).click();
  await page.locator('#autonum-disc').fill('not a number');
  await expect(page.locator('#autonum-run')).toBeDisabled();
  expect((await dialog.locator('.modal').boundingBox()).height).toBeCloseTo(before.height,0);
  await page.locator('#autonum-disc').fill('2');await expect(page.locator('#autonum-run')).toBeEnabled();
  for(const button of await dialog.locator('.ribbon-tool-footer button').all()) expect(await button.evaluate(node=>getComputedStyle(node).borderTopWidth)).toBe('0px');
  await expect(page.locator('#autonum-write')).toHaveAttribute('data-tooltip',/Bypass staging/);
  await expect(page.locator('#autonum-run')).toHaveAttribute('data-tooltip-delay','1600');
  await page.setViewportSize({width:420,height:620});
  await expect(page.locator('#autonum-run')).toBeInViewport();
  await expect(page.locator('#autonum-disc-format-control')).toBeInViewport();
  await info.attach('autonumber-quiet-dialog',{body:await page.screenshot(),contentType:'image/png'});
});

test('Disc and Autonumber have independent validity, scope and display preferences',async({page})=>{
  await page.locator('#ribbon-select-all').click();
  const stored=await page.evaluate(()=>structuredClone(window.tagrex.tracks));
  await page.locator('#ribbon-autonumber').click();
  const dialog=page.locator('#ribbon-number-dialog');
  await expect(dialog.getByRole('tab')).toHaveCount(3);
  const height=await dialog.locator('.modal').evaluate(node=>parseFloat(getComputedStyle(node).height));
  await expect.poll(async()=>(await dialog.locator('.modal').boundingBox()).height).toBeCloseTo(height,0);
  await page.locator('#autonum-start').fill('');
  await expect(page.locator('#autonum-run')).toBeDisabled();
  await page.getByRole('tab',{name:'Disc',exact:true}).click();
  await expect(page.locator('#autonum-start')).toBeHidden();
  await expect(dialog.getByRole('switch')).toHaveCount(3);
  await page.locator('#autonum-disc').fill('2');
  await expect(page.locator('#autonum-run')).toBeEnabled();
  expect((await dialog.locator('.modal').boundingBox()).height).toBeCloseTo(height,0);
  await page.locator('#autonum-disc-format-control').click();await page.getByRole('option',{name:'01',exact:true}).click();
  await page.locator('#autonum-run').click();await expect(dialog).toBeHidden();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes.flatMap(change=>change.tag_changes).map(edit=>edit.field))).toEqual(['disc','disc','disc']);
  expect(await page.evaluate(()=>window.tagrex.tracks)).toEqual(stored);
  expect(await page.evaluate(()=>localStorage.getItem('tagrex.discDigits'))).toBe('2');
  expect(await page.evaluate(()=>localStorage.getItem('tagrex.trackDigits'))).toBeNull();
  await page.locator('#ribbon-autonumber').click();await page.locator('#autonum-start').fill('4');await page.locator('#autonum-run').click();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes.map(change=>change.tag_changes.find(edit=>edit.field==='track')?.new))).toEqual(['4','5','6']);
  expect(await page.evaluate(()=>window.tagrex.previewPlan.changes.map(change=>change.tag_changes.find(edit=>edit.field==='disc')?.new))).toEqual(['2','2','2']);
});

test('Disc Write and Undo leave track numbering untouched',async({page})=>{
  await page.locator('#ribbon-select-all').click();
  const before=await page.evaluate(()=>window.tagrex.tracks.map(track=>({path:track.path,track:track.tags.track,disc:track.tags.disc})));
  await page.locator('#ribbon-autonumber').click();await page.getByRole('tab',{name:'Disc',exact:true}).click();
  await page.locator('#autonum-disc').fill('3');await page.locator('#autonum-write').click();
  await expect(page.locator('#ribbon-number-dialog')).toBeHidden();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.tracks.map(track=>track.tags.disc))).toEqual(['3','3','3']);
  expect(await page.evaluate(()=>window.tagrex.tracks.map(track=>({path:track.path,track:track.tags.track})))).toEqual(before.map(({path,track})=>({path,track})));
  await page.locator('#undo').click();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.tracks.map(track=>({path:track.path,track:track.tags.track,disc:track.tags.disc})))).toEqual(before);
});

test('shared toggles visibly move and recolour when checked by pointer and keyboard',async({page})=>{
  await page.locator('#ribbon-autonumber').click();
  const control=page.locator('#autonum-total-control');
  const thumb=control.locator('[data-slot=switch-thumb]');
  const off=await control.evaluate(node=>({background:getComputedStyle(node).backgroundColor,x:node.querySelector('[data-slot=switch-thumb]').getBoundingClientRect().x}));
  await control.click();await expect(control).toHaveAttribute('aria-checked','true');
  await expect(page.locator('#autonum-total')).toBeChecked();
  const accent=await page.evaluate(()=>{const sample=document.createElement('span');sample.style.color='var(--accent)';document.body.append(sample);const color=getComputedStyle(sample).color;sample.remove();return color;});
  await expect.poll(()=>control.evaluate(node=>getComputedStyle(node).backgroundColor)).toBe(accent);
  await expect.poll(()=>thumb.evaluate(node=>node.getBoundingClientRect().x)).toBeGreaterThan(off.x+10);
  await control.focus();await page.keyboard.press('Space');await expect(control).toHaveAttribute('aria-checked','false');
  await expect.poll(()=>control.evaluate(node=>getComputedStyle(node).backgroundColor)).toBe(off.background);
  await expect.poll(()=>thumb.evaluate(node=>node.getBoundingClientRect().x)).toBeCloseTo(off.x,0);
});

test('Vinyl converts side positions into a paired disc/track stage and leaves numeric tracks alone',async({page})=>{
  await page.evaluate(()=>{window.tagrex.tracks[0].tags.track='A1';window.tagrex.tracks[1].tags.track='B3';window.tagrex.tracks[2].tags.track='12';});
  await page.locator('#ribbon-select-all').click();await page.locator('#ribbon-autonumber').click();
  await page.getByRole('tab',{name:'Vinyl',exact:true}).click();
  await expect(page.locator('#ribbon-number-dialog .vinyl-results')).toHaveCount(0);
  await expect(page.locator('#autonum-run')).toBeEnabled();await page.locator('#autonum-run').click();
  await expect(page.locator('#ribbon-number-dialog')).toBeHidden();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes.filter(change=>change.tag_changes.some(edit=>edit.field==='disc')).length)).toBe(2);
  const converted=await page.evaluate(()=>window.tagrex.previewPlan.changes.map(change=>Object.fromEntries(change.tag_changes.map(edit=>[edit.field,edit.new]))));
  expect(converted).toContainEqual(expect.objectContaining({disc:'1',track:'1','custom:POSITION':'A1'}));
  expect(converted).toContainEqual(expect.objectContaining({disc:'2',track:'3','custom:POSITION':'B3'}));
  expect(await page.evaluate(()=>window.tagrex.tracks.map(track=>track.tags.track))).toEqual(['A1','B3','12']);
});

test('numbering options use two columns and short subtitles without a bottom preview',async({page})=>{
  await page.locator('#ribbon-autonumber').click();
  const dialog=page.locator('#ribbon-number-dialog');
  await expect(dialog.locator('#autonum-reset-album')).toHaveCount(0);
  await expect(dialog.locator('#autonum-keep-sides')).toHaveCount(0);
  for(const tab of ['Autonumber','Disc','Vinyl']){
    await dialog.getByRole('tab',{name:tab,exact:true}).click();
    const panel=dialog.getByRole('tabpanel');
    const subtitles=await panel.locator('.autonumber-option small').allTextContents();
    expect(subtitles.every(text=>text.trim().split(/\s+/).length<=4)).toBe(true);
    expect(await panel.locator('.autonumber-options').evaluate(node=>getComputedStyle(node).gridTemplateColumns.split(' ').length)).toBe(2);
    await expect(dialog.locator('.vinyl-results,.vinyl-result')).toHaveCount(0);
  }
});

test('valid option updates do not fade the numbering footer and immediate Stage uses the newest request',async({page})=>{
  await page.locator('#ribbon-select-all').click();await page.locator('#ribbon-autonumber').click();
  const stage=page.locator('#autonum-run');await expect(stage).toBeEnabled();
  await page.locator('#autonum-start').fill('7');
  await page.locator('#autonum-total-control').click();
  await expect(stage).toBeEnabled();
  await stage.click();await expect(page.locator('#ribbon-number-dialog')).toBeHidden();
  await expect.poll(()=>page.evaluate(()=>window.tagrex.previewPlan?.changes.map(change=>change.tag_changes.find(edit=>edit.field==='track')?.new))).toEqual(['7','8','9']);
});


test('Vinyl explanation is available from its label info icon',async({page})=>{
  await page.locator('#ribbon-autonumber').click();await page.getByRole('tab',{name:'Vinyl',exact:true}).click();
  const panel=page.locator('#number-vinyl-panel'),info=panel.getByRole('button',{name:'About vinyl positions'});
  await expect(info).toBeVisible();await expect(panel.locator('.vinyl-explanation')).toHaveCount(0);
  await expect(info).toHaveAccessibleDescription(/B3 becomes Disc 2, Track 3/);
  await info.focus();await expect(page.locator('#controller-tooltip-content')).toContainText('Plain numbers stay unchanged.');
});
