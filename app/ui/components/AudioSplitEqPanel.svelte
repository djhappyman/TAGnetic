<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import { prefersReducedMotion, Tween } from 'svelte/motion';
  import { cubicOut } from 'svelte/easing';
  import { fade } from 'svelte/transition';
  import { devicePixelRatio } from 'svelte/reactivity/window';
  import { Activity, AudioLines, Check, ChevronRight, Headphones, Power, SlidersHorizontal, Snowflake, WandSparkles } from '@lucide/svelte';
  import { Button as GraphButton } from '@ui-sdk/svelte/button';
  import type { ChangeDetails } from '@ui-sdk/core';
  import { createMenuController, Menu, MenuCheckboxItem, MenuContent, MenuItem, MenuItemIndicator, MenuRadioGroup, MenuRadioItem, MenuSeparator, MenuShortcut, MenuSub, MenuSubContent, MenuSubTrigger, type MenuOpenChangeReason } from '@ui-sdk/svelte/menu';
  import { createPopoverController, Popover, PopoverContent, PopoverTrigger } from '@ui-sdk/svelte/popover';
  import { createSelectController, Select, SelectContent, SelectItem, SelectTrigger } from '@ui-sdk/svelte/select';
  import Button from './AudioSplitButton.svelte';
  import EqDial from './AudioSplitEqDial.svelte';
  import { eqGainLimit, eqGainStep, eqSpectralShape, roundEqDb } from '../js/audio-eq-range.js';
  import { maxEqBands, noTreatment, treatmentNumbers, type AudioTreatment, type EqChannel } from '../js/audio-treatment';
  import {
    eqBandPatch, eqFrequencyRange, eqSoloRange, eqWidthRange, findEqNode,
    legacyEqBands, legacyEqNode, makeExtraEqNode, originalEqBand, removeEqBand, removeEqBands,
    spectrumPeakQ, visibleEqNodes, type EqNode, type EqShape,
  } from '../js/audio-eq';
  import {
    createSpectrumMotion, drawTreatmentPlot, eqChannelDomain, eqResponseChannels, frequencyX, gainY, moveEqView, plotBounds, spectrumTraceNear, spectrumY, xFrequency, yGain,
    type AnalyzerView, type EqChannelDomain, type EqView, type PlotBounds, type TreatmentProbe,
  } from '../js/audio-treatment-plot';
  import type { FrameRange } from '../js/audio-split';
  import type { PreviewBand, ToolAction, ToolSnapshot } from '../js/audio-tools-window';

  let { snapshot, probe, mode, onmode, onedit, onaction, onanalyzer, onlearn }: {
    snapshot: ToolSnapshot | null;
    probe: TreatmentProbe | null;
    mode: string | null;
    onmode: (mode: string | null) => void;
    onanalyzer: (view: AnalyzerView) => void;
    onlearn: (index: number) => void;
    onedit: (patch: Partial<AudioTreatment>, gesture?: string) => void;
    onaction: (action: ToolAction, range?: FrameRange, band?: PreviewBand) => void;
  } = $props();

  type Drag = {
    pointerId: number;
    control: HTMLButtonElement;
    sourceId: string;
    node: EqNode;
    members: EqNode[];
    previousKeys: string[];
    previousMode: string | null;
    before: Partial<AudioTreatment>;
    gesture: string;
    bounds: PlotBounds;
    x: number;
    y: number;
    lastX: number;
    lastY: number;
    targetX: number;
    targetY: number;
    kind: 'node' | 'solo' | 'dynamic';
    moved: boolean;
    applied: boolean;
    axis?: 'x' | 'y';
  };
  type Point = { x: number; y: number };
  type SelectionBox = { left: number; top: number; right: number; bottom: number };
  type CapturedPointer = { pointer: number; control: HTMLButtonElement; x: number; y: number };
  type RulerDrag = CapturedPointer & { view: EqView; anchor: number; width: number };
  type SpectrumPeak = { hz: number; db: number; x: number; y: number; q: number };
  type PeakDrag = CapturedPointer & {
    sourceId: string;
    node: EqNode;
    before: Partial<AudioTreatment>;
    gesture: string;
    gain: number;
    applied: boolean;
  };

  const id = $props.id();
  const gainControl = createPopoverController();
  const grabMotion = new Tween(1, { duration: 180, easing: cubicOut });
  const inputSpec = treatmentNumbers.eqInputDb;
  const bandMenu = createMenuController();
  const outputSpec = treatmentNumbers.eqOutputDb;
  const strengthSpec = treatmentNumbers.eqStrength;
  const spectrum = createSpectrumMotion();
  const shapes = Object.keys(legacyEqBands) as EqShape[];
  const channels = { stereo: 'Stereo', left: 'Left only', right: 'Right only', mid: 'Centre', side: 'Sides' } satisfies Record<EqChannel, string>;
  const curveLabels = { stereo: 'Stereo', left: 'Left', right: 'Right', mid: 'Centre', side: 'Sides' } satisfies Record<EqChannel, string>;
  const channelDomains = { 'left-right': 'Left / Right', 'centre-sides': 'Centre / Sides' };
  const analyzerViews = { before: 'Before', after: 'After', both: 'Both', off: 'Off' };
  const meterTicks = [0, -12, -24, -36, -48, -60];
  const meterFloor = -60;
  const editorGap = 8;
  const neutralClickRadius = 4;
  const addClickRadius = 26;
  const nodeHitRadius = 19;
  const gainRanges = [6, 12, 24] as const;
  let root: HTMLElement;
  let width = $state(0);
  let height = $state(0);
  let ghost = $state.raw<EqNode>();
  let addOrigin: Point | undefined;
  let marquee: { pointer: number; control: HTMLButtonElement; start: Point; additive: boolean } | undefined;
  let box = $state.raw<SelectionBox>();
  let group = $state.raw<{ sourceId: string; primary: string; keys: string[] }>();
  let analyzer = $state<AnalyzerView>('both');
  let frozenReadings = $state.raw<{ sourceId: string; probe: TreatmentProbe; automatic?: boolean }>();
  let plottedSpectrum: TreatmentProbe | null = null;
  let grabHover: { x: number; y: number; probe: TreatmentProbe; timer: ReturnType<typeof setTimeout> } | undefined;
  let soloed = $state<string | null>(null);
  let clipboard = $state.raw<EqNode[]>([]);
  let menuNodeKey = $state<string | null>(null);
  let menuPoint = $state<Point>();
  let menuReturn: HTMLElement | undefined;
  let menuEditsValue = false;
  let drag: Drag | undefined;
  let dragFrame = 0;
  let lastNodeClick: { sourceId: string; keys: string; point: Point; time: number; picked: string } | undefined;
  let heldPeak = $state.raw<{ sourceId: string; db: number }>();
  let overloadSource = $state<string>();
  let peakTimer: ReturnType<typeof setTimeout> | undefined;
  let meteredProbe: TreatmentProbe | null = null;
  let gestureNumber = 0;
  let moving = $state(false);
  let adjustingStrength = $state(false);
  let draggedKey = $state<string>();
  let dynamicsGesture = $state.raw<{ key: string; handle: boolean; amount: number }>();
  let editorWidth = $state(0);
  const analyzerControl = createSelectController();
  const gainRangeControl = createSelectController();
  const curveViewControl = createSelectController();
  let curveView = $state.raw<{ sourceId: string; band: string | null; domain: EqChannelDomain }>();
  let view = $state.raw<EqView>({ start: 0, end: 1, gainRange: 24 });
  let rulerDrag: RulerDrag | undefined;
  let rulerFrame = 0;
  let pendingView: EqView | undefined;
  let peakDrag: PeakDrag | undefined;
  let peakFrame = 0;
  let activePeak = $state<number>();
  let heldKey: string | undefined;
  let soloTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingSoloBand: PreviewBand | undefined;
  let currentSoloBand: PreviewBand | undefined;
  let report = $state('');

  const settings = $derived(snapshot?.settings ?? noTreatment);
  const strength = $derived(settings.eqStrength ?? strengthSpec.initial);
  const appearance = $derived(snapshot?.theme);
  const frozen = $derived(frozenReadings?.sourceId === snapshot?.sourceId ? frozenReadings?.probe ?? null : null);
  const grabbingSpectrum = $derived(Boolean(frozen && frozenReadings?.automatic));
  const rate = $derived(snapshot?.sampleRate || 44100);
  const bounds = $derived(plotBounds(width, height, 'eq', view));
  const nodes = $derived(visibleEqNodes(settings, mode));
  const visibleNodes = $derived(nodes.filter(node => {
    const x = position(node).x;
    return node.key === draggedKey || (x >= bounds.left - 0.1 && x <= bounds.right + 0.1);
  }));
  const selected = $derived(findEqNode(settings, mode));
  const selectedKeys = $derived(new Set(group && group.sourceId === snapshot?.sourceId && group.primary === mode
    ? group.keys.filter(key => nodes.some(node => node.key === key)) : mode ? [mode] : []));
  const boxed = $derived(box ? visibleNodes.filter(node => node.stored && insideBox(position(node), box!)) : []);
  const highlighted = $derived(box ? new Set([...(marquee?.additive ? selectedKeys : []), ...boxed.map(node => node.key)]) : selectedKeys);
  const selectedStored = $derived(nodes.filter(node => node.stored && selectedKeys.has(node.key)));
  const selectionBypassed = $derived(selectedStored.length > 0 && selectedStored.every(node => node.bypassed));
  const supportsUpward = $derived(probe?.upwardEq === true);
  const supportsAllShapes = $derived(probe?.dynamicAllShapes === true);
  const supportsRatio = $derived(probe?.dynamicRatio === true);
  const playing = $derived(!!snapshot?.playing);
  const liveTreatment = $derived(Boolean(playing && !snapshot?.original && !snapshot?.monitor));
  const dragReadout = $derived(moving && draggedKey ? findEqNode(settings, draggedKey) : undefined);
  const clipped = $derived(Boolean(overloadSource && overloadSource === snapshot?.sourceId));
  const soloFocus = $derived.by(() => {
    const node = findEqNode(settings, soloed);
    const band = node && soloBand(node);
    return band && { left: frequencyX(band.lowHz, bounds, rate), right: frequencyX(band.highHz, bounds, rate) };
  });
  const leftRightBands = $derived(!!settings.eqBands?.some(band => !band.bypassed && eqChannelDomain(band.channel) === 'left-right'));
  const centreSideBands = $derived(!!settings.eqBands?.some(band => !band.bypassed && eqChannelDomain(band.channel) === 'centre-sides'));
  const mixedDomains = $derived(leftRightBands && centreSideBands);
  const curveDomain = $derived(mixedDomains
    ? (curveView && curveView.sourceId === snapshot?.sourceId && curveView.band === mode ? curveView.domain : eqChannelDomain(selected?.channel) ?? 'left-right')
    : leftRightBands ? 'left-right' : centreSideBands ? 'centre-sides' : undefined);
  const curveChannels = $derived(eqResponseChannels(settings.eqBands ?? [], selected?.channel, curveDomain));
  const menuNode = $derived(findEqNode(settings, menuNodeKey));
  const groupMenu = $derived(selectedStored.length > 1 && (!menuNode || selectedKeys.has(menuNode.key)));
  const reference = $derived(snapshot?.reference);
  const canAdjust = $derived(!!snapshot);
  const availableSlots = $derived(canAdjust ? maxEqBands - (settings.eqBands?.length ?? 0) : 0);
  const canAdd = $derived(availableSlots > 0);
  const canPasteGroup = $derived(clipboard.length > 0 && clipboard.length <= availableSlots);
  const frozenPeaks = $derived.by(() => {
    if (!frozen || analyzer === 'off') return [];
    const levels = grabLevels(frozen);
    const count = Math.min(levels.length, frozen.frequencies.length);
    const candidates: SpectrumPeak[] = [];
    for (let start = 1; start < count - 1;) {
      let end = start;
      const db = levels[start];
      while (end + 1 < count && levels[end + 1] === db) end++;
      if (db > -80 && db > levels[start - 1] && db > levels[end + 1]) {
        const hz = Math.sqrt(frozen.frequencies[start] * frozen.frequencies[end]);
        const x = frequencyX(hz, bounds, rate);
        if (x >= bounds.left && x <= bounds.right) {
          const q = spectrumPeakQ(levels, frozen.frequencies, start, end);
          candidates.push({ hz, db, x, y: spectrumY(db, bounds), q });
        }
      }
      start = end + 1;
    }
    const peaks: typeof candidates = [];
    for (const peak of candidates.sort((a, b) => b.db - a.db)) {
      if (peaks.length === 8) break;
      if (peaks.every(other => Math.abs(other.x - peak.x) >= 25)) peaks.push(peak);
    }
    return peaks;
  });

  $effect(() => {
    if (!snapshot?.sourceId) return;
    const view = frozen ? 'off' : analyzer;
    untrack(() => onanalyzer(view));
  });

  $effect(() => {
    const opacity = grabbingSpectrum ? 0.13 : 1;
    const duration = prefersReducedMotion.current ? 0 : 180;
    untrack(() => { void grabMotion.set(opacity, { duration }); });
  });

  $effect(() => {
    if (!playing || !frozen) {
      if (frozenReadings?.automatic) untrack(resumeGrab);
    }
  });

  $effect(() => {
    const reading = probe;
    const sourceId = snapshot?.sourceId;
    const playing = snapshot?.playing;
    const treated = liveTreatment;
    if (!reading || !sourceId || !playing) return;
    untrack(() => {
      if (meteredProbe === reading) return;
      meteredProbe = reading;
      if (treated && reading.outputOverload) overloadSource = sourceId;
      const peak = reading.outputPeak;
      if (peak === undefined || (heldPeak?.sourceId === sourceId && peak < heldPeak.db)) return;
      heldPeak = { sourceId, db: peak };
      clearTimeout(peakTimer);
      peakTimer = setTimeout(() => { heldPeak = undefined; }, 900);
    });
  });

  const graphProbe = $derived.by(() => {
    const reading = analyzer === 'off' ? null : frozen ?? probe;
    const listensToDynamics = liveTreatment && settings.eqBands?.some(band => band.dynamic && !band.bypassed);
    if (!listensToDynamics || !frozen && analyzer !== 'off') return reading;
    if (!probe) return reading;
    const { dynamicEqMix, spectralResponse } = probe;
    return frozen ? { ...frozen, dynamicEqMix, spectralResponse } : probe;
  });

  function paint(canvas: HTMLCanvasElement) {
    void appearance;
    void devicePixelRatio.current;
    const chart = { width, height, settings, rate };
    const selectedBand = selected && drawableBand(selected);
    const bands = nodes.flatMap(node => {
      const band = node.stored ? drawableBand(node) : undefined;
      return band ? [{ band, color: bandColor(node), selected: highlighted.has(node.key) }] : [];
    });
    const ghostBand = ghost && !moving ? drawableBand(ghost) : undefined;
    const live = liveTreatment;
    const measured = graphProbe;
    const eqOpacity = grabMotion.current;
    const options = { eqOpacity, analyzer, selectedBand, bands, ghostBand, view, appearance, bandSelection: highlighted.size > 0, channelDomain: curveDomain };
    // Drag edits are already paced by the display. Deferring again leaves the curve a frame behind the node.
    return spectrum(measured, playing && !frozen && !prefersReducedMotion.current, reading => {
      plottedSpectrum = reading;
      const dynamicMix = live ? reading?.dynamicEqMix?.map(mix => Math.round(mix * 64) / 64) : undefined;
      const spectralResponse = live ? reading?.spectralResponse : undefined;
      drawTreatmentPlot(canvas, chart.width, chart.height, 'eq', chart.settings, chart.rate, reading, undefined, { ...options, dynamicMix, spectralResponse });
    }, !moving && !adjustingStrength && eqOpacity === grabMotion.target);
  }

  function drawableBand(node: EqNode) {
    return eqBandPatch(noTreatment, { ...node, extra: { id: 1 } }, rate).eqBands?.[0];
  }

  function bandColor(node: EqNode) {
    const index = node.extra ? shapes.length + (node.extra.id ?? node.extra.index ?? 0) : shapes.indexOf(node.shape);
    const hue = Math.round((212 + index * 137.508) % 360);
    const light = appearance?.theme === 'light';
    return `hsl(${hue}, ${light ? '71%, 39%' : '77%, 70%'})`;
  }

  function supportsGain(node: EqNode) {
    return 'gain' in legacyEqBands[node.shape];
  }

  function position(node: EqNode) {
    const [min, max] = eqFrequencyRange(node, rate);
    const hz = Math.max(min, Math.min(node.hz, max));
    const db = supportsGain(node) ? node.gainDb : 0;
    return { x: frequencyX(hz, bounds, rate), y: gainY(db, bounds) };
  }

  function movingNode(node: EqNode, x: number, y: number, geometry = bounds) {
    let gainDb = node.gainDb;
    if (supportsGain(node)) {
      const initialY = gainY(node.gainDb, geometry);
      gainDb = roundEqDb(node.gainDb + yGain(y, geometry, false) - yGain(initialY, geometry));
    }
    return { ...node, hz: xFrequency(x, geometry, rate), gainDb };
  }

  function plotCandidate(x: number, y: number): EqNode | undefined {
    const centre = gainY(0, bounds);
    const distance = Math.abs(y - centre);
    if (!canAdd || x < bounds.left || x > bounds.right || distance > addClickRadius) return;
    const hz = Math.round(xFrequency(x, bounds, rate));
    const neutral = distance <= neutralClickRadius;
    const highEdge = Math.min(12000, xFrequency(bounds.right, plotBounds(width, height), rate) * 0.8);
    const shape = hz <= 60 ? (neutral ? 'low' : 'bass') : hz >= highEdge ? (neutral ? 'high' : 'treble') : 'bell';
    const gainDb = neutral ? 0 : roundEqDb(yGain(y, bounds));
    return { ...legacyEqNode(noTreatment, shape), hz, gainDb, adaptiveQ: shape === 'bell', extra: {} };
  }

  function clickPlot(event: MouseEvent) {
    const origin = addOrigin;
    addOrigin = undefined;
    if (event.detail !== 1 || !origin || Math.hypot(event.clientX - origin.x, event.clientY - origin.y) >= 5) return;
    const node = plotCandidate(event.offsetX, event.offsetY);
    if (node && selectedKeys.size < 2) {
      ghost = undefined;
      add(node.shape, node.hz, node.gainDb);
    } else clearSelection();
  }

  function insideBox(point: Point, rectangle: SelectionBox) {
    return point.x >= rectangle.left && point.x <= rectangle.right && point.y >= rectangle.top && point.y <= rectangle.bottom;
  }

  function plotPoint(event: PointerEvent, control: HTMLButtonElement): Point {
    const rect = control.getBoundingClientRect();
    return {
      x: Math.max(bounds.left, Math.min(bounds.right, event.clientX - rect.left)),
      y: Math.max(bounds.top, Math.min(bounds.bottom, event.clientY - rect.top)),
    };
  }

  function startMarquee(event: PointerEvent) {
    if (event.button !== 0 || marquee || drag || peakDrag || rulerDrag) return;
    const control = event.currentTarget as HTMLButtonElement;
    addOrigin = { x: event.clientX, y: event.clientY };
    marquee = { pointer: event.pointerId, control, start: plotPoint(event, control), additive: event.ctrlKey || event.metaKey };
    control.setPointerCapture(event.pointerId);
  }

  function moveMarquee(event: PointerEvent) {
    const current = marquee;
    if (!current || event.pointerId !== current.pointer) return;
    const { start } = current;
    const point = plotPoint(event, current.control);
    if (!box && Math.hypot(point.x - start.x, point.y - start.y) < 5) return;
    addOrigin = undefined;
    ghost = undefined;
    box = { left: Math.min(start.x, point.x), right: Math.max(start.x, point.x),
      top: Math.min(start.y, point.y), bottom: Math.max(start.y, point.y) };
  }

  function finishMarquee(event?: PointerEvent) {
    const current = marquee;
    if (!current || (event && current.pointer !== event.pointerId)) return;
    if (event?.type === 'pointerup') {
      moveMarquee(event);
      if (box) {
        const keys = new Set([...(current.additive ? selectedKeys : []), ...boxed.map(node => node.key)]);
        selectGroup([...keys], mode && keys.has(mode) ? mode : boxed[0]?.key);
      }
    } else addOrigin = undefined;
    box = undefined;
    marquee = undefined;
    if (current.control.hasPointerCapture(current.pointer)) current.control.releasePointerCapture(current.pointer);
  }

  function focusNode(key: string) {
    root.querySelector<HTMLElement>('[data-eq-key="' + CSS.escape(key) + '"]')?.focus({ preventScroll: true });
  }

  async function exact(node: EqNode) {
    onmode(node.key);
    await tick();
    if (mode === node.key) root.querySelector<HTMLButtonElement>('.band-editor .dial-ring')?.click();
  }

  function clearSelection() {
    group = undefined;
    onmode(null);
  }

  function sendBand(node: EqNode, gesture?: string) {
    return sendBands([node], gesture);
  }

  function sendBands(changed: EqNode[], gesture?: string) {
    if (!canAdjust) return false;
    const patch: Partial<AudioTreatment> = {};
    for (const node of changed) Object.assign(patch, eqBandPatch({ ...settings, ...patch }, node, rate));
    if (!Object.keys(patch).length) return false;
    onedit(patch, gesture);
    return true;
  }

  function focusGraph() {
    root.querySelector<HTMLElement>('.plot-hit')?.focus({ preventScroll: true });
  }

  function bandsForAction(node?: EqNode) {
    return !node || selectedKeys.has(node.key) ? selectedStored : node.stored ? [node] : [];
  }

  async function removeBands(bands: EqNode[]) {
    if (!canAdjust || !bands.length) return;
    if (bands.some(node => soloed === node.key)) stopSolo();
    onedit(removeEqBands(settings, bands));
    clearSelection();
    report = bands.length === 1 ? bands[0].name + ' removed.' : bands.length + ' EQ bands removed.';
    await tick();
    focusGraph();
  }

  function toggle(node: EqNode) {
    if (node.stored) sendBand({ ...node, bypassed: !node.bypassed });
    else sendBand({ ...node, gainDb: supportsGain(node) ? 2 : node.gainDb, bypassed: false });
  }

  function toggleSelection() {
    sendBands(selectedStored.map(band => ({ ...band, bypassed: !selectionBypassed })));
  }

  function selectGroup(keys: string[], primary = keys[0]) {
    group = snapshot && primary && keys.length > 1 ? { sourceId: snapshot.sourceId, primary, keys } : undefined;
    onmode(primary ?? null);
  }

  function selectNode(node: EqNode, rotate: boolean, event?: MouseEvent) {
    if (event?.ctrlKey || event?.metaKey) {
      const keys = new Set(selectedKeys);
      if (keys.has(node.key)) keys.delete(node.key);
      else keys.add(node.key);
      selectGroup([...keys], keys.has(node.key) ? node.key : undefined);
      return;
    }
    if (event?.shiftKey && selected) {
      const first = Math.min(selected.hz, node.hz);
      const last = Math.max(selected.hz, node.hz);
      selectGroup([...new Set([...selectedKeys, ...nodes.filter(item => item.stored && item.hz >= first && item.hz <= last).map(item => item.key)])], node.key);
      return;
    }
    const current = position(node);
    const stack = nodes.filter(candidate => {
      const adjacent = position(candidate);
      return Math.hypot(current.x - adjacent.x, current.y - adjacent.y) <= 19;
    });
    const index = stack.findIndex(candidate => candidate.key === mode);
    const next = rotate && stack.length > 1 && index >= 0 ? stack[(index + 1) % stack.length] : node;
    selectGroup([next.key]);
    if (next.key !== node.key) void tick().then(() => focusNode(next.key));
  }

  function add(shape: EqShape, hz?: number, gainDb = 0) {
    if (!canAdd) return;
    const node = makeExtraEqNode(settings, shape);
    if (!sendBand({ ...node, hz: hz ?? node.hz, gainDb })) return;
    onmode(node.key);
    report = node.name + ' added.';
    void tick().then(() => focusNode(node.key));
  }

  function paste(node?: EqNode) {
    if (clipboard.length === 1 && node) replaceBand(node, clipboard[0]);
    else insertCopies(clipboard);
  }

  function insertCopies(bands: EqNode[]) {
    if (!bands.length || bands.length > availableSlots) return;
    const copies = bands.map(band => {
      const copy = makeExtraEqNode(settings, band.shape);
      return { ...band, key: copy.key, extra: copy.extra };
    });
    if (!sendBands(copies)) return;
    selectGroup(copies.map(copy => copy.key));
    void tick().then(() => focusNode(copies[0].key));
  }

  function replaceBand(node: EqNode | undefined, next: EqNode, focus = true) {
    if (!canAdjust) return;
    const target = node?.extra ? node : makeExtraEqNode(settings, next.shape);
    const removed = node && !node.extra ? removeEqBand(settings, node) : {};
    const patch = eqBandPatch({ ...settings, ...removed }, { ...next, key: target.key, extra: target.extra }, rate);
    if (!Object.keys(patch).length) return;
    onedit({ ...removed, ...patch });
    onmode(target.key);
    if (focus) void tick().then(() => focusNode(target.key));
  }

  function copy(bands: EqNode[]) {
    if (!bands.length) return;
    clipboard = bands;
    report = bands.length === 1 ? bands[0].name + ' settings copied.' : bands.length + ' EQ bands copied.';
  }

  function reset(bands: EqNode[]) {
    if (!sendBands(bands.map(node => ({
      ...node,
      hz: treatmentNumbers[legacyEqBands[node.shape].frequency].initial,
      gainDb: 0,
      q: node.shape === 'notch' ? 1500 : 80,
      slope: 12,
      bypassed: false,
    })))) return;
    report = bands.length === 1 ? bands[0].name + ' reset.' : bands.length + ' EQ bands reset.';
  }

  function changeShape(node: EqNode, shape: EqShape) {
    if (shape === node.shape) return;
    const base = legacyEqNode(noTreatment, shape);
    replaceBand(node, {
      ...node, shape, q: base.q, slope: base.slope,
      gainDb: supportsGain(base) ? node.gainDb : 0,
      adaptiveQ: shape === 'bell',
      dynamic: supportsDynamics(shape) ? node.dynamic : undefined,
    });
  }

  function changeChannel(node: EqNode, channel: EqChannel) {
    replaceBand(node, { ...node, channel }, false);
  }

  function editGain(key: 'eqStrength' | 'eqInputDb' | 'eqOutputDb', sourceId: string | undefined) {
    return (value: number, gesture?: string) => {
      if (sourceId && sourceId === snapshot?.sourceId) onedit({ [key]: value }, gesture);
    };
  }

  function setDynamic(node: EqNode, enabled: boolean, range = 4) {
    if (!supportsDynamics(node.shape)) return;
    const next = dynamicRange(node, enabled ? range : 0);
    if (node.extra) {
      if (sendBand(next) && !selectedKeys.has(node.key)) selectGroup([node.key]);
    } else replaceBand(node, next, false);
  }

  function supportsDynamics(shape: EqShape) {
    return shape === 'bell' || supportsAllShapes;
  }

  function dynamicRange(node: EqNode, amount: number, threshold = node.dynamic?.thresholdDb ?? -24, ratio = node.dynamic?.ratio): EqNode {
    const limit = supportsUpward ? -12 : 0;
    const range = roundEqDb(Math.max(limit, Math.min(12, amount)));
    return { ...node, dynamic: range ? { thresholdDb: threshold, amountDb: Math.sign(range) * Math.max(1, Math.abs(range)),
      ...(ratio === undefined ? {} : { ratio }), ...(node.dynamic?.spectral ? { spectral: true } : {}) } : undefined };
  }

  function rangeHandlePosition(node: EqNode) {
    const spot = position(node);
    const amount = node.dynamic?.amountDb ?? 0;
    const direction = Math.sign(amount || dynamicsGesture?.amount || 1);
    const target = gainY(yGain(spot.y, bounds) - amount, bounds);
    const y = Math.max(bounds.top, Math.min(bounds.bottom, spot.y + direction * Math.max(24, Math.abs(target - spot.y))));
    return { x: Math.abs(y - spot.y) < 23 ? (spot.x > (bounds.left + bounds.right) / 2 ? -25 : 25) : 0, y: y - spot.y };
  }

  function rangeHandleKeys(event: KeyboardEvent, node: EqNode) {
    const { key } = event;
    if (key === 'ArrowLeft' || key === 'ArrowRight') { nodeKeys(event, node); return; }
    if (event.altKey || event.ctrlKey || event.metaKey || !['ArrowUp', 'ArrowDown', 'Home'].includes(key)) return;
    event.preventDefault();
    const amount = key === 'Home' ? 0 : (node.dynamic?.amountDb ?? 0) + (key === 'ArrowDown' ? 1 : -1) * (event.shiftKey ? eqGainStep : 0.5);
    const next = dynamicRange(node, amount);
    if (sendBand(next) && !next.dynamic) void tick().then(() => focusNode(node.key));
  }

  async function editRange(node: EqNode) {
    selectGroup(selectedKeys.has(node.key) ? [...selectedKeys] : [node.key], node.key);
    await tick();
    root.querySelector<HTMLButtonElement>('.band-editor [data-eq-dial="Dynamic range"] button')?.click();
  }

  function setAdaptiveQ(node: EqNode, enabled: boolean) {
    const next = { ...node, adaptiveQ: enabled };
    if (node.extra) sendBand(next);
    else replaceBand(node, next);
  }

  function soloBand(node: EqNode): PreviewBand | undefined {
    const [min, max] = eqFrequencyRange(node, rate);
    const range = eqSoloRange({ ...node, hz: Math.max(min, Math.min(max, node.hz)) }, rate);
    return range && { ...range, channel: node.channel === 'stereo' ? undefined : node.channel };
  }

  function startSolo(node: EqNode) {
    const band = soloBand(node);
    if (!snapshot || !band) return false;
    soloed = node.key;
    currentSoloBand = band;
    onaction('solo-band', undefined, band);
    return true;
  }

  function retuneSolo() {
    soloTimer = undefined;
    const band = pendingSoloBand;
    if (!band || !soloed) return;
    pendingSoloBand = undefined;
    if (band.lowHz === currentSoloBand?.lowHz && band.highHz === currentSoloBand.highHz && band.channel === currentSoloBand.channel) return;
    currentSoloBand = band;
    onaction('retune-solo-band', undefined, band);
  }

  function stopSolo() {
    clearTimeout(soloTimer);
    soloTimer = undefined;
    pendingSoloBand = currentSoloBand = undefined;
    heldKey = undefined;
    if (!soloed) return;
    soloed = null;
    onaction('stop-audition');
  }

  function soloToggle(node: EqNode) {
    if (soloed === node.key) stopSolo();
    else startSolo(node);
  }

  function releaseHeldSolo() {
    if (heldKey !== undefined) stopSolo();
  }

  function soloKeys(event: KeyboardEvent, node: EqNode) {
    if (event.key !== ' ' && event.key !== 'Enter') return;
    event.preventDefault();
    if (event.type === 'keyup') {
      if (heldKey === event.key) stopSolo();
    } else if (!event.repeat) {
      heldKey = event.key;
      startSolo(node);
    }
  }

  function hasAnalyzerTrace(reading: TreatmentProbe, view: AnalyzerView) {
    return view !== 'off' && (view === 'after' || reading.original.length > 1)
      && (view === 'before' || reading.treated.length > 1);
  }

  function grabLevels(reading: TreatmentProbe) {
    return analyzer === 'before' || (analyzer === 'both' && snapshot?.original)
      ? reading.original : reading.treated;
  }

  function clearGrabHover() {
    if (!grabHover) return;
    clearTimeout(grabHover.timer);
    grabHover = undefined;
  }

  function resumeGrab() {
    clearGrabHover();
    if (frozenReadings?.automatic && !peakDrag) frozenReadings = undefined;
  }

  function hoverSpectrum(event: PointerEvent) {
    const sourceId = snapshot?.sourceId;
    const reading = plottedSpectrum;
    if (!sourceId || !reading || !playing || frozen || !canAdd || soloed || rulerDrag
      || event.pointerType !== 'mouse' || event.buttons || !hasAnalyzerTrace(reading, analyzer)) {
      clearGrabHover();
      return;
    }
    const { offsetX: x, offsetY: y } = event;
    if (!spectrumTraceNear(x, y, bounds, rate, reading.frequencies, grabLevels(reading))) {
      clearGrabHover();
      return;
    }
    if (grabHover && Math.hypot(x - grabHover.x, y - grabHover.y) <= 10) {
      grabHover.probe = reading;
      return;
    }
    clearGrabHover();
    const timer = setTimeout(() => {
      const candidate = grabHover?.probe;
      grabHover = undefined;
      if (!candidate || !playing || frozen || drag || soloed || marquee || rulerDrag || peakDrag
        || sourceId !== snapshot?.sourceId || !canAdd || !hasAnalyzerTrace(candidate, analyzer)) return;
      frozenReadings = { sourceId, probe: candidate, automatic: true };
      ghost = undefined;
    }, 360);
    grabHover = { x, y, probe: reading, timer };
  }

  function chooseAnalyzer(view: string | null) {
    if (!view) return;
    resumeGrab();
    const next = view as AnalyzerView;
    if (frozen && !hasAnalyzerTrace(frozen, next)) freezeSpectrum();
    analyzer = next;
  }

  function freezeSpectrum() {
    clearGrabHover();
    if (frozen) {
      finishPeak();
      frozenReadings = undefined;
      report = 'Live spectrum resumed.';
    } else if (probe && snapshot && hasAnalyzerTrace(probe, analyzer)) {
      frozenReadings = { sourceId: snapshot.sourceId, probe };
      report = 'Spectrum frozen.';
    }
  }

  function startRuler(event: PointerEvent) {
    if (event.button !== 0 || rulerDrag || drag || peakDrag) return;
    const control = event.currentTarget as HTMLButtonElement;
    const rect = control.getBoundingClientRect();
    rulerDrag = { control, pointer: event.pointerId, x: event.clientX, y: event.clientY, width: rect.width,
      anchor: Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)), view };
    ghost = undefined;
    control.setPointerCapture(event.pointerId);
  }

  function rulerView(current: RulerDrag, event: PointerEvent) {
    return moveEqView(current.view, current.anchor, Math.exp((event.clientY - current.y) / 130),
      (current.x - event.clientX) / current.width);
  }

  function moveRuler(event: PointerEvent) {
    if (!rulerDrag || rulerDrag.pointer !== event.pointerId) return;
    pendingView = rulerView(rulerDrag, event);
    if (!rulerFrame) rulerFrame = requestAnimationFrame(() => {
      rulerFrame = 0;
      if (pendingView) view = pendingView;
    });
  }

  function finishRuler(event?: PointerEvent) {
    const current = rulerDrag;
    if (!current || (event && current.pointer !== event.pointerId)) return;
    cancelAnimationFrame(rulerFrame);
    rulerFrame = 0;
    rulerDrag = undefined;
    pendingView = undefined;
    view = event?.type === 'pointerup' ? rulerView(current, event) : current.view;
    if (current.control.hasPointerCapture(current.pointer)) current.control.releasePointerCapture(current.pointer);
  }

  function resetRuler() {
    view = { ...view, start: 0, end: 1 };
  }

  function rulerKeys(event: KeyboardEvent) {
    switch (event.key) {
      case '0': resetRuler(); break;
      case '+': case '=': case 'ArrowUp': view = moveEqView(view, 0.5, 0.75, 0); break;
      case '-': case 'ArrowDown': view = moveEqView(view, 0.5, 1.33, 0); break;
      case 'ArrowLeft': view = moveEqView(view, 0.5, 1, -0.15); break;
      case 'ArrowRight': view = moveEqView(view, 0.5, 1, 0.15); break;
      default: return;
    }
    event.preventDefault();
  }

  function peakNode(peak: SpectrumPeak) {
    return { ...makeExtraEqNode(settings, 'bell'), hz: peak.hz, q: peak.q, adaptiveQ: false };
  }

  function startPeak(event: PointerEvent, peak: SpectrumPeak) {
    if (!snapshot || !canAdd || event.button !== 0 || peakDrag || drag || rulerDrag) return;
    const control = event.currentTarget as HTMLButtonElement;
    const node = peakNode(peak);
    peakDrag = { control, pointer: event.pointerId, x: event.clientX, y: event.clientY,
      sourceId: snapshot.sourceId, node, before: { eqBands: settings.eqBands },
      gesture: id + '-spectrum-' + ++gestureNumber, gain: 0, applied: false };
    activePeak = peak.hz;
    control.setPointerCapture(event.pointerId);
  }

  function peakGain(current: PeakDrag, event: PointerEvent) {
    const distance = event.clientY - current.y;
    return distance < 3 ? 0 : -Math.min(eqGainLimit, roundEqDb(distance / 4));
  }

  function paintPeak(current: PeakDrag) {
    if (current.sourceId !== snapshot?.sourceId || (!current.applied && !current.gain)) return;
    if (sendBand({ ...current.node, gainDb: current.gain }, current.gesture)) current.applied = true;
  }

  function movePeak(event: PointerEvent) {
    const current = peakDrag;
    if (!current || event.pointerId !== current.pointer) return;
    const gain = peakGain(current, event);
    if (gain === current.gain) return;
    current.gain = gain;
    if (!peakFrame) peakFrame = requestAnimationFrame(() => {
      peakFrame = 0;
      paintPeak(current);
    });
  }

  function finishPeak(event?: PointerEvent) {
    const current = peakDrag;
    if (!current || (event && event.pointerId !== current.pointer)) return;
    cancelAnimationFrame(peakFrame);
    peakFrame = 0;
    peakDrag = undefined;
    activePeak = undefined;
    if (current.sourceId === snapshot?.sourceId) {
      current.gain = event?.type === 'pointerup' ? peakGain(current, event) : 0;
      if (current.gain) {
        paintPeak(current);
        if (current.applied) onmode(current.node.key);
      } else if (current.applied) onedit(current.before, current.gesture);
    }
    if (current.control.hasPointerCapture(current.pointer)) current.control.releasePointerCapture(current.pointer);
    resumeGrab();
  }

  function nodeHits(control: HTMLElement, event: MouseEvent) {
    const plot = control.closest('.plot')!.getBoundingClientRect();
    const point = { x: event.clientX - plot.left, y: event.clientY - plot.top };
    const hits = visibleNodes.map(node => {
      const spot = position(node);
      return { node, distance: Math.hypot(spot.x - point.x, spot.y - point.y) };
    }).filter(hit => hit.distance <= nodeHitRadius).sort((a, b) =>
      a.distance - b.distance || Number(b.node.key === mode) - Number(a.node.key === mode));
    return { point, hits };
  }

  function clickNode(current: Drag, event: PointerEvent) {
    const { point, hits } = nodeHits(current.control, event);
    const nearest = hits[0];
    const linked = nearest ? hits.filter(hit => hit.distance <= nearest.distance + 4) : [];
    if (linked.length < 2 || event.ctrlKey || event.metaKey || event.shiftKey) {
      lastNodeClick = undefined;
      selectNode(current.node, true, event);
      return;
    }
    const keys = linked.map(hit => hit.node.key).sort().join(',');
    const previous = lastNodeClick;
    const repeat = previous && previous.sourceId === current.sourceId && previous.keys === keys
      && event.timeStamp - previous.time < 1000
      && Math.hypot(point.x - previous.point.x, point.y - previous.point.y) < 6;
    const ordered = linked.map(hit => hit.node).sort((a, b) => a.key.localeCompare(b.key));
    const node = repeat ? ordered[(ordered.findIndex(item => item.key === previous.picked) + 1) % ordered.length] : nearest.node;
    lastNodeClick = { sourceId: current.sourceId, keys, point, time: event.timeStamp, picked: node.key };
    selectNode(node, true, event);
  }

  function doubleClickNode(event: MouseEvent, node: EqNode) {
    const { hits } = nodeHits(event.currentTarget as HTMLElement, event);
    if (hits.length > 1 && hits[1].distance <= hits[0].distance + 4) return;
    void exact(hits[0]?.node ?? node);
  }

  function startDrag(event: PointerEvent, node: EqNode, kind: Drag['kind'] = 'node') {
    if (!snapshot || !canAdjust || event.button !== 0 || drag || peakDrag || rulerDrag || marquee) return;
    if (kind === 'solo' && !startSolo(node)) return;
    const control = event.currentTarget as HTMLButtonElement;
    if (kind === 'node') node = nodeHits(control, event).hits[0]?.node ?? node;
    const origin = position(node);
    const canPan = kind === 'node' || (kind === 'dynamic' && node.dynamic);
    const members = canPan && selectedKeys.has(node.key) && !event.ctrlKey && !event.metaKey
      ? nodes.filter(item => selectedKeys.has(item.key)) : [node];
    const before = Object.assign({}, ...members.map(item => originalEqBand(settings, item)));
    drag = {
      pointerId: event.pointerId, control, sourceId: snapshot.sourceId, node, members, previousMode: mode, previousKeys: [...selectedKeys],
      before, gesture: id + '-' + ++gestureNumber,
      bounds, x: event.clientX, y: event.clientY, lastX: event.clientX, lastY: event.clientY,
      targetX: origin.x, targetY: origin.y, kind, moved: false, applied: false,
    };
    draggedKey = node.key;
    if (kind === 'dynamic') dynamicsGesture = { key: node.key, handle: !!node.dynamic, amount: node.dynamic?.amountDb ?? 0 };
    if (kind === 'solo') selectGroup([node.key]);
    ghost = undefined;
    control.setPointerCapture(event.pointerId);
  }

  function accumulate(current: Drag, event: PointerEvent) {
    const dx = event.clientX - current.lastX;
    const dy = event.clientY - current.lastY;
    current.lastX = event.clientX;
    current.lastY = event.clientY;
    const fine = event.shiftKey ? 0.2 : 1;
    const canPan = current.kind !== 'dynamic' || !!current.node.dynamic;
    const canTilt = current.kind !== 'solo' && (current.kind === 'dynamic' || supportsGain(current.node) || current.members.length > 1);
    if (!event.altKey) current.axis = undefined;
    else if (!current.axis && canPan && canTilt && (dx || dy)) {
      current.axis = Math.abs(dx) >= Math.abs(dy) ? 'x' : 'y';
    }
    if (canPan && current.axis !== 'y') current.targetX += dx * fine;
    if (current.kind !== 'solo' && current.axis !== 'x') current.targetY += dy * fine;
    current.moved ||= Math.hypot(event.clientX - current.x, event.clientY - current.y) >= 3;
    return current.moved && (dx !== 0 || dy !== 0);
  }

  function moveGroup(members: EqNode[], ratio: number, gainDelta: number, anchorGain: number) {
    let minimumRatio = 0;
    let maximumRatio = Infinity;
    let largestGain = 0;
    for (const member of members) {
      const [min, max] = eqFrequencyRange(member, rate);
      minimumRatio = Math.max(minimumRatio, min / member.hz);
      maximumRatio = Math.min(maximumRatio, max / member.hz);
      if (supportsGain(member)) largestGain = Math.max(largestGain, Math.abs(member.gainDb));
    }
    const scale = Math.max(minimumRatio, Math.min(maximumRatio, ratio));
    const amplitude = largestGain ? Math.max(0, Math.min(eqGainLimit / largestGain, 1 + gainDelta / (anchorGain || largestGain))) : 1;
    return members.map(member => ({ ...member, hz: member.hz * scale,
      gainDb: supportsGain(member) ? (largestGain ? member.gainDb * amplitude : gainDelta) : member.gainDb }));
  }

  function applyDrag(current: Drag) {
    if (current.sourceId !== snapshot?.sourceId) return;
    const { node, members, bounds: geometry } = current;
    const origin = gainY(supportsGain(node) ? node.gainDb : 0, geometry);
    let changed: EqNode[];
    if (current.kind === 'dynamic') {
      const change = node.dynamic ? yGain(origin, geometry) - yGain(current.targetY, geometry, false) : (current.targetY - origin) / 4;
      const moved = node.dynamic ? moveGroup(members, xFrequency(current.targetX, geometry, rate) / node.hz, 0, node.gainDb) : members;
      changed = moved.map(member => member.key === node.key
        ? dynamicRange(member, (node.dynamic?.amountDb ?? 0) + change) : member);
    } else if (members.length > 1) {
      const gain = yGain(current.targetY, geometry, false) - yGain(origin, geometry);
      changed = moveGroup(members, xFrequency(current.targetX, geometry, rate) / node.hz, gain, yGain(origin, geometry));
    } else changed = [movingNode(node, current.targetX, current.targetY, geometry)];
    if (!sendBands(changed, current.gesture)) return;
    current.applied = true;
    if (mode !== node.key) selectGroup(current.kind === 'dynamic' && selectedKeys.has(node.key)
      ? [...selectedKeys] : members.map(member => member.key), node.key);
    if (current.kind === 'solo' && soloed === node.key) {
      pendingSoloBand = soloBand(changed[0]);
      soloTimer ??= setTimeout(retuneSolo, 75);
    }
  }

  function move(event: PointerEvent) {
    const current = drag;
    if (!current) {
      if (marquee) { moveMarquee(event); return; }
      if (event.currentTarget instanceof HTMLElement && event.currentTarget.classList.contains('plot-hit') && event.pointerType !== 'touch') {
        hoverSpectrum(event);
        ghost = grabbingSpectrum ? undefined : plotCandidate(event.offsetX, event.offsetY);
      }
      return;
    }
    if (event.pointerId !== current.pointerId || !accumulate(current, event) || dragFrame) return;
    dragFrame = requestAnimationFrame(() => {
      dragFrame = 0;
      if (drag !== current) return;
      moving = true;
      applyDrag(current);
    });
  }

  function finishDrag(event?: PointerEvent) {
    const current = drag;
    if (!current || (event && event.pointerId !== current.pointerId)) return;
    const pending = Boolean(dragFrame);
    cancelAnimationFrame(dragFrame);
    dragFrame = 0;
    drag = undefined;
    draggedKey = undefined;
    moving = false;
    if (event?.type === 'pointerup') {
      const changed = accumulate(current, event);
      if (pending || changed) applyDrag(current);
      else if (!current.moved && current.kind === 'node') clickNode(current, event);
      else if (!current.moved && current.kind === 'dynamic' && !current.node.dynamic) setDynamic(current.node, true);
    } else if (current.applied && current.sourceId === snapshot?.sourceId) {
      onedit(current.before, current.gesture);
      selectGroup(current.previousKeys, current.previousMode ?? undefined);
      report = 'Band adjustment cancelled.';
    }
    if (current.kind === 'solo') stopSolo();
    if (current.kind === 'dynamic') dynamicsGesture = undefined;
    if (current.control.hasPointerCapture(current.pointerId)) current.control.releasePointerCapture(current.pointerId);
  }

  function openContext(event: MouseEvent, node?: EqNode) {
    if (!canAdjust) return;
    event.preventDefault();
    event.stopPropagation();
    const control = event.currentTarget as HTMLElement;
    const rect = control.closest('.plot')!.getBoundingClientRect();
    menuPoint = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    menuNodeKey = node?.key ?? null;
    menuReturn = control;
    menuEditsValue = false;
    ghost = undefined;
    if (node) selectGroup(selectedKeys.has(node.key) ? [...selectedKeys] : [node.key], node.key);
    void bandMenu.open({ anchor: { x: event.clientX, y: event.clientY }, event });
  }

  function contextFromKey(event: KeyboardEvent) {
    if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return false;
    event.preventDefault();
    const control = event.currentTarget as HTMLElement;
    const rect = control.getBoundingClientRect();
    control.dispatchEvent(new MouseEvent('contextmenu', {
      bubbles: true, cancelable: true, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2,
    }));
    return true;
  }

  async function closeMenu(open: boolean, { reason }: ChangeDetails<MenuOpenChangeReason>) {
    if (open || menuEditsValue || (reason !== 'escape-key' && reason !== 'item-press')) return;
    const trigger = menuReturn;
    await tick();
    if (trigger?.isConnected) trigger.focus({ preventScroll: true });
    else if (mode) focusNode(mode);
    else focusGraph();
  }

  function nodeKeys(event: KeyboardEvent, node: EqNode) {
    if (contextFromKey(event) || event.altKey || event.isComposing) return;
    if (event.ctrlKey || event.metaKey) {
      const key = event.key.toLowerCase();
      if (key === 'c' && bandsForAction(node).length) {
        event.preventDefault();
        copy(bandsForAction(node));
      } else if (key === 'v' && (clipboard.length === 1 || canPasteGroup)) {
        event.preventDefault();
        paste(node);
      } else if (key === 'a') {
        event.preventDefault();
        selectGroup(nodes.filter(band => band.stored).map(band => band.key));
      }
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      void exact(node);
      return;
    }
    if (event.key === '[' || event.key === ']') {
      event.preventDefault();
      const ordered = [...nodes].sort((a, b) => a.hz - b.hz);
      const step = event.key === '[' ? -1 : 1;
      const index = ordered.findIndex(candidate => candidate.key === node.key);
      const next = ordered[(index + step + ordered.length) % ordered.length];
      if (next) {
        onmode(next.key);
        void tick().then(() => focusNode(next.key));
      }
      return;
    }
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault();
    const direction = event.key === 'ArrowRight' || event.key === 'ArrowUp' ? 1 : -1;
    const vertical = event.key === 'ArrowUp' || event.key === 'ArrowDown';
    const members = selectedKeys.has(node.key) ? nodes.filter(item => selectedKeys.has(item.key)) : [node];
    if (members.length > 1) {
      sendBands(moveGroup(members, vertical ? 1 : (event.shiftKey ? 1.015 : 1.1) ** direction,
        vertical ? direction * (event.shiftKey ? eqGainStep : 0.5) : 0, node.gainDb));
    } else if (supportsGain(node) && vertical) {
      sendBand({ ...node, gainDb: node.gainDb + direction * (event.shiftKey ? eqGainStep : 0.5) });
    } else sendBand({ ...node, hz: node.hz * (event.shiftKey ? 1.015 : 1.1) ** direction });
  }

  function adjustWheel(event: WheelEvent, node: EqNode, current?: Drag) {
    if (!event.deltaY) return;
    const spec = legacyEqBands[node.shape];
    const direction = -Math.sign(event.deltaY);
    let next: EqNode;
    if ('slope' in spec) {
      next = { ...node, slope: Math.max(12, Math.min(48, node.slope + direction * 12)) };
    } else if ('q' in spec) {
      const [min, max] = eqWidthRange(node.shape);
      const step = node.shape === 'notch' && !node.extra ? 100 : event.shiftKey ? 1 : 10;
      next = { ...node, q: Math.max(min, Math.min(max, node.q + direction * step)) };
    } else return;
    event.preventDefault();
    if (current) {
      current.node = next;
      current.members = current.members.map(member => member.key === next.key ? next : member);
      cancelAnimationFrame(dragFrame);
      dragFrame = 0;
      applyDrag(current);
    } else sendBand(next);
  }

  function scrollNode(node: EqNode, handle = false) {
    return (element: HTMLButtonElement) => {
      const wheel = (event: WheelEvent) => {
        if (event.defaultPrevented || (!handle && mode !== node.key)) return;
        if (drag) {
          if (drag.node.dynamic && drag.kind === 'dynamic') adjustWheel(event, drag.node, drag);
          return;
        }
        adjustWheel(event, findEqNode(settings, node.key) ?? node);
      };
      element.addEventListener('wheel', wheel, { passive: false });
      return () => element.removeEventListener('wheel', wheel);
    };
  }

  function escape(event: KeyboardEvent) {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    if (grabbingSpectrum) {
      event.preventDefault();
      finishPeak();
      resumeGrab();
      return;
    }
    if (drag || soloed || peakDrag || rulerDrag || marquee) {
      event.preventDefault();
      finishDrag();
      finishPeak();
      finishRuler();
      finishMarquee();
      stopSolo();
    } else if (mode && event.target instanceof Element && root.contains(event.target)) {
      event.preventDefault();
      clearSelection();
      focusGraph();
    }
  }

  function finishInteraction() {
    bandMenu.close({ reason: 'window-blur' });
    ghost = undefined;
    addOrigin = undefined;
    finishDrag();
    finishPeak();
    resumeGrab();
    finishRuler();
    finishMarquee();
    stopSolo();
  }

  function keyboard(element: HTMLElement) {
    root = element;
    const keydown = (event: KeyboardEvent) => {
      escape(event);
      if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey
        || (event.key !== 'Delete' && event.key !== 'Backspace') || drag || rulerDrag || peakDrag || marquee) return;
      const target = event.target;
      if (!(target instanceof HTMLElement) || !target.closest('.plot')
        || target.closest('input, textarea, [contenteditable], [role=menu], [role=listbox]')) return;
      const key = target.closest<HTMLElement>('[data-eq-key]')?.dataset.eqKey;
      const bands = bandsForAction(key ? findEqNode(settings, key) : undefined);
      if (!bands.length) return;
      event.preventDefault();
      if (!event.repeat) void removeBands(bands);
    };
    const wheel = (event: WheelEvent) => {
      if (!event.defaultPrevented && drag?.kind === 'dynamic' && drag.node.dynamic) adjustWheel(event, drag.node, drag);
    };
    element.addEventListener('keydown', keydown);
    element.addEventListener('wheel', wheel, { passive: false });
    return () => {
      element.removeEventListener('keydown', keydown);
      element.removeEventListener('wheel', wheel);
    };
  }

  onDestroy(() => {
    clearTimeout(peakTimer);
    finishInteraction();
  });
</script>

<svelte:window onblur={finishInteraction}/>

{#snippet dial(node: EqNode, label: string, value: number, min: number, max: number, step: number, unit: string, adjust: (current: EqNode, value: number) => EqNode, logarithmic = false, variant: 'regular' | 'gain' | 'range' = 'regular', activity = 0, detector?: number, detected = false)}
  {#key node.key}
    <EqDial {label} {value} {min} max={Math.max(min, max)} {step} {unit} color={bandColor(node)}
      disabled={!snapshot || max < min} {logarithmic} {variant} {activity} {detector} {detected} onvalue={(next, gesture) => {
        const current = findEqNode(settings, node.key);
        if (current) sendBand(adjust(current, next), gesture);
      }}/>
  {/key}
{/snippet}

{#snippet peakMeter(label: string, peak: number | undefined, hold?: number)}
  {@const db = peak === undefined ? meterFloor : Math.max(meterFloor, Math.min(0, peak))}
  <div class="peak-meter" class:peaking={db >= -1.01} role="meter" aria-label={label + ' peak level'} aria-valuemin={meterFloor}
    aria-valuemax={0} aria-valuenow={db} aria-valuetext={peak === undefined ? 'Waiting for audio' : peak.toFixed(1) + ' dBFS'}>
    <div class="peak-fill" style:--empty={100 * db / meterFloor + '%'}></div>
    {#if hold !== undefined && hold >= meterFloor}<span class="peak-hold" aria-hidden="true" style:top={100 * Math.min(0, hold) / meterFloor + '%'}></span>{/if}
  </div>
{/snippet}

{#snippet rangeDial(node: EqNode, ring = false)}
  {#if node.extra && supportsDynamics(node.shape)}
    {@const mix = snapshot?.playing && !snapshot.original && !snapshot.monitor && !node.bypassed
      ? probe?.dynamicEqMix?.[node.extra.index ?? -1] ?? 0 : 0}
    {@render dial(node, 'Dynamic range', -(node.dynamic?.amountDb ?? 0), -12, supportsUpward ? 12 : 0, eqGainStep, 'dB',
      (current, amount) => dynamicRange(current, -amount, current.dynamic?.thresholdDb ?? node.dynamic?.thresholdDb, current.dynamic?.ratio ?? node.dynamic?.ratio),
      false, ring ? 'range' : 'regular', mix)}
  {/if}
{/snippet}

<section class="eq-panel" aria-label="Equaliser controls" {@attach keyboard}>
  <Menu controller={bandMenu} modal="pointer" onOpenChange={closeMenu}>
    <div class="plot" class:has-band-selection={highlighted.size > 0} class:spectrum-grab={grabbingSpectrum}
      role="group" aria-label="Frequency response and spectrum" bind:clientWidth={width} bind:clientHeight={height}
      onpointerleave={resumeGrab}>
      <canvas {@attach paint} aria-hidden="true"></canvas>
      {#if soloFocus}
        <div class="solo-shade" aria-hidden="true" style:--solo-start={soloFocus.left + 'px'}
          style:--solo-end={soloFocus.right + 'px'} style:--solo-top={bounds.top + 'px'}
          style:--solo-bottom={height - bounds.bottom + 'px'}
          transition:fade={{ duration: prefersReducedMotion.current ? 0 : 120 }}>
          <span></span>
        </div>
      {/if}
      {#if curveDomain}
        <div class="curve-key" role="group" aria-label="Channel curves">
          {#if mixedDomains}
            <Select controller={curveViewControl} value={curveDomain} onValueChange={value => {
              if (snapshot && (value === 'left-right' || value === 'centre-sides')) {
                curveView = { sourceId: snapshot.sourceId, band: mode, domain: value };
              }
            }}>
              <SelectTrigger class="curve-domain" aria-label="EQ response channels"
                aria-description="Both views include Stereo bands" title="Choose which channel stage to plot">
                {channelDomains[curveDomain]}
              </SelectTrigger>
              <SelectContent class="app-select-menu audio-tool-popup curve-domain-popup" align="end" sideOffset={4}>
                {#each Object.entries(channelDomains) as [value, label] (value)}
                  <SelectItem {value} {label} data-slot="select-item">{label}</SelectItem>
                {/each}
              </SelectContent>
            </Select>
          {/if}
          {#each curveChannels as channel, index (channel)}
            <span class={{ secondary: index !== 0 }}>{curveLabels[channel]}</span>
          {/each}
        </div>
      {/if}
      <GraphButton type="button" class={['plot-hit', { 'add-ready': ghost }]} aria-label="Add EQ band on the graph" aria-describedby={id + '-help'} disabled={!canAdjust}
        onclick={clickPlot} onpointerdown={event => { resumeGrab(); startMarquee(event); }} onpointerup={finishMarquee}
        onpointercancel={finishMarquee} onlostpointercapture={finishMarquee}
        onpointermove={move} onpointerleave={() => { ghost = undefined; clearGrabHover(); }}
        onkeydown={event => {
          if (contextFromKey(event) || event.isComposing) return;
          if (event.ctrlKey || event.metaKey) {
            const key = event.key.toLowerCase();
            if (key === 'v' && canPasteGroup) { event.preventDefault(); paste(); }
            else if (key === 'c' && selectedStored.length) { event.preventDefault(); copy(selectedStored); }
            else if (key === 'a') {
              event.preventDefault();
              selectGroup(nodes.filter(band => band.stored).map(band => band.key));
            }
          } else if (event.key === 'Enter' && !event.ctrlKey && !event.metaKey) {
            event.preventDefault();
            add('bell');
          }
        }} oncontextmenu={event => openContext(event)}/>
      {#if box}
        <div class="selection-box" aria-hidden="true"
          style:left={box.left + 'px'} style:top={box.top + 'px'}
          style:width={box.right - box.left + 'px'} style:height={box.bottom - box.top + 'px'}></div>
      {/if}
      {#if ghost && !moving}
        {@const spot = position(ghost)}
        <div class="ghost" aria-hidden="true" style:left={spot.x + 'px'} style:top={spot.y + 'px'}></div>
      {/if}
      {#each frozenPeaks as peak (peak.hz)}
        <GraphButton type="button" class="frozen-peak" aria-label={'Grab spectrum peak at ' + Math.round(peak.hz) + ' Hz'}
          title={'Drag down to cut this peak · Enter adds a 3 dB cut · Q ' + (peak.q / 100).toFixed(2)} disabled={!canAdd && activePeak !== peak.hz}
          data-grabbed={activePeak === peak.hz} style={'left:' + peak.x + 'px;top:' + peak.y + 'px'}
          onpointerdown={event => startPeak(event, peak)} onpointermove={movePeak}
          onpointerup={finishPeak} onpointercancel={finishPeak} onlostpointercapture={finishPeak}
          onclick={event => {
            if (event.detail === 0 && canAdd && !peakDrag) {
              const node = { ...peakNode(peak), gainDb: -3 };
              if (sendBand(node)) {
                onmode(node.key);
                void tick().then(() => focusNode(node.key));
              }
            }
          }}/>
      {/each}
      {#each visibleNodes as node (node.key)}
        {@const spot = position(node)}
        {@const gesture = dynamicsGesture?.key === node.key ? dynamicsGesture : undefined}
        {@const showDynamicsIcon = supportsDynamics(node.shape) && (gesture ? !gesture.handle : !node.dynamic)}
        <div class="node-anchor" class:flip={spot.x > width - 44} class:has-dynamics={showDynamicsIcon} class:selected={highlighted.has(node.key)}
          class:soloed={node.key === soloed} style:--node-x={spot.x + 'px'} style:--node-y={spot.y + 'px'} style:--band-color={bandColor(node)}>
          <GraphButton type="button" data-eq-key={node.key} disabled={!canAdjust}
            class={['marker', { active: node.stored && !node.bypassed, bypassed: node.bypassed }]}
            aria-label={node.name} aria-description={node.bypassed ? 'Bypassed' : node.stored ? 'Active' : 'Inactive'} aria-describedby={id + '-node-help'}
            aria-pressed={highlighted.has(node.key)} title={node.name + ' · ' + Math.round(node.hz) + ' Hz' + (node.bypassed ? ' · Bypassed' : '') + ' · Enter to edit'}
            onclick={event => { if (event.detail === 0) selectNode(node, false, event); }} ondblclick={event => doubleClickNode(event, node)}
            onpointerenter={() => ghost = undefined}
            onpointerdown={event => startDrag(event, node)} onpointermove={move}
            onpointerup={finishDrag} onpointercancel={finishDrag} onlostpointercapture={finishDrag}
            onkeydown={event => nodeKeys(event, node)} oncontextmenu={event => openContext(event, node)}
            {@attach scrollNode(node)}/>
          {#if node.stored}
            <GraphButton type="button" class="solo-grip" disabled={!canAdjust}
              aria-label={'Hold to solo ' + node.name} aria-pressed={node.key === soloed}
              title={'Hold to solo ' + node.name + ' · Drag to move it'}
              onpointerdown={event => startDrag(event, node, 'solo')} onpointermove={move}
              onpointerup={finishDrag} onpointercancel={finishDrag} onlostpointercapture={finishDrag}
              onkeydown={event => soloKeys(event, node)} onkeyup={event => soloKeys(event, node)}
              onblur={releaseHeldSolo} onclick={event => { if (event.detail === 0) soloToggle(node); }}>
              <Headphones size={14} aria-hidden="true"/>
            </GraphButton>
          {/if}
          {#if showDynamicsIcon}
            <GraphButton type="button" class="dynamics-grip" disabled={!canAdjust || (!node.extra && !canAdd)}
              aria-label={'Dynamic EQ for ' + node.name} aria-pressed={!!node.dynamic}
              title={node.extra ? (supportsUpward ? 'Toggle dynamics · Drag up to lift quiet sound or down to reduce loud sound'
                : 'Toggle dynamics · Drag down for more reduction') : 'Enable dynamic EQ'}
              onpointerdown={event => { if (node.extra) startDrag(event, node, 'dynamic'); }} onpointermove={move}
              onpointerup={finishDrag} onpointercancel={finishDrag} onlostpointercapture={finishDrag}
              onclick={event => { if (event.detail === 0 || !node.extra) setDynamic(node, !node.dynamic); }}>
              <Activity size={13} aria-hidden="true"/>
            </GraphButton>
          {:else if supportsDynamics(node.shape) && (node.dynamic || gesture?.handle)}
            {@const handle = rangeHandlePosition(node)}
            <GraphButton type="button" class="range-handle" disabled={!canAdjust} data-adjusting={gesture?.handle || undefined}
              aria-label={'Dynamic range for ' + node.name} title="Drag sideways to move the band · Drag up or down for range · Scroll for width or slope"
              style={'left:calc(50% + ' + handle.x + 'px);top:calc(50% + ' + handle.y + 'px)'}
              onpointerdown={event => startDrag(event, node, 'dynamic')} onpointermove={move}
              onpointerup={finishDrag} onpointercancel={finishDrag} onlostpointercapture={finishDrag}
              onkeydown={event => rangeHandleKeys(event, node)} {@attach scrollNode(node, true)}
              onclick={event => { if (event.detail === 0) void editRange(node); }}/>
          {/if}
        </div>
      {/each}
      {#if dragReadout}
        {@const spot = position(dragReadout)}
        <span class="drag-readout" aria-hidden="true" style:left={Math.max(80, Math.min(width - 80, spot.x)) + 'px'}
          style:top={spot.y < bounds.top + 26 ? spot.y + 22 + 'px' : spot.y - 40 + 'px'}>
          {Math.round(dragReadout.hz).toLocaleString()} Hz
          {#if dynamicsGesture}{' · '}{roundEqDb(-(dragReadout.dynamic?.amountDb ?? 0))} dB range
          {:else if supportsGain(dragReadout)}{' · '}{dragReadout.gainDb > 0 ? '+' : ''}{roundEqDb(dragReadout.gainDb)} dB{/if}
        </span>
      {/if}
      <GraphButton type="button" class="frequency-ruler" aria-label="Zoom or pan frequencies"
        aria-description="Drag up to zoom, sideways to pan, or double-click to reset."
        title="Drag vertically to zoom · Drag sideways to pan · Double-click to reset"
        style={'left:' + bounds.left + 'px;right:' + (width - bounds.right) + 'px;top:' + bounds.bottom + 'px'}
        disabled={!snapshot} onpointerdown={startRuler} onpointermove={moveRuler} onpointerup={finishRuler}
        onpointercancel={finishRuler} onlostpointercapture={finishRuler} ondblclick={resetRuler} onkeydown={rulerKeys}/>
      <div class="gain-range" style:left={bounds.left + 'px'}>
        <Select controller={gainRangeControl} value={String(view.gainRange)} onValueChange={value => {
          const gainRange = gainRanges.find(range => String(range) === value);
          if (gainRange) view = { ...view, gainRange };
        }}>
          <SelectTrigger class="range-trigger" aria-label="EQ gain display range" title="EQ gain display range">±{view.gainRange} dB</SelectTrigger>
          <SelectContent class="app-select-menu audio-tool-popup" align="start" sideOffset={4}>
            {#each gainRanges as range (range)}
              <SelectItem value={String(range)} label={'±' + range + ' dB'} data-slot="select-item">±{range} dB</SelectItem>
            {/each}
          </SelectContent>
        </Select>
      </div>
    {#if selected && canAdjust}
      {@const node = selected}
      {@const [min, max] = eqFrequencyRange(node, rate)}
      {@const spec = legacyEqBands[node.shape]}
      <div class="band-editor" class:moving role="group" aria-label={node.name + ' settings'} bind:clientWidth={editorWidth}
        style:--band-x={position(node).x + 'px'} style:--editor-half={editorWidth / 2 + 'px'} style:--plot-width={width + 'px'}
        style:bottom={height - bounds.bottom + editorGap + 'px'}>
          <Button variant="ghost" size="icon-sm" class="band-power" disabled={!selectedStored.length}
            aria-label={selectionBypassed ? 'Enable selected EQ bands' : 'Bypass selected EQ bands'}
            title={selectionBypassed ? 'Enable selected EQ bands' : 'Bypass selected EQ bands'} aria-pressed={selectionBypassed}
            onclick={toggleSelection}>
            <Power size={14} aria-hidden="true"/>
          </Button>
          {@render dial(node, 'Frequency', Math.max(min, Math.min(node.hz, max)), min, max, 1, 'Hz', (current, hz) => ({ ...current, hz }), true)}
          {#if 'gain' in spec}
            <div class="gain-stack">
              {@render dial(node, 'Gain', node.gainDb, -eqGainLimit, eqGainLimit, eqGainStep, 'dB',
                (current, gainDb) => ({ ...current, gainDb }), false, 'gain')}
              {@render rangeDial(node, true)}
            </div>
          {:else}
            {@render rangeDial(node)}
          {/if}
          {#if 'q' in spec}
            {@const [minimum, maximum] = eqWidthRange(node.shape)}
            {@render dial(node, 'Width (Q)', node.q / 100, minimum / 100, maximum / 100,
              node.shape === 'notch' && !node.extra ? 1 : 0.01, '', (current, q) => ({ ...current, q: q * 100 }), true)}
          {/if}
          {#if 'slope' in spec}
            {@render dial(node, 'Slope', node.slope, 12, 48, 12, 'dB/oct', (current, slope) => ({ ...current, slope }))}
          {/if}
          {#if node.dynamic}
            {@const cut = node.dynamic}
            {@const bandIndex = node.extra?.index}
            {@const detector = liveTreatment && bandIndex !== undefined && !node.bypassed ? probe?.dynamicEqDetectors?.[bandIndex] ?? undefined : undefined}
            {@const running = snapshot?.eqLearning?.status === 'learning'}
            {@const learningError = snapshot?.eqLearning?.index === bandIndex && snapshot?.eqLearning?.status === 'error' ? snapshot.eqLearning.message : undefined}
            <div class="threshold-control">
              {@render dial(node, 'Threshold', cut.thresholdDb, -72, -6, eqGainStep, 'dBFS', (current, thresholdDb) => ({
                ...current, dynamic: current.dynamic && { ...current.dynamic, thresholdDb },
              }), false, 'regular', 0, detector, detector !== undefined && detector > -96 && (cut.amountDb > 0 ? detector > cut.thresholdDb : detector < cut.thresholdDb))}
              {#if probe?.eqThresholdLearning && snapshot?.loop && snapshot.loopRange && bandIndex !== undefined && !node.bypassed && !reference?.preview}
                <Button variant="ghost" size="icon-sm" class="learn-threshold" disabled={running}
                  aria-label="Learn threshold from loop" aria-busy={running} title={learningError ?? 'Learn threshold from loop'}
                  onclick={() => onlearn(bandIndex)}><WandSparkles size={13} aria-hidden="true"/></Button>
                {#if learningError}<span role="status" class="learn-error">{learningError}</span>{/if}
              {/if}
            </div>
            {#if supportsRatio}
              {@render dial(node, 'Ratio', cut.ratio ?? 2, 1, 20, 0.1, ':1', (current, ratio) => ({
                ...current, dynamic: current.dynamic && { ...current.dynamic, ratio },
              }))}
            {/if}
            {#if probe?.spectralEq && eqSpectralShape(node.shape)}
              <Button variant="ghost" size="icon-sm" class="spectral-mode" aria-label="Spectral dynamics"
                aria-pressed={!!cut.spectral} aria-description="Adjust individual frequencies within this band"
                title="Spectral dynamics · Adjust individual frequencies within this band"
                onclick={() => sendBand({ ...node, dynamic: { ...cut, spectral: !cut.spectral } })}>
                <AudioLines size={16} aria-hidden="true"/>
              </Button>
            {/if}
          {/if}
      </div>
    {/if}
    </div>
    <div class="eq-meters" class:playing={snapshot?.playing} role="group" aria-label="EQ levels">
      {@render peakMeter('Input', probe?.eqInputPeak)}
      {@render peakMeter('Output', probe?.outputPeak, heldPeak?.sourceId === snapshot?.sourceId ? heldPeak?.db : undefined)}
      {#if clipped}
        <Button variant="ghost" size="sm" class="clip-reset" aria-label="Clear output clipping indicator"
          title="Output would clip · Click to clear" onclick={() => { overloadSource = undefined; }}>Clip</Button>
      {/if}
      <div class="meter-scale" aria-hidden="true">
        {#each meterTicks as db (db)}
          <span style:top={100 * db / meterFloor + '%'}>{db}</span>
        {/each}
      </div>
      <div class="meter-labels" aria-hidden="true"><span>In</span><span>Out</span><span>dBFS</span></div>
    </div>
    <MenuContent class="audio-action-menu" collisionPadding={8}>
      {#if groupMenu}
        <MenuItem onSelect={toggleSelection}>{selectionBypassed ? 'Enable selected bands' : 'Bypass selected bands'}</MenuItem>
        <MenuItem onSelect={() => reset(selectedStored)}>Reset selected bands</MenuItem>
        <MenuItem disabled={selectedStored.length > availableSlots}
          onSelect={() => { menuEditsValue = true; insertCopies(selectedStored); }}>Duplicate selected bands</MenuItem>
        <MenuItem onSelect={() => copy(selectedStored)}>
          Copy selected bands <MenuShortcut class="menu-trailing">Ctrl/⌘+C</MenuShortcut>
        </MenuItem>
        <MenuItem disabled={!canPasteGroup} onSelect={() => { menuEditsValue = true; paste(); }}>
          Paste copied bands <MenuShortcut class="menu-trailing">Ctrl/⌘+V</MenuShortcut>
        </MenuItem>
        <MenuSeparator/>
        <MenuItem destructive onSelect={() => { menuEditsValue = true; void removeBands(selectedStored); }}>
          Remove selected bands <MenuShortcut class="menu-trailing">Del</MenuShortcut>
        </MenuItem>
      {:else if menuNode}
        {@const node = menuNode}
        <MenuItem onSelect={() => { menuEditsValue = true; void exact(node); }}>Edit exact values <MenuShortcut class="menu-trailing">Enter</MenuShortcut></MenuItem>
        <MenuItem onSelect={() => toggle(node)}>{node.stored && !node.bypassed ? 'Bypass band' : 'Enable band'}</MenuItem>
        <MenuItem onSelect={() => soloToggle(node)}>{soloed === node.key ? 'Stop solo' : 'Solo band'}</MenuItem>
        <MenuSeparator/>
        <MenuSub>
          <MenuSubTrigger>Change shape <ChevronRight class="menu-trailing" size={14} aria-hidden="true"/></MenuSubTrigger>
          <MenuSubContent class="audio-action-menu" sideOffset={4} collisionPadding={8}>
            {#each shapes as shape (shape)}
              {#if shape !== node.shape}
                <MenuItem disabled={!node.extra && !canAdd}
                  onSelect={() => { menuEditsValue = true; changeShape(node, shape); }}>{legacyEqBands[shape].name}</MenuItem>
              {/if}
            {/each}
          </MenuSubContent>
        </MenuSub>
        {#if snapshot?.stereo}
          <MenuSub>
            <MenuSubTrigger disabled={!node.extra && !canAdd}>Change channel <ChevronRight class="menu-trailing" size={14} aria-hidden="true"/></MenuSubTrigger>
            <MenuSubContent class="audio-action-menu" sideOffset={4} collisionPadding={8}>
              <MenuRadioGroup value={node.channel ?? 'stereo'} onValueChange={value => changeChannel(node, value as keyof typeof channels)}>
                {#each Object.entries(channels) as [channel, label] (channel)}
                  <MenuRadioItem value={channel} closeOnSelect>
                    {label}<MenuItemIndicator class="menu-trailing" aria-hidden="true"><Check size={14}/></MenuItemIndicator>
                  </MenuRadioItem>
                {/each}
              </MenuRadioGroup>
            </MenuSubContent>
          </MenuSub>
        {/if}
        {#if node.shape === 'bell'}
          <MenuCheckboxItem value="adaptive-q" checked={!!node.adaptiveQ} disabled={!node.extra && !canAdd}
            onCheckedChange={enabled => { menuEditsValue = !node.extra; setAdaptiveQ(node, enabled === true); }} closeOnSelect>
            Adaptive Q<MenuItemIndicator class="menu-trailing" aria-hidden="true"><Check size={14}/></MenuItemIndicator>
          </MenuCheckboxItem>
        {/if}
        {#if supportsDynamics(node.shape)}
          <MenuCheckboxItem value="dynamic-cut" checked={(node.dynamic?.amountDb ?? 0) > 0} disabled={!node.extra && !canAdd}
            onCheckedChange={enabled => setDynamic(node, enabled === true)} closeOnSelect>
            Dynamic cut<MenuItemIndicator class="menu-trailing" aria-hidden="true"><Check size={14}/></MenuItemIndicator>
          </MenuCheckboxItem>
          <MenuCheckboxItem value="dynamic-lift" checked={(node.dynamic?.amountDb ?? 0) < 0} disabled={!supportsUpward || (!node.extra && !canAdd)}
            onCheckedChange={enabled => setDynamic(node, enabled === true, -4)} closeOnSelect>
            Upward compression<MenuItemIndicator class="menu-trailing" aria-hidden="true"><Check size={14}/></MenuItemIndicator>
          </MenuCheckboxItem>
        {/if}
        <MenuItem onSelect={() => reset([node])}>Reset band</MenuItem>
        <MenuItem disabled={!node.stored || !canAdd} onSelect={() => { menuEditsValue = true; insertCopies([node]); }}>Duplicate band</MenuItem>
        <MenuItem disabled={!node.stored} onSelect={() => copy([node])}>Copy band settings <MenuShortcut class="menu-trailing">Ctrl/⌘+C</MenuShortcut></MenuItem>
        <MenuItem disabled={!(clipboard.length === 1 && node.extra) && !canPasteGroup}
          onSelect={() => { menuEditsValue = true; paste(node); }}>Paste band settings <MenuShortcut class="menu-trailing">Ctrl/⌘+V</MenuShortcut></MenuItem>
        <MenuSeparator/>
        <MenuItem destructive disabled={!node.stored} onSelect={() => { menuEditsValue = true; void removeBands([node]); }}>Remove band <MenuShortcut class="menu-trailing">Del</MenuShortcut></MenuItem>
      {:else}
        {#each shapes as shape (shape)}
          <MenuItem disabled={!canAdd} onSelect={() => { menuEditsValue = true; add(shape, menuPoint ? xFrequency(menuPoint.x, bounds, rate) : undefined); }}>
            Add {legacyEqBands[shape].name.toLowerCase()}
          </MenuItem>
        {/each}
        <MenuSeparator/>
        <MenuItem disabled={!canPasteGroup} onSelect={() => { menuEditsValue = true; paste(); }}>Paste band settings <MenuShortcut class="menu-trailing">Ctrl/⌘+V</MenuShortcut></MenuItem>
      {/if}
    </MenuContent>
  </Menu>
  <div class="eq-footer">
    <span id={id + '-help'} class="sr-only">
      Click near 0 dB to add a band. Drag across empty space to select several bands.
    </span>
    <div class="eq-actions">
      <Popover controller={gainControl}>
        <PopoverTrigger class="gain-trigger" title="Adjust EQ input, strength and output" disabled={!snapshot}>
          <SlidersHorizontal size={15} aria-hidden="true"/> Gain strength
        </PopoverTrigger>
        <PopoverContent class="app-select-menu audio-tool-popup eq-gain-popup" side="top" align="start"
          sideOffset={6} collisionPadding={10} aria-label="EQ gain and strength">
          <div class="gain-dials">
            {#key snapshot?.sourceId}
              <EqDial label="Input" value={settings.eqInputDb} min={inputSpec.min} max={inputSpec.max}
                step={eqGainStep} unit={inputSpec.unit} color="var(--accent-ink)" disabled={!snapshot}
                resetValue={inputSpec.initial} onvalue={editGain('eqInputDb', snapshot?.sourceId)}/>
              <EqDial label="Strength" value={strength} min={strengthSpec.min} max={strengthSpec.max}
                step={strengthSpec.step ?? 1} unit={strengthSpec.unit} color="var(--accent-ink)" disabled={!snapshot}
                resetValue={strengthSpec.initial} ondragchange={dragging => adjustingStrength = dragging}
                onvalue={editGain('eqStrength', snapshot?.sourceId)}/>
              <EqDial label="Output" value={settings.eqOutputDb} min={outputSpec.min} max={outputSpec.max}
                step={eqGainStep} unit={outputSpec.unit} color="var(--accent-ink)" disabled={!snapshot}
                resetValue={outputSpec.initial} onvalue={editGain('eqOutputDb', snapshot?.sourceId)}/>
            {/key}
          </div>
        </PopoverContent>
      </Popover>
    </div>
    <div class="eq-actions monitor-actions" role="group" aria-label="Spectrum">
      <Select controller={analyzerControl} value={analyzer} onValueChange={chooseAnalyzer}>
        <SelectTrigger class="analyzer-choice" aria-label="Spectrum display" title="Choose which spectrum to show">{analyzerViews[analyzer]}</SelectTrigger>
        <SelectContent class="app-select-menu audio-tool-popup" align="end" sideOffset={4}>
          {#each Object.entries(analyzerViews) as [value, label] (value)}
            <SelectItem {value} {label} data-slot="select-item">{label}</SelectItem>
          {/each}
        </SelectContent>
      </Select>
      <Button variant="ghost" size="icon-sm" aria-label={frozen ? 'Resume spectrum' : 'Freeze spectrum'}
        title={frozen ? 'Resume spectrum' : 'Freeze spectrum'} aria-pressed={Boolean(frozen)}
        disabled={!frozen && (!probe || !hasAnalyzerTrace(probe, analyzer))}
        onclick={freezeSpectrum}><Snowflake size={16} aria-hidden="true"/></Button>
    </div>
  </div>
  {#if reference}
    <div class="reference" role="group" aria-label="Reference EQ">
      {#if reference.status === 'choosing'}
        <span role="status">Choosing a reference…</span>
      {:else if reference.status === 'analysing'}
        <span role="status">Analysing {reference.name}…</span>
      {:else if reference.status === 'error'}
        <span role="status">{reference.message}</span>
      {:else}
        <span title={reference.name}>{reference.name} · {reference.count} {reference.count === 1 ? 'band' : 'bands'}</span>
      {/if}
      <div class="reference-actions">
        {#if reference.status === 'ready'}
          <Button size="sm" variant="ghost" aria-pressed={reference.preview ?? false}
            title="Hear and edit the suggested curve before keeping it"
            onclick={() => onaction('reference-preview')}>Preview match</Button>
          <Button size="sm" onclick={() => onaction('reference-keep')}>Keep match</Button>
        {/if}
        <Button size="sm" variant="ghost" onclick={() => onaction('reference-cancel')}>Dismiss</Button>
      </div>
    </div>
  {/if}
  <span id={id + '-node-help'} class="sr-only">Drag a selected band to move the selection. Ctrl or Command-click toggles a band. Shift-click selects a range. Enter opens exact values.</span>
  <span class="sr-only" aria-live="polite">{report}</span>
</section>

<style>
  .eq-panel {
    grid-column:1; grid-row:2 / 4; display:grid;
    grid-template-columns:minmax(0,1fr) auto; grid-template-rows:minmax(218px,1fr);
    gap:var(--sp-2,8px); min-width:0;
  }
  .eq-meters {
    grid-column:2; grid-row:1; position:relative; display:grid; grid-template-columns:repeat(2, 13px) 22px;
    grid-template-rows:minmax(0,1fr) 25px; column-gap:4px; padding-block-start:25px;
    font:10px var(--font-literal); color:var(--muted);
  }
  .meter-scale { position:relative; font-variant-numeric:tabular-nums; }
  .meter-scale span { position:absolute; inset-inline-start:0; transform:translateY(-50%); }
  .peak-meter { position:relative; overflow:hidden; background:var(--bg); border-radius:3px; }
  .peak-fill {
    position:absolute; inset:0; clip-path:inset(var(--empty) 0 0);
    background:linear-gradient(to top, var(--add) 0 90%, var(--accent-ink) 98.3%, var(--del) 98.3%);
  }
  .playing .peak-fill { transition:clip-path 55ms linear; }
  .peak-meter.peaking::after { content:''; position:absolute; inset:0 0 auto; block-size:2px; background:var(--del); }
  .peak-hold { position:absolute; inset-inline:0; height:1px; background:var(--text); opacity:.7; }
  .eq-meters :global(.clip-reset) {
    position:absolute; inset:0 auto auto 8px; height:20px; padding:0 3px; font-size:9px; color:var(--del);
  }
  :global(.eq-gain-popup) { padding:0; }
  .gain-dials {
    display:flex; align-items:center; gap:var(--sp-1); padding:var(--sp-2);
    --eq-dial-size:34px; --eq-dial-width:58px;
  }
  .drag-readout {
    position:absolute; z-index:7; transform:translateX(-50%); pointer-events:none;
    padding:4px 6px; border-radius:var(--r-compact); background:var(--panel);
    color:var(--text); font:10px var(--font-literal); white-space:nowrap;
  }
  .threshold-control { position:relative; }
  .threshold-control :global(.learn-threshold) {
    position:absolute; inset:-10px -5px auto auto; width:23px; height:23px; padding:0; color:var(--muted);
  }
  .threshold-control :global(.learn-threshold:is(:hover,:focus-visible)) { color:var(--text); }
  .threshold-control :global(.learn-threshold[aria-busy=true]) { opacity:.5; }
  .learn-error {
    position:absolute; inset:auto 0 calc(100% + 12px) auto; width:max-content; max-width:170px;
    padding:4px 6px; border-radius:var(--r-compact); background:var(--panel);
    color:var(--del); font-size:var(--text-xs);
  }
  .meter-labels {
    grid-column:1 / -1; display:grid; grid-template-columns:inherit;
    column-gap:4px; justify-items:center; align-items:end; padding-block-end:5px;
    font-size:9px;
  }
  .plot { grid-column:1; grid-row:1; position:relative; isolation:isolate; overflow:hidden; overflow:clip; min-width:0; min-height:218px; }
  .eq-footer { grid-column:1 / -1; grid-row:2; }
  .reference { grid-column:1; }
  canvas, .plot :global(.plot-hit) { position:absolute; inset:0; width:100%; height:100%; border-radius:inherit; }
  canvas { pointer-events:none; }
  .solo-shade {
    --shade:color-mix(in srgb, var(--panel) 78%, transparent);
    --feather:clamp(3px,calc((var(--solo-end) - var(--solo-start)) / 4),14px);
    position:absolute; z-index:5; inset:0; pointer-events:none;
    background:linear-gradient(to right,
      var(--shade) calc(var(--solo-start) - var(--feather)),transparent var(--solo-start),
      transparent var(--solo-end),var(--shade) calc(var(--solo-end) + var(--feather)));
  }
  .solo-shade span {
    position:absolute; inset:var(--solo-top) auto var(--solo-bottom) var(--solo-start);
    inline-size:max(0px, calc(var(--solo-end) - var(--solo-start)));
    border-inline:1px solid color-mix(in srgb, var(--text) 23%, transparent);
    box-sizing:border-box;
  }
  .plot :global(.plot-hit), .plot :global(.marker) { appearance:none; background:transparent; color:inherit; border:0; padding:0; box-shadow:none; touch-action:none; }
  .plot :global(.plot-hit) { z-index:0; cursor:default; }
  .plot :global(.plot-hit.add-ready) { cursor:crosshair; }
  .selection-box {
    position:absolute; z-index:2; pointer-events:none; box-sizing:border-box;
    background:color-mix(in srgb, var(--accent) 13%, transparent);
    border:1px solid color-mix(in srgb, var(--accent-ink) 70%, transparent);
    border-radius:2px;
  }
  .plot :global(.plot-hit:focus-visible) { outline:none; box-shadow:inset 0 -1px var(--accent-ink); }
  .plot :global(.frequency-ruler) {
    position:absolute; z-index:2; bottom:0; width:auto; height:auto; border:0;
    background:transparent; box-shadow:none; cursor:grab; touch-action:none;
  }
  .plot :global(.frequency-ruler:active) { cursor:grabbing; }
  .plot :global(.frequency-ruler:focus-visible) { outline:none; background:var(--sel-tint); }
  .gain-range { position:absolute; z-index:2; top:0; }
  .gain-range :global(.range-trigger) {
    display:block; padding:0 6px; height:23px; border:0; border-radius:var(--r-compact);
    background:transparent; color:var(--muted); font:10px var(--font-literal);
  }
  .gain-range :global(.range-trigger:is(:hover,:focus-visible,[aria-expanded=true])) { background:var(--sel-tint); color:var(--text); }
  .plot :global(.frozen-peak) {
    position:absolute; z-index:1; transform:translate(-50%, -50%); display:grid; place-items:center;
    width:25px; height:25px; border:0; border-radius:50%; padding:0;
    background:transparent; box-shadow:none; color:var(--accent-ink); cursor:ns-resize; touch-action:none;
  }
  .plot :global(.frozen-peak::before) {
    content:''; width:7px; height:7px; border:1px solid currentColor; border-radius:50%;
    background:color-mix(in srgb, currentColor 42%, transparent);
  }
  .plot :global(.frozen-peak:is(:hover,:focus-visible,[data-grabbed=true])) {
    background:var(--sel-tint); outline:none;
  }
  .plot :global(.frozen-peak[data-grabbed=true]) { cursor:grabbing; }
  .curve-key, .gain-range, .band-editor { transition:opacity 180ms ease-out; }
  .plot.spectrum-grab :is(.node-anchor, .band-editor, .curve-key, .gain-range) {
    opacity:.13; pointer-events:none;
  }
  .plot.spectrum-grab :global(.frozen-peak) { z-index:8; }
  @media (prefers-reduced-motion: reduce) {
    .plot :is(.node-anchor, .band-editor, .curve-key, .gain-range) { transition:none; }
  }
  .node-anchor {
    position:absolute; inset:0 auto auto 0; z-index:3;
    transform:translate(calc(var(--node-x) - 50%), calc(var(--node-y) - 50%));
    width:38px; height:38px; transition:opacity 110ms ease-out;
  }
  .node-anchor:is(:hover,:focus-within) { z-index:4; }
  .node-anchor:is(.selected,.soloed) { z-index:5; }
  .has-band-selection .node-anchor:not(.selected):not(:hover):not(:focus-within) { opacity:.6; }
  .node-anchor :global(.marker) {
    position:absolute; inset:0; width:100%; height:100%; display:grid; place-items:center; border-radius:50%; cursor:grab;
  }
  .plot :global(.marker:active) { cursor:grabbing; }
  .plot :global(.marker)::before {
    content:''; width:10px; height:10px; border-radius:inherit;
    border:1.5px solid var(--band-color); background:var(--panel);
  }
  .plot :global(.marker.active)::before { background:var(--band-color); opacity:.78; }
  .node-anchor.selected :global(.marker.active)::before, .plot :global(.marker.active:hover)::before { opacity:1; }
  .plot :global(.marker.bypassed)::before { border-style:dashed; opacity:.7; }
  .node-anchor.soloed :global(.marker)::before { box-shadow:0 0 0 3px color-mix(in srgb, var(--band-color) 35%, transparent); }
  .node-anchor :global(.marker:hover),
  .node-anchor.selected :global(.marker) { background:color-mix(in srgb, var(--band-color) 19%, transparent); }
  .node-anchor :global(.marker:focus-visible) { outline:none; background:color-mix(in srgb, var(--band-color) 38%, transparent); }
  .node-anchor :global(.solo-grip) {
    position:absolute; left:calc(100% - 1px); top:50%; transform:translateY(-50%);
    display:grid; place-items:center; width:24px; height:24px; padding:0; border:0; border-radius:50%;
    background:var(--panel); color:var(--band-color); box-shadow:none; opacity:0; pointer-events:none;
    cursor:grab; touch-action:none;
  }
  .node-anchor.flip :global(.solo-grip) { left:auto; right:calc(100% - 1px); }
  .node-anchor.flip.has-dynamics :global(.solo-grip) { right:calc(100% + 26px); }
  .node-anchor:is(:hover,.soloed) :global(.solo-grip),
  .node-anchor :global(.marker:focus-visible + .solo-grip),
  .node-anchor :global(.solo-grip:focus-visible) { opacity:1; pointer-events:auto; }
  .node-anchor :global(.solo-grip:active) { cursor:grabbing; }
  .node-anchor :global(.solo-grip:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
  .node-anchor :global(.dynamics-grip) {
    position:absolute; right:calc(100% - 1px); top:50%; transform:translateY(-50%);
    display:grid; place-items:center; width:24px; height:24px; padding:0; border:0; border-radius:50%;
    background:var(--panel); color:var(--band-color); box-shadow:none; opacity:0; pointer-events:none;
    cursor:ns-resize; touch-action:none;
  }
  .node-anchor:is(:hover,.selected) :global(.dynamics-grip),
  .node-anchor :global(.dynamics-grip:is([aria-pressed=true],:focus-visible)) { opacity:1; pointer-events:auto; }
  .node-anchor:not(:hover):not(.selected) :global(.dynamics-grip[aria-pressed=true]) {
    opacity:.72; background:color-mix(in srgb, var(--band-color) 12%, var(--panel));
  }
  .node-anchor :global(.dynamics-grip:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
  .node-anchor :global(.range-handle) {
    position:absolute; transform:translate(-50%, -50%); display:flex; flex-direction:column;
    align-items:center; justify-content:center; gap:3px;
    width:24px; height:24px; padding:0; border:0; box-shadow:none; border-radius:50%;
    background:transparent; color:color-mix(in srgb, var(--text) 72%, var(--panel)); cursor:move; touch-action:none;
  }
  .node-anchor :global(.range-handle)::before,
  .node-anchor :global(.range-handle)::after {
    content:''; width:8px; height:5px; background:currentColor; opacity:0;
  }
  .node-anchor :global(.range-handle)::before { clip-path:polygon(50% 0,100% 100%,0 100%); }
  .node-anchor :global(.range-handle)::after { clip-path:polygon(0 0,100% 0,50% 100%); }
  .node-anchor :global(.range-handle:focus-visible) { background:var(--sel-tint); outline:none; }
  .node-anchor :global(.range-handle:is(:hover,:focus-visible,[data-adjusting=true]))::before,
  .node-anchor :global(.range-handle:is(:hover,:focus-visible,[data-adjusting=true]))::after { opacity:1; }
  @media (hover: none) {
    .node-anchor.selected :global(.solo-grip) { opacity:1; pointer-events:auto; }
  }
  .curve-key { position:absolute; z-index:2; inset:4px 44px auto auto; display:flex; align-items:center; gap:10px; pointer-events:none; color:var(--text); font-size:var(--text-xs); }
  .curve-key span { display:flex; align-items:center; gap:4px; min-height:24px; }
  .curve-key span::before { content:''; width:10px; border-top:1px solid currentColor; }
  .curve-key :global(.curve-domain) { min-height:24px; padding:0 6px; color:var(--muted); font-size:inherit; }
  .curve-key :global(.curve-domain), :global(.curve-domain-popup) { pointer-events:auto; }
  .curve-key .secondary { color:var(--muted); }
  .curve-key .secondary::before { border-top-style:dashed; }
  .ghost { position:absolute; transform:translate(-50%, -50%); width:12px; height:12px; pointer-events:none;
    border:1px solid color-mix(in srgb, var(--accent-ink) 85%, transparent); border-radius:50%;
    background:color-mix(in srgb, var(--accent-ink) 48%, transparent);
    box-shadow:0 0 9px color-mix(in srgb, var(--accent) 52%, transparent); }
  .band-editor {
    --eq-dial-size:36px;
    --eq-dial-width:60px;
    position:absolute; z-index:2;
    left:0;
    transform:translateX(clamp(8px, calc(var(--band-x) - var(--editor-half)), calc(var(--plot-width) - 2 * var(--editor-half) - 8px)));
    display:flex; flex-wrap:wrap; justify-content:center; align-items:end; gap:var(--sp-1);
    inline-size:max-content; max-inline-size:calc(100% - 16px); box-sizing:border-box;
    padding:var(--sp-2); padding-inline-start:calc(var(--sp-2) + 24px); border-radius:var(--r-inset);
    background:color-mix(in srgb, var(--panel) 91%, transparent);
    -webkit-backdrop-filter:blur(10px); backdrop-filter:blur(10px);
    box-shadow:0 6px 18px rgb(0 0 0 / 23%);
  }
  .band-editor.moving { background:var(--panel); -webkit-backdrop-filter:none; backdrop-filter:none; }
  .gain-stack { position:relative; --eq-dial-width:72px; --eq-gain-disc:42px; --eq-range-disc:60px; }
  .band-editor :global(.band-power) {
    position:absolute; inset:4px auto auto 4px; width:24px; height:24px; border:0;
    background:transparent; box-shadow:none; color:var(--accent-ink);
  }
  .band-editor :global(.band-power[aria-pressed=true]) { color:var(--muted); }
  .band-editor :global(.band-power:focus-visible) { outline:none; background:var(--sel-tint); }
  .band-editor :global(.spectral-mode) { align-self:center; color:var(--muted); }
  .band-editor :global(.spectral-mode[aria-pressed=true]) { background:var(--sel-tint); color:var(--accent-ink); }
  .reference { display:flex; flex-wrap:wrap; justify-content:space-between; align-items:center; gap:var(--sp-1) var(--sp-2); font-size:var(--text-xs); color:var(--muted); }
  .reference > span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; max-width:100%; }
  .reference-actions { display:flex; align-items:center; gap:var(--sp-1); }
  .reference-actions :global([aria-pressed=true]) { background:var(--sel-tint); color:var(--accent-ink); }
  .eq-footer { min-height:28px; display:flex; align-items:center; flex-wrap:wrap; gap:var(--sp-1) var(--sp-3); }
  .eq-actions { display:flex; align-items:center; gap:var(--sp-1); }
  .monitor-actions { margin-inline-start:auto; }
  .eq-actions > :global(button) {
    display:inline-flex; align-items:center; justify-content:center; gap:var(--sp-1);
    min-inline-size:28px; block-size:28px; border:0; border-radius:var(--r-compact);
    background:transparent; color:var(--muted); box-shadow:none; font-size:var(--text-xs);
  }
  .eq-actions > :global(button:is(.gain-trigger,.analyzer-choice)) { padding-inline:var(--sp-2); }
  .eq-actions > :global(button:hover:not(:disabled)) { color:var(--text); background:var(--row-hover); }
  .eq-actions > :global(button:is([aria-pressed=true],[aria-expanded=true])) {
    color:var(--accent-ink); background:var(--sel-tint);
  }
  .eq-actions > :global(button:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
  @media (max-width:460px) {
    .band-editor { --eq-dial-size:30px; --eq-dial-width:49px; padding:var(--sp-1); padding-inline-start:28px; }
    .gain-stack { --eq-dial-width:60px; --eq-gain-disc:36px; --eq-range-disc:52px; }
  }
  @media (max-height:420px) {
    .eq-panel { grid-template-rows:minmax(146px,1fr); }
    .plot { min-height:146px; }
  }
  @media (prefers-reduced-motion:reduce) {
    .playing .peak-fill, .node-anchor { transition:none; }
  }
</style>
