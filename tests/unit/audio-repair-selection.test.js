import assert from 'node:assert/strict';
import { test } from 'node:test';
import { edgePanSpeed, editRepairBounds, remapRepairMask, repairDragAt, repairMaskBounds, repairRegionPath } from '../../app/ui/js/audio-repair-selection.ts';

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

test('mask geometry follows moves and resizes while subtract regions leave the add bounds unchanged', () => {
  const mask = { version: 1, featherFrames: 10, featherHz: 20, regions: [
    { kind: 'brush', operation: 'add', radiusFrames: 30, radiusHz: 60, points: [{ frame: 100, hz: 220 }, { frame: 300, hz: 440 }] },
    { kind: 'rectangle', operation: 'subtract', startFrame: 0, endFrame: 200, lowHz: 180, highHz: 440 },
  ] };
  const base = repairMaskBounds(mask.regions, 1200, 2000);
  assert.deepEqual(base, { startFrame: 70, endFrame: 330, lowHz: 160, highHz: 500 });
  const position = { startFrame: 300, endFrame: 820, lowHz: 320, highHz: 1000 };
  const moved = remapRepairMask(mask, base, position, 1200, 2000, 4000);
  assert.deepEqual(repairMaskBounds(moved.regions, 1200, 2000), position);
  assert.deepEqual(moved.regions[0].points, [{ frame: 360, hz: 440 }, { frame: 760, hz: 880 }]);
  assert.equal(moved.regions[0].radiusFrames, 60);
  assert.match(repairRegionPath(moved.regions[0], point => point.frame + ',' + point.hz), /^M.*ZM.*Z$/);
});

test('moving a full-band mask keeps the Nyquist and DC boundaries finite', () => {
  const original = { startFrame: 100, endFrame: 300, lowHz: 0, highHz: 24_000 };
  assert.deepEqual(editRepairBounds(original, { x: 0, y: 0 }, 80, Math.log(1.8), 1_000, 24_000, 0),
    { startFrame: 180, endFrame: 380, lowHz: 0, highHz: 24_000 });
});

test('moving a mask to a source boundary clips extending subtract regions to valid points', () => {
  const before = { startFrame: 120, endFrame: 300, lowHz: 120, highHz: 500 };
  const after = { startFrame: 0, endFrame: 180, lowHz: 0, highHz: 380 };
  const mask = { version: 1, featherFrames: 0, featherHz: 0, regions: [
    { kind: 'rectangle', operation: 'add', ...before },
    { kind: 'rectangle', operation: 'subtract', startFrame: 0, endFrame: 50, lowHz: 20, highHz: 90 },
    { kind: 'lasso', operation: 'subtract', points: [
      { frame: 100, hz: 220 }, { frame: 200, hz: 180 }, { frame: 240, hz: 100 }, { frame: 190, hz: 160 },
    ] },
  ] };
  const moved = remapRepairMask(mask, before, after, 2000, 1000, 2000);
  assert.deepEqual(moved.regions[0], { kind: 'rectangle', operation: 'add', ...after });
  assert.equal(moved.regions.length, 2);
  assert.ok(moved.regions[1].points.every(point => point.frame >= 0 && point.hz >= 0));
  assert.ok(moved.regions[1].points.some(point => point.frame === 0));
});

test('exact resizing rejects brushes wider than the source-relative native limit', () => {
  const point = { frame: 100_000, hz: 600 };
  const mask = { version: 1, featherFrames: 0, featherHz: 0, regions: [
    { kind: 'brush', operation: 'add', radiusFrames: 816, radiusHz: 100, points: [point] },
  ] };
  const before = repairMaskBounds(mask.regions, 600_000, 24_000);
  const tooLarge = { ...before, endFrame: before.startFrame + 7 * 48_000 };
  assert.equal(remapRepairMask(mask, before, tooLarge, 600_000, 24_000, 48_000), undefined);
  const limited = { ...before, endFrame: before.startFrame + 4 * 48_000 };
  assert.equal(remapRepairMask(mask, before, limited, 600_000, 24_000, 48_000).regions[0].radiusFrames, 96_000);
});
