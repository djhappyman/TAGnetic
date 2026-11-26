import { expect, test } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
import { noTreatment } from '../../app/ui/js/audio-treatment.ts';

const build = resolve('.test-dist');
const sourceId = 'dynamics-fixture';

async function openDynamics(page) {
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(build, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(build + sep)) return route.abort();
    try {
      await route.fulfill({
        body: await readFile(path),
        contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html',
      });
    } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
  });
  await page.addInitScript(state => {
    const handlers = new Map();
    window.dynamicsCalls = [];
    window.sendDynamicsProbe = probe => handlers.get('tagnetic:audio-tools-probe')?.({ payload: { sourceId: state.sourceId, probe } });
    const snapshot = () => handlers.get('tagnetic:audio-tools-state')?.({ payload: { ...state, settings: { ...state.settings } } });
    window.__TAURI__ = {
      event: {
        async listen(name, callback) { handlers.set(name, callback); return () => handlers.delete(name); },
        async emitTo(_target, _event, payload) {
          window.dynamicsCalls.push(payload);
          if (payload.action === 'edit') {
            Object.assign(state.settings, payload.patch);
            state.revision = payload.revision;
          }
          if (payload.action === 'sync' || payload.action === 'edit') queueMicrotask(snapshot);
        },
      },
    };
  }, {
    sourceId, frame: 48_000, frames: 480_000, sampleRate: 48_000, stereo: true,
    playing: false, original: false, loop: false,
    settings: { ...noTreatment, compressorDb: -18, compressorRatio: 3 },
    theme: { theme: 'dark', palette: 'navy', accent: [] },
    normalisation: { loudnessLufs: -22.7, gainDb: 4.7 },
  });
  await page.goto('http://tagrex.test/app/ui/audio-tools.html?tool=dynamics');
  await expect(page.getByRole('slider', { name: 'Compression threshold' })).toBeEnabled();
}

test('Dynamics rolls back a drag, keeps audition momentary, and resets only held peaks', async ({ page }) => {
  await openDynamics(page);

  const inputGain = page.getByRole('textbox', { name: 'Input gain in dB' });
  await expect(inputGain).toHaveValue('0');
  await inputGain.fill('12');
  await inputGain.press('Enter');
  await expect.poll(() => page.evaluate(() => window.dynamicsCalls.findLast(call => call.action === 'edit')?.patch))
    .toEqual({ compressorInputDb: 12 });

  const threshold = page.getByRole('slider', { name: 'Compression threshold' });
  const position = await threshold.boundingBox();
  const x = position.x + position.width / 2;
  const y = position.y + position.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x - 60, y);
  await expect(threshold).not.toHaveAttribute('aria-valuenow', '-18');
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect(threshold).toHaveAttribute('aria-valuenow', '-18');
  const edits = await page.evaluate(() => window.dynamicsCalls.filter(call => call.action === 'edit' && call.gesture));
  expect(edits.at(-1).patch.compressorDb).toBe(-18);
  expect(new Set(edits.map(edit => edit.gesture)).size).toBe(1);

  const difference = page.getByRole('button', { name: 'Hear what compression removes' });
  await expect(difference).toHaveCount(0);
  await page.getByRole('button', { name: 'Compressor audition' }).click();
  await difference.focus();
  await page.keyboard.down('Space');
  await expect(difference).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.up('Space');
  await expect(difference).toHaveAttribute('aria-pressed', 'false');
  expect(await page.evaluate(() => window.dynamicsCalls.filter(call => call.action.includes('audition') || call.action.includes('difference')).map(call => call.action)))
    .toEqual(['difference-compressor', 'stop-audition']);

  await page.evaluate(() => window.sendDynamicsProbe({
    frequencies: [], original: [], treated: [], reduction: [-2, -3], input: -10, output: -13, inputPeak: 0, outputPeak: -1,
  }));
  const peak = page.getByRole('button', { name: /^Reset peak hold/ });
  await expect(peak).toHaveAccessibleName(/Input clipped/);
  await peak.click();
  await expect(peak).toBeDisabled();

  const settings = page.getByRole('button', { name: 'Compressor settings' });
  const timing = page.getByRole('group', { name: 'Compressor settings' });
  await expect(timing).toHaveCount(0);
  await settings.click();
  await expect(difference).toHaveCount(0);
  const attack = timing.getByRole('textbox', { name: 'Attack in ms' });
  await expect(attack).toHaveValue('8');
  await expect(timing.getByRole('textbox', { name: 'Release in ms' })).toHaveValue('160');
  await expect(timing.getByRole('textbox', { name: 'Knee in dB' })).toHaveValue('0');
  await expect(timing.getByRole('textbox', { name: 'Output gain in dB' })).toHaveValue('0');
  await attack.fill('15');
  await attack.press('Enter');
  await expect.poll(() => page.evaluate(() => window.dynamicsCalls.findLast(call => call.action === 'edit')?.patch.compressorAttackMs)).toBe(15);
  await attack.press('Escape');
  await expect(timing).toHaveCount(0);
  await expect(settings).toBeFocused();

  await page.getByRole('button', { name: 'Quiet-gap reduction controls' }).click();
  await expect(page.getByRole('slider', { name: 'Quiet-gap threshold' })).toBeEnabled();
  await page.getByRole('button', { name: 'Gain controls' }).click();
  await expect(page.getByRole('group', { name: 'Input, output and headroom' })).toBeVisible();
  await page.getByRole('button', { name: 'Normalise controls' }).click();
  await page.getByRole('button', { name: 'Normalise the whole set' }).click();
  await expect(page.getByText('Whole set', { exact: true })).toBeVisible();
});
