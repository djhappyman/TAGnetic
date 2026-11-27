<script lang="ts">
  import { ScanSearch } from '@lucide/svelte';
  import Button from './AudioSplitButton.svelte';
  import RepairPopup from './AudioSplitRepairPopup.svelte';
  import { analyseRepair, type RepairAnalysisBounds, type RepairAnalysisKind, type RepairTarget } from '../js/audio-repair-analysis';
  import type { RepairChannel } from '../js/audio-repair-plot';
  import type { TreatmentProbe } from '../js/audio-treatment-plot';

  let { probe, selection, rate, maxFrames = rate * 12, channel = 'combined', target, canInterpolate = false, onselect, onaudition, oninterpolate, onsource }: {
    probe: TreatmentProbe | null;
    selection: RepairAnalysisBounds | undefined;
    rate: number;
    maxFrames?: number;
    channel?: RepairChannel;
    target?: RepairTarget;
    canInterpolate?: boolean;
    onselect: (bounds: RepairAnalysisBounds[], target?: RepairTarget) => void;
    onsource?: (transients: boolean) => void;
    onaudition?: (event: RepairAnalysisBounds) => void;
    oninterpolate?: (bounds: RepairAnalysisBounds) => void;
  } = $props();

  const tools: readonly { id: RepairAnalysisKind; label: string }[] = [
    { id: 'tone', label: 'Tone wand' },
    { id: 'harmonics', label: 'Harmonic stack' },
    { id: 'hum', label: 'Hum and harmonics' },
    { id: 'similar', label: 'Find similar' },
    { id: 'clicks', label: 'Find isolated clicks' },
    { id: 'noise', label: 'Check steady noise' },
  ];

  let kind = $state<RepairAnalysisKind>('tone');
  let sensitivity = $state(3);
  let result = $state.raw<ReturnType<typeof analyseRepair>>();
  let checked = $state<number[]>([]);
  let cursor = $state(0);
  let reviewedChannel = $state<RepairChannel>();
  const sequential = $derived(kind === 'similar' || kind === 'clicks');
  const candidate = $derived(result?.candidates[cursor]);
  const resultCount = $derived(result?.candidates.length ?? 0);
  const needsTransients = $derived(sequential && probe?.spectrogramDetail !== 'transients');
  const currentResults = $derived(reviewedChannel === channel ? result : undefined);
  const canAnalyse = $derived(!needsTransients && canReviewSelection());
  const sourceStart = $derived(probe?.spectrogramStart);
  const sourceEnd = $derived(probe?.spectrogramEnd);

  function reset() {
    result = undefined;
    checked = [];
    cursor = 0;
  }

  function review() {
    if (!selection) return;
    const source = probe && (probe.spectrogramChannel ?? 'combined') === channel ? probe : null;
    const analysed = analyseRepair(source, selection, rate, kind, sensitivity, target ?? (channel === 'combined' ? undefined : channel));
    reviewedChannel = channel;
    result = analysed;
    checked = analysed.candidates.flatMap((item, index) => item.primary ? [index] : []);
    cursor = 0;
  }

  function applyGroup() {
    const group = result?.candidates.filter((_, index) => checked.includes(index)).map(item => item.bounds) ?? [];
    if (group.length) onselect(group);
  }

  function canReviewSelection() {
    return !!selection && selection.endFrame - selection.startFrame <= maxFrames && !!probe && (probe.spectrogramChannel ?? 'combined') === channel
      && sourceStart !== undefined && sourceEnd !== undefined
      && selection.startFrame >= sourceStart - 1 && selection.endFrame <= sourceEnd + 1;
  }
</script>

<RepairPopup label="Find and review" side="top">
  {#snippet icon()}<ScanSearch size={15} aria-hidden="true"/>{/snippet}
  <div class="analysis">
  <div class="analysis-controls">
    <label>
      Find
      <select aria-label="Find repair candidates" bind:value={kind} onchange={reset}>
        {#each tools as tool (tool.id)}
          <option value={tool.id}>{tool.label}</option>
        {/each}
      </select>
    </label>
    {#if sequential}
      <label>
        Sensitivity
        <input type="range" min="1" max="5" step="1" bind:value={sensitivity} oninput={reset}/>
      </label>
    {/if}
    <Button size="sm" variant="secondary" disabled={!canAnalyse} onclick={review}>Analyse selection</Button>
    {#if kind === 'clicks' && selection && oninterpolate}
      <Button size="sm" variant="secondary" disabled={!canInterpolate}
        title="Replace the selected samples using their immediate neighbours (up to 2 ms)"
        onclick={() => { if (selection) oninterpolate?.(selection); }}>Interpolate click</Button>
    {/if}
  </div>
  {#if !canAnalyse}
    {#if !selection}
      <p>Select an area to analyse.</p>
    {:else if selection.endFrame - selection.startFrame > maxFrames}
      <p>Select up to {Math.floor(maxFrames / rate * 10) / 10} seconds to analyse.</p>
    {:else if needsTransients || !canReviewSelection()}
      <div class="analysis-controls">
        <p>{needsTransients ? 'Short events need Original audio with Transients detail.' : 'Fit the selection in the Original spectrogram to analyse it.'}</p>
        {#if onsource}
          <Button size="sm" variant="ghost" onclick={() => onsource?.(needsTransients)}>
            {needsTransients ? 'Show transients' : 'Fit original selection'}
          </Button>
        {/if}
      </div>
    {/if}
  {/if}
  {#if kind === 'clicks' && selection && oninterpolate && !canInterpolate && !currentResults?.candidates.length}
    <p>Use a full-band Time selection up to 2 ms to replace a click across the selected channel.</p>
  {/if}
  {#if currentResults}
    <p role="status">{currentResults.detail}</p>
    {#if sequential && candidate && resultCount}
      <div class="review-actions">
        <span role="status">{candidate.label} · {(candidate.bounds.startFrame / rate).toFixed(kind === 'clicks' ? 6 : 3)} s · {cursor + 1} of {resultCount}</span>
        {#if onaudition}<Button size="sm" variant="ghost" onclick={() => onaudition?.(candidate.bounds)}>Audition event</Button>{/if}
        <Button size="sm" variant="secondary" onclick={() => onselect([candidate.bounds], candidate.channel)}>Select event</Button>
        <Button size="sm" variant="ghost" disabled={cursor === 0} onclick={() => cursor -= 1}>Previous</Button>
        <Button size="sm" variant="ghost" disabled={cursor + 1 >= resultCount} onclick={() => cursor += 1}>Next</Button>
      </div>
    {:else if resultCount}
      <div class="candidate-list" role="group" aria-label="Suggested frequency bands">
        {#each currentResults.candidates as item, index (item.bounds.lowHz)}
          <label><input type="checkbox" value={index} bind:group={checked}/> {item.label}</label>
        {/each}
      </div>
      <Button size="sm" variant="secondary" disabled={checked.length === 0} onclick={applyGroup}>Select checked bands</Button>
    {/if}
  {/if}
  </div>
</RepairPopup>

<style>
  .analysis { font-size: var(--text-xs); min-width: 0; }
  .analysis-controls, .candidate-list, .review-actions { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .review-actions { margin-block: var(--sp-2); }
  .review-actions > span { font-variant-numeric: tabular-nums; }
  .candidate-list { margin-block: var(--sp-2); }
  label { display: inline-flex; align-items: center; gap: var(--sp-1); }
  select { min-height: 28px; border-radius: var(--input-radius); color: var(--text); background: var(--surface); }
  input[type='range'] { max-width: 110px; }
  input { accent-color: var(--accent); }
  p { color: var(--muted); margin: var(--sp-1) 0; }
</style>
