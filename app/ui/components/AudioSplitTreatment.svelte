<script lang="ts">
  import { ChevronDown, RotateCcw, X } from '@lucide/svelte';
  import { onDestroy, tick } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { Button } from '@ui-sdk/svelte/button';
  import { createMenuController, Menu, MenuContent, MenuItem, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';
  import { createSelectController, Select, SelectContent, SelectItem, SelectTrigger } from '@ui-sdk/svelte/select';
  import { Slider, SliderControl, SliderIndicator, SliderThumb, SliderTrack } from '@ui-sdk/svelte/slider';
  import { Tabs, TabsList, TabsPanel, TabsTab } from '@ui-sdk/svelte/tabs';
  import { Switch } from '$lib/components/ui/switch/index.js';
  import {
    audioEffects, audioTreatmentCount, effectCategories, effectIsActive, noTreatment, readTreatment,
    sameTreatment, treatmentNumbers, type AudioTreatment, type TreatmentNumber,
  } from '../js/audio-treatment';
  import type { TreatmentAnalysis } from '../js/audio-split';

  let {
    settings, stereo, original, busy, analysis, error, onchange, oncompare,
    sampleRate, onretry, onclose, oneditstart, oneditend, panelId = 'studio-audio-tools',
  }: {
    settings: Partial<AudioTreatment>;
    stereo: boolean;
    original: boolean;
    busy: boolean;
    analysis: TreatmentAnalysis | null;
    error: string;
    onchange: (settings: AudioTreatment) => void;
    oncompare?: () => void;
    sampleRate?: number;
    panelId?: string;
    onretry?: () => void;
    onclose?: () => void;
    oneditstart?: () => void;
    oneditend?: () => void;
  } = $props();

  const id = $props.id();
  const current = $derived(readTreatment(settings));
  const count = $derived(audioTreatmentCount(current));
  const gain = $derived(analysis?.gainDb ?? 0);
  const frequencyLimit = $derived(sampleRate ? Math.floor(sampleRate * 0.45) : Infinity);
  const groups = ['In use', ...effectCategories];
  const presetMenu = createMenuController();
  const selectControllers = { hum: createSelectController(), mono: createSelectController(), polarity: createSelectController() };
  let group = $state<string | null>('Repair');
  let sliding = $state<TreatmentNumber>();
  let sliderAtNeutral = false;
  let sliderPointer: number | undefined;
  let sliderEndTimer: ReturnType<typeof setTimeout>;
  const previousAmounts = new SvelteMap<string, number>();
  const hasSettings = $derived(previousAmounts.size > 0 || !sameTreatment(current, noTreatment));

  const humOptions = [
    { value: 'off', label: 'Off' }, { value: 'hz50', label: '50 Hz' }, { value: 'hz60', label: '60 Hz' },
  ] as const;
  const monoOptions = [
    { value: 'off', label: 'Keep stereo' }, { value: 'mix', label: 'Mix left and right' },
    { value: 'left', label: 'Left only' }, { value: 'right', label: 'Right only' },
  ] as const;
  const polarityOptions = [
    { value: 'off', label: 'Off' }, { value: 'left', label: 'Invert left' }, { value: 'right', label: 'Invert right' },
  ] as const;

  const presets = [
    { name: 'Tape cleanup (50 Hz)', detail: 'DC offset, rumble and mains hum', patch: { dcHz: 5, rumble: true, hum: 'hz50', humHarmonics: 3 } },
    { name: 'Tape cleanup (60 Hz)', detail: 'DC offset, rumble and mains hum', patch: { dcHz: 5, rumble: true, hum: 'hz60', humHarmonics: 3 } },
    { name: 'Soften hiss', detail: 'Reduce the highs above 10 kHz', patch: { highCutHz: 10000 } },
    { name: 'Smooth peaks', detail: 'Gentle 2:1 compression', patch: { compressorDb: -18, compressorRatio: 2 } },
  ] as const satisfies readonly { name: string; detail: string; patch: Partial<AudioTreatment> }[];

  function change(patch: Partial<AudioTreatment>) {
    const next = readTreatment({ ...current, ...patch });
    if (sameTreatment(next, current)) return;
    const losesRow = group === 'In use' && !sliding && audioEffects.some(effect =>
      effectIsActive(current, effect.key) && !effectIsActive(next, effect.key),
    );
    onchange(next);
    if (losesRow) void focusActiveTab();
  }

  async function focusActiveTab(force = false) {
    await tick();
    if (force || document.activeElement === document.body) {
      document.getElementById(panelId)?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus({ preventScroll: true });
    }
  }

  function numberMax(key: TreatmentNumber) {
    const { max } = treatmentNumbers[key];
    if (key === 'humHarmonics' && current.hum !== 'off') {
      return Math.min(max, Math.max(1, Math.floor(frequencyLimit / (current.hum === 'hz50' ? 50 : 60))));
    }
    return key === 'highCutHz' || key === 'notchHz' || key === 'rumbleHz' || key === 'dcHz' ? Math.min(max, frequencyLimit) : max;
  }

  function setNumber(key: TreatmentNumber, value: number) {
    const { min } = treatmentNumbers[key];
    change({ [key]: Math.max(min, Math.min(numberMax(key), Math.round(value))) });
  }

  function toggleNumber(key: TreatmentNumber, enabled: boolean) {
    if (!enabled && current[key]) previousAmounts.set(key, current[key]);
    const amount = previousAmounts.get(key) ?? treatmentNumbers[key].initial;
    change({ [key]: enabled ? Math.min(amount, numberMax(key)) : 0 });
  }

  function sliderChange(key: TreatmentNumber, value: number | number[]) {
    if (typeof value !== 'number') return;
    if (sliding !== key) {
      finishSlider();
      sliding = key;
      oneditstart?.();
    }
    sliderAtNeutral = treatmentNumbers[key].initial === 0 && value === 0;
    setNumber(key, value);
  }

  function finishSlider() {
    clearTimeout(sliderEndTimer);
    if (!sliding) return;
    const neutral = group === 'In use' && sliderAtNeutral;
    const focused = document.activeElement;
    const fromSlider = focused instanceof HTMLInputElement && focused.type === 'range'
      && focused.getAttribute('aria-labelledby') === `${id}-number-${sliding}`;
    sliding = undefined;
    sliderAtNeutral = false;
    oneditend?.();
    if (neutral) void focusActiveTab(fromSlider);
  }

  function endSliderInteraction(event: Event) {
    if (event instanceof PointerEvent) {
      if (sliderPointer !== undefined && sliderPointer !== event.pointerId) return;
      sliderPointer = undefined;
    } else if (event.type === 'blur') sliderPointer = undefined;
    else if (sliderPointer !== undefined) return;
    if (!sliding || event instanceof KeyboardEvent && !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End'].includes(event.key)) return;
    // The SDK omits its commit callback when a drag returns to its initial
    // value. Close our history gesture after the SDK processes the release.
    clearTimeout(sliderEndTimer);
    sliderEndTimer = setTimeout(finishSlider, 0);
  }

  onDestroy(finishSlider);

  function popupKeys(event: KeyboardEvent, popup: { close: () => void; focus: () => void }) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    popup.close();
    popup.focus();
  }

  function applyPreset(patch: Partial<AudioTreatment>) {
    change(patch);
    group = 'In use';
  }

  function resetEffect(key: string, reset: Partial<AudioTreatment>) {
    previousAmounts.delete(key);
    change(reset);
  }

  function resetAll() {
    previousAmounts.clear();
    change(noTreatment);
  }

  function closePanel() {
    finishSlider();
    onclose?.();
  }

  function presetAvailable(patch: Partial<AudioTreatment>) {
    const hum = patch.hum === 'hz50' ? 50 : patch.hum === 'hz60' ? 60 : 0;
    return Math.max(patch.highCutHz ?? 0, hum * (patch.humHarmonics || 1)) <= frequencyLimit;
  }

  function filterAvailable(key: string) {
    switch (key) {
      case 'highCutHz': return frequencyLimit >= treatmentNumbers.highCutHz.min;
      case 'notchHz': return frequencyLimit >= treatmentNumbers.notchHz.min;
      case 'rumble': return frequencyLimit >= 35;
      case 'dcHz': return frequencyLimit >= treatmentNumbers.dcHz.min;
      case 'bassDb': return frequencyLimit >= 120;
      case 'presenceDb': return frequencyLimit >= 1500;
      case 'trebleDb': return frequencyLimit >= 4000;
      default: return true;
    }
  }

  const visibleEffects = $derived(audioEffects.filter(effect =>
    group === 'In use' ? effectIsActive(current, effect.key) || !!sliding && effect.numbers?.includes(sliding) : group === effect.category,
  ));
</script>

<svelte:window onpointerupcapture={endSliderInteraction} onpointercancelcapture={endSliderInteraction}
  onkeyupcapture={endSliderInteraction} onblur={endSliderInteraction}/>

{#snippet selectControl<V extends string>(
  value: V,
  options: readonly { label: string; value: V }[],
  labelId: string,
  control: keyof typeof selectControllers,
  update: (value: V) => void,
)}
  <Select controller={selectControllers[control]} {value} onValueChange={value => { if (value !== null) update(value); }}>
    <SelectTrigger class="effect-select" aria-labelledby={labelId}>
      {options.find(option => option.value === value)?.label}<ChevronDown size={13} aria-hidden="true"/>
    </SelectTrigger>
    <SelectContent class="app-select-menu studio-effect-options" sideOffset={4} align="end" collisionPadding={12} alignItemWithTrigger={false}
      onkeydowncapture={event => popupKeys(event, selectControllers[control])}>
      {#each options as option (option.value)}
        <SelectItem value={option.value} label={option.label} class="studio-effect-option" data-slot="select-item">{option.label}</SelectItem>
      {/each}
    </SelectContent>
  </Select>
{/snippet}

{#snippet numberControl(key: TreatmentNumber, onlyInput: boolean)}
  {@const spec = treatmentNumbers[key]}
  {@const max = numberMax(key)}
  {@const value = Math.min(max, current[key] || spec.initial)}
  {@const labelId = `${id}-number-${key}`}
  {@const label = key === 'stereoWidth' ? `${value + 100}% width`
    : key === 'balance' ? (value === 0 ? 'Centered' : `${Math.abs(value)}% ${value < 0 ? 'left' : 'right'}`)
    : `${value}${spec.unit ? ' ' + spec.unit : ''}`}
  <div class={['effect-number', { 'stepper-only': onlyInput }]} onfocusout={endSliderInteraction}>
    <span id={labelId}>{spec.label}</span>
    {#if !onlyInput}
      <Slider class="effect-slider" {value} min={spec.min} {max} largeStep={spec.step ?? 5}
        format={() => label} onValueChange={value => sliderChange(key, value)} onValueCommit={finishSlider}>
        <SliderControl class="effect-slider-control" onpointerdowncapture={event => { sliderPointer ??= event.pointerId; }}>
          <SliderTrack class="effect-slider-track"><SliderIndicator class="effect-slider-fill"/></SliderTrack>
          <SliderThumb class="effect-slider-thumb" aria-labelledby={labelId}/>
        </SliderControl>
      </Slider>
    {/if}
    <NumberInput class="effect-number-input" {value} min={spec.min} {max} step={1} smallStep={1}
      largeStep={spec.step ?? 5}
      onValueCommit={value => setNumber(key, value ?? spec.initial)}>
      <NumberInputControl class="effect-value" aria-label={`${spec.label}${spec.unit ? ' in ' + spec.unit : ''}`}/>
      {#if spec.unit}<span aria-hidden="true" class="unit">{spec.unit}</span>{/if}
    </NumberInput>
  </div>
{/snippet}

<section id={panelId} class="audio-tools-panel" aria-labelledby={`${id}-heading`}>
  <div class="tools-header">
    <div class="tools-heading">
      <h3 id={`${id}-heading`}>Audio tools</h3>
      {#if count}<span class="effect-count">{count} active</span>{/if}
    </div>
    <div class="tools-actions">
      <Menu controller={presetMenu}>
        <MenuTrigger class="effect-button" title="Add a starting point to your current settings">
          Starting points <ChevronDown size={13} aria-hidden="true"/>
        </MenuTrigger>
        <MenuContent class="app-select-menu studio-effect-options" sideOffset={4} align="end" collisionPadding={12}
          onkeydowncapture={event => popupKeys(event, presetMenu)}>
          {#each presets as preset (preset.name)}
            <MenuItem class="studio-effect-option" data-slot="select-item" label={preset.name} disabled={!presetAvailable(preset.patch)} onSelect={() => applyPreset(preset.patch)}>
              <span>{preset.name}</span><small>{preset.detail}</small>
            </MenuItem>
          {/each}
        </MenuContent>
      </Menu>
      <Button class="effect-button" disabled={!hasSettings} onclick={resetAll}>Reset all</Button>
      {#if onclose}<Button class="effect-button icon-button" aria-label="Hide audio tools" title="Hide audio tools" onclick={closePanel}><X size={16}/></Button>{/if}
    </div>
  </div>

  <Tabs bind:value={group} class="effects-tabs" onValueChange={finishSlider}>
    <TabsList class="effects-tab-list" aria-label="Audio tools categories">
      {#each groups as category (category)}
        <TabsTab class="effects-tab" value={category}>
          {category}{#if category === 'In use' && count}<span class="tab-count">{count}</span>{/if}
        </TabsTab>
      {/each}
    </TabsList>

    {#each groups as category (category)}
      <TabsPanel value={category}>
        {#if category === group}
          {#if visibleEffects.length === 0}
            <p class="empty-tools">Turn on a tool to see it here.</p>
          {:else if category === 'Stereo' && !stereo}
            <p class="empty-tools">Stereo tools are available for recordings with two channels.</p>
          {:else}
            <div class="effect-grid">
              {#each visibleEffects as effect (effect.key)}
                {@const active = effectIsActive(current, effect.key)}
                {@const labelId = `${id}-${effect.key}`}
                {@const unavailable = !filterAvailable(effect.key)}
                {@const showNumbers = effect.numbers && !unavailable && (active || effect.numbers.length === 1 && treatmentNumbers[effect.numbers[0]].initial === 0)}
                <div class={['effect', { active }]}>
                  <div class="effect-heading">
                    <h4 id={labelId}>{effect.label}</h4>
                    {#if effect.key === 'rumble' || effect.key === 'normalize' || effect.key === 'swapChannels'}
                      {@const key = effect.key}
                      <Switch size="sm" aria-labelledby={labelId} disabled={unavailable} checked={current[key]} onCheckedChange={checked => change({ [key]: checked })}/>
                    {:else if effect.key === 'hum'}
                      {@render selectControl(current.hum, humOptions, labelId, 'hum', hum => change({ hum }))}
                    {:else if effect.key === 'mono'}
                      {@render selectControl(current.mono, monoOptions, labelId, 'mono', mono => change({ mono }))}
                    {:else if effect.key === 'polarity'}
                      {@render selectControl(current.polarity, polarityOptions, labelId, 'polarity', polarity => change({ polarity }))}
                    {:else if effect.key === 'dcHz' || effect.key === 'highCutHz' || effect.key === 'notchHz' || effect.key === 'compressorDb' || effect.key === 'gateDb'}
                      {@const key = effect.key}
                      <Switch size="sm" aria-labelledby={labelId} disabled={unavailable} checked={active}
                        onCheckedChange={checked => toggleNumber(key, checked)}/>
                    {/if}
                    {#if active || previousAmounts.has(effect.key) || Object.keys(effect.reset).some(key => current[key as keyof AudioTreatment] !== noTreatment[key as keyof AudioTreatment])}
                      <Button class="effect-button icon-button reset-effect" aria-label={`Reset ${effect.label}`} title={`Reset ${effect.label}`}
                        onclick={() => resetEffect(effect.key, effect.reset)}><RotateCcw size={13}/></Button>
                    {/if}
                  </div>
                  <p id={`${id}-detail-${effect.key}`} class="effect-detail">{unavailable ? 'Requires a higher recording sample rate.' : effect.description}</p>
                  {#if showNumbers && effect.numbers}
                    <div class="effect-fields">
                      {#each effect.numbers as key (key)}
                        {@render numberControl(key, key === 'humHarmonics' || key === 'compressorRatio' || key === 'notchQ' || key === 'gateReductionDb')}
                      {/each}
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        {/if}
      </TabsPanel>
    {/each}
  </Tabs>

  {#if error || busy || (current.normalize && analysis) || (oncompare && count > 0)}
    <div class="tools-feedback">
      {#if error}
        <p class="tool-error" role="alert">{error}</p>
        {#if onretry}<Button class="effect-button" onclick={onretry}>Retry analysis</Button>{/if}
      {:else if busy}
        <p role="status">Measuring recording levels…</p>
      {:else if current.normalize && analysis}
        <p>
          {#if analysis.loudnessLufs === null}No measurable loudness in this recording.
          {:else}Whole-recording adjustment {gain > 0 ? '+' : ''}{gain.toFixed(1)} dB{/if}
        </p>
      {/if}
      {#if oncompare && count > 0}
        <Button class="effect-button compare-button" aria-pressed={original} onclick={oncompare}>
          {original ? 'Hear treated' : 'Hear original'}
        </Button>
      {/if}
    </div>
  {/if}
</section>

<style>
  .audio-tools-panel { min-width:0; padding:var(--sp-3); display:grid; gap:var(--sp-2); background:var(--panel); border-radius:var(--r-inset); }
  .tools-header, .tools-heading, .tools-actions, .effect-heading { display:flex; align-items:center; gap:var(--sp-2); }
  .tools-header { justify-content:space-between; flex-wrap:wrap; }
  .tools-heading h3 { margin:0; font-size:var(--text-sm); font-weight:var(--fw-strong); }
  .tools-heading .effect-count { color:var(--muted); font-size:var(--text-xs); white-space:nowrap; }
  .tools-actions { flex-wrap:wrap; gap:var(--sp-1); }
  .tools-actions :global(.effect-button), .tools-feedback :global(.effect-button), .effect :global(.effect-button) {
    display:inline-flex; align-items:center; justify-content:center; gap:var(--sp-1); min-height:28px; padding-inline:var(--sp-2);
    border:0; border-radius:var(--r-compact); color:var(--text); background:transparent; font:inherit; cursor:pointer;
  }
  .audio-tools-panel :global(.effect-button.icon-button) { width:28px; padding:0; color:var(--muted); }
  .audio-tools-panel :global(.effect-button:is(:hover,[aria-expanded="true"]):not(:disabled)) { background:var(--row-hover); color:var(--text); }
  .audio-tools-panel :global(.effect-button:disabled), .audio-tools-panel :global(.effect-select:disabled) { opacity:.5; cursor:default; }
  .audio-tools-panel :global(:is(.effect-button,.effect-select,.effects-tab):focus-visible), .audio-tools-panel :global(.effect-value:focus-visible) {
    outline:2px solid var(--accent); outline-offset:2px;
  }
  .audio-tools-panel :global(.effects-tab-list) {
    display:flex; flex-wrap:wrap; gap:var(--sp-1); padding:var(--sp-1); background:var(--bg); border-radius:var(--r-control);
  }
  .audio-tools-panel :global(.effects-tab) {
    display:flex; align-items:center; gap:var(--sp-1); min-height:28px; padding-inline:var(--sp-2); border:0; border-radius:var(--r-compact);
    background:transparent; color:var(--muted); font:inherit; white-space:nowrap; cursor:pointer;
  }
  .audio-tools-panel :global(.effects-tab[aria-selected="true"]) { color:var(--accent-ink); background:var(--sel-tint); }
  .audio-tools-panel :global(.effects-tab:hover:not([aria-selected="true"])) { color:var(--text); background:var(--row-hover); }
  .tab-count { font-size:var(--text-xs); }
  .effect-grid {
    display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,250px),1fr)); align-items:start;
    column-gap:var(--sp-4); row-gap:var(--sp-3); padding:var(--sp-3) var(--sp-1) var(--sp-1);
  }
  .effect { min-width:0; display:grid; gap:var(--sp-1); }
  .effect-heading { min-height:28px; gap:var(--sp-1); }
  .effect-heading h4 { flex:1; min-width:0; margin:0; color:var(--text); font-size:var(--text-sm); font-weight:var(--fw-strong); }
  .effect.active h4 { color:var(--accent-ink); }
  .effect-detail, .empty-tools, .tools-feedback p { margin:0; color:var(--muted); font-size:var(--text-xs); }
  .empty-tools { padding:var(--sp-3) var(--sp-1); }
  .effect-fields { display:grid; gap:var(--sp-1); }
  .effect-number { display:grid; grid-template-columns:minmax(0,1fr) minmax(38px,.75fr) auto; align-items:center; gap:var(--sp-2); color:var(--text); font-size:var(--text-xs); }
  .effect-number.stepper-only { grid-template-columns:minmax(0,1fr) auto; }
  .effect :global(.effect-number-input) {
    display:flex; align-items:center; gap:var(--sp-1); height:28px; padding-inline:var(--sp-1) var(--sp-2); min-width:81px;
    border-radius:var(--r-control); background:var(--bg);
  }
  .effect :global(.effect-value) {
    border:0; padding-inline:var(--sp-1); width:5.5ch; height:24px; background:transparent;
    color:var(--text); font:inherit; font-variant-numeric:tabular-nums; border-radius:var(--r-compact);
  }
  .unit { color:var(--muted); white-space:nowrap; }
  .effect :global(.effect-slider-control) { position:relative; display:flex; align-items:center; min-height:24px; width:100%; touch-action:none; cursor:pointer; }
  .effect :global(.effect-slider-track) { position:relative; display:block; width:100%; height:4px; border-radius:var(--r-pill); background:var(--border); }
  .effect :global(.effect-slider-fill) { height:100%; border-radius:var(--r-pill); background:var(--accent); }
  .effect :global(.effect-slider-thumb) {
    display:block; width:12px; height:12px; border-radius:50%; background:var(--accent); box-shadow:0 1px 2px rgb(0 0 0 / .18);
  }
  .effect :global(.effect-slider-thumb:has(input:focus-visible)) { outline:2px solid var(--accent); outline-offset:3px; }
  .effect :global(.effect-select) {
    display:inline-flex; justify-content:space-between; align-items:center; gap:var(--sp-1); min-height:28px; max-width:60%; padding-inline:var(--sp-2);
    border:0; border-radius:var(--r-control); color:var(--text); background:var(--bg); font:inherit; cursor:pointer;
  }
  .tools-feedback { display:flex; align-items:center; flex-wrap:wrap; gap:var(--sp-1) var(--sp-2); min-height:28px; }
  .tools-feedback p { flex:1; }
  .tools-feedback .tool-error { color:var(--danger); }
  .tools-feedback :global(.compare-button) { margin-inline-start:auto; }
  :global(.studio-effect-options.app-select-menu .studio-effect-option) { display:grid; padding:var(--sp-1) var(--sp-2); min-height:28px; align-content:center; border-radius:var(--r-menu-item); cursor:pointer; }
  :global(.studio-effect-options.app-select-menu .studio-effect-option:is([data-highlighted],[aria-selected="true"])) { color:var(--accent-ink); background:var(--sel-tint); }
  :global(.studio-effect-options.app-select-menu .studio-effect-option[aria-disabled="true"]) { opacity:.5; cursor:default; }
  :global(.studio-effect-options.app-select-menu .studio-effect-option small) { color:var(--muted); font-size:var(--text-xs); }
  .effect :global(.reset-effect) { flex-shrink:0; }
  @media (max-width:480px) {
    .effect-number { grid-template-columns:minmax(5rem,1fr) minmax(28px,.6fr) auto; gap:var(--sp-1); }
  }
</style>
