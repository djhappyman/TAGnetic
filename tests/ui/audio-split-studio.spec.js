import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { resolve, sep } from 'node:path';
import { openTagSource } from './navigation.js';

const root = resolve(process.env.TAGNETIC_STUDIO_FIXTURE ?? '.test-dist');

async function serveFixture(route) {
  const path = resolve(root, '.' + new URL(route.request().url()).pathname);
  if (!path.startsWith(root + sep)) return route.abort();
  try {
    await route.fulfill({
      body: await readFile(path),
      contentType: path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html',
    });
  } catch { await route.fulfill({ status: 404, body: 'Missing fixture' }); }
}

test.beforeEach(async ({ page }) => {
  await page.route('http://tagrex.test/**', serveFixture);
  await page.goto('http://tagrex.test/app/ui/index.html');
  await expect.poll(() => page.evaluate(() => typeof window.tagrex?.openLibrary)).toBe('function');
  await page.evaluate(async () => {
    document.getElementById('root').value = '/music';
    await window.tagrex.openLibrary();
    const calls = [];
    const peaks = Array.from({ length: 512 }, (_, index) => (index % 2 ? 1 : -1) * (0.3 + 0.5 * Math.sin(index / 11) ** 2));
    function stereoWaveform(first = 0, last = 480000, bins = 256) {
      const peaks = [];
      const rms = [];
      for (let bin = 0; bin < bins; bin++) {
        const frame = first + (bin + 0.5) * (last - first) / bins;
        // The left channel keeps identical peaks but has two distinct energy levels.
        peaks.push(-0.8, 0.8, 0, 0);
        rms.push(frame < 240000 ? 0.21 : 0.64, 0);
      }
      return { peaks, rms };
    }
    const stereo = stereoWaveform();
    let finishExport;
    let reviewIndex = 0;
    let reviewed = null;
    window.studioFixture = { calls, slow: false, picked: '/exports', slowOpen: false, refreshes: [] };
    const eventHandlers = new Map();
    const toolVisibility = new Map();
    const toolPins = new Map();
    window.__TAURI__ = {
      event: {
        async listen(name, handler) {
          eventHandlers.set(name, handler);
          return () => eventHandlers.delete(name);
        },
        async emitTo(target, name, payload) {
          if (target === 'main') eventHandlers.get(name)?.({ payload });
          else if (name === 'tagnetic:audio-tools-state') {
            if (target === 'audio-tool-repair') window.studioFixture.repairUpdates?.push(JSON.parse(JSON.stringify(payload)));
            const previous = window.studioFixture.toolSnapshots?.[target];
            const state = payload?.sourceId === previous?.sourceId && payload && !payload.settings
              ? { ...payload, settings: previous.settings } : payload;
            window.studioFixture.toolSnapshots = { ...window.studioFixture.toolSnapshots, [target]: state };
          } else if (name === 'tagnetic:audio-tools-probe') {
            window.studioFixture.toolProbes = { ...window.studioFixture.toolProbes, [target]: payload };
          } else if (name === 'tagnetic:audio-tools-flush') {
            window.studioFixture.toolFence = payload;
          }
        },
      },
      core: {
        async invoke(command, args) {
          const timed = command === 'audio_split_transport' && window.studioFixture.repairUpdates;
          calls.push({ command, args, ...(timed ? { at: performance.now() } : {}) });
          switch (command) {
            case 'open_provider_page': return;
            case 'read_cover_summary': return { samples: [] };
            case 'read_cover_image': return { mime: 'image/png', data_base64: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jX1sAAAAASUVORK5CYII=' };
            case 'audio_split_open':
              if (window.studioFixture.slowOpen) await new Promise(resolve => { window.studioFixture.finishOpen = resolve; });
              return { id: 'recording', path: args.path, frames: 480000, sampleRate: window.studioFixture.sampleRate || 48000, originalFormat: window.studioFixture.originalFormat, sourceStamp: 'unchanged-master', tags: { album: 'Saved album', ...window.studioFixture.sourceTags }, overview: peaks, ...(window.studioFixture.stereo ? {
                stereoOverview: stereo.peaks, stereoOverviewRms: stereo.rms,
                overviewRms: stereo.rms.filter((_, index) => index % 2 === 0).map(value => value * Math.SQRT1_2),
              } : {}) };
            case 'audio_split_loading':
              return { frames: 480000, sampleRate: 48000, scannedFrames: 48000, overview: peaks };
            case 'audio_split_cancel_open':
              window.studioFixture.finishOpen?.();
              return;
            case 'audio_split_reveal': return;
            case 'audio_split_peaks': {
              if (!args.stereo) return Float32Array.from(peaks).buffer;
              const tile = stereoWaveform(args.startFrame, args.endFrame, Math.min(args.bins, args.endFrame - args.startFrame));
              return Float32Array.from(args.energy ? [...tile.peaks, ...tile.rms] : tile.peaks).buffer;
            }
            case 'audio_split_treatment':
              if (window.studioFixture.holdTreatment) await new Promise(resolve => { (window.studioFixture.treatmentWaiters ||= []).push(resolve); });
              return { gainDb: args.treatment.normalize ? 4.2 : 0, loudnessLufs: -22.2, outputChannels: args.treatment.mono === 'off' ? 2 : 1 };
            case 'audio_split_compare_levels': {
              const measure = treatment => ({ input: -30, output: -30, ...window.studioFixture.treatmentProbe?.({ ...args, treatment }) });
              return { ...measure(args.treatment), alternateOutput: measure(args.alternate).output };
            }
            case 'audio_split_probe': {
              const probe = window.studioFixture.treatmentProbe?.(args);
              return { input: -30, output: -30, ...(probe instanceof Promise ? await probe : probe) };
            }
            case 'audio_split_eq_learn_threshold': return -32.5;
            case 'audio_split_eq_match':
              return [{ id: 1, kind: 'bell', hz: 2500, gainDb: -4, q: 140 }];
            case 'audio_split_tool_window':
              if (args.action === 'hide_all') { toolVisibility.clear(); return false; }
              if (args.action === 'pin' || args.action === 'unpin') toolPins.set(args.kind, args.action === 'pin');
              if (['pin', 'unpin', 'is_pinned'].includes(args.action)) return toolPins.get(args.kind) ?? false;
              toolVisibility.set(args.kind, args.action === 'toggle' && !toolVisibility.get(args.kind));
              return toolVisibility.get(args.kind);
            case 'audio_split_status':
              if (args.audition) return { playing: true, loading: false, sequence: args.sequence };
              return window.studioFixture.statusProbe?.(args) ?? { frame: 48001, playing: true };
            case 'audio_split_audition_output': return;
            case 'audio_split_transport':
            case 'audio_split_close': return;
            case 'audio_split_cue':
              return { sourceName: 'other-master.flac', markers: [
                { frame: 48000, title: 'CUE first', artist: 'CUE artist', tags: { composer: 'CUE composer', isrc: 'GBABC2312345' }, index01: 75 },
                { frame: 144000, title: 'CUE second', index00: 210, index01: 225, pregapFrame: 134400, postgap: 4 },
              ], tags: { album: 'CUE album' }, warnings: ['This sheet has a POSTGAP instruction.'] };
            case 'audio_split_write_cue': return ['The saved CUE rounds to CD frames.'];
            case 'audio_split_discogs':
              if (args.releaseId === '65432') {
                if (args.anchors.some((item, index) => index > 0
                  && (item.startIndex < args.anchors[index - 1].endIndex || item.frame <= args.anchors[index - 1].frame))) {
                  throw new Error('Anchor track ranges and times must increase.');
                }
                const rows = ['A1', 'A2', 'B1', 'B2', 'C1', 'D1', 'C2', 'E1', 'E1'].map((position, index) => {
                  const anchor = args.anchors.find(item => item.startIndex <= index && item.endIndex > index);
                  return {
                    position, title: 'Release track ' + (index + 1), side: position[0], durationSeconds: 1,
                    frame: anchor ? anchor.frame + (index - anchor.startIndex) * 48000 : null,
                  };
                });
                return { rows, warnings: [] };
              }
              return { rows: [
                { position: 'A1', title: 'Release first', durationSeconds: 1, frame: args.anchors.length ? args.anchors[0].frame : null },
                { position: 'A2', title: 'Release second', durationSeconds: 1, frame: args.anchors.length ? args.anchors[0].frame + 48000 : null },
              ], tags: { album: 'Discogs release', genre: 'Electronic' }, warnings: [], spans: args.anchors.length ? [{ startIndex: 0, endIndex: 2, expectedEndFrame: args.anchors[0].frame + 96000, availableEndFrame: 480000 }] : [] };
            case 'audio_split_preview': {
              const { request } = args;
              const files = request.segments || request.sources.map(() => ({ include: true, startFrame: 0, tags: {} }));
              const cut = frame => request.splitMethod === 'copy' && frame === 144000 ? 143616 : frame;
              const directory = request.saveMode === 'folder' ? request.destination : '/music';
              const backups = request.saveMode === 'replace'
                ? request.sources.map(source => ({ source, path: (request.backupDirectory || '/recovery') + '/' + source.split('/').at(-1) }))
                : [];
              reviewed = {
                id: 'review-' + ++reviewIndex, splitMethod: request.splitMethod, warnings: ['Review the final tags.'], backups,
                outputs: files.flatMap((item, index) => {
                  const end = files[index + 1]?.startFrame ?? 480000;
                  return item.include ? [{
                    path: `${directory}/track-${index + 1}.${request.format}`,
                    startFrame: cut(item.startFrame), endFrame: cut(end),
                    requestedStartFrame: item.startFrame, requestedEndFrame: end,
                    sampleRate: 48000, tags: { ...request.tags, ...item.tags },
                  }] : [];
                }),
              };
              return reviewed;
            }
            case 'audio_split_progress': return { phase: 'Encoding', completed: 0, total: 2, framesDone: 10, framesTotal: 480000 };
            case 'audio_split_export': {
              if (window.studioFixture.slow) return await new Promise((resolve, reject) => { finishExport = reject; });
              return {
                paths: reviewed.outputs.map(output => output.path),
                replaced: reviewed.backups.map(backup => backup.source),
                backups: reviewed.backups.map(backup => backup.path),
                warnings: window.studioFixture.exportWarnings,
              };
            }
            case 'audio_split_undo_replacement': return {
              paths: reviewed.backups.map(backup => backup.source),
              removed: reviewed.outputs.map(output => output.path),
            };
            case 'audio_split_reveal': return;
            case 'audio_split_cancel':
              finishExport?.('Export stopped');
              finishExport = undefined;
              return;
            default: throw new Error('Unexpected audio studio call ' + command);
          }
        },
      },
      dialog: {
        open: async () => window.studioFixture.picked,
        save: async () => '/exports/draft.cue',
      },
    };
  });
});

const calls = (page, action) => page.evaluate(name => window.studioFixture.calls.filter(call => call.command === 'audio_split_' + name), action);
const rows = studio => studio.getByRole('region', { name: 'Tracks', exact: true }).locator('button.track');
const reviewAndSave = studio => studio.getByRole('button', { name: 'Review & save' }).click();
const advanced = studio => studio.getByRole('button', { name: 'Output settings' }).click();

async function importCuts(page, studio, name) {
  await studio.getByRole('button', { name: 'Import splits' }).click();
  await page.getByRole('menuitem', { name }).click();
}

async function waveformOption(page, studio, name) {
  await studio.getByRole('button', { name: 'Waveform options', exact: true }).click();
  await page.getByRole('menuitem', { name }).click();
}

test('opening waveform options clears its tooltip while the menu is open', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const options = studio.getByRole('button', { name: 'Waveform options' });
  await expect(options).toBeEnabled();
  const time = new Date('2026-01-01T00:00:00Z');
  await page.clock.install({ time });
  await page.clock.pauseAt(new Date(time.getTime() + 1000));
  const tip = page.locator('#controller-tooltip-content');
  await page.mouse.move(0, 0);
  await options.hover();
  await page.clock.runFor(550);
  await expect(tip).toBeVisible();
  await expect(tip).toHaveText('Waveform options');
  await options.click();
  await page.clock.runFor(700);
  await expect(page.getByRole('menuitem', { name: 'Zoom in' })).toBeVisible();
  await expect(tip).toBeHidden();
});

async function editFromMenu(page, target, action) {
  await target.click({ button: 'right' });
  await page.getByRole('menuitem', { name: action, exact: true }).click();
}

test('a webview reload releases the previous preview before opening another recording', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const session = () => page.evaluate(() => sessionStorage.getItem('tagnetic.audio-split-session'));
  await expect.poll(session).toBe('recording');

  await page.addInitScript(() => {
    window.reloadCalls = [];
    window.__TAURI__ = {
      core: {
        async invoke(command, args) {
          if (command === 'audio_split_close' || command === 'audio_split_open') {
            window.reloadCalls.push({ command, id: args.id });
          }
          switch (command) {
            case 'audio_split_close':
              if (args.id === 'recording') await new Promise(resolve => { window.finishReloadClose = resolve; });
              return;
            case 'audio_split_open':
              return { id: 'new-recording', path: args.path, frames: 480000, sampleRate: 48000, overview: [-0.1, 0.1] };
            case 'read_cover_summary': return { samples: [] };
            default: throw new Error('Unexpected reload fixture call ' + command);
          }
        },
      },
    };
  });
  await page.reload();
  const reloadCalls = () => page.evaluate(() => window.reloadCalls);
  await expect.poll(reloadCalls).toEqual([{ command: 'audio_split_close', id: 'recording' }]);
  await page.evaluate(() => {
    document.dispatchEvent(new CustomEvent('tagrex:audio-studio', {
      detail: { mode: 'split', paths: ['/music/next.mp3'], opener: null },
    }));
  });
  await expect(studio.getByRole('status')).toHaveText('Opening the recording…');
  expect(await reloadCalls()).toEqual([{ command: 'audio_split_close', id: 'recording' }]);
  await page.evaluate(() => window.finishReloadClose());
  await expect.poll(session).toBe('new-recording');
  expect((await reloadCalls()).map(call => call.command)).toEqual(['audio_split_close', 'audio_split_open']);

  await studio.getByRole('button', { name: 'Close audio studio' }).click();
  await expect.poll(session).toBe(null);
});

test('the track selection highlight contains both metadata lines and the time columns', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const track = rows(studio).first();
  const trackList = studio.getByRole('region', { name: 'Tracks', exact: true });
  await expect(trackList.getByRole('textbox')).toHaveCount(0);
  await track.click({ button: 'right' });
  await expect(page.getByRole('menuitem', { name: 'Loop track', exact: true })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Reset start', exact: true })).toBeDisabled();
  await page.getByRole('menuitem', { name: 'Edit Artist/Title' }).click();
  const artist = trackList.getByRole('textbox', { name: 'Artist for track 1' });
  await expect(artist).toBeFocused();
  const title = trackList.getByRole('textbox', { name: 'Title for track 1' });
  await expect(artist).toHaveAttribute('placeholder', 'Artist');
  await expect(title).toHaveAttribute('placeholder', 'Title');
  expect((await artist.boundingBox()).y).toBeCloseTo((await title.boundingBox()).y);
  await artist.fill('Source Artist');
  await artist.press('Tab');
  await expect(title).toBeFocused();
  await title.fill('First track');
  await studio.getByRole('heading', { name: 'Tracks', exact: true }).click();
  await expect(track).toContainText('First track');
  await expect(artist).toHaveCount(0);
  await expect(track).toContainText('Source Artist');
  await expect(track).toHaveAttribute('aria-pressed', 'true');
  expect(await track.evaluate(row => {
    const { top, bottom } = row.getBoundingClientRect();
    return Array.from(row.children).every(child => {
      const text = child.getBoundingClientRect();
      return text.top >= top && text.bottom <= bottom;
    });
  })).toBe(true);
  await track.press('Shift+F10');
  await page.getByRole('menuitem', { name: 'Edit Artist/Title' }).click();
  await expect(title).toHaveValue('First track');
  await expect(artist).toHaveValue('Source Artist');
  await title.fill('Discard this edit');
  await title.press('Escape');
  await expect(track).toBeFocused();
  await expect(track).not.toContainText('Discard this edit');
  await studio.getByRole('button', { name: 'Undo draft change' }).click();
  await expect(track).not.toContainText('Source Artist');
  await expect(track).not.toContainText('First track');
  await expect(studio.locator('.notice')).toBeHidden();
  expect(await calls(page, 'transport')).toHaveLength(0);
});

test('low-rate tape recordings start with a compatible MP3 bitrate', async ({ page }) => {
  await page.evaluate(() => { window.studioFixture.sampleRate = 22050; });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await reviewAndSave(studio);
  await advanced(studio);
  await expect(studio.getByRole('combobox', { name: 'MP3 quality' })).toContainText('128 kbps');
  await studio.getByRole('combobox', { name: 'MP3 quality' }).click();
  await expect(page.getByRole('option', { name: '160 kbps' })).toBeVisible();
  await expect(page.getByRole('option', { name: '192 kbps' })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(studio.getByRole('heading', { name: '1 track to save' })).toBeVisible();
  expect((await calls(page, 'preview')).at(-1).args.request.bitrate).toBe(128);
});

for (const originalFormat of ['mp3', 'flac']) {
  test(`${originalFormat.toUpperCase()} starts with frame copy and reviews the actual cuts before exact re-encode`, async ({ page }) => {
    await page.setViewportSize({ width: 430, height: 710 });
    await page.evaluate(format => { window.studioFixture.originalFormat = format; }, originalFormat);
    await page.getByRole('button', { name: 'Split recording', exact: true }).click();
    const studio = page.getByRole('dialog', { name: 'Split recording' });
    const waveform = studio.getByRole('slider', { name: 'Waveform playhead' });
    for (let second = 0; second < 3; second++) await waveform.press('ArrowRight');
    await waveform.press('m');
    await reviewAndSave(studio);
    await advanced(studio);
    const method = studio.getByRole('combobox', { name: 'Split method' });
    const format = studio.getByRole('combobox', { name: 'Output', exact: true });
    await expect(format).toContainText(originalFormat.toUpperCase());
    await expect(method).toContainText('Fast split');
    await expect(studio.getByRole('combobox', { name: 'MP3 quality' })).toHaveCount(0);
    const info = studio.getByRole('button', { name: 'About split methods' });
    await info.click();
    await expect(page.getByText('copies the audio as it is', { exact: false })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(info).toBeFocused();

    const output = studio.getByRole('region', { name: 'Output review' });
    await expect(output.locator('details, dl')).toHaveCount(0);
    await expect(output.locator('li span')).toHaveText(['track-1.' + originalFormat, 'track-2.' + originalFormat]);
    await output.getByRole('button', { name: 'Audition track-2.' + originalFormat }).click();
    await expect.poll(async () => (await calls(page, 'audition_output')).at(-1)?.args)
      .toMatchObject({ id: 'review-1', index: 1 });
    await output.getByRole('button', { name: 'Stop audition of track-2.' + originalFormat }).click();
    await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args).toMatchObject({ action: 'pause' });
    expect((await calls(page, 'preview')).at(-1).args.request).toMatchObject({ format: originalFormat, splitMethod: 'copy' });
    expect((await calls(page, 'preview')).at(-1).args.request.bitrate).toBeUndefined();

    await method.click();
    await page.getByRole('option', { name: 'Exact cuts' }).click();
    await expect.poll(async () => (await calls(page, 'cancel')).at(-1)?.args).toEqual({ id: 'review-1' });
    if (originalFormat === 'mp3') {
      const quality = studio.getByRole('combobox', { name: 'MP3 quality' });
      await expect(quality).toBeVisible();
      const bounds = await Promise.all([format.boundingBox(), method.boundingBox(), quality.boundingBox()]);
      expect(bounds[1].y).toBeCloseTo(bounds[0].y, 0);
      expect(bounds[2].y).toBeCloseTo(bounds[0].y, 0);
      expect(await format.evaluate(button => getComputedStyle(button).borderTopWidth)).toBe('0px');
      expect(await quality.evaluate(button => getComputedStyle(button).borderTopWidth)).toBe('0px');
    }
    await expect(output.getByRole('button', { name: 'Audition track-2.' + originalFormat })).toBeEnabled();
    expect((await calls(page, 'preview')).at(-1).args.request).toMatchObject({
      format: originalFormat,
      splitMethod: 'exact',
      segments: [{ startFrame: 0 }, { startFrame: 144000 }],
    });
  });
}

test('the recording chooser uses inset styling and a floating close at narrow widths', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 420 });
  await page.getByRole('button', { name: 'Clear selection', exact: true }).click();
  const opener = page.getByRole('button', { name: 'Split recording', exact: true });
  await opener.click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const close = studio.getByRole('button', { name: 'Close audio studio' });
  await expect(studio.getByRole('button', { name: 'Choose recording or CUE…' })).toBeInViewport();
  await expect(close).toHaveCSS('position', 'absolute');
  await expect(close).toHaveCSS('border-top-width', '0px');
  await expect(studio.locator('header')).toHaveCSS('border-bottom-width', '0px');
  await expect(studio.locator('footer')).toHaveCSS('border-top-width', '0px');
  await expect(studio.getByRole('button', { name: 'Review & save' })).toHaveCSS('border-top-width', '0px');
  const end = await close.boundingBox();
  await page.evaluate(() => { document.body.classList.add('close-start'); });
  const start = await close.boundingBox();
  const shell = await studio.boundingBox();
  expect(end.x).toBeGreaterThan(shell.x + shell.width / 2);
  expect(start.x).toBeLessThan(shell.x + shell.width / 2);
  await close.focus();
  await close.press('Enter');
  await expect(studio).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test('keyboard and pointer cuts, exact draft reopen and reviewed MP3 export', async ({ page }) => {
  const opener = page.getByRole('button', { name: 'Split recording', exact: true });
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await opener.click();
  await expect(studio.getByRole('heading', { name: 'Split recording' })).toBeFocused();
  await expect(rows(studio)).toHaveCount(1);

  const waveform = studio.getByRole('slider', { name: 'Waveform playhead' });
  await waveform.focus();
  await waveform.press('ArrowRight');
  await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args).toMatchObject({ action: 'seek', frame: 48000 });
  await waveform.press('m');
  await expect(rows(studio)).toHaveCount(2);
  const marker = studio.getByRole('button', { name: 'Cut at 0:01.000', exact: true });
  const bounds = await marker.boundingBox();
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 + 35, bounds.y + bounds.height / 2, { steps: 5 });
  await page.mouse.up();
  await expect(marker).toHaveCount(0);

  await editFromMenu(page, studio.getByRole('button', { name: /^Cut at / }), 'Edit cut time');
  const time = studio.getByRole('textbox', { name: 'Track start time' });
  await time.fill('0:01.000021');
  await time.press('Enter');
  await editFromMenu(page, studio.getByRole('button', { name: /^Cut at / }), 'Edit cut time');
  await expect(time).toHaveValue('0:01.000021');
  await time.press('Escape');
  await rows(studio).last().press('F2');
  const title = studio.getByRole('textbox', { name: 'Title for track 2' });
  await title.fill('Second track');
  await title.press('Enter');
  await expect(rows(studio).last()).toContainText('Second track');
  await studio.getByRole('button', { name: 'Undo draft change' }).click();
  await expect(rows(studio).last()).toContainText('Track 2');
  await studio.getByRole('button', { name: 'Redo draft change' }).click();
  await expect(rows(studio).last()).toContainText('Second track');
  await waveform.press('Shift+Space');
  await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args).toMatchObject({ action: 'audition', frame: 0, endFrame: 144001 });
  await studio.getByRole('button', { name: 'Pause audio' }).click();
  await waveform.press('Home');
  await waveform.press('ArrowRight');
  await waveform.press('ArrowRight');
  await waveformOption(page, studio, /^Zoom in/);
  await expect(studio.locator('.overview .window')).toHaveAttribute('style', /width: 50%/);
  const savedViewport = await studio.locator('.overview .window').getAttribute('style');
  await studio.getByRole('button', { name: 'Close audio studio' }).click();
  const closeDraft = page.getByRole('alertdialog', { name: 'Save split draft?' });
  await expect(closeDraft.getByRole('button', { name: 'Keep editing' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(closeDraft).toBeHidden();
  await expect(studio.getByRole('button', { name: 'Close audio studio' })).toBeFocused();
  await waveform.press('Control+s');
  await expect(studio).toContainText('Draft saved');
  await waveform.press('m');
  await expect(rows(studio)).toHaveCount(3);
  await studio.getByRole('button', { name: 'Close audio studio' }).click();
  await closeDraft.getByRole('button', { name: 'Save & close' }).click();
  await expect(opener).toBeFocused();

  await opener.click();
  await studio.getByRole('button', { name: /^(Reopen|Continue) draft$/ }).click();
  await expect(rows(studio)).toHaveCount(3);
  await expect(rows(studio).nth(1)).toContainText('Second track');
  await expect(rows(studio).last()).toHaveAttribute('aria-pressed', 'true');
  await expect(waveform).toHaveAttribute('aria-valuetext', '0:02.000');
  await expect(studio.locator('.overview .window')).toHaveAttribute('style', savedViewport);
  await reviewAndSave(studio);
  await expect(studio.getByRole('region', { name: 'Output review' })).toContainText('track-2.mp3');
  await expect(studio.getByRole('heading', { name: 'Review & save' })).toBeFocused();
  const review = (await calls(page, 'preview')).at(-1).args.request;
  expect(review).toMatchObject({
    format: 'mp3', bitrate: 192, saveMode: 'beside', destination: '',
    segments: [{ startFrame: 0 }, { startFrame: 48001, tags: { title: 'Second track' } }, { startFrame: 96000 }],
  });
  await studio.getByRole('button', { name: 'Save 3 tracks' }).click();
  await expect(studio.getByRole('region', { name: 'Completed files' })).toContainText('/music/track-3.mp3');
  await expect(studio.getByRole('heading', { name: 'Saved 3 files' })).toBeFocused();
  await studio.getByRole('button', { name: 'Open output folder' }).click();
  expect((await calls(page, 'reveal')).at(-1).args.id).toBe('review-1');
  await studio.getByRole('button', { name: 'Done' }).click();
  await expect(studio).toHaveCount(0);
  expect(await calls(page, 'cancel')).toHaveLength(0);
});

test('file menu imports a CUE with confirmation, encoding and inherited tags', async ({ page }) => {
  await page.setViewportSize({ width: 430, height: 710 });
  await page.evaluate(() => {
    window.studioFixture.picked = '/music/import.cue';
    document.documentElement.dataset.theme = 'light';
  });
  await page.locator('#tracks-body tr[data-path]').first().click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Split recording…' }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await expect(studio).toBeVisible();
  await importCuts(page, studio, 'Import CUE…');
  const importer = studio.getByRole('region', { name: 'CUE import review' });
  await expect(importer.getByRole('heading')).toBeFocused();
  await expect(importer.getByRole('button', { name: 'Use CUE markers' })).toBeDisabled();
  await importer.getByRole('combobox', { name: 'Sheet encoding' }).click();
  await page.getByRole('option', { name: 'Windows-1252' }).click();
  await expect.poll(async () => (await calls(page, 'cue')).at(-1)?.args.encoding).toBe('windows1252');
  await importer.getByRole('checkbox', { name: /as the master/ }).check();
  await importer.getByRole('button', { name: 'Use CUE markers' }).click();
  await expect(rows(studio)).toHaveCount(2);
  await expect(rows(studio).first()).toContainText('CUE first');
  await expect(studio.getByRole('button', { name: 'Start of Track 1 at 0:01.000' })).toBeVisible();
  const shared = studio.getByRole('textbox', { name: 'Recording album artist', exact: true });
  await shared.fill('Album credit');
  await shared.press('Tab');
  await editFromMenu(page, rows(studio).first(), 'Edit Artist/Title');
  const artist = studio.getByRole('textbox', { name: 'Artist for track 1' });
  await expect(artist).toHaveValue('CUE artist');
  await artist.fill('');
  await artist.press('Enter');
  await expect(studio.getByRole('checkbox', { name: 'Include', exact: true })).toHaveCount(0);
  await studio.getByRole('button', { name: 'Export CUE…' }).click();
  await expect(studio.getByRole('status')).toContainText('The saved CUE rounds to CD frames.');
  expect((await calls(page, 'write_cue')).at(-1).args).toMatchObject({
    tags: { album: 'CUE album', albumartist: 'Album credit' },
    segments: [
      { startFrame: 48000, index01: 75, tags: { title: 'CUE first', artist: '', composer: 'CUE composer', isrc: 'GBABC2312345' } },
      { postgap: 4 },
    ],
  });
  await page.keyboard.press('Escape');
  const closeDraft = page.getByRole('alertdialog', { name: 'Save split draft?' });
  await expect(closeDraft.getByRole('button', { name: 'Keep editing' })).toBeFocused();
  await closeDraft.getByRole('button', { name: 'Discard changes' }).click();
  await expect(studio).toHaveCount(0);
});

test('Discogs proposes anchored cuts and makes the remaining audio visible', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await importCuts(page, studio, 'Use Discogs…');
  const importPanel = studio.getByRole('region', { name: 'Discogs timing proposals' });
  await importPanel.getByRole('textbox', { name: 'Release ID or link' }).fill('https://www.discogs.com/release/12345-Example');
  await importPanel.getByRole('button', { name: 'Load tracklist' }).click();
  await expect(importPanel.getByRole('link', { name: 'Data provided by Discogs.' })).toHaveAttribute('href', 'https://www.discogs.com/release/12345');
  await importPanel.getByRole('link', { name: 'Data provided by Discogs.' }).click();
  await expect.poll(() => page.evaluate(() => window.studioFixture.calls.find(call => call.command === 'open_provider_page')?.args)).toMatchObject({ source: 'discogs', entity: 'release', id: '12345' });
  await expect(importPanel.getByRole('button', { name: 'Use proposed cuts' })).toBeDisabled();
  await importPanel.getByRole('button', { name: 'Anchor at 0:00.0' }).click();
  await expect(importPanel).toContainText('Extra audio will appear as its own region.');
  await importPanel.getByRole('button', { name: 'Use proposed cuts' }).click();
  await expect(rows(studio)).toHaveCount(3);
  await expect(rows(studio).last()).toContainText('Unassigned audio');
  await reviewAndSave(studio);
  await expect(studio.getByRole('region', { name: 'Output review' })).toBeVisible();
  expect((await calls(page, 'preview')).at(-1).args.request.tags).toMatchObject({ album: 'Discogs release', genre: 'Electronic' });
  expect((await calls(page, 'discogs')).at(-1).args).toMatchObject({ releaseId: '12345', anchors: [{ startIndex: 0, endIndex: 2, frame: 0 }] });
});

test('Discogs side buttons choose contiguous tracks and allow re-anchoring an earlier side', async ({ page }) => {
  await page.setViewportSize({ width: 430, height: 710 });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await importCuts(page, studio, 'Use Discogs…');
  const importer = studio.getByRole('region', { name: 'Discogs timing proposals' });
  await importer.getByRole('textbox', { name: 'Release ID or link' }).fill('65432');
  await importer.getByRole('button', { name: 'Load tracklist' }).click();
  const sideRanges = importer.getByRole('group', { name: 'Track ranges by side or disc' });
  const sideA = sideRanges.getByRole('button', { name: 'Side A' });
  const sideB = sideRanges.getByRole('button', { name: 'Side B' });
  await expect(sideRanges.getByRole('button')).toHaveText(['Side A', 'Side B', 'Side D']);
  await sideA.press('Enter');
  await expect(sideA).toHaveAttribute('aria-pressed', 'true');
  await expect(importer.getByRole('spinbutton', { name: 'First track' })).toHaveValue('1');
  await expect(importer.getByRole('spinbutton', { name: 'Last track' })).toHaveValue('2');
  const anchor = importer.getByRole('button', { name: /^Anchor at/ });
  await anchor.click();
  await expect(importer.getByText('Tracks 1–2 · 0:00.000')).toBeVisible();

  const waveform = studio.getByRole('slider', { name: 'Waveform playhead' });
  for (let step = 0; step < 4; step++) await waveform.press('ArrowRight');
  await sideB.click();
  await expect(importer.getByRole('spinbutton', { name: 'First track' })).toHaveValue('3');
  await expect(importer.getByRole('spinbutton', { name: 'Last track' })).toHaveValue('4');
  await anchor.click();
  await expect(importer.getByText('Tracks 3–4 · 0:04.000')).toBeVisible();

  await waveform.press('Home');
  await waveform.press('ArrowRight');
  await sideA.click();
  await anchor.click();
  await expect(importer.getByText('Tracks 1–2 · 0:01.000')).toBeVisible();
  expect((await calls(page, 'discogs')).at(-1).args.anchors).toEqual([
    { startIndex: 0, endIndex: 2, frame: 48000 },
    { startIndex: 2, endIndex: 4, frame: 192000 },
  ]);
});

test('whole selection conversion supports Stop and returns to review', async ({ page }) => {
  await page.locator('#ribbon-select-all').click();
  const selected = await page.evaluate(() => [...window.tagrex.selection].length);
  await expect(page.getByRole('button', { name: 'Split recording', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Convert audio', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Convert audio' });
  await expect(studio.getByRole('list', { name: 'Source files' }).getByRole('listitem')).toHaveCount(selected);
  await reviewAndSave(studio);
  await studio.getByRole('region', { name: 'Output review' }).getByRole('button', { name: /^Audition / }).first().click();
  await expect.poll(async () => (await calls(page, 'audition_output')).at(-1)?.args).toMatchObject({ index: 0 });
  await studio.getByRole('button', { name: 'Save alongside original' }).click();
  await expect(studio.getByRole('textbox', { name: 'Save location' })).toBeEnabled();
  await studio.getByRole('button', { name: 'Choose output folder' }).click();
  await expect(studio.getByRole('textbox', { name: 'Save location' })).toHaveValue('/exports');
  await page.evaluate(() => window.studioFixture.slow = true);
  await studio.getByRole('checkbox', { name: 'Create a new folder' }).check();
  const outputFolder = studio.getByRole('combobox', { name: 'Folder name', exact: true });
  await outputFolder.pressSequentially('Portable copies');
  await studio.getByRole('combobox', { name: 'Output', exact: true }).click();
  await page.getByRole('option', { name: 'FLAC (lossless)' }).click();
  const save = studio.getByRole('button', { name: `Save ${selected} files` });
  await save.click();
  await expect(studio.getByRole('progressbar', { name: 'Audio export progress' })).toBeVisible();
  await studio.getByRole('button', { name: 'Stop export' }).click();
  await expect(studio.getByRole('alert')).toHaveText('Export stopped');
  const review = studio.getByRole('button', { name: 'Review files' });
  await expect(review).toBeEnabled();
  expect(await calls(page, 'open')).toHaveLength(0);
  expect((await calls(page, 'preview')).at(-1).args.request).toMatchObject({ format: 'flac', folderPattern: 'Portable copies', saveMode: 'folder', destination: '/exports' });
  await review.click();
  await expect(save).toBeEnabled();
  const cancelled = (await calls(page, 'cancel')).length;
  await studio.getByRole('button', { name: 'Back to editing' }).click();
  await expect.poll(async () => (await calls(page, 'cancel')).length).toBe(cancelled + 1);
  await reviewAndSave(studio);
  await expect(save).toBeVisible();
  await studio.getByRole('button', { name: 'Close audio studio' }).click();
  await expect.poll(async () => (await calls(page, 'cancel')).length).toBe(cancelled + 2);
});

test('save location toggles are exclusive and shared naming presets use tag chips', async ({ page }) => {
  await page.evaluate(() => {
    document.dispatchEvent(new CustomEvent('tagrex:settings-saved', { detail: {
      mask_presets: [{ name: 'My album template', mask: '%albumartist% - %album%' }],
    } }));
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await reviewAndSave(studio);
  const address = studio.getByRole('textbox', { name: 'Save location' });
  const replace = studio.getByRole('button', { name: 'Replace original' });
  const alongside = studio.getByRole('button', { name: 'Save alongside original' });
  const folder = studio.getByRole('checkbox', { name: 'Create a new folder' });
  await expect(address).toBeDisabled();
  await expect(studio.locator('.address').getByRole('button', { name: 'Replace original' })).toBeVisible();
  await expect(studio.locator('.pattern-field').getByRole('button', { name: 'Filename pattern presets' })).toBeVisible();
  await expect(alongside).toHaveAttribute('aria-pressed', 'true');
  await expect(folder).not.toBeChecked();
  await expect(studio.getByRole('button', { name: 'Save 1 track' })).toBeEnabled();
  expect((await calls(page, 'preview')).at(-1).args.request.folderPattern).toBeUndefined();

  await replace.click();
  await expect(alongside).toHaveAttribute('aria-pressed', 'false');
  await expect(replace).toHaveAttribute('aria-pressed', 'true');
  await expect(address).toBeDisabled();
  await replace.click();
  await expect(replace).toHaveAttribute('aria-pressed', 'false');
  await address.fill('/chosen-output');
  await expect.poll(async () => (await calls(page, 'preview')).at(-1).args.request.destination).toBe('/chosen-output');

  await studio.getByRole('button', { name: 'Filename pattern presets' }).click();
  await page.getByRole('menuitem', { name: 'Artist – Title' }).click();
  await expect(studio.getByRole('combobox', { name: 'Filename pattern' }).locator('.field-value-chip')).toHaveText(['Artist', 'Title']);
  await folder.check();
  await studio.getByRole('button', { name: 'Folder name presets' }).click();
  await page.getByRole('menuitem', { name: 'My album template' }).click();
  await expect(studio.getByRole('combobox', { name: 'Folder name' }).locator('.field-value-chip')).toHaveText(['Album Artist', 'Album']);
  await expect.poll(async () => (await calls(page, 'preview')).at(-1).args.request).toMatchObject({
    saveMode: 'folder', destination: '/chosen-output', namePattern: '%artist% - %title%', folderPattern: '%albumartist% - %album%',
  });
  await folder.uncheck();
  await expect(studio.getByRole('combobox', { name: 'Folder name' })).toHaveCount(0);
  await expect(studio.getByRole('button', { name: 'Save 1 track' })).toBeEnabled();
  expect((await calls(page, 'preview')).at(-1).args.request.folderPattern).toBeUndefined();
});

test('replacement checks pending edits, reviews recovery paths and updates the library on save and undo', async ({ page }) => {
  const source = '/music/set.mp3';
  await page.evaluate(path => {
    const fixture = window.studioFixture;
    fixture.picked = '/chosen-recovery';
    fixture.replaceAllowed = false;
    fixture.exportWarnings = ['A recovery artifact is still available.'];
    document.dispatchEvent(new CustomEvent('tagrex:audio-studio', { detail: {
      mode: 'split', paths: [path], opener: null,
      canReplace: () => fixture.replaceAllowed,
      refreshLibrary: async (paths, removed) => { fixture.refreshes.push({ paths, removed }); return true; },
    } }));
  }, source);
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const waveform = studio.getByRole('slider', { name: 'Waveform playhead' });
  await waveform.press('ArrowRight');
  await waveform.press('m');
  await reviewAndSave(studio);
  await expect(studio.getByRole('button', { name: 'Save alongside original' })).toHaveAttribute('aria-pressed', 'true');
  await expect(studio.getByRole('button', { name: 'Save 2 tracks' })).toBeEnabled();
  await studio.getByRole('button', { name: 'Replace original' }).click();
  await expect(studio.getByRole('alert')).toContainText('pending library changes');
  expect((await calls(page, 'preview')).at(-1).args.request.saveMode).toBe('beside');
  await page.evaluate(() => { window.studioFixture.replaceAllowed = true; });
  await studio.getByRole('button', { name: 'Choose recovery folder' }).click();
  await expect(studio.getByText('Originals go to chosen-recovery.')).toBeVisible();
  expect((await calls(page, 'preview')).at(-1).args.request).toMatchObject({ saveMode: 'replace', backupDirectory: '/chosen-recovery' });

  const replace = studio.getByRole('button', { name: 'Replace with 2 tracks' });
  await page.evaluate(() => { window.studioFixture.replaceAllowed = false; });
  await replace.click();
  await expect(studio.getByRole('alert')).toContainText('pending library changes');
  expect(await calls(page, 'export')).toHaveLength(0);
  await page.evaluate(() => { window.studioFixture.replaceAllowed = true; });
  await replace.click();
  await expect(studio.getByRole('region', { name: 'Completed files' })).toContainText('A recovery artifact is still available.');
  const outputPaths = ['/music/track-1.mp3', '/music/track-2.mp3'];
  await expect.poll(() => page.evaluate(() => window.studioFixture.refreshes)).toEqual([{ paths: outputPaths, removed: [source] }]);
  await studio.getByRole('button', { name: 'Undo replacement' }).click();
  await expect(studio.getByRole('heading', { name: 'Restored 1 original' })).toBeFocused();
  await expect.poll(() => page.evaluate(() => window.studioFixture.refreshes)).toEqual([
    { paths: outputPaths, removed: [source] }, { paths: [source], removed: outputPaths },
  ]);
  expect(await calls(page, 'undo_replacement')).toHaveLength(1);
});

test('plain wheel zooms around the pointer and overview navigation preserves playback position', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const overview = studio.getByRole('slider', { name: 'Waveform overview' });
  const playhead = studio.getByRole('slider', { name: 'Waveform playhead' });
  const detail = studio.locator('.detail');
  const windowInOverview = studio.locator('.overview .window');
  const view = () => windowInOverview.evaluate(node => ({ left: parseFloat(node.style.left), width: parseFloat(node.style.width) }));
  const wheel = (dy, shift = false) => detail.evaluate((node, input) => {
    const bounds = node.getBoundingClientRect();
    for (const deltaY of input.dy) {
      node.dispatchEvent(new WheelEvent('wheel', { clientX: bounds.left + bounds.width * 0.75,
        clientY: bounds.top + bounds.height / 2, deltaY, shiftKey: input.shift, cancelable: true, bubbles: true }));
    }
  }, { dy, shift });

  await expect(overview).toBeVisible();
  await wheel([0]);
  expect(await view()).toEqual({ left: 0, width: 100 });
  await wheel([-1, -1, -98]);
  await expect.poll(async () => (await view()).width).toBeLessThan(82);
  const zoomed = await view();
  expect(zoomed.width).toBeGreaterThan(75);
  expect(zoomed.left + zoomed.width * 0.75).toBeCloseTo(75, 1);
  await expect.poll(async () => (await calls(page, 'peaks')).length).toBe(1);

  await wheel([50], true);
  await expect.poll(async () => (await view()).left).toBeGreaterThan(zoomed.left);
  expect((await view()).width).toBeCloseTo(zoomed.width, 2);
  await overview.press('Home');
  await expect.poll(async () => (await view()).left).toBe(0);
  await overview.press('ArrowRight');
  await expect.poll(async () => (await view()).left).toBeGreaterThan(0);
  await expect(playhead).toHaveAttribute('aria-valuenow', '0');
  expect(await calls(page, 'transport')).toHaveLength(0);

  await waveformOption(page, studio, /^Fit recording/);
  await expect.poll(view).toEqual({ left: 0, width: 100 });
});

test('a scrub continues over other controls when native pointer capture is lost', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const playhead = studio.getByRole('slider', { name: 'Waveform playhead' });
  const detail = studio.locator('.detail');
  const outside = studio.getByRole('button', { name: 'Add cut', exact: true });
  await expect(playhead).toBeVisible();
  await playhead.evaluate(node => {
    node.addEventListener('pointerdown', event => { window.scrubPointer = event.pointerId; }, { once: true });
  });
  await outside.evaluate(node => {
    window.scrubOutsideEvents = [];
    for (const type of ['pointerup', 'click']) node.addEventListener(type, () => window.scrubOutsideEvents.push(type));
  });
  const area = await detail.boundingBox();
  const other = await outside.boundingBox();
  await page.mouse.move(area.x + area.width * 0.6, area.y + area.height / 2);
  await page.mouse.down();
  await expect(playhead).toHaveAttribute('aria-valuenow', /[1-9]/);
  await detail.evaluate(node => {
    const pointer = window.scrubPointer;
    for (const target of [node, node.querySelector('.scrubber')]) {
      if (target?.hasPointerCapture(pointer)) target.releasePointerCapture(pointer);
    }
  });
  const x = other.x + other.width / 2;
  const expected = Math.round(480000 * (x - area.x) / area.width);
  await page.mouse.move(x, other.y + other.height / 2, { steps: 6 });
  await expect.poll(async () => Math.abs(Number(await playhead.getAttribute('aria-valuenow')) - expected)).toBeLessThan(750);
  await page.mouse.up();
  await expect.poll(async () => (await calls(page, 'transport')).length).toBe(1);
  expect((await calls(page, 'transport'))[0].args.action).toBe('seek');
  expect(await page.evaluate(() => window.scrubOutsideEvents)).toEqual([]);
  await expect(rows(studio)).toHaveCount(1);
  await outside.click();
  await expect(rows(studio)).toHaveCount(2);
});

test('waveform cut actions play, edit and remove the intended cut', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const playhead = studio.getByRole('slider', { name: 'Waveform playhead' });
  const area = await playhead.boundingBox();
  await page.mouse.click(area.x + area.width / 2, area.y + area.height / 2, { button: 'right' });
  await page.getByRole('menuitem', { name: 'Add cut here' }).click();
  const marker = studio.getByRole('button', { name: /^Cut at / });
  const firstTime = await marker.getAttribute('aria-label');
  expect((await marker.boundingBox()).height).toBeCloseTo(area.height);
  await editFromMenu(page, marker, 'Edit cut time');
  const frame = Number(await studio.getByRole('textbox', { name: 'Track start time' }).inputValue()
    .then(value => value.split(':')[1])) * 48000;
  await page.keyboard.press('Escape');
  expect(frame).toBeCloseTo(240000, -3);
  expect(await calls(page, 'transport')).toHaveLength(0);

  await marker.hover();
  await expect(studio.locator('.marker:hover + .marker-time')).toBeVisible();
  await marker.click();
  await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args).toMatchObject({ action: 'play', frame: Math.round(frame) });
  await marker.press('Enter');
  await expect.poll(async () => (await calls(page, 'transport')).filter(call => call.args.action === 'play').length).toBe(2);

  const cut = await marker.boundingBox();
  await page.mouse.move(cut.x + cut.width / 2, cut.y + cut.height / 2);
  await page.mouse.down();
  await page.mouse.move(cut.x + cut.width / 2 + 45, cut.y + cut.height / 2);
  await page.mouse.up();
  await expect(marker).not.toHaveAttribute('aria-label', firstTime);
  expect((await calls(page, 'transport')).filter(call => call.args.action === 'play')).toHaveLength(2);

  await studio.getByRole('textbox', { name: 'Recording album', exact: true }).focus();
  await marker.click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Edit cut time' }).click();
  const time = studio.getByRole('textbox', { name: 'Track start time' });
  await expect(time).toBeFocused();
  await time.fill('0:06.000000');
  await time.press('Enter');
  await expect(marker).toHaveAttribute('aria-label', 'Cut at 0:06.000');
  await marker.press('Shift+F10');
  await expect(page.getByRole('menuitem', { name: 'Remove cut' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('menuitem', { name: 'Remove cut' })).toBeHidden();
  await expect(marker).toBeFocused();
  await marker.click({ button: 'right' });
  await page.getByRole('menuitem', { name: 'Remove cut' }).click();
  await expect(rows(studio)).toHaveCount(1);
  await expect(playhead).toBeFocused();
  expect(await calls(page, 'transport')).toHaveLength(2);
});

test('middle drag pans without seeking and an edge-held scrub seeks once on release', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const playhead = studio.getByRole('slider', { name: 'Waveform playhead' });
  const overview = studio.locator('.overview .window');
  const left = () => overview.evaluate(node => parseFloat(node.style.left));
  await waveformOption(page, studio, /^Zoom in/);
  await expect(overview).toHaveAttribute('style', /width: 50%/);
  const area = await playhead.boundingBox();
  const y = area.y + area.height / 2;
  await page.mouse.move(area.x + area.width * 0.65, y);
  await page.mouse.down({ button: 'middle' });
  await page.mouse.move(area.x + area.width * 0.45, y, { steps: 5 });
  await page.mouse.up({ button: 'middle' });
  await expect.poll(left).toBeCloseTo(10, 0);
  await expect(playhead).toHaveAttribute('aria-valuenow', '0');
  expect(await calls(page, 'transport')).toHaveLength(0);

  await page.mouse.move(area.x + area.width * 0.5, y);
  await page.mouse.down();
  await page.mouse.move(area.x + area.width - 2, y);
  await expect.poll(left).toBeGreaterThan(18);
  const heldAt = Number(await playhead.getAttribute('aria-valuenow'));
  await expect.poll(async () => Number(await playhead.getAttribute('aria-valuenow'))).toBeGreaterThan(heldAt);
  expect(await calls(page, 'transport')).toHaveLength(0);
  await page.mouse.up();
  const releasedAt = Number(await playhead.getAttribute('aria-valuenow'));
  await expect.poll(async () => (await calls(page, 'transport')).map(call => [call.args.action, call.args.frame])).toEqual([['seek', releasedAt]]);

  await page.mouse.move(area.x + area.width / 2, y);
  await page.mouse.down();
  await page.mouse.move(area.x + 2, y);
  await expect.poll(left).toBeLessThan(17);
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect(playhead).toHaveAttribute('aria-valuenow', String(releasedAt));
  expect(await calls(page, 'transport')).toHaveLength(1);
});

test('the recording form shows album fields and keeps existing draft overrides', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const playhead = studio.getByRole('slider', { name: 'Waveform playhead' });
  await playhead.press('ArrowRight');
  await playhead.press('m');
  const recording = studio.getByRole('region', { name: 'Recording tags' });
  const album = recording.getByRole('textbox', { name: 'Recording album', exact: true });
  await expect(album).toHaveValue('Saved album');
  const tracks = studio.getByRole('region', { name: 'Tracks', exact: true });
  expect((await album.boundingBox()).x).toBeGreaterThan((await tracks.boundingBox()).x);
  await expect(recording.getByRole('textbox', { name: 'Recording comment' })).toBeVisible();
  await expect(recording.getByRole('textbox', { name: 'Recording bpm' })).toBeVisible();
  await expect(recording.getByText('More tags', { exact: true })).toHaveCount(0);
  const artwork = await recording.getByRole('heading', { name: 'Cover art' }).boundingBox();
  const year = await recording.getByRole('textbox', { name: 'Recording year' }).boundingBox();
  expect(artwork.y).toBeGreaterThan(year.y + year.height);
  const draft = studio.getByRole('group', { name: 'Draft actions' });
  await expect(draft.getByRole('button')).toHaveCount(4);
  const left = await tracks.boundingBox();
  const right = await recording.boundingBox();
  expect(left.y).toBeCloseTo(right.y);
  expect(left.height).toBeCloseTo(right.height);
  const tracksHeading = await tracks.getByRole('heading', { name: 'Tracks' }).boundingBox();
  const importButton = await tracks.getByRole('button', { name: 'Import splits' }).boundingBox();
  expect(Math.abs(tracksHeading.y + tracksHeading.height / 2 - importButton.y - importButton.height / 2)).toBeLessThan(2);
  await expect(studio.getByRole('button', { name: 'Zoom in', exact: true })).toHaveCount(0);
  await expect(tracks.getByRole('textbox')).toHaveCount(0);
  const defaultGenre = recording.getByRole('combobox', { name: 'Recording genre', exact: true });
  await defaultGenre.fill('Hardco');
  await expect(page.getByRole('option', { name: 'Hardcore', exact: true })).toBeVisible();
  await defaultGenre.press('ArrowDown');
  await defaultGenre.press('Enter');
  await expect(defaultGenre).toHaveValue('Hardcore');
  await defaultGenre.press('Tab');
  await page.evaluate(() => { window.studioFixture.picked = '/music/cover.png'; });
  await studio.getByRole('button', { name: 'Choose front cover…', exact: true }).click();
  await expect(recording.getByRole('img', { name: 'Front cover' })).toHaveAttribute('src', /^data:image\/png/);
  await studio.getByRole('button', { name: 'Save draft', exact: true }).click();
  const source = (await calls(page, 'open')).at(-1).args.path;
  await page.evaluate(path => {
    const key = 'tagnetic.audio-split-draft:' + path;
    const saved = JSON.parse(localStorage.getItem(key));
    saved.tracks[1].tags.genre = 'Breakbeat';
    localStorage.setItem(key, JSON.stringify(saved));
  }, source);
  await studio.getByRole('button', { name: /^(Reopen|Continue) draft$/ }).click();
  await defaultGenre.fill('Jungle');
  await defaultGenre.press('Tab');
  await studio.getByRole('button', { name: 'Review & save' }).click();
  await expect.poll(async () => (await calls(page, 'preview')).at(-1)?.args.request).toMatchObject({
    artworkPath: '/music/cover.png', tags: { genre: 'Jungle' }, segments: [{ tags: {} }, { tags: { genre: 'Breakbeat' } }],
  });
  await studio.getByRole('button', { name: 'Back to editing' }).click();
  await defaultGenre.hover();
  await studio.getByRole('button', { name: 'Reset recording genre' }).click();
  await expect(defaultGenre).toHaveValue('');
});

test('Track 1 can be trimmed and selecting a row visibly moves the playhead to its start', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  const opener = page.getByRole('button', { name: 'Split recording', exact: true });
  await opener.click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const waveform = studio.getByRole('slider', { name: 'Waveform playhead' });
  const first = studio.getByRole('button', { name: 'Start of Track 1 at 0:00.000' });
  await expect(first).toHaveCSS('left', '0px');
  await expect(studio.locator('.track-header span:first-child')).toHaveText('01');
  await expect(studio.getByRole('button', { name: 'Import splits' })).toHaveCSS('border-top-width', '0px');
  await editFromMenu(page, first, 'Edit start time');
  const time = studio.getByRole('textbox', { name: 'Track start time' });
  await time.fill('0:02');
  await time.press('Enter');
  await expect(studio.getByRole('button', { name: 'Start of Track 1 at 0:02.000' })).toBeVisible();
  await expect(studio.locator('.trimmed-opening')).toHaveAttribute('style', /width: 20%/);
  await waveform.press('End');
  await waveform.press('ArrowLeft');
  const nextFrame = Number(await waveform.getAttribute('aria-valuenow'));
  await waveform.press('m');
  await expect(rows(studio)).toHaveCount(2);
  await expect(studio.locator('.track-header span:first-child')).toHaveText(['01', '02']);
  const samples = await rows(studio).first().evaluate(async row => {
    const playhead = row.closest('[data-audio-studio]').querySelector('.playhead');
    const x = () => new DOMMatrixReadOnly(getComputedStyle(playhead).transform).m41;
    const frames = [x()];
    row.click();
    for (let index = 0; index < 16; index++) {
      await new Promise(resolve => requestAnimationFrame(resolve));
      frames.push(x());
    }
    return frames;
  });
  expect(samples[0]).toBeGreaterThan(samples.at(-1));
  expect(samples.some(x => x < samples[0] && x > samples.at(-1))).toBe(true);
  await expect(waveform).toHaveAttribute('aria-valuenow', '96000');
  await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args).toMatchObject({ action: 'seek', frame: 96000 });
  await studio.getByRole('button', { name: 'Save draft', exact: true }).click();
  await studio.getByRole('button', { name: 'Close audio studio' }).click();
  await opener.click();
  await studio.getByRole('button', { name: /^(Reopen|Continue) draft$/ }).click();
  await editFromMenu(page, studio.getByRole('button', { name: 'Start of Track 1 at 0:02.000' }), 'Edit start time');
  await expect(time).toHaveValue('0:02.000000');
  await time.press('Escape');
  await reviewAndSave(studio);
  await expect.poll(async () => (await calls(page, 'preview')).at(-1)?.args.request.segments).toMatchObject([
    { startFrame: 96000, include: true }, { startFrame: nextFrame, include: true },
  ]);
});

test('wheel zoom eases over successive frames while holding the pointer anchor', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await expect(studio.getByRole('slider', { name: 'Waveform overview' })).toBeVisible();
  const frames = await studio.locator('.audio-split-wave').evaluate(async waveform => {
    const detail = waveform.querySelector('.detail');
    const view = waveform.querySelector('.overview .window');
    const area = detail.getBoundingClientRect();
    for (const deltaY of [-60, -60, -60]) {
      detail.dispatchEvent(new WheelEvent('wheel', {
        clientX: area.left + area.width * 0.75, clientY: area.top + area.height / 2,
        deltaY, cancelable: true, bubbles: true,
      }));
    }
    const samples = [];
    for (let frame = 0; frame < 28; frame++) {
      await new Promise(resolve => requestAnimationFrame(resolve));
      samples.push({ width: parseFloat(view.style.width), left: parseFloat(view.style.left) });
    }
    return samples;
  });
  expect(new Set(frames.map(frame => frame.width)).size).toBeGreaterThan(5);
  expect(frames.at(-1).width).toBeLessThan(75);
  expect(frames.every(frame => Math.abs(frame.left + frame.width * 0.75 - 75) < 0.25)).toBe(true);
  expect(await calls(page, 'peaks')).toHaveLength(1);
});

test('row actions renumber splits and undo an adjacent merge in one step', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const waveform = studio.getByRole('slider', { name: 'Waveform playhead' });
  for (let index = 0; index < 6; index++) {
    await waveform.press('ArrowRight');
    await waveform.press('m');
  }
  const trackRows = rows(studio);
  await expect(trackRows).toHaveCount(7);
  const original = await trackRows.evaluateAll(items => items.map(row => row.dataset.trackId));
  await trackRows.nth(4).press('Delete');
  await expect(trackRows).toHaveCount(6);
  await expect(trackRows.nth(4)).toHaveAttribute('data-track-id', original[5]);
  await expect(trackRows.nth(4).locator('.track-number')).toHaveText('05');
  await expect(trackRows.nth(4)).toBeFocused();
  await expect(studio.locator('.track-header span:first-child')).toHaveText(['01', '02', '03', '04', '05', '06']);
  await trackRows.nth(4).press('Control+z');
  await expect(trackRows).toHaveCount(7);
  await trackRows.first().press('Delete');
  await expect(trackRows.first()).toHaveAttribute('data-track-id', original[0]);

  await trackRows.nth(1).click();
  await trackRows.nth(1).press('Shift+ArrowDown');
  await trackRows.nth(2).press('Shift+ArrowDown');
  await expect(studio.locator('.track[aria-pressed=true]')).toHaveCount(3);
  await editFromMenu(page, trackRows.nth(2), 'Merge adjacent tracks');
  await expect(trackRows).toHaveCount(5);
  expect(await trackRows.evaluateAll(items => items.map(row => row.dataset.trackId))).toEqual([original[0], original[1], ...original.slice(4)]);
  await expect(trackRows.nth(1)).toBeFocused();
  await trackRows.nth(1).press('Control+z');
  await expect(trackRows).toHaveCount(7);

  const area = await waveform.boundingBox();
  await page.mouse.click(area.x + area.width * 0.45, area.y + area.height / 2);
  const frame = Number(await waveform.getAttribute('aria-valuenow'));
  await editFromMenu(page, trackRows.nth(4), 'Move split to playhead');
  const marker = studio.locator(`[data-cut-id="${original[4]}"]`);
  expect(parseFloat(await marker.evaluate(node => node.style.left))).toBeCloseTo(frame / 4800, 4);
});

test('Repair playhead updates carry changing frames without resending a dense mask', async ({ page }, testInfo) => {
  await page.evaluate(() => {
    const fixture = window.studioFixture;
    fixture.repairUpdates = [];
    fixture.statusProbe = ({ sequence }) => {
      const retune = fixture.calls.findLast(call => call.args.action === 'retune');
      return { sequence, frame: 8_000, playing: true,
        ...(fixture.holdRetune && retune ? { retunePending: retune.args.revision } : {}) };
    };
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await studio.getByRole('button', { name: 'Repair', exact: true }).click();
  const mask = {
    startFrame: 20_000, endFrame: 52_000, lowHz: 400, highHz: 3_200, amountDb: 18,
    mask: { version: 1, featherFrames: 0, featherHz: 0, regions: [
      { kind: 'brush', operation: 'add', radiusFrames: 20, radiusHz: 40,
        points: Array.from({ length: 256 }, (_, index) => ({ frame: 20_050 + index * 120, hz: 440 + index * 10 })) },
    ] },
  };
  const send = request => page.evaluate(request => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
    kind: 'repair', sourceId: 'recording', ...request,
  }), request);
  await send({ action: 'edit', revision: 'dense-mask', patch: { spectralRepairs: [mask] } });
  await send({ action: 'play' });
  const updates = () => page.evaluate(() => window.studioFixture.repairUpdates.filter(update => !!update?.playing));
  await expect.poll(async () => new Set((await updates()).map(update => update.frame)).size).toBeGreaterThanOrEqual(5);
  const messages = await updates();
  expect(messages.every(message => !('settings' in message))).toBe(true);
  const initial = await page.evaluate(() => window.studioFixture.repairUpdates.find(update => update.revision === 'dense-mask')?.settings);
  const frameUpdate = messages.at(-1);
  const clockBytes = Buffer.byteLength(JSON.stringify(frameUpdate));
  const fullBytes = Buffer.byteLength(JSON.stringify({ ...frameUpdate, settings: initial }));
  expect(clockBytes / fullBytes).toBeLessThan(0.1);
  const editAt = await page.evaluate(async next => {
    window.studioFixture.holdRetune = true;
    const at = performance.now();
    await window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
      kind: 'repair', sourceId: 'recording', action: 'edit', revision: 'changed-mask', patch: { spectralRepairs: [next] },
    });
    return at;
  }, { ...mask, amountDb: 6 });
  await expect.poll(async () => (await updates()).some(message => message.settings?.spectralRepairs?.[0]?.amountDb === 6)).toBe(true);
  await expect.poll(async () => (await updates()).at(-1)).toMatchObject({ repairPlayback: 'updating', clockRunning: true });
  await page.evaluate(() => { window.studioFixture.holdRetune = false; });
  await expect.poll(async () => (await updates()).at(-1)?.repairPlayback).toBeUndefined();
  const transports = await calls(page, 'transport');
  expect(transports.filter(call => call.args.action === 'play')).toHaveLength(1);
  expect(transports.some(call => call.args.action === 'pause')).toBe(false);
  const retune = transports.findLast(call => call.args.action === 'retune');
  await testInfo.attach('repair-transport-metrics', {
    contentType: 'application/json',
    body: JSON.stringify({ fullBytes, clockBytes, retuneRequestMs: retune.at - editAt }),
  });
});

test('Repair plot shortcuts use the configured audition seam, return point and monitor lane', async ({ page }) => {
  await page.evaluate(() => {
    const fixture = window.studioFixture;
    fixture.stereo = true;
    fixture.statusProbe = ({ sequence }) => ({
      sequence,
      frame: fixture.calls.find(call => call.command === 'audio_split_transport' && call.args.sequence === sequence)?.args.frame ?? 0,
      playing: true,
    });
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await expect(studio.getByRole('button', { name: 'Repair', exact: true })).toBeEnabled();
  const send = request => page.evaluate(request => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
    kind: 'repair', sourceId: 'recording', ...request,
  }), request);
  const lastTransport = async () => (await calls(page, 'transport')).at(-1)?.args;
  const range = { start: 96_000, end: 144_000 };
  await send({ action: 'repair-listen', command: { type: 'preferences', smooth: true, returnTo: 'start' } });
  await send({ action: 'repair-listen', command: { type: 'channel', channel: 'right' } });
  await send({ action: 'loop', range });
  await expect.poll(lastTransport).toMatchObject({
    action: 'audition', frame: range.start, endFrame: range.end, loopStartFrame: range.start,
    smoothLoop: true, monitor: { kind: 'channel', channel: 'right' },
  });
  await send({ action: 'audition-band', range, band: { lowHz: 400, highHz: 1_700, channel: 'left' } });
  await expect.poll(lastTransport).toMatchObject({
    action: 'audition', frame: range.start, endFrame: range.end, loopStartFrame: range.start,
    smoothLoop: true, monitor: { kind: 'band', channel: 'right', lowHz: 400, highHz: 1_700 },
  });
  await send({ action: 'play' });
  await expect.poll(lastTransport).toMatchObject({ action: 'pause' });
  expect((await lastTransport()).frame).toBeGreaterThanOrEqual(range.start);
  await send({ action: 'play' });
  await expect.poll(lastTransport).toMatchObject({
    action: 'audition', endFrame: range.end, loopStartFrame: range.start,
    monitor: { kind: 'band', channel: 'right', lowHz: 400, highHz: 1_700 },
  });
  await send({ action: 'repair-listen', command: { type: 'stop' } });
  await expect.poll(lastTransport).toMatchObject({ action: 'pause', frame: 0 });
});

test('Repair selection arms a filtered loop and changes its boundaries on the running transport', async ({ page }) => {
  await page.evaluate(() => {
    window.studioFixture.statusProbe = ({ sequence }) => ({ sequence, frame: 120_000, playing: true });
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await studio.getByRole('button', { name: 'Repair', exact: true }).click();
  const send = request => page.evaluate(request => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
    kind: 'repair', sourceId: 'recording', ...request,
  }), request);
  const listen = command => send({ action: 'repair-listen', command });
  const transport = () => calls(page, 'transport');
  const lastTransport = async () => (await transport()).at(-1)?.args;
  const selection = { startFrame: 96_000, endFrame: 144_000, lowHz: 400, highHz: 1_700, amountDb: 1 };
  await listen({ type: 'set-selection', selection });
  await expect.poll(lastTransport).toMatchObject({ action: 'seek', frame: selection.startFrame });
  await send({ action: 'play' });
  await expect.poll(lastTransport).toMatchObject({ action: 'audition', frame: selection.startFrame, repeat: true,
    loopStartFrame: selection.startFrame, endFrame: selection.endFrame, previewSelection: selection });
  const { sequence } = await lastTransport();
  const adjustment = { ...selection, startFrame: 110_000, endFrame: 160_000, highHz: 900 };
  await listen({ type: 'set-selection', selection: adjustment, adjust: true });
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', sequence, repeat: true,
    loopStartFrame: adjustment.startFrame, endFrame: adjustment.endFrame, previewSelection: adjustment });
  await listen({ type: 'clear-selection' });
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', sequence, repeat: false,
    loopStartFrame: 0, endFrame: 480_000 });
  expect(await lastTransport()).not.toHaveProperty('previewSelection');
  expect((await transport()).map(call => call.args.action)).toEqual(['seek', 'audition', 'retune', 'retune']);

  await listen({ type: 'set-selection', selection: adjustment });
  await expect.poll(lastTransport).toMatchObject({ action: 'audition', frame: adjustment.startFrame, repeat: true });
  await send({ action: 'seek', range: { start: 125_000, end: 125_001 } });
  await expect.poll(async () => (await transport()).findLast(call => call.args.action === 'seek')?.args)
    .toMatchObject({ frame: 125_000 });
  expect((await transport()).filter(call => call.args.action === 'audition')).toHaveLength(2);
  await send({ action: 'play' });
  await expect.poll(lastTransport).toMatchObject({ action: 'pause' });
  expect((await lastTransport()).frame).toBeGreaterThan(adjustment.startFrame);
  await send({ action: 'play' });
  await expect.poll(lastTransport).toMatchObject({
    action: 'audition', repeat: true, loopStartFrame: adjustment.startFrame,
    endFrame: adjustment.endFrame, previewSelection: adjustment,
  });
  expect((await lastTransport()).frame).toBeGreaterThan(adjustment.startFrame);
  await listen({ type: 'clear-selection', frame: 88_000 });
  await expect.poll(lastTransport).toMatchObject({ action: 'play', frame: 88_000 });
  expect(await lastTransport()).not.toHaveProperty('previewSelection');
});

test('track and Shift-drag loops share playback and the waveform handles', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.evaluate(() => {
    const fixture = window.studioFixture;
    fixture.statusProbe = ({ sequence }) => ({
      sequence,
      frame: fixture.calls.find(call => call.command === 'audio_split_transport' && call.args.sequence === sequence)?.args.frame ?? 0,
      playing: true,
    });
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const waveform = studio.getByRole('slider', { name: 'Waveform playhead' });
  const toggle = studio.getByRole('button', { name: 'Loop preview', exact: true });
  await waveform.press('ArrowRight');
  await waveform.press('m');
  await waveform.press('ArrowRight');
  await waveform.press('m');
  await editFromMenu(page, rows(studio).nth(1), 'Loop track');
  const start = studio.getByRole('slider', { name: 'Loop start', exact: true });
  const end = studio.getByRole('slider', { name: 'Loop end', exact: true });
  const transport = () => calls(page, 'transport');
  await expect.poll(async () => (await transport()).at(-1)?.args).toMatchObject({ action: 'audition', frame: 48000, endFrame: 96000 });
  await expect(toggle).toHaveAttribute('aria-pressed', 'true');
  await expect(start).toHaveAttribute('aria-valuenow', '48000');
  await expect(end).toHaveAttribute('aria-valuenow', '96000');
  await studio.getByRole('button', { name: 'Pause audio' }).click();
  const commandCount = (await transport()).length;
  const wave = await waveform.boundingBox();
  const y = wave.y + wave.height / 2;
  await page.keyboard.down('Shift');
  await page.mouse.move(wave.x + wave.width * 0.30, y);
  await page.mouse.down();
  await page.mouse.move(wave.x + wave.width * 0.60, y, { steps: 5 });
  await page.mouse.up();
  await page.keyboard.up('Shift');
  const first = Number(await start.getAttribute('aria-valuenow'));
  const last = Number(await end.getAttribute('aria-valuenow'));
  expect(first / 48000).toBeCloseTo(3, 1);
  expect(last / 48000).toBeCloseTo(6, 1);
  expect(await transport()).toHaveLength(commandCount);
  await studio.getByRole('button', { name: 'Play audio' }).click();
  await expect.poll(async () => (await transport()).at(-1)?.args).toMatchObject({ action: 'audition', frame: first, endFrame: last });

  const handle = await end.boundingBox();
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2);
  await page.mouse.down();
  await page.mouse.move(handle.x + handle.width / 2 + 30, handle.y + handle.height / 2, { steps: 5 });
  expect(await transport()).toHaveLength(commandCount + 1);
  await page.mouse.up();
  const extended = Number(await end.getAttribute('aria-valuenow'));
  expect(extended).toBeGreaterThan(last);
  await expect.poll(async () => (await transport()).at(-1)?.args).toMatchObject({ action: 'audition', frame: first, endFrame: extended });
  await waveformOption(page, studio, /^Zoom to loop/);
  await expect.poll(() => studio.locator('.overview .window').evaluate(node => parseFloat(node.style.width))).toBeLessThan(50);
  await waveform.press('l');
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  await expect.poll(async () => (await transport()).at(-1)?.args.action).toBe('play');
});

test('choosing a track while playing keeps the editor in place and its controls mounted', async ({ page }) => {
  await page.evaluate(() => {
    const fixture = window.studioFixture;
    fixture.sourceTags = { title: 'Master title', artist: 'Source artist' };
    fixture.statusProbe = ({ sequence }) => ({
      sequence, frame: 48000, playing: true, loading: !!fixture.holdSeek,
    });
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const waveform = studio.getByRole('slider', { name: 'Waveform playhead' });
  for (let index = 0; index < 7; index++) {
    await waveform.press('ArrowRight');
    await waveform.press('m');
  }
  await expect(rows(studio)).toHaveCount(8);
  await studio.getByRole('button', { name: 'Play audio' }).click();
  await expect(studio.locator('button[aria-busy="true"]')).toHaveCount(0);
  const stable = await studio.evaluate(node => {
    const controls = [...node.querySelectorAll('.editor button, .editor input, .editor textarea, .audio-split-wave canvas')];
    const selector = '.editor button, .editor input, .editor textarea, .audio-split-wave canvas';
    const geometry = () => node.querySelector('.editor').getBoundingClientRect().top;
    const initialY = geometry();
    window.checkSplitSelection = () => {
      const current = node.querySelectorAll(selector);
      return {
        displacement: geometry() - initialY,
        sameControls: controls.length === current.length && controls.every((control, index) => current[index] === control),
      };
    };
    const list = node.querySelector('.track-list');
    return {
      visibleTracks: [...list.querySelectorAll('.track')].every(row => {
        const box = row.getBoundingClientRect();
        const times = [...row.querySelectorAll('.track-title, .track-time')];
        return times.every(item => item.getBoundingClientRect().bottom <= box.bottom + 0.5);
      }),
      remainingSpace: node.querySelector('.track-panel').getBoundingClientRect().bottom - list.getBoundingClientRect().bottom,
    };
  });
  expect(stable.visibleTracks).toBe(true);
  expect(stable.remainingSpace).toBeLessThanOrEqual(12);
  await page.evaluate(() => { window.studioFixture.holdSeek = true; });
  await rows(studio).nth(1).click();
  await expect(studio.getByRole('button', { name: 'Pause audio' })).toHaveAttribute('aria-busy', 'true');
  expect(await page.evaluate(() => window.checkSplitSelection())).toEqual({ displacement: 0, sameControls: true });
  await page.mouse.move(0, 0);
  const evenSelection = await rows(studio).nth(1).evaluate(row => getComputedStyle(row).backgroundColor);
  await rows(studio).first().click();
  await page.mouse.move(0, 0);
  const oddSelection = await rows(studio).first().evaluate(row => getComputedStyle(row).backgroundColor);
  expect(evenSelection).not.toBe(oddSelection);
  await page.evaluate(() => { window.studioFixture.holdSeek = false; });
  await expect(studio.locator('button[aria-busy="true"]')).toHaveCount(0);
  expect(await page.evaluate(() => window.checkSplitSelection())).toEqual({ displacement: 0, sameControls: true });
});

test('a delayed playback status cannot pull back the playhead during a new seek', async ({ page }) => {
  await page.evaluate(() => {
    const fixture = window.studioFixture;
    fixture.statusProbe = ({ sequence }) => {
      if (sequence === 1) return new Promise(resolve => { fixture.finishOldStatus = () => resolve({ sequence, frame: 0, playing: true }); });
      return { sequence, frame: 48000, playing: !!fixture.canPlay, loading: !fixture.canPlay };
    };
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const playhead = studio.getByRole('slider', { name: 'Waveform playhead' });
  await studio.getByRole('button', { name: 'Play audio' }).click();
  await expect.poll(() => page.evaluate(() => Boolean(window.studioFixture.finishOldStatus))).toBe(true);
  await playhead.press('ArrowRight');
  await expect.poll(async () => (await calls(page, 'status')).at(-1)?.args.sequence).toBe(2);
  await page.evaluate(() => window.studioFixture.finishOldStatus());
  await expect(studio.getByRole('button', { name: 'Pause audio' })).toHaveAttribute('aria-busy', 'true');
  await expect(playhead).toHaveAttribute('aria-valuenow', '48000');
  await page.evaluate(() => { window.studioFixture.canPlay = true; });
  await expect(studio.locator('button[aria-busy="true"]')).toHaveCount(0);
  await expect.poll(async () => Number(await playhead.getAttribute('aria-valuenow'))).toBeGreaterThan(48000);
});

test('Escape cancels a marker drag without changing the cut or closing the editor', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const playhead = studio.getByRole('slider', { name: 'Waveform playhead' });
  await playhead.press('ArrowRight');
  await playhead.press('m');
  const marker = studio.getByRole('button', { name: 'Cut at 0:01.000', exact: true });
  const bounds = await marker.boundingBox();
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 + 70, bounds.y + bounds.height / 2);
  await expect(studio.locator('.marker-time[data-dragging=true]')).toBeVisible();
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect(studio.locator('.marker-time[data-dragging=true]')).toHaveCount(0);
  await expect(marker).toBeVisible();
  await expect(page.getByRole('alertdialog', { name: 'Save split draft?' })).toBeHidden();
});

test('a long open shows coarse scan progress and can be cancelled without losing focus', async ({ page }) => {
  await page.evaluate(() => { window.studioFixture.slowOpen = true; });
  const launch = page.getByRole('button', { name: 'Split recording', exact: true });
  await launch.click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await expect(studio.getByRole('progressbar', { name: 'Recording scan progress' })).toBeVisible();
  await studio.getByRole('button', { name: 'Close audio studio' }).click();
  await expect(launch).toBeFocused();
  await expect.poll(async () => (await calls(page, 'cancel_open')).length).toBe(1);
  expect((await calls(page, 'cancel_open'))[0].args.requestId).toBe((await calls(page, 'open'))[0].args.requestId);
});

test('audio draft shortcuts and closing preserve pending library changes', async ({ page }) => {
  await openTagSource(page, 'editor');
  await page.locator('#editor-field-title').fill('Pending library tag');
  await expect.poll(() => page.evaluate(() => window.tagrex.querySession.edits)).toBe(1);
  const before = await page.evaluate(() => window.tagrex.querySession);
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await expect(studio).toContainText('Audio tools use the saved tags from disk.');
  const waveform = studio.getByRole('slider', { name: 'Waveform playhead' });
  await waveform.press('ArrowRight');
  await waveform.press('m');
  await waveform.press('Control+z');
  await expect(rows(studio)).toHaveCount(1);
  await studio.getByRole('button', { name: 'Close audio studio' }).click();
  await page.getByRole('alertdialog', { name: 'Save split draft?' }).getByRole('button', { name: 'Discard changes' }).click();
  await expect(page.locator('#editor-field-title')).toHaveValue('Pending library tag');
  expect(await page.evaluate(() => window.tagrex.querySession)).toEqual(before);
});

test('stereo waveform uses independent channel data and keeps one shared cut and playhead', async ({ page }) => {
  await page.evaluate(() => { window.studioFixture.stereo = true; });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const playhead = studio.getByRole('slider', { name: 'Waveform playhead' });
  await playhead.press('ArrowRight');
  await playhead.press('m');
  const toggle = studio.getByRole('button', { name: 'Show left and right channels' });
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-pressed', 'true');
  await expect(studio.getByText('Left', { exact: true })).toBeVisible();
  await expect(studio.getByText('Right', { exact: true })).toBeVisible();
  await expect(studio.locator('.track-header span:first-child')).toHaveText(['01', '02']);
  await expect(studio.locator('.playhead')).toHaveCount(1);
  await expect(studio.locator('.marker')).toHaveCount(2);
  const strongestRows = (fraction = 0.37) => studio.locator('.detail canvas').evaluate((canvas, fraction) => {
    const alpha = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
    const centreX = Math.round(canvas.width * fraction);
    return [0, 1].map(channel => {
      let audible = 0;
      for (let y = Math.round(channel * canvas.height / 2); y < (channel + 1) * canvas.height / 2; y++) {
        if (alpha[(y * canvas.width + centreX) * 4 + 3] > 120) audible++;
      }
      return audible;
    });
  }, fraction);
  await expect.poll(async () => (await strongestRows())[0]).toBeGreaterThan(10);
  expect((await strongestRows())[1]).toBeLessThanOrEqual(3);
  // With matching extrema, the real energy layer gets wider only where
  // the sustained signal is stronger.
  expect((await strongestRows(0.7))[0]).toBeGreaterThan((await strongestRows(0.3))[0] * 2);
  await waveformOption(page, studio, 'Zoom in');
  await expect.poll(async () => (await calls(page, 'peaks')).at(-1)?.args).toMatchObject({ stereo: true, energy: true });
  await expect.poll(async () => (await strongestRows())[0]).toBeGreaterThan(10);
  expect((await strongestRows())[1]).toBeLessThanOrEqual(3);
});

test('live treatment uses the same loop before analysis finishes and reviews the chosen settings', async ({ page }) => {
  await page.evaluate(() => { Object.assign(window.studioFixture, { stereo: true, originalFormat: 'mp3', holdTreatment: true }); });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const dynamics = studio.getByRole('button', { name: 'Dynamics', exact: true });
  const repair = studio.getByRole('button', { name: 'Repair', exact: true });
  const request = payload => page.evaluate(payload => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
    sourceId: 'recording', ...payload,
  }), payload);
  await dynamics.click();
  await expect.poll(async () => (await calls(page, 'tool_window')).findLast(({ args }) => args.action === 'toggle')?.args).toEqual({ kind: 'dynamics', action: 'toggle' });
  await request({ kind: 'dynamics', action: 'loop' });
  await expect.poll(async () => (await calls(page, 'transport')).some(({ args }) => args.action === 'audition')).toBe(true);
  const initial = (await calls(page, 'transport')).find(({ args }) => args.action === 'audition');
  expect(initial.args.endFrame - initial.args.frame).toBe(8 * 48000);
  await request({ kind: 'dynamics', action: 'edit', revision: '1', patch: { normalize: true } });
  await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args).toMatchObject({ action: 'retune', sequence: initial.args.sequence, treatment: { normalize: true } });
  await repair.click();
  await request({ kind: 'repair', action: 'edit', revision: '2', patch: { mono: 'right', hum: 'hz50', normalize: false } });
  const recipe = { normalize: true, mono: 'right', rumble: false, hum: 'hz50' };
  await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args.treatment).toMatchObject(recipe);
  await expect(dynamics).toHaveAttribute('data-active', 'true');
  await expect(repair).toHaveAttribute('data-active', 'true');
  await repair.click();
  await expect(repair).toHaveAttribute('data-active', 'true');
  await request({ kind: 'dynamics', action: 'compare', original: true });
  await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args).toMatchObject({ action: 'retune', sequence: initial.args.sequence, treatment: { normalize: false, mono: 'off', hum: 'off' } });
  await request({ kind: 'dynamics', action: 'compare', original: false });
  await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args).toMatchObject({ action: 'retune', sequence: initial.args.sequence, treatment: recipe });
  expect((await calls(page, 'transport')).filter(({ args }) => args.action === 'play' || args.action === 'audition')).toHaveLength(1);
  await page.evaluate(() => {
    window.studioFixture.holdTreatment = false;
    window.studioFixture.treatmentWaiters?.forEach(resolve => resolve());
  });
  await reviewAndSave(studio);
  await expect.poll(async () => (await calls(page, 'preview')).at(-1)?.args.request).toMatchObject({
    splitMethod: 'exact', treatment: recipe, format: 'mp3',
  });
  await expect(studio.getByText('Exact cuts · Audio treated')).toBeVisible();
});

test('effect gestures undo as one change and stale tool windows cannot change a new source', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const eq = studio.getByRole('button', { name: 'EQ', exact: true });
  await eq.click();
  await expect.poll(() => page.evaluate(() => window.studioFixture.toolSnapshots?.['audio-tool-eq']?.sourceId)).toBe('recording');
  await page.evaluate(async () => {
    const edit = payload => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
      kind: 'eq', action: 'edit', gesture: 'drag', ...payload,
    });
    await edit({ sourceId: 'old-recording', revision: '1', patch: { rumble: true } });
  });
  await expect(eq).toHaveAttribute('data-active', 'false');
  await page.evaluate(async () => {
    for (const bassDb of [1, 2, 3]) {
      await window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
        kind: 'eq', action: 'edit', sourceId: 'recording', gesture: 'drag', revision: String(bassDb), patch: { bassDb },
      });
    }
  });
  await expect(eq).toHaveAttribute('data-active', 'true');
  await studio.getByRole('button', { name: 'Undo draft change' }).click();
  await expect(eq).toHaveAttribute('data-active', 'false');
  await studio.getByRole('button', { name: 'Redo draft change' }).click();
  await expect(eq).toHaveAttribute('data-active', 'true');
});

test('an open audio tool repaints when the main palette, accent and interface font change', async ({ page, context }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await studio.getByRole('button', { name: 'EQ', exact: true }).click();
  const target = 'audio-tool-eq';
  await expect.poll(() => page.evaluate(target => window.studioFixture.toolSnapshots?.[target]?.sourceId, target)).toBe('recording');

  const tool = await context.newPage();
  await tool.route('http://tagrex.test/**', serveFixture);
  await tool.addInitScript(() => {
    const handlers = new Map();
    window.__TAURI__ = {
      event: {
        async listen(name, callback) { handlers.set(name, callback); return () => handlers.delete(name); },
        async emitTo() {},
      },
    };
    window.receiveSnapshot = payload => handlers.get('tagnetic:audio-tools-state')?.({ payload });
  });
  await tool.goto('http://tagrex.test/app/ui/audio-tools.html?tool=eq');
  await expect(tool.getByRole('button', { name: 'Play', exact: true })).toBeDisabled();
  const publish = async () => {
    const snapshot = await page.evaluate(target => window.studioFixture.toolSnapshots[target], target);
    await tool.evaluate(state => window.receiveSnapshot(state), snapshot);
  };
  await expect(async () => {
    await publish();
    await expect(tool.getByRole('button', { name: 'Play', exact: true })).toBeEnabled({ timeout: 350 });
  }).toPass();
  const before = await tool.locator('canvas').evaluate(canvas => canvas.toDataURL());
  await page.evaluate(() => {
    Object.assign(document.documentElement.dataset, { theme: 'light', palette: 'plum', uiFont: 'mono' });
    document.documentElement.style.setProperty('--accent', '#864c28');
    document.dispatchEvent(new CustomEvent('tagrex:appearance'));
  });
  await expect.poll(() => page.evaluate(target => window.studioFixture.toolSnapshots[target].theme.palette, target)).toBe('plum');
  await publish();

  const appearance = page => page.evaluate(() => {
    const root = document.documentElement;
    const style = getComputedStyle(root);
    return [root.dataset.theme, root.dataset.palette, root.dataset.uiFont, ...['--bg', '--panel', '--accent', '--text'].map(key => style.getPropertyValue(key).trim())];
  });
  expect(await appearance(tool)).toEqual(await appearance(page));
  await expect.poll(() => tool.locator('canvas').evaluate(canvas => canvas.toDataURL())).not.toBe(before);
});

test('saving a draft waits for a hidden tool to send its pending edit', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const eq = studio.getByRole('button', { name: 'EQ', exact: true });
  await eq.click();
  await page.evaluate(() => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', { kind: 'eq', action: 'sync' }));
  await eq.click();
  const save = studio.getByRole('button', { name: 'Save draft', exact: true });
  await save.click();
  await expect(save).toBeDisabled();
  await expect.poll(() => page.evaluate(() => window.studioFixture.toolFence)).toMatch(/^\d+$/);
  await page.evaluate(async () => {
    const send = args => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', { kind: 'eq', sourceId: 'recording', ...args });
    await send({ action: 'edit', revision: '1', patch: { rumble: true } });
    await send({ action: 'flushed', fence: window.studioFixture.toolFence });
  });
  await expect(save).toBeEnabled();
  await studio.getByRole('button', { name: 'Undo draft change' }).click();
  await expect(eq).toHaveAttribute('data-active', 'false');
  await studio.getByRole('button', { name: 'Reopen draft' }).click();
  await expect(eq).toHaveAttribute('data-active', 'true');
});

test('treatment recipes restore reusable settings and preserve the current repair areas', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const waveformOptions = studio.getByRole('button', { name: 'Waveform options' });
  const eq = studio.getByRole('button', { name: 'EQ', exact: true });
  const repair = studio.getByRole('button', { name: 'Repair', exact: true });
  await eq.click();
  const edit = (kind, patch) => page.evaluate(({ kind, patch }) => window.__TAURI__.event.emitTo('main',
    'tagnetic:audio-tools-command', { kind, patch, action: 'edit', sourceId: 'recording' }), { kind, patch });
  const area = { startFrame: 48000, endFrame: 96000, lowHz: 500, highHz: 1000, amountDb: 12 };
  await edit('eq', { rumble: true });
  await edit('repair', { spectralRepairs: [area] });
  await expect(eq).toHaveAttribute('data-active', 'true');
  await waveformOptions.click();
  await page.getByRole('menuitem', { name: 'Treatment recipes…' }).click();
  const recipes = page.getByRole('dialog', { name: 'Treatment recipes' });
  await recipes.getByRole('textbox', { name: 'Recipe name' }).fill('Tape cleanup');
  await recipes.getByRole('button', { name: 'Save current settings' }).click();
  await expect(recipes.getByRole('button', { name: 'Tape cleanup', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(recipes).toBeHidden();
  await expect(waveformOptions).toBeFocused();
  await edit('eq', { rumble: false });
  await edit('repair', { spectralRepairs: [{ ...area, startFrame: 72000 }] });
  await repair.click();
  await expect(eq).toHaveAttribute('data-active', 'false');
  await waveformOptions.click();
  await page.getByRole('menuitem', { name: 'Treatment recipes…' }).click();
  await recipes.getByRole('button', { name: 'Tape cleanup', exact: true }).click();
  await expect(eq).toHaveAttribute('data-active', 'true');
  await expect.poll(() => page.evaluate(() => window.studioFixture.toolSnapshots?.['audio-tool-repair']?.settings.spectralRepairs?.[0]?.startFrame)).toBe(72000);
  await expect(studio).toBeVisible();
});

test('EQ still receives peak levels when its spectrum is disabled', async ({ page }) => {
  await page.evaluate(() => {
    window.studioFixture.treatmentProbe = ({ spectrum }) => ({
      frequencies: spectrum ? [120, 3000] : [], inputPeak: -14, outputPeak: -5,
    });
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  await page.getByRole('dialog', { name: 'Split recording' }).getByRole('button', { name: 'EQ', exact: true }).click();
  await expect.poll(async () => (await calls(page, 'probe')).at(-1)?.args.spectrum).toBe(true);
  await page.evaluate(() => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
    kind: 'eq', action: 'analyzer', sourceId: 'recording', enabled: false,
  }));
  await expect.poll(async () => (await calls(page, 'probe')).at(-1)?.args.spectrum).toBe(false);
  await expect.poll(() => page.evaluate(() => window.studioFixture.toolProbes?.['audio-tool-eq']?.probe)).toMatchObject({
    frequencies: [], inputPeak: -14, outputPeak: -5,
  });
});

test('the live analyzer keeps receiving audible spectra during EQ edits', async ({ page }) => {
  await page.evaluate(() => {
    window.studioFixture.treatmentProbe = ({ playback }) => {
      if (!playback) return {};
      return new Promise(resolve => { window.studioFixture.resolvePlaybackProbe ??= resolve; });
    };
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await studio.getByRole('button', { name: 'EQ', exact: true }).click();
  await studio.getByRole('button', { name: 'Play audio', exact: true }).click();
  await page.waitForFunction(() => typeof window.studioFixture.resolvePlaybackProbe === 'function');
  await page.evaluate(async () => {
    await window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
      kind: 'eq', action: 'edit', sourceId: 'recording', patch: { presenceDb: -4 },
    });
    window.studioFixture.resolvePlaybackProbe({
      fromPlayback: true, frequencies: [120, 3000], original: [-30, -22], treated: [-32, -27],
      outputPeak: -11, dynamicEqMix: [0.6], dynamicEqDetectors: [-24],
    });
  });
  const shown = () => page.evaluate(() => window.studioFixture.toolProbes?.['audio-tool-eq']?.probe);
  await expect.poll(shown).toMatchObject({ outputPeak: -11, original: [-30, -22], treated: [-32, -27] });
  expect((await shown()).dynamicEqMix).toBeUndefined();
});

test('EQ threshold learning follows the active loop and is undoable', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await studio.getByRole('button', { name: 'EQ', exact: true }).click();
  const request = payload => page.evaluate(payload => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
    kind: 'eq', sourceId: 'recording', ...payload,
  }), payload);
  const settings = () => page.evaluate(() => window.studioFixture.toolSnapshots?.['audio-tool-eq']?.settings);
  await request({ action: 'edit', revision: 'a', patch: { eqBands: [{ id: 1, kind: 'bell', hz: 900, gainDb: 0, q: 130,
    dynamic: { thresholdDb: -24, amountDb: 4, ratio: 3 } }] } });
  await request({ action: 'loop', range: { start: 48000, end: 120000 } });
  await request({ action: 'learn-threshold', index: 0 });
  await expect.poll(async () => (await calls(page, 'eq_learn_threshold')).at(-1)?.args).toMatchObject({
    bandIndex: 0, range: { start: 48000, end: 120000 },
  });
  await expect.poll(settings).toMatchObject({ eqBands: [{ dynamic: { thresholdDb: -32.5, amountDb: 4, ratio: 3 } }] });
  await request({ action: 'undo', revision: 'b' });
  await expect.poll(settings).toMatchObject({ eqBands: [{ dynamic: { thresholdDb: -24 } }] });
});

test('A/B EQ curves share a measured level in preview and retain the selected EQ for export', async ({ page }) => {
  await page.evaluate(() => {
    window.studioFixture.treatmentProbe = ({ treatment }) => ({ input: -30, output: -30 + treatment.presenceDb });
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await studio.getByRole('button', { name: 'EQ', exact: true }).click();
  const send = request => page.evaluate(request => window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
    kind: 'eq', sourceId: 'recording', ...request,
  }), request);
  const lastTransport = async () => (await calls(page, 'transport')).at(-1)?.args;
  const setComparison = (current, alternate) => send({
    action: 'edit', patch: { presenceDb: current, presenceHz: 1000 },
    compareEq: { presenceDb: alternate, presenceHz: 1000 },
  });
  await send({ action: 'play' });
  await setComparison(4, -6);
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', previewGainDb: -10, treatment: { presenceDb: 4 } });
  await setComparison(-6, 4);
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', treatment: { presenceDb: -6 } });
  expect(await lastTransport()).not.toHaveProperty('previewGainDb');
  await reviewAndSave(studio);
  const review = (await calls(page, 'preview')).at(-1)?.args.request;
  expect(review.treatment).toMatchObject({ presenceDb: -6 });
  expect(review).not.toHaveProperty('previewGainDb');
});

test('Space restarts a finished recording from the first cut and resumes from an ordinary pause', async ({ page }) => {
  await page.evaluate(() => {
    window.studioFixture.statusProbe = ({ sequence }) => window.studioFixture.finishPlayback
      ? { sequence, frame: 479999, playing: false }
      : { sequence, frame: 72000, playing: true };
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const wave = studio.getByRole('slider', { name: 'Waveform playhead' });
  const firstCut = studio.getByRole('button', { name: /^Start of Track 1 at/ });
  const transport = async () => (await calls(page, 'transport')).at(-1)?.args;
  // Shift-Right moves the first cut by 10 ms without moving the playhead.
  await firstCut.press('Shift+ArrowRight');
  await wave.press('Space');
  await expect.poll(transport).toMatchObject({ action: 'play' });
  await expect.poll(async () => Number(await wave.getAttribute('aria-valuenow'))).toBeGreaterThanOrEqual(72000);
  await wave.press('Space');
  await expect.poll(transport).toMatchObject({ action: 'pause' });
  const paused = (await transport()).frame;
  await page.evaluate(() => { window.studioFixture.finishPlayback = true; });
  await wave.press('Space');
  await expect.poll(transport).toMatchObject({ action: 'play', frame: paused });
  await expect(wave).toHaveAttribute('aria-valuenow', '479999');
  await expect(studio.getByRole('button', { name: 'Play audio' })).toBeVisible();
  await page.evaluate(() => { window.studioFixture.finishPlayback = false; });
  await wave.press('Space');
  await expect.poll(transport).toMatchObject({ action: 'play', frame: 480 });
});

test('level matching and channel audition stay in preview while EQ treatment is retained for export', async ({ page }) => {
  await page.evaluate(() => {
    window.studioFixture.stereo = true;
    window.studioFixture.treatmentProbe = ({ treatment }) => treatment.gainDb === 8
      ? { input: -30, output: -22 } : { input: -92, output: -92 };
  });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await studio.getByRole('button', { name: 'Dynamics', exact: true }).click();
  const request = (action, data = {}, kind = 'dynamics') => page.evaluate(payload =>
    window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', { sourceId: 'recording', ...payload }), { action, kind, ...data });
  const lastTransport = async () => (await calls(page, 'transport')).at(-1)?.args;
  await request('edit', { patch: { gainDb: 8 } });
  await request('play');
  await request('level-match');
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', previewGainDb: -8, treatment: { gainDb: 8 } });
  await request('edit', { patch: { gainDb: 4 } });
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', treatment: { gainDb: 4 } });
  expect(await lastTransport()).not.toHaveProperty('previewGainDb');
  await request('audition-band', { range: { start: 48000, end: 96000 }, band: { lowHz: 500, highHz: 1000, channel: 'right' } }, 'repair');
  await expect.poll(lastTransport).toMatchObject({ action: 'audition', monitor: { kind: 'band', lowHz: 500, highHz: 1000, channel: 'right' }, treatment: { mono: 'off', gainDb: 0 } });
  await request('stop-audition', {}, 'repair');
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', treatment: { mono: 'off', gainDb: 4 } });
  expect(await lastTransport()).not.toHaveProperty('monitor');
  const { sequence } = await lastTransport();
  await request('solo-band', { band: { lowHz: 500, highHz: 1000, channel: 'mid' } }, 'eq');
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', sequence, monitor: { lowHz: 500, highHz: 1000, channel: 'mid' } });
  await request('retune-solo-band', { band: { lowHz: 750, highHz: 1500, channel: 'side' } }, 'eq');
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', sequence, monitor: { lowHz: 750, highHz: 1500, channel: 'side' } });
  await request('stop-audition', {}, 'eq');
  await request('retune-solo-band', { band: { lowHz: 600, highHz: 1200 } }, 'eq');
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', sequence, treatment: { gainDb: 4 } });
  expect(await lastTransport()).not.toHaveProperty('monitor');
  const eqPatch = { eqStrength: 65, eqBands: [{ id: 1, kind: 'bell', hz: 1000, gainDb: -3, q: 100, channel: 'side',
    dynamic: { amountDb: 6, thresholdDb: -30, ratio: 3, spectral: true } }] };
  await request('edit', { patch: eqPatch }, 'eq');
  await expect.poll(lastTransport).toMatchObject({ action: 'retune', sequence, treatment: eqPatch });
  await reviewAndSave(studio);
  await expect.poll(async () => (await calls(page, 'preview')).at(-1)?.args.request.treatment).toMatchObject({ gainDb: 4, mono: 'off', ...eqPatch });
});

test('reference EQ auditions without changing edits, then keeps the fit as one undo step', async ({ page }) => {
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  const eq = studio.getByRole('button', { name: 'EQ', exact: true });
  await eq.click();
  const request = (action, extra = {}) => page.evaluate(payload =>
    window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', { kind: 'eq', sourceId: 'recording', ...payload }),
    { action, ...extra });
  const snapshot = () => page.evaluate(() => window.studioFixture.toolSnapshots?.['audio-tool-eq']);
  await request('edit', { revision: '1', patch: { rumble: true } });
  await expect.poll(async () => (await snapshot()).settings.rumble).toBe(true);
  await request('reference-start');
  await expect.poll(async () => (await snapshot()).reference?.status).toBe('ready');
  await request('reference-preview');
  await expect.poll(async () => (await calls(page, 'transport')).at(-1)?.args.treatment.eqBands?.[0]?.hz).toBe(2500);
  const { settings, reference } = await snapshot();
  const fit = settings.eqBands[0];
  const trial = reference.generation;
  await request('edit', { revision: 'trial-1', trial, patch: { eqBands: [{ ...fit, gainDb: -5 }] } });
  await expect.poll(async () => (await snapshot()).settings.eqBands?.[0]?.gainDb).toBe(-5);
  await request('undo', { revision: 'trial-2', trial });
  await expect.poll(async () => (await snapshot()).settings.eqBands?.[0]?.gainDb).toBe(-4);
  await request('redo', { revision: 'trial-3', trial });
  await expect.poll(async () => (await snapshot()).settings.eqBands?.[0]?.gainDb).toBe(-5);
  await request('reference-cancel');
  await request('edit', { revision: 'late-trial', trial, patch: { eqBands: [{ ...fit, gainDb: 6 }] } });
  await expect.poll(async () => (await snapshot()).settings.rumble).toBe(true);
  await expect.poll(async () => (await snapshot()).settings.eqBands).toBeNull();
  await request('reference-start');
  await expect.poll(async () => (await snapshot()).reference?.status).toBe('ready');
  await request('reference-keep');
  await expect.poll(async () => (await snapshot()).settings.eqBands?.[0]?.gainDb).toBe(-4);
  await expect.poll(async () => (await snapshot()).undoLabel).toBe('Undo EQ reference match');
  await request('undo', { revision: '2' });
  await expect.poll(async () => (await snapshot()).settings.rumble).toBe(true);
});

test('requests the full spectrogram once and reuses it through transport seeks', async ({ page }) => {
  await page.evaluate(() => { window.studioFixture.sampleRate = 16_000; });
  await page.getByRole('button', { name: 'Split recording', exact: true }).click();
  const studio = page.getByRole('dialog', { name: 'Split recording' });
  await studio.getByRole('button', { name: 'Repair', exact: true }).click();

  const probes = async () => (await calls(page, 'probe')).filter(({ args }) => args.spectrogram);
  const wholeRecording = { start: 0, end: 480_000 };
  await expect.poll(async () => (await probes())[0]?.args).toMatchObject({
    viewport: wholeRecording, spectrogram: true, spectrogramTreated: true, spectrogramDifference: false,
  });
  const viewport = (range, options) => page.evaluate(({ range, options }) =>
    window.__TAURI__.event.emitTo('main', 'tagnetic:audio-tools-command', {
      sourceId: 'recording', kind: 'repair', action: 'viewport', range, options,
    }), { range, options });
  await viewport(wholeRecording, { overview: true, treated: true, difference: true });
  await expect.poll(async () => (await probes()).at(-1)?.args).toMatchObject({
    viewport: wholeRecording, spectrogram: true, spectrogramTreated: true, spectrogramDifference: true,
  });
  const count = (await probes()).length;
  await studio.getByRole('slider', { name: 'Waveform playhead' }).press('End');
  // Give the 125 ms tool timer multiple opportunities to run after the seek.
  await page.waitForTimeout(340);
  expect(await probes()).toHaveLength(count);

  const local = { start: 28_000, end: 72_000 };
  await viewport(local, { channel: 'combined', treated: true, difference: true });
  await expect.poll(async () => (await probes()).at(-1)?.args).toMatchObject({
    viewport: local, spectrogram: true, spectrogramTreated: true, spectrogramDifference: true,
  });
});
