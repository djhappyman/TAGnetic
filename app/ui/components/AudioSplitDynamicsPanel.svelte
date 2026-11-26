<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { prefersReducedMotion } from 'svelte/motion';
  import { devicePixelRatio } from 'svelte/reactivity/window';
  import { AudioLines, Ear, Headphones, Power, RotateCcw, SlidersHorizontal } from '@lucide/svelte';
  import { Button as GraphButton } from '@ui-sdk/svelte/button';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';
  import Button from './AudioSplitButton.svelte';
  import { dynamicsMode } from '../js/audio-dynamics';
  import { noTreatment, treatmentNumbers, type AudioTreatment } from '../js/audio-treatment';
  import {
    compressionRatioAt, compressorOutput, createMeterMotion, describeDynamicsMeters, drawTreatmentPlot,
    dynamicsLevelFloor, levelX, levelY, plotBounds, xLevel, yLevel,
    type PlotBounds, type TreatmentProbe,
  } from '../js/audio-treatment-plot';
  import type { ToolAction, ToolSnapshot } from '../js/audio-tools-window';
  import type { FrameRange } from '../js/audio-split';

  let { snapshot, probe, mode, onedit, onaction }: {
    snapshot: ToolSnapshot | null;
    probe: TreatmentProbe | null;
    mode: string;
    onedit: (patch: Partial<AudioTreatment>, gesture?: string) => void;
    onaction: (action: ToolAction, range?: FrameRange) => void;
  } = $props();

  type MainField = 'compressorDb' | 'gateDb' | 'gainDb';
  type GraphField = 'compressorDb' | 'gateDb' | 'compressorRatio';
  type Field = GraphField | MainField | 'compressorInputDb' | 'gateReductionDb' | 'normalizeTargetDb'
    | 'compressorAttackMs' | 'compressorReleaseMs' | 'compressorKneeDb';
  type Audition = 'bypass-compressor' | 'difference-compressor';
  type Disclosure = 'settings' | 'audition';
  type Drag = {
    pointerId: number; button: HTMLButtonElement; sourceId: string; field: GraphField; gesture: string;
    bounds: PlotBounds; floor: number; lastX: number; lastY: number; x: number; y: number;
    threshold: number; knee: number; moved: boolean; needsCommit: boolean; original: Partial<AudioTreatment>;
  };

  const id = $props.id();
  const meters = createMeterMotion();
  const ratioInput = -1;
  let width = $state(0);
  let height = $state(0);
  let disclosure = $state<Disclosure>();
  let readout = $state<GraphField>();
  let audition = $state<Audition>();
  let heldPeaks = $state.raw<{ input?: number; output?: number }>({});
  let dragging = $state(false);
  let drag: Drag | undefined;
  let dragFrame = 0;
  let gestureNumber = 0;
  let meterSource: string | undefined;
  let rememberedSource: string | undefined;
  let remembered: Partial<Record<MainField, number>> = {};

  const view = $derived(dynamicsMode(mode));
  const settings = $derived(snapshot?.settings ?? noTreatment);
  const bounds = $derived(plotBounds(width, height));
  const floor = $derived(dynamicsLevelFloor(view));
  const threshold = $derived(settings.compressorDb || treatmentNumbers.compressorDb.initial);
  const ratio = $derived(settings.compressorRatio || treatmentNumbers.compressorRatio.initial);
  const knee = $derived(settings.compressorKneeDb || 0);
  const ratioOutput = $derived(compressorOutput(ratioInput, threshold, ratio, knee));
  const gateThreshold = $derived(settings.gateDb || treatmentNumbers.gateDb.initial);
  const appearance = $derived(snapshot?.theme);
  const normalisation = $derived(settings.normalize ? snapshot?.normalisation : undefined);
  const targetLoudness = $derived(stored('normalizeTargetDb'));
  const animate = $derived(!!snapshot?.playing && !prefersReducedMotion.current);
  const inputClipped = $derived((heldPeaks.input ?? -96) >= 0);
  const outputClipped = $derived((heldPeaks.output ?? -96) >= 0);
  const peakStatus = $derived((inputClipped && outputClipped) ? 'Input and output clipped'
    : inputClipped ? 'Input clipped' : outputClipped ? 'Output clipped' : 'Peak hold');

  function newerPeak(previous: number | undefined, current: number | undefined) {
    if (current === undefined || !Number.isFinite(current)) return previous;
    return previous === undefined ? current : Math.max(previous, current);
  }

  $effect(() => {
    const source = snapshot?.sourceId;
    const input = probe?.inputPeak;
    const output = probe?.outputPeak;
    // A held peak accumulates independent samples until the user resets it.
    // Only source changes and incoming readings should feed that accumulator.
    untrack(() => {
      if (source !== meterSource) {
        meterSource = source;
        heldPeaks = {};
      }
      if (!source) return;
      const next = { input: newerPeak(heldPeaks.input, input), output: newerPeak(heldPeaks.output, output) };
      if (next.input !== heldPeaks.input || next.output !== heldPeaks.output) heldPeaks = next;
    });
  });

  function dbText(value: number | undefined) {
    if (value === undefined) return '—';
    return (value < 0 ? '−' : '') + Math.abs(value).toFixed(1) + ' dBFS';
  }

  function paint(canvas: HTMLCanvasElement) {
    void appearance;
    void devicePixelRatio.current;
    const chart = { width, height, settings, sampleRate: snapshot?.sampleRate || 44100 };
    const mode = view;
    if (mode === 'normalise') {
      drawTreatmentPlot(canvas, chart.width, chart.height, 'dynamics', chart.settings, chart.sampleRate, null, { mode, normalisation });
      return;
    }
    const options = { mode, inputPeakHold: heldPeaks.input, outputPeakHold: heldPeaks.output };
    return meters(probe, animate, measured => {
      drawTreatmentPlot(canvas, chart.width, chart.height, 'dynamics', chart.settings, chart.sampleRate, measured, options);
    });
  }

  function stored(key: Field) {
    return settings[key] || treatmentNumbers[key].initial;
  }

  function number(key: Field, value: number) {
    const { min, max } = treatmentNumbers[key];
    return Math.max(min, Math.min(max, Math.round(value)));
  }

  function change(key: Field, value: number, gesture?: string) {
    const amount = number(key, value);
    if (key === 'compressorRatio' && !settings.compressorDb) {
      onedit({ compressorDb: threshold, compressorRatio: amount }, gesture);
    } else if (key === 'gateReductionDb' && !settings.gateDb) {
      onedit({ gateDb: gateThreshold, gateReductionDb: amount }, gesture);
    } else if (key === 'normalizeTargetDb' && !settings.normalize) {
      onedit({ normalize: true, normalizeTargetDb: amount }, gesture);
    } else {
      onedit({ [key]: amount }, gesture);
    }
  }

  function toggle(key: MainField) {
    if (rememberedSource !== snapshot?.sourceId) {
      remembered = {};
      rememberedSource = snapshot?.sourceId;
    }
    const previous = settings[key];
    if (previous) remembered[key] = previous;
    onedit({ [key]: previous ? 0 : remembered[key] ?? (key === 'gainDb' ? 3 : treatmentNumbers[key].initial) });
  }

  function startDrag(event: PointerEvent, field: GraphField, fromPlot = false) {
    if (!snapshot || event.button !== 0 || drag) return;
    const button = event.currentTarget as HTMLButtonElement;
    const rect = button.parentElement!.getBoundingClientRect();
    const isRatio = field === 'compressorRatio';
    const pivot = field === 'gateDb' ? gateThreshold : threshold;
    const x = fromPlot ? event.clientX - rect.left : levelX(isRatio ? ratioInput : pivot, bounds, floor);
    const y = isRatio ? levelY(ratioOutput, bounds) : event.clientY - rect.top;
    const original = field === 'compressorRatio' ? { compressorRatio: settings.compressorRatio, compressorDb: settings.compressorDb }
      : { [field]: settings[field] };
    drag = {
      pointerId: event.pointerId, button, sourceId: snapshot.sourceId, field, gesture: id + '-dynamics-' + ++gestureNumber,
      bounds, floor, lastX: event.clientX, lastY: event.clientY, x, y, threshold, knee,
      moved: false, needsCommit: fromPlot, original,
    };
    readout = field;
    dragging = true;
    button.setPointerCapture(event.pointerId);
  }

  function accumulate(event: PointerEvent) {
    if (!drag || drag.pointerId !== event.pointerId) return false;
    const dx = event.clientX - drag.lastX;
    const dy = event.clientY - drag.lastY;
    drag.lastX = event.clientX;
    drag.lastY = event.clientY;
    const precision = event.shiftKey ? 0.2 : 1;
    drag.x += dx * precision;
    drag.y += dy * precision;
    return dx !== 0 || dy !== 0;
  }

  function applyDrag() {
    dragFrame = 0;
    if (!drag || drag.sourceId !== snapshot?.sourceId) return;
    const next = drag.field === 'compressorRatio'
      ? compressionRatioAt(ratioInput, yLevel(drag.y, drag.bounds), drag.threshold, drag.knee)
      : xLevel(drag.x, drag.bounds, drag.floor);
    change(drag.field, next, drag.gesture);
    drag.moved = true;
  }

  function moveDrag(event: PointerEvent) {
    if (accumulate(event) && !dragFrame) dragFrame = requestAnimationFrame(applyDrag);
  }

  function stopDrag(event?: PointerEvent) {
    const current = drag;
    if (!current || (event && event.pointerId !== current.pointerId)) return;
    const release = event?.type === 'pointerup' ? event : undefined;
    const changed = release ? accumulate(release) : false;
    const pending = dragFrame !== 0;
    if (pending) cancelAnimationFrame(dragFrame);
    if (release && (changed || pending || current.needsCommit)) applyDrag();
    else if (!release && current.moved && current.sourceId === snapshot?.sourceId) onedit(current.original, current.gesture);
    dragFrame = 0;
    drag = undefined;
    dragging = false;
    if (current.button.hasPointerCapture(current.pointerId)) current.button.releasePointerCapture(current.pointerId);
    if (release && current.needsCommit) document.getElementById(id + '-handle-' + current.field)?.focus();
    else if (!release) readout = undefined;
  }

  function focusField(field: Field) {
    const input = document.getElementById(id + '-' + field) as HTMLInputElement | null;
    input?.focus();
    input?.select();
  }

  function graphKeys(event: KeyboardEvent, field: GraphField) {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const { key } = event;
    if (key === 'Enter') {
      event.preventDefault();
      focusField(field);
      return;
    }
    if (key === 'Delete' || key === 'Backspace') {
      event.preventDefault();
      const main = field === 'gateDb' ? 'gateDb' : 'compressorDb';
      if (settings[main]) toggle(main);
      return;
    }
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(key)) return;
    event.preventDefault();
    const direction = key === 'ArrowRight' || key === 'ArrowUp' ? 1 : -1;
    const step = event.shiftKey || field === 'compressorRatio' ? 1 : 2;
    change(field, stored(field) + direction * step);
  }

  function startAudition(action: Audition) {
    if (!snapshot || !settings.compressorDb || audition === action) return;
    audition = action;
    onaction(action);
  }

  function stopAudition() {
    if (!audition) return;
    audition = undefined;
    onaction('stop-audition');
  }

  function auditionPointer(event: PointerEvent, action: Audition) {
    if (event.button !== 0) return;
    (event.currentTarget as HTMLButtonElement).setPointerCapture(event.pointerId);
    startAudition(action);
  }

  function auditionKeys(event: KeyboardEvent, action?: Audition) {
    if (event.key !== ' ' && event.key !== 'Enter') return;
    event.preventDefault();
    if (action) {
      if (!event.repeat) startAudition(action);
    } else stopAudition();
  }

  function captureKeys(event: KeyboardEvent) {
    if (event.key !== 'Escape' || (!drag && !audition && !(view === 'compressor' && disclosure))) return;
    event.preventDefault();
    event.stopPropagation();
    if (drag || audition) releaseControls();
    else if (disclosure) {
      document.getElementById(id + '-' + disclosure + '-toggle')?.focus();
      disclosure = undefined;
    }
  }

  function toggleDisclosure(panel: Disclosure) {
    stopAudition();
    disclosure = disclosure === panel ? undefined : panel;
  }

  function releaseControls() {
    stopDrag();
    stopAudition();
  }

  onDestroy(releaseControls);
</script>

<svelte:window onkeydowncapture={captureKeys} onblur={releaseControls}/>

{#snippet exact(key: Field, label: string, hint?: string)}
  {@const spec = treatmentNumbers[key]}
  <label class="parameter" title={hint}>
    <span>{label}</span>
    <NumberInput class="dynamics-number" value={stored(key)} min={spec.min} max={spec.max} step={spec.step || 1}
      disabled={!snapshot} onValueCommit={value => { if (value !== null) change(key, value); }}>
      <NumberInputControl id={id + '-' + key} class="parameter-input" aria-label={spec.unit && spec.unit !== ':1' ? label + ' in ' + spec.unit : label}/>
      {#if spec.unit}<span class="unit" aria-hidden="true">{spec.unit}</span>{/if}
    </NumberInput>
  </label>
{/snippet}

{#snippet power(active: boolean, title: string, action: () => void)}
  <Button variant="ghost" size="icon-sm" class="dynamics-power" aria-label={title} {title} aria-pressed={active} disabled={!snapshot} onclick={action}>
    <Power size={15} aria-hidden="true"/>
  </Button>
{/snippet}

{#snippet handle(field: GraphField, label: string, x: number, y: number, active: boolean)}
  {@const spec = treatmentNumbers[field]}
  <GraphButton id={id + '-handle-' + field} type="button" role="slider" class={['marker', { active, selected: readout === field && dragging }]}
    style={'left:' + x + 'px;top:' + y + 'px'} aria-label={label} title={label + ' · Drag or use arrows · Enter to type'}
    aria-valuemin={spec.min} aria-valuemax={spec.max} aria-valuenow={stored(field)} aria-valuetext={stored(field) + ' ' + spec.unit}
    disabled={!snapshot} onpointerdown={event => startDrag(event, field)} onpointermove={moveDrag} onpointerup={stopDrag}
    onpointercancel={stopDrag} onlostpointercapture={stopDrag}
    onkeydown={event => graphKeys(event, field)} ondblclick={() => focusField(field)}
    onfocus={() => readout = field} onblur={() => { if (!dragging) readout = undefined; }}/>
{/snippet}

{#snippet monitor(action: Audition, label: string)}
  <Button variant="ghost" size="icon-sm" class="audition tool-icon" aria-pressed={audition === action}
    aria-label={label} title={'Hold to ' + label.toLowerCase()} disabled={!snapshot || !settings.compressorDb}
    onpointerdown={(event: PointerEvent) => auditionPointer(event, action)} onpointerup={stopAudition} onpointercancel={stopAudition}
    onlostpointercapture={stopAudition} onkeydown={(event: KeyboardEvent) => auditionKeys(event, action)} onkeyup={auditionKeys}
    onblur={stopAudition} onclick={(event: MouseEvent) => { if (!event.detail) audition === action ? stopAudition() : startAudition(action); }}>
    {#if action === 'bypass-compressor'}<Ear size={14} aria-hidden="true"/>{:else}<AudioLines size={14} aria-hidden="true"/>{/if}
  </Button>
{/snippet}

<div class="dynamics-panel">
  <div class="plot" bind:clientWidth={width} bind:clientHeight={height} role="group"
    aria-label={view === 'gate' ? 'Quiet-gap response and reduction' : view === 'compressor' ? 'Compressor response and gain reduction' : view === 'gain' ? 'Input, output and headroom' : 'Whole-set loudness and target'}
    aria-describedby={id + '-meters'}>
    <span class="sr-only" id={id + '-meters'}>
      {#if view === 'normalise'}
        Whole-set target {targetLoudness} LUFS.
        {#if normalisation?.loudnessLufs != null}
          Measured {normalisation.loudnessLufs.toFixed(1)} LUFS. Estimated output {(normalisation.loudnessLufs + normalisation.gainDb).toFixed(1)} LUFS.
        {/if}
      {:else}
        {describeDynamicsMeters(probe, view)}
      {/if}
    </span>
    <canvas {@attach paint} aria-hidden="true"></canvas>
    {#if view === 'compressor' || view === 'gate'}
      {@const gate = view === 'gate'}
      <GraphButton class="plot-hit" type="button" tabindex={-1} aria-hidden="true"
        disabled={!snapshot} onpointerdown={event => startDrag(event, gate ? 'gateDb' : 'compressorDb', true)}
        onpointermove={moveDrag} onpointerup={stopDrag} onpointercancel={stopDrag} onlostpointercapture={stopDrag}/>
      {#if gate}
        {@render handle('gateDb', 'Quiet-gap threshold', levelX(gateThreshold, bounds, floor), levelY(gateThreshold, bounds, floor), Boolean(settings.gateDb))}
      {:else}
        {@render handle('compressorDb', 'Compression threshold', levelX(threshold, bounds), levelY(compressorOutput(threshold, threshold, ratio, knee), bounds), Boolean(settings.compressorDb))}
        {@render handle('compressorRatio', 'Compression ratio', levelX(ratioInput, bounds), levelY(ratioOutput, bounds), Boolean(settings.compressorDb))}
      {/if}
      {#if readout && (gate ? readout === 'gateDb' : readout !== 'gateDb')}
        {@const spec = treatmentNumbers[readout]}
        <span class="graph-readout" aria-hidden="true">{readout === 'compressorRatio' ? 'Ratio' : 'Threshold'} {stored(readout)}{spec.unit === ':1' ? ':1' : ' ' + spec.unit}</span>
      {/if}
    {/if}
    {#if view !== 'normalise'}
      <Button variant="ghost" size="icon-sm" class={inputClipped || outputClipped ? 'peak-reset clipped' : 'peak-reset'}
        disabled={heldPeaks.input === undefined && heldPeaks.output === undefined} onclick={() => heldPeaks = {}}
        title={'Reset peak hold · Input ' + dbText(heldPeaks.input) + ' · Output ' + dbText(heldPeaks.output)}
        aria-label={'Reset peak hold. ' + peakStatus + '. Input ' + dbText(heldPeaks.input) + '. Output ' + dbText(heldPeaks.output)}>
        <RotateCcw size={14} aria-hidden="true"/>
      </Button>
    {/if}
  </div>

  <div class="controls" class:compressor={view === 'compressor'} role="group" aria-label="Dynamics controls">
    {#if view === 'compressor'}
      <div class="main-values" role="group" aria-label="Compressor levels">
        {@render exact('compressorInputDb', 'Input gain', 'Adjust the level before compression')}
        {@render exact('compressorDb', 'Threshold')}
        {@render exact('compressorRatio', 'Ratio')}
      </div>
      <div class="compressor-actions">
        {@render power(Boolean(settings.compressorDb), 'Compressor', () => toggle('compressorDb'))}
        <Button id={id + '-audition-toggle'} variant="ghost" size="icon-sm" aria-expanded={disclosure === 'audition'}
          aria-controls={disclosure === 'audition' ? id + '-audition' : undefined}
          aria-label="Compressor audition" title="Compressor audition" disabled={!snapshot || !settings.compressorDb}
          onclick={() => toggleDisclosure('audition')}><Headphones size={15} aria-hidden="true"/></Button>
        <Button id={id + '-settings-toggle'} variant="ghost" size="icon-sm" aria-expanded={disclosure === 'settings'}
          aria-controls={disclosure === 'settings' ? id + '-settings' : undefined}
          aria-label="Compressor settings" title="Attack, release, knee and output gain" disabled={!snapshot}
          onclick={() => toggleDisclosure('settings')}><SlidersHorizontal size={15} aria-hidden="true"/></Button>
      </div>
    {:else if view === 'gate'}
      {@render power(Boolean(settings.gateDb), 'Quiet-gap reduction', () => toggle('gateDb'))}
      {@render exact('gateDb', 'Threshold')}
      {@render exact('gateReductionDb', 'Reduction')}
    {:else if view === 'gain'}
      {@render power(Boolean(settings.gainDb), 'Gain', () => toggle('gainDb'))}
      {@render exact('gainDb', 'Gain')}
    {:else}
      {@render power(settings.normalize, 'Normalise the whole set', () => onedit({ normalize: !settings.normalize }))}
      {@render exact('normalizeTargetDb', 'Target')}
      <span class="scope" title="One gain adjustment across every track preserves their relative loudness.">Whole set</span>
      {#if settings.normalize && !normalisation}<span class="scope" role="status">Measuring loudness…</span>{/if}
    {/if}
  </div>
  {#if view === 'compressor' && disclosure === 'settings'}
    <div class="main-values details" role="group" aria-label="Compressor settings" id={id + '-settings'}>
      {@render exact('compressorAttackMs', 'Attack')}
      {@render exact('compressorReleaseMs', 'Release')}
      {@render exact('compressorKneeDb', 'Knee')}
      {@render exact('gainDb', 'Output gain', 'Adjust playback level after compression')}
    </div>
  {:else if view === 'compressor' && disclosure === 'audition'}
    <div class="monitors" role="group" aria-label="Compressor audition" id={id + '-audition'}>
      {@render monitor('bypass-compressor', 'Bypass compressor')}
      {@render monitor('difference-compressor', 'Hear what compression removes')}
    </div>
  {/if}
</div>

<style>
  .dynamics-panel { display:grid; grid-template-rows:minmax(218px, 1fr) auto auto; gap:var(--sp-2); min-width:0; }
  .plot { position:relative; isolation:isolate; overflow:hidden; overflow:clip; min-width:0; }
  canvas, .plot :global(.plot-hit) { position:absolute; inset:0; width:100%; height:100%; border-radius:inherit; }
  canvas { pointer-events:none; }
  .plot :global(.plot-hit), .plot :global(.marker) { appearance:none; background:transparent; color:inherit; border:0; padding:0; touch-action:none; }
  .plot :global(.plot-hit) { cursor:crosshair; }
  .plot :global(.plot-hit:disabled), .plot :global(.marker:disabled) { cursor:default; }
  .plot :global(.marker) { position:absolute; transform:translate(-50%, -50%); display:grid; place-items:center; width:24px; height:24px; border-radius:var(--r-circle); cursor:grab; }
  .plot :global(.marker:active) { cursor:grabbing; }
  .plot :global(.marker)::before { content:''; border:1px solid var(--accent-ink); width:9px; height:9px; border-radius:var(--r-circle); background:var(--panel); }
  .plot :global(.marker.active)::before { background:var(--accent); }
  .plot :global(.marker:hover), .plot :global(.marker.selected) { background:color-mix(in srgb, var(--accent) 23%, transparent); }
  .plot :global(.marker:focus-visible), .plot :global(.peak-reset:focus-visible) { outline:2px solid var(--accent); outline-offset:-1px; }
  .graph-readout, .plot :global(.peak-reset) { position:absolute; top:var(--sp-2); z-index:1; border-radius:var(--r-compact); color:var(--muted); font-size:var(--text-xs); }
  .graph-readout { left:var(--sp-2); pointer-events:none; background:var(--panel); padding:var(--sp-1) var(--sp-2); }
  .plot :global(.peak-reset) { right:var(--sp-1); top:0; }
  .plot :global(.peak-reset.clipped) { color:var(--del); }
  .controls, .main-values, .compressor-actions, .monitors { display:flex; flex-wrap:wrap; align-items:center; gap:var(--sp-2); }
  .controls, .details { padding-inline:var(--sp-1); }
  .controls { min-height:38px; }
  .controls.compressor { justify-content:space-between; }
  .main-values { gap:var(--sp-3); }
  .compressor-actions, .monitors { gap:var(--sp-1); }
  .monitors { justify-content:flex-end; }
  .parameter { display:flex; flex-direction:column; align-items:flex-start; color:var(--muted); font-size:var(--text-xs); }
  .parameter :global(.dynamics-number) { display:flex; align-items:center; min-height:27px; gap:3px; padding-inline:var(--sp-1); border:0; border-radius:var(--r-control); background:transparent; }
  .parameter :global(.dynamics-number:hover) { background:var(--row-hover); }
  .parameter :global(.dynamics-number:focus-within) { outline:2px solid var(--accent); outline-offset:1px; }
  .parameter :global(.parameter-input) { width:4ch; min-width:0; border:0; outline:0; padding:0; color:var(--text); background:transparent; font-family:var(--font-literal); font-size:.875rem; }
  .unit { min-width:max-content; font-size:var(--text-xs); opacity:.83; }
  .scope { font-size:var(--text-xs); color:var(--muted); }
  .compressor-actions :global([aria-expanded=true]), .controls :global(.dynamics-power[aria-pressed=true]), .monitors :global(.audition[aria-pressed=true]) { color:var(--accent-ink); }
</style>
