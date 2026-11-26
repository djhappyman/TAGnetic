<script lang="ts">
  import { tick } from 'svelte';
  import { Check, ChevronDown, ChevronLeft, ChevronRight, Ellipsis, Pause, Pin, Play, Power, Repeat2, Scale, X } from '@lucide/svelte';
  import { createMenuController, Menu, MenuContent, MenuGroupLabel, MenuItem, MenuItemIndicator, MenuRadioGroup, MenuRadioItem, MenuSeparator, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { createSelectController, Select, SelectContent, SelectItem, SelectTrigger } from '@ui-sdk/svelte/select';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';
  import AudioSplitDynamicsPanel from './AudioSplitDynamicsPanel.svelte';
  import AudioSplitEqPanel from './AudioSplitEqPanel.svelte';
  import AudioSplitRepairPlot from './AudioSplitRepairPlot.svelte';
  import AudioSplitToolModes, { isToolMode } from './AudioSplitToolModes.svelte';
  import Button from './AudioSplitButton.svelte';
  import { findEqNode, legacyEqBands, visibleEqNodes } from '../js/audio-eq';
  import { dynamicsMode, dynamicsPresets } from '../js/audio-dynamics';
  import type { FrameRange } from '../js/audio-split';
  import { noTreatment, treatmentNumbers, type AudioTreatment, type TreatmentNumber } from '../js/audio-treatment';
  import type { TreatmentPlotMode, TreatmentProbe } from '../js/audio-treatment-plot';
  import { toolPatch, type PreviewBand, type SpectrogramViewOptions, type ToolAction, type ToolSnapshot } from '../js/audio-tools-window';

  let { kind, snapshot, probe, onedit, onaction, onviewport, onanalyzer }: {
    kind: TreatmentPlotMode;
    snapshot: ToolSnapshot | null;
    probe: TreatmentProbe | null;
    onanalyzer: (enabled: boolean) => void;
    onedit: (patch: Partial<AudioTreatment>, gesture?: string, compareEq?: Partial<AudioTreatment> | null) => void;
    onaction: (action: ToolAction, range?: FrameRange, band?: PreviewBand) => void;
    onviewport: (range: FrameRange | null, options?: SpectrogramViewOptions) => void;
  } = $props();

  type Option = { value: string; label: string };
  type Preset = { label: string; patch: Partial<AudioTreatment> };
  type CompareSlot = 'A' | 'B';
  const title = { eq: 'Equaliser', dynamics: 'Dynamics', repair: 'Repair' };
  const defaultMode = { eq: null, dynamics: 'compressor', repair: 'selection' };
  const presetMenu = createMenuController();
  const humControl = createSelectController<string>();
  const monoControl = createSelectController<string>();
  const polarityControl = createSelectController<string>();
  const humOptions = [{ value: 'off', label: 'Off' }, { value: 'hz50', label: '50 Hz' }, { value: 'hz60', label: '60 Hz' }];
  const monoOptions = [{ value: 'off', label: 'Keep stereo' }, { value: 'mix', label: 'Mix to mono' },
    { value: 'left', label: 'Use left only' }, { value: 'right', label: 'Use right only' }];
  const polarityOptions = [{ value: 'off', label: 'Off' }, { value: 'left', label: 'Invert left' }, { value: 'right', label: 'Invert right' }];
  const eqPresets: readonly Preset[] = [
    { label: 'Flat', patch: {} },
    { label: 'Reduce rumble', patch: { rumble: true, rumbleHz: 35 } },
    { label: 'Soften hiss', patch: { highCutHz: 10000 } },
  ];
  const flatEq = toolPatch('eq', noTreatment);
  const repairPresets = [
    { label: 'Tape cleanup (50 Hz)', patch: { dcHz: 5, hum: 'hz50', humHarmonics: 3 } },
    { label: 'Tape cleanup (60 Hz)', patch: { dcHz: 5, hum: 'hz60', humHarmonics: 3 } },
  ] as const;
  const id = $props.id();
  // Each native tool document keeps one kind for its lifetime.
  let mode = $state<string | null>(readMode());
  let help = $state(false);
  let eqComparison = $state.raw<{ sourceId: string; slot: CompareSlot; A: Partial<AudioTreatment>; B: Partial<AudioTreatment> }>();
  let restoreDc = 0;
  const settings = $derived(snapshot?.settings ?? noTreatment);
  const selectedShape = $derived(kind === 'eq' ? findEqNode(settings, mode)?.shape ?? '' : '');
  function chooseEqShape(value: string) {
    setMode(visibleEqNodes(settings, null).find(node => node.shape === value)?.key ?? value);
  }
  const sampleRate = $derived(snapshot?.sampleRate || 44100);
  const presets: readonly Preset[] = $derived(
    kind === 'eq' ? eqPresets.filter(preset => (preset.patch.highCutHz ?? 0) <= Math.floor(sampleRate * .45))
      : kind === 'dynamics' ? dynamicsPresets[dynamicsMode(mode ?? '')] : repairPresets,
  );
  const eqPresetIndex = $derived.by(() => {
    if (kind !== 'eq' || settings.eqBands?.length || settings.eqBypassed) return -1;
    const current = visibleEqNodes(settings, null);
    return presets.findIndex(preset => {
      const expected = visibleEqNodes({ ...noTreatment, ...preset.patch }, null);
      return current.length === expected.length && current.every((node, index) => {
        const band = expected[index];
        return node.shape === band.shape && node.hz === band.hz && node.gainDb === band.gainDb
          && node.q === band.q && node.slope === band.slope;
      });
    });
  });
  const eqPresetName = $derived(presets[eqPresetIndex]?.label ?? 'Custom EQ');
  const compareSlots = $derived(eqComparison?.sourceId === snapshot?.sourceId ? eqComparison : undefined);
  const modKey = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';
  const commonKeys = [['Space', 'Play or pause'], ['L', 'Toggle loop'], ['B', 'Compare Original and Treated'],
    [modKey + '+Z', 'Undo'], [modKey + '+Shift+Z', 'Redo'], ['?', 'Show keyboard shortcuts']];
  const details = {
    eq: [['Enter', 'Add a band or edit its exact values'], ['Arrows', 'Move the focused band'], ['Shift+arrows', 'Move in finer steps'], ['Delete', 'Remove the focused band']],
    dynamics: [['Arrows', 'Adjust the focused handle'], ['Shift+arrows', 'Adjust in finer steps']],
    repair: [['Enter', 'Audition the selection'], [modKey + '+Enter', 'Apply the repair'], ['Delete', 'Remove a selected repair'],
      ['F', 'Fit the selection'], ['0', 'Reset zoom'], ['Shift+scroll', 'Zoom frequencies']],
  };

  function readMode() {
    try {
      const stored = localStorage.getItem('tagnetic.audioToolMode.' + kind);
      if (stored && isToolMode(kind, stored)) return stored;
    } catch { /* The window works without saved preferences. */ }
    return defaultMode[kind];
  }

  function setMode(value: string | null) {
    const band = kind === 'eq' && value !== null && /^extra-(?:draft-)?\d+$/.test(value);
    if (mode === value || (value !== null && !band && !isToolMode(kind, value))) return;
    mode = value;
    try {
      const key = 'tagnetic.audioToolMode.' + kind;
      if (value === null || band) localStorage.removeItem(key);
      else localStorage.setItem(key, value);
    } catch { /* Keep the current in-window selection. */ }
  }

  function applyPreset(preset: Preset) {
    if (kind === 'eq') {
      setMode(null);
      onedit({ ...flatEq, ...preset.patch });
    } else onedit(preset.patch);
  }

  function stepEqPreset(direction: -1 | 1) {
    if (!snapshot) return;
    const index = eqPresetIndex < 0 && direction < 0 ? 0 : eqPresetIndex;
    applyPreset(presets[(index + direction + presets.length) % presets.length]);
  }

  function switchEqComparison() {
    if (!snapshot?.eqLevelComparison || snapshot.reference?.preview) return;
    const current = toolPatch('eq', settings);
    const previous = compareSlots;
    const slot = previous?.slot === 'B' ? 'A' : 'B';
    const next = { A: current, B: current, ...previous, sourceId: snapshot.sourceId, slot } satisfies NonNullable<typeof eqComparison>;
    next[previous?.slot ?? 'A'] = current;
    eqComparison = next;
    setMode(null);
    onedit(next[slot], undefined, current);
    if (snapshot.original) onaction('treated');
  }

  function endEqComparison() {
    eqComparison = undefined;
    onedit({}, undefined, null);
  }

  async function toggleHelp() {
    help = !help;
    await tick();
    document.getElementById(id + (help ? '-close-help' : '-tools-menu'))?.focus({ preventScroll: true });
  }

  function windowKeys(event: KeyboardEvent) {
    if (event.defaultPrevented || event.isComposing || event.altKey) return;
    const target = event.target;
    if (target instanceof Element && target.closest('input, textarea, select, [contenteditable=true], [role=menu], [role=listbox]')) return;
    const key = event.key.toLowerCase();
    if (key === 'escape') {
      if (event.repeat) return;
      // Local graph handlers get the first chance to restore or clear selection.
      queueMicrotask(() => {
        if (event.defaultPrevented) return;
        if (help) void toggleHelp();
        else if (kind === 'eq' && snapshot?.reference) onaction('reference-cancel');
        else if (kind === 'eq' && mode) setMode(null);
        else if (snapshot) onaction('hide');
      });
      return;
    }
    if (key === '?' && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      if (!event.repeat) void toggleHelp();
      return;
    }
    if (!snapshot) return;
    if (event.ctrlKey || event.metaKey) {
      if (key !== 'z' && key !== 'y') return;
      event.preventDefault();
      onaction(key === 'y' || event.shiftKey ? 'redo' : 'undo');
    } else if (key === 'b' || key === 'l') {
      event.preventDefault();
      if (!event.repeat) onaction(key === 'l' ? 'loop' : snapshot.original ? 'treated' : 'original');
    } else if (key === ' ' && !event.repeat && !(target instanceof Element
      && target.closest('button:not(.plot-hit):not(.marker):not(.repair-hit):not([data-transport-space]), [role=option], [role=menuitem]'))) {
      event.preventDefault();
      onaction('play');
    }
  }

  function toggleDc() {
    if (settings.dcHz) restoreDc = settings.dcHz;
    onedit({ dcHz: settings.dcHz ? 0 : restoreDc || treatmentNumbers.dcHz.initial });
  }
</script>

{#snippet choice(label: string, value: string, options: readonly Option[], controller: ReturnType<typeof createSelectController<string>>, change: (value: string) => void)}
  <Select {controller} {value} disabled={!snapshot} onValueChange={value => { if (value !== null) change(value); }}>
    <SelectTrigger class="tool-select" aria-label={label}>{options.find(option => option.value === value)?.label ?? label}<ChevronDown size={13} aria-hidden="true"/></SelectTrigger>
    <SelectContent class="app-select-menu audio-tool-popup" align="start" sideOffset={4}>
      {#each options as option (option.value)}<SelectItem value={option.value} label={option.label} data-slot="select-item">{option.label}</SelectItem>{/each}
    </SelectContent>
  </Select>
{/snippet}

{#snippet number(key: TreatmentNumber, label: string)}
  {@const spec = treatmentNumbers[key]}
  {@const value = settings[key] || spec.initial}
  <label class="parameter">
    <span>{label}</span>
    <NumberInput class="tool-number" {value} min={spec.min} max={spec.max} step={spec.step ?? 1} disabled={!snapshot}
      onValueCommit={next => { if (next !== null) onedit({ [key]: next }); }}>
      <NumberInputControl class="parameter-input" aria-label={spec.unit ? label + ' in ' + spec.unit : label}/>
      {#if spec.unit}<span class="unit" aria-hidden="true">{spec.unit}</span>{/if}
    </NumberInput>
  </label>
{/snippet}

<svelte:window onkeydown={windowKeys}/>
{#snippet modeRail()}
  {#if kind !== 'eq'}
    <AudioSplitToolModes {kind} {settings} selected={mode} enabled={!!snapshot} stereo={!!snapshot?.stereo} onchoose={setMode}/>
  {/if}
{/snippet}

<section class="tool-window" class:eq={kind === 'eq'} class:repair={kind === 'repair'} aria-label={title[kind]}>
  <header class="tool-header">
    <div class="transport" role="group" aria-label="Preview controls">
      <Button variant="ghost" size="icon-sm" disabled={!snapshot} aria-label={snapshot?.playing ? 'Pause' : 'Play'} aria-keyshortcuts="Space"
        title={snapshot?.playing ? 'Pause (Space)' : 'Play (Space)'} data-transport-space onclick={() => onaction('play')}>
        {#if snapshot?.playing}<Pause size={15} aria-hidden="true"/>{:else}<Play size={15} aria-hidden="true"/>{/if}
      </Button>
      <Button variant="ghost" size="icon-sm" class="tool-power" disabled={!snapshot} aria-label="Loop preview" title="Loop preview (L)"
        aria-keyshortcuts="L" aria-pressed={snapshot?.loop ?? false} onclick={() => onaction('loop')}><Repeat2 size={16} aria-hidden="true"/></Button>
      <div class="comparison" role="group" aria-label="Preview sound">
        <Button variant="ghost" size="sm" disabled={!snapshot} title="Compare Original and Treated (B)" aria-pressed={snapshot?.original ?? false} onclick={() => onaction('original')}>Original</Button>
        <Button variant="ghost" size="sm" disabled={!snapshot} title="Compare Original and Treated (B)" aria-pressed={!!snapshot && !snapshot.original} onclick={() => onaction('treated')}>Treated</Button>
      </div>
      <Button variant="ghost" size="icon-sm" class="tool-power" disabled={!snapshot} aria-label="Match preview levels" title="Match Original and Treated volume"
        aria-pressed={snapshot?.levelMatch ?? false} onclick={() => onaction('level-match')}><Scale size={15} aria-hidden="true"/></Button>
      {#if kind === 'eq'}
        <Button variant="ghost" size="sm" class="ab-toggle" disabled={!snapshot?.eqLevelComparison || !!snapshot.reference?.preview}
          aria-label={compareSlots ? 'Switch to EQ ' + (compareSlots.slot === 'A' ? 'B' : 'A') : 'Start EQ A/B comparison'}
          title={snapshot?.eqLevelComparison ? 'Compare two EQ settings at the same listening level' : 'Reload the app to enable EQ A/B'}
          onclick={switchEqComparison}>
          {#if compareSlots}<span class:current={compareSlots.slot === 'A'}>A</span><span aria-hidden="true">/</span><span class:current={compareSlots.slot === 'B'}>B</span>
          {:else}A/B{/if}
        </Button>
      {/if}
    </div>
    <div class="transport">
      <Button variant="ghost" size="icon-sm" class="tool-power" disabled={!snapshot} aria-label="Keep tool on top" title="Keep tool on top"
        aria-pressed={snapshot?.pinned ?? false} onclick={() => onaction('pin')}><Pin size={15} aria-hidden="true"/></Button>
      {#if kind === 'eq'}
        <Button variant="ghost" size="icon-sm" disabled={!snapshot} aria-label="Previous EQ preset" title="Previous EQ preset"
          onclick={() => stepEqPreset(-1)}><ChevronLeft size={15} aria-hidden="true"/></Button>
      {/if}
      <Menu controller={presetMenu}>
        <MenuTrigger id={id + '-tools-menu'} class={kind === 'eq' ? 'tool-menu compact-presets' : 'tool-menu'}
          aria-label={kind === 'eq' ? 'EQ preset, ' + eqPresetName : 'Tool presets'} title="Presets and shortcuts">
          {#if kind === 'eq'}<span>{eqPresetName}</span>{:else}<Ellipsis size={17} aria-hidden="true"/>{/if}
        </MenuTrigger>
        <MenuContent class="app-select-menu audio-tool-popup" align="end" sideOffset={4} collisionPadding={8}>
          {#each presets as preset (preset.label)}
            <MenuItem data-slot="select-item" label={preset.label} disabled={!snapshot}
              onSelect={() => applyPreset(preset)}>{preset.label}
              {#if kind === 'eq' && eqPresetName === preset.label}<Check size={14} aria-hidden="true"/>{/if}
            </MenuItem>
          {/each}
          <MenuSeparator/>
          {#if kind === 'eq'}
            <MenuRadioGroup value={selectedShape} onValueChange={chooseEqShape}>
              <MenuGroupLabel>Filter shape</MenuGroupLabel>
              {#each Object.entries(legacyEqBands) as [shape, spec] (shape)}
                <MenuRadioItem data-slot="select-item" value={shape} disabled={!snapshot || (shape === 'high' && sampleRate * .45 < treatmentNumbers.highCutHz.min)} closeOnSelect>
                  {spec.name}<MenuItemIndicator aria-hidden="true"><Check size={14}/></MenuItemIndicator>
                </MenuRadioItem>
              {/each}
            </MenuRadioGroup>
            <MenuSeparator/>
          {/if}
          <MenuItem disabled={!snapshot?.undoLabel} onSelect={() => onaction('undo')}>{snapshot?.undoLabel ?? 'Undo'}</MenuItem>
          <MenuItem disabled={!snapshot?.redoLabel} onSelect={() => onaction('redo')}>{snapshot?.redoLabel ?? 'Redo'}</MenuItem>
          <MenuItem disabled={!snapshot} onSelect={() => { restoreDc = 0; onedit(noTreatment); }}>Reset this tool</MenuItem>
          {#if kind === 'eq' && compareSlots}
            <MenuItem onSelect={endEqComparison}>End A/B comparison</MenuItem>
          {/if}
          <MenuSeparator/>
          <MenuItem onSelect={toggleHelp}>Keyboard shortcuts <kbd>?</kbd></MenuItem>
        </MenuContent>
      </Menu>
      {#if kind === 'eq'}
        <Button variant="ghost" size="icon-sm" disabled={!snapshot} aria-label="Next EQ preset" title="Next EQ preset"
          onclick={() => stepEqPreset(1)}><ChevronRight size={15} aria-hidden="true"/></Button>
      {/if}
    </div>
  </header>

  {#if kind === 'eq'}
    <AudioSplitEqPanel {snapshot} {probe} {mode} onmode={setMode} {onedit} {onaction} {onanalyzer}/>
  {:else if kind === 'dynamics'}
    <AudioSplitDynamicsPanel {snapshot} {probe} mode={mode ?? 'compressor'} {onedit} {onaction}/>
  {:else}
    <AudioSplitRepairPlot {snapshot} {probe} {onviewport} {onedit} {onaction} {modeRail}/>
    {#if mode !== 'selection'}
    <div class="tool-controls">
      {#if mode === 'hum'}
        {@render choice('Mains frequency', settings.hum, humOptions, humControl, value => onedit({ hum: value as AudioTreatment['hum'] }))}
        {@render number('humHarmonics', 'Harmonics')}
      {:else if mode === 'dc'}
        <Button variant="ghost" size="icon-sm" class="tool-power" aria-pressed={!!settings.dcHz} aria-label="Remove DC offset" title="Remove DC offset" onclick={toggleDc}><Power size={15}/></Button>
        {@render number('dcHz', 'Cutoff')}
      {:else if snapshot?.stereo}
        {@render choice('Mono conversion', settings.mono, monoOptions, monoControl, value => onedit({ mono: value as AudioTreatment['mono'] }))}
        <Button variant="ghost" size="sm" class="tool-power" aria-pressed={settings.swapChannels} onclick={() => onedit({ swapChannels: !settings.swapChannels })}>Swap L/R</Button>
        {@render choice('Channel polarity', settings.polarity, polarityOptions, polarityControl, value => onedit({ polarity: value as AudioTreatment['polarity'] }))}
        {@render number('balance', 'Balance')}
        {@render number('stereoWidth', 'Width change')}
      {/if}
    </div>
    {/if}
  {/if}

  {#if kind === 'dynamics'}<div class="tool-side">{@render modeRail()}</div>{/if}

  {#if help}
    <section class="shortcut-help" aria-labelledby={id + '-shortcuts'}>
      <div><h2 id={id + '-shortcuts'}>Keyboard shortcuts</h2><Button id={id + '-close-help'} variant="ghost" size="icon-sm" aria-label="Close shortcuts" title="Close shortcuts" onclick={toggleHelp}><X size={15}/></Button></div>
      <dl>
        {#each [...commonKeys, ...details[kind]] as [key, description] (key)}
          <dt><kbd>{key}</kbd></dt><dd>{description}</dd>
        {/each}
      </dl>
    </section>
  {/if}
</section>

<style>
  .tool-window {
    position:relative; min-height:max(100dvh,342px); display:grid;
    grid-template-columns:minmax(0,1fr) auto; grid-template-rows:auto 1fr auto;
    gap:var(--sp-2,8px); padding:var(--sp-3,12px); background:var(--panel); color:var(--text); font-family:var(--font-ui,system-ui);
  }
  .tool-window.eq::after {
    content:''; position:absolute; inset:1px; border:1px solid var(--border);
    border-radius:var(--r-dialog); pointer-events:none;
  }
  .tool-window:is(.eq,.repair) { grid-template-columns:minmax(0,1fr); }
  .tool-header,.transport,.tool-controls,.comparison { display:flex; align-items:center; gap:var(--sp-2,8px); }
  .tool-header { grid-column:1 / -1; justify-content:space-between; min-width:0; }
  .tool-side { grid-column:2; grid-row:2 / 4; align-self:start; }
  .transport { gap:var(--sp-1,4px); }
  .comparison { gap:2px; }
  .comparison :global(button) { block-size:24px; border:0; border-radius:var(--r-compact); background:transparent; }
  .comparison :global([aria-pressed=true]) { background:var(--sel-tint); color:var(--accent-ink,var(--text)); }
  .eq :global(.ab-toggle) {
    gap:2px; padding-inline:6px; block-size:24px; min-inline-size:36px; border:0; background:transparent; color:var(--muted);
  }
  .eq :global(.ab-toggle .current) { color:var(--accent-ink,var(--text)); }
  .tool-window :global([data-size="icon-sm"]:not([data-tool-mode])) { border:0; background:transparent; }
  .tool-window :global([data-size="icon-sm"]:not([data-tool-mode]):hover:not(:disabled)) { color:var(--text); }
  .tool-window :global([data-size="icon-sm"][aria-pressed=true]:not([data-tool-mode])),
  .tool-window :global([data-size="icon-sm"][aria-expanded=true]), .tool-controls :global(.tool-power[aria-pressed=true]) { color:var(--accent-ink,var(--text)); }
  :global(.tool-window .tool-menu) { display:grid; place-items:center; inline-size:28px; block-size:28px; border:0; border-radius:var(--r-compact,8px); background:transparent; color:var(--muted); }
  :global(.tool-window .compact-presets) {
    display:grid; inline-size:104px; padding-inline:4px; font-size:var(--text-xs); color:var(--text);
  }
  :global(.tool-window .compact-presets) span { max-inline-size:100%; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .eq .tool-header { flex-wrap:wrap; }
  :global(.tool-window .tool-menu:hover) { color:var(--text); }
  :global(.tool-window .tool-menu:focus-visible) { outline:2px solid var(--accent); }
  .tool-controls { grid-column:1; min-height:38px; padding-inline:var(--sp-1,4px); flex-wrap:wrap; }
  .parameter { display:grid; gap:2px; color:var(--muted); font-size:.6875rem; font-weight:500; }
  .parameter :global(.tool-number),:global(.tool-window .tool-select) { display:inline-flex; align-items:center; min-height:28px; border-radius:var(--input-radius,10px); border:0; background:transparent; color:var(--text); font-size:.75rem; }
  .parameter :global(.tool-number:hover), :global(.tool-window .tool-select:hover:not(:disabled)) { background:var(--row-hover); }
  .parameter :global(.tool-number) { padding-inline:8px; }
  .parameter :global(.parameter-input) { width:6ch; min-width:0; padding:0; border:0; background:transparent; color:inherit; outline:none; text-align:right; font-variant-numeric:tabular-nums; }
  .parameter :global(.tool-number:focus-within),:global(.tool-window .tool-select:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
  :global(.tool-window .tool-select) { gap:7px; justify-content:space-between; padding-inline:9px; }
  .unit { color:var(--muted); margin-inline-start:4px; }
  .shortcut-help { position:absolute; z-index:10; inset-block-start:49px; inset-inline-end:var(--sp-3,12px); width:min(310px,calc(100% - 24px)); background:var(--panel); padding:var(--sp-3,12px); border-radius:var(--r-inset,14px); box-shadow:0 8px 24px rgb(0 0 0 / 24%); }
  .shortcut-help div { display:flex; align-items:center; justify-content:space-between; gap:8px; }
  .shortcut-help h2 { margin:0; font-size:.875rem; font-weight:600; }
  .shortcut-help dl { display:grid; grid-template-columns:max-content 1fr; gap:6px 12px; margin:8px 0 0; font-size:.75rem; }
  .shortcut-help dd { margin:0; color:var(--muted); }
  :global(.app-select-menu.audio-tool-popup) { background:var(--panel); }
</style>
