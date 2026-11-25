<script lang="ts">
  import { Settings2 } from '@lucide/svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Popover from '$lib/components/ui/popover/index.js';
  import { hasTreatment, noTreatment, type AudioTreatment, type TreatmentAnalysis } from '../js/audio-split';

  let { settings, stereo, original, busy, analysis, error, onchange, oncompare }: {
    settings: AudioTreatment;
    stereo: boolean;
    original: boolean;
    busy: boolean;
    analysis: TreatmentAnalysis | null;
    error: string;
    onchange: (settings: AudioTreatment) => void;
    oncompare: () => void;
  } = $props();

  const enabled = $derived(hasTreatment(settings));
  const gain = $derived(analysis?.gainDb ?? 0);
  function change(patch: Partial<AudioTreatment>) {
    onchange({ ...settings, ...patch });
  }
</script>

<Popover.Root>
  <Popover.Trigger>
    {#snippet child({ props })}
      <Button {...props} class="audio-tools-trigger" variant="ghost" size="sm" title="Normalise, convert to mono or clean up the recording"
        data-treated={enabled}><Settings2 size={15}/> Audio tools</Button>
    {/snippet}
  </Popover.Trigger>
  <Popover.Content class="studio-audio-tools" align="end" collisionPadding={12}>
    <div class="audio-tools">
      <label class="check">
        <input type="checkbox" checked={settings.normalize} onchange={event => change({ normalize: event.currentTarget.checked })}/>
        <span><strong>Normalise</strong><small>Adjust loudness across the whole recording.</small></span>
      </label>

      {#if stereo}
        <label class="choice">
          <span>Convert to mono</span>
          <select value={settings.mono} onchange={event => change({ mono: event.currentTarget.value as AudioTreatment['mono'] })}>
            <option value="off">Keep stereo</option>
            <option value="mix">Mix left and right</option>
            <option value="left">Left only</option>
            <option value="right">Right only</option>
          </select>
        </label>
      {/if}

      <label class="check">
        <input type="checkbox" checked={settings.rumble} onchange={event => change({ rumble: event.currentTarget.checked })}/>
        <span>Reduce rumble</span>
      </label>

      <label class="choice">
        <span>Reduce mains hum</span>
        <select value={settings.hum} onchange={event => change({ hum: event.currentTarget.value as AudioTreatment['hum'] })}>
          <option value="off">Off</option>
          <option value="hz50">50 Hz</option>
          <option value="hz60">60 Hz</option>
        </select>
      </label>

      {#if error}
        <p class="feedback failed" role="alert">{error}</p>
      {:else if busy}
        <p class="feedback" role="status">Measuring recording…</p>
      {:else if settings.normalize && analysis}
        <p class="feedback" role="status">
          {#if analysis.loudnessLufs === null}No measurable loudness in this recording.
          {:else}Whole-recording gain {gain > 0 ? '+' : ''}{gain.toFixed(1)} dB{/if}
        </p>
      {/if}
      <div class="actions">
        <Button variant="ghost" size="sm" onclick={() => onchange(noTreatment)} disabled={!enabled}>Reset</Button>
        <Button variant="secondary" size="sm" onclick={oncompare} aria-pressed={original} disabled={!enabled || !analysis}>Hear original</Button>
      </div>
    </div>
  </Popover.Content>
</Popover.Root>

<style>
  :global(.studio-audio-tools[data-slot="popover-content"]) { z-index:139; width:min(320px, calc(100vw - 32px)); padding:var(--sp-3); }
  .audio-tools { display:grid; gap:var(--sp-2); }
  .check, .choice { display:flex; align-items:center; gap:var(--sp-2); color:var(--text); min-height:28px; margin:0; }
  .check { cursor:pointer; }
  .check span { display:grid; }
  strong { font-weight:var(--fw-strong); }
  small, .feedback { color:var(--muted); font-size:var(--text-xs); }
  .feedback { margin:0; }
  .feedback.failed { color:var(--danger); }
  input { accent-color:var(--accent); }
  .choice { justify-content:space-between; }
  select { min-width:0; width:144px; max-width:52%; min-height:28px; border:0; border-radius:var(--r-control); background:var(--bg); color:var(--text); padding-inline:var(--sp-2); }
  select:focus-visible, input:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
  .actions { display:flex; justify-content:space-between; gap:var(--sp-2); margin-top:var(--sp-1); }
  .actions :global([aria-pressed="true"]) { background:var(--sel-tint); color:var(--accent-ink); }
</style>
