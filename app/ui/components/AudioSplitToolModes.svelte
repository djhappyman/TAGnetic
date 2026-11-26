<script module lang="ts">
  import { Activity, ArrowDownToLine, ArrowUpToLine, AudioLines, ChartSpline, Crosshair, Equal, Headphones, ScanLine, TrendingDown, TrendingUp, Volume2, VolumeX, Zap } from '@lucide/svelte';
  import Button from './AudioSplitButton.svelte';
  import { eqBandBypassBits, type AudioTreatment } from '../js/audio-treatment';
  import type { TreatmentPlotMode } from '../js/audio-treatment-plot';

  const modes = {
    eq: [
      { key: 'low', name: 'Low cut', icon: ArrowUpToLine, effect: 'rumble' },
      { key: 'bass', name: 'Low shelf', icon: TrendingUp, effect: 'bassDb' },
      { key: 'bell', name: 'Bell', icon: ChartSpline, effect: 'presenceDb' },
      { key: 'notch', name: 'Notch', icon: ScanLine, effect: 'notchHz' },
      { key: 'treble', name: 'High shelf', icon: TrendingDown, effect: 'trebleDb' },
      { key: 'high', name: 'High cut', icon: ArrowDownToLine, effect: 'highCutHz' },
    ],
    dynamics: [
      { key: 'compressor', name: 'Compressor', icon: Activity, effect: 'compressorDb' },
      { key: 'gate', name: 'Quiet-gap reduction', icon: VolumeX, effect: 'gateDb' },
      { key: 'gain', name: 'Gain', icon: Volume2, effect: 'gainDb' },
      { key: 'normalise', name: 'Normalise', icon: AudioLines, effect: 'normalize' },
    ],
    repair: [
      { key: 'selection', name: 'Select a frequency range', icon: Crosshair, effect: 'spectralRepairs' },
      { key: 'hum', name: 'Mains hum', icon: Zap, effect: 'hum' },
      { key: 'dc', name: 'DC offset', icon: Equal, effect: 'dcHz' },
      { key: 'stereo', name: 'Stereo', icon: Headphones, effect: 'stereoWidth' },
    ],
  } as const;

  export function isToolMode(kind: TreatmentPlotMode, value: string) {
    return modes[kind].some(mode => mode.key === value);
  }
</script>

<script lang="ts">
  let { kind, selected, settings, enabled, stereo, highCut, onchoose }: {
    kind: TreatmentPlotMode;
    selected: string | null;
    settings: AudioTreatment;
    enabled: boolean;
    stereo: boolean;
    highCut: boolean;
    onchoose: (mode: string) => void;
  } = $props();

  const available = $derived(modes[kind].filter(mode => enabled
    && (mode.key !== 'stereo' || stereo) && (mode.key !== 'high' || highCut)));
  const tabStop = $derived(available.find(mode => mode.key === selected)?.key ?? available[0]?.key);

  function effectEnabled(effect: keyof AudioTreatment) {
    if (effect === 'compressorDb') return !!(settings.compressorDb || settings.compressorInputDb);
    if (effect === 'stereoWidth') {
      return !!(settings.stereoWidth || settings.balance || settings.swapChannels
        || settings.mono !== 'off' || settings.polarity !== 'off');
    }
    const value = settings[effect];
    return !!value && value !== 'off';
  }

  function bypassed(effect: keyof AudioTreatment) {
    return kind === 'eq' && Object.prototype.hasOwnProperty.call(eqBandBypassBits, effect)
      && !!(settings.eqBypassed & eqBandBypassBits[effect as keyof typeof eqBandBypassBits]);
  }

  function move(event: KeyboardEvent, key: string) {
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    const index = available.findIndex(mode => mode.key === key);
    if (index < 0) return;
    const next = event.key === 'Home' ? available[0] : event.key === 'End' ? available.at(-1)
      : available[(index + (event.key === 'ArrowDown' ? 1 : available.length - 1)) % available.length];
    if (!next) return;
    event.preventDefault();
    onchoose(next.key);
    (event.currentTarget as HTMLElement).parentElement?.querySelector<HTMLButtonElement>(`[data-tool-mode="${next.key}"]`)?.focus();
  }
</script>

<div class="mode-rail" role="toolbar" aria-orientation="vertical" aria-label="Tool modes">
  {#each modes[kind] as mode (mode.key)}
    {@const Icon = mode.icon}
    {@const active = effectEnabled(mode.effect)}
    {@const bypass = active && bypassed(mode.effect)}
    <Button variant="ghost" size="icon-sm" class="mode-button" aria-label={mode.name + ' controls'}
      title={bypass ? mode.name + ' · Bypassed' : mode.name} aria-pressed={selected === mode.key}
      aria-description={bypass ? 'Effect bypassed' : active ? 'Effect active' : undefined}
      data-effect={active && !bypass} data-bypass={bypass} data-tool-mode={mode.key} tabindex={tabStop === mode.key ? 0 : -1}
      disabled={!enabled || (mode.key === 'stereo' && !stereo) || (mode.key === 'high' && !highCut)}
      onkeydown={(event: KeyboardEvent) => move(event, mode.key)} onclick={() => onchoose(mode.key)}>
      <Icon size={16} aria-hidden="true"/>
    </Button>
  {/each}
</div>

<style>
  .mode-rail { display:flex; flex-direction:column; align-items:center; gap:var(--sp-1); min-inline-size:32px; }
  .mode-rail :global(.mode-button) { color:var(--muted); position:relative; border:0; border-radius:var(--r-compact); background:transparent; }
  .mode-rail :global(.mode-button[data-effect="true"]) { color:var(--accent-ink); }
  .mode-rail :global(.mode-button:hover:not(:disabled)) { color:var(--text); }
  .mode-rail :global(.mode-button[aria-pressed="true"]) { background:var(--sel-tint); color:var(--text); }
  .mode-rail :global(.mode-button[aria-pressed="true"][data-effect="true"]) { color:var(--accent-ink); }
  .mode-rail :global(.mode-button[data-bypass="true"])::after { content:''; position:absolute; height:1px; width:15px; background:currentColor; transform:rotate(-35deg); }
  .mode-rail :global(.mode-button:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
</style>
