<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { ChevronLeft, ChevronRight } from '@lucide/svelte';
  import { fieldLabel } from '../js/query-model.js';

  type Track = { path: string; format: string; tags: Record<string, string>; duration_secs?: number | null; has_cover?: boolean; unreadable?: boolean };
  type Group = { id: string; title: string; reason: string; tracks: Track[]; sharedTags: Record<string, string>; differingTags: string[] };
  let { group }: { group: Group } = $props();
  const pageSize = 20;
  let pagination = $state<{ groupId: string | null; page: number }>({ groupId: null, page: 0 });
  let currentPage = $derived(pagination.groupId === group.id ? Math.min(pagination.page, Math.max(0, Math.ceil(group.tracks.length / pageSize) - 1)) : 0);
  let start = $derived(currentPage * pageSize);
  let copies = $derived(group.tracks.slice(start, start + pageSize));
  let shared = $derived(Object.entries(group.sharedTags));
  function filename(path: string) { return path.split(/[\\/]/).pop() || path; }
  function duration(seconds?: number | null) {
    if (seconds == null || !Number.isFinite(seconds)) return 'Unknown duration';
    return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, '0')}`;
  }
  function artwork(track: Track) { return track.unreadable || track.has_cover == null ? 'Artwork unknown' : track.has_cover ? 'Artwork embedded' : 'No artwork'; }
  function turnPage(next: number) { pagination = { groupId: group.id, page: next }; }
</script>

{#snippet copyLabel(track: Track)}
  <span class="duplicate-copy-name">{filename(track.path)}</span>
  <span class="duplicate-copy-meta">{track.format.toUpperCase()} · {duration(track.duration_secs)} · {artwork(track)}</span>
  <span class="duplicate-copy-path">{track.path}</span>
{/snippet}

<section id="duplicate-result-details" class="duplicate-result-details" aria-labelledby="duplicate-result-title">
  <header class="duplicate-result-heading">
    <h3 id="duplicate-result-title">{group.title}</h3>
    <span class="duplicate-copy-count">{group.tracks.length} copies selected</span>
    <p class="duplicate-match-reason">{group.reason}</p>
  </header>
  <section class="duplicate-tag-section" aria-label="Shared tags">
    <h4>Shared tags</h4>
    {#if shared.length}
      <dl class="duplicate-tag-values">
        {#each shared as [field, value] (field)}<dt>{fieldLabel(field)}</dt><dd>{value}</dd>{/each}
      </dl>
    {:else}<p class="duplicate-detail-note">No shared tag values.</p>{/if}
  </section>
  <section class="duplicate-tag-section" aria-label="Tag differences">
    <h4>Tag differences</h4>
    <p class="duplicate-detail-note">{group.differingTags.length ? group.differingTags.map(fieldLabel).join(' · ') : 'All saved tags match.'}</p>
  </section>
  <section class="duplicate-copy-section" aria-label="Copies">
    <h4>Copies</h4>
    {#each copies as track (track.path)}
      {#if group.differingTags.length}
        <details class="duplicate-copy">
          <summary>{@render copyLabel(track)}</summary>
          <div class="duplicate-copy-body"><dl class="duplicate-tag-values">
            {#each group.differingTags as field (field)}<dt>{fieldLabel(field)}</dt><dd class={{'duplicate-empty-value':!track.tags[field]}}>{track.tags[field] || 'Empty'}</dd>{/each}
          </dl></div>
        </details>
      {:else}<div class="duplicate-copy duplicate-copy-static">{@render copyLabel(track)}</div>{/if}
    {/each}
    {#if group.tracks.length > pageSize}
      <nav class="duplicate-copy-pages" aria-label="Copy pages">
        <Button variant="ghost" size="sm" disabled={currentPage === 0} onclick={() => turnPage(currentPage - 1)} aria-label="Previous copies"><ChevronLeft class="ico" aria-hidden="true" /></Button>
        <span>{start + 1}–{Math.min(start + pageSize, group.tracks.length)} of {group.tracks.length}</span>
        <Button variant="ghost" size="sm" disabled={start + pageSize >= group.tracks.length} onclick={() => turnPage(currentPage + 1)} aria-label="Next copies"><ChevronRight class="ico" aria-hidden="true" /></Button>
      </nav>
    {/if}
  </section>
</section>
