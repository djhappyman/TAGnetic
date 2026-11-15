#!/usr/bin/env node
import {spawn} from 'node:child_process';
import {writeFile} from 'node:fs/promises';
import {fixture,nativeModel} from './library-query-fixtures.mjs';

const binary=process.env.TAGREX_QUERY_BENCH||'/mnt/Games/Development/tagrex/build-cache/personal-debug/debug/examples/query_bench';
const results=[];
function execute(input){return new Promise((resolve,reject)=>{
 const process=spawn(binary,[],{stdio:['pipe','pipe','pipe']});let out='',err='';
 process.stdout.on('data',b=>out+=b);process.stderr.on('data',b=>err+=b);process.on('error',reject);
 process.on('close',code=>{if(code)reject(Error(err));else try{resolve(JSON.parse(out));}catch(error){reject(error);}});
 process.stdin.end(JSON.stringify(input));
});}
for(const rows of [10000,50000,100000]){
 const tracks=fixture(rows);
 for(const text of ['Artist 17','Hardcore','17']){
  // An empty query leaves every suggestion facet cold on this fresh index.
  const result=await execute({tracks,queries:[nativeModel({old:{}})],iterations:40,suggest:{text,strategy:'fields'}});
  const seen=new Set();
  for(const rule of result.suggestion.suggestions){
   if(seen.has(rule.field))throw Error('Duplicate field rule');seen.add(rule.field);
   const count=tracks.filter(track=>{const raw=(rule.field==='format'?track.format:track.tags[rule.field]||'').toLowerCase(),value=rule.value.toLowerCase();return rule.operator==='contains'?raw.includes(value):raw===value;}).length;
   if(count!==rule.count)throw Error(`${rows}/${text}/${rule.field}: count mismatch`);
  }
  results.push({rows,text,...result.suggestion});
  console.log(JSON.stringify({rows,text,coldMs:result.suggestion.coldMs,medianMs:result.suggestion.medianMs,p95Ms:result.suggestion.p95Ms,returned:result.suggestion.returned}));
 }
}
await writeFile('docs/performance/library-suggestion-results.json',JSON.stringify({date:new Date().toISOString(),iterations:40,method:'Native development commands opt-level2. Fresh immutable index per term, empty query before first suggestion; cold includes lazy facet construction. Warm samples reuse facets. No IPC or table rendering. One rule per field and all distinct-row counts validated against fixture rows.',results},null,2)+'\n');
