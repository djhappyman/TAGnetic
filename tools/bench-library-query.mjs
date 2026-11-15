#!/usr/bin/env node
// Matched pure-engine workloads. Real Tauri/paint latency is measured separately.
import {execFileSync,spawn} from 'node:child_process';
import {performance} from 'node:perf_hooks';
import {writeFile,mkdir} from 'node:fs/promises';
import os from 'node:os';
import {fixture, workloads, oldModel, nativeModel} from './library-query-fixtures.mjs';
const baselineRef='213f06c';
const source=execFileSync('git',['show',`${baselineRef}:app/ui/js/filter-query.js`],{encoding:'utf8'});
const {compileQuery}=await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const binary=process.env.TAGREX_QUERY_BENCH||'/mnt/Games/Development/tagrex/build-cache/personal-debug/debug/examples/query_bench';
const sizes=process.env.TAGREX_QUERY_SIZES?.split(',').map(Number)||[10000,50000,100000];
const iterations=40;
const checksum=indices=>indices.reduce((sum,i)=>sum+i*31+7,0);
function baseline(tracks,w){
 const cache=new WeakMap();
 const fieldValue=(t,k)=>k==='file'?t.path.split('/').pop():t.tags[k]||'';
 const context={fields:['artist','album','title','genre','year','file'],fieldValue,coverLabel:s=>({embedded:'Embedded artwork',missing:'No embedded artwork',unknown:'Artwork status unavailable'})[s],searchText(t,folded){let c=cache.get(t);if(!c){const text=[fieldValue(t,'file'),...Object.values(t.tags)].join(' ');c={text,lower:text.toLowerCase()};cache.set(t,c);}return folded?c.lower:c.text;}};
 const compiled=compileQuery(oldModel(w),context);if(compiled.error)throw Error(compiled.error);
 function run(){const indices=[];for(let i=0;i<tracks.length;i++)if(compiled.match(tracks[i]))indices.push(i);return indices;}
 let start=performance.now(),indices=run();const coldMs=performance.now()-start;
 for(let i=0;i<5;i++)run();
 const times=[];for(let i=0;i<iterations;i++){start=performance.now();indices=run();times.push(performance.now()-start);}
 times.sort((a,b)=>a-b);
 return {coldMs,medianMs:times[Math.floor(times.length/2)],p95Ms:times[Math.ceil((times.length-1)*.95)],matched:indices.length,checksum:checksum(indices)};
}
async function rust(input){return new Promise((resolve,reject)=>{const p=spawn(binary,[],{stdio:['pipe','pipe','pipe']});let out='',err='';p.stdout.on('data',b=>out+=b);p.stderr.on('data',b=>err+=b);p.on('error',reject);p.on('close',code=>{if(code)reject(Error(err||`exit ${code}`));else try{resolve(JSON.parse(out));}catch(e){reject(e);}});p.stdin.end(JSON.stringify(input));});}
const results=[];
for(const size of sizes){
 const tracks=fixture(size);const js=workloads.map(w=>baseline(tracks,w));
 const native=await rust({tracks,queries:workloads.map(nativeModel),iterations,suggest:{text:'Hard',field:'genre'}});
 const queries=workloads.map((w,i)=>{
  const r=native.queries[i];if(js[i].matched!==r.matched||js[i].checksum!==r.checksum)throw Error(`${size}/${w.name}: result mismatch ${JSON.stringify({js:js[i],rust:r})}`);
  return {name:w.name,baseline:js[i],rust:r,speedup:js[i].medianMs/r.medianMs};
 });
 results.push({rows:size,indexMs:native.indexMs,profile:native.profile,commandsOptLevel:2,queries,suggestion:native.suggestion});
 console.log(JSON.stringify({rows:size,indexMs:native.indexMs,queries:queries.map(q=>({name:q.name,baselineMs:q.baseline.medianMs,rustMs:q.rust.medianMs,speedup:q.speedup}))}));
}
const report={baselineRef,date:new Date().toISOString(),node:process.version,cpu:os.cpus()[0].model,platform:`${process.platform}/${process.arch}`,iterations,binary,method:'Identical synthetic DTOs and equivalent queries; warm medians/p95 after index construction. Baseline is checkpoint JS evaluator, native is isolated Rust example. Development profile with tagrex-commands opt-level=2. Excludes IPC, disk scan, audio analysis and table rendering. Index construction reported separately. All counts and deterministic index checksums must match.',results};
await mkdir('docs/performance',{recursive:true});await writeFile('docs/performance/library-query-results.json',JSON.stringify(report,null,2)+'\n');
