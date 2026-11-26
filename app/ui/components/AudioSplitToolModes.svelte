<script lang="ts">
  import { Activity, ArrowDownToLine, ArrowUpToLine, AudioLines, ChartSpline, Crosshair, Equal, Headphones, ScanLine, TrendingDown, TrendingUp, Volume2, VolumeX, Zap } from '@lucide/svelte';
  import Button from './AudioSplitButton.svelte';
  import type { AudioTreatment } from '../js/audio-treatment';
  import type { TreatmentPlotMode } from '../js/audio-treatment-plot';

  let { kind, selected, settings, enabled, stereo, highCut, onchoose }: {
    kind: TreatmentPlotMode;
    selected: string | null;
    settings: AudioTreatment;
    enabled: boolean;
    stereo: boolean;
    highCut: boolean;
    onchoose: (mode: string) => void;
  } = $props();

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

  function effectEnabled(effect: keyof AudioTreatment) {
    if (effect === 'stereoWidth') {
      return !!(settings.stereoWidth || settings.balance || settings.swapChannels
        || settings.mono !== 'off' || settings.polarity !== 'off');
    }
    const value = settings[effect];
    return !!value && value !== 'off';
  }
</script>

<div class="mode-rail" role="group" aria-label="Tool modes">
  {#each modes[kind] as mode (mode.key)}
    {@const Icon = mode.icon}
    <Button variant="ghost" size="icon-sm" class="mode-button" aria-label={mode.name + ' controls'} title={mode.name}
      aria-pressed={selected === mode.key} data-effect={effectEnabled(mode.effect)}
      disabled={!enabled || (mode.key === 'stereo' && !stereo) || (mode.key === 'high' && !highCut)}
      onclick={() => onchoose(mode.key)}>
      <Icon size={16} aria-hidden="true"/>
    </Button>
  {/each}
</div>

<style>
  .mode-rail { display:flex; flex-direction:column; align-items:center; gap:var(--sp-1); min-inline-size:32px; }
  .mode-rail :global(.mode-button) { color:var(--muted); }
  .mode-rail :global(.mode-button[data-effect="true"]) { color:var(--accent-ink); }
  .mode-rail :global(.mode-button[aria-pressed="true"]) { background:var(--sel-tint); color:var(--text); }
  .mode-rail :global(.mode-button[aria-pressed="true"][data-effect="true"]) { color:var(--accent-ink); }
  .mode-rail :global(.mode-button:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
</style>
