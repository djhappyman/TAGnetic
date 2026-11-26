import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  compressionRatioAt, compressorOutput, dynamicsLevelFloor, gateOutput, levelX, levelY, plotBounds, xLevel, yLevel,
} from '../../app/ui/js/audio-treatment-plot.ts';

test('compression handles follow the hard- and soft-knee transfer curves', () => {
  assert.equal(compressorOutput(-26, -20, 4, 12), -26);
  assert.equal(compressorOutput(-20, -20, 4, 12), -21.125);
  assert.equal(compressorOutput(-14, -20, 4, 12), -18.5);
  assert.equal(compressorOutput(-14, -20, 4, 0), -18.5);

  // A low threshold's ratio handle is on the straight part of the curve.
  // With a high threshold and wide knee it is inside the transition.
  for (const threshold of [-30, -6]) {
    for (const ratio of [2, 4, 6]) {
      const output = compressorOutput(-1, threshold, ratio, 24);
      assert.ok(Math.abs(compressionRatioAt(-1, output, threshold, 24) - ratio) < 1e-9);
    }
  }
});

test('quiet-gap attenuation stays within its depth and the full control range is on the chart', () => {
  assert.equal(gateOutput(-25, -48, 12), -25);
  assert.equal(gateOutput(-54, -48, 12), -60);
  assert.equal(gateOutput(-72, -48, 12), -84);
  const bounds = plotBounds(360, 260);
  const floor = dynamicsLevelFloor('gate');
  assert.ok(levelX(-72, bounds, floor) > bounds.left);
  assert.ok(Math.abs(xLevel(levelX(-72, bounds, floor), bounds, floor) + 72) < 1e-9);
  assert.ok(Math.abs(yLevel(levelY(-72, bounds, floor), bounds, floor) + 72) < 1e-9);
});
