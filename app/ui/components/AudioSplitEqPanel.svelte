<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { prefersReducedMotion } from 'svelte/motion';
  import { devicePixelRatio } from 'svelte/reactivity/window';
  import { ChevronRight, Headphones, Plus, Snowflake, Volume2, VolumeX } from '@lucide/svelte';
  import { Button as GraphButton } from '@ui-sdk/svelte/button';
  import { createMenuController, Menu, MenuContent, MenuItem, MenuSeparator, MenuShortcut, MenuSub, MenuSubContent, MenuSubTrigger, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { createSelectController, Select, SelectContent, SelectItem, SelectTrigger } from '@ui-sdk/svelte/select';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';
  import Button from './AudioSplitButton.svelte';
  import { maxEqBands, noTreatment, treatmentNumbers, type AudioTreatment } from '../js/audio-treatment';
  import {
    eqBandPatch, eqFrequencyRange, eqSoloRange, eqWidthRange, findEqNode, isLegacyEqBand,
    legacyEqBands, legacyEqNode, makeExtraEqNode, originalEqBand, removeEqBand,
    visibleEqNodes, type EqNode, type EqShape, type ExtraEqShape,
  } from '../js/audio-eq';
  import {
    createSpectrumMotion, drawTreatmentPlot, frequencyX, gainY, plotBounds, xFrequency, yGain,
    type PlotBounds, type TreatmentProbe,
  } from '../js/audio-treatment-plot';
  import type { FrameRange } from '../js/audio-split';
  import type { ToolAction, ToolSnapshot } from '../js/audio-tools-window';

  let { snapshot, probe, mode, onmode, onedit, onaction }: {
    snapshot: ToolSnapshot | null;
    probe: TreatmentProbe | null;
    mode: string | null;
    onmode: (mode: string | null) => void;
    onedit: (patch: Partial<AudioTreatment>, gesture?: string) => void;
    onaction: (action: ToolAction, range?: FrameRange, band?: { lowHz: number; highHz: number }) => void;
  } = $props();

  type Drag = {
    pointerId: number;
    control: HTMLButtonElement;
    sourceId: string;
    node: EqNode;
    before: Partial<AudioTreatment>;
    gesture: string;
    fromPlot: boolean;
    bounds: PlotBounds;
    x: number;
    y: number;
    lastX: number;
    lastY: number;
    targetX: number;
    targetY: number;
    moved: boolean;
    applied: boolean;
  };
  type Point = { x: number; y: number };

  const id = $props.id();
  const addMenu = createMenuController();
  const bandMenu = createMenuController();
  const slopeControl = createSelectController();
  const spectrum = createSpectrumMotion();
  const slopes = [12, 24, 36, 48];
  const shapes = Object.keys(legacyEqBands) as EqShape[];
  let root: HTMLElement;
  let width = $state(0);
  let height = $state(0);
  let ghost = $state.raw<EqNode>();
  let frozen = $state.raw<TreatmentProbe | null>(null);
  let soloed = $state<string | null>(null);
  let clipboard = $state.raw<EqNode>();
  let menuNodeKey = $state<string | null>(null);
  let menuPoint = $state<Point>();
  let menuReturn: HTMLElement | undefined;
  let menuEditsValue = false;
  let drag: Drag | undefined;
  let dragFrame = 0;
  let gestureNumber = 0;
  let moving = $state(false);
  let editorOnLeft = $state<boolean>();
  let heldKey: string | undefined;
  let heldPointer: number | undefined;
  let report = $state('');

  const settings = $derived(snapshot?.settings ?? noTreatment);
  const appearance = $derived(snapshot?.theme);
  const rate = $derived(snapshot?.sampleRate || 44100);
  const bounds = $derived(plotBounds(width, height));
  const nodes = $derived(visibleEqNodes(settings, mode));
  const selected = $derived(findEqNode(settings, mode));
  const menuNode = $derived(findEqNode(settings, menuNodeKey));
  const canAdd = $derived(Boolean(snapshot) && (settings.eqBands?.length ?? 0) < maxEqBands);

  function paint(canvas: HTMLCanvasElement) {
    void appearance;
    void devicePixelRatio.current;
    const chart = { width, height, settings, rate };
    return spectrum(frozen ?? probe, !!snapshot?.playing && !frozen && !prefersReducedMotion.current,
      measured => drawTreatmentPlot(canvas, chart.width, chart.height, 'eq', chart.settings, chart.rate, measured));
  }

  function supportsGain(node: EqNode) {
    return 'gain' in legacyEqBands[node.shape];
  }

  function position(node: EqNode) {
    const [min, max] = eqFrequencyRange(node, rate);
    const hz = Math.max(min, Math.min(node.hz, max));
    const db = supportsGain(node) ? node.gainDb : node.shape === 'notch' ? -15 : -3;
    return { x: frequencyX(hz, bounds, rate), y: gainY(db, bounds) };
  }

  function movingNode(node: EqNode, x: number, y: number, geometry = bounds) {
    let gainDb = node.gainDb;
    if (supportsGain(node)) gainDb = Math.max(-6, Math.min(6, Math.round(yGain(y, geometry))));
    return { ...node, hz: xFrequency(x, geometry, rate), gainDb };
  }

  function plotCandidate(x: number, y: number, allocate: boolean): EqNode | undefined {
    if (!snapshot || x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom) return;
    const hz = xFrequency(x, bounds, rate);
    const shape = hz <= 100 ? 'low' : hz >= 7000 && rate * 0.45 >= 7000 ? 'high' : 'bell';
    let node = legacyEqNode(settings, shape);
    if (shape === 'bell' && node.stored) {
      if (!canAdd) return;
      node = allocate ? makeExtraEqNode(settings, 'bell') : { ...node, extra: {}, stored: false };
    }
    const [min, max] = eqFrequencyRange(node, rate);
    if (max < min) return;
    const next = movingNode(node, x, y);
    if (supportsGain(next) && !next.stored && !next.gainDb) next.gainDb = 2;
    return next;
  }

  function focusNode(key: string) {
    root.querySelector<HTMLElement>('[data-eq-key="' + CSS.escape(key) + '"]')?.focus({ preventScroll: true });
  }

  async function exact(node: EqNode) {
    onmode(node.key);
    await tick();
    const input = root.querySelector<HTMLInputElement>('.band-editor input');
    input?.focus({ preventScroll: true });
    input?.select();
  }

  function sendBand(node: EqNode, gesture?: string) {
    const patch = eqBandPatch(settings, node, rate);
    if (!Object.keys(patch).length) return false;
    onedit(patch, gesture);
    return true;
  }

  function focusGraph() {
    root.querySelector<HTMLElement>('.plot-hit')?.focus({ preventScroll: true });
  }

  async function remove(node: EqNode) {
    if (!node.stored) return;
    if (soloed === node.key) stopSolo();
    onedit(removeEqBand(settings, node));
    onmode(null);
    report = node.name + ' removed.';
    await tick();
    focusGraph();
  }

  function toggle(node: EqNode) {
    if (node.stored) sendBand({ ...node, bypassed: !node.bypassed });
    else sendBand({ ...node, gainDb: supportsGain(node) ? 2 : node.gainDb, bypassed: false });
  }

  function selectNode(node: EqNode, rotate: boolean) {
    const current = position(node);
    const stack = nodes.filter(candidate => {
      const adjacent = position(candidate);
      return Math.hypot(current.x - adjacent.x, current.y - adjacent.y) <= 19;
    });
    const index = stack.findIndex(candidate => candidate.key === mode);
    const next = rotate && stack.length > 1 && index >= 0 ? stack[(index + 1) % stack.length] : node;
    onmode(next.key);
    if (next.key !== node.key) void tick().then(() => focusNode(next.key));
  }

  function add(shape: ExtraEqShape, point?: Point) {
    const standard = legacyEqNode(settings, shape);
    if (standard.stored && !canAdd) return;
    const node = standard.stored ? makeExtraEqNode(settings, shape) : standard;
    const next = point ? movingNode(node, point.x, point.y) : node;
    if (shape === 'bell' && !next.gainDb) next.gainDb = 2;
    if (!sendBand({ ...next, bypassed: false })) return;
    onmode(node.key);
    report = node.name + ' added.';
    void tick().then(() => focusNode(node.key));
  }

  function paste(node?: EqNode) {
    if (!clipboard) return;
    const base = legacyEqNode(settings, clipboard.shape);
    let target = node?.shape === clipboard.shape ? node : base;
    if (target === base && base.stored && canAdd && (clipboard.shape === 'bell' || clipboard.shape === 'notch')) {
      target = makeExtraEqNode(settings, clipboard.shape);
    }
    if (sendBand({ ...target, hz: clipboard.hz, gainDb: clipboard.gainDb, q: clipboard.q,
      slope: clipboard.slope, bypassed: clipboard.bypassed })) {
      onmode(target.key);
      report = clipboard.name + ' settings pasted.';
      void tick().then(() => focusNode(target.key));
    }
  }

  function copy(node: EqNode) {
    clipboard = node;
    report = node.name + ' settings copied.';
  }

  function reset(node: EqNode) {
    const spec = legacyEqBands[node.shape];
    sendBand({
      ...node,
      hz: treatmentNumbers[spec.frequency].initial,
      gainDb: 0,
      q: node.shape === 'notch' ? 1500 : 80,
      slope: 12,
      bypassed: false,
    });
    report = node.name + ' reset.';
  }

  function canChangeShape(node: EqNode, shape: EqShape) {
    const base = legacyEqNode(settings, shape);
    return !base.stored || ((shape === 'bell' || shape === 'notch') && (Boolean(node.extra) || canAdd));
  }

  function changeShape(node: EqNode, shape: EqShape) {
    if (!canChangeShape(node, shape)) return;
    const base = legacyEqNode(settings, shape);
    if (node.extra && (shape === 'bell' || shape === 'notch')) {
      sendBand({ ...node, shape, q: base.q, gainDb: shape === 'bell' ? node.gainDb || 2 : 0 });
      report = 'Band changed to ' + base.name.toLowerCase() + '.';
      void tick().then(() => focusNode(node.key));
      return;
    }
    const removed = removeEqBand(settings, node);
    const target = base.stored && (shape === 'bell' || shape === 'notch') ? makeExtraEqNode(settings, shape) : base;
    const patch = eqBandPatch({ ...settings, ...removed }, {
      ...target, hz: node.hz, gainDb: supportsGain(target) ? node.gainDb || 2 : 0, bypassed: node.bypassed,
    }, rate);
    if (!Object.keys(patch).length) return;
    onedit({ ...removed, ...patch });
    onmode(target.key);
    report = 'Band changed to ' + target.name.toLowerCase() + '.';
    void tick().then(() => focusNode(target.key));
  }

  function startSolo(node: EqNode) {
    const range = eqSoloRange(node, rate);
    if (!snapshot || !range) return;
    soloed = node.key;
    onaction('solo-band', undefined, range);
  }

  function stopSolo() {
    heldKey = undefined;
    heldPointer = undefined;
    if (!soloed) return;
    soloed = null;
    onaction('stop-audition');
  }

  function soloToggle(node: EqNode) {
    if (soloed === node.key) stopSolo();
    else startSolo(node);
  }

  function holdSolo(event: PointerEvent, node: EqNode) {
    if (event.button !== 0 || heldPointer !== undefined) return;
    const control = event.currentTarget as HTMLButtonElement;
    control.setPointerCapture(event.pointerId);
    heldPointer = event.pointerId;
    startSolo(node);
  }

  function releaseSolo(event: PointerEvent) {
    if (heldPointer === event.pointerId) stopSolo();
  }

  function releaseHeldSolo() {
    if (heldPointer !== undefined || heldKey !== undefined) stopSolo();
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

  function freezeSpectrum() {
    if (frozen) {
      frozen = null;
      report = 'Live spectrum resumed.';
    } else if (probe) {
      frozen = probe;
      report = 'Spectrum frozen.';
    }
  }

  function startDrag(event: PointerEvent, picked?: EqNode) {
    if (!snapshot || event.button !== 0 || drag) return;
    const control = event.currentTarget as HTMLButtonElement;
    const rect = control.parentElement!.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    const node = picked ?? plotCandidate(x, y, true);
    if (!node) return;
    const origin = picked ? position(picked) : { x, y };
    drag = {
      pointerId: event.pointerId, control, sourceId: snapshot.sourceId, node,
      before: originalEqBand(settings, node), gesture: id + '-' + ++gestureNumber,
      fromPlot: !picked, bounds, x: event.clientX, y: event.clientY, lastX: event.clientX, lastY: event.clientY,
      targetX: origin.x, targetY: origin.y, moved: false, applied: false,
    };
    editorOnLeft = origin.x > width * 0.6;
    ghost = undefined;
    control.setPointerCapture(event.pointerId);
  }

  function accumulate(current: Drag, event: PointerEvent) {
    const dx = event.clientX - current.lastX;
    const dy = event.clientY - current.lastY;
    current.lastX = event.clientX;
    current.lastY = event.clientY;
    const fine = event.shiftKey ? 0.2 : 1;
    current.targetX += dx * fine;
    current.targetY += dy * fine;
    current.moved ||= Math.hypot(event.clientX - current.x, event.clientY - current.y) >= 3;
    return current.moved && (dx !== 0 || dy !== 0);
  }

  function applyDrag(current: Drag, isClick: boolean) {
    if (current.sourceId !== snapshot?.sourceId) return;
    const next = isClick ? current.node : movingNode(current.node, current.targetX, current.targetY, current.bounds);
    if (!sendBand(next, current.gesture)) return;
    current.applied = true;
    if (mode !== next.key) onmode(next.key);
  }

  function move(event: PointerEvent) {
    const current = drag;
    if (!current) {
      if (event.currentTarget instanceof HTMLElement && event.currentTarget.classList.contains('plot-hit') && event.pointerType !== 'touch') {
        ghost = plotCandidate(event.offsetX, event.offsetY, false);
      }
      return;
    }
    if (event.pointerId !== current.pointerId || !accumulate(current, event) || dragFrame) return;
    moving = true;
    dragFrame = requestAnimationFrame(() => {
      dragFrame = 0;
      if (drag === current) applyDrag(current, false);
    });
  }

  function finishDrag(event?: PointerEvent) {
    const current = drag;
    if (!current || (event && event.pointerId !== current.pointerId)) return;
    const pending = Boolean(dragFrame);
    cancelAnimationFrame(dragFrame);
    dragFrame = 0;
    drag = undefined;
    moving = false;
    editorOnLeft = undefined;
    if (event?.type === 'pointerup') {
      const changed = accumulate(current, event);
      if (pending || changed) applyDrag(current, false);
      else if (current.fromPlot && !current.moved) applyDrag(current, true);
      else if (!current.moved) selectNode(current.node, true);
      if (current.fromPlot && current.applied) void tick().then(() => focusNode(current.node.key));
    } else if (current.applied && current.sourceId === snapshot?.sourceId) {
      onedit(current.before, current.gesture);
      onmode(current.node.stored || isLegacyEqBand(current.node.key) ? current.node.key : null);
      report = 'Band adjustment cancelled.';
    }
    if (current.control.hasPointerCapture(current.pointerId)) current.control.releasePointerCapture(current.pointerId);
  }

  function openContext(event: MouseEvent, node?: EqNode) {
    if (!snapshot) return;
    event.preventDefault();
    event.stopPropagation();
    const control = event.currentTarget as HTMLElement;
    const rect = control.parentElement!.getBoundingClientRect();
    menuPoint = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    menuNodeKey = node?.key ?? null;
    menuReturn = control;
    menuEditsValue = false;
    ghost = undefined;
    if (node) onmode(node.key);
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

  async function closeMenu(open: boolean) {
    if (open || menuEditsValue) return;
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
      if (key === 'c' && node.stored) {
        event.preventDefault();
        copy(node);
      } else if (key === 'v' && clipboard) {
        event.preventDefault();
        paste(node);
      }
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      void exact(node);
      return;
    }
    if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault();
      void remove(node);
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
    if (supportsGain(node) && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
      sendBand({ ...node, gainDb: node.gainDb + direction * (event.shiftKey ? 1 : 2) });
    } else sendBand({ ...node, hz: node.hz * (event.shiftKey ? 1.015 : 1.1) ** direction });
  }

  function scrollNode(node: EqNode) {
    return (element: HTMLButtonElement) => {
      const wheel = (event: WheelEvent) => {
        if (mode !== node.key || !event.deltaY) return;
        const spec = legacyEqBands[node.shape];
        const direction = -Math.sign(event.deltaY);
        if ('slope' in spec) {
          event.preventDefault();
          sendBand({ ...node, slope: node.slope + direction * 12 });
        } else if ('q' in spec) {
          event.preventDefault();
          const step = node.shape === 'notch' && !node.extra ? 100 : event.shiftKey ? 1 : 10;
          sendBand({ ...node, q: node.q + direction * step });
        }
      };
      element.addEventListener('wheel', wheel, { passive: false });
      return () => element.removeEventListener('wheel', wheel);
    };
  }

  function escape(event: KeyboardEvent) {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    if (drag || soloed) {
      event.preventDefault();
      finishDrag();
      stopSolo();
    } else if (selected && event.target instanceof Element && event.target.closest('.band-editor')) {
      event.preventDefault();
      focusNode(selected.key);
    } else if (mode && event.target instanceof Element && root.contains(event.target)) {
      event.preventDefault();
      onmode(null);
      focusGraph();
    }
  }

  function finishInteraction() {
    finishDrag();
    stopSolo();
  }

  function keyboard(element: HTMLElement) {
    root = element;
    element.addEventListener('keydown', escape);
    return () => element.removeEventListener('keydown', escape);
  }

  onDestroy(finishInteraction);
</script>

<svelte:window onblur={finishInteraction}/>

{#snippet numberField(label: string, value: number, min: number, max: number, step: number, unit: string, commit: (value: number) => void)}
  <div class="parameter">
    <span>{label}</span>
    <NumberInput class="eq-number" {value} {min} max={Math.max(min, max)} {step} largeStep={step * 5}
      disabled={!snapshot || max < min} onValueCommit={next => { if (next !== null) commit(next); }}>
      <NumberInputControl class="eq-input" aria-label={unit ? label + ' in ' + unit : label}/>
      {#if unit}<span class="unit" aria-hidden="true">{unit}</span>{/if}
    </NumberInput>
  </div>
{/snippet}

<section class="eq-panel" aria-label="Equaliser controls" {@attach keyboard}>
  <Menu controller={bandMenu} onOpenChange={closeMenu}>
    <div class="plot" role="group" aria-label="Frequency response and spectrum" bind:clientWidth={width} bind:clientHeight={height}>
      <canvas {@attach paint} aria-hidden="true"></canvas>
      <GraphButton type="button" class="plot-hit" aria-label="Add EQ band on the graph" aria-describedby={id + '-help'} disabled={!snapshot}
        onpointerdown={event => startDrag(event)} onpointermove={move} onpointerleave={() => ghost = undefined}
        onpointerup={finishDrag} onpointercancel={finishDrag} onlostpointercapture={finishDrag}
        onkeydown={event => {
          if (contextFromKey(event) || event.isComposing) return;
          if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'v' && clipboard) {
            event.preventDefault();
            paste();
          } else if (event.key === 'Enter' && !event.ctrlKey && !event.metaKey) {
            event.preventDefault();
            add('bell');
          }
        }} oncontextmenu={event => openContext(event)}/>
      {#if ghost && !moving}
        {@const spot = position(ghost)}
        <div class="ghost" aria-hidden="true" style:left={spot.x + 'px'} style:top={spot.y + 'px'}>
          <Plus size={13}/>
          <span>{ghost.stored ? 'Move ' : 'Add '}{ghost.name.toLowerCase()}</span>
        </div>
      {/if}
      {#each nodes as node (node.key)}
        {@const spot = position(node)}
        <GraphButton type="button" data-eq-key={node.key}
          class={['marker', { active: node.stored && !node.bypassed, bypassed: node.bypassed, selected: node.key === mode, soloed: node.key === soloed }]}
          disabled={!snapshot} style={'left:' + spot.x + 'px;top:' + spot.y + 'px'}
          aria-label={node.name} aria-description={node.bypassed ? 'Bypassed' : node.stored ? 'Active' : 'Inactive'} aria-describedby={id + '-node-help'}
          aria-pressed={node.key === mode} title={node.name + ' · ' + Math.round(node.hz) + ' Hz' + (node.bypassed ? ' · Bypassed' : '') + ' · Enter to edit'}
          onclick={event => { if (event.detail === 0) selectNode(node, false); }} ondblclick={() => void exact(node)}
          onpointerdown={event => startDrag(event, node)} onpointermove={move}
          onpointerup={finishDrag} onpointercancel={finishDrag} onlostpointercapture={finishDrag}
          onkeydown={event => nodeKeys(event, node)} oncontextmenu={event => openContext(event, node)}
          {@attach scrollNode(node)}/>
      {/each}
      {#if selected}
        {@const node = selected}
        {@const [min, max] = eqFrequencyRange(node, rate)}
        {@const spec = legacyEqBands[node.shape]}
        <div class={['band-editor', { left: editorOnLeft ?? (position(node).x > width * 0.6) }]} role="group" aria-label={node.name + ' settings'}>
          <Button variant="ghost" size="icon-sm" aria-label={node.stored && !node.bypassed ? 'Bypass band' : 'Enable band'}
            title={node.stored && !node.bypassed ? 'Bypass band' : 'Enable band'} aria-pressed={node.stored && !node.bypassed} onclick={() => toggle(node)}>
            {#if node.stored && !node.bypassed}<Volume2 size={15} aria-hidden="true"/>{:else}<VolumeX size={15} aria-hidden="true"/>{/if}
          </Button>
          {@render numberField('Frequency', Math.max(min, Math.min(node.hz, max)), min, max, 1, 'Hz', hz => sendBand({ ...node, hz }))}
          {#if 'gain' in spec}
            {@render numberField('Gain', node.gainDb, -6, 6, 1, 'dB', gainDb => sendBand({ ...node, gainDb }))}
          {/if}
          {#if 'q' in spec}
            {@const [minimum, maximum] = eqWidthRange(node.shape)}
            {@render numberField('Width (Q)', node.q / 100, minimum / 100, maximum / 100,
              node.shape === 'notch' && !node.extra ? 1 : 0.01, '', q => sendBand({ ...node, q: q * 100 }))}
          {/if}
          {#if 'slope' in spec}
            <div class="parameter">
              <span>Slope</span>
              <Select controller={slopeControl} value={String(node.slope)} onValueChange={value => { if (value) sendBand({ ...node, slope: Number(value) }); }}>
                <SelectTrigger class="eq-slope" aria-label="Slope">{node.slope} dB/oct</SelectTrigger>
                <SelectContent class="app-select-menu audio-tool-popup" align="start" sideOffset={4}>
                  {#each slopes as slope (slope)}
                    <SelectItem value={String(slope)} label={slope + ' dB/oct'} data-slot="select-item">{slope} dB/oct</SelectItem>
                  {/each}
                </SelectContent>
              </Select>
            </div>
          {/if}
          <Button variant="ghost" size="icon-sm" aria-label="Hold to solo band" title="Hold to solo band" aria-pressed={soloed === node.key}
            onpointerdown={(event: PointerEvent) => holdSolo(event, node)} onpointerup={releaseSolo} onpointercancel={releaseSolo} onlostpointercapture={releaseSolo}
            onkeydown={(event: KeyboardEvent) => soloKeys(event, node)} onkeyup={(event: KeyboardEvent) => soloKeys(event, node)}
            onblur={releaseHeldSolo}
            onclick={(event: MouseEvent) => { if (event.detail === 0) soloToggle(node); }}>
            <Headphones size={15} aria-hidden="true"/>
          </Button>
        </div>
      {/if}
    </div>
    <MenuContent class="app-select-menu audio-tool-popup eq-menu">
      {#if menuNode}
        {@const node = menuNode}
        <MenuItem onSelect={() => { menuEditsValue = true; void exact(node); }}>Edit exact values <MenuShortcut aria-hidden="true">Enter</MenuShortcut></MenuItem>
        <MenuItem onSelect={() => toggle(node)}>{node.stored && !node.bypassed ? 'Bypass band' : 'Enable band'}</MenuItem>
        <MenuItem onSelect={() => soloToggle(node)}>{soloed === node.key ? 'Stop solo' : 'Solo band'}</MenuItem>
        <MenuSeparator/>
        <MenuSub>
          <MenuSubTrigger>Change shape <ChevronRight size={14} aria-hidden="true"/></MenuSubTrigger>
          <MenuSubContent class="app-select-menu eq-menu" sideOffset={4}>
            {#each shapes as shape (shape)}
              {#if shape !== node.shape}
                <MenuItem disabled={!canChangeShape(node, shape)}
                  onSelect={() => { menuEditsValue = true; changeShape(node, shape); }}>{legacyEqBands[shape].name}</MenuItem>
              {/if}
            {/each}
          </MenuSubContent>
        </MenuSub>
        <MenuItem onSelect={() => reset(node)}>Reset band</MenuItem>
        <MenuItem disabled={!node.stored} onSelect={() => copy(node)}>Copy band settings <MenuShortcut aria-hidden="true">Ctrl/⌘+C</MenuShortcut></MenuItem>
        <MenuItem disabled={!clipboard} onSelect={() => { menuEditsValue = true; paste(node); }}>Paste band settings <MenuShortcut aria-hidden="true">Ctrl/⌘+V</MenuShortcut></MenuItem>
        <MenuItem disabled={!node.stored} onSelect={() => { menuEditsValue = true; void remove(node); }}>Remove band <MenuShortcut aria-hidden="true">Del</MenuShortcut></MenuItem>
      {:else}
        <MenuItem disabled={!canAdd && legacyEqNode(settings, 'bell').stored}
          onSelect={() => { menuEditsValue = true; add('bell', menuPoint); }}>Add bell</MenuItem>
        <MenuItem disabled={!canAdd && legacyEqNode(settings, 'notch').stored}
          onSelect={() => { menuEditsValue = true; add('notch', menuPoint); }}>Add notch</MenuItem>
        <MenuItem disabled={!clipboard} onSelect={() => { menuEditsValue = true; paste(); }}>Paste band settings <MenuShortcut aria-hidden="true">Ctrl/⌘+V</MenuShortcut></MenuItem>
      {/if}
    </MenuContent>
  </Menu>
  <div class="eq-footer">
    <div class="band-list" role="group" aria-label="EQ bands">
      {#each nodes.filter(node => node.stored) as node (node.key)}
        <Button variant="ghost" size="sm" class="band-chip" aria-pressed={node.key === mode}
          data-bypassed={node.bypassed} title={node.name + (node.bypassed ? ' · Bypassed' : '')}
          onclick={() => { onmode(node.key); void tick().then(() => focusNode(node.key)); }}>
          {node.name} {Math.round(node.hz)} Hz
        </Button>
      {/each}
      <span id={id + '-help'} class={nodes.length ? 'sr-only' : 'hint'}>
        Click to add a band. Shift-drag makes smaller changes.
      </span>
    </div>
    <div class="eq-actions">
      <Menu controller={addMenu}>
        <MenuTrigger class="eq-command" aria-label="Add EQ band" title="Add EQ band" disabled={!snapshot}>
          <Plus size={17} aria-hidden="true"/>
        </MenuTrigger>
        <MenuContent class="app-select-menu audio-tool-popup eq-menu">
          <MenuItem disabled={!canAdd && legacyEqNode(settings, 'bell').stored} onSelect={() => add('bell')}>Add bell</MenuItem>
          <MenuItem disabled={!canAdd && legacyEqNode(settings, 'notch').stored} onSelect={() => add('notch')}>Add notch</MenuItem>
        </MenuContent>
      </Menu>
      <Button variant="ghost" size="icon-sm" aria-label={frozen ? 'Resume spectrum' : 'Freeze spectrum'}
        title={frozen ? 'Resume spectrum' : 'Freeze spectrum'} aria-pressed={Boolean(frozen)} disabled={!probe && !frozen}
        onclick={freezeSpectrum}><Snowflake size={16} aria-hidden="true"/></Button>
    </div>
  </div>
  <span id={id + '-node-help'} class="sr-only">Enter edits exact values. Arrows move the band and Shift makes smaller changes. Brackets select the previous or next band.</span>
  <span class="sr-only" aria-live="polite">{report}</span>
</section>

<style>
  .eq-panel { grid-column:1; grid-row:2 / 4; display:grid; grid-template-rows:minmax(218px, 1fr) auto; gap:var(--sp-2, 8px); min-width:0; }
  .plot { position:relative; isolation:isolate; overflow:hidden; overflow:clip; min-width:0; min-height:218px; }
  canvas, .plot :global(.plot-hit) { position:absolute; inset:0; width:100%; height:100%; border-radius:inherit; }
  canvas { pointer-events:none; }
  .plot :global(.plot-hit), .plot :global(.marker) { appearance:none; background:transparent; color:inherit; border:0; padding:0; touch-action:none; }
  .plot :global(.plot-hit) { cursor:crosshair; }
  .plot :global(.plot-hit:disabled) { cursor:default; }
  .plot :global(.plot-hit:focus-visible) { outline:2px solid var(--accent); outline-offset:-2px; }
  .plot :global(.marker) { position:absolute; transform:translate(-50%, -50%); display:grid; place-items:center; width:34px; height:34px; border-radius:50%; cursor:grab; }
  .plot :global(.marker:active) { cursor:grabbing; }
  .plot :global(.marker)::before {
    content:''; width:12px; height:12px; border-radius:inherit;
    border:1.5px solid var(--accent-ink, var(--text)); background:var(--panel);
  }
  .plot :global(.marker.active)::before { background:var(--accent); }
  .plot :global(.marker.bypassed)::before { border-style:dashed; opacity:.7; }
  .plot :global(.marker.soloed)::before { box-shadow:0 0 0 3px var(--sel-tint); }
  .plot :global(.marker:hover), .plot :global(.marker.selected) { background:var(--sel-tint); }
  .plot :global(.marker:focus-visible) { outline:2px solid var(--accent); outline-offset:-1px; }
  .plot :global(.marker:focus-visible), .plot :global(.marker.selected) { z-index:1; }
  .ghost { position:absolute; transform:translate(-50%, -50%); display:grid; place-items:center; width:22px; height:22px; pointer-events:none; border:1px dashed var(--accent-ink); border-radius:50%; color:var(--accent-ink); background:var(--panel); }
  .ghost span { position:absolute; top:calc(100% + 4px); width:max-content; max-width:120px; padding:2px var(--sp-1); border-radius:var(--r-compact); background:var(--panel); font-size:.6875rem; }
  .band-editor {
    position:absolute; z-index:2; inset-inline-end:33px; inset-block-end:29px; display:flex; flex-wrap:wrap;
    align-items:end; gap:var(--sp-2); padding:var(--sp-2); max-inline-size:calc(100% - 66px);
    border-radius:var(--r-inset); background:var(--panel); box-shadow:0 5px 16px rgb(0 0 0 / 22%);
  }
  .band-editor.left { inset-inline:33px auto; }
  .parameter { display:grid; gap:2px; color:var(--muted); font-size:.6875rem; font-weight:500; }
  .parameter :global(.eq-number), .parameter :global(.eq-slope) {
    display:inline-flex; align-items:center; min-height:28px; border:0; border-radius:var(--input-radius);
    background:transparent; color:var(--text); padding-inline:8px; font-size:.75rem;
  }
  .parameter :global(.eq-number:hover), .parameter :global(.eq-slope:hover) { background:var(--row-hover); }
  .parameter :global(.eq-input) { width:5ch; min-width:0; padding:0; border:0; background:transparent; color:inherit; outline:none; text-align:right; font-variant-numeric:tabular-nums; }
  .parameter :global(.eq-number:focus-within), .parameter :global(.eq-slope:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
  .unit { color:var(--muted); margin-inline-start:4px; }
  .eq-footer { min-height:36px; display:flex; align-items:start; gap:var(--sp-2); min-width:0; }
  .band-list { flex:1; display:flex; flex-wrap:wrap; gap:var(--sp-1); min-width:0; align-items:center; min-height:28px; }
  .band-list :global(.band-chip) { color:var(--muted); white-space:nowrap; border-radius:var(--r-pill); font-variant-numeric:tabular-nums; }
  .band-list :global(.band-chip[aria-pressed=true]), .eq-actions :global(button[aria-pressed=true]) { background:var(--sel-tint); color:var(--accent-ink); }
  .band-list :global(.band-chip[data-bypassed=true]) { text-decoration:line-through; }
  .hint { color:var(--muted); font-size:.75rem; }
  .eq-actions { display:flex; align-items:center; gap:var(--sp-1); color:var(--muted); }
  .eq-actions :global(.eq-command) { display:grid; place-items:center; width:28px; height:28px; border:0; border-radius:var(--r-compact); background:transparent; color:inherit; }
  .eq-actions :global(.eq-command:hover) { color:var(--text); }
  .eq-actions :global(.eq-command:focus-visible) { outline:2px solid var(--accent); }
</style>
