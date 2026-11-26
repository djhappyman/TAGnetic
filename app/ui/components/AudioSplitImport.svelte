<script lang="ts">
  import { Check } from '@lucide/svelte';
  import { Checkbox } from '@ui-sdk/svelte/checkbox';
  import { Input } from '@ui-sdk/svelte/input';
  import { NumberInput, NumberInputControl } from '@ui-sdk/svelte/number-input';
  import Button from './AudioSplitButton.svelte';
  import { audioCommand, leaf, openDiscogsRelease, timeLabel, type Cue, type Proposal, type Source, type Tags, type Track } from '../js/audio-split';

  let { source, playhead, mode, cue, cueEncoding, cueLoading, onencoding, onapply, oncancel, onerror }: {
    source: Source; playhead: number; mode: 'cue' | 'discogs'; cue?: Cue;
    cueEncoding: 'utf8' | 'windows1252'; cueLoading: boolean;
    onencoding: (encoding: 'utf8' | 'windows1252') => void;
    onapply: (tracks: Track[], tags: Tags, replace: boolean) => void; oncancel: () => void;
    onerror: (message: string) => void;
  } = $props();

  type Anchor = { startIndex: number; endIndex: number; frame: number };
  let release = $state('');
  let proposal = $state<Proposal | null>(null);
  let anchors = $state<Anchor[]>([]);
  let working = $state(false);
  let releaseId = $state('');
  let from = $state<number | null>(1);
  let to = $state<number | null>(1);
  let overwrite = $state(false);
  let cueConfirmed = $state(false);
  let live = true;
  $effect(() => () => { live = false; });
  function focusHeading(node: HTMLHeadingElement) {
    node.focus({ preventScroll: true });
  }
  const fileMismatch = $derived(!!cue && leaf(cue.sourceName) !== leaf(source.path));
  const candidates = $derived(proposal?.rows.filter(row => row.frame != null) || []);
  const warnings = $derived(mode === 'cue' ? cue?.warnings || [] : proposal?.warnings || []);
  const sourceKey = (frame: number) => 'import-' + frame;
  const sideRanges = $derived.by(() => {
    const ranges = new Map<string, { first: number; last: number; unambiguous: boolean; positions: Set<string> }>();
    proposal?.rows.forEach(({ side, position }, index) => {
      if (!side) return;
      const track = index + 1;
      const key = position.trim().toUpperCase();
      const range = ranges.get(side);
      if (!range) {
        ranges.set(side, { first: track, last: track, unambiguous: true, positions: new Set([key]) });
        return;
      }
      range.unambiguous &&= range.last + 1 === track && !range.positions.has(key);
      range.last = track;
      range.positions.add(key);
    });
    return [...ranges].filter(([, range]) => range.unambiguous);
  });

  function releaseNumber(value: string) {
    const trimmed = value.trim();
    if (/^\d+$/.test(trimmed)) return trimmed;
    try {
      const url = new URL(trimmed);
      if (url.hostname !== 'www.discogs.com' && url.hostname !== 'discogs.com') return '';
      return url.pathname.match(/\/release\/(\d+)/)?.[1] || '';
    } catch { return ''; }
  }

  async function request(id: string, ranges: Anchor[]) {
    working = true;
    onerror('');
    const ordered = [...ranges].sort((first, second) => first.startIndex - second.startIndex);
    try {
      const result = await audioCommand<Proposal>('discogs', { id: source.id, releaseId: id, anchors: ordered });
      if (!live) return;
      proposal = result;
      anchors = ordered;
      releaseId = id;
      if (!ordered.length) { from = 1; to = result.rows.length; }
    } catch (error) { if (live) onerror(String(error)); }
    finally { working = false; }
  }

  function lookup(event: SubmitEvent) {
    event.preventDefault();
    const id = releaseNumber(release);
    if (!id) { onerror('Enter a Discogs release ID or a release link.'); return; }
    void request(id, []);
  }

  function anchor() {
    const count = proposal?.rows.length ?? 0;
    if (from === null || to === null || !Number.isInteger(from) || !Number.isInteger(to) || from < 1 || to < from || to > count) {
      onerror('Choose a valid first and last track.');
      return;
    }
    const startIndex = from - 1;
    const endIndex = to;
    const range = { startIndex, endIndex, frame: playhead };
    const others = anchors.filter(anchor => anchor.startIndex !== startIndex || anchor.endIndex !== endIndex);
    if (others.some(anchor => anchor.startIndex < endIndex && startIndex < anchor.endIndex)) {
      onerror('This track range overlaps an existing anchor. Remove that anchor first.');
      return;
    }
    void request(releaseId, [...others, range]);
  }

  async function showRelease(event: MouseEvent) {
    event.preventDefault();
    try { await openDiscogsRelease(releaseId); }
    catch (error) { if (live) onerror(String(error)); }
  }

  function apply() {
    if (mode === 'cue' && cue) {
      onapply(cue.markers.map((marker, index) => ({
        id: sourceKey(marker.frame), startFrame: marker.frame, include: true,
        tags: {
          ...(marker.title == null ? {} : { title: marker.title }),
          ...(marker.artist == null ? {} : { artist: marker.artist }),
          ...marker.tags,
        },
        position: String(index + 1), pregapFrame: marker.pregapFrame, index00: marker.index00, index01: marker.index01,
        pregap: marker.pregap, postgap: marker.postgap,
      })), cue.tags || {}, overwrite);
    } else if (mode === 'discogs' && proposal && candidates.length) {
      const proposed = candidates.flatMap((row) => row.frame == null ? [] : [{
        id: sourceKey(row.frame), startFrame: row.frame, include: true,
        position: row.position, tags: { title: row.title, ...(row.performer ? { artist: row.performer } : {}) },
      }]) as Track[];
      const boundaries = new Set(proposed.map(track => track.startFrame));
      for (const span of proposal.spans || []) {
        const end = span.expectedEndFrame;
        const available = span.availableEndFrame ?? source.frames;
        if (end != null && end > 0 && end < available && end < source.frames && !boundaries.has(end)) {
          proposed.push({ id: sourceKey(end), startFrame: end, include: true, tags: { title: 'Unassigned audio' } });
          boundaries.add(end);
        }
      }
      onapply(proposed, proposal.tags || {}, overwrite);
    }
  }
</script>

{#snippet checkbox(checked: boolean, label: string, onchange: (checked: boolean) => void)}
  <Checkbox {checked} class="import-check" disabled={working} onCheckedChange={next => onchange(next === true)}>
    <span class="checkmark" aria-hidden="true">{#if checked}<Check size={12}/>{/if}</span>
    {label}
  </Checkbox>
{/snippet}

<section class="importer" aria-label={mode === 'cue' ? 'CUE import review' : 'Discogs timing proposals'}>
  <div class="header">
    <h3 tabindex="-1" {@attach focusHeading}>{mode === 'cue' ? 'Review CUE markers' : 'Use a Discogs release'}</h3>
    <Button variant="ghost" size="sm" onclick={oncancel} disabled={working}>Close import</Button>
  </div>
  {#if mode === 'cue'}
    <label class="encoding">Sheet encoding
      <select value={cueEncoding} onchange={event => onencoding(event.currentTarget.value as 'utf8' | 'windows1252')} disabled={cueLoading}>
        <option value="utf8">UTF-8 or Unicode BOM</option>
        <option value="windows1252">Windows-1252</option>
      </select>
    </label>
    {#if cueLoading}<p role="status">Reading the CUE sheet…</p>{/if}
    {#if cue}
    <p>Master named in sheet: <strong>{cue.sourceName}</strong></p>
    {#if fileMismatch}
      {@render checkbox(cueConfirmed, `Use ${leaf(source.path)} as the master for this sheet`, checked => { cueConfirmed = checked; })}
    {/if}
    <ol>
      {#each cue.markers as marker, index (index)}
        <li><span>{marker.tags?.title || marker.title || `Track ${index + 1}`}</span><span>{timeLabel(marker.frame, source.sampleRate)}</span></li>
      {/each}
    </ol>
    {/if}
  {:else if mode === 'discogs'}
    <form class="chooser" onsubmit={lookup}>
      <label>Release ID or link <Input class="import-field" bind:value={release} spellcheck="false" placeholder="Discogs release URL" disabled={working}/></label>
      <Button type="submit" size="sm" variant="outline" disabled={working || !release.trim()}>Load tracklist</Button>
    </form>
    {#if proposal}
      <a href={`https://www.discogs.com/release/${releaseId}`} onclick={showRelease}>Data provided by Discogs.</a>
      <p>Choose a side or a range, position the playhead where it starts, then set an anchor. A missing length leaves later cuts unresolved until another anchor.</p>
      {#if sideRanges.length}
        <div class="side-ranges" role="group" aria-label="Track ranges by side or disc">
          {#each sideRanges as [side, range] (side)}
            <Button type="button" size="sm" variant="secondary" aria-pressed={from === range.first && to === range.last}
              onclick={() => { from = range.first; to = range.last; }} disabled={working}>
              {side.startsWith('Disc ') ? side : 'Side ' + side}
            </Button>
          {/each}
        </div>
      {/if}
      <div class="chooser">
        <label>First track
          <NumberInput min={1} max={proposal.rows.length} bind:value={from} disabled={working}>
            <NumberInputControl class="import-field" role="spinbutton" aria-valuemin={1} aria-valuemax={proposal.rows.length} aria-valuenow={from ?? undefined}/>
          </NumberInput>
        </label>
        <label>Last track
          <NumberInput min={1} max={proposal.rows.length} bind:value={to} disabled={working}>
            <NumberInputControl class="import-field" role="spinbutton" aria-valuemin={1} aria-valuemax={proposal.rows.length} aria-valuenow={to ?? undefined}/>
          </NumberInput>
        </label>
        <Button type="button" size="sm" variant="secondary" onclick={anchor} disabled={working}>Anchor at {timeLabel(playhead, source.sampleRate, 1)}</Button>
      </div>
      {#if anchors.length}
        <ul class="anchors">
          {#each anchors as item, index (item.startIndex)}
            <li><span>Tracks {item.startIndex + 1}–{item.endIndex} · {timeLabel(item.frame, source.sampleRate)}</span><Button size="sm" variant="ghost" disabled={working} onclick={() => request(releaseId, anchors.filter((_, i) => i !== index))}>Remove anchor</Button></li>
          {/each}
        </ul>
      {/if}
      {#each proposal.spans || [] as span (span.startIndex)}
        {@const expected = span.expectedEndFrame}
        {@const available = span.availableEndFrame ?? source.frames}
        {#if expected != null && expected !== available}
          <p class="warning">Tracks {span.startIndex + 1}–{span.endIndex} are {timeLabel(Math.abs(available - expected), source.sampleRate)} {expected < available ? 'shorter' : 'longer'} than the available audio.{expected < available ? ' Extra audio will appear as its own region.' : ''}</p>
        {/if}
      {/each}
      <ol>
        {#each proposal.rows as row, index (index)}
          <li><span>{index + 1}. {row.position} {row.title}</span><span>{row.frame != null ? timeLabel(row.frame, source.sampleRate) + ' est.' : row.durationSeconds == null ? 'No duration' : `${row.durationSeconds}s`}</span></li>
        {/each}
      </ol>
    {:else if working}<p role="status">Loading release…</p>{/if}
  {/if}
  {#each warnings as warning, index (index)}<p class="warning">{warning}</p>{/each}
  <div class="chooser">
    {@render checkbox(overwrite, 'Replace existing cuts with these markers', checked => { overwrite = checked; })}
    <Button size="sm" type="button" onclick={apply} disabled={working || cueLoading || (mode === 'cue' ? !cue?.markers.length || fileMismatch && !cueConfirmed : !candidates.length)}>Use {mode === 'cue' ? 'CUE markers' : 'proposed cuts'}</Button>
  </div>
</section>

<style>
  .importer { display:grid; gap:var(--sp-2); background:var(--panel); border-radius:var(--r-inset); padding:var(--sp-3); }
  .header, .chooser { display:flex; align-items:end; justify-content:space-between; gap:var(--sp-2); flex-wrap:wrap; }
  .side-ranges { display:flex; flex-wrap:wrap; gap:var(--sp-1); }
  .side-ranges :global(button[data-variant="secondary"]), .chooser :global(button[data-variant="secondary"]) { background:var(--bg); }
  .side-ranges :global(button[data-variant="secondary"]:where(:hover:not(:disabled))), .chooser :global(button[data-variant="secondary"]:hover:not(:disabled)) { background:var(--row-hover); }
  .side-ranges :global(button[aria-pressed="true"]) { background:var(--sel-tint); color:var(--accent-ink); }
  .chooser label { flex:1 1 8rem; }
  h3,p { margin:0; } h3 { font-size:var(--text-sm); font-weight:600; }
  p, label { color:var(--muted); }
  a { justify-self:start; color:var(--accent-ink); text-decoration:underline; }
  label { display:grid; gap:var(--sp-1); }
  .importer :global(.import-check) { display:flex; align-items:center; gap:var(--sp-2); width:fit-content; padding:var(--sp-1) var(--sp-2); border:0; border-radius:var(--r-compact); background:transparent; color:var(--text); text-align:start; }
  .importer :global(.import-check:hover:enabled) { background:var(--row-hover); }
  .importer :global(.import-check:focus-visible) { outline:2px solid var(--accent); }
  .importer :global(.import-check:disabled) { opacity:.5; }
  .checkmark { flex:0 0 15px; height:15px; display:grid; place-items:center; border:1px solid var(--border); border-radius:4px; background:var(--bg); }
  :global(.import-check[aria-checked="true"]) .checkmark { border-color:var(--accent); background:var(--accent); color:var(--primary-foreground); }
  .encoding { justify-self:start; }
  select, .importer :global(.import-field) { width:100%; height:32px; min-width:0; color:var(--text); background:var(--bg); border:0; border-radius:var(--r-control); padding:0 var(--sp-2); }
  select:focus-visible, .importer :global(.import-field:focus-visible) { outline:2px solid var(--accent); }
  ol, ul { display:grid; gap:var(--sp-1); padding:0; margin:0; list-style:none; max-height:170px; overflow:auto; overscroll-behavior:contain; }
  li { display:flex; align-items:center; justify-content:space-between; gap:var(--sp-2); padding:var(--sp-1) var(--sp-2); }
  ol li span:last-child { font-family:var(--font-literal); flex-shrink:0; color:var(--muted); }
  .warning { padding:var(--sp-1) var(--sp-2); background:var(--bg); border-radius:var(--r-compact); }
</style>
