import {test,expect} from '@playwright/test';
import {readFile} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
const root=resolve('.test-dist');
test.beforeEach(async({page})=>{
  await page.route('http://tagrex.test/**',async route=>{
    const path=resolve(root,'.'+new URL(route.request().url()).pathname);
    if(!path.startsWith(root+sep))return route.abort();
    try{await route.fulfill({body:await readFile(path),contentType:path.endsWith('.js')?'text/javascript':path.endsWith('.css')?'text/css':path.endsWith('.woff2')?'font/woff2':'text/html'});}catch{await route.fulfill({status:404});}
  });
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(()=>page.evaluate(()=>typeof window.tagrex?.openLibrary)).toBe('function');
  await page.setViewportSize({width:1280,height:600});
  await page.addStyleTag({content:'#tracks tbody tr[data-path] td {height:23.5px!important;padding-block:0!important;} #tracks tbody tr.group-head td {height:25.5px!important;padding-block:0!important;}'});
  await page.evaluate(async()=>{
    await window.tagrex.invoke('open_drop',{paths:Array.from({length:53},(_,i)=>`/music/group-${Math.floor(i/18)}/track-${i}.mp3`)});
    document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();
    await document.fonts.ready;
    await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  });
});

for(const size of [53,3000])test(`virtual scrolling keeps the scroll extent stable with fractional row heights (${size} files)`,async({page})=>{
  if(size>53)await page.evaluate(async size=>{await window.tagrex.invoke('open_drop',{paths:Array.from({length:size},(_,i)=>`/music/group-${Math.floor(i/250)}/track-${i}.mp3`)});document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();},size);
  const sample=await page.evaluate(async()=>{
    const viewport=document.getElementById('files-view'),heights=[];
    for(let i=0;i<30;i++) {
      viewport.scrollTop=Math.round(i*(viewport.scrollHeight-viewport.clientHeight)/29);
      await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
      heights.push(viewport.scrollHeight);
    }
    return{spread:Math.max(...heights)-Math.min(...heights),heights};
  });
  expect(sample.spread,JSON.stringify(sample.heights)).toBeLessThanOrEqual(1);
});

test('loaded album rows remain mounted with zero cell mutations during scrolling',async({page})=>{
  const result=await page.evaluate(async()=>{
    const viewport=document.getElementById('files-view'),body=document.getElementById('tracks-body');
    const original=[...body.querySelectorAll('tr[data-path]')];
    let mutations=0;const observer=new MutationObserver(records=>mutations+=records.length);observer.observe(body,{childList:true,subtree:true,attributes:true,characterData:true});
    for(let i=0;i<20;i++){viewport.scrollTop=(i%2?0:viewport.scrollHeight);await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));}
    observer.disconnect();return{mutations,retained:original.every(row=>row.isConnected)};
  });
  expect(result.mutations).toBe(0);expect(result.retained).toBe(true);
});

test('large-library scroll bursts paint one bounded window per frame',async({page})=>{
  await page.evaluate(async()=>{
    await window.tagrex.invoke('open_drop',{paths:Array.from({length:3000},(_,i)=>`/music/all/track-${i}.mp3`)});
    document.getElementById('root').value='/dropped';await window.tagrex.openLibrary();
  });
  const count=await page.evaluate(async()=>{
    const viewport=document.getElementById('files-view'),create=document.createElement;let rows=0;
    document.createElement=function(tag,...args){if(tag==='tr')rows++;return create.call(this,tag,...args);};
    try{for(let i=1;i<=12;i++){viewport.scrollTop=i*800;viewport.dispatchEvent(new Event('scroll'));}await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));return rows;}finally{document.createElement=create;}
  });
  expect(count).toBeLessThan(100);
});
