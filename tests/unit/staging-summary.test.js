import { test } from 'node:test';
import assert from 'node:assert/strict';
import { recordTransformationChanges, summarizeStaging, createStagingSummary } from '../../app/ui/js/staging-summary.js';
const plan = (...tags) => ({changes:[{path:'/music/a.flac', tag_changes:tags}]});
const tag = (field, old, next) => ({field,old,new:next});

test('final transformed field wins over its manual edit and counts once', () => {
  const result = plan(tag('title','Saved','Manual!'),tag('artist','Old','New'));
  recordTransformationChanges('preview_live_edits', {groups:[{}],edits:[{path:'/music/a.flac',field:'title',value:'Manual'},{path:'/music/a.flac',field:'artist',value:'New'}]}, result);
  assert.deepEqual(summarizeStaging(result),{transformations:1,tags:1});
});
test('enabled transformations that leave the raw value alone do not count', () => {
  const result=plan(tag('title','Saved','Manual'));
  recordTransformationChanges('preview_live_edits',{groups:[{}],edits:[{path:'/music/a.flac',field:'title',value:'Manual'}]},result);
  assert.deepEqual(summarizeStaging(result),{transformations:0,tags:1});
});
test('transformation-only changes count per final field, excluding reverted fields', () => {
  const result=plan(tag('title','Saved','Saved!'),tag('artist','A','A'),tag('comment',null,''));
  recordTransformationChanges('preview_transform_groups',{groups:[{},{}]},result);
  assert.deepEqual(summarizeStaging(result),{transformations:1,tags:0});
});
test('cleanup of an existing plan preserves other edits and earlier transformation attribution', () => {
  const before=plan(tag('title','Saved','Saved!'),tag('artist','Old','New'));
  recordTransformationChanges('preview_transform_groups',{}, {changes:[{path:'/music/a.flac',tag_changes:[before.changes[0].tag_changes[0]]}]});
  const after=plan(tag('title','Saved','Saved!'),tag('artist','Old','New'),tag('album','A','B'));
  recordTransformationChanges('preview_transform_over_plan',{plan:before},after);
  assert.deepEqual(summarizeStaging(after),{transformations:2,tags:1});
});
test('native derived fields identify transformed values without counting artwork or rename operations', () => {
  const result=plan(tag('title','Saved','Saved!'),tag('artist','Old','New'));
  result.changes[0].cover_change={};result.changes[0].rename_to='/music/new.flac';
  assert.deepEqual(summarizeStaging(result,(path,field)=>path==='/music/a.flac'&&field==='title'),{transformations:1,tags:1});
});

test('native summary patches changed rows without walking the remaining stage', () => {
  const summarize=createStagingSummary();
  const result=plan(tag('title','Saved','Manual'));
  assert.deepEqual(summarize(result),{transformations:0,tags:1});
  const change={path:'/music/a.flac',tag_changes:[tag('title','Saved','Manual!'),tag('artist','Old','New')]};
  result.changes=new Proxy([change],{get(target,property){if(property===Symbol.iterator)throw Error('Full stage traversal');return Reflect.get(target,property);}});
  const delta={nativeDelta:true,changedKeys:[{path:change.path,field:'title'},{path:change.path,field:'artist'}],changeAt:()=>change};
  assert.deepEqual(summarize(result,(_path,field)=>field==='title',delta),{transformations:1,tags:1});
  delta.changeAt=()=>undefined;
  assert.deepEqual(summarize(result,()=>false,delta),{transformations:0,tags:0});
});
