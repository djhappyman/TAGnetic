<script module lang="ts">
  export type RepairPlotControls = {
    reset: () => void;
    follow: () => void;
    clear: () => void;
    audition: () => void;
    enabled: boolean;
    following: boolean;
    hasSelection: boolean;
  };
</script>

<script lang="ts">
  import { onDestroy, type Snippet } from 'svelte';
  import { devicePixelRatio } from 'svelte/reactivity/window';
  import { Check, Headphones, Layers, LocateFixed, RotateCcw, Trash2, X } from '@lucide/svelte';
  import { Button as PlotButton } from '@ui-sdk/svelte/button';
  import { createMenuController, Menu, MenuContent, MenuItem, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';
  import Button from './AudioSplitButton.svelte';
  import { plotBounds, type PlotBounds, type TreatmentProbe } from '../js/audio-treatment-plot';
  import {
    boundedTime, buildRepairTexture, clampRepair, drawRepairViewport, frameAtX, frameX, frequencyAtY,
    frequencyY, recordingFrequencyRange, repairTimeLabel, spectrogramRange,
    type RepairTexture, type RepairViewport,
  } from '../js/audio-repair-plot';
  import { maxSpectralRepairs, sameRepairBounds, type SpectralRepair, type AudioTreatment } from '../js/audio-treatment';
  import type { ToolAction, ToolSnapshot } from '../js/audio-tools-window';
  import type { FrameRange } from '../js/audio-split';

  let { snapshot, probe, onviewport, onedit, onaction, controls }: {
    snapshot: ToolSnapshot | null;
    probe: TreatmentProbe | null;
    onviewport: (range: FrameRange | null) => void;
    onedit: (patch: Partial<AudioTreatment>, gesture?: string) => void;
    onaction: (action: ToolAction, range?: FrameRange) => void;
    controls?: Snippet<[RepairPlotControls]>;
  } = $props();

  type Pick = Omit<SpectralRepair, 'amountDb'> & { index?: number; saved?: SpectralRepair };
  type Gesture = {
    pointer: number; node: HTMLButtonElement; rect: DOMRect; bounds: PlotBounds;
    firstX: number; firstY: number; x: number; y: number; mode: 'pick' | 'pan'; moved: boolean;
    origin: RepairViewport; previous: RepairViewport | null;
  };
  const defaultAmount = 12;
  const editLimit = maxSpectralRepairs;
  const editMenu = createMenuController();
  const plotHelpID = $props.id();
  const noRepairs: readonly (SpectralRepair | null)[] = [];
  let width = $state(0);
  let height = $state(0);
  let shown = $state.raw<RepairViewport>();
  let manual = $state.raw<RepairViewport | null>(null);
  let selection = $state.raw<Pick>();
  let draft = $state.raw<Pick>();
  let amount = $state<number | null>(defaultAmount);
  let moving = $state<'pick' | 'pan'>();
  let interaction: Gesture | undefined;
  let viewFrame = 0;
  let gestureFrame = 0;
  let viewportTimer: ReturnType<typeof setTimeout> | undefined;
  let requestedTime: FrameRange | null = null;
  let previousAutomatic: RepairViewport | undefined;
  let textureHistory: RepairTexture[] = [];
  let textureColor = '';

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
  const selected = $derived(moving === 'pick' ? draft ?? selection : selection);
  const appearance = $derived(snapshot?.theme);
  const settings = $derived(snapshot?.settings);
  const repairs = $derived(settings?.spectralRepairs ?? noRepairs);
  const attached = $derived(repairs.flatMap((repair, index) => repair ? [{ repair, index }] : []));
  const active = $derived.by(() => {
    if (selection?.index === undefined || !selection.saved) return undefined;
    const edit = repairs[selection.index];
    return edit && sameRepairBounds(edit, selection.saved) ? edit : undefined;
  });
  const freeSlot = $derived(repairs.findIndex(value => !value));
  const nextSlot = $derived(freeSlot >= 0 ? freeSlot : repairs.length);
  const validPick = $derived(Boolean(selection && selection.endFrame > selection.startFrame && selection.highHz - selection.lowHz >= 20));
  const viewportReady = $derived(!!snapshot);
  const guides = $derived.by(() => {
    const hum = settings?.hum === 'hz50' ? 50 : settings?.hum === 'hz60' ? 60 : 0;
    const result = hum ? Array.from({ length: settings?.humHarmonics || 1 }, (_, index) => hum * (index + 1)) : [];
    if (settings?.notchHz) result.push(settings.notchHz);
    return result;
  });

  function paint(canvas: HTMLCanvasElement) {
    void appearance;
    const scale = Math.min(3, devicePixelRatio.current || 1);
    const style = getComputedStyle(canvas);
    const accent = style.getPropertyValue('--accent').trim() || '#5c88e1';
    if (accent !== textureColor) {
      textureColor = accent;
      textureHistory = [];
    }
    if (probe && measuredTime) {
      const known = textureHistory.findIndex(item => item.time.start === measuredTime.start && item.time.end === measuredTime.end);
      if (known < 0) {
        const texture = buildRepairTexture(probe, rate, accent);
        if (texture) textureHistory = [...textureHistory.slice(-2), texture];
      } else if (known < textureHistory.length - 1) {
        textureHistory.push(...textureHistory.splice(known, 1));
      }
    }
    drawRepairViewport(canvas, width, height, visible, textureHistory, rate, {
      ink: style.getPropertyValue('--muted').trim() || '#aeb7d0',
      grid: style.getPropertyValue('--border').trim() || '#3d435b',
      guide: style.getPropertyValue('--accent-ink').trim() || accent,
      font: style.getPropertyValue('--font-literal').trim() || 'monospace',
    }, guides, scale);
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

  function requestViewport(time: FrameRange | null) {
    clearTimeout(viewportTimer);
    viewportTimer = undefined;
    const range = time ? { start: Math.round(time.start), end: Math.round(time.end) } : null;
    if (requestedTime?.start === range?.start && requestedTime?.end === range?.end) return;
    requestedTime = range;
    onviewport(range);
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
    resume();
  }

  function audition() {
    if (selection) onaction('loop', { start: selection.startFrame, end: selection.endFrame });
  }

  function zoomAt(event: WheelEvent) {
    if (!viewportReady || interaction || event.deltaY === 0) return;
    if (event.target instanceof Element && event.target.closest('.repair-tools')) return;
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

  function begin(event: PointerEvent) {
    if (!viewportReady || interaction || (event.button !== 0 && event.button !== 1)) return;
    const node = event.currentTarget as HTMLButtonElement;
    const rect = node.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    if (x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom) return;
    const origin = visible;
    interaction = {
      pointer: event.pointerId, node, rect, bounds, firstX: x, firstY: y, x, y,
      mode: event.button === 1 ? 'pan' : 'pick', moved: false, origin, previous: manual,
    };
    if (event.button === 1) event.preventDefault();
    setManual(origin);
    moving = interaction.mode;
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
      const time = boundedTime(origin.time.start - (x - firstX) * span / (gesture.bounds.right - gesture.bounds.left), span, rate, frames);
      const logFull = Math.log(fullFrequency.highHz / fullFrequency.lowHz);
      const band = Math.log(origin.frequency.highHz / origin.frequency.lowHz);
      const logLow = Math.log(fullFrequency.lowHz);
      const movedBy = (y - firstY) * band / (gesture.bounds.bottom - gesture.bounds.top);
      const low = clampRepair(Math.log(origin.frequency.lowHz) + movedBy, logLow, logLow + logFull - band);
      setManual({ time, frequency: { lowHz: Math.exp(low), highHz: Math.exp(low + band) } });
      scheduleViewport();
      return;
    }
    // The starting point belongs to the view at pointer-down. The moving
    // endpoint projects through the new geometry if the window is resized.
    const start = Math.round(frameAtX(firstX, gesture.bounds, origin.time));
    const end = Math.round(frameAtX(x, bounds, origin.time));
    const first = Math.round(frequencyAtY(firstY, gesture.bounds, origin.frequency));
    const last = Math.round(frequencyAtY(y, bounds, origin.frequency));
    draft = { startFrame: Math.min(start, end), endFrame: Math.max(start, end), lowHz: Math.min(first, last), highHz: Math.max(first, last) };
  }

  function move(event: PointerEvent) {
    const gesture = interaction;
    if (!gesture || gesture.pointer !== event.pointerId) return;
    updatePoint(gesture, event);
    if (gestureFrame || !gesture.moved) return;
    gestureFrame = requestAnimationFrame(() => {
      gestureFrame = 0;
      if (interaction === gesture) paintGesture(gesture);
    });
  }

  function chooseExisting(pick: SpectralRepair, index: number, center = false) {
    selection = { ...pick, index, saved: pick };
    amount = pick.amountDb;
    if (center) {
      const span = Math.min(rate * 12, Math.max(rate * 3.5, 1.2 * (pick.endFrame - pick.startFrame)));
      const centerFrame = (pick.startFrame + pick.endFrame) / 2;
      const time = boundedTime(centerFrame - span / 2, span, rate, frames);
      setManual({ time, frequency: fullFrequency }, 160);
      requestViewport(time);
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
      if (gesture.mode === 'pick' && draft && draft.endFrame > draft.startFrame) {
        selection = draft;
        amount = defaultAmount;
      } else if (gesture.mode === 'pick' && !gesture.moved) {
        const time = frameAtX(gesture.x, bounds, gesture.origin.time);
        const hz = frequencyAtY(gesture.y, bounds, gesture.origin.frequency);
        const found = attached.find(({ repair }) =>
          time >= repair.startFrame && time <= repair.endFrame && hz >= repair.lowHz && hz <= repair.highHz);
        if (found) chooseExisting(found.repair, found.index);
        else {
          shown = gesture.origin;
          manual = gesture.previous;
        }
      } else if (!gesture.moved) {
        shown = gesture.origin;
        manual = gesture.previous;
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
    if (gesture.node.hasPointerCapture(gesture.pointer)) gesture.node.releasePointerCapture(gesture.pointer);
  }

  function end(event: PointerEvent) {
    if (event.pointerId === interaction?.pointer) finish(event.type === 'pointerup' ? event : undefined);
  }

  function keys(event: KeyboardEvent) {
    if (!viewportReady || interaction || event.ctrlKey || event.metaKey || event.altKey) return;
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Enter', 'Escape'].includes(event.key)) return;
    event.preventDefault();
    if (event.key === 'Escape') { clear(); return; }
    if (event.key === 'Enter') {
      if (selection) audition();
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

  function apply() {
    if (!selection || !validPick || interaction) return;
    const index = active && selection.index !== undefined ? selection.index : nextSlot;
    if (index >= editLimit) return;
    const updated = [...repairs];
    const { startFrame, endFrame, lowHz, highHz } = selection;
    updated[index] = { startFrame, endFrame, lowHz, highHz, amountDb: amount ?? defaultAmount };
    onedit({ spectralRepairs: updated });
    selection = { ...selection, index, saved: updated[index] };
  }

  function remove() {
    const index = selection?.index;
    if (index === undefined || !active) return;
    const updated = repairs.map((value, slot) => slot === index ? null : value);
    onedit({ spectralRepairs: updated.some(Boolean) ? updated : null });
    clear();
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
      + ' · ' + pick.lowHz + '–' + pick.highHz + ' Hz';
  }

  onDestroy(() => {
    if (interaction) finish(undefined, false);
    cancelView();
    clearTimeout(viewportTimer);
  });
</script>

<svelte:window onblur={() => finish()}/>

<div class="repair-layout">
  <div class={['repair-plot', { panning: moving === 'pan' }]} bind:clientWidth={width} bind:clientHeight={height}
    aria-label="Source spectrogram" role="group" {@attach wheelBinding} {@attach followBinding}>
    <span id={plotHelpID} class="sr-only">Scroll to zoom in time or Shift-scroll for frequency. Middle-drag to pan.
      Left and Right adjust the start time. Up and Down adjust the low frequency.
      Hold Shift to adjust the other edge. Enter auditions and Escape clears.</span>
    <span class="sr-only" role="status">{selection ? editLabel(selection) : ''}</span>
    <canvas aria-hidden="true" {@attach paint}></canvas>
    <PlotButton class="repair-hit" type="button" disabled={!viewportReady}
      aria-label="Select time and frequency" aria-describedby={plotHelpID}
      onpointerdown={begin} onpointermove={move} onpointerup={end} onpointercancel={end} onlostpointercapture={end}
      onauxclick={event => event.preventDefault()} onkeydown={keys}/>
    {#if snapshot && snapshot.frame >= visible.time.start && snapshot.frame <= visible.time.end}
      <span class="playhead" aria-hidden="true" style:left={frameX(snapshot.frame, bounds, visible.time) + 'px'}
        style:top={bounds.top + 'px'} style:bottom={height - bounds.bottom + 'px'}></span>
    {/if}
    {#each attached as { repair, index } (index)}
      {@const position = boxStyle(repair)}
      {#if position}<span class="applied" aria-hidden="true" style={position}></span>{/if}
    {/each}
    {#if selected}
      {@const position = boxStyle(selected)}
      {#if position}<span class="picked" aria-hidden="true" style={position}></span>{/if}
    {/if}
    <div class="repair-tools">
      {#if selection && !moving}
        <div class="attenuation" aria-label="Frequency attenuation" role="group">
          <NumberInput class="repair-number" bind:value={amount} min={1} max={36} step={1}>
            <NumberInputControl class="repair-amount" aria-label="Attenuation in dB" title="Attenuation in dB" placeholder={String(defaultAmount)}/>
            <span aria-hidden="true">dB</span>
          </NumberInput>
          <Button variant="ghost" size="sm" disabled={!validPick || (!active && nextSlot >= editLimit)}
            title={!validPick ? 'Select at least 20 Hz' : !active && nextSlot >= editLimit ? 'Eight repairs are allowed' : 'Apply frequency attenuation'}
            onclick={apply}><Check size={14} aria-hidden="true"/>{active ? 'Update' : 'Apply'}</Button>
          {#if active}
            <Button variant="ghost" size="icon-sm" aria-label="Remove repair" title="Remove repair" onclick={remove}><Trash2 size={14} aria-hidden="true"/></Button>
          {/if}
        </div>
      {/if}
      {#if attached.length}
        <Menu controller={editMenu}>
          <MenuTrigger class="repair-edits" aria-label="Applied repairs" title="Applied repairs"><Layers size={15} aria-hidden="true"/></MenuTrigger>
          <MenuContent class="app-select-menu" align="end" sideOffset={6} collisionPadding={8}>
            {#each attached as { repair, index } (index)}
              {@const label = editLabel(repair)}
              <MenuItem data-slot="select-item" {label} onSelect={() => chooseExisting(repair, index, true)}>{label}</MenuItem>
            {/each}
          </MenuContent>
        </Menu>
      {/if}
    </div>
  </div>
  {#if controls}
    {@render controls({ reset, follow: toggleFollow, clear, audition, enabled: viewportReady, following: !manual, hasSelection: Boolean(selection) })}
  {:else}
    <div class="view-controls" aria-label="Spectrogram controls" role="group">
      <Button variant="ghost" size="icon-sm" aria-label="Follow playhead" title="Follow playhead" aria-pressed={!manual} disabled={!viewportReady} onclick={toggleFollow}>
        <LocateFixed size={15} aria-hidden="true"/>
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label="Reset spectrogram zoom" title="Reset spectrogram zoom" disabled={!viewportReady} onclick={reset}>
        <RotateCcw size={15} aria-hidden="true"/>
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label="Audition selected area" title="Audition selected area" disabled={!selection} onclick={audition}>
        <Headphones size={15} aria-hidden="true"/>
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label="Clear selected area and follow playhead" title="Clear selected area and follow playhead"
        disabled={!selection} onclick={clear}><X size={15} aria-hidden="true"/></Button>
    </div>
  {/if}
</div>

<style>
  .repair-layout {
    min-width: 0;
    min-height: 218px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--sp-2, 8px);
  }
  .repair-plot {
    position: relative;
    isolation: isolate;
    overflow: hidden;
    overflow: clip;
    min-height: 218px;
    border-radius: var(--r-inset, 14px);
    background: var(--panel);
  }
  canvas, .repair-plot :global(.repair-hit) {
    position: absolute;
    width: 100%;
    height: 100%;
    inset: 0;
    border-radius: inherit;
  }
  canvas { pointer-events: none; }
  .repair-plot :global(.repair-hit) {
    appearance: none;
    background: transparent;
    border: 0;
    padding: 0;
    cursor: crosshair;
    touch-action: none;
  }
  .repair-plot :global(.repair-hit:focus-visible) { outline: 2px solid var(--accent); outline-offset: -2px; }
  .repair-plot :global(.repair-hit:disabled) { cursor: default; }
  .panning :global(.repair-hit) { cursor: grabbing; }
  .playhead, .applied, .picked { position: absolute; pointer-events: none; }
  .playhead { width: 1px; background: var(--accent-ink, var(--text)); opacity: .84; }
  .picked {
    border: 1px solid var(--accent-ink, var(--text));
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .applied {
    border: 1px dashed color-mix(in srgb, var(--accent-ink) 78%, transparent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .repair-tools {
    position: absolute;
    inset-block-end: 30px;
    inset-inline-end: 35px;
    display: flex;
    align-items: center;
    gap: var(--sp-2, 8px);
  }
  .attenuation, .repair-tools :global(.repair-edits) {
    display: flex;
    align-items: center;
    gap: 3px;
    padding: var(--sp-1, 4px);
    border: 0;
    border-radius: var(--r-control, 10px);
    color: var(--text);
    background: var(--bg);
    box-shadow: 0 3px 12px rgb(0 0 0 / 22%);
  }
  .repair-tools :global(.repair-edits) { justify-content: center; width: 29px; height: 29px; }
  .repair-tools :global(.repair-edits:hover) { color: var(--accent-ink); }
  .attenuation :global(.repair-number) {
    display: flex;
    align-items: center;
    padding: 0 6px;
    gap: 3px;
    font-size: .72rem;
    border-radius: var(--r-compact, 8px);
    background: var(--panel);
  }
  .attenuation :global(.repair-amount) {
    width: 3ch;
    color: var(--text);
    background: transparent;
    border: 0;
    padding: 5px 0;
    text-align: right;
    outline: none;
    font-variant-numeric: tabular-nums;
  }
  .attenuation :global(.repair-number:focus-within), .repair-tools :global(.repair-edits:focus-visible) {
    outline: 2px solid var(--accent);
  }
  .view-controls { display: flex; flex-direction: column; align-items: center; gap: var(--sp-1, 4px); }
  .view-controls :global([aria-pressed=true]) {
    color: var(--accent-ink);
    background: color-mix(in srgb, var(--accent) 16%, var(--panel));
  }
</style>
