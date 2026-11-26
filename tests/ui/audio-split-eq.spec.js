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
      stereo: true, playing: false, original: false, loop: false, eqLevelComparison: true,
      theme: { theme: 'dark', palette: 'navy', font: 'plex', accent: [] },
    };
    let state = initial;
    let playingBeforeSolo;
    const requests = [];
    const publish = () => handlers.get('tagnetic:audio-tools-state')?.({ payload: state });
    window.eqFixture = {
      requests,
      settings: () => state.settings,
      setSettings(patch) {
        state = { ...state, settings: { ...state.settings, ...patch } };
        publish();
      },
      setBands(eqBands) {
        state = { ...state, settings: { ...state.settings, eqBands } };
        publish();
      },
      setPlaying(playing) {
        state = { ...state, playing };
        publish();
      },
      setOriginal(original) {
        state = { ...state, original };
        publish();
      },
      probe(original, treated, peaks = {}) {
        handlers.get('tagnetic:audio-tools-probe')?.({
          payload: { sourceId: state.sourceId, probe: {
            frequencies: original.length ? [120, 3000] : [], original, treated, input: -18, output: -18, reduction: [], eqInputPeak: peaks.inputPeak, ...peaks,
          } },
        });
      },
    };
    window.__TAURI__ = {
      event: {
        async listen(name, callback) { handlers.set(name, callback); return () => handlers.delete(name); },
        async emitTo(_target, _name, request) {
          requests.push(request);
          if (request.action === 'sync') publish();
          if (request.action === 'compare') {
            state = { ...state, original: request.original };
            queueMicrotask(publish);
          }
          if (request.action === 'solo-band') {
            playingBeforeSolo ??= state.playing;
            state = { ...state, playing: true, monitor: 'band' };
            queueMicrotask(publish);
          }
          if (request.action === 'stop-audition') {
            state = { ...state, playing: playingBeforeSolo ?? state.playing, monitor: undefined };
            playingBeforeSolo = undefined;
            queueMicrotask(publish);
          }
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
  const anchor = { x: Math.round(size.width * .52), y: Math.round(size.height / 2) };
  await hit.hover({ position: anchor });
  await expect(graph.locator('.ghost')).toBeVisible();
  await hit.click({ position: anchor });
  const bell = graph.getByRole('button', { name: 'Bell 1', exact: true });
  await expect(bell).toBeFocused();
  const controls = page.getByRole('group', { name: 'Bell 1 settings' });
  await bell.click();
  await expect(controls).toBeVisible();
  await expect(page.getByRole('group', { name: 'EQ bands' })).toHaveCount(0);
  await expect(controls.getByRole('textbox')).toHaveCount(0);
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.gainDb).toBe(0);
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.adaptiveQ).toBe(true);
  await bell.press('Shift+F10');
  const adaptive = page.getByRole('menuitemcheckbox', { name: 'Adaptive Q', exact: true });
  await expect(adaptive).toHaveAttribute('aria-checked', 'true');
  await adaptive.click();
  await expect.poll(async () => !!(await settingsOf(page)).eqBands?.[0]?.adaptiveQ).toBe(false);
  await bell.press('Enter');
  const frequency = controls.getByRole('textbox', { name: 'Frequency in Hz' });
  await expect(frequency).toBeFocused();
  const openGraph = await graph.boundingBox();
  expect(openGraph).toEqual(size);
  const strip = await controls.boundingBox();
  expect(strip.y).toBeGreaterThan(openGraph.y);
  expect(strip.y + strip.height).toBeLessThan(openGraph.y + openGraph.height);
  const band = await bell.boundingBox();
  expect(strip.x + strip.width / 2).toBeCloseTo(band.x + band.width / 2, 0);
  await frequency.fill('1750');
  await frequency.press('Enter');
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.hz).toBe(1750);
  await expect(frequency).toHaveCount(0);

  const bandGain = controls.getByRole('textbox', { name: 'Gain in dB', exact: true });
  const gainReadout = controls.locator('[data-eq-dial="Gain"] .readout');
  const savedGain = async () => (await settingsOf(page)).eqBands?.[0]?.gainDb;
  await gainReadout.click();
  await expect(bandGain).toHaveCount(0);
  await gainReadout.dblclick();
  await expect(bandGain).toBeFocused();
  await expect(bandGain).toHaveCSS('outline-style', 'none');
  await bandGain.fill('13.7');
  await bandGain.press('Enter');
  await expect.poll(savedGain).toBe(13.7);
  await gainReadout.dblclick();
  await bandGain.fill('2');
  await bandGain.press('Escape');
  await expect(bandGain).toHaveCount(0);
  expect(await savedGain()).toBe(13.7);
  await bell.press('ArrowUp');
  await expect.poll(savedGain).toBe(14.2);
  await bell.press('Shift+ArrowDown');
  await expect.poll(savedGain).toBe(14.1);
  const dot = await bell.boundingBox();
  const x = dot.x + dot.width / 2;
  const y = dot.y + dot.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.keyboard.down('Shift');
  await page.mouse.move(x, y - 5, { steps: 5 });
  await page.mouse.up();
  await page.keyboard.up('Shift');
  await expect.poll(savedGain).toBeGreaterThan(14.1);
  expect(await savedGain()).toBeLessThan(14.6);

  await bell.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Bypass band', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.bypassed).toBe(true);
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.hz).toBe(1750);
  await bell.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Enable band', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.bypassed).toBe(false);

  await graph.getByRole('button', { name: 'Add EQ band on the graph' }).press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Add bell' }).click();
  const extra = graph.getByRole('button', { name: 'Bell 2', exact: true });
  await expect(extra).toBeVisible();
  await expect(controls).toHaveCount(0);
  await expect(graph.getByRole('group', { name: 'Bell 2 settings' })).toBeVisible();
  await page.mouse.move(0, 0);
  await expect(bell).toHaveCSS('box-shadow', 'none');
  await expect(extra).toHaveCSS('box-shadow', 'none');
  await expect(bell).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expect(extra).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expect(extra).toHaveCSS('outline-style', 'none');
  const firstColor = await bell.evaluate(node => getComputedStyle(node).getPropertyValue('--band-color'));
  expect(await extra.evaluate(node => getComputedStyle(node).getPropertyValue('--band-color'))).not.toBe(firstColor);
  await extra.press('[');
  await expect(bell).toBeFocused();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.length).toBe(2);
  await bell.press('Control+c');
  await extra.press('Control+v');
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[1]?.hz).toBe(1750);
  await extra.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Change shape' }).hover();
  await page.getByRole('menuitem', { name: 'Notch', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[1]?.kind).toBe('notch');
});

test('EQ cancels a captured drag, freezes probes and releases momentary band audition', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const hit = graph.getByRole('button', { name: 'Add EQ band on the graph' });
  const size = await hit.boundingBox();
  await hit.click({ position: { x: Math.round(size.width * .52), y: Math.round(size.height / 2) } });
  const bell = graph.getByRole('button', { name: 'Bell 1', exact: true });
  const controls = page.getByRole('group', { name: 'Bell 1 settings' });
  await bell.dblclick();
  await expect(controls).toBeVisible();
  const origin = await controls.boundingBox();
  const baseline = (await settingsOf(page)).eqBands?.[0]?.hz;
  const dot = await bell.boundingBox();
  const gripX = dot.x + 5;
  const gripY = dot.y + dot.height / 2;
  await page.mouse.move(gripX, gripY);
  await expect(graph.locator('.ghost')).toHaveCount(0);
  expect(await bell.evaluate((element, { x, y }) => document.elementFromPoint(x, y) === element, { x: gripX, y: gripY })).toBe(true);
  await page.mouse.down();
  await page.mouse.move(gripX + 55, gripY - 20, { steps: 5 });
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.hz).not.toBe(baseline);
  await expect.poll(async () => (await controls.boundingBox()).x).toBeGreaterThan(origin.x + 45);
  expect((await controls.boundingBox()).y).toBeCloseTo(origin.y, 0);
  await expect(controls).toBeVisible();
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.hz).toBe(baseline);
  await expect.poll(async () => (await controls.boundingBox()).x).toBeCloseTo(origin.x, 0);
  await expect.poll(async () => (await controls.boundingBox()).y).toBeCloseTo(origin.y, 0);

  const input = page.getByRole('meter', { name: 'Input peak level' });
  const output = page.getByRole('meter', { name: 'Output peak level' });
  const firstMeter = await input.boundingBox();
  const secondMeter = await output.boundingBox();
  const openGraph = await graph.boundingBox();
  expect(firstMeter.x).toBeGreaterThan(openGraph.x + openGraph.width);
  expect(secondMeter.x).toBeGreaterThan(firstMeter.x);
  expect(firstMeter.height).toBeCloseTo(openGraph.height - 50, 0);
  expect(secondMeter.height).toBeCloseTo(firstMeter.height, 0);
  await page.evaluate(() => window.eqFixture.probe([-45, -31], [-44, -36], { inputPeak: -12, outputPeak: -6 }));
  await expect(input).toHaveAttribute('aria-valuenow', '-12');
  await expect(output).toHaveAttribute('aria-valuenow', '-6');
  const freeze = page.getByRole('button', { name: 'Freeze spectrum' });
  await freeze.click();
  await expect.poll(() => page.evaluate(() => window.eqFixture.requests.findLast(call => call.action === 'analyzer')?.enabled)).toBe(false);
  const image = () => graph.locator('canvas').evaluate(canvas => canvas.toDataURL());
  const before = await image();
  await page.evaluate(() => window.eqFixture.probe([], [], { inputPeak: -9, outputPeak: -1 }));
  await expect(input).toHaveAttribute('aria-valuenow', '-9');
  await expect(output).toHaveAttribute('aria-valuenow', '-1');
  await expect(page.getByRole('button', { name: 'Resume spectrum' })).toHaveAttribute('aria-pressed', 'true');
  expect(await image()).toBe(before);
  await page.getByRole('button', { name: 'Resume spectrum' }).click();
  await expect.poll(() => page.evaluate(() => window.eqFixture.requests.findLast(call => call.action === 'analyzer')?.enabled)).toBe(true);
  await page.evaluate(() => window.eqFixture.probe([-9, -68], [-69, -12], { inputPeak: -9, outputPeak: -1 }));
  await expect.poll(image).not.toBe(before);

  const solo = graph.getByRole('button', { name: 'Hold to solo Bell 1' });
  await solo.focus();
  await page.keyboard.down('Space');
  await expect(solo).toHaveAttribute('aria-pressed', 'true');
  await expect.poll(() => page.evaluate(() => window.eqFixture.requests.some(call => call.action === 'solo-band'))).toBe(true);
  await page.keyboard.up('Space');
  await expect(solo).toHaveAttribute('aria-pressed', 'false');
  await expect.poll(() => page.evaluate(() => window.eqFixture.requests.some(call => call.action === 'stop-audition'))).toBe(true);
});

test('the EQ curve keeps painting through consecutive frames of a captured node drag', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => {
    window.eqFixture.setBands([{ id: 1, kind: 'bell', hz: 800, gainDb: 6, q: 100 }]);
    window.eqFixture.setPlaying(true);
    window.eqFixture.probe([-32, -41], [-30, -41]);
  });
  const node = graph.getByRole('button', { name: 'Bell 1', exact: true });
  await node.click();
  await node.evaluate(element => {
    element.addEventListener('pointerdown', event => { element.dataset.testPointer = String(event.pointerId); }, { once: true });
  });
  const box = await node.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  const frames = await node.evaluate((element, { x, y }) => {
    const canvas = element.closest('.plot').querySelector('canvas');
    const context = canvas.getContext('2d');
    const canvasLeft = canvas.getBoundingClientRect().left;
    const stroke = context.stroke;
    const lineTo = Path2D.prototype.lineTo;
    const peaks = new WeakMap();
    let peakX = -Infinity;
    let inSync = 0;
    let moved = 0;
    const centre = () => {
      const { left, width } = element.getBoundingClientRect();
      return left + width / 2 - canvasLeft;
    };
    let previousX = centre();
    Path2D.prototype.lineTo = function (x, y) {
      if (y < (peaks.get(this)?.y ?? Infinity)) peaks.set(this, { x, y });
      return lineTo.call(this, x, y);
    };
    context.stroke = function (path) {
      if (this.lineWidth === 2.8) peakX = peaks.get(path)?.x ?? peakX;
      return path ? stroke.call(this, path) : stroke.call(this);
    };
    return new Promise(resolve => {
      let frame = 0;
      function move() {
        const nodeX = centre();
        if (nodeX !== previousX) {
          moved++;
          if (Math.abs(peakX - nodeX) < 2) inSync++;
        }
        previousX = nodeX;
        if (++frame > 18) {
          context.stroke = stroke;
          Path2D.prototype.lineTo = lineTo;
          resolve({ moved, inSync });
          return;
        }
        element.dispatchEvent(new PointerEvent('pointermove', {
          bubbles: true, pointerId: Number(element.dataset.testPointer), pointerType: 'mouse', buttons: 1,
          clientX: x + frame * 5, clientY: y - frame,
        }));
        requestAnimationFrame(move);
      }
      requestAnimationFrame(move);
    });
  }, { x, y });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  expect(frames.moved).toBeGreaterThan(12);
  expect(frames.inSync).toBeGreaterThanOrEqual(frames.moved - 1);
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0].hz).toBe(800);
});

test('a band can target one channel and use a dynamic cut, then change shape in the same slot', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await graph.getByRole('button', { name: 'Add EQ band on the graph' }).press('Enter');
  const bell = graph.getByRole('button', { name: 'Bell 1', exact: true });
  await expect(bell).toBeFocused();
  const bandId = (await settingsOf(page)).eqBands[0].id;
  await bell.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Change channel' }).hover();
  await page.getByRole('menuitemradio', { name: 'Right only' }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.channel).toBe('right');
  await bell.press('Shift+F10');
  await page.getByRole('menuitemcheckbox', { name: 'Dynamic cut' }).click();
  await page.setViewportSize({ width: 420, height: 320 });
  await page.getByRole('button', { name: /^Dynamic range,/ }).press('Enter');
  const range = page.getByRole('textbox', { name: 'Dynamic range in dB' });
  await range.fill('-7.4');
  await range.press('Enter');
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]?.dynamic).toEqual({ thresholdDb: -24, amountDb: 7.4 });
  await bell.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Change shape' }).hover();
  await page.getByRole('menuitem', { name: 'High shelf', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]).toMatchObject({ id: bandId, kind: 'treble', channel: 'right' });
  await expect(range).toHaveCount(0);
});

test('Centre and Sides targeting keeps its graph separate from Left and Right and solos the selected channel', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const curves = graph.getByRole('group', { name: 'Channel curves' });
  await page.evaluate(() => window.eqFixture.setBands([
    { id: 1, kind: 'bell', hz: 550, gainDb: 6, q: 140, channel: 'left' },
    { id: 2, kind: 'bell', hz: 2050, gainDb: -8, q: 140 },
  ]));
  const bell = graph.getByRole('button', { name: 'Bell 2', exact: true });
  await bell.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Change channel' }).hover();
  await page.getByRole('menuitemradio', { name: 'Centre', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[1].channel).toBe('mid');
  await bell.click();
  const channels = curves.getByRole('combobox', { name: 'EQ response channels' });
  await expect(channels).toHaveText('Centre / Sides');
  await expect(curves.locator('span.secondary')).toHaveText('Sides');
  await channels.click();
  await page.getByRole('option', { name: 'Left / Right' }).click();
  await expect(curves.locator('span.secondary')).toHaveText('Right');
  await bell.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Change channel' }).hover();
  await page.getByRole('menuitemradio', { name: 'Sides', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[1].channel).toBe('side');
  await bell.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Solo band', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.eqFixture.requests.find(request => request.action === 'solo-band')?.band.channel)).toBe('side');
});

test('spectral dynamics uses its own toggle on eligible bands when available', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([{
    id: 1, kind: 'bell', hz: 1500, gainDb: 1, q: 200, dynamic: { thresholdDb: -28, amountDb: 4, ratio: 3 },
  }]));
  await graph.getByRole('button', { name: 'Bell 1', exact: true }).click();
  const spectral = graph.getByRole('button', { name: 'Spectral dynamics', exact: true });
  await expect(spectral).toHaveCount(0);
  await page.evaluate(() => window.eqFixture.probe([], [], { spectralEq: true }));
  await spectral.click();
  await expect(spectral).toHaveAttribute('aria-pressed', 'true');
  await graph.getByRole('button', { name: /^Dynamic range,/ }).press('Enter');
  const range = graph.getByRole('textbox', { name: 'Dynamic range in dB' });
  await range.fill('-6');
  await range.press('Enter');
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0].dynamic).toEqual({
    thresholdDb: -28, amountDb: 6, ratio: 3, spectral: true,
  });
  await spectral.press('Enter');
  await expect(spectral).toHaveAttribute('aria-pressed', 'false');
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0].dynamic).toEqual({
    thresholdDb: -28, amountDb: 6, ratio: 3,
  });
});

test('a marquee moves multiple EQ bands in parallel as one cancellable gesture', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([
    { id: 1, kind: 'bell', hz: 300, gainDb: 4, q: 80 },
    { id: 2, kind: 'bell', hz: 1000, gainDb: -2, q: 140 },
    { id: 3, kind: 'bell', hz: 8000, gainDb: 0, q: 140 },
  ]));
  const first = graph.getByRole('button', { name: 'Bell 1', exact: true });
  const second = graph.getByRole('button', { name: 'Bell 2', exact: true });
  const third = graph.getByRole('button', { name: 'Bell 3', exact: true });
  const a = await first.boundingBox();
  const b = await second.boundingBox();
  await page.mouse.move(a.x - 24, a.y - 24);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width + 24, b.y + b.height + 24, { steps: 8 });
  await expect(graph.locator('.selection-box')).toBeVisible();
  await page.mouse.up();
  await expect(graph.locator('.selection-box')).toHaveCount(0);
  await expect(first).toHaveAttribute('aria-pressed', 'true');
  await expect(second).toHaveAttribute('aria-pressed', 'true');
  await expect(third).toHaveAttribute('aria-pressed', 'false');
  expect((await settingsOf(page)).eqBands).toHaveLength(3);

  const x = b.x + b.width / 2;
  const y = b.y + b.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 55, y + 30, { steps: 6 });
  const bands = async () => (await settingsOf(page)).eqBands;
  await expect.poll(async () => (await bands())[1].hz).toBeGreaterThan(1200);
  const [movedA, movedB, untouched] = await bands();
  expect(movedA.hz / 300).toBeCloseTo(movedB.hz / 1000, 2);
  expect(movedA.gainDb).toBeGreaterThan(4);
  expect(movedB.gainDb).toBeLessThan(-2);
  expect(movedA.gainDb / 4).toBeCloseTo(movedB.gainDb / -2, 1);
  expect(untouched.hz).toBe(8000);
  const edits = await page.evaluate(() => window.eqFixture.requests.filter(request => request.action === 'edit'));
  expect(new Set(edits.map(edit => edit.gesture)).size).toBe(1);
  expect(edits.at(-1).patch.eqBands.slice(0, 2).map(band => band.hz)).toEqual([movedA.hz, movedB.hz]);
  const plot = await graph.boundingBox();
  await page.mouse.move(x + 55, plot.y + plot.height / 2, { steps: 5 });
  await expect.poll(async () => (await bands()).slice(0, 2).map(band => Math.abs(band.gainDb) <= 0.2)).toEqual([true, true]);
  await page.mouse.move(x + 55, y + 30, { steps: 5 });
  await expect.poll(async () => (await bands()).slice(0, 2).map(band => band.gainDb)).toEqual([movedA.gainDb, movedB.gainDb]);
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect.poll(async () => (await bands()).map(band => band.hz)).toEqual([300, 1000, 8000]);
  await expect(first).toHaveAttribute('aria-pressed', 'true');
  await expect(second).toHaveAttribute('aria-pressed', 'true');
  await third.click({ modifiers: ['Control'] });
  await expect(third).toHaveAttribute('aria-pressed', 'true');
  await expect(first).toHaveAttribute('aria-pressed', 'true');
  const power = graph.getByRole('button', { name: 'Bypass selected EQ bands' });
  const editor = graph.getByRole('group', { name: 'Bell 3 settings' });
  const panel = await editor.boundingBox();
  const powerBox = await power.boundingBox();
  expect(powerBox.x - panel.x).toBeCloseTo(4, 0);
  expect(powerBox.y - panel.y).toBeCloseTo(4, 0);
  await power.click();
  expect((await bands()).every(band => band.bypassed)).toBe(true);
  await graph.getByRole('button', { name: 'Enable selected EQ bands' }).click();
  expect((await bands()).every(band => !band.bypassed)).toBe(true);
});

test('group actions preserve unselected bands and send one edit for each operation', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([
    { kind: 'bell', hz: 200, gainDb: 4, q: 80 },
    { kind: 'bell', hz: 900, gainDb: -2, q: 140 },
    { id: 20, kind: 'bell', hz: 6000, gainDb: 1, q: 80 },
  ]));
  const marker = number => graph.getByRole('button', { name: 'Bell ' + number, exact: true });
  const patches = () => page.evaluate(() => window.eqFixture.requests.filter(request => request.action === 'edit'));
  const frequencies = async () => (await settingsOf(page)).eqBands?.map(band => band.hz);
  await marker(1).click();
  await marker(2).click({ modifiers: ['Control'] });
  await page.mouse.move(0, 0);
  await expect(marker(3).locator('..')).toHaveCSS('opacity', '0.6');
  await expect(marker(1).locator('..')).toHaveCSS('opacity', '1');
  await marker(1).click({ button: 'right' });
  await expect(marker(2)).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('menuitem', { name: 'Copy selected bands' }).click();
  expect(await patches()).toHaveLength(0);
  await page.keyboard.press('Delete');
  await expect.poll(frequencies).toEqual([6000]);
  expect(await patches()).toHaveLength(1);

  const hit = graph.getByRole('button', { name: 'Add EQ band on the graph' });
  await expect(hit).toBeFocused();
  await hit.press('Control+v');
  await expect.poll(frequencies).toEqual([6000, 200, 900]);
  await expect(marker(2)).toBeFocused();
  await page.keyboard.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Duplicate selected bands' }).click();
  await expect.poll(frequencies).toEqual([6000, 200, 900, 200, 900]);
  await expect(marker(4)).toBeFocused();
  await page.keyboard.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Remove selected bands' }).click();
  await expect.poll(frequencies).toEqual([6000, 200, 900]);
  expect(await patches()).toHaveLength(4);
});

test('the node-side dynamic cut and outer dial adjust compression independently of gain and frequency', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([{ id: 1, kind: 'bell', hz: 750, gainDb: 3, q: 130 }]));
  const node = graph.getByRole('button', { name: 'Bell 1', exact: true });
  await node.click();
  const grip = graph.getByRole('button', { name: 'Dynamic EQ for Bell 1' });
  const nodeBox = await node.boundingBox();
  const gripBox = await grip.boundingBox();
  expect(gripBox.x + gripBox.width).toBeLessThan(nodeBox.x + 2);
  await grip.click();
  const band = async () => (await settingsOf(page)).eqBands[0];
  await expect.poll(band).toMatchObject({ hz: 750, gainDb: 3, dynamic: { amountDb: 4 } });
  await expect(grip).toHaveCount(0);
  const handle = graph.getByRole('button', { name: 'Dynamic range for Bell 1' });
  await expect(handle).toBeVisible();
  expect((await handle.boundingBox()).y).toBeGreaterThan(nodeBox.y);
  expect(await node.boundingBox()).toEqual(nodeBox);
  const handleBox = await handle.boundingBox();
  const handleX = handleBox.x + handleBox.width / 2;
  const handleY = handleBox.y + handleBox.height / 2;
  await page.mouse.move(handleX, handleY);
  await page.mouse.down();
  await page.mouse.move(handleX, handleY + 28, { steps: 4 });
  await page.mouse.up();
  const handled = await band();
  expect(handled.dynamic.amountDb).toBeGreaterThan(4);
  expect(handled).toMatchObject({ hz: 750, gainDb: 3 });

  const range = page.getByRole('button', { name: /^Dynamic range,/ });
  const gain = page.getByRole('button', { name: /^Gain,/ });
  const ring = await range.boundingBox();
  const face = await gain.locator('.dial-face').boundingBox();
  expect(ring.width).toBeGreaterThan(face.width);
  expect(ring.x + ring.width / 2).toBeCloseTo(face.x + face.width / 2, 0);
  const x = ring.x + ring.width - 3;
  const y = ring.y + ring.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + 35, { steps: 5 });
  await page.mouse.up();
  expect((await band()).dynamic.amountDb).toBeGreaterThan(handled.dynamic.amountDb);
  expect(await band()).toMatchObject({ hz: 750, gainDb: 3 });
  const amount = (await band()).dynamic.amountDb;

  await page.mouse.move(face.x + face.width / 2, face.y + face.height / 2);
  await page.mouse.down();
  await page.mouse.move(face.x + face.width / 2, face.y + face.height / 2 - 12, { steps: 4 });
  await page.mouse.up();
  expect((await band()).gainDb).toBeGreaterThan(3);
  expect((await band()).dynamic.amountDb).toBe(amount);
  await handle.press('Home');
  expect((await band()).dynamic).toBeUndefined();
  await expect(handle).toHaveCount(0);
  await expect(grip).toBeVisible();
  await page.evaluate(() => window.eqFixture.probe([], [], { upwardEq: true }));
  await range.press('Enter');
  const value = page.getByRole('textbox', { name: 'Dynamic range in dB' });
  await value.fill('5.5');
  await value.press('Enter');
  await expect.poll(band).toMatchObject({ hz: 750, dynamic: { amountDb: -5.5 } });
  const moved = await node.boundingBox();
  await page.mouse.move(moved.x + moved.width / 2, moved.y + moved.height / 2);
  await page.mouse.down();
  await page.mouse.move(moved.x + moved.width / 2 + 35, moved.y + moved.height / 2, { steps: 4 });
  await page.mouse.up();
  expect((await band()).hz).toBeGreaterThan(750);
  expect((await band()).dynamic.amountDb).toBe(-5.5);
});

test('the dynamic handle pans and changes width during a captured drag, and ratio edits stay independent', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => {
    window.eqFixture.setBands([{ id: 1, kind: 'bell', hz: 750, gainDb: 3, q: 130,
      dynamic: { thresholdDb: -24, amountDb: 6 } }]);
    window.eqFixture.probe([], [], { dynamicRatio: true });
  });
  const handle = graph.getByRole('button', { name: 'Dynamic range for Bell 1' });
  const box = await handle.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await expect(graph.locator('.range-guide')).toHaveCount(0);
  const arrowOpacity = () => handle.evaluate(button => ['::before', '::after'].map(arrow => getComputedStyle(button, arrow).opacity));
  await graph.getByRole('button', { name: 'Bell 1', exact: true }).hover();
  await expect.poll(arrowOpacity).toEqual(['0', '0']);
  await page.mouse.move(x, y);
  await expect.poll(arrowOpacity).toEqual(['1', '1']);
  await page.mouse.down();
  await page.mouse.move(x + 35, y);
  await page.mouse.wheel(0, -100);
  await page.mouse.up();
  const band = async () => (await settingsOf(page)).eqBands[0];
  await expect.poll(async () => (await band()).hz).toBeGreaterThan(750);
  expect(await band()).toMatchObject({ q: 140, gainDb: 3, dynamic: { amountDb: 6 } });
  const dial = page.getByRole('button', { name: /^Ratio,/ });
  await dial.press('Enter');
  const ratio = page.getByRole('textbox', { name: /^Ratio/ });
  await ratio.fill('4.2');
  await ratio.press('Enter');
  await expect.poll(band).toMatchObject({ dynamic: { ratio: 4.2, amountDb: 6, thresholdDb: -24 } });
});

test('every EQ shape can switch dynamic direction and retains its other settings', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.probe([], [], { upwardEq: true, dynamicAllShapes: true }));
  const shapes = [
    { kind: 'low', gainDb: 0, q: 80 }, { kind: 'bass', gainDb: -4, q: 80 },
    { kind: 'bell', gainDb: 3, q: 120 }, { kind: 'notch', gainDb: 0, q: 800 },
    { kind: 'treble', gainDb: 4, q: 80 }, { kind: 'high', gainDb: 0, q: 80 },
  ];
  const range = graph.getByRole('button', { name: /^Dynamic range,/ });
  const field = graph.getByRole('textbox', { name: 'Dynamic range in dB' });
  for (const [index, shape] of shapes.entries()) {
    const staticBand = { id: index + 1, hz: 2000, ...shape };
    await page.evaluate(band => window.eqFixture.setBands([band]), staticBand);
    await graph.locator('.marker').click();
    await expect(graph.locator('.dynamics-grip')).toBeVisible();
    for (const gain of [-6, 5.5]) {
      await range.press('Enter');
      await field.fill(String(gain));
      await field.press('Enter');
      await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]).toMatchObject({
        ...staticBand, dynamic: { amountDb: -gain, thresholdDb: -24 },
      });
    }
  }
  await graph.locator('.marker').press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Change shape' }).hover();
  await page.getByRole('menuitem', { name: 'Notch', exact: true }).click();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]).toMatchObject({
    kind: 'notch', dynamic: { amountDb: -5.5, thresholdDb: -24 },
  });
});

test('one click near the centred zero line adds the previewed bell, shelf or cut', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const hit = graph.getByRole('button', { name: 'Add EQ band on the graph' });
  const size = await hit.boundingBox();
  const ghost = graph.locator('.ghost');
  const image = () => graph.locator('canvas').evaluate(canvas => {
    const scale = canvas.width / canvas.clientWidth;
    const side = Math.ceil((canvas.clientWidth < 460 ? 34 : 43) * scale);
    const top = Math.ceil(25 * scale);
    const pixels = canvas.getContext('2d').getImageData(side, top, canvas.width - side * 2, canvas.height - top * 2).data;
    let hash = 0;
    for (const value of pixels) hash = Math.imul(hash, 31) + value | 0;
    return hash;
  });
  const centre = Math.round(size.height / 2);
  const left = (size.width < 460 ? 34 : 43) + 6;
  const middle = Math.round(size.width / 2);
  const right = Math.round(size.width - left);
  await hit.click({ position: { x: middle, y: centre - 55 } });
  await expect(ghost).toHaveCount(0);
  expect((await settingsOf(page)).eqBands).toBeNull();
  const blank = await image();

  await page.mouse.move(size.x + middle, size.y + centre);
  await expect(ghost).toHaveCount(1);
  await expect(ghost).toHaveText('');
  await expect.poll(image).not.toBe(blank);
  const dot = await ghost.boundingBox();
  expect(dot.y + dot.height / 2 - size.y).toBeCloseTo(size.height / 2, 0);
  const opacity = await ghost.evaluate(element => {
    const canvas = document.createElement('canvas');
    const ctx = canvas.getContext('2d');
    ctx.fillStyle = getComputedStyle(element).backgroundColor;
    ctx.fillRect(0, 0, 1, 1);
    return ctx.getImageData(0, 0, 1, 1).data[3];
  });
  expect(opacity).toBeGreaterThan(0);
  expect(opacity).toBeLessThan(160);
  await page.getByRole('button', { name: 'Play', exact: true }).hover();
  await expect(ghost).toHaveCount(0);
  await expect.poll(image).toBe(blank);

  const cases = [
    { x: middle, y: centre - 16, kind: 'bell', sign: 1 },
    { x: middle, y: centre + 16, kind: 'bell', sign: -1 },
    { x: left, y: centre, kind: 'low', sign: 0 },
    { x: left, y: centre - 16, kind: 'bass', sign: 1 },
    { x: right, y: centre, kind: 'high', sign: 0 },
    { x: right, y: centre + 16, kind: 'treble', sign: -1 },
  ];
  for (const { x, y, kind, sign } of cases) {
    await page.evaluate(() => window.eqFixture.setBands(null));
    await expect.poll(image).toBe(blank);
    await hit.hover({ position: { x, y } });
    await expect(ghost).toHaveText('');
    await expect.poll(image).not.toBe(blank);
    await hit.click({ position: { x, y } });
    const bands = (await settingsOf(page)).eqBands;
    expect(bands).toHaveLength(1);
    expect(bands[0].kind).toBe(kind);
    expect(Math.sign(bands[0].gainDb)).toBe(sign);
    await expect(ghost).toHaveCount(0);
  }
});

test('dial adjustments are fractional and Escape restores the starting value', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const hit = graph.getByRole('button', { name: 'Add EQ band on the graph' });
  await hit.focus();
  const frame = await page.getByRole('region', { name: 'Equaliser', exact: true }).evaluate(element => {
    const style = getComputedStyle(element, '::after');
    return { inset: style.top, width: style.borderTopWidth, radius: style.borderTopLeftRadius };
  });
  expect(frame).toEqual({ inset: '1px', width: '1px', radius: '22px' });
  await expect(hit).toHaveCSS('outline-style', 'none');
  expect(await hit.evaluate(button => getComputedStyle(button, '::after').content)).toBe('none');
  await hit.press('Enter');
  const bell = graph.getByRole('button', { name: 'Bell 1', exact: true });
  const controls = page.getByRole('group', { name: 'Bell 1 settings' });
  const ring = controls.locator('[data-eq-dial="Gain"] .dial-ring');
  const bounds = await ring.locator('.readout').boundingBox();
  const x = bounds.x + bounds.width / 2;
  const y = bounds.y + 3;
  const gain = async () => (await settingsOf(page)).eqBands?.[0]?.gainDb;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.keyboard.down('Shift');
  await page.mouse.move(x, y - 9, { steps: 4 });
  await expect.poll(gain).toBeGreaterThan(0);
  expect(await gain()).toBeLessThan(1);
  await page.keyboard.press('Escape');
  await page.keyboard.up('Shift');
  await page.mouse.up();
  await expect.poll(gain).toBe(0);
  await bell.press('Escape');
  await expect(controls).toHaveCount(0);
  await expect(hit).toBeFocused();
  await expect(bell).toHaveAttribute('aria-pressed', 'false');
});

test('the named preset switcher cycles EQ presets and keeps filter shapes accessible', async ({ page }) => {
  const eq = page.getByRole('region', { name: 'Equaliser', exact: true });
  const presetMenu = eq.getByRole('button', { name: /^EQ preset,/ });
  const previous = eq.getByRole('button', { name: 'Previous EQ preset' });
  const next = eq.getByRole('button', { name: 'Next EQ preset' });
  const pin = await eq.getByRole('button', { name: 'Keep tool on top' }).boundingBox();
  const left = await previous.boundingBox();
  const centre = await presetMenu.boundingBox();
  const right = await next.boundingBox();
  expect(left.x - (pin.x + pin.width)).toBeLessThan(12);
  expect(centre.x).toBeGreaterThan(left.x);
  expect(right.x).toBeGreaterThan(centre.x);
  expect(centre.y).toBeCloseTo(pin.y, 0);
  await expect(eq.getByRole('toolbar', { name: 'Tool modes' })).toHaveCount(0);
  await expect(presetMenu).toHaveText('Flat');
  await next.click();
  await expect(presetMenu).toHaveText('Reduce rumble');
  await next.click();
  await expect(presetMenu).toHaveText('Soften hiss');
  expect((await settingsOf(page)).rumble).toBe(false);
  expect((await settingsOf(page)).highCutHz).toBe(10000);
  await previous.click();
  await expect(presetMenu).toHaveText('Reduce rumble');
  expect((await settingsOf(page)).highCutHz).toBe(0);
  await presetMenu.click();
  const preset = page.getByRole('menuitem', { name: 'Reduce rumble', exact: true });
  const filterShape = page.getByRole('menuitem', { name: 'Filter shape', exact: true });
  await expect(preset).toHaveAttribute('aria-current', 'true');
  expect((await preset.boundingBox()).height).toBeGreaterThanOrEqual(32);
  await expect(page.getByRole('menuitemradio', { name: 'Bell', exact: true })).toHaveCount(0);
  await filterShape.press('ArrowRight');
  await page.getByRole('menuitemradio', { name: 'Bell', exact: true }).click();
  await expect(eq.getByRole('button', { name: 'Bell', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.setViewportSize({ width: 340, height: 350 });
  await page.evaluate(() => { document.documentElement.dataset.theme = 'light'; });
  await presetMenu.click();
  await filterShape.hover();
  await expect(page.getByRole('menu', { name: 'Filter shape', exact: true })).toBeInViewport({ ratio: 1 });
  await expect(page.getByRole('menuitemradio', { name: 'Bell', exact: true })).toHaveAttribute('aria-checked', 'true');
  await preset.click();
  await expect.poll(async () => (await settingsOf(page)).rumble).toBe(true);
});

test('the gain range reframes bands and the ruler zooms around the pointer, pans and resets', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([
    { id: 1, kind: 'bell', hz: 500, gainDb: 3, q: 80 },
    { id: 2, kind: 'bell', hz: 25, gainDb: 0, q: 80 },
  ]));
  const band = graph.getByRole('button', { name: 'Bell 1', exact: true });
  const outer = graph.getByRole('button', { name: 'Bell 2', exact: true });
  const ruler = graph.getByRole('button', { name: 'Zoom or pan frequencies' });
  const range = graph.getByRole('combobox', { name: 'EQ gain display range' });
  const origin = await band.boundingBox();
  await range.click();
  await page.getByRole('option', { name: '±6 dB', exact: true }).click();
  await expect(range).toHaveText('±6 dB');
  expect((await band.boundingBox()).y).toBeLessThan(origin.y - 30);
  expect((await settingsOf(page)).eqBands[0].gainDb).toBe(3);
  const box = await ruler.boundingBox();
  const anchorX = origin.x + origin.width / 2;
  const rulerY = box.y + box.height / 2;
  await page.mouse.move(anchorX, rulerY);
  await page.mouse.down();
  await page.mouse.move(anchorX, rulerY - 125, { steps: 6 });
  await page.mouse.up();
  await expect(outer).toHaveCount(0);
  expect(Math.abs((await band.boundingBox()).x - origin.x)).toBeLessThan(2);
  await page.mouse.move(anchorX, rulerY);
  await page.mouse.down();
  await page.mouse.move(anchorX + 64, rulerY, { steps: 5 });
  await page.mouse.up();
  expect((await band.boundingBox()).x).toBeGreaterThan(origin.x + 55);
  await ruler.dblclick();
  await expect(outer).toBeVisible();
  expect((await band.boundingBox()).x).toBeCloseTo(origin.x, 0);
  await expect(range).toHaveText('±6 dB');
});

test('frozen peaks can be cut by dragging and Escape removes the audition cut', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const peak = graph.getByRole('button', { name: 'Grab spectrum peak at 440 Hz' });
  await page.evaluate(() => {
    const levels = [-84, -63, -20, -63, -70];
    window.eqFixture.probe(levels, levels, { frequencies: [20, 220, 440, 880, 1500] });
  });
  await expect(peak).toHaveCount(0);
  await page.getByRole('button', { name: 'Freeze spectrum', exact: true }).click();
  const box = await peak.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + 24, { steps: 4 });
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]).toMatchObject({
    kind: 'bell', hz: 440, q: 1034, gainDb: -6,
  });
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect.poll(async () => (await settingsOf(page)).eqBands).toBeNull();
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + 16);
  await page.mouse.up();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]).toMatchObject({ hz: 440, gainDb: -4 });
  await expect(graph.getByRole('group', { name: 'Bell 1 settings' })).toBeVisible();
  await page.getByRole('button', { name: 'Resume spectrum', exact: true }).click();
  await expect(peak).toHaveCount(0);
  await page.evaluate(() => {
    const levels = [-84, -28, -20, -24, -70];
    window.eqFixture.probe(levels, levels, { frequencies: [20, 220, 440, 880, 1500] });
  });
  await page.getByRole('button', { name: 'Freeze spectrum', exact: true }).click();
  await peak.press('Enter');
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[1]).toMatchObject({ hz: 440, q: 110, gainDb: -3 });
});

test('hovering over the playing spectrum focuses its peaks until the cut is added', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const peak = graph.getByRole('button', { name: 'Grab spectrum peak at 440 Hz' });
  await page.evaluate(() => {
    const levels = [-84, -63, -20, -63, -70];
    window.eqFixture.setPlaying(true);
    window.eqFixture.probe(levels, levels, { frequencies: [20, 220, 440, 880, 1500] });
  });
  const box = await graph.boundingBox();
  const inset = box.width < 460 ? 34 : 43;
  const x = box.x + inset + Math.log(440 / 20) / Math.log(1000) * (box.width - 2 * inset);
  const y = box.y + box.height - 25 - (76 / 96) * (box.height - 50);
  await page.mouse.move(x, y);
  await expect(graph).toHaveClass(/spectrum-grab/);
  await expect(peak).toBeVisible();
  await page.mouse.move(0, 0);
  await expect(graph).not.toHaveClass(/spectrum-grab/);
  await page.mouse.move(x, y);
  await expect(peak).toBeVisible();
  await page.mouse.down();
  await page.mouse.move(x, y + 20, { steps: 4 });
  await page.mouse.up();
  await expect.poll(async () => (await settingsOf(page)).eqBands?.[0]).toMatchObject({ hz: 440, gainDb: -5 });
  await expect(graph).not.toHaveClass(/spectrum-grab/);
  await expect(peak).toHaveCount(0);
});

test('grabbing an after-only peak works while auditioning Original', async ({ page }) => {
  const select = page.getByRole('combobox', { name: 'Spectrum display' });
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await select.click();
  await page.getByRole('option', { name: 'After', exact: true }).click();
  await page.evaluate(() => {
    window.eqFixture.setOriginal(true);
    window.eqFixture.probe([], [-84, -63, -20, -63, -70], { frequencies: [20, 220, 440, 880, 1500] });
  });
  await page.getByRole('button', { name: 'Freeze spectrum' }).click();
  await expect(graph.getByRole('button', { name: 'Grab spectrum peak at 440 Hz' })).toBeVisible();
  await select.click();
  await page.getByRole('option', { name: 'Before', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Freeze spectrum' })).toBeDisabled();
  await expect.poll(() => page.evaluate(() => window.eqFixture.requests.findLast(request => request.action === 'analyzer')?.view)).toBe('before');
});

test('the EQ A/B toggle remembers both edited curves and keeps the selected one when ended', async ({ page }) => {
  const preset = page.getByRole('button', { name: /^EQ preset,/ });
  const next = page.getByRole('button', { name: 'Next EQ preset' });
  await next.click();
  await page.getByRole('button', { name: 'Start EQ A/B comparison' }).click();
  await next.click();
  await expect(preset).toHaveText('Soften hiss');
  await page.getByRole('button', { name: 'Switch to EQ A' }).click();
  await expect(preset).toHaveText('Reduce rumble');
  await page.getByRole('button', { name: 'Switch to EQ B' }).click();
  await expect(preset).toHaveText('Soften hiss');
  const lastComparison = () => page.evaluate(() => window.eqFixture.requests.findLast(request => request.compareEq !== undefined));
  expect(await lastComparison()).toMatchObject({ patch: { highCutHz: 10000 }, compareEq: { rumble: true } });
  await preset.click();
  await page.getByRole('menuitem', { name: 'End A/B comparison' }).click();
  await expect(page.getByRole('button', { name: 'Start EQ A/B comparison' })).toBeVisible();
  await expect.poll(lastComparison).toMatchObject({ compareEq: null });
  expect((await settingsOf(page)).highCutHz).toBe(10000);
});

test('each EQ response has its own coloured wash that fades toward zero', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([
    { id: 1, kind: 'bell', hz: 375, gainDb: 12, q: 140 },
    { id: 2, kind: 'bell', hz: 2350, gainDb: -12, q: 140 },
  ]));
  const samples = () => graph.locator('canvas').evaluate(canvas => {
    const ctx = canvas.getContext('2d');
    const { clientWidth: width, clientHeight: height } = canvas;
    const scale = canvas.width / width;
    const inset = width < 460 ? 34 : 43;
    const pixel = (hz, db) => {
      const x = inset + Math.log(hz / 20) / Math.log(1000) * (width - 2 * inset);
      const y = height - 25 - (db + 24) / 48 * (height - 50);
      const rows = Array.from({ length: 3 }, (_, index) =>
        Array.from(ctx.getImageData(Math.round(x * scale), Math.round(y * scale) + index - 1, 1, 1).data));
      return rows.reduce((strongest, pixel) => pixel[3] > strongest[3] ? pixel : strongest);
    };
    return [pixel(375, 8), pixel(2350, -8), pixel(375, 1), pixel(2350, -1)];
  });
  await expect.poll(async () => (await samples()).slice(0, 2).every(pixel => pixel[3] >= 39 && pixel[3] <= 50)).toBe(true);
  const [boost, cut, fadedBoost, fadedCut] = await samples();
  expect(boost.slice(0, 3)).not.toEqual(cut.slice(0, 3));
  expect(boost[3]).toBeGreaterThan(fadedBoost[3] + 15);
  expect(cut[3]).toBeGreaterThan(fadedCut[3] + 15);
});

test('the EQ line reacts to live dynamic reduction with the spectrum off', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([{
    id: 1, kind: 'bell', hz: 1500, gainDb: 0, q: 250, dynamic: { thresholdDb: -24, amountDb: 12 },
  }]));
  await page.getByRole('combobox', { name: 'Spectrum display' }).click();
  await page.getByRole('option', { name: 'Off', exact: true }).click();
  await page.evaluate(() => window.eqFixture.setPlaying(true));
  const curvePixels = () => graph.locator('canvas').evaluate(canvas => {
    const ctx = canvas.getContext('2d');
    const { clientWidth: width, clientHeight: height } = canvas;
    const scale = canvas.width / width;
    const inset = width < 460 ? 34 : 43;
    const colorAt = (hz, db) => {
      const x = inset + Math.log(hz / 20) / Math.log(1000) * (width - 2 * inset);
      const y = height - 25 - (db + 24) / 48 * (height - 50);
      const rows = Array.from({ length: 3 }, (_, index) =>
        Array.from(ctx.getImageData(Math.round(x * scale), Math.round(y * scale) + index - 1, 1, 1).data));
      return rows.reduce((strongest, pixel) => pixel[3] > strongest[3] ? pixel : strongest);
    };
    return { far: colorAt(40, 0), center: colorAt(1500, 0), cut: colorAt(1500, -12) };
  });
  const before = await curvePixels();
  expect(before.center).toEqual(before.far);
  await page.evaluate(() => window.eqFixture.probe([], [], { dynamicEqMix: [1] }));
  await expect.poll(async () => (await curvePixels()).cut).toEqual(before.center);
  const active = await curvePixels();
  expect(active.far).toEqual(before.far);
  expect(active.center).not.toEqual(before.center);
  await page.evaluate(() => window.eqFixture.probe([], [], { dynamicEqMix: [0] }));
  await expect.poll(curvePixels).toEqual(before);
});

test('spectral dynamics put separate frequency cuts on the combined curve', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const canvas = graph.locator('canvas');
  const curveAt = (hz, db) => canvas.evaluate((element, { hz, db }) => {
    const { clientWidth: width, clientHeight: height } = element;
    const scale = element.width / width;
    const inset = width < 460 ? 34 : 43;
    const x = Math.round((inset + Math.log(hz / 20) / Math.log(1000) * (width - 2 * inset)) * scale);
    const y = Math.round((height - 25 - (db + 24) / 48 * (height - 50)) * scale);
    const rgba = element.getContext('2d').getImageData(x, y - 2, 1, 5).data;
    return Math.max(rgba[3], rgba[7], rgba[11], rgba[15], rgba[19]);
  }, { hz, db });
  await page.evaluate(() => {
    window.eqFixture.setBands([{
      id: 1, kind: 'bell', hz: 1000, gainDb: 0, q: 80,
      dynamic: { thresholdDb: -30, amountDb: 8, ratio: 4, spectral: true },
    }]);
    window.eqFixture.setPlaying(true);
    window.eqFixture.probe([], [], {
      dynamicEqMix: [1], spectralResponse: [{
        channel: 'stereo', firstBin: 8, binHz: 100, gainsDb: [0, -6, 0, -4, 0],
      }],
    });
  });
  await expect.poll(() => curveAt(900, -6)).toBeGreaterThan(240);
  await expect.poll(() => curveAt(1000, 0)).toBeGreaterThan(240);
  await expect.poll(() => curveAt(1100, -4)).toBeGreaterThan(240);
});

test('the spectrum slopes through repeated bass readings and preserves the peak', async ({ page }) => {
  const canvas = page.getByRole('group', { name: 'Frequency response and spectrum' }).locator('canvas');
  await page.getByRole('combobox', { name: 'Spectrum display' }).click();
  await page.getByRole('option', { name: 'Before', exact: true }).click();
  const columns = () => canvas.evaluate(canvas => {
    const ctx = canvas.getContext('2d');
    const { clientWidth: width, clientHeight: height } = canvas;
    const scale = canvas.width / width;
    const inset = width < 460 ? 34 : 43;
    const spectra = [44, 76, Math.sqrt(120 * 180)].map(hz => {
      const x = inset + Math.log(hz / 20) / Math.log(1000) * (width - 2 * inset);
      const pixels = ctx.getImageData(Math.round(x * scale), 0, 1, canvas.height).data;
      return Array.from({ length: canvas.height }, (_, y) => pixels[y * 4 + 3]);
    });
    return { spectra, height, scale };
  });
  const baseline = (await columns()).spectra;
  await page.evaluate(() => {
    window.eqFixture.setPlaying(true);
    const levels = [-80, -80, -60, -60, -60, -28, -28, -68];
    window.eqFixture.probe(levels, levels, { frequencies: [20, 30, 40, 60, 90, 120, 180, 300] });
  });
  await expect.poll(async () => {
    const { spectra, height, scale } = await columns();
    const [low, high, peak] = spectra.map((pixels, column) => {
      let alpha = 0;
      let peak = -1;
      for (let y = 0; y < pixels.length; y++) {
        const strength = pixels[y] - baseline[column][y];
        if (strength <= alpha) continue;
        alpha = strength;
        peak = y;
      }
      return peak < 0 ? NaN : -96 + (height - 25 - peak / scale) / (height - 50) * 96;
    });
    return high - low > 7 && Math.abs(peak + 28) < 1.5;
  }).toBe(true);
});

test('the graph context menu closes on Escape, an outside click, or window blur', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  const hit = graph.getByRole('button', { name: 'Add EQ band on the graph' });
  const context = page.getByRole('menu').filter({ has: page.getByRole('menuitem', { name: 'Add low cut' }) });
  await hit.click({ button: 'right', position: { x: 200, y: 55 } });
  await expect(context).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(context).toBeHidden();
  await expect(hit).toBeFocused();
  await hit.click({ button: 'right', position: { x: 200, y: 55 } });
  await expect(context).toBeVisible();
  await page.mouse.click(62, 42);
  await expect(context).toBeHidden();
  await hit.click({ button: 'right', position: { x: 200, y: 55 } });
  await expect(context).toBeVisible();
  await page.evaluate(() => window.dispatchEvent(new FocusEvent('blur')));
  await expect(context).toBeHidden();
  await hit.click({ button: 'right', position: { x: 200, y: 55 } });
  await context.getByRole('menuitem', { name: 'Add bell' }).click();
  await expect(context).toBeHidden();
  await expect(graph.getByRole('button', { name: 'Bell 1', exact: true })).toBeVisible();
});

test('overlapping nodes cycle on click and an axis-locked drag shows its temporary readout', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([1, 2].map(id => ({
    id, kind: 'bell', hz: 1000, gainDb: 5, q: 130,
  }))));
  const first = graph.getByRole('button', { name: 'Bell 1', exact: true });
  const second = graph.getByRole('button', { name: 'Bell 2', exact: true });
  const box = await first.boundingBox();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  const selected = graph.locator('.marker[aria-pressed=true]');
  await page.mouse.click(x, y);
  const initial = await selected.getAttribute('aria-label');
  await page.mouse.click(x, y);
  const secondClick = await selected.getAttribute('aria-label');
  expect(secondClick).not.toBe(initial);
  await page.keyboard.down('Alt');
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 55, y + 17, { steps: 4 });
  await expect(graph.locator('.drag-readout')).toContainText('Hz');
  await page.mouse.up();
  await page.keyboard.up('Alt');
  await expect(graph.locator('.drag-readout')).toHaveCount(0);
  const bands = (await settingsOf(page)).eqBands;
  expect(bands.map(band => band.gainDb)).toEqual([5, 5]);
  expect(bands[secondClick === 'Bell 1' ? 0 : 1].hz).toBeGreaterThan(1000);
  await expect(first).toBeVisible();
  await expect(second).toBeVisible();
});

test('the threshold shows the band detector and the output meter latches clipping and trims independently', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => {
    window.eqFixture.setBands([{ id: 1, kind: 'bell', hz: 1200, gainDb: 8, q: 150, dynamic: { thresholdDb: -24, amountDb: 4 } }]);
    window.eqFixture.setPlaying(true);
    window.eqFixture.probe([], [], { outputPeak: -1.1, outputOverload: true, dynamicEqDetectors: [-18] });
  });
  await graph.getByRole('button', { name: 'Bell 1', exact: true }).click();
  await expect(graph.locator('[data-eq-dial=Threshold] .detector-mark.detected')).toBeVisible();
  const clear = page.getByRole('button', { name: 'Clear output clipping indicator' });
  await expect(clear).toBeVisible();
  await page.evaluate(() => window.eqFixture.probe([], [], { inputPeak: -2, eqInputPeak: -17, outputPeak: -8, outputOverload: false, dynamicEqDetectors: [-36] }));
  await expect(graph.locator('[data-eq-dial=Threshold] .detector-mark.detected')).toHaveCount(0);
  await expect(page.getByRole('meter', { name: 'Input peak level' })).toHaveAttribute('aria-valuenow', '-17');
  await expect(page.locator('.peak-hold')).toHaveCount(1);
  await clear.click();
  await expect(clear).toHaveCount(0);
  await page.getByRole('button', { name: 'Gain strength', exact: true }).click();
  const popup = page.getByRole('dialog', { name: 'Gain strength' });
  const input = popup.getByRole('button', { name: /^Input,/ });
  const strength = popup.getByRole('button', { name: /^Strength,/ });
  const output = popup.getByRole('button', { name: /^Output,/ });
  const positions = await Promise.all([input, strength, output].map(dial => dial.boundingBox()));
  expect(positions.map(box => box.y)).toEqual(Array(3).fill(positions[0].y));
  expect(positions[0].x).toBeLessThan(positions[1].x);
  expect(positions[1].x).toBeLessThan(positions[2].x);
  await input.press('Enter');
  const inputGain = popup.getByRole('textbox', { name: 'Input in dB' });
  await inputGain.fill('3.2');
  await inputGain.press('Enter');
  await expect.poll(() => settingsOf(page)).toMatchObject({ eqInputDb: 3.2, eqOutputDb: 0 });
  await output.press('Enter');
  const trim = page.getByRole('textbox', { name: 'Output in dB' });
  await trim.fill('-6.3');
  await trim.press('Enter');
  await expect.poll(() => settingsOf(page)).toMatchObject({ eqInputDb: 3.2, eqOutputDb: -6.3, eqBands: [{ gainDb: 8 }] });
  await output.dblclick();
  await expect.poll(() => settingsOf(page)).toMatchObject({ eqOutputDb: 0, eqBands: [{ gainDb: 8 }] });
});

test('EQ strength scales plotted bells and shelves while preserving their values and output trim', async ({ page }) => {
  const canvas = page.getByRole('group', { name: 'Frequency response and spectrum' }).locator('canvas');
  const curveAt = (hz, db) => canvas.evaluate((element, { hz, db }) => {
    const ctx = element.getContext('2d');
    const { clientWidth: width, clientHeight: height } = element;
    const scale = element.width / width;
    const inset = width < 460 ? 34 : 43;
    const x = Math.round((inset + Math.log(hz / 20) / Math.log(1000) * (width - 2 * inset)) * scale);
    const y = Math.round((height - 25 - (db + 24) / 48 * (height - 50)) * scale);
    const rgba = ctx.getImageData(x, y - 2, 1, 5).data;
    return Math.max(rgba[3], rgba[7], rgba[11], rgba[15], rgba[19]);
  }, { hz, db });
  await page.evaluate(() => window.eqFixture.setSettings({
    presenceHz: 330, presenceDb: -10, eqOutputDb: -4, rumble: true, rumbleHz: 80, rumbleSlope: 48,
    eqBands: [{ id: 1, kind: 'treble', hz: 3000, gainDb: 12, q: 140, dynamic: { thresholdDb: -24, amountDb: -4 } }],
  }));
  await expect.poll(() => curveAt(330, -10)).toBeGreaterThan(240);
  await expect.poll(() => curveAt(12000, 12)).toBeGreaterThan(240);
  await page.getByRole('button', { name: 'Gain strength', exact: true }).click();
  const strength = page.getByRole('dialog', { name: 'Gain strength' }).getByRole('button', { name: /^Strength,/ });
  const dial = await strength.boundingBox();
  const x = dial.x + dial.width / 2;
  const y = dial.y + dial.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + 85, { steps: 5 });
  await expect.poll(() => settingsOf(page)).toMatchObject({
    eqStrength: 50, presenceDb: -10, eqOutputDb: -4, rumbleSlope: 48, eqBands: [{ gainDb: 12, dynamic: { amountDb: -4 } }],
  });
  await expect.poll(() => curveAt(330, -5)).toBeGreaterThan(240);
  await expect.poll(() => curveAt(12000, 6)).toBeGreaterThan(240);
  await page.mouse.up();
  await strength.dblclick();
  await expect.poll(() => settingsOf(page)).toMatchObject({ eqStrength: 100, eqOutputDb: -4 });
  await expect.poll(() => curveAt(330, -10)).toBeGreaterThan(240);
});

test('the EQ treatment bypass toggles with the button and the B shortcut', async ({ page }) => {
  const bypass = page.getByRole('button', { name: 'Bypass treatment' });
  await expect(bypass).toHaveAttribute('aria-pressed', 'false');
  await bypass.click();
  await expect(bypass).toHaveAttribute('aria-pressed', 'true');
  await bypass.press('b');
  await expect(bypass).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByRole('group', { name: 'Preview sound' })).toHaveCount(0);
});

test('saved EQ presets persist through rename and delete, and A can be copied into B', async ({ page }) => {
  const presetMenu = settings => page.getByRole('button', { name: 'EQ preset, ' + settings, exact: true });
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([{ id: 1, kind: 'bell', hz: 1200, gainDb: -5, q: 150 }]));
  await presetMenu('Custom EQ').click();
  await page.getByRole('menuitem', { name: 'Save as preset…' }).click();
  await page.getByRole('textbox', { name: 'Preset name' }).fill('Tape notch');
  await page.getByRole('button', { name: 'Save preset' }).click();
  await expect(presetMenu('Tape notch')).toBeVisible();
  await page.getByRole('button', { name: 'Start EQ A/B comparison' }).click();
  await presetMenu('Tape notch').click();
  await page.getByRole('menuitem', { name: 'Reduce rumble', exact: true }).click();
  await expect.poll(() => settingsOf(page)).toMatchObject({ rumble: true, eqBands: null });
  await presetMenu('Reduce rumble').click();
  await page.getByRole('menuitem', { name: 'Copy A to B' }).click();
  await expect(graph.getByRole('button', { name: 'Bell 1', exact: true })).toBeVisible();
  await presetMenu('Tape notch').click();
  await page.getByRole('menuitem', { name: 'Manage saved presets…' }).click();
  await page.getByRole('button', { name: 'Rename Tape notch' }).click();
  await page.getByRole('textbox', { name: 'Preset name' }).fill('Tape cleanup');
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Delete Tape cleanup' })).toBeVisible();
  await page.reload();
  await presetMenu('Flat').click();
  await page.getByRole('menuitem', { name: 'Tape cleanup' }).click();
  await expect(graph.getByRole('button', { name: 'Bell 1', exact: true })).toBeVisible();
  await presetMenu('Tape cleanup').click();
  await page.getByRole('menuitem', { name: 'Manage saved presets…' }).click();
  await page.getByRole('button', { name: 'Delete Tape cleanup' }).click();
  await page.getByRole('button', { name: 'Delete preset', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Delete Tape cleanup' })).toHaveCount(0);
});

test('the node’s solo handle retunes the audition while held and releases on pointer up', async ({ page }) => {
  const graph = page.getByRole('group', { name: 'Frequency response and spectrum' });
  await page.evaluate(() => window.eqFixture.setBands([{ id: 1, kind: 'bell', hz: 1000, gainDb: 0, q: 80 }]));
  const node = graph.getByRole('button', { name: 'Bell 1', exact: true });
  const handle = graph.getByRole('button', { name: 'Hold to solo Bell 1', exact: true });
  await expect(handle).toHaveCSS('opacity', '0');
  await node.hover();
  await expect(handle).toHaveCSS('opacity', '1');
  const point = await handle.boundingBox();
  const x = point.x + point.width / 2;
  const y = point.y + point.height / 2;
  const auditions = () => page.evaluate(() => window.eqFixture.requests.filter(call => ['solo-band', 'retune-solo-band'].includes(call.action)));
  await page.mouse.move(x, y);
  await page.mouse.down();
  await expect(handle).toHaveAttribute('aria-pressed', 'true');
  const shade = graph.locator('.solo-shade');
  await expect(shade).toBeVisible();
  const shadeBox = await shade.boundingBox();
  const graphBox = await graph.boundingBox();
  expect(shadeBox.y).toBe(graphBox.y);
  expect(shadeBox.height).toBe(graphBox.height);
  const edges = await shade.locator('span').boundingBox();
  const nodeBox = await node.boundingBox();
  expect(Math.abs(edges.x + edges.width / 2 - (nodeBox.x + nodeBox.width / 2))).toBeLessThan(1);
  await expect(shade.locator('span')).toHaveCSS('border-inline-start-width', '1px');
  const spot = () => shade.evaluate(element => Number.parseFloat(element.style.getPropertyValue('--solo-start')));
  const left = await spot();
  await expect.poll(async () => (await auditions()).length).toBe(1);
  const initial = (await auditions())[0].band;
  await page.mouse.move(x + 95, y - 15, { steps: 6 });
  await expect.poll(async () => (await auditions()).at(-1).band.lowHz).toBeGreaterThan(initial.lowHz);
  await expect.poll(spot).toBeGreaterThan(left + 70);
  expect((await auditions()).at(-1).action).toBe('retune-solo-band');
  await expect(handle).toHaveAttribute('aria-pressed', 'true');
  await expect.poll(async () => (await settingsOf(page)).eqBands[0].hz).toBeGreaterThan(1000);
  expect((await settingsOf(page)).eqBands[0].gainDb).toBe(0);
  await page.mouse.up();
  await expect(handle).toHaveAttribute('aria-pressed', 'false');
  await expect(shade).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => window.eqFixture.requests.filter(call => call.action === 'stop-audition').length)).toBe(1);
});
