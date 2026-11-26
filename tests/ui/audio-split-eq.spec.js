import { expect, test } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
import { noTreatment } from '../../app/ui/js/audio-treatment.ts';

const root = resolve('.test-dist');

test.beforeEach(async ({ page }) => {
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      await route.fulfill({
        body: await readFile(path),
        contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html',
      });
    } catch {
      await route.fulfill({ status: 404, body: 'Missing fixture' });
    }
  });
  await page.addInitScript(settings => {
    const handlers = new Map();
    const initial = {
      sourceId: 'fixture', frame: 0, sampleRate: 48000, frames: 480000, settings,
      stereo: true, playing: false, original: false, loop: false,
      theme: { theme: 'dark', palette: 'navy', font: 'plex', accent: [] },
    };
    let state = initial;
    const requests = [];
    const publish = () => handlers.get('tagnetic:audio-tools-state')?.({ payload: state });
    window.eqFixture = {
      requests,
      settings: () => state.settings,
      probe(original, treated) {
        handlers.get('tagnetic:audio-tools-probe')?.({
          payload: { sourceId: state.sourceId, probe: { frequencies: [120, 3000], original, treated, input: -18, output: -18, reduction: [] } },
        });
      },
    };
    window.__TAURI__ = {
      event: {
        async listen(name, callback) { handlers.set(name, callback); return () => handlers.delete(name); },
        async emitTo(_target, _name, request) {
          requests.push(request);
          if (request.action === 'sync') publish();
          if (request.action === 'edit') {
            state = { ...state, revision: request.revision, settings: { ...state.settings, ...request.patch } };
            queueMicrotask(publish);
          }
        },
      },
      core: { async invoke() {} },
    };
  }, noTreatment);
  await page.goto('http://tagrex.test/app/ui/audio-tools.html?tool=eq');
  await expect(page.getByRole('button', { name: 'Add EQ band on the graph' })).toBeEnabled();
});

const settingsOf = page => page.evaluate(() => window.eqFixture.settings());

test('EQ graph creates bands, opens exact values and preserves parameters through bypass', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const hit = graph.getByRole('button', { name: 'Add EQ band on the graph' });
  const size = await hit.boundingBox();
  const anchor = { x: Math.round(size.width * .52), y: Math.round(size.height * .29) };
  await hit.hover({ position: anchor });
  await expect(graph.getByText('Add bell')).toBeVisible();
  await hit.click({ position: anchor });
  const bell = graph.getByRole('button', { name: 'Bell', exact: true });
  await expect(bell).toBeFocused();
  await bell.press('Enter');
  const frequency = graph.getByRole('textbox', { name: 'Frequency in Hz' });
  await expect(frequency).toBeFocused();
  await frequency.fill('1750');
  await frequency.press('Enter');
  await expect.poll(async () => (await settingsOf(page)).presenceHz).toBe(1750);

  await bell.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Bypass band', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBypassed).toBe(4);
  await expect.poll(async () => (await settingsOf(page)).presenceHz).toBe(1750);
  await bell.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Enable band', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBypassed).toBe(0);

  const add = page.getByRole('button', { name: 'Add EQ band', exact: true });
  await add.click();
  await page.getByRole('menuitem', { name: 'Add bell' }).click();
  const extra = graph.getByRole('button', { name: 'Bell 1', exact: true });
  await expect(extra).toBeVisible();
  await extra.press('[');
  await expect(bell).toBeFocused();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.length).toBe(1);
  await bell.press('Control+c');
  await extra.press('Control+v');
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.hz).toBe(1750);
  await extra.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Change shape' }).hover();
  await page.getByRole('menuitem', { name: 'Notch', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.kind).toBe('notch');
});

test('EQ cancels a captured drag, freezes probes and releases momentary band audition', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const hit = graph.getByRole('button', { name: 'Add EQ band on the graph' });
  const size = await hit.boundingBox();
  await hit.click({ position: { x: Math.round(size.width * .52), y: Math.round(size.height * .29) } });
  const bell = graph.getByRole('button', { name: 'Bell', exact: true });
  const baseline = (await settingsOf(page)).presenceHz;
  const dot = await bell.boundingBox();
  await page.mouse.move(dot.x + dot.width / 2, dot.y + dot.height / 2);
  await page.mouse.down();
  await page.mouse.move(dot.x + dot.width / 2 + 55, dot.y + dot.height / 2, { steps: 5 });
  await expect.poll(async () => (await settingsOf(page)).presenceHz).not.toBe(baseline);
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect.poll(async () => (await settingsOf(page)).presenceHz).toBe(baseline);

  await page.evaluate(() => window.eqFixture.probe([-45, -31], [-44, -36]));
  const freeze = page.getByRole('button', { name: 'Freeze spectrum' });
  await freeze.click();
  const image = () => graph.locator('canvas').evaluate(canvas => canvas.toDataURL());
  const before = await image();
  await page.evaluate(() => window.eqFixture.probe([-9, -68], [-69, -12]));
  await expect(page.getByRole('button', { name: 'Resume spectrum' })).toHaveAttribute('aria-pressed', 'true');
  expect(await image()).toBe(before);
  await page.getByRole('button', { name: 'Resume spectrum' }).click();
  await expect.poll(image).not.toBe(before);

  const solo = graph.getByRole('button', { name: 'Hold to solo band' });
  await solo.focus();
  await page.keyboard.down('Space');
  await expect.poll(() => page.evaluate(() => window.eqFixture.requests.at(-1)?.action)).toBe('solo-band');
  await page.keyboard.up('Space');
  await expect.poll(() => page.evaluate(() => window.eqFixture.requests.at(-1)?.action)).toBe('stop-audition');
});
