import { test } from 'node:test';
import assert from 'node:assert/strict';
import { movableDirections, shiftSelectedOrder } from '../../app/ui/js/row-order.js';
const tracks = 'abcdef'.split('').map(path => ({path}));
const order = result => result?.map(t => t.path).join('');
test('moves disjoint selections down one slot without reversing selected blocks', () => {
  assert.equal(order(shiftSelectedOrder(tracks, ['abcdef'.split('')], new Set(['a','b','d']), 1)), 'cabedf');
  assert.equal(order(shiftSelectedOrder(tracks, ['abcdef'.split('')], new Set(['b','c','e']), -1)), 'bcaedf');
});
test('group boundaries and filtered-out slots remain fixed', () => {
  assert.equal(shiftSelectedOrder(tracks, [['a','b'],['c','d']], new Set(['b','d']), 1), null);
  assert.equal(order(shiftSelectedOrder(tracks, [['a','c','e']], new Set(['a']), 1)), 'cbadef');
});
test('empty, all-selected and boundary selections cannot move', () => {
  assert.equal(shiftSelectedOrder(tracks, [[]], new Set(), 1), null);
  assert.equal(shiftSelectedOrder(tracks, [['a']], new Set(['a']), -1), null);
  assert.deepEqual(movableDirections([['a','b']], new Set(['a','b'])), {up:false,down:false});
  assert.deepEqual(movableDirections([['a','b']], new Set(['a'])), {up:false,down:true});
});
