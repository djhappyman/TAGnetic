// Run through the existing development WebView. Synthetic indices never touch
// the live library, selection, pending edits, audio files or Undo journal.
import {fixture, workloads, oldModel, nativeModel} from './library-query-fixtures.mjs';

export async function run({sizes=[10000,50000,100000],iterations=40}={}) {
  const {compileQuery}=await import('/js/filter-query.js');
  const {invoke}=window.__TAURI__.core;
  const state=window.tagrex;
  const capture=async()=>({...state.querySession,paths:state.tracks.map(t=>t.path).sort(),selection:[...state.selection],historyIds:(await invoke('history')).map(x=>x.id)});
  const before=await capture();
  const checksum=indices=>indices.reduce((sum,i)=>sum+i*31+7,0);
  const stats=times=>{const sorted=[...times].sort((a,b)=>a-b);return {medianMs:sorted[Math.floor(sorted.length/2)],p95Ms:sorted[Math.ceil((sorted.length-1)*.95)],maxMs:sorted.at(-1)};};
  const pause=()=>new Promise(resolve=>setTimeout(resolve,8));
  async function timed(operation,count){
    const times=[],gaps=[];let result,last=performance.now();
    const timer=setInterval(()=>{const now=performance.now();gaps.push(now-last);last=now;},4);
    try {for(let i=0;i<count;i++){await pause();const started=performance.now();result=await operation();times.push(performance.now()-started);}await pause();}
    finally{clearInterval(timer);}
    return {...stats(times),heartbeatMaxGapMs:Math.max(...gaps,0),heartbeatOver50ms:gaps.filter(x=>x>50).length,result};
  }
  const results=[];
  for(const rows of sizes){
    const tracks=fixture(rows),cache=new WeakMap();
    const fieldValue=(t,k)=>k==='file'?t.path.split('/').pop():t.tags[k]||'';
    const context={fields:['artist','album','title','genre','year','file'],fieldValue,coverLabel:s=>({embedded:'Embedded artwork',missing:'No embedded artwork',unknown:'Artwork status unavailable'})[s],searchText(t,folded){let c=cache.get(t);if(!c){const text=[fieldValue(t,'file'),...Object.values(t.tags)].join(' ');c={text,lower:text.toLowerCase()};cache.set(t,c);}return folded?c.lower:c.text;}};
    const started=performance.now(),index=await invoke('library_benchmark_create',{tracks});
    const setupRoundTripMs=performance.now()-started,queries=[];
    try {
      for(const workload of workloads){
        const compiled=compileQuery(oldModel(workload),context);if(compiled.error)throw Error(compiled.error);
        const baselineRun=()=>{const indices=[];for(let i=0;i<tracks.length;i++)if(compiled.match(tracks[i]))indices.push(i);return indices;};
        const query=nativeModel(workload);let requestId=0;
        const nativeRun=()=>invoke('library_benchmark_query',{benchmarkId:index.benchmarkId,request:{requestId:++requestId,snapshotId:index.snapshotId,query}});
        for(let i=0;i<5;i++){baselineRun();await nativeRun();}
        const baseline=await timed(baselineRun,iterations),native=await timed(nativeRun,iterations);
        if(baseline.result.length!==native.result.matched||checksum(baseline.result)!==checksum(native.result.indices))throw Error(`${rows}/${workload.name}: results differ`);
        queries.push({name:workload.name,matched:native.result.matched,baseline:{...baseline,result:undefined},native:{...native,result:undefined},nativeEngineMs:native.result.elapsedMs});
      }
      // Empty query measures the worst result payload: every matching index.
      const query={...nativeModel(workloads[0]),text:''};let requestId=0;
      const all=await timed(()=>invoke('library_benchmark_query',{benchmarkId:index.benchmarkId,request:{requestId:++requestId,snapshotId:index.snapshotId,query}}),10);
      results.push({rows,indexMs:index.indexMs,setupRoundTripMs,queries,allRowsPayload:{...all,result:undefined,matched:all.result.matched}});
    } finally {await invoke('library_benchmark_dispose',{benchmarkId:index.benchmarkId});}
  }
  const after=await capture();if(JSON.stringify(before)!==JSON.stringify(after))throw Error('Live library state changed during isolated measurements.');
  return {date:new Date().toISOString(),userAgent:navigator.userAgent,iterations,method:'Same workloads as pure benchmark. Baseline evaluator and full Rust IPC round trips measured in actual development WebKit. Warm runs, 4ms heartbeat, 8ms yield between samples. No table rendering or audio scan. Synthetic setup includes fixture serialization; no real App replacement.',statePreserved:{tracks:after.paths.length,selected:after.selection.length,edits:after.edits,preview:after.preview,undoBatches:after.historyIds.length},results};
}
