import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

// Exercise the production painter with an off-window rectangle. The proxies
// count model visits, so this catches linear scroll work without timing limits.
const source = readFileSync(process.env.TAGREX_TABLE_SHEET_SOURCE || 'app/ui/js/spreadsheet.js', 'utf8');
const ast = ts.createSourceFile('spreadsheet.js', source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
function functions(names) {
  const found = new Map();
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && names.includes(node.name?.text)) found.set(node.name.text, node.getText(ast));
    ts.forEachChild(node, visit);
  }
  visit(ast);
  return names.filter(name => found.has(name)).map(name => found.get(name)).join('\n');
}
function node() {
  const attributes = new Map(), classes = new Set();
  return { tabIndex: -1, hidden: false, getAttribute: key => attributes.get(key),
    setAttribute: (key, value) => attributes.set(key, String(value)), hasAttribute: key => attributes.has(key), removeAttribute: key => attributes.delete(key),
    classList: { add: key => classes.add(key), contains: key => classes.has(key), toggle: (key, on) => on ? classes.add(key) : classes.delete(key) } };
}

for (const size of [20000, 50000, 100000]) test(`painting a ${size}-row rectangle visits only mounted rows and preserves range edges`, () => {
  const values = Array.from({ length: size }, (_, index) => `p${index}`);
  let visits = 0;
  const paths = new Proxy(values, { get(target, key, receiver) { if (/^\d+$/.test(String(key))) visits++; return Reflect.get(target, key, receiver); } });
  const keys = ['file', 'artist', 'title'];
  const mounted = [0, Math.floor(size / 2), size - 1].map(index => ({ ...node(), dataset: { path: values[index] }, cells: Array.from({ length: 5 }, node) }));
  const snapshots = [];
  const context = vm.createContext({
    rows: () => paths, columns: () => keys, rowIndex: path => Number(path.slice(1)),
    active: { path: values.at(-1), key: 'title' }, anchor: { path: values[0], key: 'artist' },
    lastRows: paths, lastColumns: keys, sameSequence: (a, b) => a === b || a.length === b.length && a.every((value, index) => value === b[index]),
    wrap: { hidden: false }, host: node(), table: { ...node(), tHead: { rows: [node()] } },
    body: { querySelectorAll: () => mounted },
    options: { rowCount: () => size, rowNumber: path => Number(path.slice(1)) + 2, isReviewing: () => false, readOnly: () => '', label: key => key },
    TABLE_DATA_COLUMN_OFFSET: 2, paintedCells: new WeakMap(), observeSelectionHeader: () => {},
    attribute: (target, key, value) => target?.setAttribute(key, value), classState: (target, key, value) => target.classList.toggle(key, value),
    same: (a, b) => !!a && !!b && a.path === b.path && a.key === b.key,
    lastSummary: '', lastToolbar: null, toolbar: { update: value => snapshots.push(value) },
    busy: false, scrollRefresh: false, cellOf: () => null, gridOwnsFocus: false,
  });
  vm.runInContext(functions(['rangeBounds', 'range', 'refresh']), context);
  for (let iteration = 0; iteration < 20; iteration++) vm.runInContext('refresh({scrollOnly:true})', context);
  assert.equal(visits, 0, 'scroll paint must not copy or iterate an off-window range');
  for (const row of mounted) {
    assert.equal(row.cells[2].classList.contains('sheet-range'), false);
    assert.equal(row.cells[3].classList.contains('sheet-range'), true);
    assert.equal(row.cells[4].classList.contains('sheet-range'), true);
    assert.equal(row.cells[3].classList.contains('sheet-range-left'), true);
    assert.equal(row.cells[4].classList.contains('sheet-range-right'), true);
  }
  assert.equal(mounted[0].cells[3].classList.contains('sheet-range-top'), true);
  assert.equal(mounted[1].cells[3].classList.contains('sheet-range-top'), false);
  assert.equal(mounted[2].cells[4].classList.contains('sheet-range-bottom'), true);
  assert.equal(mounted[2].cells[4].classList.contains('sheet-active'), true);
  assert.equal(snapshots.at(-1).label, `title · ${size} × 2 cells`);
  assert.equal(snapshots.at(-1).canFill, true);
  // Explicit clipboard/batch operations still materialize every selected path.
  const range = vm.runInContext('range()', context);
  assert.equal(range.paths.length, size);
  assert.deepEqual(Array.from(range.keys), ['artist', 'title']);
});
