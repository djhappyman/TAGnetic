<script lang="ts">
  import { tick } from 'svelte';
  import { Button } from '@ui-sdk/svelte/button';
  import { createMenuController, Menu, MenuContent, MenuItem } from '@ui-sdk/svelte/menu';
  import { timeLabel, type FrameRange, type PeakTile, type Track } from '../js/audio-split';

  let {
    start, end, total, sampleRate, tile = null, overview, stereoOverview, overviewRms, stereoOverviewRms, stereo = false, playhead, tracks, selected,
    readonly = false, playing = false, coverage = 1, onmove, onseek, oncancel, onselect, onmarker, onviewport, onwheelzoom, ongesture, onkey,
    onplaymarker, onadd, oneditmarker, onremovemarker, ontrim, onrestoremarker, visualPlayhead,
    loopRegion = null, loopEnabled = false, onloop, onmovetoplayhead, canmove, ontrack, trackTitle,
  }: {
    start: number; end: number; total: number; sampleRate: number;
    tile?: PeakTile | null; overview: Float32Array; stereoOverview?: Float32Array; stereo?: boolean;
    overviewRms?: Float32Array; stereoOverviewRms?: Float32Array;
    playhead: number; visualPlayhead?: number; ontrack?: (track: Track) => void; trackTitle?: (track: Track) => string | undefined;
    tracks: Track[]; selected: string; readonly?: boolean; playing?: boolean; coverage?: number;
    onmove?: (frame: number) => void; onseek?: (frame: number) => void;
    oncancel?: (frame: number) => void; onselect?: (id: string) => void; onmarker?: (id: string, frame: number) => void;
    onviewport?: (start: number, span: number) => void;
    onwheelzoom?: (delta: number, fraction: number) => void;
    ongesture?: (active: boolean) => void; onkey?: (event: KeyboardEvent) => void;
    onplaymarker?: (id: string, frame: number) => void; onadd?: (frame: number) => void;
    oneditmarker?: (id: string) => void; onremovemarker?: (id: string) => void;
    ontrim?: (frame: number) => void;
    onrestoremarker?: (id: string) => void;
    loopRegion?: FrameRange | null; loopEnabled?: boolean; onloop?: (region: FrameRange) => void;
    onmovetoplayhead?: (id: string) => void; canmove?: (id: string) => boolean;
  } = $props();

  let surface: HTMLDivElement;
  let detail = $state<HTMLCanvasElement>();
  let mini = $state<HTMLCanvasElement>();
  type GesturePointer = { pointer: number; control: HTMLElement; left: number; width: number };
  type LoopEdge = 'start' | 'end';
  type Drag = GesturePointer & {
    prior: number; frame: number; marker?: string;
    loop?: { anchor: number; edge?: LoopEdge };
    min: number; max: number; first: number; range: number; initialFirst: number;
    downX: number; downY: number; x: number; moved: boolean;
  };
  let drag = $state<Drag | null>(null);
  let panDrag = $state.raw<(GesturePointer & { start: number; offset: number; span: number; middle: boolean }) | null>(null);
  let edgeFrame = 0;
  let edgeTime = 0;
  const menuController = createMenuController();
  let menuOpen = $state(false);
  let menuTarget = $state<{ frame: number; marker?: string } | null>(null);
  let menuReturn: HTMLElement | null = null;
  let editingFromMenu = false;
  let releasedPointer: number | null = null;
  let pendingView: { start: number; span: number } | null = null;
  let viewFrame = 0;
  let moveFrame = 0;
  let detailWidth = $state(0);
  type CanvasSize = { width: number; height: number; scale: number; color: string };
  let detailSize = $state.raw<CanvasSize>();
  let miniSize = $state.raw<CanvasSize>();

  function measure(canvas: HTMLCanvasElement): CanvasSize {
    const { width, height } = canvas.getBoundingClientRect();
    return { width, height, scale: Math.min(3, window.devicePixelRatio || 1), color: getComputedStyle(canvas).color };
  }

  function draw(canvas: HTMLCanvasElement, size: CanvasSize, grid: boolean, first: number, last: number, levels: PeakTile[], lanes = 1) {
    const context = canvas.getContext('2d');
    if (!context || !size.width || !size.height) return;
    const { width, height, scale, color } = size;
    const pixelsWide = Math.round(width * scale);
    const pixelsHigh = Math.round(height * scale);
    if (canvas.width !== pixelsWide) canvas.width = pixelsWide;
    if (canvas.height !== pixelsHigh) canvas.height = pixelsHigh;
    context.clearRect(0, 0, pixelsWide, pixelsHigh);
    context.strokeStyle = color;
    context.fillStyle = color;
    context.lineWidth = scale;
    context.globalAlpha = 0.16;
    context.beginPath();
    const laneHeight = pixelsHigh / lanes;
    for (let channel = 0; channel < lanes; channel++) {
      const centre = (channel + 0.5) * laneHeight;
      context.moveTo(0, centre);
      context.lineTo(pixelsWide, centre);
    }
    if (grid) {
      for (let quarter = 1; quarter < 4; quarter++) {
        context.moveTo(pixelsWide * quarter / 4, 0);
        context.lineTo(pixelsWide * quarter / 4, pixelsHigh);
      }
    }
    context.stroke();
    const toX = (frame: number) => (frame - first) * pixelsWide / Math.max(1, last - first);
    const fine = levels[1];
    for (const level of levels) {
      if (lanes === 2 && !level.stereo) continue;
      const stride = level.stereo ? 4 : 2;
      const count = level.values.length / stride;
      if (!count || level.end <= level.start) continue;
      const step = (level.end - level.start) / count;
      const from = Math.max(0, Math.floor((first - level.start) / step));
      const until = Math.min(count, Math.ceil((last - level.start) / step));
      const wideBins = toX(first + step) > 7;
      for (let channel = 0; channel < lanes; channel++) {
        const centre = (channel + 0.5) * laneHeight;
        const toY = (sample: number) => centre - laneHeight * Math.min(1, Math.max(-1, sample)) * 0.44;
        const amplitude = (index: number, high: boolean) => {
          const offset = index * stride + Number(high);
          if (!level.stereo || lanes === 2) return level.values[offset + channel * 2];
          const left = level.values[offset];
          const right = level.values[offset + 2];
          return high ? Math.max(left, right) : Math.min(left, right);
        };
        const rms = level.rms;
        const rmsStride = level.stereo ? 2 : 1;
        const hasRms = rms && rms.length === count * rmsStride;
        const energyEdge = (index: number, high: boolean) => {
          if (!rms) return 0;
          const offset = index * rmsStride;
          // The combined view pools energy while the split view keeps each lane exact.
          const value = level.stereo && lanes === 1
            ? Math.sqrt((rms[offset] ** 2 + rms[offset + 1] ** 2) * 0.5)
            : rms[offset + channel];
          const inner = (high ? 0.72 : -0.72) * value;
          return Math.min(amplitude(index, true), Math.max(amplitude(index, false), inner));
        };
        const path = (begin: number, until: number, edge: (index: number, high: boolean) => number) => {
          const envelope = new Path2D();
          const top = (i: number) => toY(edge(i, true));
          const bottom = (i: number) => Math.max(top(i) + scale, toY(edge(i, false)));
          if (wideBins) {
            for (let i = begin; i < until; i++) {
              const frame = level.start + i * step;
              envelope.rect(toX(frame), top(i), toX(first + step), bottom(i) - top(i));
            }
            return envelope;
          }
          envelope.moveTo(toX(level.start + begin * step), top(begin));
          for (let i = begin; i < until; i++) {
            envelope.lineTo(toX(level.start + (i + 0.5) * step), top(i));
          }
          envelope.lineTo(toX(level.start + until * step), top(until - 1));
          envelope.lineTo(toX(level.start + until * step), bottom(until - 1));
          for (let i = until - 1; i >= begin; i--) {
            envelope.lineTo(toX(level.start + (i + 0.5) * step), bottom(i));
          }
          envelope.lineTo(toX(level.start + begin * step), bottom(begin));
          envelope.closePath();
          return envelope;
        };
        const plot = (begin: number, until: number) => {
          if (begin >= until) return;
          context.globalAlpha = wideBins ? 0.17 : 0.27;
          context.fill(path(begin, until, amplitude));
          if (hasRms) {
            context.globalAlpha = wideBins ? 0.29 : 0.43;
            context.fill(path(begin, until, energyEdge));
          }
        };
        if (fine && level !== fine) {
          plot(from, Math.min(until, Math.ceil((fine.start - level.start) / step)));
          plot(Math.max(from, Math.floor((fine.end - level.start) / step)), until);
        } else plot(from, until);
      }
    }
  }

  const coarse = $derived({ start: 0, end: total * coverage, values: overview, rms: overviewRms });
  const channelCoarse = $derived(stereo && stereoOverview?.length
    ? { ...coarse, values: stereoOverview, rms: stereoOverviewRms, stereo: true }
    : coarse);

  $effect(() => {
    if (mini && miniSize) draw(mini, miniSize, false, 0, total, [coarse]);
  });
  $effect(() => {
    if (detail && detailSize) draw(detail, detailSize, true, start, end, tile ? [channelCoarse, tile] : [channelCoarse], stereo ? 2 : 1);
  });

  function observe(node: HTMLDivElement) {
    const update = () => {
      if (mini) miniSize = measure(mini);
      if (detail) {
        detailSize = measure(detail);
        detailWidth = detailSize.width;
      }
    };
    const observer = new ResizeObserver(update);
    const colors = new MutationObserver(update);
    observer.observe(node);
    colors.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'data-palette', 'class', 'style'] });
    const listeners = new AbortController();
    const capture = { capture: true, signal: listeners.signal };
    if (!readonly) {
      window.addEventListener('pointerdown', clearReleasedPointer, capture);
      window.addEventListener('pointermove', continueGesture, capture);
      window.addEventListener('pointerup', finishGesture, capture);
      window.addEventListener('pointercancel', finishGesture, capture);
      window.addEventListener('click', suppressReleaseClick, capture);
      window.addEventListener('blur', cancelGesture, { signal: listeners.signal });
    }
    return () => {
      listeners.abort();
      observer.disconnect();
      colors.disconnect();
      if (drag || panDrag) cancelGesture();
      cancelAnimationFrame(viewFrame);
      cancelAnimationFrame(moveFrame);
    };
  }

  const span = $derived(Math.max(1, end - start));
  const percent = (frame: number) => (frame - start) / span * 100;
  const center = $derived(Math.round((start + end) / 2));
  const announceFrame = $derived(playing ? Math.min(total, Math.round(Math.floor(playhead * 10 / sampleRate) * sampleRate / 10)) : playhead);
  const visible = $derived(tracks.map((track, index) => ({ track, index }))
    .filter(({ track }) => track.startFrame >= start && track.startFrame <= end));
  const trackHeaders = $derived(tracks.map((track, index) => ({
    track, index,
    start: drag?.marker === track.id ? drag.frame : track.startFrame,
    end: drag?.marker && drag.marker === tracks[index + 1]?.id ? drag.frame : (tracks[index + 1]?.startFrame ?? total),
  })).filter(region => region.start < end && region.end > start));
  const selectedHeader = $derived(trackHeaders.find(region => region.track.id === selected));
  const firstTrack = $derived(tracks[0]);
  const opening = $derived(drag && drag.marker === firstTrack?.id ? drag.frame : firstTrack?.startFrame ?? 0);
  const shownPlayhead = $derived(visualPlayhead ?? playhead);
  const shownLoop = $derived(drag?.loop ? orderedRange(drag.loop.anchor, drag.frame) : loopRegion);

  function orderedRange(first: number, last: number): FrameRange {
    return { start: Math.min(first, last), end: Math.max(first, last) };
  }

  function pointerFraction(x: number, { left, width }: { left: number; width: number }) {
    return Math.min(1, Math.max(0, (x - left) / Math.max(1, width)));
  }

  function capturePointer(control: HTMLElement, pointer: number) {
    // The window handlers keep the gesture active if capture is unavailable.
    try { control.setPointerCapture(pointer); } catch { /* Window capture remains active. */ }
  }

  function begin(event: PointerEvent, marker?: Track, axis = surface, first = start, range = span, edge?: LoopEdge) {
    if (event.button !== 0 || drag || panDrag) return;
    const control = event.currentTarget as HTMLElement;
    event.preventDefault();
    control.focus({ preventScroll: true });
    capturePointer(axis, event.pointerId);
    ongesture?.(true);
    const { left, width } = axis.getBoundingClientRect();
    const at = Math.round(first + range * pointerFraction(event.clientX, { left, width }));
    const loop = edge && loopRegion ? { anchor: edge === 'start' ? loopRegion.end : loopRegion.start, edge }
      : event.shiftKey && axis === surface ? { anchor: at } : undefined;
    if (loop) marker = undefined;
    const frame = edge && loopRegion ? loopRegion[edge] : marker?.startFrame ?? Math.min(loop ? total : total - 1, at);
    const index = marker ? tracks.indexOf(marker) : -1;
    drag = {
      pointer: event.pointerId, control: axis, prior: marker?.startFrame ?? playhead, frame, marker: marker?.id, left, width, first, range,
      loop,
      initialFirst: first, downX: event.clientX, downY: event.clientY, x: event.clientX, moved: false,
      min: loop?.edge === 'end' ? loop.anchor + 1 : index > 0 ? tracks[index - 1].startFrame + 1 : 0,
      max: loop?.edge === 'start' ? loop.anchor - 1 : loop ? total : index >= 0 ? (tracks[index + 1]?.startFrame ?? total) - 1 : total - 1,
    };
    if (loop) drag.prior = frame;
    if (marker) onselect?.(marker.id);
    else if (!loop) {
      onmove?.(frame);
      scheduleEdgeScroll();
    }
  }

  function frameUnderPointer(active: Drag) {
    const frame = active.marker || active.loop?.edge
      ? Math.round(active.prior + active.first - active.initialFirst + (active.x - active.downX) * active.range / Math.max(1, active.width))
      : Math.round(active.first + active.range * pointerFraction(active.x, active));
    return Math.max(active.min, Math.min(active.max, frame));
  }

  let pointerTarget = 0;
  function movePlayheadOrMarker(event: PointerEvent, active: Drag) {
    if ((active.marker || active.loop) && !active.moved) {
      const distance = active.loop ? Math.abs(event.clientX - active.downX) : Math.hypot(event.clientX - active.downX, event.clientY - active.downY);
      if (distance < 4) return;
      active.moved = true;
    }
    active.x = event.clientX;
    pointerTarget = frameUnderPointer(active);
    if (!moveFrame) moveFrame = requestAnimationFrame(() => {
      moveFrame = 0;
      if (!drag) return;
      drag.frame = pointerTarget;
      if (!drag.marker && !drag.loop) onmove?.(pointerTarget);
    });
    scheduleEdgeScroll();
  }

  function edgePressure(active: Drag) {
    const zone = Math.min(52, active.width / 10);
    if (zone <= 0) return 0;
    const x = active.x - active.left;
    if (x < zone) return Math.max(-1, (x - zone) / zone);
    return Math.max(0, Math.min(1, (x - active.width + zone) / zone));
  }

  function scheduleEdgeScroll() {
    if (edgeFrame || !drag || drag.marker || drag.control !== surface || drag.range >= total) return;
    const pressure = edgePressure(drag);
    if ((pressure < 0 && drag.first > 0) || (pressure > 0 && drag.first < total - drag.range)) {
      edgeTime = performance.now();
      edgeFrame = requestAnimationFrame(scrollWhileScrubbing);
    }
  }

  function scrollWhileScrubbing(now: number) {
    edgeFrame = 0;
    if (!drag) return;
    const pressure = edgePressure(drag);
    const elapsed = Math.max(0, Math.min(32, now - edgeTime)) / 1000;
    const direction = Math.sign(pressure);
    const nextFirst = drag.first + direction * pressure ** 2 * drag.range * 1.5 * elapsed;
    drag.first = Math.max(0, Math.min(total - drag.range, nextFirst));
    onviewport?.(Math.round(drag.first), drag.range);
    pointerTarget = frameUnderPointer(drag);
    drag.frame = pointerTarget;
    if (!drag.loop) onmove?.(pointerTarget);
    scheduleEdgeScroll();
  }

  function clearReleasedPointer(event: PointerEvent) {
    if (event.pointerId === (drag || panDrag)?.pointer) cancelGesture();
    if (!drag && !panDrag) releasedPointer = null;
  }

  function suppressReleaseClick(event: MouseEvent) {
    if (releasedPointer === null || event.detail === 0 ||
      (event instanceof PointerEvent && event.pointerId > 0 && event.pointerId !== releasedPointer)) return;
    releasedPointer = null;
    event.preventDefault();
    event.stopImmediatePropagation();
  }

  function continueGesture(event: PointerEvent) {
    const active = drag || panDrag;
    if (active?.pointer !== event.pointerId) return;
    if (event.pointerType === 'mouse' && !(event.buttons & (panDrag?.middle ? 4 : 1))) {
      endGesture(false);
      return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
    if (drag) movePlayheadOrMarker(event, drag);
    else movePan(event);
  }

  function finishGesture(event: PointerEvent) {
    if (event.pointerId !== (drag || panDrag)?.pointer) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    endGesture(event.type === 'pointercancel');
  }

  function cancelGesture() {
    endGesture(true);
  }

  function endGesture(canceled: boolean) {
    const active = drag || panDrag;
    if (!active) return;
    releasedPointer = active.pointer;
    if (drag) endDrag(canceled);
    else finishPan(canceled);
    if (active.control.hasPointerCapture(active.pointer)) active.control.releasePointerCapture(active.pointer);
    ongesture?.(false);
  }

  function endDrag(canceled: boolean) {
    if (!drag) return;
    const { marker, loop, prior, moved, first, initialFirst, range } = drag;
    const frame = moveFrame ? pointerTarget : drag.frame;
    cancelAnimationFrame(moveFrame);
    cancelAnimationFrame(edgeFrame);
    moveFrame = 0;
    edgeFrame = 0;
    drag = null;
    if (canceled) {
      if (first !== initialFirst) onviewport?.(initialFirst, range);
      if (!marker && !loop) oncancel?.(prior);
      return;
    }
    if (loop) {
      if (moved && frame !== loop.anchor && (!loop.edge || frame !== prior)) onloop?.(orderedRange(loop.anchor, frame));
    } else if (marker) {
      if (!moved) onplaymarker?.(marker, prior);
      else if (frame !== prior) onmarker?.(marker, frame);
    }
    else onseek?.(frame);
  }

  function cancelByKey(event: KeyboardEvent) {
    if (event.key !== 'Escape' || (!drag && !panDrag)) return false;
    event.preventDefault();
    event.stopPropagation();
    cancelGesture();
    return true;
  }

  function sliderKeys(event: KeyboardEvent) {
    if (cancelByKey(event) || openMenuFromKey(event)) return;
    const delta = event.shiftKey ? 10 * sampleRate : sampleRate;
    const frames: Record<string, number> = {
      ArrowLeft: playhead - delta, ArrowDown: playhead - delta,
      ArrowRight: playhead + delta, ArrowUp: playhead + delta,
      Home: 0, End: total,
    };
    if (event.key in frames && !event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      onseek?.(Math.max(0, Math.min(total, frames[event.key])));
    } else onkey?.(event);
  }

  function markerKeys(event: KeyboardEvent, track: Track) {
    if (cancelByKey(event) || openMenuFromKey(event)) return;
    // Native button activation handles Enter and Space. Keep Shift+Space for looping.
    if (event.key === 'Enter' || (event.key === ' ' && !event.shiftKey)) return;
    if (!event.altKey && !event.ctrlKey && !event.metaKey && ['ArrowLeft', 'ArrowRight'].includes(event.key)) {
      event.preventDefault();
      const step = event.shiftKey ? Math.max(1, Math.round(sampleRate / 100)) : 1;
      onmarker?.(track.id, track.startFrame + (event.key === 'ArrowLeft' ? -step : step));
    } else onkey?.(event);
  }

  function loopHandleKeys(event: KeyboardEvent, edge: LoopEdge) {
    if (cancelByKey(event) || !loopRegion) return;
    const direction = event.key === 'ArrowLeft' || event.key === 'ArrowDown' ? -1
      : event.key === 'ArrowRight' || event.key === 'ArrowUp' ? 1 : 0;
    if (!direction || event.altKey || event.ctrlKey || event.metaKey) { onkey?.(event); return; }
    event.preventDefault();
    const step = event.shiftKey ? Math.max(1, Math.round(sampleRate / 100)) : 1;
    const { start, end } = loopRegion;
    const next = loopRegion[edge] + direction * step;
    const frame = edge === 'start' ? Math.max(0, Math.min(end - 1, next)) : Math.max(start + 1, Math.min(total, next));
    if (frame !== loopRegion[edge]) onloop?.({ ...loopRegion, [edge]: frame });
  }

  function queueViewport(first: number, size: number) {
    pendingView = { start: Math.max(0, Math.min(total - size, first)), span: size };
    if (!viewFrame) viewFrame = requestAnimationFrame(() => {
      viewFrame = 0;
      const view = pendingView;
      pendingView = null;
      if (view) onviewport?.(view.start, view.span);
    });
  }

  function zoomWheel(event: WheelEvent) {
    if (readonly || (!event.deltaX && !event.deltaY)) return;
    if (drag || panDrag || menuOpen) { event.preventDefault(); return; }
    const factor = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? detailWidth : 1;
    const view = pendingView || { start, span };
    const pan = event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY);
    if (pan) {
      if (view.span >= total) return;
      event.preventDefault();
      const delta = (event.shiftKey ? event.deltaY || event.deltaX : event.deltaX) * factor;
      queueViewport(view.start + delta * view.span / Math.max(1, detailWidth), view.span);
      return;
    }
    event.preventDefault();
    if (!event.deltaY) return;
    const delta = Math.max(-350, Math.min(350, event.deltaY * factor));
    onwheelzoom?.(delta, pointerFraction(event.clientX, surface.getBoundingClientRect()));
  }

  function startOverview(event: PointerEvent) {
    if (event.button !== 0 || panDrag || drag) return;
    const control = event.currentTarget as HTMLDivElement;
    if (span >= total) { begin(event, undefined, control, 0, total); return; }
    const { left, width } = control.getBoundingClientRect();
    const at = Math.round(total * pointerFraction(event.clientX, { left, width }));
    event.preventDefault();
    control.focus({ preventScroll: true });
    capturePointer(control, event.pointerId);
    ongesture?.(true);
    const offset = at >= start && at <= end ? at - start : span / 2;
    panDrag = { pointer: event.pointerId, control, start, offset, span, left, width, middle: false };
    queueViewport(at - offset, span);
  }

  function startMiddlePan(event: PointerEvent) {
    if (readonly || event.button !== 1) return;
    event.preventDefault();
    if (drag || panDrag || span >= total) return;
    surface.querySelector<HTMLElement>('.scrubber')?.focus({ preventScroll: true });
    capturePointer(surface, event.pointerId);
    ongesture?.(true);
    const { left, width } = surface.getBoundingClientRect();
    panDrag = { pointer: event.pointerId, control: surface, start, offset: event.clientX, span, left, width, middle: true };
  }

  function movePan(event: PointerEvent) {
    if (!panDrag) return;
    const { middle, start, offset, span, width } = panDrag;
    const first = middle
      ? start + (offset - event.clientX) * span / Math.max(1, width)
      : Math.round(total * pointerFraction(event.clientX, panDrag)) - offset;
    queueViewport(first, span);
  }

  function finishPan(canceled: boolean) {
    if (!panDrag) return;
    if (canceled) queueViewport(panDrag.start, panDrag.span);
    panDrag = null;
  }

  function prepareMenu(event: MouseEvent) {
    event.preventDefault();
    if (readonly || drag || panDrag) {
      return;
    }
    const marker = (event.target as HTMLElement)?.closest<HTMLElement>('[data-cut-id]');
    const track = tracks.find(track => track.id === marker?.dataset.cutId);
    const frame = Math.round(start + span * pointerFraction(event.clientX, surface.getBoundingClientRect()));
    menuTarget = track ? { frame: track.startFrame, marker: track.id } : { frame };
    menuReturn = marker ?? surface.querySelector<HTMLElement>('.scrubber');
    editingFromMenu = false;
    void menuController.open({ anchor: { x: event.clientX, y: event.clientY }, event });
  }

  function openMenuFromKey(event: KeyboardEvent) {
    if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return false;
    event.preventDefault();
    const control = event.currentTarget as HTMLElement;
    const rect = control.getBoundingClientRect();
    const wave = surface.getBoundingClientRect();
    const fraction = Math.max(0, Math.min(1, (playhead - start) / span));
    const clientX = control.dataset.cutId ? rect.left + rect.width / 2 : wave.left + wave.width * fraction;
    control.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX, clientY: rect.top + rect.height / 2 }));
    return true;
  }

  async function closeMenuFocus(open: boolean) {
    if (open || editingFromMenu) return;
    const trigger = menuReturn;
    await tick();
    if (menuOpen || !surface.isConnected) return;
    const target = trigger?.isConnected ? trigger : surface.querySelector<HTMLElement>('.scrubber');
    target?.focus({ preventScroll: true });
  }

  function menuKeys(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    // The surrounding dialog has its own Escape listener. Close the menu first.
    event.preventDefault();
    event.stopPropagation();
    menuController.close({ event });
  }

  function overviewKeys(event: KeyboardEvent) {
    if (cancelByKey(event)) return;
    if (event.altKey || event.metaKey || event.ctrlKey) return;
    const step = Math.max(1, Math.round(span / (event.shiftKey ? 2 : 10)));
    const from = pendingView?.start ?? start;
    const destinations: Record<string, number> = {
      ArrowLeft: from - step, ArrowDown: from - step, ArrowRight: from + step, ArrowUp: from + step,
      Home: 0, End: total - span,
    };
    if (event.key in destinations) {
      event.preventDefault();
      queueViewport(destinations[event.key], span);
    } else onkey?.(event);
  }

  function wheelControl(node: HTMLDivElement) {
    surface = node;
    node.addEventListener('wheel', zoomWheel, { passive: false });
    return () => node.removeEventListener('wheel', zoomWheel);
  }
</script>

<div class="audio-split-wave" {@attach observe}>
  {#if !readonly && trackHeaders.length}
    <div class="track-headers" aria-label="Timeline tracks" role="group">
      {#each trackHeaders as region (region.track.id)}
        {@const title = trackTitle?.(region.track) || ''}
        <Button type="button" class="track-header" aria-pressed={region.track.id === selected}
          aria-label={title ? `Go to track ${region.index + 1}, ${title}` : `Go to track ${region.index + 1}`}
          title={title || `Track ${region.index + 1}`}
          style={`left:${Math.max(0, percent(region.start))}%;right:${Math.max(0, 100 - percent(region.end))}%`}
          onclick={() => ontrack?.(region.track)}>
          <span>{String(region.index + 1).padStart(2, '0')}</span>{#if title}<span>{title}</span>{/if}
        </Button>
      {/each}
    </div>
  {/if}
  <Menu controller={menuController} bind:open={menuOpen} onOpenChangeComplete={closeMenuFocus}>
        <!-- The waveform owns pointer gestures, including touch scrubbing. -->
        <div class={['detail', { panning: panDrag?.middle, 'stereo-detail': stereo }]} role="group" aria-label="Waveform detail" {@attach wheelControl}
          onpointerdowncapture={startMiddlePan} oncontextmenu={prepareMenu}
          onauxclick={event => { if (event.button === 1) event.preventDefault(); }}>
          {#if selectedHeader}
            <div class="selected-region" style:left={`${Math.max(0, percent(selectedHeader.start))}%`}
              style:right={`${Math.max(0, 100 - percent(selectedHeader.end))}%`} aria-hidden="true"></div>
          {/if}
          <canvas bind:this={detail} aria-hidden="true"></canvas>
          {#if stereo}
            <div class="channel-label left">Left</div>
            <div class="channel-label right">Right</div>
          {/if}
          {#if !readonly}<div class="scrubber" role="slider" tabindex="0" aria-label="Waveform playhead"
            aria-valuemin={0} aria-valuemax={total} aria-valuenow={announceFrame} aria-valuetext={timeLabel(announceFrame, sampleRate)}
            onkeydown={sliderKeys} onpointerdown={event => begin(event)}></div>{/if}
          {#if !overview.length && !tile?.values.length}<span class="hint">Preparing waveform…</span>{/if}
          {#if opening > start}
            <div class="trimmed-opening" style:width={`${Math.min(100, percent(opening))}%`} aria-hidden="true"></div>
          {/if}
          {#if shownLoop && shownLoop.start < shownLoop.end && shownLoop.start < end && shownLoop.end > start}
            <div class="loop-region" data-active={loopEnabled || !!drag?.loop}
              style:left={`${Math.max(0, percent(shownLoop.start))}%`} style:right={`${Math.max(0, 100 - percent(shownLoop.end))}%`} aria-hidden="true">
              {#if percent(shownLoop.end) - percent(shownLoop.start) > 8}<span>Loop</span>{/if}
            </div>
          {/if}
          {#each visible as { track, index } (track.id)}
            {@const position = drag?.marker === track.id ? drag.frame : track.startFrame}
            <Button class={['marker', { selected: track.id === selected }]} type="button"
              style={`left:${percent(position)}%`}
              data-cut-id={track.id} data-dragging={drag?.marker === track.id}
              aria-label={`${index === 0 ? 'Start of Track 1' : 'Cut'} at ${timeLabel(track.startFrame, sampleRate)}`} aria-pressed={track.id === selected}
              aria-haspopup="menu"
              onclick={event => { if (event.detail === 0) onplaymarker?.(track.id, track.startFrame); }}
              onkeydown={event => markerKeys(event, track)}
              onpointerdown={event => begin(event, track)}>
              <span class="marker-line" aria-hidden="true"></span>
            </Button>
            <span class="marker-time" data-dragging={drag?.marker === track.id}
              style:left={`clamp(45px, ${percent(position)}%, calc(100% - 45px))`} aria-hidden="true">{timeLabel(position, sampleRate)}</span>
          {/each}
          {#if shownLoop && shownLoop.start < shownLoop.end}
            {#each ['start', 'end'] as edge (edge)}
              {@const key = edge as LoopEdge}
              {@const frame = shownLoop[key]}
              {#if frame >= start && frame <= end && (loopRegion || drag?.loop?.edge)}
                <div class="loop-handle" data-edge={edge} role="slider" tabindex="0" aria-label={`Loop ${edge}`}
                  aria-valuemin={key === 'start' ? 0 : shownLoop.start + 1} aria-valuemax={key === 'start' ? shownLoop.end - 1 : total}
                  aria-valuenow={frame} aria-valuetext={timeLabel(frame, sampleRate)}
                  style:left={`${percent(frame)}%`}
                  onkeydown={event => loopHandleKeys(event, key)} onpointerdown={event => begin(event, undefined, surface, start, span, key)}></div>
              {/if}
            {/each}
          {/if}
          {#if !readonly && (shownPlayhead >= start && shownPlayhead <= end || shownPlayhead !== playhead)}
            <div class="playhead" style:transform={`translateX(${Math.min(100, Math.max(0, percent(shownPlayhead))) / 100 * detailWidth}px)`} aria-hidden="true"></div>
          {/if}
        </div>
    <MenuContent class="app-select-menu studio-wave-context" finalFocus="none" aria-label="Waveform commands" onkeydowncapture={menuKeys}>
      {#if menuTarget}
        {@const target = menuTarget}
        {#if target.marker}
          {@const id = target.marker}
          {#if id === firstTrack?.id}
            <MenuItem onSelect={() => { editingFromMenu = true; oneditmarker?.(id); }}>Edit start time</MenuItem>
            <MenuItem disabled={!firstTrack.startFrame} onSelect={() => ontrim?.(0)}>Reset start to beginning</MenuItem>
          {:else}
            <MenuItem onSelect={() => { editingFromMenu = true; oneditmarker?.(id); }}>Edit cut time</MenuItem>
            <MenuItem onSelect={() => onremovemarker?.(id)}>Remove cut</MenuItem>
          {/if}
          {#if tracks.some(track => track.id === id && !track.include)}
            <MenuItem onSelect={() => onrestoremarker?.(id)}>Restore track</MenuItem>
          {/if}
          {#if onmovetoplayhead}
            <MenuItem disabled={!canmove?.(id)} onSelect={() => onmovetoplayhead(id)}>Move split to playhead</MenuItem>
          {/if}
        {:else}
          <MenuItem disabled={!firstTrack || target.frame <= firstTrack.startFrame || target.frame >= total || tracks.some(track => track.startFrame === target.frame)}
            onSelect={() => onadd?.(target.frame)}>Add cut here</MenuItem>
          {#if firstTrack && target.frame < (tracks[1]?.startFrame ?? total)}
            <MenuItem disabled={target.frame === firstTrack.startFrame} onSelect={() => ontrim?.(target.frame)}>Start Track 1 here</MenuItem>
          {/if}
        {/if}
      {/if}
    </MenuContent>
  </Menu>
  <div class="ruler" aria-hidden="true">
    {#each [0, 1, 2, 3, 4] as division (division)}
      <span>{timeLabel(start + span * division / 4, sampleRate, span < sampleRate * 12 ? 1 : 0)}</span>
    {/each}
  </div>
  <div class="overview">
    <canvas bind:this={mini} aria-hidden="true"></canvas>
    <div class="window" style:left={`${start / Math.max(1,total) * 100}%`} style:width={`${span / Math.max(1,total) * 100}%`}></div>
    {#if !readonly}<div class="overview-pan" role="slider" tabindex="0" aria-label="Waveform overview"
      title="Drag to move the visible range" aria-valuemin={0} aria-valuemax={Math.max(0,total - span)} aria-valuenow={start}
      aria-valuetext={`View centred at ${timeLabel(center, sampleRate)}`}
      onpointerdown={startOverview}
      onkeydown={overviewKeys}></div>{/if}
  </div>
</div>

<style>
  .audio-split-wave { position:relative; display:grid; gap:var(--sp-1); }
  .track-headers { position:relative; height:28px; overflow:hidden; background:var(--panel); border-radius:var(--r-media) var(--r-media) 0 0; margin-bottom:calc(-1 * var(--sp-1)); }
  .track-headers :global(.track-header) { position:absolute; inset-block:0; display:flex; align-items:center; gap:var(--sp-1); overflow:hidden; padding:0; border:0; border-radius:0; font-size:var(--text-xs); color:var(--muted); background:transparent; white-space:nowrap; }
  .track-headers :global(.track-header span:first-child) { margin-inline-start:var(--sp-2); font-family:var(--font-literal); }
  .track-headers :global(.track-header span:last-child) { overflow:hidden; text-overflow:ellipsis; }
  .track-headers :global(.track-header[aria-pressed=true]) { color:var(--text); background:var(--sel-tint); }
  .track-headers :global(.track-header:hover) { color:var(--text); background:var(--row-hover); }
  .track-headers :global(.track-header:focus-visible) { color:var(--text); outline:2px solid var(--accent); outline-offset:-2px; }
  .track-headers + .detail { border-start-start-radius:0; border-start-end-radius:0; }
  .selected-region { position:absolute; inset-block:0; background:color-mix(in srgb,var(--accent) 6%,transparent); pointer-events:none; }
  .channel-label { position:absolute; inset-inline-start:var(--sp-2); color:var(--text); font-size:var(--text-xs); pointer-events:none; }
  .channel-label.left { inset-block-start:var(--sp-1); }
  .channel-label.right { inset-block-start:calc(50% + var(--sp-1)); }
  .overview, .detail { position:relative; overflow:hidden; border-radius:var(--r-media); }
  .overview { height:20px; margin-top:var(--sp-1); background:var(--panel); }
  .overview-pan, .scrubber { position:absolute; inset:0; width:100%; background:transparent; border:0; border-radius:inherit; touch-action:none; }
  .scrubber { cursor:crosshair; }
  .panning :is(.scrubber, :global(.marker)) { cursor:grabbing; }
  .overview-pan { cursor:grab; }
  .overview-pan:active { cursor:grabbing; }
  .overview-pan:focus-visible, .scrubber:focus-visible { outline:2px solid var(--accent); outline-offset:-3px; }
  .window { position:absolute; inset-block:1px; background:color-mix(in srgb,var(--accent) 15%,transparent); border:1px solid var(--accent); border-radius:4px; pointer-events:none; }
  .detail { height:clamp(112px, 24dvh, 208px); background:var(--panel); }
  .detail.stereo-detail { height:clamp(152px, 31dvh, 256px); }
  canvas { width:100%; height:100%; display:block; color:var(--muted); }
  .detail canvas { position:relative; }
  .overview canvas { opacity:.76; }
  .hint { position:absolute; inset:0; display:grid; place-items:center; pointer-events:none; color:var(--muted); }
  .loop-region { position:absolute; inset-block:0; pointer-events:none; background:color-mix(in srgb,var(--accent) 9%,transparent); }
  .loop-region[data-active=true] { background:color-mix(in srgb,var(--accent) 12%,transparent); border-inline:1px solid color-mix(in srgb,var(--accent) 65%,transparent); }
  .loop-region span { margin-inline-start:var(--sp-1); color:var(--text); font-size:var(--text-xs); }
  .loop-handle { position:absolute; bottom:1px; z-index:2; width:18px; height:24px; translate:-50% 0; cursor:ew-resize; touch-action:none; }
  .loop-handle[data-edge=start] { translate:-30% 0; }
  .loop-handle[data-edge=end] { translate:-70% 0; }
  .loop-handle::after { content:''; position:absolute; inset:4px 5px; border:1px solid var(--accent); border-radius:var(--r-compact); background:var(--panel); }
  .loop-handle:is(:hover,:focus-visible)::after { background:var(--accent); }
  .trimmed-opening { position:absolute; inset-block:0; left:0; background:color-mix(in srgb,var(--panel) 68%,transparent); pointer-events:none; }
  .detail :global(.marker) { position:absolute; top:0; height:100%; width:24px; transform:translateX(-50%); background:transparent; border:0; padding:0; cursor:pointer; touch-action:none; }
  .detail :global(.marker[data-dragging=true]) { cursor:ew-resize; }
  .marker-line { position:absolute; top:3px; bottom:0; left:50%; width:1px; background:var(--muted); }
  .marker-line::before { content:''; position:absolute; top:-3px; left:-3px; width:7px; height:7px; background:inherit; clip-path:polygon(0 0,100% 0,100% 50%,50% 100%,0 50%); }
  .detail :global(.marker.selected .marker-line) { background:var(--accent); width:2px; }
  .detail :global(.marker:is(:hover, :focus-visible, [data-dragging=true]) .marker-line) { background:color-mix(in srgb, var(--accent) 55%, var(--text)); width:2px; }
  .marker-time { position:absolute; bottom:var(--sp-2); z-index:1; translate:-50% 0; border-radius:var(--r-compact); padding:var(--sp-1) var(--sp-2); color:var(--text); background:var(--bg); font:var(--text-xs)/1.4 var(--font-literal); pointer-events:none; visibility:hidden; }
  .detail :global(.marker:is(:hover, :focus-visible) + .marker-time), .marker-time[data-dragging=true] { visibility:visible; }
  .detail :global(.marker:focus-visible) { outline:2px solid var(--accent); outline-offset:-2px; }
  :global(.app-select-menu.studio-wave-context [data-slot='menu-item']) { display:flex; align-items:center; min-height:32px; border-radius:var(--r-menu-item); padding:var(--sp-1) var(--sp-3); cursor:default; outline:none; }
  .playhead { position:absolute; inset-block:0; left:0; width:1px; pointer-events:none; background:var(--text); }
  .playhead::before { content:''; position:absolute; top:0; left:-4px; border:4px solid transparent; border-top-color:var(--text); }
  .ruler { display:flex; justify-content:space-between; color:var(--muted); font:var(--text-xs)/1.4 var(--font-literal); }
</style>
