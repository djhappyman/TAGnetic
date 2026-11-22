import { openTagSource } from './navigation.js';
import {test, expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {resolve, sep} from 'node:path';
const root=resolve('.test-dist');
test.use({viewport:{width:1440,height:900},reducedMotion:'reduce'});
test.beforeEach(async({page})=>{
  page.issues=[];page.on('pageerror',error=>page.issues.push(error.message));
  await page.addInitScript(()=>{
    window.pageCalls=[];window.pagingMode='normal';window.pendingPages=[];
    window.pagingHarness=args=>{
      window.pageCalls.push(structuredClone(args));
      const page=args.query.page,size=args.query.per_page,base=args.query.album==='New search'?1000:0;
      const hits=Array.from({length:Math.max(0,Math.min(size,12-(page-1)*size))},(_,i)=>({
        id:String(300001+base+(window.pagingMode==='repeat'?0:(page-1)*size)+i),artist:'Various',title:`Release ${base+(page-1)*size+i+1}`,year:2004,format:'CD',thumb_url:null,cover_url:null,
      }));
      if(window.pagingMode==='duplicates'&&hits.length>1)hits[1]={...hits[0]};
      if(page===2&&window.pagingMode==='fail'){window.pagingMode='normal';return Promise.reject(new Error('Temporary page failure'));}
      if(page===2&&window.pagingMode==='hold')return new Promise(resolve=>window.pendingPages.push(()=>resolve(hits)));
      return Promise.resolve(hits);
    };
  });
  await page.route('http://tagrex.test/**',async route=>{
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);if(!path.startsWith(root+sep))return route.abort();
    try{
      let body=await readFile(path);
      if(path.endsWith('.js'))body=Buffer.from(body.toString()
        .replace(/case\s*["'\x60]provider_search["'\x60]:/g,'case "provider_search": if(window.pagingHarness) return window.pagingHarness(arguments[1]);')
        .replace(/case\s*["'\x60]provider_fetch_release["'\x60]:/g,'case "provider_fetch_release": if(window.detailHarness) return window.detailHarness(arguments[1]);')
        .replace(/case\s*["'\x60]provider_fetch_image["'\x60]:/g,'case "provider_fetch_image": if(window.imageHarness) return window.imageHarness(arguments[1]);'));
      await route.fulfill({body,contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});
    }catch{await route.fulfill({status:404,body:'Missing fixture'});}
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await openTagSource(page, 'online');
  await page.locator('.online-scroll').evaluate(node=>{node.style.maxHeight='160px';});
  await page.locator('#discogs-token').evaluate(node=>node.value='fixture');
});
test.afterEach(async({page})=>expect(page.issues).toEqual([]));
async function search(page,query='Original search'){
  await page.locator('#discogs-query').fill(query);await page.locator('#discogs-query').press('Enter');
  await expect(page.locator('.release-card')).toHaveCount(5);
  await expect(page.locator('#discogs-search')).not.toHaveClass(/busy/);
}
async function bottom(page){await page.locator('.online-scroll').evaluate(node=>{node.scrollTop=node.scrollHeight;node.dispatchEvent(new Event('scroll'));});}
async function settle(page){await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));}

test('scroll appends once, keeps the existing cards and query, and stops after the final page',async({page})=>{
  await search(page);await page.locator('.release-card').first().evaluate(node=>window.firstCard=node);
  await page.locator('#discogs-query').fill('Unsubmitted text');
  await bottom(page);await expect(page.locator('.release-card')).toHaveCount(10);
  expect(await page.evaluate(()=>window.firstCard===document.querySelector('.release-card'))).toBe(true);
  await expect(page.locator('#discogs-search')).not.toHaveClass(/busy/);
  await bottom(page);await expect(page.locator('.release-card')).toHaveCount(12);
  await expect(page.locator('#discogs-search')).not.toHaveClass(/busy/);await bottom(page);await settle(page);
  expect(await page.evaluate(()=>window.pageCalls.map(call=>[call.query.page,call.query.album]))).toEqual([[1,'Original search'],[2,'Original search'],[3,'Original search']]);
  await expect(page.locator('#release-more')).toBeHidden();
  await expect(page.locator('.online-scroll')).not.toHaveClass(/can-scroll-down/);
});

test('Editor scrolling leaves hidden Online tracklists idle until the Online tab reopens', async ({page}) => {
  await page.evaluate(() => {
    window.detailHarness = ({releaseId}) => Promise.resolve({
      id: releaseId, artist: 'Fixture', title: 'Fixture', year: 2004,
      genres: [], styles: [], labels: [], images: [],
      tracks: Array.from({length: 160}, (_, i) => ({
        position: String(i + 1), artist: 'Fixture', title: `Track ${i + 1}`, duration_secs: 60,
      })),
    });
  });
  await search(page);
  const firstCard = page.locator('.release-card').first();
  await firstCard.locator('.release-head').click();
  await expect(firstCard.locator('.tk-t').first()).toHaveText('Track 1');
  await firstCard.evaluate(card => {
    const query = card.querySelector;
    window.tracklistChecks = 0;
    card.querySelector = function (selector) {
      if (selector === '.release-tracklist tbody') window.tracklistChecks++;
      return query.call(this, selector);
    };
  });
  await openTagSource(page, 'editor');
  const editor = page.locator('.editor-scroll');
  await editor.evaluate(port => {
    const filler = document.createElement('div');
    filler.style.cssText = 'height:1200px;flex:none';
    port.append(filler);
  });
  await expect(editor).toHaveClass(/can-scroll-down/);
  await settle(page);
  await editor.evaluate(port => { window.tracklistChecks = 0; port.scrollTop = 100; });
  await expect(editor).toHaveClass(/can-scroll-up/);
  await settle(page);
  expect(await page.evaluate(() => window.tracklistChecks)).toBe(0);

  await openTagSource(page, 'online');
  await page.locator('.online-scroll').evaluate(port => { port.scrollTop = 2600; });
  await expect.poll(() => page.evaluate(() => window.tracklistChecks)).toBeGreaterThan(0);
  await expect(firstCard.locator('.tk-t').first()).not.toHaveText('Track 1');
});

test('slow paging does not duplicate requests, and Stop rejects late results without restarting',async({page})=>{
  await search(page);await page.evaluate(()=>window.pagingMode='hold');await bottom(page);
  await expect.poll(()=>page.evaluate(()=>window.pendingPages.length)).toBe(1);
  for(let n=0;n<4;n++)await bottom(page);
  expect(await page.evaluate(()=>window.pageCalls.length)).toBe(2);
  await page.locator('#discogs-search').click();await page.evaluate(()=>window.pendingPages.shift()());
  await bottom(page);await settle(page);
  await expect(page.locator('.release-card')).toHaveCount(5);
  expect(await page.evaluate(()=>window.pageCalls.length)).toBe(2);
});

test('a failed page offers a small retry and never retries in a loop',async({page})=>{
  await search(page);await page.evaluate(()=>window.pagingMode='fail');await bottom(page);
  await expect(page.locator('#load-more')).toBeVisible();await bottom(page);await settle(page);
  expect(await page.evaluate(()=>window.pageCalls.length)).toBe(2);
  await page.locator('#load-more').click();await expect(page.locator('.release-card')).toHaveCount(10);
  expect(await page.evaluate(()=>window.pageCalls.map(call=>call.query.page))).toEqual([1,2,2]);
});

test('a newer search rejects the preceding slow page',async({page})=>{
  await search(page);await page.evaluate(()=>window.pagingMode='hold');await bottom(page);
  await expect.poll(()=>page.evaluate(()=>window.pendingPages.length)).toBe(1);
  await search(page,'New search');await page.evaluate(()=>window.pendingPages.shift()());await settle(page);
  await expect(page.locator('.release-card')).toHaveCount(5);
  await expect(page.locator('.release-card').first()).toHaveAttribute('data-id','301001');
});

test('hidden tabs do not page; repeated provider pages stop instead of looping',async({page})=>{
  await search(page);await openTagSource(page, 'editor');
  await bottom(page);await settle(page);expect(await page.evaluate(()=>window.pageCalls.length)).toBe(1);
  await page.evaluate(()=>window.pagingMode='repeat');await openTagSource(page, 'online');
  await bottom(page);await expect.poll(()=>page.evaluate(()=>window.pageCalls.length)).toBe(2);
  await expect(page.locator('#discogs-search')).not.toHaveClass(/busy/);await bottom(page);await settle(page);
  await expect(page.locator('.release-card')).toHaveCount(5);
  expect(await page.evaluate(()=>window.pageCalls.length)).toBe(2);
});

test('short result pages fill available space without requiring an impossible scroll',async({page})=>{
  await page.setViewportSize({width:1440,height:1600});
  await page.locator('.online-scroll').evaluate(node=>{node.style.maxHeight='none';});
  await page.locator('#discogs-query').fill('Original search');await page.locator('#discogs-query').press('Enter');
  await expect(page.locator('.release-card')).toHaveCount(12);
  expect(await page.evaluate(()=>window.pageCalls.map(call=>call.query.page))).toEqual([1,2,3]);
});

test('import options remain usable and remembered after closing the popup',async({page})=>{
  await search(page);await page.locator('#import-fields-trigger').click();
  const popup=page.locator('#import-fields-popover');await expect(popup.locator('#import-vinyl-disc')).toBeVisible();
  expect(await page.locator('.release-toolbar #search-per-page').count()).toBe(0);
  await popup.locator('#import-vinyl-disc').click();
  await popup.locator('#search-per-page').selectOption('10',{force:true});
  await expect(page.locator('.release-card')).toHaveCount(10);
  await page.keyboard.press('Escape');await expect(popup).toBeHidden();
  await page.locator('#import-fields-trigger').click();
  await expect(popup.locator('#import-vinyl-disc')).toHaveAttribute('aria-checked','true');
  await expect(popup.locator('#search-per-page')).toHaveValue('10');
  const more=popup.locator('.import-fields-more');
  expect(await more.evaluate(node=>getComputedStyle(node).borderTopWidth)).toBe('0px');
  expect(await more.evaluate(node=>getComputedStyle(node).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
  const all=popup.getByRole('button',{name:'Select all',exact:true});
  expect(await all.evaluate(node=>getComputedStyle(node).borderRadius)).toBe('999px');
  await all.click();await expect(popup.getByRole('checkbox',{name:'Title',exact:true})).toHaveAttribute('aria-checked','true');
  await popup.getByRole('button',{name:'Clear all',exact:true}).click();await expect(popup.getByRole('checkbox',{name:'Title',exact:true})).toHaveAttribute('aria-checked','false');
});

for(const theme of ['dark','light'])test(`all four tabs fade overflow while final content stays opaque (${theme})`,async({page},info)=>{
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
  for(const [tab,selector] of [['online','.online-scroll'],['editor','.editor-scroll'],['filename','#subtab-filename .from-name-scroll'],['fromtext','#subtab-fromtext .from-name-scroll']]){
    await openTagSource(page, tab);
    const port=page.locator(selector);
    await port.evaluate(node=>{const filler=document.createElement('div');filler.style.cssText='height:1800px;flex:none';filler.textContent='Scrolling content';node.append(filler);node.scrollTop=0;});
    await expect(port).toHaveClass(/can-scroll-down/);
    expect(await port.evaluate(node=>getComputedStyle(node).maskImage)).toContain('linear-gradient');
    await port.evaluate(node=>{node.scrollTop=node.scrollHeight;});await expect(port).not.toHaveClass(/can-scroll-down/);
    if(tab==='editor') {
      await expect(port).toHaveClass(/can-scroll-up/);
      expect(await port.evaluate(node=>getComputedStyle(node).getPropertyValue('--editor-fade-bottom'))).toBe('0px');
    } else expect(await port.evaluate(node=>getComputedStyle(node).maskImage)).toBe('none');
    if(tab!=='online'){
      await port.evaluate(node=>{node.scrollTop=0;});await expect(port).toHaveClass(/can-scroll-down/);
      const input=port.locator('input:visible:not(:disabled),textarea:visible:not(:disabled)').first();if(await input.count())await input.focus();
      await expect.poll(()=>port.evaluate(node=>getComputedStyle(node).maskImage)).toContain('linear-gradient');
    }
  }
  await openTagSource(page, 'online');await search(page);
  await page.locator('.mode-col').screenshot({path:info.outputPath(`online-${theme}.png`)});
});

test('duplicate entries within one provider page are displayed only once',async({page})=>{
  await page.evaluate(()=>window.pagingMode='duplicates');
  await page.locator('#discogs-query').fill('Original search');await page.locator('#discogs-query').press('Enter');
  await expect(page.locator('.release-card')).toHaveCount(4);
  await expect(page.locator('#discogs-search')).not.toHaveClass(/busy/);
  expect(await page.locator('.release-card').evaluateAll(nodes=>new Set(nodes.map(node=>node.dataset.id)).size)).toBe(4);
});

test('hover clear cancels a slow search and removes results while keeping staged edits',async({page})=>{
  await page.locator('#root-display').click();await page.locator('#root').fill('/music');await page.locator('#root').press('Enter');
  await page.locator('#tracks-body tr[data-path]').first().click();await openTagSource(page, 'editor');
  await page.locator('#editor-field-title').fill('Keep staged title');await expect(page.locator('#staged-edits')).toBeVisible();
  await openTagSource(page, 'online');await search(page);
  await page.evaluate(()=>window.pagingMode='hold');await bottom(page);await expect.poll(()=>page.evaluate(()=>window.pendingPages.length)).toBe(1);
  await page.mouse.move(0,0);await expect(page.locator('#discogs-clear')).toHaveCSS('opacity','0');
  await page.locator('#query-box').hover();await expect(page.locator('#discogs-clear')).toHaveCSS('opacity','1');
  await page.locator('#discogs-clear').click();await expect(page.locator('#discogs-query')).toHaveValue('');
  await page.evaluate(()=>window.pendingPages.shift()());await settle(page);
  await expect(page.locator('.release-card')).toHaveCount(0);await expect(page.locator('#release-toolbar')).toBeHidden();
  await expect(page.locator('#staged-edits')).toBeVisible();await expect(page.locator('#diff-apply')).toBeEnabled();
  expect(await page.evaluate(()=>window.pageCalls.length)).toBe(2);
});

test('the clear search action stays keyboard reachable without hover',async({page})=>{
  await page.locator('#discogs-query').fill('Unsubmitted search');await page.mouse.move(0,0);
  await page.locator('#discogs-query').focus();await page.keyboard.press('Tab');
  await expect(page.locator('#discogs-clear')).toBeFocused();await expect(page.locator('#discogs-clear')).toHaveCSS('opacity','1');
  await page.keyboard.press('Enter');await expect(page.locator('#discogs-query')).toHaveValue('');await expect(page.locator('#discogs-query')).toBeFocused();
});


test('Clear and a new provider reject a late card expansion with the same release ID',async({page})=>{
  await page.evaluate(()=>{
    window.detailMode='prefetch';window.pendingDetails=[];
    window.detailHarness=args=>{
      const release={id:args.releaseId,artist:'Various',title:window.detailMode==='current'?'Current provider':'Stale provider',year:2004,genres:[],styles:[],labels:[],images:[],cover_image_url:null,tracks:[{position:'1',artist:'Artist',title:window.detailMode==='current'?'Current track':'Stale track',duration_secs:60}]};
      if(window.detailMode==='prefetch')return Promise.reject(new Error('No background details'));
      if(window.detailMode==='hold')return new Promise(resolve=>window.pendingDetails.push(()=>resolve(release)));
      return Promise.resolve(release);
    };
  });
  await search(page);await page.evaluate(()=>window.detailMode='hold');
  await page.locator('.release-card').first().locator('.release-head').click();
  await expect.poll(()=>page.evaluate(()=>window.pendingDetails.length)).toBe(1);
  await page.locator('#query-box').hover();await page.locator('#discogs-clear').click();await expect(page.locator('.release-card')).toHaveCount(0);
  await page.evaluate(()=>{window.detailMode='current';document.getElementById('online-source').value='musicbrainz';});
  await search(page);await page.evaluate(()=>window.pendingDetails.shift()());await settle(page);
  await page.locator('.release-card').first().locator('.release-head').click();
  await expect(page.locator('.release-card').first().locator('.tk-t')).toHaveText('Current track');
  await expect(page.locator('.release-card')).toHaveCount(5);
});

test('paging shows an animated inline loader and appends only new card animations', async ({ page }, info) => {
  await page.emulateMedia({reducedMotion:'no-preference'});
  await search(page); await page.locator('.release-card').first().evaluate(node => window.originalCard = node);
  await expect(page.locator('#load-more')).toBeVisible();
  await page.evaluate(() => window.pagingMode = 'hold'); await bottom(page);
  await expect(page.locator('#release-loading')).toBeVisible();
  await expect(page.locator('#release-loading')).toHaveText('Loading more releases…');
  expect(await page.locator('.release-loading-spinner').evaluate(node => node.getAnimations().length)).toBe(1);
  const scroll = await page.locator('.online-scroll').evaluate(node => node.scrollTop);
  await page.locator('.online-scroll').screenshot({path:info.outputPath('online-loading.png')});
  // Record animations as they are authored; avoid timing-dependent sampling.
  await page.evaluate(() => {
    window.pageAnimations = [];
    const animate = Element.prototype.animate;
    Element.prototype.animate = function(frames, options) {
      if (this.classList.contains('release-card')) window.pageAnimations.push(this.dataset.id);
      return animate.call(this, frames, options);
    };
    window.pendingPages.shift()();
  });
  await expect(page.locator('.release-card')).toHaveCount(10);
  expect(await page.evaluate(() => window.pageAnimations.length)).toBe(5);
  expect(await page.evaluate(() => window.pageAnimations.includes(window.originalCard.dataset.id))).toBe(false);
  expect(await page.evaluate(() => window.originalCard === document.querySelector('.release-card'))).toBe(true);
  expect(await page.locator('.online-scroll').evaluate(node => node.scrollTop)).toBe(scroll);
  await expect(page.locator('#release-loading')).toBeHidden();
});

test('detail prefetch shares the pending thumbnail request and retains its image node', async ({page}) => {
  await page.evaluate(() => {
    window.pagingHarness = () => Promise.resolve([{
      id:'600001', artist:'Artist', title:'Thumbnail fixture', year:2004, format:'CD',
      thumb_url:'https://fixture.invalid/thumb.png',
    }]);
    window.detailHarness = () => Promise.resolve({
      id:'600001', artist:'Artist', title:'Thumbnail fixture', year:2004, format:'CD',
      labels:[], images:[], tracks:[],
    });
    window.thumbnailCalls = [];
    window.imageHarness = args => new Promise(resolve => {
      window.thumbnailCalls.push(args.url);
      window.finishThumbnail = () => resolve({mime:'image/png',data_base64:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jUa0AAAAASUVORK5CYII='});
    });
  });
  await page.locator('#discogs-query').fill('Thumbnail fixture');
  await page.locator('#discogs-query').press('Enter');
  await expect(page.locator('.release-card')).toHaveCount(1);
  await expect(page.locator('#discogs-search')).not.toHaveClass(/busy/);
  expect(await page.evaluate(() => window.thumbnailCalls)).toEqual(['https://fixture.invalid/thumb.png']);
  await page.evaluate(() => {
    window.thumbnailMounts = 0;
    const cover = document.querySelector('.release-cover');
    const observer = new MutationObserver(records => {
      for (const record of records) {
        for (const node of record.addedNodes) if (node.nodeName === 'IMG') window.thumbnailMounts++;
      }
    });
    observer.observe(cover, {childList:true});
    window.finishThumbnail();
  });
  await expect(page.locator('.release-cover img')).toHaveCount(1);
  await settle(page);
  expect(await page.evaluate(() => window.thumbnailMounts)).toBe(1);
});
