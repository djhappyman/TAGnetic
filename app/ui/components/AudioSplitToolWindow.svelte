<script lang="ts">
  import { onDestroy } from 'svelte';
  import { prefersReducedMotion } from 'svelte/motion';
  import { devicePixelRatio } from 'svelte/reactivity/window';
  import { ChevronDown, Ellipsis, Pause, Play, Power, Repeat2 } from '@lucide/svelte';
  import { Button as GraphButton } from '@ui-sdk/svelte/button';
  import { createMenuController, Menu, MenuContent, MenuItem, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { createSelectController, Select, SelectContent, SelectItem, SelectTrigger } from '@ui-sdk/svelte/select';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';
  import Button from './AudioSplitButton.svelte';
  import AudioSplitRepairPlot from './AudioSplitRepairPlot.svelte';
  import AudioSplitToolModes from './AudioSplitToolModes.svelte';
  import { noTreatment, treatmentNumbers, type AudioTreatment, type TreatmentNumber } from '../js/audio-treatment';
  import {
    createMeterMotion, createSpectrumMotion, describeDynamicsMeters,
    drawTreatmentPlot, frequencyX, gainY, levelX, levelY, plotBounds, xFrequency, xLevel, yGain, yLevel,
    type PlotBounds, type TreatmentPlotMode, type TreatmentProbe,
  } from '../js/audio-treatment-plot';
  import type { ToolAction, ToolSnapshot } from '../js/audio-tools-window';
  import type { FrameRange } from '../js/audio-split';

  let { kind, snapshot, probe, onedit, onaction, onviewport }: {
    kind: TreatmentPlotMode;
    snapshot: ToolSnapshot | null;
    probe: TreatmentProbe | null;
    onedit: (patch: Partial<AudioTreatment>, gesture?: string) => void;
    onaction: (action: ToolAction, range?: FrameRange) => void;
    onviewport: (range: FrameRange | null) => void;
  } = $props();

  type Option = { value: string; label: string };
  type Band = 'low' | 'bass' | 'bell' | 'notch' | 'treble' | 'high';
  type Dynamics = 'compressor' | 'gate' | 'gain' | 'normalise';
  type Repair = 'selection' | 'hum' | 'dc' | 'stereo';
  type DragMode = Band | 'threshold' | 'ratio';
  type Restorable = Band | 'compressor' | 'gate' | 'dc' | 'gain';
  type Drag = {
    pointerId: number; button: HTMLButtonElement; sourceId: string; mode: DragMode; gesture: string;
    rect: DOMRect; bounds: PlotBounds; x: number; y: number; lastX: number; lastY: number;
    targetX: number; targetY: number; started: boolean;
  };

  const bands = {
    low: { label: 'Low cut (rumble)', enabled: 'rumble', frequency: 'rumbleHz', slope: 'rumbleSlope' },
    bass: { label: 'Low shelf (bass)', enabled: 'bassDb', frequency: 'bassHz', gain: 'bassDb' },
    bell: { label: 'Bell (presence)', enabled: 'presenceDb', frequency: 'presenceHz', gain: 'presenceDb', q: 'presenceQ' },
    notch: { label: 'Notch (whistle)', enabled: 'notchHz', frequency: 'notchHz', q: 'notchQ' },
    treble: { label: 'High shelf (treble)', enabled: 'trebleDb', frequency: 'trebleHz', gain: 'trebleDb' },
    high: { label: 'High cut (hiss)', enabled: 'highCutHz', frequency: 'highCutHz', slope: 'highCutSlope' },
  } as const satisfies Record<Band, { label: string; enabled: keyof AudioTreatment; frequency: TreatmentNumber; gain?: TreatmentNumber; q?: TreatmentNumber; slope?: TreatmentNumber }>;
  const bandOptions = (Object.keys(bands) as Band[]).map(value => ({ value, label: bands[value].label }));
  const humOptions = [{ value: 'off', label: 'Off' }, { value: 'hz50', label: '50 Hz' }, { value: 'hz60', label: '60 Hz' }] as const;
  const monoOptions = [
    { value: 'off', label: 'Keep stereo' }, { value: 'mix', label: 'Mix to mono' },
    { value: 'left', label: 'Use left only' }, { value: 'right', label: 'Use right only' },
  ] as const;
  const polarityOptions = [{ value: 'off', label: 'Off' }, { value: 'left', label: 'Invert left' }, { value: 'right', label: 'Invert right' }] as const;

  const title = { eq: 'Equaliser', dynamics: 'Dynamics', repair: 'Repair' };
  const plotName = { eq: 'Frequency response and spectrum', dynamics: 'Compressor response and gain reduction' };
  const plotAction = {
    eq: 'Drag from the left or right for a frequency cut, or from the centre for a bell',
    dynamics: 'Drag to set the compression threshold',
  };
  const presets = {
    eq: [
      { label: 'Reduce rumble', patch: { rumble: true, rumbleHz: 35 } },
      { label: 'Soften hiss', patch: { highCutHz: 10000 } },
    ],
    dynamics: [
      { label: 'Smooth peaks', patch: { compressorDb: -18, compressorRatio: 2 } },
    ],
    repair: [
      { label: 'Tape cleanup (50 Hz)', patch: { dcHz: 5, hum: 'hz50', humHarmonics: 3 } },
      { label: 'Tape cleanup (60 Hz)', patch: { dcHz: 5, hum: 'hz60', humHarmonics: 3 } },
    ],
  } as const satisfies Record<TreatmentPlotMode, readonly { label: string; patch: Partial<AudioTreatment> }[]>;

  const presetMenu = createMenuController();
  const humControl = createSelectController();
  const monoControl = createSelectController();
  const polarityControl = createSelectController();
  const slopeControl = createSelectController();
  const slopeOptions = [12, 24, 36, 48].map(slope => ({ value: String(slope), label: slope + ' dB/oct' }));
  const id = $props.id();
  let band = $state<Band | null>(null);
  let dynamics = $state<Dynamics>('compressor');
  let repair = $state<Repair>('selection');
  let editorOnLeft = $state<boolean>();
  let width = $state(0);
  let height = $state(0);
  let gestureNumber = 0;
  let drag: Drag | undefined;
  let dragFrame = 0;
  let restoreSource: string | undefined;
  let restore: Partial<Record<Restorable, number>> = {};

  const settings = $derived(snapshot?.settings ?? noTreatment);
  const appearance = $derived(snapshot?.theme);
  const animate = $derived(!!snapshot?.playing && !prefersReducedMotion.current);
  const meters = createMeterMotion();
  const spectrum = createSpectrumMotion();
  const sampleRate = $derived(snapshot?.sampleRate || 44100);
  const bounds = $derived(plotBounds(width, height));
  const markerBands = $derived(bandOptions.filter(option => option.value === band || Boolean(settings[bands[option.value].enabled])));
  const compressorThreshold = $derived(settings.compressorDb || treatmentNumbers.compressorDb.initial);
  const compressorRatio = $derived(settings.compressorRatio || treatmentNumbers.compressorRatio.initial);
  const ratioInput = -1;
  const ratioOutput = $derived(compressorThreshold + (ratioInput - compressorThreshold) / compressorRatio);

  function paint(canvas: HTMLCanvasElement) {
    if (kind === 'repair') return;
    void appearance; // Redraw even when the recording is paused and only the theme changed.
    void devicePixelRatio.current;
    const chart = { width, height, kind, settings, sampleRate } as const;
    const motion = kind === 'eq' ? spectrum : meters;
    return motion(probe, animate, measured =>
      drawTreatmentPlot(canvas, chart.width, chart.height, chart.kind, chart.settings, chart.sampleRate, measured));
  }

  function selectMode(mode: string) {
    if (kind === 'eq') band = mode as Band;
    else if (kind === 'dynamics') dynamics = mode as Dynamics;
    else repair = mode as Repair;
  }

  function numberMax(key: TreatmentNumber) {
    const { max } = treatmentNumbers[key];
    const limit = Math.floor(sampleRate * 0.45);
    if (key === 'humHarmonics' && settings.hum !== 'off') {
      return Math.min(max, Math.floor(limit / (settings.hum === 'hz50' ? 50 : 60)));
    }
    return key.endsWith('Hz') ? Math.min(max, limit) : max;
  }

  function numeric(key: TreatmentNumber, value: number) {
    return Math.max(treatmentNumbers[key].min, Math.min(numberMax(key), Math.round(value)));
  }

  function available(key: TreatmentNumber) {
    return numberMax(key) >= treatmentNumbers[key].min;
  }

  function stored(key: TreatmentNumber) {
    return settings[key] || treatmentNumbers[key].initial;
  }

  function remember(key: Restorable, previous: number) {
    if (restoreSource !== snapshot?.sourceId) {
      restore = {};
      restoreSource = snapshot?.sourceId;
    }
    if (previous) restore[key] = previous;
    return restore[key];
  }

  function changeBand(target: Band, frequency: number, gain?: number, gesture?: string) {
    const filter = bands[target];
    if (!available(filter.frequency)) return;
    const hz = numeric(filter.frequency, frequency);
    if (target === 'low') onedit({ rumble: true, rumbleHz: hz }, gesture);
    else if ('gain' in filter) {
      const db = numeric(filter.gain, gain ?? (settings[filter.gain] || 2));
      onedit({ [filter.frequency]: hz, [filter.gain]: db }, gesture);
    } else onedit({ [filter.frequency]: hz }, gesture);
  }

  function toggleBand(target: Band, enabled: boolean) {
    const filter = bands[target];
    if (enabled) {
      const previous = remember(target, 0);
      changeBand(target, 'gain' in filter ? stored(filter.frequency) : previous ?? stored(filter.frequency),
        'gain' in filter ? previous ?? 2 : undefined);
      return;
    }
    const key = filter.enabled;
    if (key === 'rumble') onedit({ rumble: false });
    else {
      remember(target, settings[key]);
      onedit({ [key]: 0 });
    }
  }

  function toggleAmount(effect: 'compressor' | 'gate' | 'dc' | 'gain', key: TreatmentNumber) {
    const previous = remember(effect, settings[key]);
    onedit({ [key]: settings[key] ? 0 : previous ?? (effect === 'gain' ? 3 : treatmentNumbers[key].initial) });
  }

  function bandPosition(target: Band) {
    const filter = bands[target];
    const gain = 'gain' in filter ? settings[filter.gain] : target === 'notch' ? -15 : -3;
    return { x: frequencyX(stored(filter.frequency), bounds, sampleRate), y: gainY(gain, bounds) };
  }

  function updateNumber(key: TreatmentNumber, value: number) {
    const amount = numeric(key, value);
    if (kind === 'eq' && band && key === bands[band].frequency) changeBand(band, amount);
    else if (key === 'compressorRatio' && !settings.compressorDb) onedit({ compressorDb: compressorThreshold, compressorRatio: amount });
    else if (key === 'gateReductionDb' && !settings.gateDb) onedit({ gateDb: treatmentNumbers.gateDb.initial, gateReductionDb: amount });
    else if (key === 'normalizeTargetDb' && !settings.normalize) onedit({ normalize: true, normalizeTargetDb: amount });
    else onedit({ [key]: amount });
  }

  function startDrag(event: PointerEvent, mode?: DragMode) {
    if (!snapshot || event.button !== 0 || drag || (kind === 'dynamics' && dynamics !== 'compressor')) return;
    const button = event.currentTarget as HTMLButtonElement;
    const rect = button.parentElement!.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    const fraction = (x - bounds.left) / (bounds.right - bounds.left);
    const target = mode ?? (kind === 'eq'
      ? fraction < 0.20 ? 'low' : fraction > 0.80 ? 'high' : 'bell'
      : 'threshold');
    const existing = mode && mode in bands ? bandPosition(mode as Band)
      : mode === 'threshold' ? { x: levelX(compressorThreshold, bounds), y }
      : mode === 'ratio' ? { x, y: levelY(ratioOutput, bounds) }
      : undefined;
    drag = {
      pointerId: event.pointerId, button, sourceId: snapshot.sourceId, mode: target, gesture: id + '-' + ++gestureNumber,
      rect, bounds, x, y, lastX: x, lastY: y, targetX: existing?.x ?? x, targetY: existing?.y ?? y, started: false,
    };
    if (target in bands) {
      band = target as Band;
      editorOnLeft = drag.targetX > width * .60;
    }
    button.setPointerCapture(event.pointerId);
  }

  function accumulateDrag(current: Drag, event: PointerEvent) {
    const x = event.clientX - current.rect.left;
    const y = event.clientY - current.rect.top;
    const deltaX = x - current.lastX;
    const deltaY = y - current.lastY;
    const fine = event.shiftKey ? 0.2 : 1;
    current.lastX = x;
    current.lastY = y;
    current.targetX += deltaX * fine;
    current.targetY += deltaY * fine;
    current.started ||= Math.hypot(x - current.x, y - current.y) >= 3;
    return current.started && (deltaX !== 0 || deltaY !== 0);
  }

  function applyDrag(current: Drag) {
    if (current.sourceId !== snapshot?.sourceId) return;
    const { mode, targetX, targetY, gesture, bounds: geometry } = current;
    if (mode === 'threshold') {
      onedit({ compressorDb: numeric('compressorDb', xLevel(targetX, geometry)) }, gesture);
    } else if (mode === 'ratio') {
      const ratio = (ratioInput - compressorThreshold) / Math.max(1, yLevel(targetY, geometry) - compressorThreshold);
      onedit({ compressorDb: compressorThreshold, compressorRatio: numeric('compressorRatio', ratio) }, gesture);
    } else {
      changeBand(mode, xFrequency(targetX, geometry, sampleRate), yGain(targetY, geometry), gesture);
    }
  }

  function moveDrag(event: PointerEvent) {
    const current = drag;
    if (!current || event.pointerId !== current.pointerId || !accumulateDrag(current, event) || dragFrame) return;
    dragFrame = requestAnimationFrame(() => {
      dragFrame = 0;
      if (drag === current) applyDrag(current);
    });
  }

  function finishDrag(release?: PointerEvent) {
    const current = drag;
    if (!current) return;
    const pending = dragFrame !== 0;
    cancelAnimationFrame(dragFrame);
    dragFrame = 0;
    if (release) {
      const changed = accumulateDrag(current, release);
      if (pending || changed) applyDrag(current);
    }
    drag = undefined;
    editorOnLeft = undefined;
    if (current.button.hasPointerCapture(current.pointerId)) current.button.releasePointerCapture(current.pointerId);
  }

  function endDrag(event: PointerEvent) {
    if (drag?.pointerId !== event.pointerId) return;
    finishDrag(event.type === 'pointerup' ? event : undefined);
  }

  onDestroy(() => finishDrag());

  function bandKeys(event: KeyboardEvent, target: Band) {
    if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault();
      toggleBand(target, false);
      return;
    }
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault();
    const filter = bands[target];
    const direction = event.key === 'ArrowRight' || event.key === 'ArrowUp' ? 1 : -1;
    if ('gain' in filter && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
      changeBand(target, stored(filter.frequency), settings[filter.gain] + direction);
    } else if ('slope' in filter && event.shiftKey) {
      const slope = numeric(filter.slope, stored(filter.slope) + direction * 12);
      onedit({ [filter.slope]: slope });
    } else changeBand(target, Math.round(stored(filter.frequency) * (event.shiftKey ? 1.015 : 1.1) ** direction), undefined);
  }

  function compressorKeys(event: KeyboardEvent, key: 'compressorDb' | 'compressorRatio') {
    if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault();
      if (settings.compressorDb) toggleAmount('compressor', 'compressorDb');
      return;
    }
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault();
    updateNumber(key, stored(key) + (event.key === 'ArrowRight' || event.key === 'ArrowUp' ? 1 : -1));
  }

  function scrollBand(target: Band) {
    const filter = bands[target];
    return (node: HTMLButtonElement) => {
      const wheel = (event: WheelEvent) => {
        if (target !== band || !event.deltaY) return;
        const key = 'slope' in filter ? filter.slope : 'q' in filter ? filter.q : undefined;
        if (!key) return;
        event.preventDefault();
        const step = key === 'presenceQ' ? 10 : key === 'notchQ' ? 1 : 12;
        updateNumber(key, stored(key) - Math.sign(event.deltaY) * step);
      };
      node.addEventListener('wheel', wheel, { passive: false });
      return () => node.removeEventListener('wheel', wheel);
    };
  }

  function windowKeys(event: KeyboardEvent) {
    if (!snapshot || event.defaultPrevented || event.isComposing || event.altKey) return;
    const target = event.target;
    if (target instanceof Element && target.closest('input, textarea, select, [contenteditable=true]')) return;
    const key = event.key.toLowerCase();
    if (event.ctrlKey || event.metaKey) {
      if (key !== 'z' && key !== 'y') return;
      event.preventDefault();
      onaction(key === 'y' || event.shiftKey ? 'redo' : 'undo');
    } else if (key === ' ' && !event.repeat && !(target instanceof Element
      && target.closest('button:not(.plot-hit):not(.marker):not(.repair-hit), [role=option], [role=menuitem]'))) {
      event.preventDefault();
      onaction('play');
    }
  }
</script>

<svelte:window onkeydown={windowKeys} onblur={() => finishDrag()}/>

{#snippet choice(label: string, value: string | null, options: readonly Option[],
  controller: ReturnType<typeof createSelectController<string>>, onvalue: (value: string) => void)}
  <Select {controller} {value} disabled={!snapshot} onValueChange={value => { if (value !== null) onvalue(value); }}>
    <SelectTrigger class="tool-select" aria-label={label}>
      {options.find(option => option.value === value)?.label ?? label}<ChevronDown size={13} aria-hidden="true"/>
    </SelectTrigger>
    <SelectContent class="app-select-menu" align="start" sideOffset={4} collisionPadding={8} alignItemWithTrigger={false}>
      {#each options as option (option.value)}
        <SelectItem value={option.value} label={option.label} data-slot="select-item">{option.label}</SelectItem>
      {/each}
    </SelectContent>
  </Select>
{/snippet}

{#snippet number(key: TreatmentNumber, label = treatmentNumbers[key].label)}
  {@const spec = treatmentNumbers[key]}
  {@const ratio = key === 'presenceQ' ? 100 : 1}
  {@const max = Math.max(spec.min, numberMax(key))}
  <div class="parameter">
    <span>{label}</span>
    <NumberInput class="tool-number" value={Math.min(stored(key), max) / ratio} min={spec.min / ratio} max={max / ratio} step={1 / ratio}
      largeStep={(spec.step ?? 5) / ratio} disabled={!snapshot || !available(key)} onValueCommit={value => updateNumber(key, (value ?? spec.initial / ratio) * ratio)}>
      <NumberInputControl class="parameter-input" aria-label={spec.unit && spec.unit !== ':1' ? label + ' in ' + spec.unit : label}/>
      {#if spec.unit}<span class="unit" aria-hidden="true">{spec.unit}</span>{/if}
    </NumberInput>
  </div>
{/snippet}

{#snippet power(active: boolean, name: string, action: () => void, unavailable = false)}
  <Button variant="ghost" size="icon-sm" class="tool-power" disabled={!snapshot || unavailable} aria-label={name} title={name} aria-pressed={active} onclick={action}>
    <Power size={15} aria-hidden="true"/>
  </Button>
{/snippet}

<section class="tool-window" aria-label={title[kind]}>
  <header class="tool-header">
    <div class="transport" role="group" aria-label="Preview controls">
      <Button variant="ghost" size="icon-sm" disabled={!snapshot} aria-label={snapshot?.playing ? 'Pause' : 'Play'} title={snapshot?.playing ? 'Pause (Space)' : 'Play (Space)'} onclick={() => onaction('play')}>
        {#if snapshot?.playing}<Pause size={15} aria-hidden="true"/>{:else}<Play size={15} aria-hidden="true"/>{/if}
      </Button>
      <Button variant="ghost" size="icon-sm" class="tool-power" disabled={!snapshot} aria-label="Loop preview" title="Loop preview" aria-pressed={snapshot?.loop ?? false} onclick={() => onaction('loop')}>
        <Repeat2 size={16} aria-hidden="true"/>
      </Button>
      <div class="comparison" role="group" aria-label="Preview sound">
        <Button variant="ghost" size="sm" disabled={!snapshot} aria-pressed={snapshot?.original ?? false} onclick={() => onaction('original')}>Original</Button>
        <Button variant="ghost" size="sm" disabled={!snapshot} aria-pressed={Boolean(snapshot && !snapshot.original)} onclick={() => onaction('treated')}>Treated</Button>
      </div>
    </div>
    <div class="transport">
      {#if kind === 'eq'}
        <div class="legend" aria-hidden="true"><span>Original</span><span>Treated</span></div>
      {/if}
      <Menu controller={presetMenu}>
        <MenuTrigger class="tool-menu" disabled={!snapshot} aria-label="Tool presets" title="Tool presets"><Ellipsis size={17} aria-hidden="true"/></MenuTrigger>
        <MenuContent class="app-select-menu" align="end" sideOffset={4} collisionPadding={8}>
          {#each presets[kind] as preset (preset.label)}
            {@const cutoff = 'highCutHz' in preset.patch ? preset.patch.highCutHz : 0}
            <MenuItem data-slot="select-item" label={preset.label} disabled={cutoff > Math.floor(sampleRate * 0.45)}
              onSelect={() => onedit(preset.patch)}>{preset.label}</MenuItem>
          {/each}
          <MenuItem data-slot="select-item" label="Reset this tool" onSelect={() => { restore = {}; onedit(noTreatment); }}>Reset this tool</MenuItem>
        </MenuContent>
      </Menu>
    </div>
  </header>

  {#if kind === 'repair'}
    <AudioSplitRepairPlot {snapshot} {probe} {onviewport} {onedit} {onaction}/>
  {:else}
  <div class="plot" bind:clientWidth={width} bind:clientHeight={height} aria-label={plotName[kind]}
    aria-describedby={kind === 'dynamics' ? id + '-meters' : undefined} role="group">
    {#if kind === 'dynamics'}<span id={id + '-meters'} class="sr-only">{describeDynamicsMeters(probe)}</span>{/if}
    <canvas {@attach paint} aria-hidden="true"></canvas>
    <GraphButton class="plot-hit" type="button" aria-label={plotAction[kind]} disabled={!snapshot || (kind === 'dynamics' && dynamics !== 'compressor')} onpointerdown={event => startDrag(event)}
      onpointermove={moveDrag} onpointerup={endDrag} onpointercancel={endDrag} onlostpointercapture={endDrag}/>
    {#if kind === 'eq'}
      {#each markerBands as marker (marker.value)}
        {@const spot = bandPosition(marker.value)}
        {@const active = Boolean(settings[bands[marker.value].enabled])}
        <GraphButton type="button" class={['marker', { active, selected: marker.value === band }]} disabled={!snapshot}
          style={'left:' + spot.x + 'px;top:' + spot.y + 'px'} aria-label={marker.label} aria-pressed={marker.value === band} title={marker.label}
          onclick={() => band = marker.value}
          onpointerdown={event => startDrag(event, marker.value)} onpointermove={moveDrag} onpointerup={endDrag}
          onpointercancel={endDrag} onlostpointercapture={endDrag} onkeydown={event => bandKeys(event, marker.value)}
          {@attach scrollBand(marker.value)}/>
      {/each}
      {#if band}
        {@const target = band}
        {@const filter = bands[target]}
        <div class={['band-editor', { left: editorOnLeft ?? (bandPosition(target).x > width * .60) }]} role="group" aria-label={filter.label + ' settings'}>
          {@render power(Boolean(settings[filter.enabled]), filter.label + ' effect', () => toggleBand(target, !settings[filter.enabled]), !available(filter.frequency))}
          {@render number(filter.frequency, 'Frequency')}
          {#if 'gain' in filter}{@render number(filter.gain, 'Gain')}{/if}
          {#if 'slope' in filter}
            {@const key = filter.slope}
            <div class="parameter">
              <span>Slope</span>
              {@render choice('Slope', String(stored(key)), slopeOptions, slopeControl, value => updateNumber(key, Number(value)))}
            </div>
          {/if}
          {#if 'q' in filter}{@render number(filter.q, 'Q')}{/if}
        </div>
      {/if}
    {:else if kind === 'dynamics' && dynamics === 'compressor'}
      {@const active = Boolean(settings.compressorDb)}
      {#each [{ label: 'Compression threshold', mode: 'threshold', field: 'compressorDb', x: compressorThreshold, y: compressorThreshold },
        { label: 'Compression ratio', mode: 'ratio', field: 'compressorRatio', x: ratioInput, y: ratioOutput }] as marker (marker.mode)}
        <GraphButton type="button" class={['marker', { active }]} disabled={!snapshot}
          style={'left:' + levelX(marker.x, bounds) + 'px;top:' + levelY(marker.y, bounds) + 'px'} aria-label={marker.label} title={marker.label}
          onpointerdown={event => startDrag(event, marker.mode as 'threshold' | 'ratio')} onpointermove={moveDrag} onpointerup={endDrag}
          onpointercancel={endDrag} onlostpointercapture={endDrag}
          onkeydown={event => compressorKeys(event, marker.field as 'compressorDb' | 'compressorRatio')}/>
      {/each}
    {/if}
  </div>
  {/if}

  <div class="tool-side">
    <AudioSplitToolModes {kind} {settings} selected={kind === 'eq' ? band : kind === 'dynamics' ? dynamics : repair}
      enabled={!!snapshot} stereo={!!snapshot?.stereo} highCut={available('highCutHz')} onchoose={selectMode}/>
  </div>

  {#if kind !== 'repair' || repair !== 'selection'}
  <div class="tool-controls">
    {#if kind === 'eq'}
      {#if !band}
        <span class="hint">Drag near an edge for a cut, or in the middle for a bell.</span>
      {/if}
    {:else if kind === 'dynamics'}
      {#if dynamics === 'compressor'}
        {@render power(Boolean(settings.compressorDb), 'Compressor', () => toggleAmount('compressor', 'compressorDb'))}
        {@render number('compressorDb', 'Threshold')}
        {@render number('compressorRatio', 'Ratio')}
      {:else if dynamics === 'gate'}
        {@render power(Boolean(settings.gateDb), 'Quiet-gap reduction', () => toggleAmount('gate', 'gateDb'))}
        {@render number('gateDb', 'Threshold')}
        {@render number('gateReductionDb', 'Reduction')}
      {:else if dynamics === 'gain'}
        {@render power(Boolean(settings.gainDb), 'Gain', () => toggleAmount('gain', 'gainDb'))}
        {@render number('gainDb', 'Gain')}
      {:else}
        {@render power(settings.normalize, 'Normalise', () => onedit({ normalize: !settings.normalize }))}
        {@render number('normalizeTargetDb', 'Target')}
      {/if}
    {:else}
      {#if repair === 'hum'}
        {@render choice('Mains frequency', settings.hum, humOptions, humControl, value => onedit({ hum: value as AudioTreatment['hum'] }))}
        {@render number('humHarmonics', 'Harmonics')}
      {:else if repair === 'dc'}
        {@render power(Boolean(settings.dcHz), 'Remove DC offset', () => toggleAmount('dc', 'dcHz'), !available('dcHz'))}
        {@render number('dcHz', 'Cutoff')}
      {:else if snapshot?.stereo}
        {@render choice('Mono conversion', settings.mono, monoOptions, monoControl, value => onedit({ mono: value as AudioTreatment['mono'] }))}
        <Button variant="ghost" size="sm" class="tool-power" aria-pressed={settings.swapChannels} onclick={() => onedit({ swapChannels: !settings.swapChannels })}>Swap L/R</Button>
        {@render choice('Channel polarity', settings.polarity, polarityOptions, polarityControl, value => onedit({ polarity: value as AudioTreatment['polarity'] }))}
        {@render number('balance', 'Balance')}
        {@render number('stereoWidth', 'Width change')}
      {:else}
        <span class="hint">Stereo controls need a two-channel recording.</span>
      {/if}
    {/if}
  </div>
  {/if}
</section>

<style>
  .tool-window {
    min-height: max(100dvh, 342px);
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-rows: auto 1fr auto;
    gap: var(--sp-2, 8px);
    padding: var(--sp-3, 12px);
    background: var(--bg);
    color: var(--text);
    font-family: var(--font-ui, system-ui);
  }
  .tool-header, .transport, .tool-controls, .comparison, .legend {
    display: flex;
    align-items: center;
    gap: var(--sp-2, 8px);
  }
  .tool-header { grid-column: 1 / -1; justify-content: space-between; min-width: 0; }
  .tool-side { grid-column: 2; grid-row: 2 / 4; align-self: start; }
  .transport { gap: var(--sp-1, 4px); }
  .comparison {
    gap: 2px;
    padding: 2px;
    border-radius: var(--r-pill, 999px);
    background: var(--panel);
  }
  .comparison :global(button) { block-size: 24px; border-radius: var(--r-pill, 999px); }
  .comparison :global([aria-pressed=true]), :global(.tool-window .tool-power[aria-pressed=true]) {
    background: color-mix(in srgb, var(--accent) 18%, var(--panel));
    color: var(--accent-ink, var(--text));
  }
  .legend {
    margin-inline-end: var(--sp-2, 8px);
    color: var(--muted);
    font-size: .6875rem;
  }
  .legend span { display: flex; align-items: center; gap: 4px; }
  .legend span::before { content: ''; inline-size: 10px; border-block-start: 1px dashed currentColor; }
  .legend span:last-child::before { border-block-start-style: solid; }
  .legend span:last-child { color: var(--accent-ink, var(--text)); }
  :global(.tool-window .tool-menu) {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: var(--r-compact, 8px);
    background: transparent;
    color: var(--muted);
  }
  :global(.tool-window .tool-menu:hover) { background: var(--panel); color: var(--text); }
  :global(.tool-window .tool-menu:focus-visible) { outline: 2px solid var(--accent); }

  .plot {
    position: relative;
    isolation: isolate;
    overflow: hidden;
    overflow: clip;
    min-width: 0;
    min-height: 218px;
    border-radius: var(--r-inset, 14px);
    background: var(--panel);
  }
  canvas, .plot :global(.plot-hit) { position: absolute; inset: 0; width: 100%; height: 100%; border-radius: inherit; }
  canvas { pointer-events: none; }
  .plot :global(.plot-hit), .plot :global(.marker) {
    appearance: none;
    background: transparent;
    color: inherit;
    border: 0;
    padding: 0;
    touch-action: none;
  }
  .plot :global(.plot-hit) { cursor: crosshair; }
  .plot :global(.plot-hit:disabled), .plot :global(.marker:disabled) { cursor: default; }
  .plot :global(.plot-hit:focus-visible) { outline: 2px solid var(--accent); outline-offset: -2px; }
  .plot :global(.marker) {
    position: absolute;
    transform: translate(-50%, -50%);
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    cursor: grab;
  }
  .plot :global(.marker:active) { cursor: grabbing; }
  .plot :global(.marker)::before {
    content: '';
    width: 11px;
    height: 11px;
    border-radius: inherit;
    border: 1.5px solid var(--accent-ink, var(--text));
    background: var(--panel);
  }
  .plot :global(.marker.active)::before { background: var(--accent); }
  .plot :global(.marker:hover), .plot :global(.marker.selected) { background: color-mix(in srgb, var(--accent) 23%, transparent); }
  .plot :global(.marker:focus-visible) { outline: 2px solid var(--accent); outline-offset: -1px; }
  .plot :global(.marker:focus-visible), .plot :global(.marker.selected) { z-index: 1; }
  .band-editor {
    position: absolute;
    z-index: 2;
    inset-inline-end: 33px;
    inset-block-end: 29px;
    display: flex;
    align-items: end;
    gap: var(--sp-2, 8px);
    padding: var(--sp-2, 8px);
    border-radius: var(--r-inset, 14px);
    background: var(--bg);
    box-shadow: 0 5px 16px rgb(0 0 0 / 22%);
  }
  .band-editor.left { inset-inline: 33px auto; }

  .tool-controls { min-height: 46px; padding-inline: var(--sp-1, 4px); flex-wrap: wrap; }
  .parameter {
    display: grid;
    gap: 2px;
    color: var(--muted);
    font-size: .6875rem;
    font-weight: 500;
  }
  .parameter :global(.tool-number), :global(.tool-window .tool-select) {
    display: inline-flex;
    align-items: center;
    min-height: 28px;
    border-radius: var(--r-control, 10px);
    border: 0;
    background: var(--panel);
    color: var(--text);
    font-size: .75rem;
  }
  .parameter :global(.tool-number) { padding-inline: 8px; }
  .parameter :global(.parameter-input) {
    width: 6ch;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    outline: none;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .parameter :global(.tool-number:focus-within), :global(.tool-window .tool-select:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  :global(.tool-window .tool-select) { gap: 7px; justify-content: space-between; padding-inline: 9px; }
  :global(.tool-window .tool-select:hover) { background: color-mix(in srgb, var(--panel) 90%, var(--text)); }
  .unit { color: var(--muted); margin-inline-start: 4px; }
  .hint { color: var(--muted); font-size: .75rem; }
  @media (max-width: 385px) { .legend { display: none; } }
</style>
