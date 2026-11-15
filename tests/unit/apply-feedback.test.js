import test from 'node:test';
import assert from 'node:assert/strict';
import {createApplyFeedback} from '../../app/ui/js/apply-feedback.js';
function setup(changes, reducedMotion=false) {
  let id=0;const frames=new Map(),timers=new Map(),events=[];
  const feedback=createApplyFeedback({progress:event=>events.push(event)},changes,{reducedMotion,frame:cb=>{frames.set(++id,cb);return id;},cancelFrame:id=>frames.delete(id),timer:cb=>{timers.set(++id,cb);return id;},clearTimer:id=>timers.delete(id)});
  return {feedback,events,frames,timers,tick:()=>{const callback=frames.values().next().value;assert.ok(callback);callback();}};
}
const rename=i=>({path:`/${i}.mp3`,rename_to:`/new-${i}.mp3`,tag_changes:[]});
test('fast rename feedback remains ordered and drains within eight frames regardless of file count',async()=>{
  for(const size of [1,3,25,10000]){
    const changes=Array.from({length:size},(_,i)=>rename(i)),h=setup(changes);
    for(const c of changes)h.feedback.progress({path:c.path,completed:'transfer'});
    const done=h.feedback.finish();assert.equal(h.events.length,0);
    let frames=0;while(h.frames.size){h.tick();frames++;assert.ok(frames<=8);}
    await done;assert.deepEqual(h.events.map(e=>e.path),changes.map(c=>c.path));assert.equal(h.timers.size,0);
  }
});
test('successful return fills missing completion events and ignores later arrivals',async()=>{
  const h=setup([rename(0),rename(1)]);h.feedback.progress({path:'/0.mp3',completed:'transfer'});
  const done=h.feedback.finish();h.feedback.progress({path:'/1.mp3',completed:'transfer'});
  while(h.frames.size)h.tick();await done;assert.equal(h.events.length,2);
});
test('cancellation drops queued feedback and prevents later repaint',async()=>{
  const h=setup([rename(0)]);h.feedback.progress({path:'/0.mp3',completed:'transfer'});
  h.feedback.cancel();h.feedback.progress({path:'/0.mp3',completed:'transfer'});await h.feedback.finish();
  assert.equal(h.events.length,0);assert.equal(h.frames.size,0);assert.equal(h.timers.size,0);
});
test('reduced motion updates immediately, without presentation timers',async()=>{
  const h=setup([rename(0)],true);h.feedback.progress({path:'/0.mp3',completed:'transfer'});
  assert.equal(h.events.length,1);await h.feedback.finish();assert.equal(h.frames.size,0);assert.equal(h.events.length,1);
});
test('timer fallback drains feedback when animation frames do not fire',async()=>{
  const h=setup([rename(0)]);const done=h.feedback.finish();h.timers.values().next().value();await done;
  assert.equal(h.events.length,1);assert.equal(h.frames.size,0);
});
