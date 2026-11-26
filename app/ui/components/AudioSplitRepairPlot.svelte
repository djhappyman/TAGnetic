<script lang="ts">
  import { onDestroy, untrack, type Snippet } from 'svelte';
  import { devicePixelRatio } from 'svelte/reactivity/window';
  import { Check, Crosshair, Headphones, Layers, LocateFixed, Play, RotateCcw, SlidersHorizontal, Trash2, X } from '@lucide/svelte';
  import { Button as PlotButton } from '@ui-sdk/svelte/button';
  import { Input } from '@ui-sdk/svelte/input';
  import { createMenuController, Menu, MenuCheckboxItem, MenuContent, MenuGroup, MenuGroupLabel, MenuItem, MenuItemIndicator, MenuRadioGroup, MenuRadioItem, MenuSeparator, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';
  import Button from './AudioSplitButton.svelte';
  import { plotBounds, type TreatmentProbe } from '../js/audio-treatment-plot';
  import {
    boundedTime, buildRepairTexture, clampRepair, drawRepairOverview, drawRepairViewport, frameAtX, frameX, frequencyAtY,
    frequencyY, recordingFrequencyRange, repairTimeLabel, spectrogramRange,
    type FrequencyRange, type RepairChannel, type RepairContrast, type RepairTexture, type RepairViewport, type RepairWaveform,
  } from '../js/audio-repair-plot';
  import { edgePanSpeed, editRepairBounds, repairDragAt, repairDragCursor, type RepairDrag } from '../js/audio-repair-selection';
  import { maxSpectralRepairs, sameRepairBounds, type SpectralRepair, type AudioTreatment } from '../js/audio-treatment';
  import type { PreviewBand, ToolAction, ToolSnapshot } from '../js/audio-tools-window';
  import { frameAtTime, timeLabel, type FrameRange } from '../js/audio-split';

  let { snapshot, probe, onviewport, onedit, onaction, modeRail }: {
    snapshot: ToolSnapshot | null;
    probe: TreatmentProbe | null;
    onviewport: (range: FrameRange | null, options?: { treated?: boolean; channel?: RepairChannel }) => void;
    onedit: (patch: Partial<AudioTreatment>, gesture?: string) => void;
    onaction: (action: ToolAction, range?: FrameRange, band?: PreviewBand) => void;
    modeRail: Snippet;
  } = $props();

  type Pick = Omit<SpectralRepair, 'amountDb'> & { index?: number; saved?: SpectralRepair };
  type Gesture = {
    pointer: number; node: HTMLButtonElement; rect: DOMRect;
    firstX: number; firstY: number; firstFrame: number; firstHz: number;
    x: number; y: number; mode: 'pick' | 'pan' | 'edit'; moved: boolean;
    origin: RepairViewport; previous: RepairViewport | null;
    baseline?: { pick: Pick; handles: RepairDrag; amount: number };
  };
  type ViewOptions = { treated: boolean; channel: RepairChannel; waveform: boolean; contrast: RepairContrast };
  type ProbeView = { treated: boolean; channel: RepairChannel };
  type TimeEdge = 'startFrame' | 'endFrame';
  type FrequencyEdge = 'lowHz' | 'highHz';
  const defaultAmount = 12;
  const editLimit = maxSpectralRepairs;
  const editMenu = createMenuController();
  const viewMenu = createMenuController();
  const channelMenu = createMenuController();
  const plotHelpID = $props.id();
  const noRepairs: readonly (SpectralRepair | null)[] = [];
  const channels: readonly RepairChannel[] = ['combined', 'left', 'right'];
  const channelLabel = { combined: 'Combined', left: 'Left', right: 'Right' };
  const contrastLabel = { balanced: 'Balanced', detail: 'Reveal faint detail', strong: 'Emphasise strong signals' } as const;
  const viewKey = 'tagnetic.audio-repair-view.v1';
  const defaultView: ViewOptions = { treated: false, channel: 'combined', waveform: false, contrast: 'balanced' };
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
  let plot: HTMLElement | undefined;
  let interaction: Gesture | undefined;
  let viewFrame = 0;
  let gestureFrame = 0;
  let panTime = 0;
  let lastPanProbe = 0;
  let viewportTimer: ReturnType<typeof setTimeout> | undefined;
  let requested: (ProbeView & { time: FrameRange | null }) | undefined;
  let viewedSource: string | undefined;
  let previousAutomatic: RepairViewport | undefined;
  let probeHistory: TreatmentProbe[] = [];
  let textureCache = new WeakMap<TreatmentProbe, RepairTexture>();
  let textureStyle = '';

  const rate = $derived(snapshot?.sampleRate || 44100);
  const frames = $derived(snapshot?.frames || probe?.spectrogramEnd || Math.round(rate * 3.5));
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
  const bounds = $derived(plotBounds(width, height));
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
  const channel = $derived(stereoDisplay ? view.channel : 'combined');
  const waveform = $derived(snapshot?.overview);
  const stereoWaveform = $derived(snapshot?.stereoOverview);
  const cuts = $derived(snapshot?.cuts ?? []);
  const loopRange = $derived(snapshot?.loopRange);
  const drawnRepairs = $derived(attached.filter(({ repair }) => channel === 'combined' || !repair.channel || repair.channel === channel));
  const updatingView = $derived(!!probe && !matchesView(probe, { treated: view.treated, channel }));
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
        waveform: saved?.waveform === true,
        channel: saved?.channel === 'left' || saved?.channel === 'right' ? saved.channel : 'combined',
        contrast: saved?.contrast === 'detail' || saved?.contrast === 'strong' ? saved.contrast : 'balanced',
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
      && (result.spectrogramChannel ?? 'combined') === requestedView.channel;
  }

  function recordingWaveform(mode: RepairChannel): RepairWaveform | undefined {
    const values = mode !== 'combined' && stereoWaveform?.length ? stereoWaveform : waveform;
    if (!values?.length) return;
    return { values, time: { start: 0, end: frames }, stereo: values === stereoWaveform, channel: mode };
  }

  function paintOverview(canvas: HTMLCanvasElement) {
    void appearance;
    const overview = recordingWaveform('combined');
    if (!overview) return;
    const ink = getComputedStyle(canvas).getPropertyValue('--muted').trim() || '#aeb7d0';
    drawRepairOverview(canvas, overviewWidth, overviewHeight, Math.min(3, devicePixelRatio.current || 1), overview, ink);
  }

  function paint(canvas: HTMLCanvasElement) {
    void appearance;
    const scale = Math.min(3, devicePixelRatio.current || 1);
    const style = getComputedStyle(canvas);
    const accent = style.getPropertyValue('--accent').trim() || '#5c88e1';
    const styleKey = accent + ':' + view.contrast + ':' + rate;
    if (styleKey !== textureStyle) {
      textureStyle = styleKey;
      textureCache = new WeakMap();
    }
    const image = probe;
    if (image && measuredTime && image !== probeHistory.at(-1)) {
      const sameTile = (tile: TreatmentProbe) => tile.spectrogramStart === image.spectrogramStart
        && tile.spectrogramEnd === image.spectrogramEnd
        && matchesView(tile, { treated: image.spectrogramTreated ?? false, channel: image.spectrogramChannel ?? 'combined' });
      probeHistory = [...probeHistory.filter(tile => !sameTile(tile)).slice(-5), image];
    }
    const wanted = { treated: view.treated, channel };
    const lastImage = probeHistory.at(-1);
    const currentImage = probeHistory.filter(tile => matchesView(tile, wanted)).at(-1);
    const fallback = lastImage && { treated: lastImage.spectrogramTreated ?? false, channel: lastImage.spectrogramChannel ?? 'combined' };
    const paintedView = currentImage ? wanted : fallback;
    const textures = !paintedView ? [] : probeHistory.flatMap(tile => {
      if (!matchesView(tile, paintedView)) return [];
      let texture = textureCache.get(tile);
      if (!texture) {
        texture = buildRepairTexture(tile, rate, accent, view.contrast) ?? undefined;
        if (texture) textureCache.set(tile, texture);
      }
      return texture ? [texture] : [];
    });
    const waveformImage = currentImage ?? lastImage;
    const waveformTime = spectrogramRange(waveformImage);
    const line = !view.waveform ? undefined : waveformImage?.spectrogramWaveform?.length && waveformTime
      ? { values: waveformImage.spectrogramWaveform, time: waveformTime } : recordingWaveform(paintedView?.channel ?? channel);
    drawRepairViewport(canvas, width, height, visible, textures, rate, {
      ink: style.getPropertyValue('--muted').trim() || '#aeb7d0',
      grid: style.getPropertyValue('--border').trim() || '#3d435b',
      guide: style.getPropertyValue('--accent-ink').trim() || accent,
      font: style.getPropertyValue('--font-literal').trim() || 'monospace',
    }, guides, scale, line);
  }

  function cancelView() {
    if (viewFrame) cancelAnimationFrame(viewFrame);
    viewFrame = 0;
  }

  function animateViewport(target: RepairViewport, duration: number) {
    const from = visible;
    cancelView();
    const distance = Math.abs(target.time.start - from.time.start);
    const span = target.time.end - target.time.start;
    const timeChanged = from.time.start !== target.time.start || from.time.end !== target.time.end;
    const frequencyChanged = from.frequency.lowHz !== target.frequency.lowHz || from.frequency.highHz !== target.frequency.highHz;
    if ((!timeChanged && !frequencyChanged) || !duration || distance > span * 1.3 || matchMedia('(prefers-reduced-motion: reduce)').matches) {
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
        time: { start: mix(from.time.start, target.time.start), end: mix(from.time.end, target.time.end) },
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

  function requestViewport(time: FrameRange | null, mode: ProbeView = { treated: view.treated, channel }) {
    clearTimeout(viewportTimer);
    viewportTimer = undefined;
    const range = time ? { start: Math.round(time.start), end: Math.round(time.end) } : null;
    if (requested && requested.time?.start === range?.start && requested.time?.end === range?.end
      && requested.treated === mode.treated && requested.channel === mode.channel) return;
    requested = { time: range, ...mode };
    onviewport(range, mode);
  }

  function reflectView() {
    const id = sourceId;
    const mode = { treated: view.treated, channel };
    if (!id) return;
    untrack(() => {
      if (viewedSource !== id) {
        viewedSource = id;
        const range = snapshot?.loopRange;
        if (range && range.end > range.start) {
          const duration = range.end - range.start;
          const size = Math.min(rate * 12, Math.max(rate * 3.5, duration * 1.15));
          const frame = snapshot?.frame ?? range.start;
          const center = duration <= rate * 12 ? (range.start + range.end) / 2
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

  function reset() {
    const middle = snapshot?.frame ?? (visible.time.start + visible.time.end) / 2;
    const target = { time: boundedTime(middle - rate * 1.75, rate * 3.5, rate, frames), frequency: fullFrequency };
    setManual(target, 160);
    requestViewport(target.time);
  }

  function clear() {
    selection = undefined;
    precise = false;
    inputError = undefined;
    cursor = 'crosshair';
  }

  function fitSelection() {
    if (!selection) return;
    const { startFrame, endFrame, lowHz, highHz } = selection;
    const span = (endFrame - startFrame) * 1.2;
    const middle = (startFrame + endFrame) / 2;
    const size = clampRepair(span, Math.min(frames, rate / 4), Math.min(frames, rate * 12));
    const time = boundedTime(middle - size / 2, size, rate, frames);
    const low = Math.log(Math.max(lowHz, fullFrequency.lowHz));
    const high = Math.log(Math.max(highHz, lowHz + 1));
    const padding = Math.max(.1, (high - low) * .15);
    const frequency = {
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
    onaction('audition-band', { start: startFrame, end: endFrame }, { lowHz, highHz, channel });
  }

  function jumpTo(value: number) {
    const frame = Math.round(clampRepair(value, 0, frames - 1));
    const span = visible.time.end - visible.time.start;
    if (manual) {
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
    jumpTo((event.clientX - rect.left) * frames / Math.max(1, rect.width));
  }

  function overviewKeys(event: KeyboardEvent) {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const current = snapshot?.frame ?? 0;
    const step = event.shiftKey ? 10 * rate : rate;
    const jump: Record<string, number> = { ArrowLeft: current - step, ArrowRight: current + step, Home: 0, End: frames - 1 };
    if (!(event.key in jump)) return;
    event.preventDefault();
    jumpTo(jump[event.key]);
  }

  function zoomAt(event: WheelEvent) {
    if (!viewportReady || interaction || event.deltaY === 0) return;
    event.preventDefault();
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const speed = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? height : 1;
    const factor = Math.exp(clampRepair(event.deltaY * speed * .0025, -.7, .7));
    const current = manual ?? visible;
    if (event.shiftKey) {
      const half = Math.log(fullFrequency.highHz / fullFrequency.lowHz);
      const logMin = Math.log(fullFrequency.lowHz);
      const ratio = clampRepair((bounds.bottom - (event.clientY - rect.top)) / (bounds.bottom - bounds.top), 0, 1);
      const span = Math.log(current.frequency.highHz / current.frequency.lowHz);
      const anchor = Math.log(current.frequency.lowHz) + ratio * span;
      const size = clampRepair(span * factor, .10, half);
      const low = clampRepair(anchor - size * ratio, logMin, logMin + half - size);
      setManual({ ...current, frequency: { lowHz: Math.exp(low), highHz: Math.exp(low + size) } }, 150);
    } else {
      const ratio = clampRepair((event.clientX - rect.left - bounds.left) / (bounds.right - bounds.left), 0, 1);
      const span = current.time.end - current.time.start;
      const size = clampRepair(span * factor, Math.min(frames, rate / 4), Math.min(frames, rate * 12));
      const anchor = current.time.start + ratio * span;
      setManual({ ...current, time: boundedTime(anchor - size * ratio, size, rate, frames) }, 160);
    }
    scheduleViewport();
  }

  function wheelBinding(node: HTMLElement) {
    node.addEventListener('wheel', zoomAt, { passive: false });
    return () => node.removeEventListener('wheel', zoomAt);
  }

  function rectangle(pick: Pick, viewport = visible) {
    const { time, frequency } = viewport;
    return {
      left: frameX(pick.startFrame, bounds, time),
      right: frameX(pick.endFrame, bounds, time),
      top: frequencyY(pick.highHz, bounds, frequency),
      bottom: frequencyY(pick.lowHz, bounds, frequency),
    };
  }

  function hit(x: number, y: number) {
    if (selection) {
      const handles = repairDragAt(rectangle(selection), bounds, x, y);
      if (handles) return { pick: selection, handles, amount: amount ?? defaultAmount };
    }
    for (let position = drawnRepairs.length - 1; position >= 0; position--) {
      const { repair, index } = drawnRepairs[position];
      const handles = repairDragAt(rectangle(repair), bounds, x, y);
      if (handles) return { pick: { ...repair, index, saved: repair }, handles, amount: repair.amountDb };
    }
  }

  function begin(event: PointerEvent) {
    if (!viewportReady || interaction || (event.button !== 0 && event.button !== 1)) return;
    const node = event.currentTarget as HTMLButtonElement;
    const rect = node.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    if (x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom) return;
    const origin = visible;
    const baseline = event.button === 0 ? hit(x, y) : undefined;
    interaction = {
      pointer: event.pointerId, node, rect, firstX: x, firstY: y,
      firstFrame: frameAtX(x, bounds, origin.time), firstHz: frequencyAtY(y, bounds, origin.frequency), x, y,
      mode: event.button === 1 ? 'pan' : baseline ? 'edit' : 'pick', moved: false, origin, previous: manual, baseline,
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
    gesture.y = event.clientY - gesture.rect.top;
    gesture.moved ||= Math.hypot(gesture.x - gesture.firstX, gesture.y - gesture.firstY) >= 3;
  }

  function paintGesture(gesture: Gesture) {
    if (!gesture.moved) return;
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
      draft = { ...initial.pick, ...editRepairBounds(initial.pick, initial.handles, frame - gesture.firstFrame, Math.log(hz / gesture.firstHz), frames, fullFrequency.highHz) };
      return;
    }
    const first = Math.round(gesture.firstFrame);
    const end = Math.round(frame);
    const lowHz = Math.round(Math.min(gesture.firstHz, hz));
    const highHz = Math.round(Math.max(gesture.firstHz, hz));
    draft = { startFrame: Math.min(first, end), endFrame: Math.max(first, end), lowHz, highHz, channel: channel === 'combined' ? undefined : channel };
  }

  function slideFrequency(range: FrequencyRange, logOffset: number) {
    const ratio = range.highHz / range.lowHz;
    const lowHz = clampRepair(range.lowHz * Math.exp(logOffset), fullFrequency.lowHz, fullFrequency.highHz / ratio);
    return { lowHz, highHz: Math.min(fullFrequency.highHz, lowHz * ratio) };
  }

  function edgePan(gesture: Gesture, now: number) {
    if (gesture.mode === 'pan') return false;
    const panX = edgePanSpeed(gesture.x, bounds.left, bounds.right);
    const panY = edgePanSpeed(gesture.y, bounds.top, bounds.bottom);
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
      cursor = repairDragCursor(hit(event.offsetX, event.offsetY)?.handles);
      return;
    }
    if (gesture.pointer !== event.pointerId) return;
    updatePoint(gesture, event);
    if (gesture.moved) queueGesture(gesture);
  }

  function chooseExisting(pick: SpectralRepair, index: number, center = false) {
    selection = { ...pick, index, saved: pick };
    amount = pick.amountDb;
    inputError = undefined;
    if (center) {
      const span = Math.min(rate * 12, Math.max(rate * 3.5, 1.2 * (pick.endFrame - pick.startFrame)));
      const centerFrame = (pick.startFrame + pick.endFrame) / 2;
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
        amount = gesture.baseline?.amount ?? defaultAmount;
      } else if (gesture.baseline && !gesture.moved) {
        selection = gesture.baseline.pick;
        amount = gesture.baseline.amount;
      } else if (!gesture.moved) {
        shown = gesture.origin;
        manual = gesture.previous;
        if (gesture.mode === 'pick') clear();
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
    if (event.key.toLowerCase() === 'f' && selection) {
      event.preventDefault();
      fitSelection();
      return;
    }
    if (event.key === '0') {
      event.preventDefault();
      reset();
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
    if (!manual) {
      setManual(visible);
      scheduleViewport();
    }
    const current = selection ?? {
      startFrame: Math.round(visible.time.start), endFrame: Math.round(visible.time.end),
      lowHz: Math.round(visible.frequency.lowHz), highHz: Math.round(visible.frequency.highHz),
      channel: channel === 'combined' ? undefined : channel,
    };
    const next = { ...current };
    const nudge = Math.max(1, Math.round(rate * .1));
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
        const step = event.key === 'ArrowUp' ? 1.06 : 1 / 1.06;
        if (event.shiftKey) next.highHz = clampRepair(Math.round(next.highHz * step), next.lowHz + 20, fullFrequency.highHz);
        else next.lowHz = clampRepair(Math.round(next.lowHz * step), 20, next.highHz - 20);
        break;
      }
    }
    selection = next;
  }

  function timeLimits(edge: TimeEdge, pick: Pick) {
    return edge === 'startFrame' ? [0, pick.endFrame - 1] : [pick.startFrame + 1, frames];
  }

  function commitTime(input: HTMLInputElement, edge: TimeEdge) {
    if (!selection) return false;
    const value = frameAtTime(input.value, rate);
    const [min, max] = timeLimits(edge, selection);
    if (value === null || value < min || value > max) {
      inputError = { edge, message: 'Use a time between ' + timeLabel(min, rate) + ' and ' + timeLabel(max, rate) + '.' };
      return false;
    }
    selection = { ...selection, [edge]: value };
    input.value = timeLabel(value, rate, input === document.activeElement ? 6 : 3);
    if (inputError?.edge === edge) inputError = undefined;
    return true;
  }

  function exactTimeKeys(event: KeyboardEvent, edge: TimeEdge) {
    if (!selection) return;
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === 'Escape') {
      event.preventDefault();
      input.value = timeLabel(selection[edge], rate, 6);
      inputError = undefined;
      focusPlot();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      if (commitTime(input, edge) && (event.ctrlKey || event.metaKey)) apply();
    } else if ((event.key === 'ArrowUp' || event.key === 'ArrowDown') && !event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      const [min, max] = timeLimits(edge, selection);
      const nudge = event.shiftKey ? 1 : Math.max(1, Math.round(rate / 100));
      const base = frameAtTime(input.value, rate) ?? selection[edge];
      const next = clampRepair(base + nudge * (event.key === 'ArrowUp' ? 1 : -1), min, max);
      input.value = timeLabel(next, rate, 6);
      commitTime(input, edge);
    }
  }

  function editFrequency(edge: FrequencyEdge, value: number | null) {
    if (selection && value !== null) selection = { ...selection, [edge]: Math.round(value) };
  }

  function fieldShortcuts(event: KeyboardEvent) {
    if (event.defaultPrevented || event.key !== 'Enter' || (!event.ctrlKey && !event.metaKey)) return;
    const target = event.target;
    if (!(target instanceof HTMLInputElement) || !target.closest('.repair-layout')) return;
    event.preventDefault();
    target.blur();
    apply();
  }

  function apply() {
    if (!selection || !validPick || interaction || inputError) return;
    const index = active && selection.index !== undefined ? selection.index : nextSlot;
    if (index >= editLimit) return;
    const updated = [...repairs];
    const { startFrame, endFrame, lowHz, highHz, channel: target } = selection;
    updated[index] = { startFrame, endFrame, lowHz, highHz, amountDb: amount ?? defaultAmount, ...(target && stereo ? { channel: target } : {}) };
    onedit({ spectralRepairs: updated });
    selection = { ...selection, index, saved: updated[index] };
  }

  function remove() {
    const index = selection?.index;
    if (index === undefined || !active) return;
    const updated = repairs.map((value, slot) => slot === index ? null : value);
    onedit({ spectralRepairs: updated.some(Boolean) ? updated : null });
    clear();
    focusPlot();
  }

  function boxStyle(pick: Pick) {
    const { time, frequency } = visible;
    if (pick.endFrame <= time.start || pick.startFrame >= time.end
      || pick.highHz <= frequency.lowHz || pick.lowHz >= frequency.highHz) return '';
    const left = frameX(Math.max(pick.startFrame, time.start), bounds, time);
    const top = frequencyY(Math.min(pick.highHz, frequency.highHz), bounds, frequency);
    const right = frameX(Math.min(pick.endFrame, time.end), bounds, time);
    const bottom = frequencyY(Math.max(pick.lowHz, frequency.lowHz), bounds, frequency);
    return 'left:' + left + 'px;top:' + top + 'px;width:' + Math.max(1, right - left) + 'px;height:' + Math.max(1, bottom - top) + 'px';
  }

  function editLabel(pick: Pick) {
    return repairTimeLabel(pick.startFrame, rate, true) + '–' + repairTimeLabel(pick.endFrame, rate, true)
      + ' · ' + pick.lowHz + '–' + pick.highHz + ' Hz' + (pick.channel ? ' · ' + channelLabel[pick.channel] : '');
  }

  onDestroy(() => {
    if (interaction) finish(undefined, false);
    cancelView();
    clearTimeout(viewportTimer);
  });
</script>

<svelte:window onblur={() => finish()} onkeydown={fieldShortcuts}/>

<div class="repair-layout">
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
      <span class="overview-cut" aria-hidden="true" style:left={String(frame * 100 / frames) + '%'}></span>
    {/each}
    {#if snapshot}
      <span class="overview-playhead" aria-hidden="true" style:left={String(snapshot.frame * 100 / frames) + '%'}></span>
    {/if}
    <PlotButton class="overview-hit" type="button" aria-label="Jump in recording overview"
      title="Click to jump. Arrows move by one second, Shift and arrows by ten seconds."
      disabled={!viewportReady} onclick={overviewClick} onkeydown={overviewKeys}/>
  </div>

  <div class={['repair-plot', { panning: moving === 'pan' }]} bind:clientWidth={width} bind:clientHeight={height}
    aria-label={(view.treated ? 'Treated' : 'Source') + ' spectrogram'} role="group"
    style:--repair-cursor={cursor} {@attach markPlot} {@attach wheelBinding} {@attach reflectView} {@attach followBinding}>
    <span id={plotHelpID} class="sr-only">Scroll to zoom in time or Shift-scroll for frequency. Middle-drag to pan.
      Left and Right adjust the start time. Up and Down adjust the low frequency.
      Hold Shift to adjust the other edge. Enter auditions the time range. Shift+Enter auditions the band.
      Ctrl or Command+Enter applies the selection. F fits the selection. Zero resets the view.
      Escape cancels a drag or clears the selection. Delete removes a saved repair.</span>
    <span class="sr-only" role="status">{selection ? editLabel(selection) : ''}</span>
    {#if updatingView}<span class="view-status" role="status">Updating spectrogram…</span>{/if}
    <canvas aria-hidden="true" {@attach paint}></canvas>
    <PlotButton class="repair-hit" type="button" disabled={!viewportReady}
      aria-label="Select time and frequency" aria-describedby={plotHelpID}
      onpointerdown={begin} onpointermove={move} onpointerup={end} onpointercancel={end} onlostpointercapture={end}
      onauxclick={event => event.preventDefault()} onkeydown={keys}/>
    {#if snapshot && snapshot.frame >= visible.time.start && snapshot.frame <= visible.time.end}
      <span class="playhead" aria-hidden="true" style:left={frameX(snapshot.frame, bounds, visible.time) + 'px'}
        style:top={bounds.top + 'px'} style:bottom={height - bounds.bottom + 'px'}></span>
    {/if}
    {#each drawnRepairs as { repair, index } (index)}
      {@const position = boxStyle(repair)}
      {#if position}<span class="applied" aria-hidden="true" style={position}></span>{/if}
    {/each}
    {#if selected}
      {@const position = boxStyle(selected)}
      {@const rect = rectangle(selected)}
      {#if position}
        <span class="picked" aria-hidden="true" style={position}></span>
        {#if !moving}
          {#each [[rect.left, rect.top], [rect.right, rect.top], [rect.left, rect.bottom], [rect.right, rect.bottom]] as [x, y], index (index)}
            {#if x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom}
              <span class="repair-handle" aria-hidden="true" style:left={x + 'px'} style:top={y + 'px'}></span>
            {/if}
          {/each}
        {/if}
      {/if}
    {/if}
  </div>

  <div class="repair-rail">{@render modeRail()}</div>

  <div class="repair-selection">
    <div class="repair-summary">
      <span class="range-text">{selection ? editLabel(selection) : 'Drag across the spectrogram to select an area.'}</span>
      <div class="view-controls" aria-label="Spectrogram controls" role="group">
        <Menu controller={viewMenu}>
          <MenuTrigger class="repair-menu" aria-label="Spectrogram display" aria-description={(view.treated ? 'Treated' : 'Source') + ', ' + channelLabel[channel]}
            title={'Spectrogram display · ' + (view.treated ? 'Treated' : 'Source') + ', ' + channelLabel[channel]} disabled={!viewportReady}>
            <SlidersHorizontal size={15} aria-hidden="true"/>
          </MenuTrigger>
          <MenuContent class="app-select-menu audio-tool-popup" align="end" sideOffset={6} collisionPadding={8}>
            <MenuRadioGroup value={view.treated ? 'treated' : 'source'} onValueChange={value => changeView({ treated: value === 'treated' })}>
              <MenuGroupLabel>Spectrogram</MenuGroupLabel>
              <MenuRadioItem data-slot="select-item" value="source" closeOnSelect>
                <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>Source
              </MenuRadioItem>
              <MenuRadioItem data-slot="select-item" value="treated" closeOnSelect>
                <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>Treated
              </MenuRadioItem>
            </MenuRadioGroup>
            {#if stereoDisplay}
              <MenuSeparator/>
              <MenuRadioGroup value={channel} onValueChange={value => changeView({ channel: value as RepairChannel })}>
                <MenuGroupLabel>Display channel</MenuGroupLabel>
                {#each channels as mode (mode)}
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
            <MenuCheckboxItem data-slot="select-item" value="waveform" checked={view.waveform}
              onCheckedChange={checked => changeView({ waveform: checked === true })} closeOnSelect>
              <MenuItemIndicator><Check size={13} aria-hidden="true"/></MenuItemIndicator>Waveform overlay
            </MenuCheckboxItem>
          </MenuContent>
        </Menu>
        <Button variant="ghost" size="icon-sm" aria-label="Follow playhead" title="Follow playhead" aria-pressed={!manual} disabled={!viewportReady} onclick={toggleFollow}>
          <LocateFixed size={15} aria-hidden="true"/>
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Fit selected area" aria-keyshortcuts="F" title="Fit selected area (F)" disabled={!selection} onclick={fitSelection}>
          <Crosshair size={15} aria-hidden="true"/>
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Reset spectrogram zoom" aria-keyshortcuts="0" title="Reset spectrogram zoom (0)" disabled={!viewportReady} onclick={reset}>
          <RotateCcw size={15} aria-hidden="true"/>
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Clear selected area" aria-keyshortcuts="Escape" title="Clear selected area (Esc)"
          disabled={!selection} onclick={() => { clear(); focusPlot(); }}><X size={15} aria-hidden="true"/></Button>
        <Menu controller={editMenu}>
          <MenuTrigger class="repair-menu" aria-label="Applied repairs" title="Applied repairs" disabled={!attached.length}>
            <Layers size={15} aria-hidden="true"/>
          </MenuTrigger>
          <MenuContent class="app-select-menu audio-tool-popup" align="end" sideOffset={6} collisionPadding={8}>
            {#each attached as { repair, index } (index)}
              {@const label = editLabel(repair)}
              <MenuItem data-slot="select-item" {label} onSelect={() => chooseExisting(repair, index, true)}>{label} · −{repair.amountDb} dB</MenuItem>
            {/each}
          </MenuContent>
        </Menu>
      </div>
    </div>
    {#if selection}
      <div class="selection-commands" aria-label="Selected frequency area" role="group">
        <Button variant="ghost" size="sm" aria-label="Audition selected area" title="Audition time range (Enter)" aria-keyshortcuts="Enter" onclick={audition}>
          <Play size={13} aria-hidden="true"/>Audition
        </Button>
        <Button variant="ghost" size="sm" aria-label="Audition selected frequencies only" title="Audition only this band (Shift+Enter)"
          aria-keyshortcuts="Shift+Enter" aria-pressed={snapshot?.monitor === 'band'} disabled={!validPick} onclick={auditionBand}>
          <Headphones size={14} aria-hidden="true"/>Band only
        </Button>
        {#if stereo}
          <Menu controller={channelMenu}>
            <MenuTrigger class="repair-channel" aria-label="Apply repair to channel" title="Choose which channels the repair affects">{selection.channel ? channelLabel[selection.channel] : 'Both'}</MenuTrigger>
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
        <NumberInput class="repair-number" bind:value={amount} min={1} max={36} step={1}>
          <NumberInputControl class="repair-amount" aria-label="Attenuation in dB" title="Attenuation in dB" placeholder={String(defaultAmount)}/>
          <span aria-hidden="true">dB</span>
        </NumberInput>
        <Button variant="ghost" size="icon-sm" aria-label="Exact selection boundaries" title="Exact selection boundaries" aria-expanded={precise}
          onclick={() => precise = !precise}><SlidersHorizontal size={14} aria-hidden="true"/></Button>
        <Button size="sm" disabled={!validPick || (!active && nextSlot >= editLimit) || !!inputError}
          title={!validPick ? 'Select at least 20 Hz' : !active && nextSlot >= editLimit ? 'Eight repairs are allowed' : 'Apply frequency attenuation (Ctrl/⌘+Enter)'}
          aria-keyshortcuts="Control+Enter Meta+Enter" onclick={apply}>
          <Check size={14} aria-hidden="true"/>{active ? 'Update' : 'Apply'}
        </Button>
        {#if active}
          <Button variant="ghost" size="icon-sm" aria-label="Remove repair" title="Remove repair (Delete)" aria-keyshortcuts="Delete"
            onclick={remove}><Trash2 size={14} aria-hidden="true"/></Button>
        {/if}
      </div>
    {/if}
  </div>

  {#if precise && selection}
    <div class="precision-panel" role="group" aria-label="Exact selection boundaries">
      {#each ['startFrame', 'endFrame'] as edge (edge)}
        {@const key = edge as TimeEdge}
        <label>
          <span>{key === 'startFrame' ? 'Start time' : 'End time'}</span>
          <Input class="repair-time-input" value={timeLabel(selection[key], rate)}
            aria-invalid={inputError?.edge === key} aria-describedby={inputError?.edge === key ? plotHelpID + '-error' : undefined}
            onfocus={event => {
              if (!selection || inputError?.edge === key) return;
              event.currentTarget.value = timeLabel(selection[key], rate, 6);
              event.currentTarget.select();
            }}
            onblur={event => commitTime(event.currentTarget, key)}
            onkeydown={event => exactTimeKeys(event, key)}/>
        </label>
      {/each}
      {#each ['lowHz', 'highHz'] as edge (edge)}
        {@const key = edge as FrequencyEdge}
        {@const name = key === 'lowHz' ? 'Low frequency' : 'High frequency'}
        <label>
          <span>{name}</span>
          <NumberInput class="repair-number" value={selection[key]} min={key === 'lowHz' ? 20 : Math.min(fullFrequency.highHz, selection.lowHz + 20)}
            max={key === 'lowHz' ? Math.max(20, selection.highHz - 20) : fullFrequency.highHz} step={1} largeStep={10}
            onValueCommit={value => editFrequency(key, value)}>
            <NumberInputControl class="repair-hertz-input" aria-label={name + ' in Hz'}/>
            <span aria-hidden="true">Hz</span>
          </NumberInput>
        </label>
      {/each}
      {#if inputError}<p class="boundary-error" id={plotHelpID + '-error'} role="status">{inputError.message}</p>{/if}
    </div>
  {/if}
</div>

<style>
  .repair-layout {
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-rows: auto minmax(218px, 1fr) auto;
    gap: var(--sp-2, 8px);
  }
  .recording-overview, .repair-plot {
    position: relative;
    isolation: isolate;
    overflow: hidden;
    overflow: clip;
  }
  .recording-overview { grid-column: 1 / -1; min-height: 28px; border-radius: var(--r-compact, 8px); }
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
  .overview-window, .overview-loop, .overview-cut, .overview-playhead {
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
  .overview-cut { width: 1px; background: var(--muted); opacity: .6; }
  .overview-playhead { width: 2px; background: var(--accent-ink, var(--text)); }
  .repair-plot { min-height: 218px; border-radius: var(--r-inset, 14px); }
  .repair-plot :global(.repair-hit) { cursor: var(--repair-cursor, crosshair); }
  .panning :global(.repair-hit) { cursor: grabbing; }
  .playhead, .applied, .picked, .repair-handle, .view-status { position: absolute; pointer-events: none; }
  .playhead { width: 1px; background: var(--accent-ink, var(--text)); opacity: .84; }
  .picked {
    border: 1px solid var(--accent-ink, var(--text));
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
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
  .selection-commands {
    min-width: 0;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-1, 4px);
  }
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
    grid-template-columns: repeat(auto-fit, minmax(94px, 1fr));
    gap: var(--sp-2, 8px);
  }
  .precision-panel label { display: grid; gap: var(--sp-1, 4px); min-width: 0; color: var(--muted); font-size: var(--text-xs); }
  .boundary-error { grid-column: 1 / -1; margin: 0; color: var(--danger, var(--destructive)); font-size: var(--text-xs); }
  .repair-rail { align-self:start; }
  .view-controls { display:flex; align-items:center; flex-wrap:wrap; gap:var(--sp-1); margin-inline-start:auto; }
  .view-controls :global([aria-pressed=true]), .selection-commands :global([aria-pressed=true]) { color:var(--accent-ink); }
</style>
