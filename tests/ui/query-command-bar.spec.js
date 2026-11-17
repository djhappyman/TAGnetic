import { openTagSource, toggleDuplicates } from './navigation.js';
import {test,expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
const root=resolve('.test-dist');
const errors=new WeakMap();
test.beforeEach(async({page})=>{
 const issues=[];errors.set(page,issues);page.on('pageerror',e=>issues.push(e.message));
 await page.route('http://tagrex.test/**',async route=>{
  const path=resolve(root,'.'+new URL(route.request().url()).pathname);if(!path.startsWith(root+sep))return route.abort();
  try{await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});}catch{await route.fulfill({status:404,body:'Missing fixture'});}
 });
 await page.goto('http://tagrex.test/app/ui/index.html');
 await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
 await page.locator('#root-display').click();await page.locator('#root').fill('/music');await page.locator('#root').press('Enter');
 await expect(page.locator('#tracks-body tr[data-path]')).toHaveCount(3);
 await page.evaluate(()=>{window.tagrex.tracks.forEach((t,i)=>{t.tags.genre=i===2?'Jungle':'Hardcore';});window.tagrex.renderTracks();});
});
test.afterEach(async({page})=>expect(errors.get(page)).toEqual([]));
const rows=page=>page.locator('#tracks-body tr[data-path]');
async function chip(page,text,label){await page.locator('#filter').fill(text);await page.getByRole('option',{name:label,exact:false}).first().click();}
async function pauseClock(page){
 const time=new Date('2026-01-01T00:00:00Z');
 await page.clock.install({time});
 await page.clock.pauseAt(new Date(time.getTime()+1000));
}
async function openHydratedDuplicates(page){
 await page.evaluate(()=>{window.duplicateDetailHydrated=false;document.addEventListener('tagrex:request-duplicate-selection',()=>window.duplicateDetailHydrated=true,{once:true});});
 await toggleDuplicates(page);
 await expect.poll(()=>page.evaluate(()=>window.duplicateDetailHydrated)).toBe(true);
}
test('search label is inset and switching modes starts a fresh query with the keyboard',async({page})=>{
 const toggle=page.getByRole('tab',{name:'Duplicates',exact:true});
 await expect(page.locator('#query-command-mode')).toHaveCount(0);
 await expect(page.locator('.query-mode-label')).toHaveText('');await expect(page.locator('.query-mode-divider')).toHaveCount(0);
 await expect(page.locator('.query-mode-label use')).toHaveAttribute('href','#i-search');
 await expect(page.locator('#filter')).toHaveAttribute('placeholder','Start typing…');
 await chip(page,'hardcore','Genre: Hardcore');await page.locator('#filter').fill('Radio');await page.locator('#filter').press('Escape');
 await toggle.focus();await page.keyboard.press('Space');
 await expect(toggle).toHaveAttribute('aria-pressed','true');await expect(page.locator('.query-mode-label')).toHaveText('');
 await expect(page.locator('#query-duplicate-kind')).toBeVisible();
 await expect(page.locator('#filter')).toHaveValue('');await expect(page.locator('.filter-condition-chip')).toHaveCount(0);
 await expect(page.locator('#filter-panel #query-duplicate-scan, #filter-panel #query-duplicate-kind, #filter-panel #filter-feedback')).toHaveCount(0);
 await expect(page.locator('#panel-deduplicator #query-duplicate-scan')).toBeVisible();await expect(page.locator('#panel-deduplicator #filter-feedback')).toBeVisible();
 await page.locator('#filter').fill('Hardcore');await page.locator('.mode-tab[data-mode="tagger"]').focus();
 await page.keyboard.press('Enter');await expect(toggle).toHaveAttribute('aria-pressed','false');
 await expect(page.locator('.query-mode-label')).toHaveText('');await expect(page.locator('#query-duplicate-kind')).toHaveCount(0);
 await expect(page.locator('#filter')).toHaveValue('');await expect(page.locator('.filter-condition-chip')).toHaveCount(0);await expect(rows(page)).toHaveCount(3);
});
test('ready state keeps open files visible with Scan files beneath its message',async({page})=>{
 await expect(page.locator('#duplicate-actionbar')).toHaveCount(0);
 await toggleDuplicates(page);
 const scan=page.locator('#query-duplicate-scan');await expect(scan).toHaveText('Scan files');await expect(scan).toBeEnabled();
 await expect(rows(page)).toHaveCount(3);await expect(page.locator('#panel-deduplicator #query-duplicate-scan')).toHaveCount(1);await expect(page.locator('#diff-actionbar')).toBeVisible();await expect(page.locator('#diff-apply')).toBeDisabled();
 await expect(page.locator('#filter-feedback h3')).toHaveText('Find duplicate files');
 for(const width of [1280,620]){
  await page.setViewportSize({width,height:800});await expect(scan).toBeVisible();
  const geometry=await scan.evaluate(node=>{const box=node.getBoundingClientRect(),message=document.getElementById('filter-feedback').getBoundingClientRect();return {left:box.left,right:box.right,height:box.height,top:box.top,messageBottom:message.bottom,viewport:innerWidth};});
  expect(geometry.height).toBeCloseTo(32,1);expect(geometry.top).toBeGreaterThanOrEqual(geometry.messageBottom-0.5);expect(geometry.left).toBeGreaterThanOrEqual(0);expect(geometry.right).toBeLessThanOrEqual(geometry.viewport);
 }
 await expect(page.locator('.mode-col .inspector-help')).toHaveCount(0);
 await page.getByRole('tab',{name:'Tags',exact:true}).click();await expect(scan).toHaveCount(0);
});
test('selected duplicate result shows shared tags, differences and every copy in the sidebar',async({page})=>{
 await page.evaluate(()=>{const t=window.tagrex.tracks;for(const track of t.slice(0,2)){track.mockHash='matching';track.tags.artist='Hixxy';track.tags.title='Test Tune';}t[0].tags.album='First copy';t[1].tags.album='Second copy';});
 await toggleDuplicates(page);await expect(rows(page)).toHaveCount(3);await page.locator('#query-duplicate-scan').click();
 await expect(page.locator('tr.duplicate-group')).toHaveCount(1);await expect(rows(page)).toHaveCount(2);await rows(page).first().click();
 await expect.poll(()=>page.evaluate(()=>window.tagrex.selection.size)).toBe(2);
 const result=page.locator('#duplicate-result-details');await expect(result).toBeVisible();await expect(result.locator('.duplicate-copy-count')).toHaveText('2 copies selected');
 await expect(result.getByRole('region',{name:'Shared tags'})).toContainText('Hixxy');await expect(result.getByRole('region',{name:'Tag differences'})).toContainText('Album');
 await expect(result.locator('.duplicate-copy')).toHaveCount(2);await result.locator('summary').first().click();await expect(result.locator('.duplicate-copy-body').first()).toContainText('First copy');
 await expect(result.locator('.duplicate-copy-path').first()).toContainText('/music/');
 await toggleDuplicates(page);await expect(result).toHaveCount(0);
});
test('large group details page copies without mounting the whole group',async({page})=>{
 await page.evaluate(async()=>{
  await window.tagrex.invoke('open_drop',{paths:Array.from({length:45},(_,i)=>`copies/copy-${i}.flac`)});
  document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();
  for(const track of window.tagrex.tracks){track.mockHash='same-copies';track.tags.title='Many copies';}
 });
 await openHydratedDuplicates(page);
 await page.locator('#query-duplicate-scan').click();
 await expect(page.locator('tr.duplicate-group')).toHaveCount(1);await expect(rows(page)).toHaveCount(45);await rows(page).first().click();
 const result=page.locator('#duplicate-result-details');await expect(result.locator('.duplicate-copy')).toHaveCount(20);await expect(result.locator('.duplicate-copy-pages')).toContainText('1–20 of 45');
 await result.getByRole('button',{name:'Next copies',exact:true}).click();await expect(result.locator('.duplicate-copy-pages')).toContainText('21–40 of 45');
 await result.getByRole('button',{name:'Next copies',exact:true}).click();await expect(result.locator('.duplicate-copy')).toHaveCount(5);await expect(result.locator('.duplicate-copy-pages')).toContainText('41–45 of 45');
 await toggleDuplicates(page);await expect(result).toHaveCount(0);
});
test('committed filter is one tinted pill with matching remove ink and accessible hit areas',async({page})=>{
 await chip(page,'hardcore','Genre: Hardcore');
 const pill=page.locator('.filter-condition-chip');await expect(pill.locator('.query-committed-value')).toHaveText('Hardcore');
 await expect(pill.locator('.query-value-chip,.query-field-label')).toHaveCount(0);
 const paint=await pill.evaluate(node=>{const value=node.querySelector('.query-committed-value'),remove=node.lastElementChild,css=getComputedStyle(node),r=remove.getBoundingClientRect();return {background:css.backgroundColor,radius:parseFloat(css.borderRadius),color:css.color,valueBackground:getComputedStyle(value).backgroundColor,removeColor:getComputedStyle(remove).color,width:r.width,height:r.height};});
 expect(paint.background).not.toBe('rgba(0, 0, 0, 0)');expect(paint.radius).toBeGreaterThan(0);expect(paint.valueBackground).toBe('rgba(0, 0, 0, 0)');expect(paint.removeColor).toBe(paint.color);expect(paint.width).toBeGreaterThanOrEqual(24);expect(paint.height).toBeGreaterThanOrEqual(24);
 const remove=pill.getByRole('button',{name:'Remove Genre: Hardcore',exact:true});await pill.getByRole('button',{name:'Edit Genre: Hardcore',exact:true}).focus();await page.keyboard.press('Tab');await expect(remove).toBeFocused();
 expect(await remove.evaluate(node=>getComputedStyle(node).outlineStyle)).toBe('solid');await page.keyboard.press('Enter');await expect(pill).toHaveCount(0);await expect(rows(page)).toHaveCount(3);
});
test('header owns duplicate navigation and an in-field preset button at desktop and narrow widths',async({page})=>{
 for(const width of [1280,620,375]){
  await page.setViewportSize({width,height:800});
  await expect(page.locator('.mode-tabs #query-mode-toggle')).toBeInViewport();
  await expect(page.locator('.topbar #filter-panel .field-tools #presets-btn')).toBeInViewport();
  const layout=await page.locator('.filter-ctl').boundingBox();
  expect(layout.x).toBeGreaterThanOrEqual(0);expect(layout.x+layout.width).toBeLessThanOrEqual(width);
 }
});
test('suggested chips live inside the input and plain text narrows them',async({page})=>{
 await expect(page.locator('#filter-add')).toHaveCount(0);
 await chip(page,'hardcore','Genre: Hardcore');
 await expect(page.locator('.filter-ctl .filter-condition-chip')).toHaveCount(1);
 await expect(rows(page)).toHaveCount(2);
 await page.locator('#filter').fill('Radio');await expect(rows(page)).toHaveCount(1);await expect(rows(page)).toContainText('Wish Mountain');
 await page.locator('#filter-clear').click();await expect(rows(page)).toHaveCount(2);
 await page.getByRole('button',{name:'Remove Genre: Hardcore',exact:true}).click();await expect(rows(page)).toHaveCount(3);
});
test('editing a chip adds OR values and a numeric refinement',async({page})=>{
 await chip(page,'hardcore','Genre: Hardcore');
 await page.getByRole('button',{name:'Edit Genre: Hardcore',exact:true}).click();
 await page.locator('#query-editor input:not([type=checkbox])').fill('Hardcore; Jungle');
 await page.getByRole('button',{name:'Save filter',exact:true}).click();await expect(rows(page)).toHaveCount(3);
 await expect(page.locator('.filter-condition-chip')).toContainText('Hardcore or Jungle');
 await page.locator('#filter').fill('');await page.locator('#filter').press('Alt+Enter');
 await page.locator('#query-editor select[id^=field-]').selectOption('year',{force:true});
 await page.locator('#query-editor select[id^=operator-]').selectOption('between',{force:true});
 await page.locator('#query-editor input:not([type=checkbox])').fill('1996; 1996');
 await page.getByRole('button',{name:'Save filter',exact:true}).click();await expect(rows(page)).toHaveCount(3);
});
test('keyboard chooses a suggestion and Escape keeps the query and selected files',async({page})=>{
 const before=await page.evaluate(()=>[...window.tagrex.selection]);
 await page.locator('#filter').fill('hardcore');await expect(page.getByRole('option',{name:'Genre: Hardcore',exact:false})).toBeVisible();
 await page.locator('#filter').press('ArrowDown');await page.locator('#filter').press('Escape');
 await expect(page.locator('#filter')).toHaveValue('hardcore');await expect(page.locator('#query-suggestions')).toHaveCount(0);
 expect(await page.evaluate(()=>[...window.tagrex.selection])).toEqual(before);
});
test('saved presets restore chips and duplicates mode without storing file selections',async({page})=>{
 await chip(page,'hardcore','Genre: Hardcore');
 await page.locator('#presets-btn').click();await page.locator('.preset-name').fill('Hardcore workflow');await page.locator('.preset-save button').click();
 await page.keyboard.press('Escape');await page.getByRole('button',{name:'Remove Genre: Hardcore',exact:true}).click();
 await page.locator('#presets-btn').click();await page.getByRole('button',{name:'Hardcore workflow',exact:true}).click();
 await expect(rows(page)).toHaveCount(2);
 const saved=await page.evaluate(()=>JSON.parse(localStorage.getItem('tagrex.filterPresets.conditions.v2'))[0]);
 expect(saved.root.children[0].field).toBe('genre');expect(saved.selection).toBeUndefined();
 await toggleDuplicates(page);await expect(page.locator('#query-mode-toggle')).toHaveAttribute('aria-pressed','true');
 await expect(page.locator('#query-duplicate-kind')).toBeVisible();await expect(page.locator('#filter-feedback')).toContainText('Find duplicate files');
 await toggleDuplicates(page);await expect(page.locator('#query-mode-toggle')).toHaveAttribute('aria-pressed','false');await expect(rows(page)).toHaveCount(3);
});
test('duplicate groups keep companion copies visible when a chip matches one member',async({page})=>{
 await page.evaluate(()=>{const t=window.tagrex.tracks;t[0].mockHash='same';t[1].mockHash='same';t[1].tags.genre='House';});
 await toggleDuplicates(page);await page.locator('#query-duplicate-scan').click();
 await expect(page.locator('tr.duplicate-group')).toHaveCount(1);await expect(rows(page)).toHaveCount(2);
 await chip(page,'hardcore','Genre: Hardcore');await expect(rows(page)).toHaveCount(2);
 await expect(page.locator('tr.duplicate-group')).toContainText('2 copies');
});

test('suggestions wait for typed input and preview coloured case-insensitive chips',async({page})=>{
 const input=page.locator('#filter');await input.focus();await expect(page.locator('#query-suggestions')).toHaveCount(0);
 await pauseClock(page);await input.fill('HARDCORE');
 // Controller/runner latency must not consume the controlled debounce interval.
 await new Promise(resolve=>setTimeout(resolve,220));
 await page.clock.runFor(100);await expect(page.locator('#query-suggestions')).toHaveCount(0);
 await page.clock.runFor(120);await expect(page.getByRole('option',{name:'Genre: Hardcore',exact:false})).toBeVisible();
 await expect(page.locator('#query-suggestions .query-suggestion-chip').first()).toBeVisible();
 await expect(page.locator('#query-suggestions mark').first()).toHaveText(/hardcore/i);
});
test('one Artist contains rule covers variations and exact member matching remains editable',async({page})=>{
 await page.evaluate(()=>{const t=window.tagrex.tracks;t[0].tags.artist='Hixxy & Sharkey';t[1].tags.artist='Hixxy / Dougal';t[2].tags.artist='Hixxy2';});
 await page.locator('#filter').fill('HIXXY');
 await expect(page.getByRole('option',{name:'Artist contains: HIXXY',exact:true})).toHaveCount(1);
 await page.getByRole('option',{name:'Artist contains: HIXXY',exact:true}).click();
 await expect(rows(page)).toHaveCount(3);
 await page.getByRole('button',{name:'Edit Artist contains: HIXXY',exact:true}).click();
 await page.locator('#query-editor select[id^=operator-]').selectOption('member',{force:true});
 await page.getByRole('button',{name:'Save filter',exact:true}).click();
 await expect(rows(page)).toHaveCount(2);await expect(page.locator('.filter-condition-chip .query-committed-value')).toHaveText('HIXXY');
});
test('Enter before the suggestion delay keeps plain search and cancels the pending popup',async({page})=>{
 await pauseClock(page);await page.locator('#filter').fill('Radio');await page.locator('#filter').press('Enter');
 await page.clock.runFor(300);await expect(page.locator('#query-suggestions')).toHaveCount(0);
 await expect(rows(page)).toHaveCount(1);await expect(page.locator('.filter-condition-chip')).toHaveCount(0);
});
test('natural phrases preview explicit filters and leave ordinary searches available',async({page})=>{
 await page.locator('#filter').fill('hardcore from 1996 missing artwork');
 const proposal=page.getByRole('option').filter({has:page.locator('.query-suggestion-chip[data-field=genre]')}).first();
 await expect(proposal.locator('.query-suggestion-chip')).toHaveCount(3);await proposal.click();
 await expect(page.locator('.filter-condition-chip')).toHaveCount(3);await expect(rows(page)).toHaveCount(1);await expect(rows(page)).toContainText('Radio');
 await expect(page.locator('#filter')).toHaveValue('');
});
test('leaving duplicates cancels a scan whose native startup finishes later',async({page})=>{
 await page.evaluate(()=>{
  const resolve=Promise.resolve;window.cancelledScanCalls=0;let hold=true;
  Promise.resolve=function(value){
   if(hold&&value?.jobId===1&&value.snapshotId===1&&value.status==='complete'){
    hold=false;return new Promise(done=>window.releaseScan=()=>done(value));
   }
   if(value?.status==='cancelled')window.cancelledScanCalls++;
   return resolve.call(this,value);
  };
 });
 await toggleDuplicates(page);await page.locator('#query-duplicate-scan').click();
 await expect(page.locator('#query-duplicate-cancel')).toBeVisible();
 await toggleDuplicates(page);await expect(page.locator('#query-mode-toggle')).toHaveAttribute('aria-pressed','false');
 await page.evaluate(()=>window.releaseScan());
 await expect.poll(()=>page.evaluate(()=>window.cancelledScanCalls)).toBe(1);
 await expect(rows(page)).toHaveCount(3);await expect(page.locator('#filter-feedback')).not.toContainText(/Scanning|cancelled/i);
});
test('duplicates search and suggestions use only scan members',async({page})=>{
 await page.evaluate(()=>{window.tagrex.tracks[0].mockHash='copy';window.tagrex.tracks[1].mockHash='copy';});
 await toggleDuplicates(page);
 await page.locator('#filter').fill('Jungle');
 await page.waitForTimeout(240);await expect(page.locator('#query-suggestions')).toHaveCount(0);
 await page.locator('#query-duplicate-scan').click();
 await expect(page.locator('#panel-deduplicator #filter-feedback')).toContainText(/groups/);
 await page.locator('#filter').fill('Hardcore');
 await expect(page.getByRole('option',{name:'Genre: Hardcore',exact:true})).toBeVisible();
 await page.locator('#filter').fill('Jungle');
 await expect(page.getByRole('option',{name:'Search everywhere: Jungle',exact:true})).toBeVisible();
 await expect(page.locator('#query-suggestions .query-suggestion-chip')).toHaveCount(0);
 await expect(rows(page)).toHaveCount(0);
});

async function progressiveFixture(page,size=4){
 await page.evaluate(async size=>{
  await window.tagrex.invoke('open_drop',{paths:Array.from({length:size},(_,i)=>`recording-${String(i).padStart(5,'0')}.mp3`)});
  document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();
  window.tagrex.tracks.forEach((track,i)=>{track.mockHash=`pair-${Math.floor(i/2)}`;track.tags.title=`Recording ${Math.floor(i/2)+1}`;track.tags.genre=i<2?'Hardcore':'Jungle';});
  window.tagrex.renderTracks();
  const resolve=Promise.resolve;let first=true;
  Promise.resolve=function(value){
   if(first&&value?.jobId===1&&value.snapshotId===1&&value.status==='complete'){
    first=false;const groups=value.groups;
    window.publishDuplicateGroups=(count,status='running')=>Object.assign(value,{groups:groups.slice(0,count),groupsCount:count,groupsRevision:value.groupsRevision+1,status,phase:status==='complete'?'complete':'hashing',processed:count*2});
    window.publishDuplicateGroups(1);
   }
   return resolve.call(this,value);
  };
 },size);
 await toggleDuplicates(page);await page.locator('#query-duplicate-scan').click();
 await expect(page.locator('tr.duplicate-group')).toHaveCount(1);await expect(rows(page)).toHaveCount(2);
}

test('verified groups appear during scanning under track names and retain focused copy rows',async({page})=>{
 await progressiveFixture(page);
 await expect(page.locator('#filter-feedback')).toHaveAttribute('data-scan-status','running');
 await expect(page.locator('tr.duplicate-group .group-label')).toHaveText('Recording 1');
 await expect(rows(page).first().locator('td[data-field="title"]')).toContainText('Recording 1');
 const cell=rows(page).first().locator('td[data-field="title"]');await cell.click();
 await page.evaluate(()=>{window.retainedCopy=document.querySelector('#tracks-body tr[data-path]');window.retainedCell=document.activeElement;window.retainedSelection=[...window.tagrex.selection];window.publishDuplicateGroups(2,'complete');});
 await expect(page.locator('tr.duplicate-group')).toHaveCount(2);await expect(rows(page)).toHaveCount(4);
 await expect(page.locator('tr.duplicate-group .group-label')).toHaveText(['Recording 1','Recording 2']);
 expect(await page.evaluate(()=>({row:window.retainedCopy===document.querySelector('#tracks-body tr[data-path]'),connected:window.retainedCopy.isConnected,focus:document.activeElement===window.retainedCell,selected:JSON.stringify([...window.tagrex.selection])===JSON.stringify(window.retainedSelection)}))).toEqual({row:true,connected:true,focus:true,selected:true});
 await expect(page.locator('#filter-feedback')).toHaveAttribute('data-scan-status','complete');
});

test('running suggestions use verified groups and Cancel clears partial results and stale replies',async({page})=>{
 await progressiveFixture(page);
 await page.locator('#filter').fill('Jungle');await expect(page.getByRole('option',{name:'Search everywhere: Jungle',exact:true})).toBeVisible();
 await expect(page.locator('#query-suggestions .query-suggestion-chip')).toHaveCount(0);
 await page.locator('#filter-clear').click();await expect(rows(page)).toHaveCount(2);
 await page.evaluate(()=>{
  const resolve=Promise.resolve;let first=true;
  Promise.resolve=function(value){
   if(first&&value?.duplicateStatus==='running'&&value.groups?.length){first=false;return new Promise(done=>window.releaseOldQuery=()=>done(value));}
   return resolve.call(this,value);
  };
 });
 await page.locator('#filter').fill('Recording');await expect.poll(()=>page.evaluate(()=>typeof window.releaseOldQuery)).toBe('function');
 await page.locator('#query-duplicate-cancel').click();await expect(rows(page)).toHaveCount(0);
 await page.evaluate(()=>window.releaseOldQuery());await page.waitForTimeout(400);
 await expect(rows(page)).toHaveCount(0);await expect(page.locator('#filter-feedback')).toHaveAttribute('data-scan-status','cancelled');
});

for(const size of [20000,50000,100000])test(`large progressive groups preserve the scroll anchor and mount a bounded window (${size} files)`,async({page},info)=>{
 test.skip(size>20000&&!process.env.TAGREX_DUPLICATE_BENCH,'Opt-in serial duplicate geometry measurement');
 test.setTimeout(60000);
 await progressiveFixture(page,size);
 await page.evaluate(size=>{window.partialGroups=Math.floor(size/5);window.partialReady=false;const ready=event=>{if(!event.detail.pending&&event.detail.matched===window.partialGroups*2){window.partialReady=true;document.removeEventListener('tagrex:filter-state',ready);}};document.addEventListener('tagrex:filter-state',ready);window.publishDuplicateGroups(window.partialGroups);},size);
 await expect.poll(()=>page.evaluate(()=>window.partialReady)).toBe(true);
 await expect.poll(()=>page.evaluate(()=>document.querySelector('#files-view').scrollHeight)).toBeGreaterThan(100000);
 const sample=await page.evaluate(async size=>{
  const view=document.getElementById('files-view');view.scrollTop=100000;await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
  const anchor=document.querySelector('#tracks-body tr[data-path]'),path=anchor.dataset.path,top=anchor.getBoundingClientRect().top;
  const started=performance.now();let tick=started,maxHeartbeatGapMs=0;
  const timer=setInterval(()=>{const now=performance.now();maxHeartbeatGapMs=Math.max(maxHeartbeatGapMs,now-tick);tick=now;},4);
  await new Promise(resolve=>{const ready=event=>{if(!event.detail.pending&&event.detail.matched===size){document.removeEventListener('tagrex:filter-state',ready);requestAnimationFrame(()=>requestAnimationFrame(resolve));}};document.addEventListener('tagrex:filter-state',ready);window.publishDuplicateGroups(size/2,'complete');});
  clearInterval(timer);const row=document.querySelector(`#tracks-body tr[data-path="${path}"]`);
  return {files:size,groups:size/2,mounted:document.querySelectorAll('#tracks-body tr:not(.spacer)').length,retained:row===anchor,anchorDeltaPx:row?Math.abs(row.getBoundingClientRect().top-top):null,publishToPaintMs:performance.now()-started,maxHeartbeatGapMs};
 },size);
 await expect(page.locator('#filter-feedback')).toHaveAttribute('data-scan-status','complete');
 expect(sample.mounted).toBeLessThan(100);expect(sample.retained).toBe(true);expect(sample.anchorDeltaPx).not.toBeNull();expect(sample.anchorDeltaPx).toBeLessThanOrEqual(1);
 if(process.env.TAGREX_DUPLICATE_BENCH)console.log(`duplicate-progress ${JSON.stringify(sample)}`);
 await info.attach(`duplicate-progress-${size}.json`,{body:JSON.stringify(sample,null,2),contentType:'application/json'});
});

async function captureDuplicateDetails(page){
 await page.evaluate(()=>{
  window.duplicateDetails=[];
  document.addEventListener('tagrex:duplicate-selection',event=>{window.duplicateDetail=event.detail.group;window.duplicateDetails.push(event.detail.group);});
  document.dispatchEvent(new Event('tagrex:request-duplicate-selection'));
 });
}

test('duplicates keeps the normal table until explicit Scan and clears it while startup is pending',async({page})=>{
 await captureDuplicateDetails(page);expect(await page.evaluate(()=>window.duplicateDetail)).toBeNull();
 const before=await rows(page).evaluateAll(nodes=>nodes.map(node=>node.dataset.path));
 await page.evaluate(()=>{
  window.tagrex.tracks[0].mockHash='same';window.tagrex.tracks[1].mockHash='same';
  const resolve=Promise.resolve;let first=true;
  Promise.resolve=function(value){if(first&&value?.jobId===1&&value.status==='complete'){first=false;return new Promise(done=>window.finishScanStartup=()=>done(value));}return resolve.call(this,value);};
 });
 await toggleDuplicates(page);await expect(rows(page)).toHaveCount(3);
 await page.locator('#filter').fill('unmatched text');await page.locator('#filter').press('Escape');
 expect(await rows(page).evaluateAll(nodes=>nodes.map(node=>node.dataset.path))).toEqual(before);
 await page.locator('#filter-clear').click();await page.locator('#query-duplicate-scan').click();
 await expect(rows(page)).toHaveCount(0);await expect(page.locator('tr.duplicate-group')).toHaveCount(0);
 await page.evaluate(()=>window.finishScanStartup());await expect(rows(page)).toHaveCount(2);
});

test('row and header activation select all group copies and publish the saved DTO detail contract',async({page})=>{
 await captureDuplicateDetails(page);await progressiveFixture(page);
 await page.evaluate(()=>{
  const tracks=window.tagrex.tracks;tracks[0].tags.album='First album';tracks[1].tags.album='Second album';
  tracks[0].has_cover=true;tracks[1].has_cover=false;tracks[0].duration_secs=210;tracks[1].duration_secs=211;
  window.publishDuplicateGroups(2,'complete');
 });
 await expect(rows(page)).toHaveCount(4);await rows(page).first().locator('td.file').click();
 await expect.poll(()=>page.evaluate(()=>window.tagrex.selection.size)).toBe(2);
 const detail=await page.evaluate(()=>({group:window.duplicateDetail,paths:[...window.tagrex.selection]}));
 expect(Object.keys(detail.group)).toEqual(['id','title','reason','tracks','sharedTags','differingTags']);
 expect(detail.group.title).toBe('Recording 1');expect(detail.group.reason).toBe('Browser fixture evidence');
 expect(detail.group.sharedTags.genre).toBe('Hardcore');expect(detail.group.differingTags).toContain('album');
 expect(detail.group.tracks.map(track=>track.path)).toEqual(detail.paths);
 expect(detail.group.tracks.map(track=>track.has_cover)).toEqual([true,false]);
 await page.locator('tr.duplicate-group').nth(1).locator('.group-label').click();
 await expect.poll(()=>page.evaluate(()=>window.duplicateDetail?.title)).toBe('Recording 2');
 await expect(rows(page).nth(0)).not.toHaveClass(/selected/);await expect(rows(page).nth(2)).toHaveClass(/selected/);await expect(rows(page).nth(3)).toHaveClass(/selected/);
 await expect(page.locator('tr.duplicate-group').nth(1)).toHaveAttribute('aria-expanded','true');
 await page.locator('tr.duplicate-group').first().focus();await page.keyboard.press('Space');
 await expect.poll(()=>page.evaluate(()=>window.duplicateDetail?.title)).toBe('Recording 1');
 await page.locator('tr.duplicate-group').first().locator('.group-caret').click();
 await expect(page.locator('tr.duplicate-group').first()).toHaveAttribute('aria-expanded','false');
 expect(await page.evaluate(()=>window.duplicateDetail?.title)).toBe('Recording 1');
 await page.keyboard.press('ArrowRight');await expect(page.locator('tr.duplicate-group').first()).toHaveAttribute('aria-expanded','true');
 await rows(page).nth(2).focus();await expect(rows(page).nth(2)).toBeFocused();await page.keyboard.press('Enter');
 await expect.poll(()=>page.evaluate(()=>window.duplicateDetail?.title)).toBe('Recording 2');
 await page.evaluate(()=>{window.duplicateDetail=null;document.dispatchEvent(new Event('tagrex:request-duplicate-selection'));});
 expect(await page.evaluate(()=>window.duplicateDetail?.tracks.length)).toBe(2);
});

test('a selected verified group grows atomically and refreshes its tag differences',async({page})=>{
 await captureDuplicateDetails(page);await progressiveFixture(page);await rows(page).first().locator('td.file').click();
 const id=await page.evaluate(()=>window.duplicateDetail.id);
 await page.evaluate(()=>window.publishDuplicateGroups(1,'complete').groups[0].indices.push(2));
 await expect(rows(page)).toHaveCount(3);await expect(page.locator('tr.duplicate-group .group-count')).toHaveText('3 copies');
 await expect.poll(()=>page.evaluate(()=>window.tagrex.selection.size)).toBe(3);
 expect(await page.evaluate(()=>({id:window.duplicateDetail.id,count:window.duplicateDetail.tracks.length,different:window.duplicateDetail.differingTags.includes('title')}))).toEqual({id,count:3,different:true});
});

test('filter exclusion, Cancel, method and mode changes dismiss group detail without losing selected paths',async({page})=>{
 await captureDuplicateDetails(page);await progressiveFixture(page);await rows(page).first().locator('td.file').click();
 const selected=await page.evaluate(()=>[...window.tagrex.selection]);
 await page.locator('#filter').fill('Jungle');await expect(rows(page)).toHaveCount(0);
 expect(await page.evaluate(()=>window.duplicateDetail)).toBeNull();expect(await page.evaluate(()=>[...window.tagrex.selection])).toEqual(selected);
 await page.locator('#filter-clear').click();await expect(rows(page)).toHaveCount(2);await rows(page).first().locator('td.file').click();
 await page.locator('#query-duplicate-cancel').click();await expect(rows(page)).toHaveCount(0);
 expect(await page.evaluate(()=>window.duplicateDetail)).toBeNull();
 await page.evaluate(()=>document.dispatchEvent(new CustomEvent('tagrex:query-duplicate-kind',{detail:{kind:'same_audio'}})));
 await expect(rows(page)).toHaveCount(4);expect(await page.evaluate(()=>[...window.tagrex.selection])).toEqual(selected);
 await toggleDuplicates(page);await expect(rows(page)).toHaveCount(4);
 expect(await page.evaluate(()=>window.duplicateDetail)).toBeNull();
});

test('duplicate review preserves pending editor values and the staged plan while showing saved tags',async({page})=>{
 await captureDuplicateDetails(page);
 await page.evaluate(()=>{window.tagrex.tracks[0].mockHash='copy';window.tagrex.tracks[1].mockHash='copy';});
 await openTagSource(page, 'editor');
 await page.locator('#fields-body [data-key="title"]').fill('Pending title');await expect(page.locator('#diff-apply')).toBeEnabled();
 const plan=await page.evaluate(()=>JSON.stringify(window.tagrex.previewPlan));
 await toggleDuplicates(page);await expect(rows(page)).toHaveCount(3);
 await page.locator('#query-duplicate-scan').click();await expect(rows(page)).toHaveCount(2);
 await rows(page).first().locator('td.file').click();
 expect(await page.evaluate(()=>JSON.stringify(window.tagrex.previewPlan))).toBe(plan);
 expect(await page.evaluate(()=>window.duplicateDetail.tracks.some(track=>track.tags.title==='Pending title'))).toBe(false);
 expect(await page.evaluate(()=>window.tagrex.querySession.edits)).toBeGreaterThan(0);
 await expect(page.locator('#diff-apply')).toBeDisabled();
});

test('query toggle reveals a closed compact inspector and moves keyboard focus into Scan',async({page})=>{
 await page.locator('#panel-toggle').click();await page.setViewportSize({width:620,height:800});
 const toggle=page.locator('#query-mode-toggle');await expect(toggle).toBeVisible();await toggle.focus();await page.keyboard.press('Space');
 await expect(page.locator('#query-duplicate-scan')).toBeVisible();await expect(page.getByRole('tab',{name:'Inspector',exact:true})).toHaveAttribute('aria-selected','true');
 await expect(page.locator('#query-duplicate-scan')).toBeFocused();
});

test('a ready-state query error remains visible instead of showing the scan prompt',async({page})=>{
 await openHydratedDuplicates(page);
 await page.locator('#filter').fill('x'.repeat(2049));
 await expect(page.locator('#filter')).toHaveAttribute('aria-invalid','true');
 const message='Filter text must be at most 2048 characters.';
 await expect(page.locator('#filter-feedback')).toContainText(message);
 await page.evaluate(()=>document.dispatchEvent(new Event('tagrex:request-filter-state')));
 await expect(page.locator('#filter-feedback')).toContainText(message);
 await expect(page.locator('#filter-feedback')).not.toHaveClass(/duplicate-ready/);
 await expect(page.locator('#filter-feedback h3')).toHaveCount(0);
});

for (const theme of ['light','dark']) test(`header search grows with the window and keeps the file pane stable in ${theme}`, async ({page}) => {
 await page.evaluate(theme => document.documentElement.dataset.theme=theme,theme);
 await page.setViewportSize({width:1280,height:900});
 const narrow=await page.locator('#filter-panel').boundingBox();
 await page.setViewportSize({width:1600,height:900});
 const wide=await page.locator('#filter-panel').boundingBox();
 expect(wide.width).toBeGreaterThan(narrow.width+250);
 await expect(page.locator('.topbar #filter-panel')).toBeVisible();
 await expect(page.locator('.files-col #filter-panel')).toHaveCount(0);
 const before=await page.locator('.files-col').boundingBox();
 const selected=page.locator('.mode-tab[aria-selected="true"]');
 expect(await selected.evaluate(n=>getComputedStyle(n,'::after').content)).toBe('none');
 expect(await selected.evaluate(n=>getComputedStyle(n).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)');
 await chip(page,'hardcore','Genre: Hardcore');
 const after=await page.locator('.files-col').boundingBox();
 expect(after.y).toBeCloseTo(before.y,0);expect(after.height).toBeCloseTo(before.height,0);
 const table=await page.locator('#files-view').boundingBox();
 expect(after.y+after.height-table.y-table.height).toBeCloseTo(8,0);
 expect(await page.locator('#files-view').evaluate(n=>getComputedStyle(n).maskImage)).toContain('8px');
 await page.locator('#filter').fill('artist');
 await expect(page.locator('#query-suggestions')).toBeVisible();
 const suggestions=await page.locator('#query-suggestions').boundingBox();
 const field=await page.locator('.filter-ctl').boundingBox();
 expect(suggestions.y).toBeGreaterThanOrEqual(field.y+field.height);
 await page.locator('#presets-btn').click();
 await expect(page.locator('#query-suggestions')).toBeHidden();
 await expect(page.locator('#presets-menu')).toBeVisible();
 const presets=await page.locator('#presets-menu').boundingBox();
 expect(presets.y).toBeGreaterThanOrEqual(field.y+field.height);
});
