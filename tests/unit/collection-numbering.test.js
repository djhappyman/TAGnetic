import test from 'node:test';
import assert from 'node:assert/strict';
import {createNumberingActions,numberingOptions} from '../../app/ui/js/numbering-actions.js';

function fixture(){
  let grouping='folder',ordered=['one','two'],base=null,revision=0,locked=new Set(),groups=new Map();
  const staged=[],written=[],calls=[];
  let resolve;
  const response={plan:{changes:[]},edits:[{path:'one',field:'disc',value:'2'}],matched:2,skipped:0};
  let delayed=false;
  const run=createNumberingActions({captureSelection:()=>({paths:['one','two']}),isSelectionCurrent:()=>true,
    getPlan:()=>base,inputRevision:()=>revision,read:(_,field)=>({saved:'Saved',value:field==='album'?'Staged album':'B3'}),
    group:path=>groups.get(path)??path,order:()=>ordered,context:()=>grouping,isFieldLocked:field=>locked.has(field),
    invoke:async(command,args)=>{calls.push({command,args});return delayed?new Promise(done=>{resolve=done;}):response;},
    adoptTagOnlyPreview:()=>staged.push('adopt'),recordLiveFields:edits=>staged.push(edits),writeQuickPlan:plan=>written.push(plan)});
  return {run,response,staged,written,calls,setGrouping:value=>{grouping=value;},setOrder:value=>{ordered=value;},
    changeBase:()=>{base={};},changeRevision:()=>{revision++;},lock:field=>locked.add(field),changeGroup:()=>groups.set('one','new group'),delay:()=>{delayed=true;},resolve:()=>resolve(response)};
}

test('active controls serialize without inactive tab values or calculations',()=>{
  assert.deepEqual(numberingOptions({kind:'tracks',start:7,trackTotal:true,resetAlbum:true,discTotal:true}),{kind:'tracks',start:'7',trackTotal:true,restartGroup:false});
  assert.deepEqual(numberingOptions({kind:'disc',start:'002',advanceGroup:true,trackTotal:true}),{kind:'disc',start:'002',discTotal:false,advanceGroup:true,resetAlbum:false});
  assert.throws(()=>numberingOptions({kind:'unknown'}),/Choose/);
});
test('Rust receives ordered selection, group identities and freshest source values',async()=>{
  const f=fixture();f.setOrder(['two','one']);
  await f.run('numbering-preview',{kind:'vinyl',source:'disc',keepPosition:true});
  const {command,args}=f.calls[0];assert.equal(command,'preview_numbering_dialog');
  assert.deepEqual(args.paths,['two','one']);assert.deepEqual(args.groups,['two','one']);
  assert.deepEqual(args.working.changes[0].tag_changes,[{field:'album',old:'Saved',new:'Staged album'},{field:'disc',old:'Saved',new:'B3'}]);
  assert.equal(args.edits,undefined);
});
test('stage publishes all returned assignments, including values unchanged on disk',async()=>{
  const f=fixture(),p=await f.run('numbering-preview',{kind:'disc',start:2});
  await f.run('numbering-stage',{token:p.token});assert.deepEqual(f.staged,['adopt',f.response.edits]);assert.equal(f.written.length,0);
  await assert.rejects(f.run('numbering-stage',{token:p.token}),/changed/);
});
test('Write uses only the Rust operation plan',async()=>{
  const f=fixture(),p=await f.run('numbering-preview',{kind:'disc',start:2});
  await f.run('numbering-write',{token:p.token});assert.deepEqual(f.written,[f.response.plan]);assert.equal(f.staged.length,0);
});
test('group, order, stage, revision and lock changes each invalidate publication',async()=>{
  for(const mutate of [f=>f.setGrouping('album'),f=>f.setOrder(['two','one']),f=>f.changeBase(),f=>f.changeRevision(),f=>f.changeGroup(),f=>f.lock('disc')]){
    const f=fixture(),p=await f.run('numbering-preview',{kind:'disc',start:2});mutate(f);
    await assert.rejects(f.run('numbering-stage',{token:p.token}),/changed/);assert.equal(f.staged.length,0);
  }
});
test('closed and superseded async previews cannot be adopted',async()=>{
  const f=fixture();f.delay();const pending=f.run('numbering-preview',{kind:'disc',start:2});
  await f.run('numbering-cancel');f.resolve();await assert.rejects(pending,/changed/);
  await assert.rejects(f.run('numbering-write',{token:1}),/changed/);
});
test('invalid native plans and required locked totals block the whole action',async()=>{
  const f=fixture();f.response.plan.changes=[{tag_changes:[{invalid:true}]}];
  const p=await f.run('numbering-preview',{kind:'disc',start:2});assert.equal(p.invalid,true);
  await assert.rejects(f.run('numbering-write',{token:p.token}),/not supported/);
  const locked=fixture();locked.lock('disctotal');
  await assert.rejects(locked.run('numbering-preview',{kind:'disc',start:2,discTotal:true}),/Unlock/);assert.equal(locked.calls.length,0);
});
