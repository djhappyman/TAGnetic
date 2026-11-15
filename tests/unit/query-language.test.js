import test from 'node:test';
import assert from 'node:assert/strict';
import {interpretQueryText} from '../../app/ui/js/query-language.js';
import {normalizeRoot} from '../../app/ui/js/query-model.js';
test('common phrases propose typed rules while retaining unrecognized text',()=>{
 assert.equal(interpretQueryText('artist hixxy').nodes[0].operator,'contains');
 assert.deepEqual(interpretQueryText('genre hardcore or jungle').nodes[0].values,['hardcore','jungle']);
 assert.deepEqual(interpretQueryText('year between 1995 and 1998').nodes[0].values,['1995','1998']);
 assert.equal(interpretQueryText('bitrate below 192 kbps').nodes[0].operator,'lt');
 const mixed=interpretQueryText('hardcore from 1995 missing artwork');
 assert.equal(mixed.remaining,'hardcore');assert.equal(mixed.nodes.length,2);
 assert.equal(interpretQueryText('ordinary song title').nodes.length,0);
});
test('proposals remain explicit, and malformed numeric ranges cannot become valid queries',()=>{
 const result=interpretQueryText('year between 2005 and 1995');
 assert.throws(()=>normalizeRoot({kind:'group',id:'root',mode:'all',children:result.nodes}));
});
test('quoted titles preserve words that otherwise look like filter instructions',()=>{
 for(const title of ['Live or Die','Missing Artwork','From 1996']){
  const parsed=interpretQueryText(`title "${title}"`);
  assert.equal(parsed.nodes.length,1);assert.equal(parsed.nodes[0].field,'title');assert.deepEqual(parsed.nodes[0].values,[title]);
 }
 assert.equal(interpretQueryText('title "Unfinished').nodes.length,0);
 assert.deepEqual(interpretQueryText('artist "Hixxy \\"live\\""').nodes[0].values,['Hixxy "live"']);
});
