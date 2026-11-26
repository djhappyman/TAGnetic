import assert from 'node:assert/strict';
import { test } from 'node:test';
import { noTreatment, readTreatment, sameTreatment } from '../../app/ui/js/audio-treatment.ts';
import {
  createSpectrumMotion, eqResponse, frequencyX, gainY, levelX, levelY, plotBounds, xFrequency, xLevel, yGain, yLevel,
} from '../../app/ui/js/audio-treatment-plot.ts';

function closeTo(actual, expected, tolerance = 0.0001) {
  assert.ok(Math.abs(actual - expected) < tolerance, `Expected ${actual} to be within ${tolerance} of ${expected}`);
}

const rate = 48_000;

test('repair drafts retain the positions of edits across removal and undo', () => {
  const repair = startFrame => ({ startFrame, endFrame: startFrame + 2000, lowHz: 350, highHz: 550, amountDb: 12 });
  const [first, second, third] = [repair(10_000), repair(40_000), repair(120_000)];
  const original = readTreatment({ spectralRepairs: [first, second, third] });
  const removed = readTreatment({ spectralRepairs: [null, second, third, null] });
  assert.deepEqual(removed.spectralRepairs, [null, second, third]);
  assert.deepEqual(original.spectralRepairs[2], removed.spectralRepairs[2]);
  assert.equal(sameTreatment(readTreatment(removed), removed), true);
  assert.equal(sameTreatment(original, removed), false);
});

test('EQ response follows Butterworth cutoff and slope settings in Rust', () => {
  const lowCut = { ...noTreatment, rumble: true };
  const gentler = eqResponse(lowCut, rate);
  const steeper = eqResponse({ ...lowCut, rumbleSlope: 48 }, rate);
  closeTo(gentler(35), -3.0103);
  closeTo(steeper(35), -3.0103);
  assert.ok(gentler(17.5) > steeper(17.5) + 30);

  const highCut = { ...noTreatment, highCutHz: 8000 };
  const highGentler = eqResponse(highCut, rate);
  const highSteeper = eqResponse({ ...highCut, highCutSlope: 48 }, rate);
  closeTo(highGentler(8000), -3.0103);
  closeTo(highSteeper(8000), -3.0103);
  assert.ok(highGentler(16_000) > highSteeper(16_000) + 30);
});

test('EQ plots bell tuning, shelf pivot points, and exact notch centers', () => {
  const bell = { ...noTreatment, presenceHz: 2300, presenceDb: 6 };
  const wide = eqResponse(bell, rate);
  const narrow = eqResponse({ ...bell, presenceQ: 400 }, rate);
  closeTo(wide(2300), 6);
  closeTo(narrow(2300), 6);
  assert.ok(wide(3000) > narrow(3000));
  closeTo(eqResponse({ ...noTreatment, bassHz: 230, bassDb: -6 }, rate)(230), -3);
  closeTo(eqResponse({ ...noTreatment, trebleHz: 5600, trebleDb: 6 }, rate)(5600), 3);
  assert.ok(eqResponse({ ...noTreatment, notchHz: 999, notchQ: 30 }, rate)(999) < -80);
});

test('pointer transforms match the rendered scales on desktop and compact plots', () => {
  for (const [width, height] of [[1000, 240], [375, 190]]) {
    const bounds = plotBounds(width, height);
    for (const sampleRate of [32_000, rate]) {
      closeTo(xFrequency(bounds.left, bounds, sampleRate), 20);
      closeTo(xFrequency(bounds.right, bounds, sampleRate), Math.min(20_000, sampleRate * 0.45));
      closeTo(xFrequency(frequencyX(1734, bounds, sampleRate), bounds, sampleRate), 1734);
    }
    closeTo(yGain(gainY(-11.5, bounds), bounds), -11.5);
    closeTo(xLevel(levelX(-21, bounds), bounds), -21);
    closeTo(yLevel(levelY(-21, bounds), bounds), -21);
    assert.ok(levelY(-60, bounds) < bounds.bottom);
  }
});


test('EQ spectrum moves between measured playback snapshots and stays still when paused', t => {
  let time = 100;
  let nextFrame = 1;
  const frames = new Map();
  const originalRequest = globalThis.requestAnimationFrame;
  const originalCancel = globalThis.cancelAnimationFrame;
  t.after(() => {
    globalThis.requestAnimationFrame = originalRequest;
    globalThis.cancelAnimationFrame = originalCancel;
  });
  t.mock.method(performance, 'now', () => time);
  globalThis.requestAnimationFrame = callback => {
    const id = nextFrame++;
    frames.set(id, callback);
    return id;
  };
  globalThis.cancelAnimationFrame = id => frames.delete(id);
  function advance(elapsed) {
    time += elapsed;
    for (const [id, callback] of [...frames]) {
      frames.delete(id);
      callback(time);
    }
  }
  const initial = { frequencies: [20, 80], original: [-51, -32], treated: [-54, -21], input: 0, output: 0, reduction: [] };
  const next = { ...initial, original: [-35, -40], treated: [-39, -30] };
  const motion = createSpectrumMotion();
  let shown;
  const paint = probe => { shown = probe; };
  motion(initial, true, paint);
  assert.equal(shown, initial);
  assert.equal(frames.size, 0);

  const stop = motion(next, true, paint);
  assert.deepEqual(shown.original, initial.original);
  assert.deepEqual(shown.treated, initial.treated);
  advance(40);
  assert.ok(shown.original[0] > initial.original[0] && shown.original[0] < next.original[0]);
  assert.ok(shown.treated[1] < initial.treated[1] && shown.treated[1] > next.treated[1]);
  advance(65);
  assert.equal(shown, next);
  assert.equal(frames.size, 0);
  stop();

  motion({ ...next }, true, paint);
  assert.equal(frames.size, 0);
  const cancelled = motion(initial, true, paint);
  assert.equal(frames.size, 1);
  cancelled();
  motion(initial, false, paint);
  assert.equal(shown, initial);
  assert.equal(frames.size, 0);
  const otherGrid = { ...next, frequencies: [20, 100] };
  motion(otherGrid, true, paint);
  assert.equal(shown, otherGrid);
  assert.equal(frames.size, 0);
});
