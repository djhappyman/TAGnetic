import test from 'node:test';
import assert from 'node:assert/strict';
import { patternFields } from '../../app/ui/js/pattern-input.js';
import { patternParts } from '../../app/ui/js/field-pattern.js';
const entries = [
  {name:'artist',token:'%artist%',render:true,extract:true},
  {name:'filename',token:'%filename%',render:true,extract:false},
  {name:'skip',token:'%skip%',render:false,extract:true},
  {name:'upper',token:'$upper()',render:true,extract:false},
];
test('pattern suggestions respect rendering/extraction and leave functions as native expressions', () => {
  assert.deepEqual(patternFields(entries,'render',x=>x).map(x=>x.key),['artist','filename']);
  assert.deepEqual(patternFields(entries,'extract',x=>x).map(x=>x.key),['artist','skip']);
  assert.deepEqual(patternFields(entries,'text',x=>x).map(x=>x.key),['artist','skip']);
});
test('width chips preserve their exact spelling, adjacent tokens and custom namespaces', () => {
  const fields = [{key:'track',label:'Track'},{key:'custom:VENUE',label:'Venue'}];
  const raw = '[%track:03%]%custom:VENUE:12% - $upper(%unknown%)';
  const parts = patternParts(raw,fields);
  assert.equal(parts.map(x=>x.text).join(''),raw);
  assert.deepEqual(parts.filter(x=>x.field).map(x=>[x.field.key,x.width]),[['track','03'],['custom:VENUE','12']]);
});
