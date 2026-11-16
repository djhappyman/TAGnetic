import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createSharedStageSync, optimisticSharedPlan, stageRemainder, stagedFields, automaticStageScope } from '../../app/ui/js/shared-stage.js';

const change = (path, values) => ({ path, tag_changes: Object.entries(values).map(([field, value]) => ({ field, old: 'Saved', new: value })), rename_to: null, cover_change: null });
const plan = (...changes) => ({ description: 'Shared changes', changes });
const values = value => Object.fromEntries(value.changes[0].tag_changes.map(field => [field.field, field.new]));
const deferred = () => { let resolve; const promise = new Promise(done => resolve = done); return { promise, resolve }; };

test('an empty automatic preview does not republish or hydrate an unchanged shared stage', async () => {
  const current = plan(change('/music/a.flac', { title: 'Agent title' }));
  let publications = 0, hydrations = 0;
  const sync = createSharedStageSync({
    read: async () => ({ revision: 1, plan: current, human_plan: null }),
    publish: async () => { publications++; throw new Error('No local edit to publish'); },
    clear: async () => { throw new Error('No discard requested'); },
    context: () => ({ inputRevision: 0, library: current, busy: false }),
    adopt: () => { hydrations++; },
  });
  await sync.start();
  await sync.changed(null);
  await sync.changed(plan());
  assert.equal(publications, 0);
  assert.equal(hydrations, 1);
  assert.equal(sync.snapshot().plan, current);
});

test('a human title overlays an agent title while artist, album and artwork remain live', () => {
  const cover = { old: [], new: [{ kind: 'front', data_base64: 'SHARED_IMAGE_BYTES' }] };
  const base = plan({ ...change('/music/a.flac', { artist: 'Agent artist', title: 'Agent title', album: 'Agent album' }), cover_change: cover });
  const result = optimisticSharedPlan(stageRemainder({ plan: base }), plan(change('/music/a.flac', { title: 'Human title' })));
  assert.deepEqual(values(result), { artist: 'Agent artist', album: 'Agent album', title: 'Human title' });
  assert.equal(result.changes[0].cover_change, cover);
  assert.equal(result.changes[0].cover_change.new[0], cover.new[0]);
});

test('human-first staging survives another field added to the same file', () => {
  const base = plan(change('/music/a.flac', { title: 'Human first', artist: 'Agent later' }));
  const result = optimisticSharedPlan(stageRemainder({ plan: base }), plan(change('/music/a.flac', { album: 'Human album' })));
  assert.deepEqual(values(result), { title: 'Human first', artist: 'Agent later', album: 'Human album' });
});

test('a human saved-value reversion removes only the touched proposed field', () => {
  const base = plan(change('/music/a.flac', { artist: 'Keep artist', title: 'Remove title' }));
  const result = optimisticSharedPlan(base, null, [{ path: '/music/a.flac', field: 'title' }]);
  assert.deepEqual(values(result), { artist: 'Keep artist' });
});

test('automatic preset removal scopes only its human fields, preserving an agent cover', () => {
  const cover = { old: [], new: [{ kind: 'front', data_base64: 'IMAGE' }] };
  const automatic = plan(change('/music/a.flac', { title: 'Cleaned' }));
  const base = plan({ ...automatic.changes[0], cover_change: cover });
  const result = optimisticSharedPlan(base, null, stagedFields(automatic));
  assert.deepEqual(result.changes[0].tag_changes, []);
  assert.equal(result.changes[0].cover_change, cover);
});

test('a large native stage is not traversed for an optimistic input overlay', () => {
  let visited = 0;
  const huge = { changes: new Array(100000) };
  Object.defineProperty(huge.changes, '0', { get() { visited++; throw new Error('No full-stage input traversal'); } });
  assert.equal(stageRemainder({ plan: huge }), huge);
  assert.equal(optimisticSharedPlan(huge, plan(change('/music/a.flac', { title: 'New input' }))), huge);
  assert.equal(visited, 0);
});

test('selection changes retain previously staged human fields and never clear agent artwork', () => {
  const first = plan(change('/music/first.flac', { title: 'Keep first' }));
  const initialScope = stagedFields(first);
  const second = plan(change('/music/second.flac', { artist: 'Second artist' }));
  const moved = automaticStageScope(initialScope, second, ['/music/second.flac']);
  assert.deepEqual(moved.touched, [{ path: '/music/second.flac', field: 'artist' }]);
  assert.deepEqual(moved.fields, [...initialScope, ...stagedFields(second)]);
  const emptySelection = automaticStageScope(moved.fields, null, []);
  assert.deepEqual(emptySelection.touched, []);
  assert.deepEqual(emptySelection.fields, moved.fields);
  const changedPreset = automaticStageScope(moved.fields, null, ['/music/second.flac']);
  assert.deepEqual(changedPreset.touched, stagedFields(second));
  assert.deepEqual(changedPreset.fields, initialScope);
  const cover = { old: [], new: [{ kind: 'front', data_base64: 'KEEP_COVER' }] };
  const stage = plan(first.changes[0], { ...second.changes[0], cover_change: cover });
  const updated = optimisticSharedPlan(stage, null, changedPreset.touched);
  assert.equal(updated.changes[0].tag_changes[0].new, 'Keep first');
  assert.equal(updated.changes[1].cover_change, cover);
});

test('coalesced local publications use the latest human value and do not echo hydration', async () => {
  let native = { revision: 0, plan: null, human_plan: null };
  let inputRevision = 0;
  const library = {}, writes = [], hydrated = [];
  const sync = createSharedStageSync({
    read: async () => native,
    publish: async human => { writes.push(human); return native = { revision: native.revision + 1, plan: human, human_plan: human }; },
    clear: async () => native,
    context: () => ({ inputRevision, library, busy: false }),
    adopt: value => hydrated.push(value),
  });
  await sync.start();
  inputRevision++;
  sync.changed(plan(change('/music/a.flac', { title: 'Before' })));
  const latest = plan(change('/music/a.flac', { title: 'Latest' }));
  inputRevision++;
  sync.changed(latest);
  await sync.flush();
  assert.equal(writes.length, 1);
  assert.equal(writes[0], latest);
  await sync.refresh(); await sync.flush();
  assert.equal(writes.length, 1);
  assert.equal(hydrated.length, 2);
  assert.equal(hydrated.at(-1).plan, latest);
});

test('a native response cannot replace a newer raw input before its preview publishes', async () => {
  const response = deferred();
  const library = {}, hydrated = [];
  let inputRevision = 0, block = false;
  const sync = createSharedStageSync({
    read: async () => block ? response.promise : ({ revision: 0, plan: null, human_plan: null }),
    publish: async human => ({ revision: 2, plan: human, human_plan: human }), clear: async () => null,
    context: () => ({ inputRevision, library, busy: false }),
    adopt: value => hydrated.push(value),
  });
  await sync.start(); block = true;
  const pending = sync.refresh();
  await Promise.resolve(); await Promise.resolve();
  inputRevision++;
  const human = plan(change('/music/a.flac', { title: 'Fresh typing' }));
  sync.changed(human);
  response.resolve({ revision: 1, plan: plan(change('/music/a.flac', { title: 'Old agent value' })), human_plan: null });
  await pending; await sync.flush();
  assert.deepEqual(hydrated.map(value => value.revision), [0, 2]);
  assert.equal(hydrated.at(-1).plan, human);
});

test('native revision races are retried silently against the current stage', async () => {
  let revision = 1, attempts = 0, errors = 0;
  const library = {};
  const sync = createSharedStageSync({
    read: async () => ({ revision, plan: null, human_plan: null }),
    publish: async human => {
      if (++attempts === 1) { revision++; throw new Error('stage_conflict: staging changed'); }
      return { revision: ++revision, plan: human, human_plan: human };
    },
    clear: async () => null, context: () => ({ inputRevision: 0, library, busy: false }),
    adopt: () => {}, error: () => errors++,
  });
  await sync.start();
  sync.changed(plan(change('/music/a.flac', { title: 'Human desired' })));
  await sync.flush();
  assert.equal(attempts, 2); assert.equal(errors, 0); assert.equal(sync.revision(), 3);
});

test('discard is a barrier before subsequently typed fields are published', async () => {
  const calls = [], library = {};
  let revision = 0;
  const sync = createSharedStageSync({
    read: async () => ({ revision, plan: null, human_plan: null }),
    publish: async human => { calls.push('publish'); return { revision: ++revision, plan: human, human_plan: human }; },
    clear: async () => { calls.push('clear'); return { revision: ++revision, plan: null, human_plan: null }; },
    context: () => ({ inputRevision: 0, library, busy: false }), adopt: () => {},
  });
  await sync.start();
  sync.clear(); sync.changed(plan(change('/music/a.flac', { title: 'After discard' })));
  await sync.flush();
  assert.deepEqual(calls, ['clear', 'publish']);
});

test('unavailable native staging does not spin and prevents an unacknowledged Apply', async () => {
  let reads = 0, errors = 0;
  const sync = createSharedStageSync({
    read: async () => { reads++; throw new Error('Unavailable'); }, publish: async () => {}, clear: async () => {},
    context: () => ({ inputRevision: 0, library: null, busy: false }), adopt: () => {}, error: () => errors++,
  });
  await sync.start();
  await assert.rejects(sync.flush(), /Unavailable/);
  assert.equal(reads, 1); assert.equal(errors, 1);
});
