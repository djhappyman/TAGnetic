<script lang="ts">
  import { Headphones, Play, Repeat2, Square } from '@lucide/svelte';
  import { Input } from '@ui-sdk/svelte/input';
  import Button from './AudioSplitButton.svelte';
  import { frameAtTime, timeLabel, type FrameRange } from '../js/audio-split';
  import type { SpectralRepair } from '../js/audio-treatment';
  import type { PreviewBand, RepairListenCommand, RepairListenState, RepairLoopPreferences, ToolSnapshot } from '../js/audio-tools-window';

  let { snapshot, range, band, candidate, repairSlot, onlisten }: {
    snapshot: ToolSnapshot | null;
    range?: FrameRange;
    band?: PreviewBand;
    candidate?: SpectralRepair;
    repairSlot?: number;
    onlisten: (command: RepairListenCommand) => void;
  } = $props();

  const channels = { stereo: 'Stereo', left: 'Left', right: 'Right' } as const;
  const playbackMessage = {
    buffering: 'Buffering audio…',
    updating: 'Updating audio preview…',
    failed: 'Could not update the audio. The previous preview is still playing.',
  } as const;
  let loopMode = $state<'exact' | 'context'>('exact');
  let manualRange = $state.raw<FrameRange>();
  let smooth = $state(false);
  let returnTo = $state<RepairLoopPreferences['returnTo']>('hold');
  let rangeError = $state('');
  const rate = $derived(snapshot?.sampleRate ?? 48_000);
  const total = $derived(snapshot?.frames ?? 0);
  const signal = $derived(snapshot?.repairListen?.signal ?? (snapshot?.original ? 'original' : 'repaired'));
  const channel = $derived(snapshot?.repairListen?.channel ?? 'stereo');
  const contextual = $derived(range && {
    start: Math.max(0, range.start - Math.round(.75 * rate)),
    end: Math.min(total, range.end + Math.round(.75 * rate)),
  });
  const auditionRange = $derived(manualRange ?? (snapshot?.loop ? snapshot.loopRange : undefined)
    ?? (loopMode === 'context' ? contextual : range) ?? snapshot?.loopRange);
  const canListen = $derived(!!snapshot && !!auditionRange);

  function loop(region: FrameRange | undefined, selectedBand?: PreviewBand, preservePosition = false, play = true) {
    if (region) onlisten({ type: 'loop', range: region, band: selectedBand, smooth, returnTo, preservePosition, play });
  }

  function listen(mode: 'exact' | 'context') {
    loopMode = mode;
    manualRange = undefined;
    rangeError = '';
    loop(mode === 'context' ? contextual : range);
  }

  function sendPreferences() {
    onlisten({ type: 'preferences', smooth, returnTo });
  }

  function editLoop(edge: keyof FrameRange, field: HTMLInputElement) {
    if (!auditionRange) return;
    const frame = frameAtTime(field.value, rate);
    const updated = frame === null ? undefined : { ...auditionRange, [edge]: frame };
    if (!updated || updated.start < 0 || updated.end > total || updated.end <= updated.start) {
      rangeError = 'Enter a time inside the recording. The loop end must follow its start.';
      return;
    }
    rangeError = '';
    field.value = timeLabel(updated[edge], rate, 6);
    if (auditionRange[edge] === updated[edge]) return;
    manualRange = updated;
    if (snapshot?.loop) loop(updated, snapshot.monitor === 'band' ? band : undefined, true, snapshot.playing);
  }

  function loopKeys(edge: keyof FrameRange, event: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (event.key === 'Enter') {
      event.preventDefault();
      editLoop(edge, event.currentTarget);
    } else if (event.key === 'Escape' && auditionRange) {
      event.preventDefault();
      event.currentTarget.value = timeLabel(auditionRange[edge], rate, 6);
      rangeError = '';
    }
  }
</script>

<div class="listen" role="group" aria-label="Listen to repair">
  <div class="listen-main">
    <Button variant="ghost" size="sm" disabled={!range} aria-label="Audition selected area" title="Loop the selection exactly"
      onclick={() => listen('exact')}><Play size={13} aria-hidden="true"/>Listen</Button>
    <Button variant="ghost" size="sm" disabled={!range} title="Include 750 ms before and after the selection"
      onclick={() => listen('context')}><Repeat2 size={13} aria-hidden="true"/>With context</Button>
    <Button variant="ghost" size="sm" disabled={!band || !canListen} aria-label="Audition selected frequencies only"
      aria-pressed={snapshot?.monitor === 'band'} onclick={() => {
        if (snapshot?.monitor === 'band') onlisten({ type: 'signal', signal });
        else loop(auditionRange, band);
      }}><Headphones size={14} aria-hidden="true"/>Band only</Button>
  </div>
  {#if snapshot?.repairPlayback}
    <p class="listen-status" role="status">{playbackMessage[snapshot.repairPlayback]}</p>
  {/if}
  <details>
    <summary>Loop and comparison</summary>
    <div class="listen-options">
      <div class="listen-main">
        <Button variant="ghost" size="sm" disabled={!snapshot} onclick={() => onlisten({ type: 'all' })}>Whole mix</Button>
        <Button variant="ghost" size="sm" disabled={!snapshot?.playing} onclick={() => onlisten({ type: 'stop', returnTo })}>
          <Square size={12} aria-hidden="true"/>Stop
        </Button>
      </div>
      {#if snapshot?.stereo}
        <div class="listen-main" role="group" aria-label="Monitor channel">
          {#each Object.entries(channels) as [value, label] (value)}
            <Button variant="ghost" size="sm" aria-pressed={channel === value}
              onclick={() => onlisten({ type: 'channel', channel: value as RepairListenState['channel'] })}>{label}</Button>
          {/each}
        </div>
      {/if}
      {#if candidate}
        <div class="listen-main" role="group" aria-label="Preview repair strength">
          <Button variant="ghost" size="sm" disabled={candidate.amountDb === 1} aria-pressed={snapshot?.repairListen?.trial === 'light'}
            onclick={() => onlisten({ type: 'trial', strength: 'light', candidate, slot: repairSlot })}>Half strength</Button>
          <Button variant="ghost" size="sm" aria-pressed={snapshot?.repairListen?.trial === 'full'}
            onclick={() => onlisten({ type: 'trial', strength: 'full', candidate, slot: repairSlot })}>Selected strength</Button>
        </div>
      {/if}
      {#if auditionRange}
        <div class="listen-main">
          <label>Loop start
            <Input type="text" inputmode="decimal" value={timeLabel(auditionRange.start, rate, 6)}
              aria-invalid={!!rangeError} onblur={event => editLoop('start', event.currentTarget)}
              onkeydown={event => loopKeys('start', event)}/>
          </label>
          <label>Loop end
            <Input type="text" inputmode="decimal" value={timeLabel(auditionRange.end, rate, 6)}
              aria-invalid={!!rangeError} onblur={event => editLoop('end', event.currentTarget)}
              onkeydown={event => loopKeys('end', event)}/>
          </label>
          <Button variant="ghost" size="sm" onclick={() => loop(auditionRange)} disabled={!canListen}>Play loop</Button>
        </div>
      {/if}
      <div class="listen-main">
        <label class="seam"><input type="checkbox" checked={smooth} onchange={event => {
          smooth = event.currentTarget.checked;
          sendPreferences();
        }}/> Smooth loop join</label>
        <label>On stop
          <select value={returnTo} onchange={event => {
            returnTo = event.currentTarget.value as RepairLoopPreferences['returnTo'];
            sendPreferences();
          }}>
            <option value="hold">Stay at playhead</option>
            <option value="loop">Return to loop start</option>
            <option value="start">Return to audition origin</option>
          </select>
        </label>
      </div>
      {#if rangeError}<p role="status">{rangeError}</p>{/if}
    </div>
  </details>
</div>

<style>
  .listen, .listen-main { display:flex; flex-wrap:wrap; align-items:center; gap:var(--sp-1); }
  details { color:var(--muted); }
  details[open] { flex-basis:100%; }
  summary { padding:var(--sp-1); cursor:pointer; }
  .listen-options { display:grid; gap:var(--sp-1); margin-block:var(--sp-1); }
  label { display:inline-flex; align-items:center; gap:var(--sp-1); font-size:var(--text-xs); }
  label :global(input[type="text"]), select { width:auto; min-width:0; padding-inline:var(--sp-2); }
  label :global(input[type="text"]) { width:8.5em; font-family:var(--font-literal); }
  select { min-height:28px; border-radius:var(--input-radius); color:var(--text); background:var(--surface); }
  .seam { margin-inline-end:var(--sp-2); }
  p { margin:0; }
  .listen-status { color:var(--muted); font-size:var(--text-xs); }
</style>
