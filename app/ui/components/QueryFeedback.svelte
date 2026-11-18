<script lang="ts">
  let { scan = false }: { scan?: boolean } = $props();
  let feedback = $state('');
  let error = $state(false);
  let status = $state('not_scanned');
  let title = $derived(status === 'running' ? 'Scanning files' : status === 'complete' ? 'Scan complete' : status === 'cancelled' ? 'Scan cancelled' : status === 'failed' ? 'Scan failed' : 'Ready to scan');
  function receive() {
    const changed = (event: Event) => {
      const s=(event as CustomEvent).detail;
      error=!!s.error;
      status=s.duplicateScanning?'running':s.duplicateStatus||'not_scanned';
      feedback=s.error||s.duplicateMessage||(!s.pending&&s.total&&!s.matched?'No files match. Edit or clear a filter.':'');
      if(scan&&status==='not_scanned'&&!s.error)feedback='Duplicate groups will appear in the file table.';
    };
    document.addEventListener('tagrex:filter-state',changed);
    document.dispatchEvent(new Event('tagrex:request-filter-state'));
    return()=>document.removeEventListener('tagrex:filter-state',changed);
  }
</script>
<div id="filter-feedback" class={['filter-feedback',{'filter-error':error,'duplicate-scan-status':scan,'duplicate-ready':scan&&status==='not_scanned'&&!error}]} data-scan-status={scan?status:undefined} hidden={!scan&&!feedback} role="status" aria-live="polite" {@attach receive}>
  {#if scan && status === 'not_scanned' && !error}
    <svg class="ico duplicate-ready-icon" aria-hidden="true"><use href="#i-search" /></svg>
    <h3>Find duplicate files</h3>
    <p>Scan the open files to find matching copies.</p>
  {:else}
    {#if scan}<div class="duplicate-status-heading"><svg class="ico" aria-hidden="true"><use href={status==='complete'?'#i-check':'#i-dedup'} /></svg><strong>{title}</strong></div>{/if}
    <span class="query-feedback-message">{feedback}</span>
  {/if}
</div>
