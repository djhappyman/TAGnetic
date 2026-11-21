import { test } from 'node:test';
import assert from 'node:assert/strict';
import { indexedMovableDirections, movableDirections, shiftSelectedOrder } from '../../app/ui/js/row-order.js';
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

test('sparse move controls check only selected visible neighbours', () => {
  const paths = Array.from({length: 100000}, (_, i) => String(i));
  const indexes = new Map(paths.map((path, index) => [path, index]));
  let probes = 0;
  const selected = new Set(['50000', 'hidden']);
  selected.has = path => { probes++; return Set.prototype.has.call(selected, path); };
  assert.deepEqual(indexedMovableDirections(paths, indexes, selected), {up: true, down: true});
  assert.equal(probes, 2);
});

test('indexed move detection matches every selection across independent group boundaries', () => {
  const groups = [['a', 'b'], ['c'], ['d', 'e', 'f']];
  const paths = groups.flat();
  const indexes = new Map(paths.map((path, index) => [path, index]));
  const groupByPath = new Map(groups.flatMap((group, index) => group.map(path => [path, index])));
  for (let mask = 0; mask < (1 << paths.length); mask++) {
    const selected = new Set(paths.filter((_, index) => mask & (1 << index)));
    selected.add('hidden');
    assert.deepEqual(indexedMovableDirections(paths, indexes, selected, groupByPath), movableDirections(groups, selected));
  }
});
