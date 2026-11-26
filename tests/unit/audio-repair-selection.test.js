import assert from 'node:assert/strict';
import { test } from 'node:test';
import { edgePanSpeed, editRepairBounds, repairDragAt } from '../../app/ui/js/audio-repair-selection.ts';

test('a clipped selection exposes its real corners instead of a false resize edge', () => {
  const plot = { left: 10, top: 10, right: 110, bottom: 110 };
  const selection = { left: -5, top: 25, right: 70, bottom: 92 };
  assert.deepEqual(repairDragAt(selection, plot, 10, 54), { x: 0, y: 0 });
  assert.deepEqual(repairDragAt(selection, plot, 70, 25), { x: 1, y: -1 });
  assert.deepEqual(repairDragAt(selection, plot, 26, 92), { x: 0, y: 1 });
  assert.equal(repairDragAt(selection, plot, 95, 54), undefined);
});

test('moving a repair preserves its time width and logarithmic band within the recording', () => {
  const original = { startFrame: 100, endFrame: 300, lowHz: 500, highHz: 1000 };
  assert.deepEqual(editRepairBounds(original, { x: 0, y: 0 }, -500, Math.log(2), 1000, 1800),
    { startFrame: 0, endFrame: 200, lowHz: 900, highHz: 1800 });
  assert.deepEqual(editRepairBounds(original, { x: 1, y: -1 }, -500, Math.log(.2), 1000, 1800),
    { startFrame: 100, endFrame: 101, lowHz: 500, highHz: 520 });
});

test('edge pan accelerates only in the plot gutters', () => {
  assert.equal(edgePanSpeed(55, 10, 100), 0);
  assert.equal(edgePanSpeed(9, 10, 100), -1);
  assert.equal(edgePanSpeed(101, 10, 100), 1);
  assert.ok(edgePanSpeed(95, 10, 100) > edgePanSpeed(85, 10, 100));
});
