import { expect, test } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
import { noTreatment } from '../../app/ui/js/audio-treatment.ts';

const root = resolve(process.env.TAGNETIC_REPAIR_FIXTURE ?? '.test-dist');
const rate = 48_000;

test.beforeEach(async ({ page }) => {
  await page.route('http://tagrex.test/**', async route => {
    const path = resolve(root, '.' + new URL(route.request().url()).pathname);
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      const contentType = path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css'
        : path.endsWith('.woff2') ? 'font/woff2' : 'text/html';
      await route.fulfill({ body: await readFile(path), contentType });
    } catch { await route.fulfill({ status: 404 }); }
  });
  await page.addInitScript(({ rate, settings }) => {
    const calls = [];
    const handlers = new Map();
    const history = [];
    let pendingProbe;
    const overview = Array.from({ length: 256 }, (_, i) => Math.sin(i * .21) ** 2 * (i % 2 ? .8 : -.8));
    let snapshot = {
      sourceId: 'repair-fixture', frame: 10 * rate, frames: 60 * rate, sampleRate: rate,
      playing: false, original: false, loop: false, stereo: true, settings, cuts: [8 * rate, 44 * rate],
      overview, theme: { theme: 'dark', palette: 'navy', accent: [] },
    };
    const send = (name, payload) => handlers.get('tagnetic:audio-tools-' + name)?.({ payload });
    function showProbe(range, options) {
      const { start, end } = range ?? { start: snapshot.frame - 84_000, end: snapshot.frame + 84_000 };
      send('probe', {
        sourceId: snapshot.sourceId, probe: {
          frequencies: [20, 100, 500, 2500, 10_000, 20_000], original: [], treated: [], reduction: [], input: -20, output: -20,
          spectrogram: Array.from({ length: 20 }, (_, index) => [-94, -31 + index, -42, options?.treated ? -73 : -25, -42, -92]),
          spectrogramStart: start, spectrogramEnd: end, spectrogramChannel: options?.channel ?? 'combined',
          spectrogramTreated: options?.treated === true, spectrogramWaveform: overview,
          spectrogramDifference: options?.difference === true, spectrogramDetail: options?.detail ?? 'balanced',
          ...(options?.detail === 'transients' && !options.treated ? {
            spectrogramClicks: [{ frame: 10 * rate + 171, sensitivity: 2 }],
            spectrogramWindowFrames: 1024,
          } : {}),
          ...(options?.channel === 'stereo' ? {
            spectrogramLeft: Array.from({ length: 20 }, () => [-90, -18, -50, -63, -88, -96]),
            spectrogramRight: Array.from({ length: 20 }, () => [-90, -63, -50, -18, -88, -96]),
            spectrogramLeftWaveform: overview,
            spectrogramRightWaveform: overview.map(value => value * .5),
          } : {}),
        },
      });
    }
    const fixture = {
      calls, get snapshot() { return snapshot; }, holdProbe: false,
      updateClock(clock) {
        snapshot = { ...snapshot, ...clock };
        send('state', { ...snapshot, settings: undefined, overview: undefined, stereoOverview: undefined, cuts: undefined });
      },
      setCuts(cuts) {
        snapshot = { ...snapshot, cuts };
        send('state', snapshot);
      },
      setTheme(theme) {
        snapshot = { ...snapshot, theme: { ...snapshot.theme, theme } };
        send('state', snapshot);
      },
      loadRecording(frames, frame) {
        snapshot = { ...snapshot, sourceId: 'long-repair-fixture', frames, frame };
        send('state', snapshot);
      },
      deliverProbe() {
        if (pendingProbe) showProbe(pendingProbe.range, pendingProbe.options);
        pendingProbe = undefined;
        fixture.holdProbe = false;
      },
    };
    window.repairFixture = fixture;
    window.__TAURI__ = {
      event: {
        async listen(name, handler) { handlers.set(name, handler); return () => handlers.delete(name); },
        async emitTo(target, _name, payload) {
          if (target !== 'main') return;
          calls.push(payload);
          switch (payload.action) {
            case 'sync':
              send('state', snapshot);
              showProbe();
              break;
            case 'viewport':
              if (payload.options?.overview) break;
              if (fixture.holdProbe) pendingProbe = payload;
              else showProbe(payload.range, payload.options);
              break;
            case 'seek':
              snapshot = { ...snapshot, frame: payload.range.start };
              send('state', snapshot);
              break;
            case 'repair-listen': {
              const command = payload.command;
              const before = snapshot.repairListen ?? { signal: 'repaired', channel: 'stereo' };
              if (command.type === 'loop') {
                snapshot = { ...snapshot, loop: true, playing: command.play !== false, loopRange: command.range,
                  repairListen: before, monitor: command.band ? 'band' : undefined };
              } else if (command.type === 'signal') {
                snapshot = { ...snapshot, repairListen: { ...before, signal: command.signal }, monitor: undefined };
              } else if (command.type === 'trial') {
                snapshot = { ...snapshot, repairListen: { ...before, signal: 'repaired', trial: command.strength }, monitor: undefined };
              } else if (command.type === 'stop') {
                snapshot = { ...snapshot, playing: false };
              }
              send('state', snapshot);
              break;
            }
            case 'edit':
              history.push(snapshot.settings);
              snapshot = { ...snapshot, settings: { ...snapshot.settings, ...payload.patch }, revision: payload.revision };
              send('state', snapshot);
              break;
            case 'undo':
              snapshot = { ...snapshot, settings: history.pop() ?? settings, revision: payload.revision };
              send('state', snapshot);
              break;
          }
        },
      },
    };
  }, { rate, settings: noTreatment });
  await page.goto('http://tagrex.test/app/ui/audio-tools.html?tool=repair');
  await expect(page.getByRole('button', { name: 'Jump in recording overview' })).toBeEnabled();
});

async function drawSelection(page) {
  const plot = await page.getByRole('button', { name: 'Select time and frequency' }).boundingBox();
  await page.mouse.move(plot.x + plot.width * .31, plot.y + plot.height * .27);
  await page.mouse.down();
  await page.mouse.move(plot.x + plot.width * .66, plot.y + plot.height * .66, { steps: 5 });
  await page.mouse.up();
  await expect(page.getByRole('button', { name: 'Reduce', exact: true })).toBeEnabled();
  return plot;
}

test('keeps audition bounds and preview strengths separate from repair edits', async ({ page }) => {
  await drawSelection(page);
  await expect(page.getByRole('group', { name: 'Compare repair audio' }).getByText('Audio', { exact: true })).toBeVisible();
  const loopCommands = () => page.evaluate(() => window.repairFixture.calls
    .filter(call => call.action === 'repair-listen' && call.command.type === 'loop').map(call => call.command));
  await page.getByRole('button', { name: 'Audition selected area', exact: true }).click();
  await page.getByRole('button', { name: 'With context', exact: true }).click();
  await expect.poll(async () => (await loopCommands()).length).toBe(2);
  const [exact, context] = await loopCommands();
  expect(context.range).toEqual({ start: exact.range.start - 36_000, end: exact.range.end + 36_000 });

  const selected = page.locator('.picked');
  const before = await selected.boundingBox();
  await page.getByText('Loop and comparison', { exact: true }).press('Space');
  expect(await page.evaluate(() => window.repairFixture.calls.some(call => call.action === 'play'))).toBe(false);
  await page.getByLabel('Smooth loop join').check();
  await page.getByLabel('On stop').selectOption('start');
  const loopStart = page.getByRole('textbox', { name: 'Loop start' });
  await loopStart.fill('9.250');
  await loopStart.press('Enter');
  await expect.poll(async () => (await loopCommands()).at(-1)).toMatchObject({
    range: { start: 9.25 * rate, end: context.range.end }, smooth: true, returnTo: 'start', preservePosition: true,
  });
  const after = await selected.boundingBox();
  expect(after.x).toBeCloseTo(before.x);
  expect(after.width).toBeCloseTo(before.width);

  await page.getByRole('button', { name: 'Half strength', exact: true }).click();
  await page.getByRole('button', { name: 'Removed', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Removed', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const result = await page.evaluate(() => ({
    trial: window.repairFixture.calls.find(call => call.command?.type === 'trial')?.command,
    saved: window.repairFixture.snapshot.settings.spectralRepairs ?? null,
  }));
  expect(result.trial).toMatchObject({ strength: 'light',
    candidate: { startFrame: exact.range.start, endFrame: exact.range.end, amountDb: 12 } });
  expect(result.saved).toBeNull();

  const surface = page.getByRole('button', { name: 'Select time and frequency' });
  await surface.press(process.platform === 'darwin' ? 'Meta+Enter' : 'Control+Enter');
  await expect.poll(() => page.evaluate(() => window.repairFixture.snapshot.settings.spectralRepairs?.[0]?.amountDb)).toBe(12);
  await page.evaluate(() => window.repairFixture.updateClock({
    frame: 500_000, playing: true, clockRunning: true, repairPlayback: 'updating',
  }));
  await expect(page.getByText('Updating audio preview…', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Half strength', exact: true }).click();
  await expect.poll(() => page.evaluate(() =>
    window.repairFixture.calls.filter(call => call.command?.type === 'trial').at(-1)?.command.slot)).toBe(0);
  await surface.press('Escape');
  await expect(loopStart).toBeVisible();
  await expect(loopStart).toHaveValue('0:09.250000');
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  const loopEnd = page.getByRole('textbox', { name: 'Loop end' });
  await loopEnd.fill('10.500');
  await loopEnd.press('Enter');
  await expect.poll(async () => (await loopCommands()).at(-1)).toMatchObject({
    range: { start: 9.25 * rate, end: 10.5 * rate }, play: false,
  });
});

test('interpolates a full-band manual Time selection without saving its attenuation mask', async ({ page }) => {
  await page.getByRole('button', { name: 'Time', exact: true }).click();
  await drawSelection(page);
  await page.getByRole('button', { name: 'Exact selection boundaries' }).click();
  const duration = page.getByRole('textbox', { name: 'Duration' });
  await duration.fill('0:00.001');
  await duration.press('Enter');
  await page.getByText('Find and review', { exact: true }).click();
  await page.getByRole('combobox', { name: 'Find repair candidates' }).selectOption('clicks');
  await page.getByRole('button', { name: 'Interpolate click' }).click();
  const stored = () => page.evaluate(() => window.repairFixture.snapshot.settings.spectralRepairs?.[0] ?? {});
  await expect.poll(stored).toMatchObject({ interpolate: true, lowHz: 20, highHz: 20_000 });
  const repair = await stored();
  expect(repair.endFrame - repair.startFrame).toBe(48);
  expect(repair).not.toHaveProperty('mask');
});

test('reviews a native click suggestion and saves one undoable interpolation', async ({ page }) => {
  await drawSelection(page);
  await page.getByText('Find and review', { exact: true }).click();
  await page.getByRole('combobox', { name: 'Find repair candidates' }).selectOption('clicks');
  await expect(page.getByRole('button', { name: 'Analyse selection' })).toBeDisabled();
  await page.getByRole('button', { name: 'Show transients' }).click();
  await expect.poll(() => page.evaluate(() => window.repairFixture.calls.filter(call => call.action === 'viewport').at(-1)?.options?.detail))
    .toBe('transients');
  await page.getByRole('button', { name: 'Analyse selection' }).click();
  const patch = () => page.evaluate(() => window.repairFixture.snapshot.settings.spectralRepairs);
  await expect(page.getByText('Click 1 · 10.003562 s · 1 of 1')).toBeVisible();
  expect(await patch()).toBeNull();
  await page.getByRole('button', { name: 'Audition event' }).click();
  await expect.poll(() => page.evaluate(() => window.repairFixture.calls.findLast(call => call.action === 'loop')?.range))
    .toEqual({ start: 456_171, end: 504_172 });
  expect(await patch()).toBeNull();
  await page.getByRole('button', { name: 'Select event' }).click();
  await page.getByRole('button', { name: 'Interpolate click' }).click();
  await expect.poll(patch).toMatchObject([{ startFrame: 480_171, endFrame: 480_172, interpolate: true }]);
  const modifier = process.platform === 'darwin' ? 'Meta' : 'Control';
  await page.getByRole('button', { name: 'Select time and frequency' }).press(modifier + '+z');
  await expect.poll(patch).toBeNull();
});

test('saves a brush with a subtracting lasso and one grouped repair history entry', async ({ page }) => {
  const surface = page.getByRole('button', { name: 'Select time and frequency' });
  await page.getByRole('button', { name: 'Brush', exact: true }).click();
  let plot = await surface.boundingBox();
  const move = (x, y, steps = 1) => page.mouse.move(plot.x + plot.width * x, plot.y + plot.height * y, { steps });
  await move(.25, .42);
  await expect(page.locator('.brush-footprint')).toBeVisible();
  await page.mouse.down();
  await move(.45, .32, 4);
  await move(.70, .52, 4);
  await page.mouse.up();
  await expect(page.locator('.shape-overlay')).toHaveCount(1);

  await page.getByRole('button', { name: 'Lasso', exact: true }).click();
  await page.getByRole('button', { name: 'Subtract', exact: true }).click();
  plot = await surface.boundingBox();
  await move(.37, .35);
  await page.mouse.down();
  await move(.52, .28, 3);
  await move(.55, .49, 3);
  await move(.35, .49, 3);
  await page.mouse.up();
  await expect(page.locator('.shape-overlay mask path')).toHaveCount(2);
  await page.getByRole('button', { name: 'Reduce', exact: true }).click();
  const saved = () => page.evaluate(() => window.repairFixture.snapshot.settings.spectralRepairs?.[0]);
  await expect.poll(saved).toMatchObject({ mask: {
    version: 1, featherFrames: 0, featherHz: 0,
    regions: [
      { kind: 'brush', operation: 'add', radiusFrames: 816, radiusHz: 110 },
      { kind: 'lasso', operation: 'subtract' },
    ],
  } });
  expect((await saved()).mask.regions[0].points.length).toBeGreaterThan(2);
  expect(await page.evaluate(() => window.repairFixture.calls.filter(call => call.action === 'edit').length)).toBe(1);

  const bypass = page.getByRole('button', { name: 'Bypass', exact: true });
  await bypass.click();
  await expect(bypass).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: 'Exact selection boundaries' }).click();
  const name = page.getByRole('textbox', { name: 'Repair name' });
  await name.fill('Crowd whistle');
  await name.press('Tab');
  await expect.poll(saved).toMatchObject({ name: 'Crowd whistle', bypassed: true });
});

test('undo restores the last saved boundaries without keeping the newer selection', async ({ page }) => {
  await drawSelection(page);
  const reduce = page.getByRole('button', { name: 'Reduce', exact: true });
  const field = page.getByRole('textbox', { name: 'Start time' });
  await reduce.click();
  await page.getByRole('button', { name: 'Exact selection boundaries' }).click();
  const saved = () => page.evaluate(() => window.repairFixture.snapshot.settings.spectralRepairs?.[0]?.startFrame);
  const original = await saved();
  await field.fill('0:10.000');
  await field.press('Enter');
  await field.press((process.platform === 'darwin' ? 'Meta' : 'Control') + '+Enter');
  await expect.poll(saved).toBe(480_000);
  const surface = page.getByRole('button', { name: 'Select time and frequency' });
  await surface.press((process.platform === 'darwin' ? 'Meta' : 'Control') + '+z');
  await expect.poll(saved).toBe(original);
  await expect(field).toHaveValue(/^0:09\./);
});

test('uses the full source band for a time stripe and leaves the selection on ruler seek', async ({ page }) => {
  await page.getByRole('button', { name: 'Time', exact: true }).click();
  await expect(page.getByText('Full frequency band', { exact: true })).toBeVisible();
  await drawSelection(page);
  const plot = await page.getByRole('button', { name: 'Select time and frequency' }).boundingBox();
  await page.mouse.click(plot.x + plot.width * .75, plot.y + plot.height - 11);
  await expect(page.getByRole('button', { name: 'Reduce', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Reduce', exact: true }).click();
  const saved = () => page.evaluate(() => window.repairFixture.snapshot.settings.spectralRepairs?.[0]);
  await expect.poll(saved).toMatchObject({ lowHz: 0, highHz: 24_000, mask: { regions: [
    { kind: 'rectangle', operation: 'add', lowHz: 0, highHz: 24_000 },
  ] } });
  expect(await page.evaluate(() => window.repairFixture.calls.some(call => call.action === 'seek'))).toBe(true);
});

test('finds full-width bands directly and edits their exact duration and scope', async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 600 });
  const graph = page.getByRole('button', { name: 'Select time and frequency' });
  const reduce = page.getByRole('button', { name: 'Reduce', exact: true });
  const bounds = page.getByRole('button', { name: 'Exact selection boundaries' });
  const frequency = page.getByRole('button', { name: 'Frequency', exact: true });
  await expect(reduce).toBeDisabled();
  await expect(bounds).toBeDisabled();
  await expect(frequency).toBeVisible();
  await graph.press('3');
  await expect(frequency).toHaveAttribute('aria-pressed', 'true');
  const scope = page.getByRole('combobox', { name: 'Frequency selection scope' });
  await expect(scope).toHaveValue('view');
  await expect(scope.locator('option[value=loop]')).toBeDisabled();

  async function band(from, to, fullWidth = true) {
    const box = await graph.boundingBox();
    const x = box.x + box.width * .5;
    await page.mouse.move(x, box.y + box.height * from);
    await page.mouse.down();
    await page.mouse.move(x, box.y + box.height * to, { steps: 5 });
    await page.mouse.up();
    const pick = await page.locator('.picked').boundingBox();
    if (fullWidth) expect(pick.width).toBeGreaterThan(box.width - 90);
  }
  await band(.25, .52);
  await bounds.click();
  const duration = page.getByRole('textbox', { name: 'Duration' });
  await duration.fill('wrong');
  await duration.press('Enter');
  await expect(duration).toHaveAttribute('aria-invalid', 'true');
  await expect(reduce).toBeDisabled();
  await duration.fill('0:00.750');
  await duration.press('Enter');
  await expect(duration).toHaveValue('0:00.750000');
  await expect(duration).toHaveAttribute('aria-invalid', 'false');
  await expect(reduce).toBeEnabled();
  await bounds.click();

  await page.getByRole('button', { name: 'Audition selected area' }).click();
  await expect(scope.locator('option[value=loop]')).toBeEnabled();
  const loop = await page.evaluate(() => window.repairFixture.snapshot.loopRange);
  await scope.selectOption('loop');
  await band(.57, .72, false);
  await reduce.click();
  const repair = slot => page.evaluate(slot => window.repairFixture.snapshot.settings.spectralRepairs?.[slot], slot);
  await expect.poll(() => repair(0)).toMatchObject({ startFrame: loop.start, endFrame: loop.end });
  await graph.press('Escape');

  await scope.selectOption('recording');
  await band(.30, .46);
  await reduce.click();
  await expect.poll(() => repair(1)).toMatchObject({ startFrame: 0, endFrame: 60 * rate, mask: { regions: [
    { kind: 'rectangle', operation: 'add', startFrame: 0, endFrame: 60 * rate },
  ] } });
  await page.evaluate(() => window.repairFixture.setTheme('light'));
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await expect(frequency).toHaveAttribute('aria-pressed', 'true');
  await expect(scope).toHaveValue('recording');
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(640);
});

test('links the stereo lanes and keeps the dB scale fixed when changing the comparison', async ({ page }) => {
  const display = page.getByRole('button', { name: 'Spectrogram display' });
  const surface = page.getByRole('button', { name: 'Select time and frequency' });
  const selectDisplay = async name => {
    await display.click();
    await page.getByRole('menuitemradio', { name, exact: true }).click();
  };
  await selectDisplay('Linked stereo');
  await selectDisplay('Tones');
  await selectDisplay('Neutral');
  await expect(page.locator('.repair-plot canvas')).toHaveCount(2);
  const legend = page.locator('.db-legend');
  const scale = await legend.getAttribute('aria-label');
  const plot = await surface.boundingBox();
  await page.mouse.move(plot.x + plot.width * .30, plot.y + plot.height * .64);
  await page.mouse.down();
  await page.mouse.move(plot.x + plot.width * .63, plot.y + plot.height * .82, { steps: 5 });
  await page.mouse.up();
  await expect(page.getByRole('button', { name: 'Apply repair to channel' })).toHaveText('Right');
  await page.keyboard.down('Shift');
  await page.mouse.move(plot.x + plot.width * .27, plot.y + plot.height * .18);
  await page.mouse.down();
  await page.mouse.move(plot.x + plot.width * .60, plot.y + plot.height * .36, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up('Shift');
  await expect(page.getByText('Add and subtract use the selected repair channel.', { exact: false })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Apply repair to channel' })).toHaveText('Right');
  await selectDisplay('Removed');
  await expect(page.getByRole('group', { name: 'Removed spectrogram' })).toBeVisible();
  await expect(legend).toHaveAttribute('aria-label', scale);
  await expect.poll(() => page.evaluate(() => window.repairFixture.calls.filter(call => call.action === 'viewport').at(-1)?.options))
    .toEqual({ treated: true, difference: true, channel: 'stereo', detail: 'tones' });
  await display.click();
  await page.getByRole('menuitemcheckbox', { name: 'Waveform only' }).click();
  await expect(page.getByRole('group', { name: 'Removed waveform' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Time', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(legend).toHaveCount(0);
});

test('navigates the recording, refines a repair and auditions time and frequency separately', async ({ page }) => {
  const overview = page.getByRole('button', { name: 'Jump in recording overview' });
  await expect(page.locator('.overview-cut')).toHaveCount(2);
  await overview.press('Shift+ArrowRight');
  await expect.poll(() => page.evaluate(() => window.repairFixture.snapshot.frame)).toBe(20 * rate);
  const plot = await drawSelection(page);

  const selection = page.locator('.picked');
  const before = await selection.boundingBox();
  await page.mouse.move(before.x + before.width, before.y + before.height);
  await page.mouse.down();
  await page.mouse.move(before.x + before.width + 17, before.y + before.height + 11, { steps: 4 });
  await page.mouse.up();
  await expect.poll(async () => (await selection.boundingBox()).width).toBeGreaterThan(before.width + 8);

  const resized = await selection.boundingBox();
  await page.mouse.move(resized.x + resized.width / 2, resized.y + resized.height / 2);
  await page.mouse.down();
  await page.mouse.move(plot.x + plot.width - 2, resized.y + resized.height / 2, { steps: 5 });
  const visibleStart = () => page.locator('.overview-window').evaluate(el => parseFloat(el.style.left));
  const edgeStart = await visibleStart();
  await expect.poll(visibleStart).toBeGreaterThan(edgeStart + .1);
  await page.mouse.up();

  await page.getByRole('button', { name: 'Exact selection boundaries' }).click();
  const start = page.getByRole('textbox', { name: 'Start time' });
  await start.fill('0:19.125');
  await start.press('Enter');
  await expect(start).toHaveValue('0:19.125000');
  await start.press('Shift+ArrowUp');
  await expect(start).toHaveValue('0:19.125021');
  const surface = page.getByRole('button', { name: 'Select time and frequency' });
  await page.getByRole('button', { name: 'Apply repair to channel' }).click();
  await page.getByRole('menuitemradio', { name: 'Right only' }).click();
  await surface.press('Enter');
  await surface.press('Shift+Enter');
  await expect.poll(() => page.evaluate(() => window.repairFixture.calls.find(call => call.action === 'loop')?.range.start)).toBe(918_001);
  await expect.poll(() => page.evaluate(() => window.repairFixture.calls.find(call => call.action === 'audition-band')?.band.channel)).toBe('right');

  await surface.press(process.platform === 'darwin' ? 'Meta+Enter' : 'Control+Enter');
  await expect.poll(() => page.evaluate(() => window.repairFixture.snapshot.settings.spectralRepairs?.[0])).toMatchObject({
    startFrame: 918_001, channel: 'right', amountDb: 12,
  });
  await surface.press('F');
  const fitted = await visibleStart();
  await surface.press('Escape');
  await expect(selection).toHaveCount(0);
  await expect.poll(visibleStart).toBeCloseTo(fitted, 0);
  await expect(page.getByRole('button', { name: 'Follow playhead' })).toHaveAttribute('aria-pressed', 'false');
  await page.getByRole('button', { name: 'Applied repairs' }).click();
  await page.getByRole('menuitem').filter({ hasText: '−12 dB' }).click();
  await surface.press('Delete');
  await expect.poll(() => page.evaluate(() => window.repairFixture.snapshot.settings.spectralRepairs)).toBeNull();
  await surface.press(process.platform === 'darwin' ? 'Meta+z' : 'Control+z');
  await expect.poll(() => page.evaluate(() => window.repairFixture.snapshot.settings.spectralRepairs?.[0]?.channel)).toBe('right');
});

test('persists spectrogram display choices and requests the correct channel and treatment', async ({ page }) => {
  const display = page.getByRole('button', { name: 'Spectrogram display' });
  const graph = await page.getByRole('group', { name: 'Source spectrogram' }).boundingBox();
  const mode = await page.getByRole('button', { name: 'Select a frequency range controls' }).boundingBox();
  const controls = await page.getByRole('group', { name: 'Spectrogram controls' }).boundingBox();
  expect(mode.x).toBeGreaterThanOrEqual(graph.x + graph.width);
  expect(controls.y).toBeGreaterThanOrEqual(graph.y + graph.height);
  const bandEnergy = () => page.locator('.repair-plot canvas').evaluate(canvas => {
    // Sample inside the 2500 Hz band, away from plot axes and menus.
    const data = canvas.getContext('2d').getImageData(
      Math.round(canvas.width * .25), Math.round(canvas.height * .27),
      Math.round(canvas.width * .5), Math.round(canvas.height * .09),
    ).data;
    let sum = 0;
    for (let index = 3; index < data.length; index += 4) sum += data[index];
    return 4 * sum / data.length;
  });
  await expect.poll(bandEnergy).toBeGreaterThan(75);
  await page.evaluate(() => window.repairFixture.holdProbe = true);
  const source = await bandEnergy();
  await display.click();
  await page.getByRole('menuitemradio', { name: 'Treated' }).click();
  await expect(page.getByText('Updating spectrogram…')).toBeVisible();
  expect(await bandEnergy()).toBeGreaterThan(source * .88);
  await page.evaluate(() => window.repairFixture.deliverProbe());
  await expect(page.getByText('Updating spectrogram…')).toHaveCount(0);
  await expect.poll(bandEnergy).toBeLessThan(source * .5);
  await display.click();
  await page.getByRole('menuitemradio', { name: 'Left', exact: true }).click();
  await display.click();
  await page.getByRole('menuitemradio', { name: 'Reveal faint detail' }).click();
  await display.click();
  await page.getByRole('menuitemcheckbox', { name: 'Waveform overlay' }).click();
  await expect.poll(() => page.evaluate(() => window.repairFixture.calls.filter(call => call.action === 'viewport').at(-1)?.options))
    .toEqual({ treated: true, channel: 'left' });
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('tagnetic.audio-repair-view.v1'))))
    .toMatchObject({ treated: true, channel: 'left', contrast: 'detail', waveform: true });
  await page.reload();
  await display.click();
  for (const name of ['Treated', 'Left', 'Reveal faint detail']) {
    await expect(page.getByRole('menuitemradio', { name, exact: true })).toHaveAttribute('aria-checked', 'true');
  }
  await expect(page.getByRole('menuitemcheckbox', { name: 'Waveform overlay' })).toHaveAttribute('aria-checked', 'true');
});

test('navigates an hour of audio at narrow width without full-track spectral requests', async ({ page }) => {
  await page.setViewportSize({ width: 620, height: 760 });
  await page.evaluate(rate => window.repairFixture.loadRecording(3600 * rate, 2460 * rate), rate);
  const display = page.getByRole('button', { name: 'Spectrogram display' });
  await display.click();
  await page.getByRole('menuitemradio', { name: 'Removed', exact: true }).click();
  await expect(page.getByRole('group', { name: 'Removed spectrogram' })).toBeVisible();
  const viewportEvents = () => page.evaluate(() => window.repairFixture.calls.filter(call => call.action === 'viewport'));
  await expect.poll(async () => (await viewportEvents()).at(-1)?.options?.difference).toBe(true);

  await page.getByRole('button', { name: 'Fit recording', exact: true }).click();
  const graph = page.getByRole('group', { name: 'Source waveform overview' });
  const overviewRange = page.locator('.overview-window');
  await expect(graph).toBeVisible();
  await expect(page.getByText('Source waveform · zoom in for spectral detail')).toBeVisible();
  await expect(overviewRange).toHaveCSS('left', '0px');
  await expect.poll(() => overviewRange.evaluate(el => parseFloat(el.style.width))).toBe(100);
  await expect.poll(async () => (await viewportEvents()).at(-1)).toMatchObject({ range: null, options: { overview: true } });
  const broadNoProbe = (await viewportEvents()).length;

  const bounds = await graph.boundingBox();
  const mouse = { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 };
  await page.mouse.move(mouse.x, mouse.y);
  const [frameDeltas] = await Promise.all([
    page.evaluate(() => new Promise(resolve => {
      const intervals = [];
      let first;
      let previous;
      function sample(now) {
        first ??= now;
        if (previous !== undefined) intervals.push(now - previous);
        previous = now;
        if (now - first < 250) requestAnimationFrame(sample);
        else resolve(intervals);
      }
      requestAnimationFrame(sample);
    })),
    page.mouse.wheel(0, -280),
  ]);
  if (process.env.TAGNETIC_NAVIGATION_BENCHMARK) {
    const sorted = frameDeltas.toSorted((a, b) => a - b);
    const ms = percentile => sorted[Math.floor((sorted.length - 1) * percentile)]?.toFixed(1);
    console.log('Hour-long waveform zoom frame intervals', { samples: sorted.length, p50: ms(.5), p95: ms(.95), max: ms(1) });
  }
  await expect.poll(() => overviewRange.evaluate(el => parseFloat(el.style.width))).toBeLessThan(51);
  const firstZoom = await overviewRange.evaluate(el => ({ left: parseFloat(el.style.left), width: parseFloat(el.style.width) }));
  expect(firstZoom.left / (100 - firstZoom.width)).toBeGreaterThan(.44);
  expect(firstZoom.left / (100 - firstZoom.width)).toBeLessThan(.56);
  expect(await viewportEvents()).toHaveLength(broadNoProbe);

  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(mouse.x - 30, mouse.y, { steps: 5 });
  await page.mouse.up({ button: 'middle' });
  await expect.poll(() => overviewRange.evaluate(el => parseFloat(el.style.left))).toBeGreaterThan(firstZoom.left + 1);
  await expect.poll(() => overviewRange.evaluate(el => parseFloat(el.style.width))).toBeCloseTo(firstZoom.width);
  expect(await viewportEvents()).toHaveLength(broadNoProbe);
  await page.mouse.move(mouse.x, mouse.y);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(mouse.x + 30, mouse.y, { steps: 5 });
  await page.mouse.up({ button: 'middle' });
  await page.mouse.move(mouse.x, mouse.y);

  for (let step = 0; step < 9; step++) await page.mouse.wheel(0, -280);
  await expect(page.getByRole('group', { name: 'Removed spectrogram' })).toBeVisible();
  await expect.poll(async () => {
    const last = (await viewportEvents()).at(-1);
    return !!last?.range && !last.options?.overview && last.options?.difference === true;
  }).toBe(true);
  const local = (await viewportEvents()).at(-1);
  expect(local.options.overview).not.toBe(true);
  expect(local.range.end - local.range.start).toBeLessThanOrEqual(12 * rate);
  expect(local.range.start / rate).toBeGreaterThan(1600);
  expect(local.range.end / rate).toBeLessThan(2000);

  await page.getByRole('button', { name: 'Select time and frequency' }).press('0');
  await expect(graph).toBeVisible();
  await expect.poll(() => overviewRange.evaluate(el => parseFloat(el.style.width))).toBe(100);
});


test('playhead feedback continues between snapshots and holds while audio is buffering', async ({ page }) => {
  const position = () => page.locator('.overview-playhead').evaluate(marker =>
    new DOMMatrixReadOnly(getComputedStyle(marker).transform).m41);
  await page.evaluate(() => window.repairFixture.updateClock({ playing: true, clockRunning: true }));
  const first = await position();
  await expect.poll(position).toBeGreaterThan(first);
  await page.evaluate(() => window.repairFixture.updateClock({ clockRunning: false }));
  await expect.poll(position).toBeLessThanOrEqual(first);
  const stopped = await position();
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(await position()).toBe(stopped);
});


test('many cut landmarks share one keyboard entry in the overview', async ({ page }) => {
  await page.evaluate(() => window.repairFixture.setCuts(Array.from({ length: 48 }, (_, index) => (index + 1) * 48_000)));
  const overview = page.getByRole('button', { name: 'Jump in recording overview' });
  await expect(page.locator('.overview-cut[tabindex="-1"]')).toHaveCount(48);
  await overview.press('PageDown');
  await expect.poll(() => page.evaluate(() => window.repairFixture.snapshot.frame)).toBe(528_000);
  await overview.press('PageUp');
  await expect.poll(() => page.evaluate(() => window.repairFixture.snapshot.frame)).toBe(480_000);
});
