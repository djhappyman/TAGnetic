import test from 'node:test';
import assert from 'node:assert/strict';
import {createDuplicateJobs} from '../../app/ui/js/duplicate-jobs.js';

test('cancelling while native scan startup is pending cancels the eventual job',async()=>{
  let finish;const calls=[],states=[];
  const jobs=createDuplicateJobs((command,args)=>{
    calls.push({command,args});
    return command==='library_duplicates_start'?new Promise(resolve=>finish=resolve):Promise.resolve();
  },state=>states.push(state.status),()=>assert.fail('A cancelled scan cannot complete the view'));
  const pending=jobs.start('likely');await jobs.cancel();
  finish({jobId:42,status:'running'});await pending;
  assert.deepEqual(calls.at(-1),{command:'library_duplicates_cancel',args:{jobId:42}});
  assert.deepEqual(states,['running','cancelled']);
});

test('cancel publishes the native cleared-scope revision and stops polling',async()=>{
  const states=[],calls=[];
  const jobs=createDuplicateJobs(async(command)=>{
    calls.push(command);
    return command==='library_duplicates_start'
      ?{jobId:7,status:'running',groupsRevision:2,groupsCount:3}
      :{jobId:7,status:'cancelled',groupsRevision:3,groupsCount:0,phase:'cancelled'};
  },state=>states.push(state),()=>assert.fail('Cancelled scan cannot complete'));
  await jobs.start('exact');await jobs.cancel();
  assert.equal(states.at(-1).groupsRevision,3);assert.equal(states.at(-1).groupsCount,0);
  assert.deepEqual(calls,['library_duplicates_start','library_duplicates_cancel']);
});
