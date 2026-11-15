<script lang="ts">
  import QueryFeedback from './QueryFeedback.svelte';
  import DuplicateScanAction from './DuplicateScanAction.svelte';
  import DuplicateResultDetails from './DuplicateResultDetails.svelte';

  let kind = $state('exact');
  let status = $state('not_scanned');
  let hasResults = $state(false);
  let group = $state.raw<any>(null);
  function connect() {
    const sync = (event: Event) => {
      const state = (event as CustomEvent).detail;
      kind = state.duplicateKind || 'exact';
      status = state.duplicateScanning ? 'running' : state.duplicateStatus || 'not_scanned';
      hasResults = !!state.duplicateScanStarted && state.matched > 0;
    };
    const selected = (event: Event) => { group = (event as CustomEvent).detail.group; };
    document.addEventListener('tagrex:filter-state', sync);
    document.addEventListener('tagrex:duplicate-selection', selected);
    const frame = requestAnimationFrame(() => {
      document.dispatchEvent(new Event('tagrex:request-filter-state'));
      document.dispatchEvent(new Event('tagrex:request-duplicate-selection'));
    });
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener('tagrex:filter-state', sync);
      document.removeEventListener('tagrex:duplicate-selection', selected);
    };
  }
</script>

<div class="duplicate-scan-controls" {@attach connect}>
  <div class="duplicate-method">
  <label for="query-duplicate-kind">Matching method</label>
  <select id="query-duplicate-kind" data-size="sm" aria-label="Duplicate matching" value={kind}
    onchange={event => document.dispatchEvent(new CustomEvent('tagrex:query-duplicate-kind', { detail: { kind: event.currentTarget.value } }))}>
    <option value="exact">Exact copies</option>
    <option value="same_audio">Same audio</option>
    <option value="likely">Likely recording</option>
  </select>
  </div>
  <div class="duplicate-scan-workflow" data-state={status}>
    <QueryFeedback scan />
    <DuplicateScanAction />
  </div>
  {#if group}<DuplicateResultDetails {group} />
  {:else if hasResults}<p class="duplicate-selection-hint">Select a group to compare its copies.</p>{/if}
</div>
