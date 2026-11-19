import test from 'node:test';
import assert from 'node:assert/strict';
import { createFieldFormatActions } from '../../app/ui/js/field-format-actions.js';
function harness() {
  const tags = {title:'Hixxy - Toy Town',artist:'Old Artist',album:'Bonkers',comment:'Disk comment'};
  let plan = {changes:[{path:'p',tag_changes:[{field:'comment',old:'Disk comment',new:'Keep this edit'}]}]};
  let revision = 1, selected = true, locked = new Set();
  const calls = {entries:[],writes:[],adopted:0,requests:[]};
  let native = {changes:[{path:'p',tag_changes:[{field:'title',new:'Toy Town'},{field:'artist',new:'Hixxy'},{field:'comment',new:'Keep this edit'}]}],unrendered:[]};
  let invoke = async (_,args) => {calls.requests.push(args);return native;};
  const run = createFieldFormatActions({invoke:(...args)=>invoke(...args),captureSelection:()=>({paths:['p']}),isSelectionCurrent:()=>selected,
    getPlan:()=>plan,inputRevision:()=>revision,workingField:(_,field)=>({value:plan?.changes[0]?.tag_changes.find(edit=>edit.field===field)?.new ?? tags[field] ?? ''}),
    fieldCatalogue:()=>['artist','title','album','comment'].map(key=>({key,label:key[0].toUpperCase()+key.slice(1)})),
    trackAt:()=>({tags}),isFieldLocked:field=>locked.has(field),adoptTagOnlyPreview:()=>calls.adopted++,recordLiveFields:entries=>calls.entries.push(...entries),writeQuickPlan:plan=>calls.writes.push(plan)});
  return {run,calls,setNative:value=>native=value,setInvoke:value=>invoke=value,changeInput:()=>revision++,changeSelection:()=>selected=false,lock:field=>locked.add(field),changePlan:()=>plan={...plan}};
}
const options={kind:'extract',source:'title',pattern:'%artist% - %title%'};
test('Extract stages multiple destination values once and leaves unrelated pending fields alone',async()=>{
  const h=harness();const result=await h.run('field-format-preview',options);
  assert.equal(h.calls.adopted,0);assert.deepEqual(h.calls.entries,[]);
  assert.deepEqual(h.calls.requests[0].operations,[{kind:'extract',source:'%title%',pattern:options.pattern}]);
  await h.run('field-format-stage',{token:result.token});
  assert.deepEqual(h.calls.entries,[{path:'p',field:'artist',value:'Hixxy'},{path:'p',field:'title',value:'Toy Town'}]);
  assert.equal(h.calls.adopted,1);assert.deepEqual(h.calls.writes,[]);
});
for(const change of ['changeInput','changeSelection','changePlan']) test(`stale ${change} invalidates staging`,async()=>{
  const h=harness();const result=await h.run('field-format-preview',options);h[change]();
  await assert.rejects(h.run('field-format-stage',{token:result.token}),/changed/);assert.deepEqual(h.calls.entries,[]);
});
test('locks skip the protected destination and a later lock invalidates the accepted preview',async()=>{
  const h=harness();h.lock('artist');const result=await h.run('field-format-preview',options);
  assert.deepEqual(result.locked,['Artist']);await h.run('field-format-stage',{token:result.token});
  assert.deepEqual(h.calls.entries,[{path:'p',field:'title',value:'Toy Town'}]);
  const x=harness();const preview=await x.run('field-format-preview',options);x.lock('artist');
  await assert.rejects(x.run('field-format-stage',{token:preview.token}),/changed/);
});
test('mismatches preserve all fields and invalid native values cannot stage',async()=>{
  const h=harness();h.setNative({changes:[],unrendered:[{field:'extraction pattern',paths:['p'],files:1}]});
  const result=await h.run('field-format-preview',options);assert.equal(result.unmatched,1);
  await h.run('field-format-stage',{token:result.token});assert.deepEqual(h.calls.entries,[]);
  const x=harness();x.setNative({changes:[{path:'p',tag_changes:[{field:'title',new:'bad',invalid:true}]}]});
  const invalid=await x.run('field-format-preview',options);assert.equal(invalid.invalid,true);
  await assert.rejects(x.run('field-format-stage',{token:invalid.token}),/invalid/);
});
test('cancelled and superseded native replies cannot become accepted previews',async()=>{
  const h=harness();let resolve;h.setInvoke(()=>new Promise(done=>{resolve=done;}));
  const pending=h.run('field-format-preview',options);await h.run('field-format-cancel');
  resolve({changes:[]});await assert.rejects(pending,/changed/);
  await assert.rejects(h.run('field-format-stage',{token:1}),/changed/);
});
test('Write saves only extracted destinations, never inherited stage fields or structural operations',async()=>{
  const h=harness();const result=await h.run('field-format-preview',options);
  await h.run('field-format-write',{token:result.token});
  assert.deepEqual(h.calls.entries,[]);assert.equal(h.calls.writes.length,1);
  assert.deepEqual(h.calls.writes[0].changes[0].tag_changes.map(edit=>edit.field),['title','artist']);
  assert.equal(h.calls.writes[0].changes[0].rename_to,null);
});
