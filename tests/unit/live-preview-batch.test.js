import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import vm from 'node:vm';
import ts from 'typescript';

const root = resolve(process.env.TAGREX_BULK_SOURCE || '.');
const read = path => readFileSync(resolve(root, path), 'utf8');
const app = ts.createSourceFile('app.js', read('app/ui/app.js'), ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
let ribbon;
function visit(node) {
  if (ts.isCallExpression(node) && node.expression.getText(app) === 'connectRibbonActions') ribbon = node.arguments[0].getText(app);
  ts.forEachChild(node, visit);
}
visit(app);
assert.ok(ribbon, 'Production ribbon callback must exist');
const generator = read('app/ui/js/generator.js');
const adoption = generator.match(/^function adoptTagOnlyPreview\([^]*?^}/m)?.[0];
assert.ok(adoption, 'Production tag-only preview adoption must exist');

// Execute the production pending-input module and ribbon callback without app
// boot, timers, native commands or media. Count notifications, not wall time.
function harness(tracks, values = []) {
  const index = new Map(tracks.map(track => [track.path, track]));
  const paths = tracks.map(track => track.path);
  const subject = { paths };
  const calls = { events: 0, pendingEntriesAtEvents: 0, invoke: [], writes: [], observed: [] };
  const c = vm.createContext({
    edits: new Map(), tracks, trackAt: path => index.get(path), setPendingImportCover() {},
    applying: false, undoing: false, captureSelection: () => subject,
    current: true, isSelectionCurrent: () => c.current,
    previewPlan: null, isAutomaticTransformPreview: () => false,
    sharedStagingEnabled: false, diffByPath: null,
    setPreviewPlan: plan => { c.previewPlan = plan; }, setPreviewSource() {},
    CustomEvent: class { constructor(type) { this.type = type; } },
    document: { dispatchEvent(event) {
      assert.equal(event.type, 'tagrex:live-input');
      calls.events++;
      c.adoptTagOnlyPreview();
      calls.pendingEntriesAtEvents += c.edits.size;
      if (tracks.length < 10) calls.observed.push(JSON.parse(JSON.stringify(c.liveInputs().edits)));
    } },
    invoke: async (command, args) => {
      calls.invoke.push({ command, args });
      if (command === 'render_column') return values;
      if (command === 'preview_tag_edits') return { changes: args.edits };
      throw Error(`Unexpected command: ${command}`);
    },
    writeQuickPlan: plan => calls.writes.push(plan),
    hooks: { previewEdits: async () => c.liveInputChanged() },
  });
  vm.runInContext(read('app/ui/js/live-preview.js').replace(/^import .*;\n/m, '').replace(/^export /gm, ''), c);
  vm.runInContext(adoption, c);
  c.runRibbon = vm.runInContext(`(${ribbon})`, c);
  return { c, calls, paths };
}
const plain = value => JSON.parse(JSON.stringify(value));
const rows = count => Array.from({ length: count }, (_, index) => ({ path: `p${index}`, tags: { title: `Disk ${index}` } }));

for (const count of [2000, 20000]) test(`Format Values stages ${count} ordered values in one live notification`, async t => {
  const tracks = rows(count);
  const values = tracks.map((_, index) => index % 7 ? `Formatted ${index}` : '');
  const { c, calls, paths } = harness(tracks, values);
  const revision = c.liveInputRevision();
  assert.deepEqual(plain(await c.runRibbon('format-values', { field: 'title', pattern: '%artist%', mode: 'stage' })), { dialog: 'ribbon-format-dialog' });
  assert.deepEqual(plain(c.liveInputs().edits), paths.map((path, index) => ({ path, field: 'title', value: values[index] })));
  assert.deepEqual(paths, tracks.map(track => track.path));
  assert.deepEqual(calls.invoke.map(call => call.command), ['render_column']);
  assert.equal(calls.writes.length, 0);
  t.diagnostic(JSON.stringify({ entries: count, events: calls.events, pendingEntriesAtEvents: calls.pendingEntriesAtEvents }));
  assert.equal(calls.events, 1, 'one complete pending-input notification per ribbon action');
  assert.equal(c.liveInputRevision() - revision, 1);
  assert.equal(calls.pendingEntriesAtEvents, count, 'listeners see the whole batch once');
});

test('batch preserves imported overlays, cleanup, invalid values, missing paths and edit ordering', () => {
  const { c, calls } = harness(rows(3));
  assert.equal(typeof c.recordLiveFields, 'function');
  c.replaceLiveImport({ changes: [{ path: 'p1', tag_changes: [{ field: 'title', new: 'Imported' }, { field: 'artist', new: 'Imported artist' }] }] }, null, ['p1'], 0);
  c.recordLiveField(['p0'], 'artist', 'Keep');
  c.recordLiveField(['p0', 'p1'], 'title', 'Invalid draft', false);
  calls.events = 0; calls.observed.length = 0;
  c.recordLiveFields([
    { path: 'p0', field: 'title', value: 'Disk 0' },
    { path: 'p1', field: 'title', value: 'Imported' },
    { path: 'missing', field: 'title', value: 'Ignored', valid: false },
    { path: 'p2', field: 'title', value: '', valid: false },
    { path: 'p2', field: 'artist', value: 'First' },
    { path: 'p2', field: 'artist', value: 'Last' },
  ]);
  const expected = [
    { path: 'p1', field: 'title', value: 'Imported' },
    { path: 'p1', field: 'artist', value: 'Imported artist' },
    { path: 'p0', field: 'artist', value: 'Keep' },
    { path: 'p2', field: 'title', value: '' },
    { path: 'p2', field: 'artist', value: 'Last' },
  ];
  assert.deepEqual(plain(c.liveInputs().edits), expected);
  assert.deepEqual(calls.observed, [expected]);
  assert.equal(c.edits.get('p0').has('title'), false);
  assert.equal(c.edits.has('p1'), false, 'matching imported base removes only the raw override');
  assert.equal(c.edits.has('missing'), false);
  assert.equal(c.hasInvalidLiveFields(), true);
  assert.equal(c.hasInvalidLiveFields(['title']), false);
  assert.equal(calls.events, 1);
  c.recordLiveFields([{ path: 'p2', field: 'title', value: 'Disk 2' }]);
  assert.equal(c.hasInvalidLiveFields(), false);
  assert.equal(c.edits.get('p2').has('title'), false);
});

test('Format Values adopts independent staged fields before clearing later-path values back to base', async () => {
  const { c, calls } = harness(rows(3), ['Formatted', 'Disk 1', 'Disk 2']);
  c.replaceLiveImport({ changes: [{ path: 'p1', tag_changes: [{ field: 'album', new: 'Imported album' }] }] }, null, ['p1'], 0);
  c.previewPlan = { changes: [
    { path: 'p1', tag_changes: [{ field: 'title', new: 'Previous staged title' }, { field: 'artist', new: 'Keep staged artist' }] },
    { path: 'p2', tag_changes: [{ field: 'title', new: 'Previous later title' }] },
  ] };
  calls.events = 0; calls.observed.length = 0;
  await c.runRibbon('format-values', { field: 'title', pattern: '%title%', mode: 'stage' });
  const expected = [
    { path: 'p1', field: 'album', value: 'Imported album' },
    { path: 'p1', field: 'artist', value: 'Keep staged artist' },
    { path: 'p0', field: 'title', value: 'Formatted' },
  ];
  assert.deepEqual(plain(c.liveInputs().edits), expected);
  assert.deepEqual(calls.observed, [expected]);
  assert.equal(c.edits.has('p2'), false);
  assert.equal(calls.events, 1);
});

test('a no-op batch still invalidates once; single-value editing retains its notification contract', async () => {
  const { c, calls } = harness(rows(2), ['Disk 0', 'Disk 1']);
  await c.runRibbon('format-values', { field: 'title', pattern: '%title%', mode: 'stage' });
  assert.equal(c.edits.size, 0);
  assert.equal(calls.events, 1);
  c.recordLiveField(['p0', 'p1'], 'title', 'Typed');
  assert.equal(calls.events, 2);
  assert.deepEqual(plain(c.liveInputs().edits).map(edit => edit.value), ['Typed', 'Typed']);
});

test('Write uses its existing plan path; stale render results never mutate pending inputs', async () => {
  const { c, calls, paths } = harness(rows(2), ['Write', '']);
  await c.runRibbon('format-values', { field: 'title', pattern: '%artist%', mode: 'write' });
  assert.deepEqual(calls.invoke.map(call => call.command), ['render_column', 'preview_tag_edits']);
  assert.deepEqual(plain(calls.invoke[1].args), { edits: paths.map((path, index) => ({ path, field: 'title', value: index ? '' : 'Write' })), paths, cover: null });
  assert.equal(calls.writes.length, 1);
  assert.equal(calls.events, 0);
  assert.equal(c.edits.size, 0);
  c.current = false;
  await assert.rejects(c.runRibbon('format-values', { field: 'title', mode: 'stage' }), /selection changed/);
  assert.equal(calls.events, 0);
  assert.equal(c.edits.size, 0);
});
