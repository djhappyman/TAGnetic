import { test } from 'node:test';
import assert from 'node:assert/strict';
import { desiredPlanFingerprint, equalDesiredFingerprints, canAdoptAgentRebase } from '../../app/ui/js/agent-write-guard.js';

const plan = () => ({ changes: [{ path: '/music/a.flac', tag_changes: [{ field: 'title', old: 'Old', new: 'Human title' }], cover_change: { old: [], new: [{ mime: 'image/png', data_base64: 'ORIGINAL', kind: 'front' }] } }] });
const publication = value => ({ generation: 4, inputRevision: 7, desired: desiredPlanFingerprint(value) });

test('an unchanged human intent accepts rebased saved values', () => {
  const current = plan(); const captured = publication(current);
  current.changes[0].tag_changes[0].old = 'Agent wrote this';
  assert.equal(canAdoptAgentRebase(captured, { plan: current, generation: 4, inputRevision: 7, event: { manual_rebased: true } }), true);
});

test('a new raw input is retained before its native preview can publish', () => {
  const current = plan(); const captured = publication(current);
  assert.equal(canAdoptAgentRebase(captured, { plan: current, generation: 4, inputRevision: 8, event: { manual_rebased: true } }), false);
});

test('a newer staged plan cannot be replaced by an earlier agent response', () => {
  const current = plan(); const captured = publication(current);
  assert.equal(canAdoptAgentRebase(captured, { plan: current, generation: 5, inputRevision: 7, event: { manual_rebased: true } }), false);
  assert.equal(canAdoptAgentRebase(captured, { plan: current, generation: 4, inputRevision: 7, event: { manual_rebased: false } }), false);
  assert.equal(canAdoptAgentRebase(undefined, { plan: current, generation: 4, inputRevision: 7, event: {} }), false);
});

test('in-place tag and image changes are detected without serializing artwork', () => {
  const current = plan(); const captured = publication(current);
  current.changes[0].tag_changes[0].new = 'Newer human title';
  assert.equal(equalDesiredFingerprints(captured.desired, desiredPlanFingerprint(current)), false);
  const imageOnly = plan(); const before = desiredPlanFingerprint(imageOnly);
  imageOnly.changes[0].cover_change.new[0].data_base64 = 'DIFFERENT';
  assert.equal(equalDesiredFingerprints(before, desiredPlanFingerprint(imageOnly)), false);
});

test('pooled and inline artwork express the same desired image', () => {
  const inline = plan(); const pooled = plan();
  pooled.cover_assets = ['ORIGINAL'];
  pooled.changes[0].cover_change.new[0].data_ref = 0;
  pooled.changes[0].cover_change.new[0].data_base64 = '';
  assert.equal(equalDesiredFingerprints(desiredPlanFingerprint(inline), desiredPlanFingerprint(pooled)), true);
});

test('a huge draft takes the native refresh path without traversing its cells', () => {
  let visited = 0;
  const huge = { changes: new Array(100000) };
  Object.defineProperty(huge.changes, '0', { get() { visited++; throw new Error('A UI fingerprint must not visit this row'); } });
  const captured = publication(huge);
  assert.equal(canAdoptAgentRebase(captured, { plan: huge, generation: 4, inputRevision: 7, event: { manual_rebased: true } }), false);
  assert.equal(visited, 0);
});
