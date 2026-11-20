import test from 'node:test';
import assert from 'node:assert/strict';
import {TEXT_FUNCTIONS,readFunction,functionParts,buildFunction,encodeArgument} from '../../app/ui/js/mock-function-pattern.js';
import {patternParts} from '../../app/ui/js/field-pattern.js';
const fields=[{key:'artist',label:'Artist'},{key:'title',label:'Title'}];
test('whole functions retain nested calls, quoted separators and exact raw spelling',()=>{
  for(const value of ["$replace($upper(%artist%),',','(')","$UPPER(%artist%)","$replace(%title%,o''clock,)","$replace([%artist%,%title%],a,b)"]){
    const call=readFunction(value);assert.equal(call.raw,value);assert.equal(call.end,value.length);
    const parts=functionParts(value,fields,patternParts);assert.equal(parts.length,1);assert.equal(parts[0].text,value);
  }
  assert.deepEqual(readFunction('$replace(a,b,)').args,['a','b','']);
  assert.equal(readFunction('$upper(%artist%'),null);
});
test('only the agreed text functions collapse and extraction has no function chips',()=>{
  assert.deepEqual(TEXT_FUNCTIONS.map(x=>x.key),['upper','lower','trim','strpos','replace']);
  assert.equal(functionParts('$if2(%artist%,%title%)',fields,patternParts).some(x=>x.fn),false);
  assert.equal(functionParts('$upper(%artist%)',fields,patternParts,false).some(x=>x.fn),false);
  const raw='before $upper(%artist%) after %title%';
  assert.equal(functionParts(raw,fields,patternParts).map(x=>x.text).join(''),raw);
});
test('argument serialization preserves expressions and quotes introduced literal separators',()=>{
  assert.equal(buildFunction('replace',['%title%',',','_']),"$replace(%title%,',',_)");
  assert.equal(buildFunction('upper',['$trim(%artist%)']),'$upper($trim(%artist%))');
  assert.equal(encodeArgument("o'clock"),"o''clock");
  assert.equal(buildFunction('replace',['%title%','_', '']),'$replace(%title%,_,)');
  assert.equal(encodeArgument("it's John's"),"it''s John''s");
  assert.throws(()=>buildFunction('if2',['a','b']));
});
