import assert from 'node:assert/strict';
import { test } from 'node:test';
import { projectRepairFrame } from '../../app/ui/js/audio-repair-clock.ts';

const clock = { frame: 4800, observed: 1_000, rate: 48_000, frames: 100_000, running: true };

test('the Repair playhead fills the gaps between transport snapshots', () => {
  assert.equal(projectRepairFrame(clock, 1_016), 5568);
  assert.equal(projectRepairFrame(clock, 1_125), 10_800);
  assert.equal(projectRepairFrame(clock, 990), 4800);
});

test('stalled and paused previews stop projecting while the loop wraps in source coordinates', () => {
  assert.equal(projectRepairFrame(clock, 2_000), 16_800);
  assert.equal(projectRepairFrame({ ...clock, running: false }, 1_050), 4800);
  assert.equal(projectRepairFrame({ ...clock, frame: 19_000, loop: { start: 10_000, end: 20_000 } }, 1_100), 13_800);
  assert.equal(projectRepairFrame({ ...clock, frame: 99_000 }, 1_100), 99_999);
});
