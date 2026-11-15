import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { RowHeights } from '../../app/ui/js/row-heights.js';
import { functionSource } from './interaction-codex-harness.js';

test('fractional mixed row heights and exact boundaries match a linear oracle after random updates', () => {
  const heights = Array.from({ length: 3811 }, (_, i) => i % 13 ? 23.5 : 47.125);
  const index = new RowHeights(); index.reset(heights.length, i => heights[i]);
  let seed = 1977;
  const random = () => (seed = (seed * 1664525 + 1013904223) >>> 0);
  for (let iteration = 0; iteration < 2000; iteration++) {
    const row = random() % heights.length;
    heights[row] = 18 + (random() % 250) / 8;
    index.set(row, heights[row]);
    const query = random() % heights.length;
    const top = heights.slice(0, query).reduce((a, b) => a + b, 0);
    assert.equal(index.top(query), top);
    assert.equal(index.indexAt(top), query);
    assert.equal(index.indexAt(top + heights[query] - 0.001), query);
    if (query) assert.equal(index.indexAt(top - 0.001), query - 1);
  }
  assert.equal(index.top(heights.length), heights.reduce((a, b) => a + b, 0));
  assert.equal(index.indexAt(-1), 0);
  assert.equal(index.indexAt(Infinity), heights.length - 1);
  index.reset(0, () => { throw Error('empty model'); });
  assert.equal(index.top(0), 0); assert.equal(index.indexAt(0), 0);
  index.reset(1, () => 22.5); assert.equal(index.indexAt(22.5), 0);
});

for (const size of [20000, 50000, 100000]) test(`new tall rows in a ${size}-row model update only their indexed heights`, () => {
  const heights = new RowHeights(); heights.reset(size, () => 23.5);
  let treeReads = 0, treeWrites = 0;
  heights.tree = new Proxy(heights.tree, {
    get(target, key) { if (/^\d+$/.test(String(key))) treeReads++; return Reflect.get(target, key, target); },
    set(target, key, value) { treeWrites++; return Reflect.set(target, key, value, target); },
  });
  const items = Array.from({ length: size }, () => ({ kind: 'track' }));
  const first = Math.floor(size / 2);
  let rebuilds = 0, visits = 0;
  const context = vm.createContext({
    renderedFirst: first, itemOffsets: heights, itemHeights: new Array(size),
    rowHeightEstimate: 23.5, rowEstimateMeasured: true, groupHeightEstimate: 27.5, groupEstimateMeasured: true,
    viewItems: new Proxy(items, { get(target, key, receiver) { if (/^\d+$/.test(String(key))) visits++; return Reflect.get(target, key, receiver); } }),
    tracksBody: { children: Array.from({ length: 30 }, () => ({ classList: { contains: () => false }, getBoundingClientRect: () => ({ height: 47.125 }) })) },
    recomputeItemTops: () => { rebuilds++; }, tableWrap: { scrollTop: 5000 }, setSpacerHeights: () => {},
  });
  const app = readFileSync('app/ui/app.js', 'utf8');
  vm.runInContext(functionSource(app, 'measureRenderedRows'), context);
  vm.runInContext('measureRenderedRows()', context);
  assert.equal(rebuilds, 0, 'individual staged-row measurements never rebuild the library');
  assert.equal(visits, 30);
  assert.ok(treeWrites <= 30 * Math.ceil(Math.log2(size)), `bounded height updates: ${treeWrites}`);
  assert.ok(treeReads < 600, `bounded prefix reads: ${treeReads}`);
  assert.equal(heights.top(first), first * 23.5);
  assert.equal(heights.top(size), size * 23.5 + 30 * (47.125 - 23.5));
  // A width change can make a previously tall row compact. Its old indexed
  // height must be replaced even when the new measurement equals the baseline.
  vm.runInContext('for(let i=renderedFirst;i<renderedFirst+30;i++)itemHeights[i]=null; tracksBody.children.forEach(row=>row.getBoundingClientRect=()=>({height:23.5})); measureRenderedRows()', context);
  assert.equal(heights.top(size), size * 23.5);
  assert.equal(rebuilds, 0);
});
