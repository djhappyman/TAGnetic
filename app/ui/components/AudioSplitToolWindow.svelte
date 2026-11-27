<script lang="ts">
  import { tick, untrack, type Snippet } from 'svelte';
  import { AudioLines, Check, ChevronDown, ChevronLeft, ChevronRight, Ellipsis, Equal, FastForward, Headphones, Pause, Pin, Play, Power, Repeat2, Rewind, Scale, SkipBack, X, Zap } from '@lucide/svelte';
  import { createMenuController, Menu, MenuContent, MenuGroup, MenuGroupLabel, MenuItem, MenuItemIndicator, MenuRadioGroup, MenuRadioItem, MenuSeparator, MenuShortcut, MenuSub, MenuSubContent, MenuSubTrigger, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { createSelectController, Select, SelectContent, SelectItem, SelectTrigger } from '@ui-sdk/svelte/select';
  import { Input } from '@ui-sdk/svelte/input';
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import AudioSplitDynamicsPanel from './AudioSplitDynamicsPanel.svelte';
  import AudioSplitEqPanel from './AudioSplitEqPanel.svelte';
  import EqDial from './AudioSplitEqDial.svelte';
  import AudioSplitRepairListen from './AudioSplitRepairListen.svelte';
  import AudioSplitRepairPlot from './AudioSplitRepairPlot.svelte';
  import AudioSplitRepairMeters from './AudioSplitRepairMeters.svelte';
  import RepairPopup from './AudioSplitRepairPopup.svelte';
  import AudioSplitToolModes, { isToolMode } from './AudioSplitToolModes.svelte';
  import Button from './AudioSplitButton.svelte';
  import { findEqNode, legacyEqBands, visibleEqNodes } from '../js/audio-eq';
  import { dynamicsMode, dynamicsPresets } from '../js/audio-dynamics';
  import { frameAtTime, timeLabel, type FrameRange } from '../js/audio-split';
  import { noTreatment, readTreatment, sameTreatment, treatmentNumbers, type AudioTreatment, type SpectralRepair, type TreatmentNumber } from '../js/audio-treatment';
  import type { AnalyzerView, TreatmentPlotMode, TreatmentProbe } from '../js/audio-treatment-plot';
  import { readEqPresets, saveEqPresets, treatmentForRecording, type SavedEqPreset } from '../js/audio-treatment-recipes';
  import { toolPatch, type PreviewBand, type RepairListenCommand, type RepairMeterUpdate, type SpectrogramViewOptions, type ToolAction, type ToolSnapshot } from '../js/audio-tools-window';
  import { projectRepairFrame, repairClockHorizon } from '../js/audio-repair-clock';

  let { kind, snapshot, probe, notice, meter, onedit, onaction, onviewport, onanalyzer, onlearn, onlisten }: {
    kind: TreatmentPlotMode;
    snapshot: ToolSnapshot | null;
    probe: TreatmentProbe | null;
    notice?: string;
    meter?: RepairMeterUpdate;
    onanalyzer: (view: AnalyzerView) => void;
    onlearn: (index: number) => void;
    onlisten: (command: RepairListenCommand) => void;
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
  const signalMenu = createMenuController();
  const signalLabels = { original: 'Original', repaired: 'Repaired', removed: 'Removed' } as const;
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
  let savedPresets = $state.raw(untrack(() => kind === 'eq' ? readEqPresets() : []));
  let presetDialog = $state<'save' | 'manage' | null>(null);
  let renamePreset = $state<string>();
  let removePreset = $state<string>();
  let presetName = $state('');
  let presetError = $state('');
  let presetToSave: Partial<AudioTreatment> = {};
  let eqComparison = $state.raw<{ sourceId: string; slot: CompareSlot; A: Partial<AudioTreatment>; B: Partial<AudioTreatment> }>();
  let restoreDc = $state(0);
  const settings = $derived(snapshot?.settings ?? noTreatment);
  const sampleRate = $derived(snapshot?.sampleRate || 44100);
  const repairSignal = $derived(snapshot?.repairListen?.signal ?? (snapshot?.original ? 'original' : 'repaired'));
  const playTime = $derived(timeLabel(snapshot?.frame ?? 0, sampleRate, 3));
  const meterReading = $derived(meter?.sequence === snapshot?.transportSequence ? meter : undefined);
  let enteredTime = $state<string>();
  let timeInvalid = $state(false);
  const selectedShape = $derived(kind === 'eq' ? findEqNode(settings, mode)?.shape ?? '' : '');
  function chooseEqShape(value: string) {
    setMode(visibleEqNodes(settings, null).find(node => node.shape === value)?.key ?? value);
  }
  const presets: readonly Preset[] = $derived(
    kind === 'eq' ? [...eqPresets.filter(preset => (preset.patch.highCutHz ?? 0) <= Math.floor(sampleRate * .45)), ...savedPresets]
      : kind === 'dynamics' ? dynamicsPresets[dynamicsMode(mode ?? '')] : repairPresets,
  );
  const eqPresetIndex = $derived.by(() => {
    if (kind !== 'eq') return -1;
    const current = readTreatment(toolPatch('eq', settings));
    return presets.findIndex(preset => sameTreatment(current, readTreatment(eqPresetPatch(preset))));
  });
  const eqPresetName = $derived(presets[eqPresetIndex]?.label ?? 'Custom EQ');
  const compareSlots = $derived(eqComparison?.sourceId === snapshot?.sourceId ? eqComparison : undefined);
  const modKey = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl';
  const commonKeys = [['Space', 'Play or pause'], ['L', 'Toggle loop'], ['B', 'Compare the preview with Original'],
    [modKey + '+Z', 'Undo'], [modKey + '+Shift+Z', 'Redo'], ['?', 'Show keyboard shortcuts']];
  const details = {
    eq: [['Enter', 'Add a band or edit its exact values'], ['Arrows', 'Move the focused band'], ['Shift+arrows', 'Move in finer steps'], [modKey === '⌘' ? 'Option+drag' : 'Alt+drag', 'Lock the drag to one axis'], ['Delete', 'Remove the focused band']],
    dynamics: [['Arrows', 'Adjust the focused handle'], ['Shift+arrows', 'Adjust in finer steps']],
    repair: [['1–5', 'Rectangle, Time, Frequency, Brush, Lasso'], ['Enter', 'Audition the selection'],
      ['Shift+Enter', 'Audition the surrounding mix'], [modKey + '+Enter', 'Apply the repair'], ['Delete', 'Remove a selected repair'],
      ['F', 'Fit the selection'], ['0', 'Fit the recording'], ['Shift+scroll', 'Zoom frequencies']],
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

  function eqPresetPatch(preset: Preset) {
    return toolPatch('eq', treatmentForRecording(readTreatment(preset.patch), noTreatment, sampleRate, snapshot?.stereo ?? false));
  }

  function applyEqCurve(patch: Partial<AudioTreatment>, alternate?: Partial<AudioTreatment>) {
    const next = readTreatment({ ...settings, ...patch });
    if (mode && !findEqNode(next, mode)) setMode(null);
    onedit(patch, undefined, alternate);
  }

  function applyPreset(preset: Preset) {
    if (kind !== 'eq') onedit(preset.patch);
    else if (compareSlots) auditionPreset(preset);
    else applyEqCurve(eqPresetPatch(preset));
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
    applyEqCurve(next[slot], current);
    if (snapshot.original) onaction('treated');
  }

  function auditionPreset(preset: Preset) {
    if (!snapshot?.eqLevelComparison || snapshot.reference?.preview) return;
    const A = compareSlots?.slot === 'B' ? compareSlots.A : toolPatch('eq', settings);
    const B = eqPresetPatch(preset);
    eqComparison = { sourceId: snapshot.sourceId, slot: 'B', A, B };
    applyEqCurve(B, A);
    if (snapshot.original) onaction('treated');
  }

  function copyAtoB() {
    if (!compareSlots || snapshot?.reference?.preview) return;
    const A = compareSlots.slot === 'A' ? toolPatch('eq', settings) : compareSlots.A;
    eqComparison = { ...compareSlots, A, B: A };
    applyEqCurve(A, A);
  }

  function openPresetDialog(dialog: 'save' | 'manage') {
    presetDialog = dialog;
    presetName = '';
    presetError = '';
    renamePreset = undefined;
    removePreset = undefined;
    if (dialog === 'save') presetToSave = toolPatch('eq', settings);
  }

  function storePresets(next: SavedEqPreset[]) {
    try {
      saveEqPresets(next);
      savedPresets = next;
      presetError = '';
      return true;
    } catch {
      presetError = 'Could not save your preset changes.';
      return false;
    }
  }

  function saveNamedPreset(event: SubmitEvent) {
    event.preventDefault();
    const label = presetName.trim();
    if (!label || label.length > 64) return;
    if ([...eqPresets, ...savedPresets.filter(preset => preset.id !== renamePreset)]
      .some(preset => preset.label.toLowerCase() === label.toLowerCase())) {
      presetError = 'A preset already uses that name.';
      return;
    }
    if (presetDialog === 'save') {
      if (storePresets([...savedPresets, { id: crypto.getRandomValues(new Uint32Array(4)).join('-'), label, patch: presetToSave }])) presetDialog = null;
    } else if (storePresets(savedPresets.map(preset => preset.id === renamePreset ? { ...preset, label } : preset))) {
      renamePreset = undefined;
    }
  }

  async function startRename(preset: SavedEqPreset) {
    renamePreset = preset.id;
    removePreset = undefined;
    presetError = '';
    presetName = preset.label;
    await tick();
    document.getElementById(id + '-preset-name')?.focus();
  }

  function deleteNamedPreset() {
    if (storePresets(savedPresets.filter(preset => preset.id !== removePreset))) removePreset = undefined;
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

  let pointerButton: Element | null = null;

  function rememberPointerButton(event: PointerEvent) {
    if (kind === 'repair') pointerButton = event.target instanceof Element ? event.target.closest('button') : null;
  }

  function windowKeys(event: KeyboardEvent) {
    if (event.key === 'Tab') pointerButton = null;
    if (event.defaultPrevented || event.isComposing || event.altKey || presetDialog) return;
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
    if (kind === 'repair' && key === 'l' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      if (!event.repeat) onaction('loop');
      return;
    }
    if (event.ctrlKey || event.metaKey) {
      if (key !== 'z' && key !== 'y') return;
      event.preventDefault();
      onaction(key === 'y' || event.shiftKey ? 'redo' : 'undo');
    } else if (key === 'b' || key === 'l') {
      event.preventDefault();
      const original = kind === 'repair' ? repairSignal === 'original' : snapshot.original;
      if (!event.repeat) onaction(key === 'l' ? 'loop' : original ? 'treated' : 'original');
    } else if (key === ' ' || (kind === 'repair' && event.code === 'Space')) {
      const control = target instanceof Element && target.closest(
        'button:not(.plot-hit):not(.marker):not(.repair-hit):not(.overview-hit):not([data-transport-space]), summary, [role=option], [role=menuitem]');
      if (control && (control !== pointerButton || control.matches('[role], [aria-haspopup], [aria-expanded]')
        || control.closest('[role=dialog]'))) return;
      event.preventDefault();
      if (!event.repeat) onaction('play');
    }
  }

  function toggleDc() {
    if (settings.dcHz) restoreDc = settings.dcHz;
    onedit({ dcHz: settings.dcHz ? 0 : restoreDc || treatmentNumbers.dcHz.initial });
  }

  function seek(frame: number) {
    if (!snapshot?.frames) return;
    const start = Math.max(0, Math.min(frame, snapshot.frames - 1));
    if (start !== snapshot.frame) onaction('seek', { start, end: start + 1 });
  }

  function commitPlayTime() {
    if (enteredTime === undefined) return;
    const frame = frameAtTime(enteredTime, sampleRate);
    if (frame === null || frame < 0 || frame > (snapshot?.frames ?? 0)) {
      timeInvalid = true;
      return;
    }
    if (enteredTime !== playTime) seek(frame);
    enteredTime = undefined;
    timeInvalid = false;
  }

  function playbackPosition(input: HTMLInputElement) {
    if (!snapshot || !(snapshot.clockRunning ?? snapshot.playing) || enteredTime !== undefined) return;
    const loop = snapshot.loop ? snapshot.loopRange : undefined;
    const observed = performance.now();
    const clock = { frame: snapshot.frame, observed, running: true, rate: sampleRate, frames: snapshot.frames, loop,
      endFrame: !loop && snapshot.selectionPlaying ? snapshot.loopRange?.end : undefined };
    let animation = 0;
    const paint = (now: number) => {
      if (document.activeElement !== input) input.value = timeLabel(projectRepairFrame(clock, now), clock.rate, 3);
      if (now - observed < repairClockHorizon) animation = requestAnimationFrame(paint);
    };
    animation = requestAnimationFrame(paint);
    return () => cancelAnimationFrame(animation);
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

{#snippet dial(key: TreatmentNumber, label: string)}
  {@const spec = treatmentNumbers[key]}
  <EqDial {label} value={settings[key] || (key === 'dcHz' ? restoreDc || spec.initial : 0)}
    min={spec.min} max={spec.max} step={spec.step ?? 1} unit={spec.unit} color="var(--accent-ink)"
    resetValue={spec.initial} onvalue={(value, gesture) => onedit({ [key]: value }, gesture)}/>
{/snippet}

<svelte:window onkeydown={windowKeys} onpointerdowncapture={rememberPointerButton}/>
{#snippet modeRail()}
  <RepairPopup label="Mains hum" disabled={!snapshot} active={settings.hum !== 'off'}>
    {#snippet icon()}<Zap size={16} aria-hidden="true"/>{/snippet}
    <div class="repair-options">
      {@render choice('Mains frequency', settings.hum, humOptions, humControl, value => onedit({ hum: value as AudioTreatment['hum'] }))}
      {@render dial('humHarmonics', 'Harmonics')}
    </div>
  </RepairPopup>
  <RepairPopup label="DC offset" disabled={!snapshot} active={!!settings.dcHz}>
    {#snippet icon()}<Equal size={16} aria-hidden="true"/>{/snippet}
    <div class="repair-options">
      <Button role="switch" size="sm" variant="ghost" class="repair-switch" aria-checked={!!settings.dcHz}
        onclick={toggleDc}>Remove DC offset</Button>
      {@render dial('dcHz', 'Cutoff')}
    </div>
  </RepairPopup>
  <RepairPopup label="Stereo" disabled={!snapshot?.stereo}
    active={!!(settings.balance || settings.stereoWidth || settings.swapChannels || settings.mono !== 'off' || settings.polarity !== 'off')}>
    {#snippet icon()}<Headphones size={16} aria-hidden="true"/>{/snippet}
    <div class="repair-options stereo-options">
      {@render choice('Mono conversion', settings.mono, monoOptions, monoControl, value => onedit({ mono: value as AudioTreatment['mono'] }))}
      {@render choice('Channel polarity', settings.polarity, polarityOptions, polarityControl, value => onedit({ polarity: value as AudioTreatment['polarity'] }))}
      <Button role="switch" size="sm" variant="ghost" class="repair-switch" aria-checked={settings.swapChannels}
        onclick={() => onedit({ swapChannels: !settings.swapChannels })}>Swap L/R</Button>
    </div>
    <div class="repair-dials">
      {@render dial('balance', 'Balance')}
      {@render dial('stereoWidth', 'Width change')}
    </div>
  </RepairPopup>
{/snippet}

{#snippet transportControls()}
    <div class="transport" role="group" aria-label="Preview controls">
      <Button variant="ghost" size="icon-sm" disabled={!snapshot} aria-label={snapshot?.playing ? 'Pause' : 'Play'} aria-keyshortcuts="Space"
        title={snapshot?.playing ? 'Pause (Space)' : 'Play (Space)'} data-transport-space onclick={() => onaction('play')}>
        {#if snapshot?.playing}<Pause size={15} aria-hidden="true"/>{:else}<Play size={15} aria-hidden="true"/>{/if}
      </Button>
      {#if kind === 'repair'}
        <div class="repair-seek" role="group" aria-label="Seek">
          <Button variant="ghost" size="icon-sm" disabled={!snapshot} aria-label="Go to recording start" title="Go to recording start"
            onclick={() => seek(0)}><SkipBack size={15} aria-hidden="true"/></Button>
          <Button variant="ghost" size="icon-sm" disabled={!snapshot} aria-label="Back 5 seconds" title="Back 5 seconds"
            onclick={() => seek((snapshot?.frame ?? 0) - 5 * sampleRate)}><Rewind size={15} aria-hidden="true"/></Button>
          <Input class="repair-position" type="text" inputmode="decimal" aria-label="Playback position"
            {@attach playbackPosition}
            title="Playback position · enter a time to seek" value={enteredTime ?? playTime} disabled={!snapshot} aria-invalid={timeInvalid}
            onfocus={event => event.currentTarget.select()}
            oninput={event => { enteredTime = event.currentTarget.value; timeInvalid = false; }}
            onblur={() => { commitPlayTime(); enteredTime = undefined; timeInvalid = false; }}
            onkeydown={event => {
              if (event.key === 'Enter') { event.preventDefault(); commitPlayTime(); }
              if (event.key === 'Escape') { event.preventDefault(); enteredTime = undefined; timeInvalid = false; }
            }}/>
          <Button variant="ghost" size="icon-sm" disabled={!snapshot} aria-label="Forward 5 seconds" title="Forward 5 seconds"
            onclick={() => seek((snapshot?.frame ?? 0) + 5 * sampleRate)}><FastForward size={15} aria-hidden="true"/></Button>
        </div>
      {/if}
      <Button variant="ghost" size="icon-sm" class="tool-power" disabled={!snapshot}
        aria-label={kind === 'repair' ? 'Loop selection' : 'Loop preview'} title={kind === 'repair' ? 'Repeat the selection or add a playback loop (Ctrl/⌘+L)' : 'Loop preview (L)'}
        aria-keyshortcuts={kind === 'repair' ? 'L Control+L Meta+L' : 'L'}
        aria-pressed={!!(snapshot?.loop || (kind === 'repair' && snapshot?.repeatSelection))} onclick={() => onaction('loop')}><Repeat2 size={16} aria-hidden="true"/></Button>
      {#if kind === 'eq'}
        <Button variant="ghost" size="icon-sm" class="treatment-bypass" disabled={!snapshot}
          aria-label="Bypass treatment" aria-pressed={snapshot?.original ?? false} aria-keyshortcuts="B"
          title={snapshot?.original ? 'Resume treated playback (B)' : 'Bypass treatment (B)'}
          onclick={() => onaction(snapshot?.original ? 'treated' : 'original')}><Power size={15} aria-hidden="true"/></Button>
      {:else if kind === 'repair'}
        <Menu controller={signalMenu}>
          <MenuTrigger class="repair-signal" aria-label={'Playback signal, ' + signalLabels[repairSignal]} title="Change playback and spectrogram signal (B)"
            disabled={!snapshot}><AudioLines size={15} aria-hidden="true"/>{signalLabels[repairSignal]}<ChevronDown size={11} aria-hidden="true"/></MenuTrigger>
          <MenuContent class="audio-action-menu" align="start" sideOffset={5} collisionPadding={8}>
            <MenuRadioGroup value={repairSignal} onValueChange={value => onlisten({ type: 'signal', signal: value as typeof repairSignal })}>
              {#each Object.entries(signalLabels) as [key, label] (key)}
                <MenuRadioItem value={key} closeOnSelect>{label}<MenuItemIndicator class="menu-trailing" aria-hidden="true"><Check size={14}/></MenuItemIndicator></MenuRadioItem>
              {/each}
            </MenuRadioGroup>
          </MenuContent>
        </Menu>
      {:else}
        <div class="comparison" role="group" aria-label="Preview sound">
          <Button variant="ghost" size="sm" disabled={!snapshot} title="Compare Original and Treated (B)" aria-pressed={snapshot?.original ?? false} onclick={() => onaction('original')}>Original</Button>
          <Button variant="ghost" size="sm" disabled={!snapshot} title="Compare Original and Treated (B)" aria-pressed={!!snapshot && !snapshot.original} onclick={() => onaction('treated')}>Treated</Button>
        </div>
      {/if}
      {#if kind !== 'repair'}
        <Button variant="ghost" size="icon-sm" class="tool-power" disabled={!snapshot} aria-label="Match preview levels" title="Match Original and Treated volume"
          aria-pressed={snapshot?.levelMatch ?? false} onclick={() => onaction('level-match')}><Scale size={15} aria-hidden="true"/></Button>
      {/if}
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
{/snippet}

{#snippet windowControls()}
    <div class="transport preset-controls">
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
        <MenuContent class="audio-action-menu" align="end" sideOffset={4} collisionPadding={8}>
          <MenuGroup>
            {#if kind === 'eq'}<MenuGroupLabel class="menu-heading">{compareSlots ? 'Audition in B' : 'Presets'}</MenuGroupLabel>{/if}
            {#each presets as preset (preset.label)}
              {@const current = kind === 'eq' && eqPresetName === preset.label}
              <MenuItem aria-current={current ? 'true' : undefined} label={preset.label} disabled={!snapshot}
                onSelect={() => applyPreset(preset)}>{preset.label}
                {#if current}<Check class="menu-trailing" size={14} aria-hidden="true"/>{/if}
              </MenuItem>
            {/each}
          </MenuGroup>
          <MenuSeparator/>
          {#if kind === 'eq'}
            {#if snapshot?.eqLevelComparison && !snapshot.reference?.preview}
              {#if compareSlots}
                <MenuItem onSelect={copyAtoB}>Copy A to B</MenuItem>
              {:else}
                <MenuSub>
                  <MenuSubTrigger>Audition preset in B <ChevronRight class="menu-trailing" size={14} aria-hidden="true"/></MenuSubTrigger>
                  <MenuSubContent class="audio-action-menu" sideOffset={4} collisionPadding={8}>
                    {#each presets as preset (preset.label)}
                      <MenuItem onSelect={() => auditionPreset(preset)}>{preset.label}</MenuItem>
                    {/each}
                  </MenuSubContent>
                </MenuSub>
              {/if}
            {/if}
            <MenuItem disabled={!snapshot || snapshot.reference?.status === 'choosing' || snapshot.reference?.status === 'analysing'}
              onSelect={() => onaction('reference-start')}>Match to reference…</MenuItem>
            <MenuItem disabled={!snapshot} onSelect={() => openPresetDialog('save')}>Save as preset…</MenuItem>
            <MenuItem disabled={!savedPresets.length} onSelect={() => openPresetDialog('manage')}>Manage saved presets…</MenuItem>
            <MenuSub>
              <MenuSubTrigger disabled={!snapshot}>Filter shape <ChevronRight class="menu-trailing" size={14} aria-hidden="true"/></MenuSubTrigger>
              <MenuSubContent class="audio-action-menu" sideOffset={4} collisionPadding={8}>
                <MenuRadioGroup value={selectedShape} onValueChange={chooseEqShape}>
                  {#each Object.entries(legacyEqBands) as [shape, spec] (shape)}
                    <MenuRadioItem value={shape} disabled={shape === 'high' && sampleRate * .45 < treatmentNumbers.highCutHz.min} closeOnSelect>
                      {spec.name}<MenuItemIndicator class="menu-trailing" aria-hidden="true"><Check size={14}/></MenuItemIndicator>
                    </MenuRadioItem>
                  {/each}
                </MenuRadioGroup>
              </MenuSubContent>
            </MenuSub>
            <MenuSeparator/>
          {/if}
          <MenuItem disabled={!snapshot?.undoLabel} onSelect={() => onaction('undo')}>{snapshot?.undoLabel ?? 'Undo'}</MenuItem>
          <MenuItem disabled={!snapshot?.redoLabel} onSelect={() => onaction('redo')}>{snapshot?.redoLabel ?? 'Redo'}</MenuItem>
          <MenuItem disabled={!snapshot} onSelect={() => { restoreDc = 0; onedit(noTreatment); }}>Reset this tool</MenuItem>
          {#if kind === 'eq' && compareSlots}
            <MenuItem onSelect={endEqComparison}>End A/B comparison</MenuItem>
          {/if}
          <MenuSeparator/>
          <MenuItem onSelect={toggleHelp}>Keyboard shortcuts <MenuShortcut class="menu-trailing">?</MenuShortcut></MenuItem>
        </MenuContent>
      </Menu>
      {#if kind === 'eq'}
        <Button variant="ghost" size="icon-sm" disabled={!snapshot} aria-label="Next EQ preset" title="Next EQ preset"
          onclick={() => stepEqPreset(1)}><ChevronRight size={15} aria-hidden="true"/></Button>
      {/if}
    </div>
{/snippet}

<section class={['tool-window', { eq: kind === 'eq', repair: kind === 'repair' }]} aria-label={title[kind]}>
  {#if kind !== 'repair'}
    <header class={['tool-header', { 'tool-chrome': kind === 'eq' }]}>
      {@render transportControls()}
      {@render windowControls()}
    </header>
  {/if}
  {#if kind === 'eq'}
    <AudioSplitEqPanel {snapshot} {probe} {mode} onmode={setMode} {onedit} {onaction} {onanalyzer} {onlearn}/>
  {:else if kind === 'dynamics'}
    <AudioSplitDynamicsPanel {snapshot} {probe} mode={mode ?? 'compressor'} {onedit} {onaction}/>
  {:else}
    {#snippet repairHeader(overview: Snippet)}
      <header class="tool-header tool-chrome overview-bar">
        {@render overview()}
        {@render windowControls()}
      </header>
    {/snippet}
    {#snippet repairFooter(controls: Snippet)}
      <footer class="repair-transport tool-chrome" data-tooltip-side="top">
        {@render transportControls()}
        <AudioSplitRepairMeters reading={meterReading} running={!!(snapshot?.clockRunning ?? snapshot?.playing)}
          stereo={!!(snapshot?.repairStereoOutput ?? snapshot?.stereo)} monitor={snapshot?.repairListen?.channel ?? 'stereo'}/>
        {@render controls()}
      </footer>
    {/snippet}
    {#snippet listen(selection: SpectralRepair | undefined, candidate: SpectralRepair | undefined, repairSlot: number | undefined)}
      <AudioSplitRepairListen {snapshot} {selection} {candidate} {repairSlot} {onlisten}/>
    {/snippet}
    {#key snapshot?.sourceId}
      <AudioSplitRepairPlot {snapshot} {probe} {notice} {onviewport} {onedit} {onaction} {onlisten} {modeRail} {listen} header={repairHeader} footer={repairFooter}
        signal={repairSignal} onsignal={signal => onlisten({ type: 'signal', signal })}/>
    {/key}
  {/if}

  {#if kind === 'dynamics'}
    <div class="tool-side">
      <AudioSplitToolModes {kind} {settings} selected={mode} enabled={!!snapshot} stereo={!!snapshot?.stereo} onchoose={setMode}/>
    </div>
  {/if}

  {#if kind === 'eq'}
    <Dialog.Root open={presetDialog !== null} onOpenChange={open => { if (!open) presetDialog = null; }}>
      <Dialog.Content class="eq-preset-dialog" overlayProps={{ class: 'controller-dialog-overlay eq-preset-overlay' }}
        onCloseAutoFocus={event => { event.preventDefault(); document.getElementById(id + '-tools-menu')?.focus(); }}>
        <Dialog.Title>{presetDialog === 'save' ? 'Save EQ preset' : 'Saved EQ presets'}</Dialog.Title>
        <Dialog.Description>{presetDialog === 'save' ? 'Reuse this EQ curve on another recording.' : 'Rename or remove your EQ presets.'}</Dialog.Description>
        {#if presetDialog === 'save' || renamePreset}
          <form class="preset-form" onsubmit={saveNamedPreset}>
            <Input id={id + '-preset-name'} aria-label="Preset name" placeholder="Preset name" maxlength={64} bind:value={presetName} oninput={() => presetError = ''}/>
            <Button size="sm" type="submit" disabled={!presetName.trim()}>{renamePreset ? 'Rename' : 'Save preset'}</Button>
            {#if renamePreset}
              <Button size="sm" variant="ghost" onclick={() => { renamePreset = undefined; presetError = ''; }}>Cancel</Button>
            {/if}
          </form>
        {/if}
        {#if presetDialog === 'manage'}
          <ul class="preset-list" aria-label="Saved presets">
            {#each savedPresets as preset (preset.id)}
              <li>
                <span>{preset.label}</span>
                <Button size="sm" variant="ghost" aria-label={'Rename ' + preset.label} onclick={() => startRename(preset)}>Rename</Button>
                <Button size="sm" variant="ghost" aria-label={'Delete ' + preset.label}
                  onclick={() => { removePreset = preset.id; renamePreset = undefined; presetError = ''; }}>Delete</Button>
              </li>
            {/each}
          </ul>
          {#if removePreset}
            <div class="preset-form">
              <span>Delete {savedPresets.find(preset => preset.id === removePreset)?.label}?</span>
              <Button size="sm" variant="destructive" onclick={deleteNamedPreset}>Delete preset</Button>
              <Button size="sm" variant="ghost" onclick={() => { removePreset = undefined; presetError = ''; }}>Cancel</Button>
            </div>
          {/if}
        {/if}
        {#if presetError}<p class="preset-error" role="alert">{presetError}</p>{/if}
      </Dialog.Content>
    </Dialog.Root>
  {/if}

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
  :global(.eq-preset-overlay[data-slot=dialog-overlay]) { z-index:144; }
  :global(.eq-preset-dialog[data-slot=dialog-content]) {
    z-index:145; width:min(420px, calc(100vw - 32px)); max-width:none; max-height:calc(100dvh - 32px); overflow:auto;
    padding:var(--sp-4); border:1px solid var(--ui-muted-border); border-radius:var(--r-dialog-compact);
    background:var(--panel); color:var(--text);
  }
  .preset-form { display:flex; align-items:center; flex-wrap:wrap; gap:var(--sp-2); }
  .preset-form :global(input) { flex:1; min-width:8rem; }
  .preset-list { display:grid; gap:var(--sp-1); padding:0; max-height:40vh; overflow:auto; }
  .preset-list li { display:grid; grid-template-columns:minmax(0,1fr) auto auto; align-items:center; gap:var(--sp-1); }
  .preset-list li > span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .preset-error { color:var(--del); }

  .tool-window {
    position:relative; min-height:max(100dvh,342px); display:grid;
    grid-template-columns:minmax(0,1fr) auto; grid-template-rows:auto 1fr auto;
    gap:var(--sp-2,8px); padding:var(--sp-3,12px); background:var(--panel); color:var(--text); font-family:var(--font-ui,system-ui);
  }
  .tool-window:is(.eq,.repair)::after {
    content:''; position:absolute; inset:1px; border:1px solid var(--border);
    border-radius:var(--r-dialog); pointer-events:none;
  }
  .tool-window:is(.eq,.repair) { grid-template-columns:minmax(0,1fr); }
  .tool-window.repair {
    --repair-well:color-mix(in srgb,var(--panel) 87%,var(--border));
    min-height:100dvh; grid-template-rows:minmax(0,1fr);
  }
  .tool-header,.transport,.comparison { display:flex; align-items:center; gap:var(--sp-2,8px); }
  .tool-header { grid-column:1 / -1; justify-content:space-between; min-width:0; }
  .tool-header.overview-bar { min-height:32px; }
  .overview-bar .preset-controls { flex:none; }
  .repair-transport {
    display:flex; align-items:center; flex-wrap:wrap; gap:var(--sp-1) var(--sp-2); min-width:0;
    min-height:36px; padding-inline:var(--sp-1);
  }
  .repair-transport .transport { flex-wrap:wrap; }
  .repair-transport :global(.repair-meters) { flex:1 1 210px; max-inline-size:360px; }
  .repair-transport :global(.repair-footer-tools) { margin-inline-start:auto; }
  .tool-side { grid-column:2; grid-row:2 / 4; align-self:start; }
  .transport { gap:var(--sp-1,4px); }
  .comparison { gap:2px; }
  .comparison :global(button) { block-size:24px; border:0; border-radius:var(--r-compact); background:transparent; }
  .comparison :global([aria-pressed=true]) { background:var(--sel-tint); color:var(--accent-ink,var(--text)); }
  .repair-seek { display:flex; align-items:center; gap:2px; }
  .repair :global(.repair-position) {
    width:11ch; min-width:0; padding:0 var(--sp-1); block-size:28px;
    border:0; border-radius:var(--r-compact); background:var(--repair-well); box-shadow:none;
    color:var(--text); font-family:var(--font-literal); font-variant-numeric:tabular-nums; font-size:var(--text-xs); text-align:center;
  }
  .repair :global(.repair-position:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
  .repair :global(.repair-position[aria-invalid=true]) { outline:1px solid var(--del); }
  .repair :global(.repair-signal) {
    display:inline-flex; align-items:center; gap:var(--sp-1); block-size:28px; padding-inline:var(--sp-2);
    border:0; border-radius:var(--r-compact); background:transparent; color:var(--muted); font-size:var(--text-xs);
  }
  .eq :global(.ab-toggle) {
    gap:2px; padding-inline:6px; block-size:24px; min-inline-size:36px; border:0; background:transparent; color:var(--muted);
  }
  .eq :global(.ab-toggle .current) { color:var(--accent-ink,var(--text)); }
  .tool-window :global([data-size="icon-sm"]:not([data-tool-mode])) { border:0; background:transparent; }
  .tool-window :global([data-size="icon-sm"]:not([data-tool-mode]):hover:not(:disabled)) { color:var(--text); }
  .tool-window :global([data-size="icon-sm"][aria-pressed=true]:not([data-tool-mode])),
  .tool-window :global([data-size="icon-sm"][aria-expanded=true]) { color:var(--accent-ink,var(--text)); }
  :global(.tool-window .tool-menu) { display:grid; place-items:center; inline-size:28px; block-size:28px; border:0; border-radius:var(--r-compact,8px); background:transparent; color:var(--muted); }
  :global(.tool-window .compact-presets) {
    display:grid; inline-size:104px; padding-inline:4px; font-size:var(--text-xs); color:var(--text);
  }
  :global(.tool-window .compact-presets) span { max-inline-size:100%; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .eq .tool-header { flex-wrap:wrap; gap:var(--sp-1) var(--sp-3); }
  .tool-chrome .transport :global(button) {
    border:0; box-shadow:none; background:transparent; color:var(--muted);
  }
  .tool-chrome .transport :global(button:hover:not(:disabled)) { color:var(--text); background:var(--row-hover); }
  .tool-chrome .transport :global(button:is([aria-pressed=true],[aria-expanded=true])) {
    color:var(--accent-ink); background:var(--sel-tint);
  }
  .tool-chrome .transport :global(button:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
  .eq .tool-header :global(button.treatment-bypass[aria-pressed=true]) {
    color:color-mix(in srgb,var(--del) 78%,var(--muted));
    background:color-mix(in srgb,var(--del) 14%,transparent);
  }
  .eq .preset-controls { margin-inline-start:auto; }
  .eq .preset-controls :global(.compact-presets) { color:var(--text); font-weight:var(--fw-medium,500); }
  :global(.tool-window .tool-menu:hover) { color:var(--text); }
  :global(.tool-window .tool-menu:focus-visible) { outline:2px solid var(--accent); }
  .repair-options,.repair-dials { display:flex; align-items:center; gap:var(--sp-3); }
  .stereo-options { flex-wrap:wrap; gap:var(--sp-1); }
  .repair-dials { justify-content:center; }
  .repair-options :global(.tool-select) {
    display:inline-flex; align-items:center; justify-content:space-between; gap:var(--sp-2); min-height:28px;
    padding-inline:var(--sp-2); border-radius:var(--input-radius); border:0;
    background:var(--row-hover); color:var(--text); font-size:var(--text-xs);
  }
  .repair-options :global(.tool-select:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
  .repair-options :global(.repair-switch[aria-checked=true]) { background:var(--sel-tint); color:var(--accent-ink); }
  .shortcut-help { position:absolute; z-index:10; inset-block-start:49px; inset-inline-end:var(--sp-3,12px); width:min(310px,calc(100% - 24px)); background:var(--panel); padding:var(--sp-3,12px); border-radius:var(--r-inset,14px); box-shadow:0 8px 24px rgb(0 0 0 / 24%); }
  .shortcut-help div { display:flex; align-items:center; justify-content:space-between; gap:8px; }
  .shortcut-help h2 { margin:0; font-size:.875rem; font-weight:600; }
  .shortcut-help dl { display:grid; grid-template-columns:max-content 1fr; gap:6px 12px; margin:8px 0 0; font-size:.75rem; }
  .shortcut-help dd { margin:0; color:var(--muted); }
  :global(.app-select-menu.audio-tool-popup) { background:var(--panel); }
  @media (max-width: 480px) {
    .repair .transport { gap: 2px; }
  }
  @media (max-height: 420px) {
    .tool-window.repair { height:100dvh; padding:var(--sp-2); }
    .tool-header.overview-bar { min-height:28px; }
  }
</style>
