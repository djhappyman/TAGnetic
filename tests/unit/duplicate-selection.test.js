import test from 'node:test';
import assert from 'node:assert/strict';
import { duplicateGroupTitle, duplicateSelectionDetail } from '../../app/ui/js/duplicate-selection.js';

test('duplicate detail separates exact saved tag differences without changing DTOs', () => {
  const tracks = [
    {path:'/a.mp3',format:'MP3',duration:210,has_cover:true,tags:{title:'Recording',artist:'Hixxy',album:'A',comment:'',genre:'Hardcore'}},
    {path:'/b.flac',format:'FLAC',duration:211,has_cover:false,tags:{title:'Recording',artist:'Hixxy',album:'B',genre:'hardcore',year:'1996'}},
  ];
  const before = JSON.stringify(tracks), lookup = path => tracks.find(track => track.path === path);
  const group = {id:'one',label:'Fallback',reason:'Verified file contents',paths:tracks.map(track => track.path)};
  const detail = duplicateSelectionDetail(group,lookup);
  assert.deepEqual(detail.sharedTags,{artist:'Hixxy',title:'Recording'});
  assert.deepEqual(detail.differingTags,['album','genre','year']);
  assert.deepEqual(Object.keys(detail),['id','title','reason','tracks','sharedTags','differingTags']);
  assert.equal(detail.tracks[0],tracks[0]);assert.equal(detail.tracks[1].has_cover,false);
  assert.equal(JSON.stringify(tracks),before);
});

test('missing and empty tags compare equally and prototype-like tag keys stay data', () => {
  const tags = JSON.parse('{"__proto__":"saved","comment":""}');
  const tracks = [{path:'a',tags},{path:'b',tags:JSON.parse('{"__proto__":"saved"}')}];
  const detail = duplicateSelectionDetail({id:'two',reason:'bytes',paths:['a','b']},path=>tracks.find(t=>t.path===path));
  assert.deepEqual(Object.entries(detail.sharedTags),[['__proto__','saved']]);
  assert.deepEqual(detail.differingTags,[]);assert.equal(Object.getPrototypeOf(detail.sharedTags),Object.prototype);
});

test('title falls back to filename and a missing group or library resolves to null', () => {
  const group = {id:'one',label:'Fallback',paths:['C:\\music\\copy.flac']};
  assert.equal(duplicateGroupTitle(group,path=>({path,tags:{title:'   '}})),'copy.flac');
  assert.equal(duplicateSelectionDetail(null,()=>null),null);
  assert.equal(duplicateSelectionDetail(group,()=>null),null);
});
