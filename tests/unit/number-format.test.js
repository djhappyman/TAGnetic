import test from 'node:test';
import assert from 'node:assert/strict';
import { formatNumber, numberWidth, setNumberWidth } from '../../app/ui/js/number-format.js';

// The module reads its saved widths once at import, so every test states the
// width it starts from and counts the preference writes and table renders the
// setter actually causes.
function harness() {
  const calls = { stored: 0, renders: 0 };
  globalThis.localStorage = { getItem: () => null, setItem: () => { calls.stored++; } };
  globalThis.CustomEvent = class { constructor(type) { this.type = type; } };
  globalThis.document = { dispatchEvent(event) { assert.equal(event.type, 'tagrex:number-format'); calls.renders++; } };
  return calls;
}

test('setting a width to its current value persists and renders nothing', () => {
  const calls = harness();
  setNumberWidth('track', 1);
  setNumberWidth('track', '1');
  assert.deepEqual(calls, { stored: 0, renders: 0 });
  assert.equal(numberWidth('track'), 1);
});

test('a changed width is stored, clamped and rendered exactly once', () => {
  const calls = harness();
  setNumberWidth('track', 2);
  assert.deepEqual(calls, { stored: 1, renders: 1 });
  assert.equal(numberWidth('track'), 2);
  assert.equal(formatNumber('7', 'track'), '07');

  setNumberWidth('disc', 99);
  assert.deepEqual(calls, { stored: 2, renders: 2 });
  assert.equal(numberWidth('disc'), 3);
  assert.equal(formatNumber('9', 'disc'), '009');

  setNumberWidth('track', 2);
  assert.deepEqual(calls, { stored: 2, renders: 2 });
});

test('unknown widths and non-numeric values stay at one digit', () => {
  const calls = harness();
  setNumberWidth('position', 2);
  assert.deepEqual(calls, { stored: 0, renders: 0 });
  setNumberWidth('disc', 'not a number');
  assert.deepEqual(calls, { stored: 1, renders: 1 });
  assert.equal(numberWidth('disc'), 1);
});
