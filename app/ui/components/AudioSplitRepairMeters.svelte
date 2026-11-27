<script lang="ts">
  import { untrack } from 'svelte';
  import type { RepairMeterUpdate, RepairMonitorChannel } from '../js/audio-tools-window';

  let { reading, running, stereo, monitor }: {
    reading?: RepairMeterUpdate;
    running: boolean;
    stereo: boolean;
    monitor: RepairMonitorChannel;
  } = $props();

  const scale = [
    { db: -60, position: 0 }, { db: -36, position: 26 }, { db: -24, position: 46 },
    { db: -12, position: 64 }, { db: -6, position: 79 }, { db: -3, position: 90 }, { db: 0, position: 100 },
  ];
  const floor = scale[0].db;
  type Level = { db: number; fill: number; hold: number; until: number; clipped: boolean };
  const empty: Level = { db: -Infinity, fill: floor, hold: floor, until: 0, clipped: false };
  let levels = $state.raw<[Level, Level]>([empty, empty]);
  let lastReading: RepairMeterUpdate | undefined;
  let lastAt = 0;
  const lanes = $derived(stereo ? [0, 1] as const : [0] as const);
  const current = $derived(running && !!reading);

  function meterPosition(db: number) {
    const value = Math.max(floor, Math.min(0, db));
    const index = scale.findIndex(point => point.db >= value);
    if (index <= 0) return 0;
    const from = scale[index - 1];
    const to = scale[index];
    return from.position + (to.position - from.position) * (value - from.db) / (to.db - from.db);
  }

  function meterLabel(db: number) {
    return Number.isFinite(db) ? (db < 0 ? '−' : db > 0 ? '+' : '') + Math.abs(db).toFixed(1) : '−∞';
  }

  function measure(peak: number, previous: Level, now: number, decay: number): Level {
    const db = peak > 0 && Number.isFinite(peak) ? 20 * Math.log10(peak) : -Infinity;
    const refresh = db >= previous.hold || now >= previous.until;
    return {
      db, fill: Math.max(floor, db, previous.fill - decay),
      hold: refresh ? Math.max(floor, db) : previous.hold,
      until: refresh ? now + 850 : previous.until,
      clipped: previous.clipped || peak >= 1,
    };
  }

  $effect(() => {
    if (!running || !reading) {
      lastAt = 0;
      return;
    }
    if (reading === lastReading) return;
    const now = performance.now();
    const decay = lastAt && reading.sequence === lastReading?.sequence ? Math.min(250, now - lastAt) * .075 : Infinity;
    lastAt = now;
    lastReading = reading;
    const previous = untrack(() => levels);
    const [left, right] = reading.peaks;
    levels = [measure(stereo ? left : Math.max(left, right), previous[0], now, decay),
      measure(right, previous[1], now, decay)];
  });

  function reset(index: 0 | 1) {
    const next: [Level, Level] = [...levels];
    const level = next[index];
    next[index] = { ...level, hold: floor, until: 0, clipped: false };
    levels = next;
  }
</script>

<div class="repair-meters" role="group" aria-label="Playback output levels">
  <div class="meter-scale" aria-hidden="true">
    {#each scale as tick (tick.db)}
      <span style:left={tick.position + '%'}>{tick.db < 0 ? '−' + -tick.db : '0'}</span>
    {/each}
  </div>
  <span class="meter-unit" aria-hidden="true">dBFS</span>
  {#each lanes as index (index)}
    {@const name = stereo ? index === 0 ? 'Left' : 'Right' : 'Mono'}
    {@const enabled = !stereo || monitor === 'stereo' || monitor === name.toLowerCase()}
    {@const level = levels[index]}
    {@const db = current ? level.db : -Infinity}
    <span class={['meter-channel', { dimmed: !enabled }]} aria-hidden="true">{name[0]}</span>
    <div class="meter-track" role="meter" aria-label={name + ' playback level'} aria-valuemin={floor} aria-valuemax={0}
      aria-valuenow={Math.max(floor, Math.min(0, db))}
      aria-valuetext={current ? meterLabel(db) + ' dBFS' : 'No playback'}>
      <span class="meter-fill" style:--level={meterPosition(current ? level.fill : floor) + '%'}></span>
      {#if level.hold > floor}
        <span class="meter-hold" style:left={meterPosition(level.hold) + '%'}></span>
      {/if}
    </div>
    <button type="button" class={['meter-value', { clipped: level.clipped }]}
      aria-label={'Reset ' + name.toLowerCase() + ' peak hold' + (level.clipped ? ', clipping detected' : '')}
      title={level.clipped ? 'Clipping detected · click to reset' : 'Click to reset peak hold'}
      onclick={() => reset(index)}>
      {level.clipped ? 'CLIP' : current ? meterLabel(db) : '—'}
    </button>
  {/each}
</div>

<style>
  .repair-meters {
    display: grid; grid-template-columns: 1ch minmax(120px, 1fr) 5ch; align-items: center;
    gap: 2px var(--sp-2); min-width: 0; color: var(--muted); font: 10px/1.15 var(--font-literal);
    font-variant-numeric: tabular-nums;
  }
  .meter-scale { grid-column: 2; position: relative; height: 1em; }
  .meter-scale span { position: absolute; transform: translateX(-50%); white-space: nowrap; }
  .meter-scale span:first-child { transform: none; }
  .meter-scale span:last-child { transform: translateX(-100%); }
  .meter-unit { grid-column: 3; text-align: right; }
  .meter-channel { grid-column: 1; color: var(--text); }
  .meter-channel.dimmed { color: var(--muted); }
  .meter-track { position: relative; grid-column: 2; height: 6px; border-radius: var(--r-pill); background: var(--row-hover); }
  .meter-fill {
    position: absolute; inset: 0; border-radius: inherit; clip-path: inset(0 calc(100% - var(--level)) 0 0);
    background: linear-gradient(90deg, var(--sel-tint), var(--accent), var(--accent-ink));
    background: linear-gradient(90deg, var(--sel-tint), var(--accent) 62%, oklch(from var(--accent-ink) l c calc(h + 30)));
  }
  .meter-hold { position: absolute; inset-block: -1px; width: 1px; background: var(--text); transform: translateX(-100%); }
  .meter-value {
    grid-column: 3; min-width: 5ch; min-height: 14px; padding: 0; text-align: right; border: 0;
    border-radius: var(--r-compact); background: transparent; color: inherit; font: inherit;
  }
  .meter-value:hover { color: var(--text); }
  .meter-value:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .meter-value.clipped { color: var(--del); font-weight: var(--fw-semibold, 600); }
  @media (max-width: 500px) {
    .meter-scale span:nth-child(2), .meter-scale span:nth-child(6) { display: none; }
  }
</style>
