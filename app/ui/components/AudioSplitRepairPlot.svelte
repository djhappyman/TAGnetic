<script lang="ts">
  import { onDestroy, tick, untrack, type Snippet } from 'svelte';
  import { devicePixelRatio } from 'svelte/reactivity/window';
  import { Check, Crosshair, Headphones, Lasso, Layers, LocateFixed, Maximize2, MoveHorizontal, MoveVertical, Paintbrush, Play, RectangleHorizontal, SlidersHorizontal, Trash2, X } from '@lucide/svelte';
  import { Button as PlotButton } from '@ui-sdk/svelte/button';
  import { Input } from '@ui-sdk/svelte/input';
  import { createMenuController, Menu, MenuCheckboxItem, MenuContent, MenuGroup, MenuGroupLabel, MenuItem, MenuItemIndicator, MenuRadioGroup, MenuRadioItem, MenuSeparator, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';
  import Button from './AudioSplitButton.svelte';
  import AudioSplitRepairAnalysis from './AudioSplitRepairAnalysis.svelte';
  import { plotBounds, type TreatmentProbe } from '../js/audio-treatment-plot';
  import {
    boundedTime, buildRepairTexture, clampRepair, drawRepairOverview, drawRepairViewport, frameAtX, frameX, frequencyAtY,
    frequencyY, interpolateRepairTime, recordingFrequencyRange, repairDbScale, repairLevelAt, repairPalettes, repairTimeLabel, spectralWindowLimit, spectrogramRange,
    type FrequencyRange, type RepairChannel, type RepairContrast, type RepairPaletteName, type RepairTexture, type RepairViewport, type RepairWaveform,
  } from '../js/audio-repair-plot';
  import { edgePanSpeed, editRepairBounds, remapRepairMask, repairDragAt, repairDragCursor, repairMaskBounds, repairRegionPath,
    type RepairBounds, type RepairDrag, type RepairSelectionTool } from '../js/audio-repair-selection';
  import { maxRepairMaskPoints, maxRepairMaskRegions, maxRepairMaskTotalPoints, maxSpectralRepairs, sameRepairBounds, sameSavedRepair,
    type RepairMaskPoint, type RepairMaskRegion, type SpectralRepair, type AudioTreatment } from '../js/audio-treatment';
  import type { PreviewBand, SpectrogramViewOptions, ToolAction, ToolSnapshot } from '../js/audio-tools-window';
  import { projectRepairFrame, repairClockHorizon, type RepairClock } from '../js/audio-repair-clock';
  import { frameAtTime, timeLabel, type FrameRange } from '../js/audio-split';

  let { snapshot, probe, onviewport, onedit, onaction, modeRail, listen }: {
    snapshot: ToolSnapshot | null;
    probe: TreatmentProbe | null;
    onviewport: (range: FrameRange | null, options?: SpectrogramViewOptions) => void;
    onedit: (patch: Partial<AudioTreatment>, gesture?: string) => void;
    onaction: (action: ToolAction, range?: FrameRange, band?: PreviewBand) => void;
    modeRail: Snippet;
    listen?: Snippet<[FrameRange | undefined, PreviewBand | undefined, SpectralRepair | undefined, number | undefined]>;
  } = $props();

  type Pick = Omit<SpectralRepair, 'amountDb'> & { index?: number; saved?: SpectralRepair };
  type Gesture = {
    pointer: number; node: HTMLButtonElement; rect: DOMRect;
    firstX: number; firstY: number; firstFrame: number; firstHz: number;
    x: number; y: number; mode: 'pick' | 'pan' | 'edit'; moved: boolean;
    origin: RepairViewport; previous: RepairViewport | null; lane: RepairChannel;
    tool: RepairSelectionTool; operation: 'replace' | 'add' | 'subtract'; points: RepairMaskPoint[]; frequencyTime: FrameRange;
    brush: { radiusFrames: number; radiusHz: number };
    baseline?: { pick: Pick; handles: RepairDrag; amount: number };
  };
  type Detail = 'balanced' | 'transients' | 'tones';
  type DisplayChannel = RepairChannel | 'stereo';
  type ViewOptions = { treated: boolean; difference: boolean; channel: DisplayChannel; waveform: boolean; waveformOnly: boolean; opacity: number;
    contrast: RepairContrast; palette: RepairPaletteName; offset: number; detail: Detail };
  type ProbeView = { treated: boolean; difference?: boolean; channel: DisplayChannel; detail?: Detail };
  type TimeEdge = 'startFrame' | 'endFrame' | 'duration';
  type FrequencyEdge = 'lowHz' | 'highHz';
  type FrequencyScope = 'view' | 'loop' | 'recording';
  const defaultAmount = 12;
  const editLimit = maxSpectralRepairs;
  const editMenu = createMenuController();
  const viewMenu = createMenuController();
  const channelMenu = createMenuController();
  const plotHelpID = $props.id();
  const noRepairs: readonly (SpectralRepair | null)[] = [];
  const channels: readonly RepairChannel[] = ['combined', 'left', 'right'];
  const displayChannels: readonly DisplayChannel[] = [...channels, 'stereo'];
  const channelLabel = { combined: 'Combined', left: 'Left', right: 'Right', stereo: 'Linked stereo' };
  const selectionTools = [
    { tool: 'rectangle', label: 'Rectangle', shortcut: '1', icon: RectangleHorizontal, hint: 'Drag a time and frequency area' },
    { tool: 'time', label: 'Time', shortcut: '2', icon: MoveHorizontal, hint: 'Drag across time to select the full frequency band' },
    { tool: 'frequency', label: 'Frequency', shortcut: '3', icon: MoveVertical, hint: 'Drag up or down to select a frequency band across a time range' },
    { tool: 'brush', label: 'Brush', shortcut: '4', icon: Paintbrush, hint: 'Paint a frequency path' },
    { tool: 'lasso', label: 'Lasso', shortcut: '5', icon: Lasso, hint: 'Draw a closed outline' },
  ] as const;
  const timeLabels = { startFrame: 'Start time', endFrame: 'End time', duration: 'Duration' };
  const scopeLabels = { view: 'Visible view', loop: 'Loop', recording: 'Whole recording' };
  const paletteLabel = { warm: 'Warm', neutral: 'Neutral', blue: 'Blue and gold' };
  const detailLabel = { balanced: 'Balanced', transients: 'Transients', tones: 'Tones' };
  const contrastLabel = { balanced: 'Balanced', detail: 'Reveal faint detail', strong: 'Emphasise strong signals' } as const;
  const viewKey = 'tagnetic.audio-repair-view.v1';
  const defaultView: ViewOptions = { treated: false, difference: false, channel: 'combined', waveform: false, waveformOnly: false, opacity: 52,
    contrast: 'balanced', palette: 'warm', offset: 0, detail: 'balanced' };
  const storedView = readView();
  let view = $state.raw<ViewOptions>(storedView);
  let width = $state(0);
  let height = $state(0);
  let overviewWidth = $state(0);
  let overviewHeight = $state(0);
  let shown = $state.raw<RepairViewport>();
  let manual = $state.raw<RepairViewport | null>(null);
  let selection = $state.raw<Pick>();
  let draft = $state.raw<Pick>();
  let amount = $state<number | null>(defaultAmount);
  let moving = $state<Gesture['mode']>();
  let cursor = $state('crosshair');
  let precise = $state(false);
  let inputError = $state<{ edge: TimeEdge; message: string }>();
  let selectionNotice = $state('');
  let tool = $state<RepairSelectionTool>('rectangle');
  let frequencyScope = $state<FrequencyScope>('view');
  let operation = $state<Gesture['operation']>('replace');
  let brushAdvanced = $state(false);
  let brushMs = $state(34);
  let brushHz = $state(220);
  let showColourScale = $state(false);
  let hover = $state.raw<{ x: number; y: number; frame: number; hz: number; lane: RepairChannel }>();
  let history = $state.raw<RepairViewport[]>([]);
  let lastCameraGesture = 0;
  let lastAmount = $state(defaultAmount);
  let restoredRepair = $state.raw<{ repair: SpectralRepair; index: number }>();
  let sourceProbe = $state.raw<TreatmentProbe | null>(null);
  let compareName = $derived(view.difference ? 'Removed' : view.treated ? 'Treated' : 'Source');
  let legend = $derived(repairDbScale(view.contrast, view.offset));
  let plot: HTMLElement | undefined;
  let interaction: Gesture | undefined;
  let viewFrame = 0;
  let gestureFrame = 0;
  let hoverFrame = 0;
  let pendingHover: typeof hover;
  let panTime = 0;
  let lastPanProbe = 0;
  let viewportTimer: ReturnType<typeof setTimeout> | undefined;
  let requested: (ProbeView & { time: FrameRange | null; overview: boolean }) | undefined;
  let viewedSource: string | undefined;
  let previousAutomatic: RepairViewport | undefined;
  let playbackClock: RepairClock | undefined;
  let probeHistory: TreatmentProbe[] = [];
  let textureCache = new WeakMap<TreatmentProbe, Partial<Record<RepairChannel, RepairTexture>>>();
  let textureStyle = '';

  const rate = $derived(snapshot?.sampleRate || 44100);
  const frames = $derived(snapshot?.frames || probe?.spectrogramEnd || Math.round(rate * 3.5));
  const detailLimit = $derived(spectralWindowLimit(rate, snapshot?.stereo === true));
  const fullFrequency = $derived(recordingFrequencyRange(rate));
  const measuredTime = $derived(spectrogramRange(probe));
  const automatic = $derived.by<RepairViewport>(() => {
    const frame = snapshot?.frame ?? 0;
    const fallback = boundedTime(frame - rate * 1.75, rate * 3.5, rate, frames);
    // An explicit query can still be arriving when Follow is re-enabled.
    const matchesFollow = measuredTime && Math.abs(measuredTime.start - fallback.start) < rate * .6
      && Math.abs((measuredTime.end - measuredTime.start) - (fallback.end - fallback.start)) < rate * .35;
    return { time: matchesFollow ? measuredTime : fallback, frequency: fullFrequency };
  });
  const visible = $derived(shown ?? manual ?? automatic);
  const overviewMode = $derived(visible.time.end - visible.time.start > detailLimit);
  const waveformView = $derived(overviewMode || view.waveformOnly);
  const currentTool = $derived(waveformView ? 'time' : tool);
  const toolDescription = $derived(selectionTools.find(item => item.tool === currentTool));
  const selected = $derived(moving && moving !== 'pan' ? draft ?? selection : selection);
  const appearance = $derived(snapshot?.theme);
  const settings = $derived(snapshot?.settings);
  const repairs = $derived(settings?.spectralRepairs ?? noRepairs);
  const attached = $derived(repairs.flatMap((repair, index) => repair ? [{ repair, index }] : []));
  const active = $derived.by(() => {
    if (selection?.index === undefined || !selection.saved) return undefined;
    const edit = repairs[selection.index];
    return edit && edit.channel === selection.saved.channel && sameRepairBounds(edit, selection.saved) ? edit : undefined;
  });
  const freeSlot = $derived(repairs.findIndex(value => !value));
  const nextSlot = $derived(freeSlot >= 0 ? freeSlot : repairs.length);
  const validPick = $derived(Boolean(selection && selection.endFrame > selection.startFrame && selection.highHz - selection.lowHz >= 20));
  const viewportReady = $derived(!!snapshot);
  const sourceId = $derived(snapshot?.sourceId);
  const stereo = $derived(snapshot?.stereo === true);
  const stereoDisplay = $derived(stereo && (!view.treated || settings?.mono === 'off'));
  const displayChannel = $derived(stereoDisplay ? view.channel : 'combined');
  const laneHeight = $derived(displayChannel === 'stereo' ? height / 2 : height);
  const bounds = $derived(plotBounds(width, laneHeight));
  const channel = $derived(displayChannel === 'stereo' ? 'combined' : displayChannel);
  const probeView = $derived<ProbeView>({ treated: view.treated, ...(view.difference ? { difference: true } : {}), channel: displayChannel,
    ...(view.detail === 'balanced' ? {} : { detail: view.detail }) });
  const waveform = $derived(snapshot?.overview);
  const stereoWaveform = $derived(snapshot?.stereoOverview);
  const cuts = $derived(snapshot?.cuts ?? []);
  const loopRange = $derived(snapshot?.loopRange);
  const scope = $derived(frequencyScope === 'loop' && !loopRange ? 'view' : frequencyScope);
  const drawnRepairs = $derived(attached.filter(({ repair }) => channel === 'combined' || !repair.channel || repair.channel === channel));
  const updatingView = $derived(viewportReady && !waveformView && (!probe || !matchesView(probe, probeView)
    || (manual && (!measuredTime || measuredTime.start > visible.time.start + 1 || measuredTime.end < visible.time.end - 1))));
  const hoverLevel = $derived.by(() => {
    if (!hover || !probe || !matchesView(probe, probeView)) return;
    const spectrogram = displayChannel !== 'stereo' ? probe.spectrogram
      : hover.lane === 'left' ? probe.spectrogramLeft : probe.spectrogramRight;
    return repairLevelAt({ ...probe, spectrogram }, hover.frame, hover.hz);
  });
  const displayLanes = $derived<RepairChannel[]>(displayChannel === 'stereo' ? ['left', 'right'] : [channel]);
  const guides = $derived.by(() => {
    const hum = settings?.hum === 'hz50' ? 50 : settings?.hum === 'hz60' ? 60 : 0;
    const result = hum ? Array.from({ length: settings?.humHarmonics || 1 }, (_, index) => hum * (index + 1)) : [];
    if (settings?.notchHz) result.push(settings.notchHz);
    return result;
  });

  function readView(): ViewOptions {
    try {
      const saved = JSON.parse(localStorage.getItem(viewKey) ?? '{}') as Partial<ViewOptions> | null;
      return {
        treated: saved?.treated === true,
        difference: saved?.treated === true && saved?.difference === true,
        waveform: saved?.waveform === true,
        waveformOnly: saved?.waveformOnly === true,
        opacity: typeof saved?.opacity === 'number' ? clampRepair(saved.opacity, 0, 100) : defaultView.opacity,
        channel: saved?.channel === 'left' || saved?.channel === 'right' || saved?.channel === 'stereo' ? saved.channel : 'combined',
        contrast: saved?.contrast === 'detail' || saved?.contrast === 'strong' ? saved.contrast : 'balanced',
        palette: saved?.palette === 'neutral' || saved?.palette === 'blue' ? saved.palette : 'warm',
        offset: typeof saved?.offset === 'number' ? clampRepair(saved.offset, -50, 20) : 0,
        detail: saved?.detail === 'transients' || saved?.detail === 'tones' ? saved.detail : 'balanced',
      };
    } catch {
      return defaultView;
    }
  }

  function changeView(patch: Partial<ViewOptions>) {
    view = { ...view, ...patch };
    try { localStorage.setItem(viewKey, JSON.stringify(view)); } catch { /* Keep this session's choice. */ }
  }

  function matchesView(result: TreatmentProbe, requestedView: ProbeView) {
    return (result.spectrogramTreated ?? false) === requestedView.treated
      && (result.spectrogramDifference ?? false) === (requestedView.difference ?? false)
      && (result.spectrogramChannel ?? 'combined') === requestedView.channel
      && (result.spectrogramDetail ?? 'balanced') === (requestedView.detail ?? 'balanced');
  }

  function recordingWaveform(mode: RepairChannel): RepairWaveform | undefined {
    const values = mode !== 'combined' && stereoWaveform?.length ? stereoWaveform : waveform;
    if (!values?.length) return;
    return { values, time: { start: 0, end: frames }, stereo: values === stereoWaveform, channel: mode };
  }

  function playheadAttachment(overview: boolean) {
    return (marker: HTMLElement) => {
      if (!snapshot) return;
      const frame = snapshot.frame;
      const running = snapshot.clockRunning ?? snapshot.playing;
      const loop = snapshot.loop ? loopRange : undefined;
      if (!playbackClock || playbackClock.frame !== frame || playbackClock.running !== running
        || playbackClock.loop?.start !== loop?.start || playbackClock.loop?.end !== loop?.end) {
        playbackClock = { frame, running, rate, frames, loop, observed: performance.now() };
      }
      const clock = playbackClock;
      const time = overview ? undefined : visible.time;
      const geometry = overview ? undefined : bounds;
      const miniWidth = overview ? overviewWidth : 0;
      let animation = 0;
      const paint = (now: number) => {
        const position = projectRepairFrame(clock, now);
        marker.hidden = !!time && (position < time.start || position > time.end);
        if (!marker.hidden) {
          const x = time && geometry ? frameX(position, geometry, time) : position * miniWidth / clock.frames;
          marker.style.transform = 'translateX(' + x + 'px)';
        }
        if (clock.running && now - clock.observed < repairClockHorizon) animation = requestAnimationFrame(paint);
      };
      paint(performance.now());
      return () => cancelAnimationFrame(animation);
    };
  }

  function paintOverview(canvas: HTMLCanvasElement) {
    void appearance;
    const overview = recordingWaveform('combined');
    if (!overview) return;
    const ink = getComputedStyle(canvas).getPropertyValue('--muted').trim() || '#aeb7d0';
    drawRepairOverview(canvas, overviewWidth, overviewHeight, Math.min(3, devicePixelRatio.current || 1), overview, ink);
  }

  function modeOf(image: TreatmentProbe): ProbeView {
    return { treated: image.spectrogramTreated ?? false, difference: image.spectrogramDifference,
      channel: image.spectrogramChannel ?? 'combined', detail: image.spectrogramDetail };
  }

  function attachPaint(lane: RepairChannel) {
    return (canvas: HTMLCanvasElement) => paint(canvas, lane);
  }

  function paint(canvas: HTMLCanvasElement, lane: RepairChannel) {
    void appearance;
    const scale = Math.min(3, devicePixelRatio.current || 1);
    const style = getComputedStyle(canvas);
    const accent = style.getPropertyValue('--accent').trim() || '#5c88e1';
    const palette = {
      ink: '#e2ddeb',
      grid: style.getPropertyValue('--border').trim() || '#3d435b',
      guide: style.getPropertyValue('--accent-ink').trim() || accent,
      font: style.getPropertyValue('--font-literal').trim() || 'monospace',
    };
    if (overviewMode) {
      const line = recordingWaveform(lane);
      drawRepairViewport(canvas, width, laneHeight, visible, [], rate, palette, [], scale,
        line && { ...line, opacity: .8 }, true);
      return;
    }
    const styleKey = view.palette + ':' + view.contrast + ':' + view.offset + ':' + rate;
    if (styleKey !== textureStyle) {
      textureStyle = styleKey;
      textureCache = new WeakMap();
    }
    const image = probe;
    if (image && measuredTime && image !== probeHistory.at(-1)) {
      const sameTile = (tile: TreatmentProbe) => tile.spectrogramStart === image.spectrogramStart
        && tile.spectrogramEnd === image.spectrogramEnd && matchesView(tile, modeOf(image));
      probeHistory = [...probeHistory.filter(tile => !sameTile(tile)).slice(-5), image];
    }
    const wanted = probeView;
    const lastImage = probeHistory.at(-1);
    const currentImage = probeHistory.filter(tile => matchesView(tile, wanted)).at(-1);
    const paintedView = currentImage ? wanted : lastImage && modeOf(lastImage);
    const textures = !paintedView || view.waveformOnly ? [] : probeHistory.flatMap(tile => {
      if (!matchesView(tile, paintedView)) return [];
      const stereoTile = tile.spectrogramChannel === 'stereo' && lane !== 'combined';
      const laneColumns = lane === 'left' ? tile.spectrogramLeft : tile.spectrogramRight;
      if (stereoTile && !laneColumns?.length) return [];
      const cache = textureCache.get(tile) ?? {};
      let texture = cache[lane];
      if (!texture) {
        const selectedTile = stereoTile ? { ...tile, spectrogram: laneColumns } : tile;
        texture = buildRepairTexture(selectedTile, rate, view.palette, view.contrast, view.offset) ?? undefined;
        if (texture) {
          cache[lane] = texture;
          textureCache.set(tile, cache);
        }
      }
      return texture ? [texture] : [];
    });
    const waveformImage = currentImage ?? lastImage;
    const waveformTime = spectrogramRange(waveformImage);
    const fallbackWaveform = view.treated ? undefined : recordingWaveform(lane);
    const laneWaveform = displayChannel !== 'stereo' ? waveformImage?.spectrogramWaveform
      : lane === 'left' ? waveformImage?.spectrogramLeftWaveform : waveformImage?.spectrogramRightWaveform;
    const opacity = view.waveformOnly ? .83 : view.opacity / 100;
    const line = !view.waveform && !view.waveformOnly ? undefined
      : laneWaveform?.length && waveformTime
        ? { values: laneWaveform, time: waveformTime, opacity, clipping: view.waveformOnly }
        : fallbackWaveform && { ...fallbackWaveform, opacity, clipping: view.waveformOnly };
    drawRepairViewport(canvas, width, laneHeight, visible, textures, rate, palette, guides, scale, line, view.waveformOnly);
  }

  $effect(() => {
    if (probe && !probe.spectrogramTreated && !probe.spectrogramDifference) sourceProbe = probe;
  });

  function cancelView() {
    if (viewFrame) cancelAnimationFrame(viewFrame);
    viewFrame = 0;
  }

  function animateViewport(target: RepairViewport, duration: number) {
    const from = visible;
    cancelView();
    const distance = Math.abs(target.time.start - from.time.start);
    const span = target.time.end - target.time.start;
    const ratio = span / (from.time.end - from.time.start);
    const timeChanged = from.time.start !== target.time.start || from.time.end !== target.time.end;
    const frequencyChanged = from.frequency.lowHz !== target.frequency.lowHz || from.frequency.highHz !== target.frequency.highHz;
    const distantPan = ratio > .67 && ratio < 1.5 && distance > span * 1.3;
    if ((!timeChanged && !frequencyChanged) || !duration || distantPan || matchMedia('(prefers-reduced-motion: reduce)').matches) {
      shown = target;
      return;
    }
    const started = performance.now();
    const step = (now: number) => {
      const progress = clampRepair((now - started) / duration, 0, 1);
      const ease = 1 - (1 - progress) ** 3;
      const mix = (a: number, b: number) => a + (b - a) * ease;
      const logMix = (a: number, b: number) => Math.exp(mix(Math.log(a), Math.log(b)));
      shown = progress === 1 ? target : {
        time: interpolateRepairTime(from.time, target.time, ease),
        frequency: { lowHz: logMix(from.frequency.lowHz, target.frequency.lowHz), highHz: logMix(from.frequency.highHz, target.frequency.highHz) },
      };
      viewFrame = progress < 1 ? requestAnimationFrame(step) : 0;
    };
    viewFrame = requestAnimationFrame(step);
  }

  function followMeasuredViewport(target: RepairViewport) {
    if (target.time.start === previousAutomatic?.time.start && target.time.end === previousAutomatic.time.end
      && target.frequency.lowHz === previousAutomatic.frequency.lowHz && target.frequency.highHz === previousAutomatic.frequency.highHz) return;
    previousAutomatic = target;
    animateViewport(target, 125);
  }

  function followBinding() {
    if (manual || !snapshot) return;
    const target = automatic;
    const frame = requestAnimationFrame(() => followMeasuredViewport(target));
    return () => cancelAnimationFrame(frame);
  }

  function requestViewport(time: FrameRange | null, mode: ProbeView = probeView) {
    clearTimeout(viewportTimer);
    viewportTimer = undefined;
    let range = time ? { start: Math.round(time.start), end: Math.round(time.end) } : null;
    const overview = !!range && range.end - range.start > detailLimit;
    if (overview) {
      if (requested?.overview) return;
      range = null;
    }
    if (requested && requested.overview === overview && requested.time?.start === range?.start && requested.time?.end === range?.end
      && requested.treated === mode.treated && requested.channel === mode.channel
      && requested.difference === mode.difference && requested.detail === mode.detail) return;
    requested = { time: range, overview, ...mode };
    onviewport(range, overview ? { ...mode, overview: true } : mode);
  }

  function reflectView() {
    const id = sourceId;
    const mode = probeView;
    if (!id) return;
    untrack(() => {
      if (viewedSource !== id) {
        viewedSource = id;
        const range = snapshot?.loopRange;
        if (range && range.end > range.start) {
          const duration = range.end - range.start;
          const size = Math.min(detailLimit, Math.max(rate * 3.5, duration * 1.15));
          const frame = snapshot?.frame ?? range.start;
          const center = duration <= detailLimit ? (range.start + range.end) / 2
            : frame >= range.start && frame < range.end ? frame : range.start + size / 2;
          setManual({ time: boundedTime(center - size / 2, size, rate, frames), frequency: fullFrequency });
        }
      }
      requestViewport(manual?.time ?? null, mode);
    });
  }

  function scheduleViewport() {
    clearTimeout(viewportTimer);
    viewportTimer = setTimeout(() => requestViewport(manual?.time ?? null), 95);
  }

  function setManual(target: RepairViewport, duration = 0) {
    animateViewport(target, duration);
    manual = target;
  }

  function resume() {
    animateViewport(automatic, 140);
    manual = null;
    requestViewport(null);
  }

  function toggleFollow() {
    if (manual) resume();
    else {
      const viewport = visible;
      setManual(viewport);
      requestViewport(viewport.time);
    }
  }

  function fitRecording() {
    if (visible.time.start !== 0 || visible.time.end !== frames
      || visible.frequency.lowHz !== fullFrequency.lowHz || visible.frequency.highHz !== fullFrequency.highHz) rememberCamera();
    const target = { time: { start: 0, end: frames }, frequency: fullFrequency };
    setManual(target, 160);
    requestViewport(target.time);
  }

  function showSpectralDetail() {
    const time = visible.time;
    const frame = snapshot?.frame;
    const middle = frame !== undefined && frame >= time.start && frame < time.end ? frame : (time.start + time.end) / 2;
    const size = Math.min(detailLimit, rate * 3.5);
    const detail = boundedTime(middle - size / 2, size, rate, frames);
    rememberCamera();
    setManual({ ...visible, time: detail });
    requestViewport(detail);
  }

  async function toggleBounds() {
    precise = !precise;
    if (precise) {
      await tick();
      document.getElementById(plotHelpID + '-bounds')?.querySelector('input')?.focus();
    }
  }

  function clear() {
    selection = undefined;
    if (operation === 'subtract') operation = 'replace';
    precise = false;
    inputError = undefined;
    cursor = 'crosshair';
  }

  function rememberCamera() {
    history = [...history.slice(-15), manual ?? visible];
  }

  function previousCamera() {
    const camera = history.at(-1);
    if (!camera) return;
    history = history.slice(0, -1);
    setManual(camera, 140);
    requestViewport(camera.time);
  }

  function fitSelection(axis: 'both' | 'time' | 'frequency' = 'both') {
    if (!selection) return;
    rememberCamera();
    const { startFrame, endFrame, lowHz, highHz } = selection;
    const span = (endFrame - startFrame) * 1.2;
    const middle = (startFrame + endFrame) / 2;
    const size = clampRepair(span, Math.min(frames, rate / 4), frames);
    const time = axis === 'frequency' ? visible.time : boundedTime(middle - size / 2, size, rate, frames);
    const low = Math.log(clampRepair(lowHz, fullFrequency.lowHz, fullFrequency.highHz - 1));
    const high = Math.log(clampRepair(highHz, Math.exp(low) + 1, fullFrequency.highHz));
    const padding = Math.max(.1, (high - low) * .15);
    const frequency = axis === 'time' ? visible.frequency : {
      lowHz: Math.max(fullFrequency.lowHz, Math.exp(low - padding)),
      highHz: Math.min(fullFrequency.highHz, Math.exp(high + padding)),
    };
    setManual({ time, frequency }, 160);
    requestViewport(time);
  }

  function audition() {
    if (selection) onaction('loop', { start: selection.startFrame, end: selection.endFrame });
  }

  function auditionBand() {
    if (!selection || !validPick) return;
    if (snapshot?.monitor === 'band') {
      onaction('stop-audition');
      return;
    }
    const { startFrame, endFrame, lowHz, highHz, channel } = selection;
    if (lowHz < 20 || highHz > fullFrequency.highHz) return;
    onaction('audition-band', { start: startFrame, end: endFrame }, { lowHz, highHz, channel });
  }

  function jumpTo(value: number, recenter = true, detail = false) {
    const frame = Math.round(clampRepair(value, 0, frames - 1));
    const span = detail ? Math.min(detailLimit, rate * 3.5) : visible.time.end - visible.time.start;
    if (manual && recenter) {
      if (detail) rememberCamera();
      const time = boundedTime(frame - span / 2, span, rate, frames);
      setManual({ ...visible, time }, 125);
      requestViewport(time);
    }
    onaction('seek', { start: frame, end: frame + 1 });
  }

  function overviewClick(event: MouseEvent) {
    if (event.detail === 0) {
      jumpTo(snapshot?.frame ?? 0);
      return;
    }
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    jumpTo((event.clientX - rect.left) * frames / Math.max(1, rect.width), true, overviewMode);
  }

  function overviewKeys(event: KeyboardEvent) {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const current = snapshot?.frame ?? 0;
    if (event.key === 'PageUp' || event.key === 'PageDown') {
      event.preventDefault();
      const direction = event.key === 'PageUp' ? -1 : 1;
      const landmarks = [
        ...cuts.map(frame => ({ frame, slot: undefined })),
        ...attached.map(({ repair, index }) => ({ frame: repair.startFrame, slot: index })),
      ];
      const target = landmarks
        .filter(item => (item.frame - current) * direction > 0)
        .sort((left, right) => (left.frame - right.frame) * direction)[0];
      if (target?.slot !== undefined) {
        const repair = repairs[target.slot];
        if (repair) overviewLandmark(repair, target.slot);
      } else if (target) jumpTo(target.frame, true, overviewMode);
      return;
    }
    const step = event.shiftKey ? 10 * rate : rate;
    const jump: Record<string, number> = { ArrowLeft: current - step, ArrowRight: current + step, Home: 0, End: frames - 1 };
    if (!(event.key in jump)) return;
    event.preventDefault();
    jumpTo(jump[event.key]);
  }

  function zoomAt(event: WheelEvent) {
    if (!viewportReady || interaction || (!event.deltaX && !event.deltaY)) return;
    event.preventDefault();
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const speed = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? height : 1;
    const factor = Math.exp(clampRepair(event.deltaY * speed * .0025, -.7, .7));
    if (currentTool === 'brush' && event.altKey) {
      brushMs = clampRepair(Math.round(brushMs * factor), 2, 500);
      brushHz = clampRepair(Math.round(brushHz * factor), 20, 20_000);
      return;
    }
    const now = performance.now();
    if (now - lastCameraGesture > 320 || !lastCameraGesture) rememberCamera();
    lastCameraGesture = now;
    const current = manual ?? visible;
    if (!event.altKey && !event.shiftKey && Math.abs(event.deltaX) > Math.abs(event.deltaY)) {
      const span = current.time.end - current.time.start;
      const distance = event.deltaX * speed * span / Math.max(1, bounds.right - bounds.left);
      const time = boundedTime(current.time.start + distance, span, rate, frames);
      if (time.start !== current.time.start) {
        setManual({ ...current, time }, 110);
        scheduleViewport();
      }
      return;
    }
    const x = event.clientX - rect.left;
    const y = localY(event.clientY - rect.top);
    if (event.shiftKey || x < bounds.left) {
      if (waveformView) return;
      const half = Math.log(fullFrequency.highHz / fullFrequency.lowHz);
      const logMin = Math.log(fullFrequency.lowHz);
      const ratio = clampRepair((bounds.bottom - y) / (bounds.bottom - bounds.top), 0, 1);
      const span = Math.log(current.frequency.highHz / current.frequency.lowHz);
      const anchor = Math.log(current.frequency.lowHz) + ratio * span;
      const size = clampRepair(span * factor, .10, half);
      const low = clampRepair(anchor - size * ratio, logMin, logMin + half - size);
      setManual({ ...current, frequency: { lowHz: Math.exp(low), highHz: Math.exp(low + size) } }, 150);
    } else {
      const ratio = clampRepair((event.clientX - rect.left - bounds.left) / (bounds.right - bounds.left), 0, 1);
      const span = current.time.end - current.time.start;
      const size = clampRepair(span * factor, Math.min(frames, rate / 4), frames);
      const anchor = current.time.start + ratio * span;
      setManual({ ...current, time: boundedTime(anchor - size * ratio, size, rate, frames) }, 160);
    }
    scheduleViewport();
  }

  function wheelBinding(node: HTMLElement) {
    node.addEventListener('wheel', zoomAt, { passive: false });
    return () => node.removeEventListener('wheel', zoomAt);
  }

  function laneAt(y: number): RepairChannel {
    return displayChannel !== 'stereo' ? channel : y < laneHeight ? 'left' : 'right';
  }

  function localY(y: number) {
    return displayChannel === 'stereo' && y >= laneHeight ? y - laneHeight : y;
  }

  function laneOffset(lane: RepairChannel) {
    return displayChannel === 'stereo' && lane === 'right' ? laneHeight : 0;
  }

  function rectangle(pick: Pick, viewport = visible) {
    const { time, frequency } = viewport;
    return {
      left: frameX(pick.startFrame, bounds, time),
      right: frameX(pick.endFrame, bounds, time),
      top: waveformView ? bounds.top : frequencyY(pick.highHz, bounds, frequency),
      bottom: waveformView ? bounds.bottom : frequencyY(pick.lowHz, bounds, frequency),
    };
  }

  function hit(x: number, y: number, lane: RepairChannel) {
    if (selection && (lane === 'combined' || !selection.channel || selection.channel === lane)) {
      const handles = repairDragAt(rectangle(selection), bounds, x, y);
      if (handles) return { pick: selection, handles, amount: amount ?? defaultAmount };
    }
    for (let position = drawnRepairs.length - 1; position >= 0; position--) {
      const { repair, index } = drawnRepairs[position];
      if (lane !== 'combined' && repair.channel && repair.channel !== lane) continue;
      const handles = repairDragAt(rectangle(repair), bounds, x, y);
      if (handles) return { pick: { ...repair, index, saved: repair }, handles, amount: repair.amountDb };
    }
  }

  function gestureOperation(event: PointerEvent) {
    return event.altKey ? 'subtract' : event.shiftKey ? 'add' : operation;
  }

  function activeHandles(handles: RepairDrag | undefined, action: Gesture['operation']): RepairDrag | undefined {
    if (!handles || action === 'subtract') return;
    if (currentTool === 'time') return handles.x ? { x: handles.x, y: 0 } : undefined;
    if (currentTool === 'frequency') return handles.y ? { x: 0, y: handles.y } : undefined;
    if (handles.x || handles.y || (action === 'replace' && currentTool === 'rectangle')) return handles;
  }

  function begin(event: PointerEvent) {
    if (!viewportReady || interaction || (event.button !== 0 && event.button !== 1)) return;
    const node = event.currentTarget as HTMLButtonElement;
    const rect = node.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const local = event.clientY - rect.top;
    const lane = laneAt(local);
    const y = localY(local);
    if (x < bounds.left || x > bounds.right) return;
    if (event.button === 0 && (y > bounds.bottom || event.ctrlKey || event.metaKey)) {
      jumpTo(frameAtX(x, bounds, visible.time), false);
      event.preventDefault();
      return;
    }
    if (y < bounds.top || y > bounds.bottom) return;
    const origin = visible;
    const action = gestureOperation(event);
    const target = event.button === 0 ? hit(x, y, lane) : undefined;
    const handles = activeHandles(target?.handles, action);
    const baseline = target && handles ? { ...target, handles } : undefined;
    selectionNotice = '';
    if (event.button === 0 && !baseline && action === 'subtract' && !selection) {
      selectionNotice = 'Draw a selection before subtracting from it.';
      return;
    }
    if (event.button === 0 && !baseline && action !== 'replace' && selection
      && lane !== 'combined' && selection.channel !== lane) {
      selectionNotice = 'Add and subtract use the selected repair channel. Draw a new selection to edit this lane.';
      return;
    }
    const firstFrame = frameAtX(x, bounds, origin.time);
    const firstHz = frequencyAtY(y, bounds, origin.frequency);
    if (event.button === 1) rememberCamera();
    interaction = {
      pointer: event.pointerId, node, rect, firstX: x, firstY: y,
      firstFrame, firstHz, x, y, lane,
      mode: event.button === 1 ? 'pan' : baseline ? 'edit' : 'pick', moved: false, origin, previous: manual, baseline,
      tool: currentTool, operation: action, points: [{ frame: Math.round(firstFrame), hz: Math.round(firstHz) }],
      frequencyTime: frequencySelectionTime(origin.time),
      brush: { radiusFrames: Math.max(1, Math.round(rate * brushMs / 2000)), radiusHz: Math.max(1, Math.round(brushHz / 2)) },
    };
    if (event.button === 1) event.preventDefault();
    setManual(origin);
    moving = interaction.mode;
    inputError = undefined;
    panTime = 0;
    node.setPointerCapture(event.pointerId);
  }

  function updatePoint(gesture: Gesture, event: PointerEvent) {
    gesture.x = event.clientX - gesture.rect.left;
    gesture.y = event.clientY - gesture.rect.top - laneOffset(gesture.lane);
    const x = gesture.mode === 'pick' && gesture.tool === 'frequency' ? 0 : gesture.x - gesture.firstX;
    const y = gesture.mode === 'pick' && gesture.tool === 'time' ? 0 : gesture.y - gesture.firstY;
    gesture.moved ||= Math.hypot(x, y) >= 3;
  }

  function frequencySelectionTime(time: FrameRange) {
    const window = scope === 'recording' ? { start: 0, end: frames }
      : scope === 'loop' && loopRange ? loopRange : time;
    const start = clampRepair(Math.round(window.start), 0, frames - 1);
    return { start, end: clampRepair(Math.round(window.end), start + 1, frames) };
  }

  function paintGesture(gesture: Gesture) {
    if (!gesture.moved && (gesture.mode !== 'pick' || gesture.tool !== 'brush')) return;
    const { origin, firstX, firstY, x, y } = gesture;
    if (gesture.mode === 'pan') {
      const span = origin.time.end - origin.time.start;
      const time = boundedTime(origin.time.start - (x - firstX) * span / (bounds.right - bounds.left), span, rate, frames);
      const widthHz = Math.log(origin.frequency.highHz / origin.frequency.lowHz);
      const offset = (y - firstY) * widthHz / (bounds.bottom - bounds.top);
      setManual({ time, frequency: slideFrequency(origin.frequency, offset) });
      scheduleViewport();
      return;
    }
    const frame = frameAtX(x, bounds, visible.time);
    const hz = frequencyAtY(y, bounds, visible.frequency);
    const initial = gesture.baseline;
    if (initial) {
      const max = initial.pick.mask ? Math.max(fullFrequency.highHz, Math.ceil(rate / 2)) : fullFrequency.highHz;
      const handles = waveformView ? { ...initial.handles, y: 0 as const } : initial.handles;
      const geometry = editRepairBounds(initial.pick, handles, frame - gesture.firstFrame, waveformView ? 0 : Math.log(hz / gesture.firstHz), frames, max,
        initial.pick.mask ? 0 : 20);
      draft = reshape(initial.pick, geometry);
      return;
    }
    const first = Math.round(gesture.firstFrame);
    const end = Math.round(frame);
    const band = { lowHz: Math.round(Math.min(gesture.firstHz, hz)), highHz: Math.round(Math.max(gesture.firstHz, hz)) };
    const span = { startFrame: Math.min(first, end), endFrame: Math.max(first, end) };
    const target = gesture.lane === 'combined' ? undefined : gesture.lane;
    if (gesture.tool === 'rectangle' && gesture.operation === 'replace') {
      draft = { ...span, ...band, channel: target };
      return;
    }
    const operation = gesture.operation === 'subtract' ? 'subtract' : 'add';
    let region: RepairMaskRegion;
    if (gesture.tool === 'brush' || gesture.tool === 'lasso') {
      appendGesturePoint(gesture, { frame: end, hz: Math.round(hz) });
      if (gesture.tool === 'lasso' && gesture.points.length < 3) return;
      region = gesture.tool === 'brush'
        ? { kind: 'brush', operation, points: [...gesture.points], ...gesture.brush }
        : { kind: 'lasso', operation, points: [...gesture.points] };
    } else {
      if (gesture.tool === 'frequency') {
        span.startFrame = gesture.frequencyTime.start;
        span.endFrame = gesture.frequencyTime.end;
      }
      if (gesture.tool === 'time') {
        band.lowHz = 0;
        band.highHz = Math.ceil(rate / 2);
      }
      if (span.endFrame <= span.startFrame || band.highHz - band.lowHz < 20) return;
      region = { kind: 'rectangle', operation, ...span, ...band };
    }
    const base = gesture.operation === 'replace' ? undefined : selection;
    draft = maskedPick([...regionsFor(base), region], base, target);
  }

  function regionsFor(pick?: Pick): RepairMaskRegion[] {
    if (!pick) return [];
    if (pick.mask) return pick.mask.regions;
    const { startFrame, endFrame, lowHz, highHz } = pick;
    return [{ kind: 'rectangle', operation: 'add', startFrame, endFrame, lowHz, highHz }];
  }

  function appendGesturePoint(gesture: Gesture, point: RepairMaskPoint) {
    const points = gesture.points;
    const last = points.at(-1);
    if (last?.frame === point.frame && last?.hz === point.hz) return;
    const available = Math.min(maxRepairMaskPoints, maxRepairMaskTotalPoints - pointCount(regionsFor(gesture.operation === 'replace' ? undefined : selection)));
    if (available <= 0) return;
    if (points.length >= available) {
      if (available < 3) return;
      gesture.points = points.filter((_, index) => index % 2 === 0);
    }
    gesture.points.push(point);
  }

  function pointCount(regions: RepairMaskRegion[]) {
    return regions.reduce((total, region) => total + (region.kind === 'rectangle' ? 0 : region.points.length), 0);
  }

  function maskedPick(regions: RepairMaskRegion[], base?: Pick, channel?: Pick['channel']): Pick | undefined {
    if (regions.length > maxRepairMaskRegions || pointCount(regions) > maxRepairMaskTotalPoints) return;
    const limits = repairMaskBounds(regions, frames, Math.ceil(rate / 2));
    if (!limits) return;
    const template = base?.interpolate ? undefined : base;
    return { ...template, ...limits, channel: base ? base.channel : channel,
      mask: { version: 1, featherFrames: base?.mask?.featherFrames ?? 0, featherHz: base?.mask?.featherHz ?? 0, regions } };
  }

  function selectSuggestion(bounds: RepairBounds[], target = selection?.channel) {
    if (!bounds.length) return;
    selection = bounds.length === 1 ? { ...bounds[0], channel: target }
      : maskedPick(bounds.map(bounds => ({ kind: 'rectangle', operation: 'add', ...bounds })), undefined, target);
    precise = false;
    inputError = undefined;
  }

  function reshape(pick: Pick, geometry: RepairBounds) {
    selectionNotice = '';
    if (!pick.mask) return { ...pick, ...geometry };
    const mask = remapRepairMask(pick.mask, pick, geometry, frames, Math.ceil(rate / 2), rate);
    if (!mask) {
      selectionNotice = 'This resize would exceed the brush size limit. Draw a longer stroke to cover more time.';
      return;
    }
    return { ...pick, ...geometry, mask };
  }

  function slideFrequency(range: FrequencyRange, logOffset: number) {
    const ratio = range.highHz / range.lowHz;
    const lowHz = clampRepair(range.lowHz * Math.exp(logOffset), fullFrequency.lowHz, fullFrequency.highHz / ratio);
    return { lowHz, highHz: Math.min(fullFrequency.highHz, lowHz * ratio) };
  }

  function edgePan(gesture: Gesture, now: number) {
    if (gesture.mode === 'pan') return false;
    const panX = gesture.mode === 'pick' && gesture.tool === 'frequency' ? 0 : edgePanSpeed(gesture.x, bounds.left, bounds.right);
    const panY = gesture.mode === 'pick' && gesture.tool === 'time' ? 0 : edgePanSpeed(gesture.y, bounds.top, bounds.bottom);
    const elapsed = panTime ? Math.min(40, now - panTime) : 16;
    panTime = now;
    if ((!panX && !panY) || elapsed <= 0) return false;
    const current = visible;
    const span = current.time.end - current.time.start;
    const band = Math.log(current.frequency.highHz / current.frequency.lowHz);
    const time = panX ? boundedTime(current.time.start + panX * elapsed * span / 1450, span, rate, frames) : current.time;
    const frequency = panY ? slideFrequency(current.frequency, -panY * elapsed * band / 1450) : current.frequency;
    if (time.start === current.time.start && Math.abs(Math.log(frequency.lowHz / current.frequency.lowHz)) < 1e-8) return false;
    setManual({ time, frequency });
    if (panX && now - lastPanProbe >= 180) {
      lastPanProbe = now;
      requestViewport(time);
    }
    return true;
  }

  function queueGesture(gesture: Gesture) {
    if (gestureFrame) return;
    gestureFrame = requestAnimationFrame(now => {
      gestureFrame = 0;
      if (interaction !== gesture) return;
      const panning = edgePan(gesture, now);
      paintGesture(gesture);
      if (panning) queueGesture(gesture);
    });
  }

  function move(event: PointerEvent) {
    const gesture = interaction;
    if (!gesture) {
      const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
      const x = event.clientX - rect.left;
      const local = event.clientY - rect.top;
      const y = localY(local);
      const lane = laneAt(local);
      cursor = repairDragCursor(activeHandles(hit(x, y, lane)?.handles, gestureOperation(event)));
      pendingHover = x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom
        ? { x, y: local, frame: frameAtX(x, bounds, visible.time), hz: frequencyAtY(y, bounds, visible.frequency), lane } : undefined;
      if (!hoverFrame) hoverFrame = requestAnimationFrame(() => {
        hover = pendingHover;
        hoverFrame = 0;
      });
      return;
    }
    if (gesture.pointer !== event.pointerId) return;
    updatePoint(gesture, event);
    if (gesture.moved) {
      if (gesture.mode === 'pick' && (gesture.tool === 'brush' || gesture.tool === 'lasso')) {
        for (const sample of event.getCoalescedEvents?.() ?? []) {
          const frame = frameAtX(sample.clientX - gesture.rect.left, bounds, visible.time);
          const y = sample.clientY - gesture.rect.top - laneOffset(gesture.lane);
          const hz = frequencyAtY(y, bounds, visible.frequency);
          appendGesturePoint(gesture, { frame: Math.round(frame), hz: Math.round(hz) });
        }
        appendGesturePoint(gesture, {
          frame: Math.round(frameAtX(gesture.x, bounds, visible.time)),
          hz: Math.round(frequencyAtY(gesture.y, bounds, visible.frequency)),
        });
      }
      queueGesture(gesture);
    }
  }

  function clearHover() {
    if (hoverFrame) cancelAnimationFrame(hoverFrame);
    pendingHover = undefined;
    hover = undefined;
    hoverFrame = 0;
  }

  function chooseExisting(pick: SpectralRepair, index: number, center = false) {
    selectionNotice = '';
    selection = { ...pick, index, saved: pick };
    amount = pick.amountDb;
    inputError = undefined;
    if (center) {
      const duration = pick.endFrame - pick.startFrame;
      const span = Math.min(detailLimit, Math.max(rate * 3.5, 1.2 * duration));
      const centerFrame = duration <= span ? (pick.startFrame + pick.endFrame) / 2 : pick.startFrame + span / 4;
      const time = boundedTime(centerFrame - span / 2, span, rate, frames);
      setManual({ time, frequency: fullFrequency }, 160);
      requestViewport(time);
      requestAnimationFrame(focusPlot);
    }
  }

  function finish(release?: PointerEvent, restoreViewport = true) {
    const gesture = interaction;
    if (!gesture) return;
    if (gestureFrame) cancelAnimationFrame(gestureFrame);
    gestureFrame = 0;
    if (release) {
      updatePoint(gesture, release);
      paintGesture(gesture);
      if (gesture.mode !== 'pan' && draft && draft.endFrame > draft.startFrame) {
        selection = draft;
        amount = gesture.baseline?.amount ?? (gesture.operation === 'replace' ? defaultAmount : amount);
      } else if (gesture.baseline && !gesture.moved) {
        selection = gesture.baseline.pick;
        amount = gesture.baseline.amount;
      } else if (!gesture.moved) {
        shown = gesture.origin;
        manual = gesture.previous;
        if (gesture.mode === 'pick' && gesture.operation === 'replace') clear();
      }
      requestViewport(manual?.time ?? null);
    } else {
      cancelView();
      shown = gesture.origin;
      manual = gesture.previous;
      if (restoreViewport) requestViewport(manual?.time ?? null);
    }
    interaction = undefined;
    moving = undefined;
    draft = undefined;
    cursor = 'crosshair';
    if (gesture.node.hasPointerCapture(gesture.pointer)) gesture.node.releasePointerCapture(gesture.pointer);
  }

  function end(event: PointerEvent) {
    if (event.pointerId === interaction?.pointer) finish(event.type === 'pointerup' ? event : undefined);
  }

  function markPlot(node: HTMLElement) {
    plot = node;
    return () => { plot = undefined; };
  }

  function focusPlot() {
    plot?.querySelector<HTMLButtonElement>('.repair-hit')?.focus({ preventScroll: true });
  }

  function chooseTool(value: RepairSelectionTool) {
    tool = value;
    if (value !== 'time') {
      if (view.waveformOnly) changeView({ waveformOnly: false });
      if (overviewMode) showSpectralDetail();
    }
    selectionNotice = '';
    clearHover();
  }

  function keys(event: KeyboardEvent) {
    if (!viewportReady || event.altKey || event.defaultPrevented) return;
    if (event.key === 'Escape' && (interaction || selection)) {
      event.preventDefault();
      if (interaction) finish();
      else clear();
      return;
    }
    if (interaction) return;
    if (event.ctrlKey || event.metaKey) {
      if (event.key === 'Enter' && selection) {
        event.preventDefault();
        apply();
      }
      return;
    }
    if (event.key === 'Delete' && active) {
      event.preventDefault();
      remove();
      return;
    }
    const mode = selectionTools.find(item => item.shortcut === event.key);
    if (mode && !event.shiftKey) {
      event.preventDefault();
      chooseTool(mode.tool);
      return;
    }
    if (event.key.toLowerCase() === 'f' && selection) {
      event.preventDefault();
      fitSelection();
      return;
    }
    if (event.key.toLowerCase() === 'r' && selection && !active) {
      event.preventDefault();
      amount = lastAmount;
      apply();
      return;
    }
    if (event.key === '0') {
      event.preventDefault();
      fitRecording();
      return;
    }
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Enter'].includes(event.key)) return;
    event.preventDefault();
    if (event.key === 'Enter') {
      if (event.shiftKey && selection) auditionBand();
      else if (selection) audition();
      else onaction('loop', { start: Math.round(visible.time.start), end: Math.round(visible.time.end) });
      return;
    }
    if (currentTool === 'time' && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) return;
    if (currentTool === 'frequency' && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) return;
    if (!manual) {
      setManual(visible);
      scheduleViewport();
    }
    const initial: Pick = {
      startFrame: Math.round(visible.time.start), endFrame: Math.round(visible.time.end),
      lowHz: Math.round(visible.frequency.lowHz), highHz: Math.round(visible.frequency.highHz),
      channel: channel === 'combined' ? undefined : channel,
    };
    let current = selection ?? initial;
    if (!selection && (currentTool === 'time' || currentTool === 'frequency')) {
      const time = frequencySelectionTime(visible.time);
      const geometry = currentTool === 'time' ? { ...initial, lowHz: 0, highHz: Math.ceil(rate / 2) }
        : { ...initial, startFrame: time.start, endFrame: time.end };
      current = maskedPick([{ kind: 'rectangle', operation: 'add', ...geometry }], undefined, initial.channel) ?? initial;
    }
    const next = { ...current };
    const nudge = Math.max(1, Math.round((visible.time.end - visible.time.start) / 36));
    const maxHz = current.mask ? Math.ceil(rate / 2) : fullFrequency.highHz;
    switch (event.key) {
      case 'ArrowLeft':
      case 'ArrowRight': {
        const offset = event.key === 'ArrowLeft' ? -nudge : nudge;
        if (event.shiftKey) next.endFrame = clampRepair(next.endFrame + offset, next.startFrame + 1, frames);
        else next.startFrame = clampRepair(next.startFrame + offset, 0, next.endFrame - 1);
        break;
      }
      case 'ArrowUp':
      case 'ArrowDown': {
        const ratio = Math.exp(Math.log(visible.frequency.highHz / visible.frequency.lowHz) / 115);
        const step = event.key === 'ArrowUp' ? ratio : 1 / ratio;
        if (event.shiftKey) next.highHz = clampRepair(Math.round(next.highHz * step), next.lowHz + 20, maxHz);
        else next.lowHz = clampRepair(Math.round(Math.max(1, next.lowHz) * step), current.mask ? 0 : 20, next.highHz - 20);
        break;
      }
    }
    selection = reshape(current, next) ?? selection;
  }

  function timeLimits(edge: TimeEdge, pick: Pick) {
    if (edge === 'duration') return [1, frames - pick.startFrame];
    return edge === 'startFrame' ? [0, pick.endFrame - 1] : [pick.startFrame + 1, frames];
  }

  function timeValue(edge: TimeEdge, pick: Pick) {
    return edge === 'duration' ? pick.endFrame - pick.startFrame : pick[edge];
  }

  function commitTime(input: HTMLInputElement, edge: TimeEdge) {
    if (!selection) return false;
    const value = frameAtTime(input.value, rate);
    const [min, max] = timeLimits(edge, selection);
    if (value === null || value < min || value > max) {
      const kind = edge === 'duration' ? 'duration' : 'time';
      inputError = { edge, message: 'Use a ' + kind + ' between ' + timeLabel(min, rate, 6) + ' and ' + timeLabel(max, rate, 6) + '.' };
      return false;
    }
    const updated = reshape(selection, edge === 'duration'
      ? { ...selection, endFrame: selection.startFrame + value }
      : { ...selection, [edge]: value });
    input.value = timeLabel(updated ? value : timeValue(edge, selection), rate, 6);
    if (!updated) return false;
    selection = updated;
    if (inputError?.edge === edge) inputError = undefined;
    return true;
  }

  function exactTimeKeys(event: KeyboardEvent, edge: TimeEdge) {
    if (!selection) return;
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === 'Escape') {
      event.preventDefault();
      input.value = timeLabel(timeValue(edge, selection), rate, 6);
      inputError = undefined;
      focusPlot();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      if (commitTime(input, edge) && (event.ctrlKey || event.metaKey)) apply();
    } else if ((event.key === 'ArrowUp' || event.key === 'ArrowDown') && !event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      const [min, max] = timeLimits(edge, selection);
      const nudge = event.shiftKey ? 1 : Math.max(1, Math.round(rate / 100));
      const base = frameAtTime(input.value, rate) ?? timeValue(edge, selection);
      const next = clampRepair(base + nudge * (event.key === 'ArrowUp' ? 1 : -1), min, max);
      input.value = timeLabel(next, rate, 6);
      commitTime(input, edge);
    }
  }

  function editFrequency(edge: FrequencyEdge, value: number | null) {
    if (selection && value !== null) selection = reshape(selection, { ...selection, [edge]: Math.round(value) }) ?? selection;
  }

  function fieldShortcuts(event: KeyboardEvent) {
    if (event.defaultPrevented || event.key !== 'Enter' || (!event.ctrlKey && !event.metaKey)) return;
    const target = event.target;
    if (!(target instanceof HTMLInputElement) || !target.closest('.precision-panel, .selection-commands')) return;
    event.preventDefault();
    target.blur();
    apply();
  }

  function asRepair(pick: Pick, strength: number): SpectralRepair {
    const { startFrame, endFrame, lowHz, highHz, channel: target, mask, name, bypassed, interpolate } = pick;
    return { startFrame, endFrame, lowHz, highHz, amountDb: strength, ...(target && stereo ? { channel: target } : {}),
      ...(mask ? { mask } : {}), ...(name ? { name } : {}), ...(bypassed ? { bypassed: true } : {}),
      ...(interpolate ? { interpolate: true } : {}) };
  }

  const listenRange = $derived(selection ? { start: selection.startFrame, end: selection.endFrame } : undefined);
  const listenBand = $derived(selection && validPick && selection.lowHz >= 20 && selection.highHz <= fullFrequency.highHz
    ? { lowHz: selection.lowHz, highHz: selection.highHz, channel: selection.channel } : undefined);
  const listenRepair = $derived.by(() => {
    if (!selection || selection.interpolate || !validPick || (!active && nextSlot >= editLimit)) return;
    return asRepair(selection, amount ?? defaultAmount);
  });

  function apply() {
    if (!selection || !validPick || interaction || inputError) return;
    if (selection.interpolate && !validClickBounds(selection)) return;
    const index = active && selection.index !== undefined ? selection.index : nextSlot;
    if (index >= editLimit) return;
    const updated = [...repairs];
    const repair = asRepair(selection, selection.interpolate ? 1 : amount ?? defaultAmount);
    if (!selection.interpolate) lastAmount = repair.amountDb;
    updated[index] = repair;
    onedit({ spectralRepairs: updated });
    selection = { ...selection, index, saved: repair };
  }

  function validClickBounds(bounds: RepairBounds) {
    return bounds.startFrame >= 2 && bounds.endFrame + 2 <= frames
      && bounds.endFrame > bounds.startFrame && bounds.endFrame - bounds.startFrame <= Math.floor(rate * .002)
      && bounds.lowHz <= 20 && bounds.highHz >= fullFrequency.highHz;
  }

  function singleTimeRegion(pick: Pick) {
    if (!pick.mask) return true;
    const [region] = pick.mask.regions;
    return pick.mask.regions.length === 1 && region?.kind === 'rectangle' && region.operation === 'add'
      && region.startFrame === pick.startFrame && region.endFrame === pick.endFrame
      && region.lowHz === 0 && region.highHz === Math.ceil(rate / 2);
  }

  function interpolateClick(bounds: RepairBounds) {
    if (!selection || !singleTimeRegion(selection) || active?.interpolate || nextSlot >= editLimit || !validClickBounds(bounds)) return;
    const index = nextSlot;
    const updated = [...repairs];
    const { startFrame, endFrame } = bounds;
    const pick = { startFrame, endFrame, lowHz: 20, highHz: fullFrequency.highHz, channel: selection.channel, interpolate: true };
    const repair = asRepair(pick, 1);
    updated[index] = repair;
    onedit({ spectralRepairs: updated });
    selection = { ...pick, index, saved: repair };
  }

  function remove() {
    const index = selection?.index;
    if (index === undefined || !active) return;
    restoredRepair = { repair: active, index };
    const updated = repairs.map((value, slot) => slot === index ? null : value);
    onedit({ spectralRepairs: updated.some(Boolean) ? updated : null });
    clear();
    focusPlot();
  }

  $effect(() => {
    const committed = repairs;
    untrack(() => {
      if (interaction) return;
      const pick = selection;
      if (pick?.index !== undefined && pick.saved) {
        const next = committed[pick.index];
        if (next && sameSavedRepair(next, pick.saved)) return;
        // A history action can change the committed slot while a new drawing
        // is in progress. Only follow the slot when its selection was saved.
        const strength = pick.interpolate ? 1 : amount ?? defaultAmount;
        if (!sameSavedRepair(asRepair(pick, strength), pick.saved)) return;
        if (next) chooseExisting(next, pick.index);
        else {
          restoredRepair = { repair: pick.saved, index: pick.index };
          clear();
        }
      } else if (!pick && restoredRepair) {
        const next = committed[restoredRepair.index];
        if (next && sameSavedRepair(next, restoredRepair.repair)) {
          chooseExisting(next, restoredRepair.index);
          restoredRepair = undefined;
        }
      }
    });
  });

  function updateMetadata(patch: PickMeta) {
    if (!selection || selection.index === undefined || !active) return;
    const slot = selection.index;
    const saved = { ...active, ...patch };
    if (!saved.name) delete saved.name;
    if (!saved.bypassed) delete saved.bypassed;
    onedit({ spectralRepairs: repairs.map((repair, index) => index === slot ? saved : repair) });
    selection = { ...selection, ...patch, saved };
  }

  type PickMeta = { name?: string; bypassed?: boolean };

  function feather(axis: 'featherFrames' | 'featherHz', value: number | null) {
    if (!selection || selection.interpolate || value === null) return;
    const regions = regionsFor(selection);
    selection = { ...selection, mask: { version: 1, featherFrames: selection.mask?.featherFrames ?? 0,
      featherHz: selection.mask?.featherHz ?? 0, [axis]: axis === 'featherFrames' ? Math.round(value * rate / 1000) : Math.round(value), regions } };
  }

  function boxStyle(pick: Pick, lane: RepairChannel) {
    const { time, frequency } = visible;
    if (pick.endFrame <= time.start || pick.startFrame >= time.end
      || (!waveformView && (pick.highHz <= frequency.lowHz || pick.lowHz >= frequency.highHz))) return '';
    const left = frameX(Math.max(pick.startFrame, time.start), bounds, time);
    const top = waveformView ? bounds.top : frequencyY(Math.min(pick.highHz, frequency.highHz), bounds, frequency);
    const right = frameX(Math.min(pick.endFrame, time.end), bounds, time);
    const bottom = waveformView ? bounds.bottom : frequencyY(Math.max(pick.lowHz, frequency.lowHz), bounds, frequency);
    return 'left:' + left + 'px;top:' + (top + laneOffset(lane)) + 'px;width:' + Math.max(1, right - left) + 'px;height:' + Math.max(1, bottom - top) + 'px';
  }

  function editLabel(pick: Pick) {
    const band = pick.lowHz === 0 && pick.highHz >= Math.ceil(rate / 2) ? 'Full band' : pick.lowHz + '–' + pick.highHz + ' Hz';
    return repairTimeLabel(pick.startFrame, rate, true) + '–' + repairTimeLabel(pick.endFrame, rate, true)
      + ' · ' + band + (pick.channel ? ' · ' + channelLabel[pick.channel] : '');
  }

  function editOverlay(pick: Pick, lane: RepairChannel) {
    return pick.mask?.regions.map(region => ({
      path: repairRegionPath(region, point => {
        const x = frameX(point.frame, bounds, visible.time);
        const y = frequencyY(Math.max(1, point.hz), bounds, visible.frequency) + laneOffset(lane);
        return x.toFixed(2) + ',' + y.toFixed(2);
      }),
      operation: region.operation,
      lasso: region.kind === 'lasso',
    }));
  }

  function featherPixels(pick: Pick) {
    if (!pick.mask) return 0;
    const { time, frequency } = visible;
    const timeWidth = pick.mask.featherFrames * (bounds.right - bounds.left) / (time.end - time.start);
    const center = Math.sqrt(Math.max(1, pick.lowHz) * pick.highHz);
    const freqWidth = frequencyY(center, bounds, frequency) - frequencyY(center + pick.mask.featherHz, bounds, frequency);
    return Math.min(30, Math.max(timeWidth, freqWidth));
  }

  function noteLabel(hz: number) {
    const pitch = Math.round(69 + 12 * Math.log2(hz / 440));
    const names = ['C', 'C♯', 'D', 'D♯', 'E', 'F', 'F♯', 'G', 'G♯', 'A', 'A♯', 'B'];
    return names[(pitch % 12 + 12) % 12] + (Math.floor(pitch / 12) - 1);
  }

  function fitContrast() {
    if (!probe?.spectrogram?.length || !matchesView(probe, probeView)) return;
    const peak = probe.spectrogram.reduce((max, column) => column.reduce((max, level) => Math.max(max, level), max), -140);
    const offset = Math.round(clampRepair(peak - repairDbScale(view.contrast).ceiling, -50, 20));
    changeView({ offset });
  }

  function overviewLandmark(pick: SpectralRepair, index: number) {
    chooseExisting(pick, index, true);
    jumpTo(pick.startFrame, false);
  }

  const analysisChannel = $derived(displayChannel === 'stereo' ? selection?.channel ?? 'combined' : channel);
  const analysisProbe = $derived.by(() => {
    if (!sourceProbe) return null;
    if (sourceProbe.spectrogramChannel !== 'stereo') return sourceProbe;
    const spectrogram = analysisChannel === 'left' ? sourceProbe.spectrogramLeft
      : analysisChannel === 'right' ? sourceProbe.spectrogramRight : sourceProbe.spectrogram;
    return { ...sourceProbe, spectrogram, spectrogramChannel: analysisChannel };
  });
  const minSelectableHz = $derived(selection?.mask ? 0 : 20);
  const maxSelectableHz = $derived(selection?.mask ? Math.ceil(rate / 2) : fullFrequency.highHz);
  const canInterpolate = $derived(!!selection && singleTimeRegion(selection) && !active?.interpolate && nextSlot < editLimit && validClickBounds(selection));

  onDestroy(() => {
    if (interaction) finish(undefined, false);
    if (hoverFrame) cancelAnimationFrame(hoverFrame);
    cancelView();
    clearTimeout(viewportTimer);
  });
</script>

<svelte:window onblur={() => finish()} onkeydown={fieldShortcuts}/>

<div class="repair-layout">
  <div class="selection-toolbar" role="group" aria-label="Spectrogram selection tools">
    <div class="draw-tools" role="group" aria-label="Selection shape">
      {#each selectionTools as { tool: mode, label, icon: Icon, shortcut, hint } (mode)}
        <Button class="draw-tool" variant="ghost" size="sm" aria-pressed={currentTool === mode} aria-keyshortcuts={shortcut}
          disabled={!viewportReady} title={hint + ' (' + shortcut + ')'} onclick={() => chooseTool(mode)}>
          <Icon size={14} aria-hidden="true"/>{label}
        </Button>
      {/each}
    </div>
    <div class="draw-tools operation-tools" role="group" aria-label="Combine selection regions">
      <Button class="draw-tool" variant="ghost" size="sm" aria-pressed={operation === 'add'} disabled={!viewportReady}
        title="Add to the current selection (Shift-drag)" onclick={() => operation = operation === 'add' ? 'replace' : 'add'}>Add</Button>
      <Button class="draw-tool" variant="ghost" size="sm" aria-pressed={operation === 'subtract'} disabled={!selection}
        title="Subtract from the current selection (Alt/Option-drag)" onclick={() => operation = operation === 'subtract' ? 'replace' : 'subtract'}>Subtract</Button>
    </div>
    {#if currentTool === 'frequency'}
      <label class="selection-scope">
        Across
        <select aria-label="Frequency selection scope" value={scope}
          onchange={event => { frequencyScope = event.currentTarget.value as FrequencyScope; }}>
          <option value="view">{scopeLabels.view}</option>
          <option value="loop" disabled={!loopRange}>{scopeLabels.loop}</option>
          <option value="recording">{scopeLabels.recording}</option>
        </select>
      </label>
    {:else if currentTool === 'time'}
      <span class="range-text">Full frequency band</span>
    {:else if currentTool === 'brush'}
      <Button variant="ghost" size="sm" aria-expanded={brushAdvanced} aria-controls={plotHelpID + '-brush'}
        title="Alt/Option-scroll changes brush width" onclick={() => brushAdvanced = !brushAdvanced}>Brush size</Button>
      {#if brushAdvanced}
        <div id={plotHelpID + '-brush'} class="brush-controls">
          <label>Time <input type="range" min={2} max={500} bind:value={brushMs}/> {brushMs} ms</label>
          <label>Frequency <input type="range" min={20} max={20_000} step={10} bind:value={brushHz}/> {brushHz} Hz</label>
        </div>
      {/if}
    {/if}
  </div>
  <div class="overview-bar">
    <div class="recording-overview" bind:clientWidth={overviewWidth} bind:clientHeight={overviewHeight}>
      <canvas aria-hidden="true" {@attach paintOverview}></canvas>
      {#if loopRange && snapshot?.loop}
        <span class="overview-loop" aria-hidden="true" style:left={String(loopRange.start * 100 / frames) + '%'}
          style:width={String((loopRange.end - loopRange.start) * 100 / frames) + '%'}></span>
      {/if}
      <span class="overview-window" aria-hidden="true"
        style:left={String(visible.time.start * 100 / frames) + '%'}
        style:width={String((visible.time.end - visible.time.start) * 100 / frames) + '%'}></span>
      {#each cuts as frame (frame)}
        <button type="button" tabindex="-1" class="overview-cut" style:left={String(frame * 100 / frames) + '%'}
          aria-label={'Jump to cut at ' + repairTimeLabel(frame, rate, true)} onclick={() => jumpTo(frame)}></button>
      {/each}
      {#if snapshot}
        <span class="overview-playhead" aria-hidden="true" {@attach playheadAttachment(true)}></span>
      {/if}
      <PlotButton class="overview-hit" type="button" aria-label="Jump in recording overview"
        aria-keyshortcuts="ArrowLeft ArrowRight PageUp PageDown Home End"
        title="Click to jump. Page Up and Page Down move between cuts and repairs. Arrow keys move by one second."
        disabled={!viewportReady} onclick={overviewClick} onkeydown={overviewKeys}/>
      {#each attached as { repair, index } (index)}
        <button type="button" tabindex="-1" class={['overview-repair', { bypassed: repair.bypassed }]} style:left={String(repair.startFrame * 100 / frames) + '%'}
          aria-label={'Jump to ' + (repair.name || 'repair ' + (index + 1))} title={repair.name || editLabel(repair)}
          onclick={() => overviewLandmark(repair, index)}></button>
      {/each}
    </div>
    <div class="image-scale">
      <span class="image-caption">{overviewMode ? 'Source waveform' : compareName + (waveformView ? ' waveform' : ' image')}</span>
      {#if !waveformView}
        <span class="db-legend" aria-label={legend.floor + ' to ' + legend.ceiling + ' dBFS'}>
          <span>{legend.floor}</span>
          <button type="button" class="db-gradient" style:--db-colours={repairPalettes[view.palette].gradient}
            aria-label="Adjust spectrogram level scale" aria-expanded={showColourScale} aria-controls={plotHelpID + '-levels'}
            onclick={() => showColourScale = !showColourScale}></button>
          <span>{legend.ceiling} dBFS</span>
        </span>
        {#if showColourScale}
          <div id={plotHelpID + '-levels'} class="waveform-opacity">
            <label>Scale offset
              <input type="range" min={-50} max={20} value={view.offset} aria-label="Spectrogram scale offset in dB"
                oninput={event => changeView({ offset: event.currentTarget.valueAsNumber })}/>
            </label>
            <Button variant="ghost" size="sm" onclick={() => changeView({ contrast: 'balanced', offset: 0 })}>Reset scale</Button>
          </div>
        {/if}
      {/if}
    </div>
  </div>

  <div class={['repair-plot', { panning: moving === 'pan', 'linked-stereo': displayChannel === 'stereo' }]}
    bind:clientWidth={width} bind:clientHeight={height}
    aria-label={overviewMode ? 'Source waveform overview' : compareName + (waveformView ? ' waveform' : ' spectrogram')} role="group"
    style:--repair-cursor={cursor} style:--spectrogram-bg={repairPalettes[view.palette].background}
    {@attach markPlot} {@attach wheelBinding} {@attach reflectView} {@attach followBinding}>
    <span id={plotHelpID} class="sr-only">Keys 1 to 5 choose Rectangle, Time, Frequency, Brush or Lasso. Time covers the full frequency band.
      Frequency covers the named time scope. Scroll to zoom in time or Shift-scroll for frequency. Scroll sideways or middle-drag to pan.
      Left and Right adjust start time. Up and Down adjust the low frequency. Stripe tools edit their own axis.
      Hold Shift to adjust the other edge. Enter auditions the time range. Shift+Enter auditions the band.
      Ctrl or Command+Enter applies the selection. F fits the selection. R repeats the last reduction. Zero resets the view.
      Shift-drag adds and Alt or Option-drag subtracts. Use the ruler or Ctrl or Command-click to seek.
      Escape cancels a drag or clears the selection. Delete removes a saved repair.</span>
    <span class="sr-only" role="status">
      {toolDescription?.label} selection tool{currentTool === 'frequency' ? ' · ' + scopeLabels[scope] : ''}
      {selection ? ' · ' + editLabel(selection) : ''}
    </span>
    {#if selectionNotice}<span class="view-status" role="status">{selectionNotice}</span>{/if}
    {#if !selectionNotice && overviewMode}
      <span class="view-status" role="status">Source waveform · zoom in for spectral detail</span>
    {:else if !selectionNotice && updatingView}
      <span class="view-status" role="status">Updating spectrogram…</span>
    {/if}
    {#each displayLanes as lane (lane)}
      <canvas aria-hidden="true" class={{ 'lower-lane': displayChannel === 'stereo' && lane === 'right' }} {@attach attachPaint(lane)}></canvas>
      {#if displayChannel === 'stereo'}
        <span class="lane-label" style:top={(laneOffset(lane) + bounds.top + 4) + 'px'}>{channelLabel[lane]}</span>
      {/if}
    {/each}
    <PlotButton class="repair-hit" type="button" disabled={!viewportReady}
      aria-label="Select time and frequency" aria-describedby={plotHelpID}
      onpointerdown={begin} onpointermove={move} onpointerup={end} onpointercancel={end} onlostpointercapture={end}
      onpointerleave={clearHover}
      onauxclick={event => event.preventDefault()} onkeydown={keys}/>
    {#if snapshot}
      <span class="playhead" aria-hidden="true" {@attach playheadAttachment(false)}
        style:top={bounds.top + 'px'} style:bottom={height - bounds.bottom - laneOffset(displayLanes.at(-1) ?? 'combined') + 'px'}></span>
    {/if}
    {#each displayLanes as lane (lane)}
      {#if loopRange && snapshot?.loop && loopRange.start < visible.time.end && loopRange.end > visible.time.start}
        {@const left = frameX(Math.max(visible.time.start, loopRange.start), bounds, visible.time)}
        {@const right = frameX(Math.min(visible.time.end, loopRange.end), bounds, visible.time)}
        <span class="loop-bracket" aria-hidden="true" style:left={left + 'px'} style:top={(bounds.bottom + laneOffset(lane) - 5) + 'px'}
          style:width={(right - left) + 'px'}></span>
      {/if}
      {#each drawnRepairs as { repair, index } (index)}
        {@const position = lane === 'combined' || !repair.channel || repair.channel === lane ? boxStyle(repair, lane) : ''}
        {#if position}<span class={['applied', { shaped: !!repair.mask, bypassed: repair.bypassed }]} aria-hidden="true" style={position}></span>{/if}
      {/each}
      {#if selected && (lane === 'combined' || !selected.channel || selected.channel === lane)}
        {@const position = boxStyle(selected, lane)}
        {@const rect = rectangle(selected)}
        {@const paths = waveformView ? undefined : editOverlay(selected, lane)}
        {#if position}
          {#if paths}
            {@const maskId = plotHelpID + '-mask-' + lane}
            <svg class="shape-overlay" aria-hidden="true" viewBox={'0 0 ' + width + ' ' + height}>
              <defs><mask id={maskId} maskUnits="userSpaceOnUse" x={0} y={0} {width} {height}>
                {#each paths as shape, index (index)}
                  <path d={shape.path} fill={shape.operation === 'add' ? 'white' : 'black'} fill-rule={shape.lasso ? 'evenodd' : 'nonzero'}/>
                {/each}
              </mask></defs>
              {#if featherPixels(selected) > 0}
                <rect x={bounds.left} y={bounds.top + laneOffset(lane)} width={bounds.right - bounds.left} height={bounds.bottom - bounds.top}
                  mask={'url(#' + maskId + ')'} opacity=".72" style:filter={'blur(' + featherPixels(selected).toFixed(1) + 'px)'}/>
              {/if}
              <rect x={bounds.left} y={bounds.top + laneOffset(lane)} width={bounds.right - bounds.left} height={bounds.bottom - bounds.top}
                mask={'url(#' + maskId + ')'}/>
            </svg>
          {/if}
          <span class={['picked', { shaped: !!paths, audition: snapshot?.monitor === 'band' }]} aria-hidden="true" style={position}></span>
          {#if !moving}
            {#each [[rect.left, rect.top], [rect.right, rect.top], [rect.left, rect.bottom], [rect.right, rect.bottom]] as [x, y], index (index)}
              {#if x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom}
                <span class="repair-handle" aria-hidden="true" style:left={x + 'px'} style:top={(y + laneOffset(lane)) + 'px'}></span>
              {/if}
            {/each}
          {/if}
        {/if}
      {/if}
    {/each}
    {#if hover && !moving}
      <span class="pointer-guide" aria-hidden="true" style:left={hover.x + 'px'} style:top={hover.y + 'px'}></span>
      <span class="pointer-readout" aria-hidden="true">
        {repairTimeLabel(hover.frame, rate, true)}
        {#if !waveformView}
          · {Math.round(hover.hz)} Hz · {noteLabel(hover.hz)}
          {#if hoverLevel !== undefined} · ~{Math.round(hoverLevel)} dBFS{/if}
        {/if}
      </span>
      {#if currentTool === 'brush'}
        {@const xRadius = Math.min(width / 2, (bounds.right - bounds.left) * rate * brushMs / 2000 / (visible.time.end - visible.time.start))}
        {@const yRadius = Math.max(2, Math.abs(frequencyY(Math.max(1, hover.hz - brushHz / 2), bounds, visible.frequency)
          - frequencyY(hover.hz + brushHz / 2, bounds, visible.frequency)) / 2)}
        <span class="brush-footprint" aria-hidden="true" style:left={hover.x + 'px'} style:top={hover.y + 'px'}
          style:width={(xRadius * 2) + 'px'} style:height={Math.min(laneHeight, yRadius * 2) + 'px'}></span>
      {/if}
    {/if}
  </div>

  <div class="repair-rail">{@render modeRail()}</div>

  <div class="repair-selection">
    <div class="repair-summary">
      <span class="range-text selection-readout">{selected ? editLabel(selected) : toolDescription?.hint}</span>
      <div class="view-controls" aria-label="Spectrogram controls" role="group">
        <Menu controller={viewMenu}>
          <MenuTrigger class="repair-menu" aria-label="Spectrogram display" aria-description={compareName + ', ' + channelLabel[displayChannel]}
            title={'Spectrogram display · ' + compareName + ', ' + channelLabel[displayChannel]} disabled={!viewportReady}>
            <SlidersHorizontal size={15} aria-hidden="true"/>
          </MenuTrigger>
          <MenuContent class="app-select-menu audio-tool-popup" align="end" sideOffset={6} collisionPadding={8}>
            <MenuRadioGroup value={view.difference ? 'removed' : view.treated ? 'treated' : 'source'}
              onValueChange={value => changeView({ treated: value !== 'source', difference: value === 'removed' })}>
              <MenuGroupLabel>Spectrogram</MenuGroupLabel>
              <MenuRadioItem data-slot="select-item" value="source" closeOnSelect>
                <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>Source
              </MenuRadioItem>
              <MenuRadioItem data-slot="select-item" value="treated" closeOnSelect>
                <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>Treated
              </MenuRadioItem>
              <MenuRadioItem data-slot="select-item" value="removed" closeOnSelect>
                <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>Removed
              </MenuRadioItem>
            </MenuRadioGroup>
            {#if stereoDisplay}
              <MenuSeparator/>
              <MenuRadioGroup value={displayChannel} onValueChange={value => changeView({ channel: value as DisplayChannel })}>
                <MenuGroupLabel>Display channel</MenuGroupLabel>
                {#each displayChannels as mode (mode)}
                  <MenuRadioItem data-slot="select-item" value={mode} closeOnSelect>
                    <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>{channelLabel[mode]}
                  </MenuRadioItem>
                {/each}
              </MenuRadioGroup>
            {/if}
            <MenuSeparator/>
            <MenuRadioGroup value={view.contrast} onValueChange={value => changeView({ contrast: value as RepairContrast })}>
              <MenuGroupLabel>Contrast</MenuGroupLabel>
              {#each Object.entries(contrastLabel) as [mode, label] (mode)}
                <MenuRadioItem data-slot="select-item" value={mode} closeOnSelect>
                  <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>{label}
                </MenuRadioItem>
              {/each}
            </MenuRadioGroup>
            <MenuSeparator/>
            <MenuRadioGroup value={view.palette} onValueChange={value => changeView({ palette: value as RepairPaletteName })}>
              <MenuGroupLabel>Palette</MenuGroupLabel>
              {#each Object.entries(paletteLabel) as [mode, label] (mode)}
                <MenuRadioItem data-slot="select-item" value={mode} closeOnSelect>
                  <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>{label}
                </MenuRadioItem>
              {/each}
            </MenuRadioGroup>
            <MenuSeparator/>
            <MenuRadioGroup value={view.detail} onValueChange={value => changeView({ detail: value as Detail })}>
              <MenuGroupLabel>Detail</MenuGroupLabel>
              {#each Object.entries(detailLabel) as [mode, label] (mode)}
                <MenuRadioItem data-slot="select-item" value={mode} closeOnSelect>
                  <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>{label}
                </MenuRadioItem>
              {/each}
            </MenuRadioGroup>
            <MenuSeparator/>
            <MenuItem data-slot="select-item" disabled={waveformView} onSelect={fitContrast}>Fit contrast to view</MenuItem>
            <MenuItem data-slot="select-item" onSelect={() => changeView({ contrast: 'balanced', offset: 0 })}>Reset contrast</MenuItem>
            <MenuSeparator/>
            <MenuItem data-slot="select-item" disabled={!selection} onSelect={() => fitSelection('time')}>Fit selection in time</MenuItem>
            <MenuItem data-slot="select-item" disabled={!selection || waveformView} onSelect={() => fitSelection('frequency')}>Fit selection in frequency</MenuItem>
            <MenuItem data-slot="select-item" disabled={!history.length} onSelect={previousCamera}>Previous zoom</MenuItem>
            <MenuSeparator/>
            <MenuCheckboxItem data-slot="select-item" value="waveform" checked={view.waveform}
              onCheckedChange={checked => changeView({ waveform: checked === true })} closeOnSelect>
              <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>Waveform overlay
            </MenuCheckboxItem>
            <MenuCheckboxItem data-slot="select-item" value="waveform-only" checked={view.waveformOnly}
              onCheckedChange={checked => changeView({ waveformOnly: checked === true })} closeOnSelect>
              <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>Waveform only
            </MenuCheckboxItem>
          </MenuContent>
        </Menu>
        {#if view.waveform && !view.waveformOnly}
          <label class="waveform-opacity">Waveform
            <input type="range" min={0} max={100} value={view.opacity} aria-label="Waveform opacity"
              oninput={event => changeView({ opacity: event.currentTarget.valueAsNumber })}/>
          </label>
        {/if}
        <Button variant="ghost" size="icon-sm" aria-label="Follow playhead" title="Follow playhead" aria-pressed={!manual} disabled={!viewportReady} onclick={toggleFollow}>
          <LocateFixed size={15} aria-hidden="true"/>
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Fit selected area" aria-keyshortcuts="F" title="Fit selected area (F)" disabled={!selection} onclick={() => fitSelection()}>
          <Crosshair size={15} aria-hidden="true"/>
        </Button>
        <Button variant="ghost" size="sm" aria-keyshortcuts="0" title="Fit recording (0)" disabled={!viewportReady} onclick={fitRecording}>
          <Maximize2 size={15} aria-hidden="true"/> Fit recording
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Clear selected area" aria-keyshortcuts="Escape" title="Clear selected area (Esc)"
          disabled={!selection} onclick={() => { clear(); focusPlot(); }}><X size={15} aria-hidden="true"/></Button>
        <Menu controller={editMenu}>
          <MenuTrigger class="repair-menu" aria-label="Applied repairs" title="Applied repairs" disabled={!attached.length}>
            <Layers size={15} aria-hidden="true"/>
          </MenuTrigger>
          <MenuContent class="app-select-menu audio-tool-popup" align="end" sideOffset={6} collisionPadding={8}>
            {#each attached as { repair, index } (index)}
              {@const label = repair.name ? repair.name + ' · ' + editLabel(repair) : editLabel(repair)}
              <MenuItem data-slot="select-item" {label} onSelect={() => chooseExisting(repair, index, true)}>
                {label} · {repair.interpolate ? 'Click interpolation' : '−' + repair.amountDb + ' dB'}{repair.bypassed ? ' · Bypassed' : ''}
              </MenuItem>
            {/each}
          </MenuContent>
        </Menu>
      </div>
    </div>
    <div class="edit-controls">
      {#if listen}{@render listen(listenRange, listenBand, listenRepair, active ? selection?.index : undefined)}{/if}
      <div class="selection-commands" aria-label="Selection actions" role="group">
        {#if selection}
          {#if !listen}
            <Button variant="ghost" size="sm" aria-label="Audition selected area" title="Audition time range (Enter)" aria-keyshortcuts="Enter" onclick={audition}>
              <Play size={13} aria-hidden="true"/>Listen
            </Button>
            <Button variant="ghost" size="sm" aria-label="Audition selected frequencies only" title="Audition only this band (Shift+Enter)"
              aria-keyshortcuts="Shift+Enter" aria-pressed={snapshot?.monitor === 'band'} disabled={!listenBand} onclick={auditionBand}>
              <Headphones size={14} aria-hidden="true"/>Band only
            </Button>
          {/if}
          {#if stereo}
            <Menu controller={channelMenu}>
              <MenuTrigger class="repair-channel" aria-label="Apply repair to channel" title="Choose which channels the repair affects">{selection.channel ? channelLabel[selection.channel] : 'Both channels'}</MenuTrigger>
              <MenuContent class="app-select-menu audio-tool-popup" align="end" sideOffset={6} collisionPadding={8}>
                <MenuGroup>
                  <MenuGroupLabel>Apply repair to</MenuGroupLabel>
                  <MenuRadioGroup value={selection.channel ?? 'combined'} onValueChange={value => {
                    if (selection) selection = { ...selection, channel: value === 'left' || value === 'right' ? value : undefined };
                  }}>
                    {#each channels as mode (mode)}
                      <MenuRadioItem data-slot="select-item" value={mode} closeOnSelect>
                        <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>{mode === 'combined' ? 'Both channels' : channelLabel[mode] + ' only'}
                      </MenuRadioItem>
                    {/each}
                  </MenuRadioGroup>
                </MenuGroup>
              </MenuContent>
            </Menu>
          {/if}
          {#if !selection.interpolate}
            <NumberInput class="repair-number" bind:value={amount} min={1} max={36} step={1}>
              <NumberInputControl class="repair-amount" aria-label="Attenuation in dB" title="Attenuation in dB" placeholder={String(defaultAmount)}/>
              <span aria-hidden="true">dB</span>
            </NumberInput>
          {/if}
          {#if active}
            <Button variant="ghost" size="sm" aria-pressed={!!active.bypassed}
              onclick={() => updateMetadata({ bypassed: !active?.bypassed })}>Bypass</Button>
            <Button variant="ghost" size="icon-sm" aria-label="Remove repair" title="Remove repair (Delete)" aria-keyshortcuts="Delete"
              onclick={remove}><Trash2 size={14} aria-hidden="true"/></Button>
          {/if}
        {/if}
        <div class="commit-controls">
          <Button variant="ghost" size="sm" aria-label="Exact selection boundaries" title="Edit exact selection boundaries"
            aria-expanded={precise && !!selection} aria-controls={plotHelpID + '-bounds'} disabled={!selection}
            onclick={() => void toggleBounds()}>Bounds</Button>
          <Button size="sm" disabled={!selection || !validPick || (!active && nextSlot >= editLimit) || !!inputError || (!!selection.interpolate && !validClickBounds(selection))}
            title={!selection ? 'Draw a selection to reduce' : !validPick ? 'Select at least 20 Hz'
              : !active && nextSlot >= editLimit ? 'All eight repair slots are used' : 'Apply selected reduction (Ctrl/⌘+Enter)'}
            aria-keyshortcuts="Control+Enter Meta+Enter" onclick={apply}>
            <Check size={14} aria-hidden="true"/>{active ? 'Update' : 'Reduce'}
          </Button>
        </div>
      </div>
    </div>
    {#if precise && selection}
      <div class="precision-panel" id={plotHelpID + '-bounds'} role="group" aria-label="Exact selection boundaries">
        {#each ['startFrame', 'endFrame', 'duration'] as edge (edge)}
          {@const key = edge as TimeEdge}
          <label>
            <span>{timeLabels[key]}</span>
            <Input class="repair-time-input" value={timeLabel(timeValue(key, selection), rate, 6)}
              aria-invalid={inputError?.edge === key} aria-describedby={inputError?.edge === key ? plotHelpID + '-error' : undefined}
              onfocus={event => {
                if (!selection || inputError?.edge === key) return;
                event.currentTarget.value = timeLabel(timeValue(key, selection), rate, 6);
                event.currentTarget.select();
              }}
              onblur={event => commitTime(event.currentTarget, key)}
              onkeydown={event => exactTimeKeys(event, key)}/>
          </label>
        {/each}
        {#if !selection.interpolate}
          {#each ['lowHz', 'highHz'] as edge (edge)}
            {@const key = edge as FrequencyEdge}
            {@const name = key === 'lowHz' ? 'Low frequency' : 'High frequency'}
            <label>
              <span>{name}</span>
              <NumberInput class="repair-number" value={selection[key]} min={key === 'lowHz' ? minSelectableHz : Math.min(maxSelectableHz, selection.lowHz + 20)}
                max={key === 'lowHz' ? Math.max(minSelectableHz, selection.highHz - 20) : maxSelectableHz} step={1} largeStep={10}
                onValueCommit={value => editFrequency(key, value)}>
                <NumberInputControl class="repair-hertz-input" aria-label={name + ' in Hz'}/>
                <span aria-hidden="true">Hz</span>
              </NumberInput>
            </label>
          {/each}
          <label>
            <span>Time feather</span>
            <NumberInput class="repair-number" value={Math.round((selection.mask?.featherFrames ?? 0) * 1000 / rate)} min={0} max={1000} step={1}
              onValueCommit={value => feather('featherFrames', value)}>
              <NumberInputControl class="repair-hertz-input" aria-label="Time feather in ms"/><span aria-hidden="true">ms</span>
            </NumberInput>
          </label>
          <label>
            <span>Frequency feather</span>
            <NumberInput class="repair-number" value={selection.mask?.featherHz ?? 0} min={0} max={10_000} step={1}
              onValueCommit={value => feather('featherHz', value)}>
              <NumberInputControl class="repair-hertz-input" aria-label="Frequency feather in Hz"/><span aria-hidden="true">Hz</span>
            </NumberInput>
          </label>
        {/if}
        {#if active}
          <label>
            <span>Repair name</span>
            <Input class="repair-time-input" value={selection.name ?? ''} maxlength={80}
              onchange={event => updateMetadata({ name: event.currentTarget.value.trim() })}/>
          </label>
        {/if}
        {#if inputError}<p class="boundary-error" id={plotHelpID + '-error'} role="status">{inputError.message}</p>{/if}
      </div>
    {/if}
    <AudioSplitRepairAnalysis probe={analysisProbe} {selection} {rate} maxFrames={detailLimit} channel={analysisChannel} target={selection?.channel} onselect={selectSuggestion}
      {canInterpolate} oninterpolate={interpolateClick}
      onsource={transients => {
        if (!selection) return;
        changeView({ treated: false, difference: false, waveformOnly: false, ...(transients ? { detail: 'transients' as const } : {}) });
        const { startFrame, endFrame } = selection;
        const size = Math.min(detailLimit, Math.max(rate / 4, (endFrame - startFrame) * 1.15));
        const time = boundedTime((startFrame + endFrame - size) / 2, size, rate, frames);
        rememberCamera();
        setManual({ ...visible, time }, 140);
        requestViewport(time);
      }}
      onaudition={event => onaction('loop', {
        start: Math.max(0, event.startFrame - Math.round(rate / 2)),
        end: Math.min(frames, event.endFrame + Math.round(rate / 2)),
      })}/>
  </div>

</div>

<style>
  .repair-layout {
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-rows: auto auto minmax(218px, 1fr) auto;
    gap: var(--sp-2, 8px);
  }
  .recording-overview, .repair-plot {
    position: relative;
    isolation: isolate;
    overflow: hidden;
    overflow: clip;
  }
  .overview-bar { grid-column: 1 / -1; display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .recording-overview { flex: 1; min-width: 0; min-height: 28px; border-radius: var(--r-compact, 8px); }
  .selection-toolbar { grid-column: 1 / -1; display: flex; align-items: center; flex-wrap: wrap; gap: var(--sp-1) var(--sp-2); }
  .draw-tools { display: flex; align-items: center; flex-wrap: wrap; gap: var(--sp-1); min-width: 0; }
  .draw-tools :global(.draw-tool) { min-block-size: 28px; padding-inline: var(--sp-2); border: 0; border-radius: var(--r-compact); }
  .draw-tools :global(.draw-tool[aria-pressed=true]) { color: var(--text); background: var(--sel-tint); }
  .draw-tools :global(.draw-tool[aria-pressed=true] svg) { color: var(--accent-ink); }
  .operation-tools { padding-inline-start: var(--sp-2); border-inline-start: 1px solid var(--border); }
  .selection-scope { display: flex; align-items: center; gap: var(--sp-1); color: var(--muted); font-size: var(--text-xs); }
  .selection-scope select { min-block-size: 28px; padding-inline: var(--sp-2) var(--sp-1); background: var(--row-hover); color: var(--text); border: 0; border-radius: var(--input-radius); }
  .selection-scope select:focus-visible { outline: 2px solid var(--accent); }
  .image-scale { display: flex; align-items: center; flex-wrap: wrap; gap: var(--sp-1) var(--sp-2); }
  .image-caption { color: var(--text); font-size: var(--text-xs); white-space: nowrap; }
  .db-legend { display: flex; align-items: center; gap: var(--sp-1); color: var(--muted); font-size: var(--text-xs); }
  .db-gradient { width: 72px; height: 24px; padding: 0; border: 0; border-radius: var(--r-compact); background: transparent; }
  .db-gradient::before { content: ''; display: block; height: 7px; border-radius: var(--r-pill); background: linear-gradient(90deg, var(--db-colours)); }
  .db-gradient:focus-visible { outline: 2px solid var(--accent); }
  .brush-controls, .waveform-opacity { display: flex; align-items: center; flex-wrap: wrap; gap: var(--sp-1); color: var(--muted); font-size: var(--text-xs); }
  .brush-controls label { display: flex; align-items: center; gap: var(--sp-1); }
  .brush-controls input, .waveform-opacity input { max-width: 74px; accent-color: var(--accent); }
  .recording-overview canvas, .recording-overview :global(.overview-hit),
  .repair-plot canvas, .repair-plot :global(.repair-hit) {
    position: absolute;
    width: 100%;
    height: 100%;
    inset: 0;
    border-radius: inherit;
  }
  canvas { pointer-events: none; }
  .recording-overview :global(.overview-hit), .repair-plot :global(.repair-hit) {
    appearance: none;
    background: transparent;
    border: 0;
    padding: 0;
    touch-action: none;
  }
  .recording-overview :global(.overview-hit) { cursor: pointer; }
  .recording-overview :global(.overview-hit:disabled), .repair-plot :global(.repair-hit:disabled) { cursor: default; }
  .recording-overview :global(.overview-hit:focus-visible), .repair-plot :global(.repair-hit:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .overview-window, .overview-loop, .overview-playhead {
    position: absolute;
    inset-block: 0;
    pointer-events: none;
  }
  .overview-window {
    min-width: 2px;
    box-shadow: inset 0 0 0 1px var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-radius: var(--r-media-edge, 2px);
  }
  .overview-loop { background: color-mix(in srgb, var(--accent-ink, var(--accent)) 22%, transparent); }
  .overview-cut { position: absolute; z-index: 1; inset-block: 0; width: 10px; padding: 0; transform: translateX(-50%); background: transparent; border: 0; }
  .overview-cut::before { content: ''; display: block; margin-inline: auto; width: 1px; height: 100%; background: var(--muted); opacity: .65; }
  .overview-cut:focus-visible::before { width: 2px; opacity: 1; background: var(--accent-ink); }
  .overview-playhead { left: 0; width: 2px; background: var(--accent-ink, var(--text)); }
  .overview-repair { position: absolute; bottom: 0; min-width: 14px; min-height: 14px; padding: 0; transform: translateX(-50%); color: var(--accent-ink); background: transparent; border: 0; }
  .overview-repair::before { content: ''; display: inline-block; width: 7px; height: 7px; transform: rotate(45deg); border-radius: 1px; background: currentColor; }
  .overview-repair.bypassed { opacity: .45; }
  .repair-plot { min-height: 218px; border-radius: var(--r-inset, 14px); background: var(--spectrogram-bg, #1a1015); }
  .repair-plot.linked-stereo { min-height: 312px; }
  .linked-stereo canvas { height: 50%; border-radius: 0; }
  .linked-stereo canvas.lower-lane { inset-block-start: 50%; border-top: 1px solid #45404a; }
  .lane-label { position: absolute; right: 26px; pointer-events: none; color: #f2ebf8; font-size: var(--text-xs); }
  .repair-plot :global(.repair-hit) { cursor: var(--repair-cursor, crosshair); }
  .panning :global(.repair-hit) { cursor: grabbing; }
  .playhead, .applied, .picked, .repair-handle, .view-status { position: absolute; pointer-events: none; }
  .playhead { left: 0; z-index: 1; width: 1px; background: #fff4d7; opacity: .88; }
  .picked {
    border: 1px solid var(--accent-ink, var(--text));
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .picked.shaped { border-style: dotted; background: transparent; }
  .picked.audition { border-style: dashed; border-color: #f8deb0; }
  .shape-overlay { position: absolute; inset: 0; width: 100%; height: 100%; fill: color-mix(in srgb, var(--accent) 32%, transparent); pointer-events: none; }
  .loop-bracket { position: absolute; height: 5px; pointer-events: none; border: solid #f7dba7; border-width: 0 1px 2px; opacity: .9; }
  .brush-footprint, .pointer-guide { position: absolute; pointer-events: none; transform: translate(-50%, -50%); }
  .brush-footprint { border: 1px solid #f2ebf8; border-radius: 50%; }
  .pointer-guide { width: 13px; height: 13px; opacity: .75; }
  .pointer-guide::before, .pointer-guide::after { content: ''; position: absolute; background: #f2ebf8; }
  .pointer-guide::before { width: 13px; height: 1px; top: 6px; }
  .pointer-guide::after { width: 1px; height: 13px; left: 6px; }
  .pointer-readout { position: absolute; top: var(--sp-1); left: 42px; padding-inline: var(--sp-1); pointer-events: none; color: #f2ebf8; background: var(--spectrogram-bg); font-size: var(--text-xs); font-variant-numeric: tabular-nums; }
  .repair-handle {
    width: 7px;
    height: 7px;
    transform: translate(-50%, -50%);
    border: 1px solid var(--bg);
    border-radius: var(--r-media-edge, 2px);
    background: var(--accent-ink, var(--text));
  }
  .applied {
    border: 1px dashed color-mix(in srgb, var(--accent-ink) 78%, transparent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .applied.shaped { background: transparent; }
  .applied.bypassed { opacity: .42; }
  .view-status {
    z-index: 1;
    inset-block-start: var(--sp-2, 8px);
    inset-inline-end: var(--sp-2, 8px);
    padding: var(--sp-1, 4px) var(--sp-2, 8px);
    border-radius: var(--r-pill, 999px);
    color: var(--muted);
    background: var(--panel);
    font-size: var(--text-xs);
  }
  .repair-selection {
    grid-column: 1 / -1;
    display: grid;
    gap: var(--sp-1, 4px);
  }
  .repair-summary {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-1, 4px) var(--sp-2, 8px);
  }
  .range-text { color: var(--muted); font-size: var(--text-xs); font-variant-numeric: tabular-nums; }
  .edit-controls { display: flex; align-items: center; flex-wrap: wrap; gap: var(--sp-1) var(--sp-2); min-width: 0; }
  .edit-controls :global(.listen:has(details[open])) { flex-basis: 100%; }
  .selection-commands {
    margin-inline-start: auto;
    min-width: 0;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-1, 4px);
  }
  .commit-controls { display: flex; align-items: center; gap: var(--sp-1); margin-inline-start: auto; }
  .repair-layout :global(.repair-menu), .repair-layout :global(.repair-channel) {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    min-height: 28px;
    border: 0;
    border-radius: var(--r-compact, 8px);
    color: var(--muted);
    background: transparent;
  }
  .repair-layout :global(.repair-menu:hover), .repair-layout :global(.repair-channel:hover) {
    color: var(--accent-ink);
  }
  .repair-layout :global(.repair-menu:disabled) { opacity: .45; }
  .repair-layout :global(.repair-channel) { padding-inline: var(--sp-2, 8px); font-size: var(--text-xs); }
  .repair-layout :global(.repair-number), .repair-layout :global(.repair-time-input) {
    display: flex;
    align-items: center;
    min-width: 0;
    padding: 0 6px;
    gap: 3px;
    min-height: 28px;
    font-size: var(--text-xs);
    border-radius: var(--input-radius, 10px);
    color: var(--text);
    background: transparent;
  }
  .repair-layout :global(.repair-number:hover), .repair-layout :global(.repair-time-input:hover) { background:var(--row-hover); }
  .repair-layout :global(.repair-amount), .repair-layout :global(.repair-hertz-input), .repair-layout :global(.repair-time-input) {
    color: var(--text);
    border: 0;
    font-variant-numeric: tabular-nums;
    outline: none;
  }
  .repair-layout :global(.repair-amount), .repair-layout :global(.repair-hertz-input) {
    background: transparent;
    padding: 5px 0;
    text-align: right;
  }
  .repair-layout :global(.repair-amount) { width: 3ch; }
  .repair-layout :global(.repair-hertz-input), .repair-layout :global(.repair-time-input) { width: 100%; min-width: 0; }
  .repair-layout :global(.repair-number:focus-within), .repair-layout :global(.repair-time-input:focus-visible),
  .repair-layout :global(.repair-menu:focus-visible), .repair-layout :global(.repair-channel:focus-visible) {
    outline: 2px solid var(--accent);
  }
  .repair-layout :global(.repair-time-input[aria-invalid=true]) { outline: 2px solid var(--danger, var(--destructive)); }
  .precision-panel {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(110px, 1fr));
    gap: var(--sp-2, 8px);
    padding-block-start: var(--sp-1);
  }
  .precision-panel label { display: grid; gap: var(--sp-1, 4px); min-width: 0; color: var(--muted); font-size: var(--text-xs); }
  .boundary-error { grid-column: 1 / -1; margin: 0; color: var(--danger, var(--destructive)); font-size: var(--text-xs); }
  .repair-rail { align-self:start; }
  .view-controls { display:flex; align-items:center; flex-wrap:wrap; gap:var(--sp-1); margin-inline-start:auto; }
  .view-controls :global([aria-pressed=true]), .selection-commands :global([aria-pressed=true]) { color:var(--accent-ink); }
  @media (max-width: 480px) {
    .draw-tools:not(.operation-tools) { flex-basis: 100%; display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); }
    .draw-tools :global(.draw-tool) { padding-inline: 3px; }
    .draw-tools :global(.draw-tool svg) { display: none; }
    .image-scale { gap: var(--sp-1); }
    .db-gradient { width: 44px; }
  }
  @media (max-height: 420px) {
    .repair-layout { grid-template-rows: auto auto minmax(150px, 1fr) auto; }
    .repair-plot { min-height: 150px; }
    .repair-plot.linked-stereo { min-height: 236px; }
  }
</style>
