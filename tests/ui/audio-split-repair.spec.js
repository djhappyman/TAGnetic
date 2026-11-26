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
        },
      });
    }
    const fixture = {
      calls, get snapshot() { return snapshot; }, holdProbe: false,
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
              if (fixture.holdProbe) pendingProbe = payload;
              else showProbe(payload.range, payload.options);
              break;
            case 'seek':
              snapshot = { ...snapshot, frame: payload.range.start };
              send('state', snapshot);
              break;
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
  await expect(page.getByRole('button', { name: 'Apply', exact: true })).toBeEnabled();
  return plot;
}

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
