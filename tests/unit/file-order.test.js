import {test} from 'node:test';
import assert from 'node:assert/strict';
import {captureFileOrder, restoreFileOrder} from '../../app/ui/js/file-order.js';
const row = path => ({path});

test('manual order survives reread while new files keep their sorted slots', () => {
  const a=row('a'), b=row('b'), c=row('c'), fresh=row('new');
  assert.deepEqual(restoreFileOrder([a,fresh,b,c], ['c','a','b']), [c,fresh,a,b]);
});
test('renamed and removed files retain only the surviving alignment', () => {
  const order=captureFileOrder([row('a'),row('b'),row('gone')],true,new Map([['a','renamed']]));
  assert.deepEqual(restoreFileOrder([row('b'),row('renamed')],order).map(t=>t.path), ['renamed','b']);
});
test('ordinary sorted lists do not acquire manual ordering', () => {
  const tracks=[row('a'),row('b')];
  assert.equal(captureFileOrder(tracks,false),null);
  assert.equal(restoreFileOrder(tracks,null),tracks);
});
test('large aligned libraries avoid argument limits and preserve row objects', () => {
  const tracks=Array.from({length:150000},(_,i)=>row(String(i)));
  const order=tracks.map(t=>t.path).reverse();
  const restored=restoreFileOrder(tracks,order);
  assert.equal(restored[0],tracks.at(-1));
  assert.equal(restored.at(-1),tracks[0]);
  assert.equal(new Set(restored).size,tracks.length);
});
