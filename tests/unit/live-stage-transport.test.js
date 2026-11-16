import test from 'node:test';
import assert from 'node:assert/strict';
import { createLiveStageTransport } from '../../app/ui/js/live-stage-transport.js';

const snapshot = () => ({revision:1,human_revision:0,plan:{changes:[{path:'a',tag_changes:[{field:'title',old:'Saved',new:'Agent'}],cover_change:{old:[],new:[{data_base64:'keep-by-reference'}]}}]},human_plan:null});
test('scalar delta retains plan, unaffected fields and artwork without snapshot transfer',async()=>{
  const initial=snapshot();
  const transport=createLiveStageTransport(async(command,args)=>{
    if(command==='mcp_shared_stage_delta') return {session:'s',revision:1,resync:true,snapshot:initial};
    assert.equal(command,'mcp_publish_shared_intents');
    assert.deepEqual(args.intents,[{path:'a',field:'title',value:'Human'}]);
    return {accepted:true,delta:{session:'s',revision:2,human_revision:2,changes:[{path:'a',field:'title',tag:{old:'Saved',new:'Human'}}],removed:[],removed_paths:[]}};
  });
  const first=await transport.read();
  const artwork=first.plan.changes[0].cover_change;
  const next=await transport.publish([{path:'a',field:'title',value:'Human'}],1);
  assert.equal(next.plan,first.plan);
  assert.equal(next.plan.changes[0].cover_change,artwork);
  assert.equal(next.plan.changes[0].tag_changes[0].old,'Saved');
  assert.equal(next.plan.changes[0].tag_changes[0].new,'Human');
  assert.deepEqual(next.changedKeys,[{path:'a',field:'title'}]);
});
test('delta removal and changed native session reconcile authoritative state',async()=>{
  let step=0;
  const transport=createLiveStageTransport(async()=>[
    {session:'s',revision:1,resync:true,snapshot:snapshot()},
    {session:'s',revision:2,changes:[],removed:[{path:'a',field:'title'}],removed_paths:['a']},
    {session:'new',revision:0,resync:true,snapshot:{revision:0,plan:null}},
  ][step++]);
  await transport.read();
  assert.equal((await transport.read()).plan,null);
  assert.equal((await transport.read()).revision,0);
});

for (const size of [20000,50000,100000]) test(`scalar acknowledgement retains ${size} rows without array-wide work`,async()=>{
  const changes=Array.from({length:size},(_,index)=>({path:String(index),tag_changes:[{field:'title',old:'Saved',new:'Agent'}]}));
  let reads=0;
  const transport=createLiveStageTransport(async command=>{
    if(command==='mcp_shared_stage_delta') {reads++;return {session:'s',revision:1,resync:true,snapshot:{revision:1,plan:{changes}}};}
    return {accepted:true,delta:{session:'s',revision:2,changes:[{path:'7',field:'title',tag:{old:'Saved',new:'Human'}}]}};
  });
  const first=await transport.read();const rows=first.plan.changes;
  for (const method of ['map','filter','some','forEach']) rows[method]=()=>{throw new Error(`scalar ack traversed ${method}`);};
  const start=performance.now();
  const result=await transport.publish([{path:'7',field:'title',value:'Human'}],1);
  assert.equal(result.plan.changes,rows);
  assert.equal(result.changeAt('7').tag_changes[0].new,'Human');
  assert.equal(reads,1);
  console.log(JSON.stringify({size,scalarDeltaMs:performance.now()-start,unchangedRows:size-1}));
});

test('local raw reversion and invalid drafts remain distinct from accepted values',async()=>{
  const state=await import('../../app/ui/js/state.js');
  const working=await import('../../app/ui/js/working-values.js');
  state.setTracks([{path:'local',tags:{title:'Saved',year:'2000'}}]);
  state.setDiffByPath(new Map([['local',{tag_changes:[{field:'title',old:'Saved',new:'Agent'}]}]]));
  working.presentRawField('local','title','Saved');
  assert.equal(working.workingField('local','title').value,'Saved');
  working.presentRawField('local','year','12',false);
  assert.equal(working.workingField('local','year').invalid,true);
  assert.equal(working.workingField('local','year').value,'12');
  working.clearRawFields();
  assert.equal(working.workingField('local','title').value,'Agent');
});

test('a delta beginning beyond the cached revision recovers instead of resurrecting cleared fields',async()=>{
  let read=0,fullReads=0;
  const transport=createLiveStageTransport(async command=>{
    if(command==='mcp_shared_stage_delta')return read++===0
      ? {session:'s',revision:1,resync:true,snapshot:snapshot()}
      : {session:'s',revision:3,from:2,changes:[{path:'b',field:'title',tag:{old:'B',new:'Fresh'}}]};
    assert.equal(command,'mcp_shared_stage');fullReads++;
    return {revision:3,human_revision:3,plan:{changes:[{path:'b',tag_changes:[{field:'title',old:'B',new:'Fresh'}]}]},human_plan:null};
  });
  await transport.read();
  const value=await transport.read();
  assert.deepEqual(value.plan.changes.map(change=>change.path),['b']);
  assert.equal(fullReads,1);
});

test('large native intent sets are published in bounded batches at the accepted revisions',async()=>{
  const sizes=[],revisions=[];let revision=1;
  const transport=createLiveStageTransport(async(command,args)=>{
    if(command==='mcp_shared_stage_delta')return{session:'s',revision,resync:true,snapshot:{revision,plan:null}};
    assert.equal(command,'mcp_publish_shared_intents');
    sizes.push(args.intents.length);revisions.push(args.expectedRevision);
    assert.ok(args.intents.length<=512);
    return{accepted:true,delta:{session:'s',from:revision,revision:++revision,changes:args.intents.map(({path,field,value})=>({path,field,tag:{old:'Saved',new:value}}))}};
  });
  await transport.read();
  const value=await transport.publish(Array.from({length:1030},(_,index)=>({path:String(index),field:'title',value:'Bulk'})),1);
  assert.deepEqual(sizes,[512,512,6]);
  assert.deepEqual(revisions,[1,2,3]);
  assert.equal(value.plan.changes.length,1030);
});

test('explicit recompute sends zero intents and the admitted stage session',async()=>{
  let sent;
  const transport=createLiveStageTransport(async(command,args)=>{
    if(command==='mcp_shared_stage_delta')return{session:'s',revision:1,resync:true,snapshot:{revision:1,plan:null}};
    sent=args;return{accepted:true,delta:{session:'s',revision:2,changes:[]}};
  },{session_check:true,recompute:true});
  await transport.read();
  const next=await transport.publish([],1,{recompute:true,groups:[],paths:['a']});
  assert.deepEqual(sent,{expectedRevision:1,session:'s',intents:[],recompute:true,groups:[],paths:['a']});
  assert.equal(next.revision,2);
});

test('artwork deltas hydrate distinct old/new refs once and preserve invalid state without a snapshot',async()=>{
  let step=0;const fetched=[];
  const cover=asset=>({asset,mime:'image/png',kind:'front',description:'Front',invalid:false});
  const transport=createLiveStageTransport(async(command,args)=>{
    if(command==='mcp_shared_stage_asset'){fetched.push(args.asset);return{...cover(args.asset),data_base64:args.asset};}
    assert.equal(command,'mcp_shared_stage_delta');
    return step++===0?{session:'s',revision:1,resync:true,snapshot:{revision:1,plan:null}}:
      {session:'s',revision:2,changes:[{path:'a',field:'artwork',artwork:{invalid:true,old:[cover('old')],covers:[{...cover('new'),invalid:true},cover('new')]}}]};
  });
  await transport.read();const next=await transport.read();
  assert.deepEqual(fetched,['old','new']);
  assert.equal(next.nativeDelta,true);
  assert.equal(next.plan.changes[0].cover_change.old[0].data_base64,'old');
  assert.equal(next.plan.changes[0].cover_change.new[0].invalid,true);
  assert.equal(next.plan.changes[0].cover_change.invalid,true);
});

test('rename deltas retain move roots and folder companions',async()=>{
  let step=0;
  const transport=createLiveStageTransport(async()=>step++===0?{session:'s',revision:1,resync:true,snapshot:{revision:1,plan:null}}:
    {session:'s',revision:2,changes:[{path:'a',field:'filename',rename:{to:'dest/a',copy:true,creates_folder:true,rename_root:'dest',sidecars:[['a.lrc','dest/a.lrc']],folder_extras:['folder.jpg']}}]});
  await transport.read();const row=(await transport.read()).plan.changes[0];
  assert.equal(row.rename_root,'dest');assert.deepEqual(row.folder_extras,['folder.jpg']);
});

test('removing one block delta leaves other staged fields intact without a full snapshot',async()=>{
  let step=0;
  const initial=snapshot();initial.plan.changes[0].block_changes=[{kind:'ID3',new:{text:'changed lyrics'}},{kind:'APE',new:{text:'keep'}}];
  const transport=createLiveStageTransport(async command=>{
    assert.equal(command,'mcp_shared_stage_delta');
    return step++===0?{session:'s',revision:1,resync:true,snapshot:initial}:
      {session:'s',revision:2,changes:[],removed:[{path:'a',field:'block:ID3'}]};
  });
  await transport.read();const next=await transport.read();
  assert.deepEqual(next.plan.changes[0].block_changes.map(block=>block.kind),['APE']);
  assert.equal(next.plan.changes[0].tag_changes[0].new,'Agent');
});

test('a resync in an early bulk chunk remains a full adoption after later incremental chunks',async()=>{
  let revision=1,count=0;
  const transport=createLiveStageTransport(async(command,args)=>{
    if(command==='mcp_shared_stage_delta')return{session:'s',revision,resync:true,snapshot:{revision,plan:null}};
    revision++;
    if(count++===0)return{accepted:true,delta:{session:'s',revision,resync:true,snapshot:{revision,derived_fields:args.intents.map(({path,field})=>({path,field})),plan:{changes:args.intents.map(entry=>({path:entry.path,tag_changes:[{field:entry.field,old:'Saved',new:entry.value}]}))}}}};
    return{accepted:true,delta:{session:'s',revision,changes:args.intents.map(entry=>({path:entry.path,field:entry.field,derived:true,tag:{old:'Saved',new:entry.value}}))}};
  });
  await transport.read();
  const result=await transport.publish(Array.from({length:513},(_,i)=>({path:String(i),field:'title',value:'Current'})),1);
  assert.equal(result.plan.changes.length,513);assert.equal(result.nativeDelta,false);
  assert.equal(result.derived_fields.length,513);
});

test('bulk incremental adoption aggregates derived ownership across every chunk',async()=>{
  let revision=1;
  const transport=createLiveStageTransport(async(command,args)=>{
    if(command==='mcp_shared_stage_delta')return{session:'s',revision,resync:true,snapshot:{revision,plan:null}};
    return{accepted:true,delta:{session:'s',revision:++revision,changes:args.intents.map(entry=>({...entry,derived:true,tag:{old:'Saved',new:entry.value}}))}};
  });
  await transport.read();
  const result=await transport.publish(Array.from({length:513},(_,i)=>({path:String(i),field:'title',value:'Derived'})),1);
  assert.equal(result.derivedChanges.length,513);assert.ok(result.derivedChanges.every(entry=>entry.derived));
});
