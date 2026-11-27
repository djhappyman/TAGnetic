import assert from 'node:assert/strict';
import { test } from 'node:test';
import { analyseRepair } from '../../app/ui/js/audio-repair-analysis.ts';

const rate = 48_000;
const step = 480;
const frequencies = Array.from({ length: 200 }, (_, i) => 20 + i * 20);
const range = { startFrame: 0, endFrame: 24 * step, lowHz: 960, highHz: 1040 };

function tile(fill) {
  return {
    frequencies,
    spectrogram: Array.from({ length: 24 }, (_, x) => frequencies.map(hz => fill(hz, x))),
    spectrogramStart: 0,
    spectrogramEnd: 24 * step,
    spectrogramWindowFrames: 1024,
  };
}

test('the tone wand and harmonic review favour steady ridges and preserve gaps', () => {
  const probe = tile((hz, x) => hz === 1000 || hz === 3000 ? -24 : hz === 2000 && x < 4 ? -16 : -84);
  const tone = analyseRepair(probe, range, rate, 'tone', 3);
  const stack = analyseRepair(probe, range, rate, 'harmonics', 3);
  assert.equal(tone.candidates.length, 1);
  assert.deepEqual(stack.candidates.map(candidate => candidate.label), ['Fundamental near 1000 Hz', 'Harmonic 3 near 3000 Hz']);
  assert.ok(stack.candidates[0].bounds.highHz < 2000);
  assert.ok(stack.candidates[1].bounds.lowHz > 2000);
});

test('a full-height time selection can find a steady tone without a narrow seed band', () => {
  const probe = tile((hz, x) => hz === 1960 ? -28 : hz === 320 && x < 8 ? -15 : -84);
  const result = analyseRepair(probe, { ...range, lowHz: 0, highHz: 4000 }, rate, 'tone', 3);
  assert.equal(result.candidates[0]?.label, 'Nearby ridge near 1960 Hz');
});

test('the source guards deny analysis of treated or stale offscreen selections', () => {
  const probe = tile(hz => hz === 1000 ? -12 : -85);
  assert.equal(analyseRepair({ ...probe, spectrogramTreated: true }, range, rate, 'tone', 3).candidates.length, 0);
  assert.equal(analyseRepair({ ...probe, spectrogramDifference: true }, range, rate, 'tone', 3).candidates.length, 0);
  assert.equal(analyseRepair(probe, { ...range, startFrame: 4 * step, endFrame: 25 * step }, rate, 'tone', 3).candidates.length, 0);
});

test('find similar suggests comparable local peaks and omits the seed and weak events', () => {
  const probe = tile((hz, x) => {
    if (hz < 900 || hz > 2200) return -85;
    if (x === 4 || x === 15) return -28;
    if (x === 19) return -77;
    return -85;
  });
  const seed = { startFrame: 4 * step, endFrame: 5 * step, lowHz: 800, highHz: 2300 };
  assert.deepEqual(analyseRepair(probe, seed, rate, 'similar', 3).candidates.map(x => x.bounds.startFrame), [15 * step]);
});

test('click review requires a native PCM scan', () => {
  const probe = { ...tile(() => -48), spectrogramWindowFrames: 8192 };
  assert.match(analyseRepair(probe, range, rate, 'clicks', 3).detail, /Transients/);
  const clicks = analyseRepair({ ...tile(() => -48), spectrogramClicks: [{ frame: 1200, sensitivity: 1 }, { frame: 2500, sensitivity: 4 },
      { frame: 3000, sensitivity: 2 }, { frame: 4500, sensitivity: 1 }] },
    { ...range, startFrame: 2000, endFrame: 4300 }, rate, 'clicks', 3);
  assert.deepEqual(clicks.candidates.map(({ bounds }) => [bounds.startFrame, bounds.endFrame]), [[3000, 3001]]);
});

test('click review respects the selected source channel', () => {
  const probe = { ...tile(() => -48), spectrogramClicks: [
    { frame: 2500, sensitivity: 3, channel: 'left' }, { frame: 3500, sensitivity: 1, channel: 'right' },
  ] };
  const result = analyseRepair(probe, range, rate, 'clicks', 3, 'right');
  assert.equal(result.candidates.length, 1);
  assert.equal(result.candidates[0].channel, 'right');
  assert.equal(result.candidates[0].bounds.startFrame, 3500);
});

test('steady-noise analysis flags variation without generating a repair', () => {
  const steady = tile(hz => -55 - hz / 200);
  const variable = tile((hz, x) => -55 - hz / 200 + (x % 2 ? -16 : 16));
  const bands = { ...range, lowHz: 300, highHz: 4000 };
  assert.match(analyseRepair(steady, bands, rate, 'noise', 3).detail, /steady spectral floor/);
  assert.match(analyseRepair(variable, bands, rate, 'noise', 3).detail, /varies/);
  assert.equal(analyseRepair(steady, bands, rate, 'noise', 3).candidates.length, 0);
});
