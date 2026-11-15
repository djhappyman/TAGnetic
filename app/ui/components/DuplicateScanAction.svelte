<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';

  let scanning = $state(false);
  let total = $state(0);
  function connect() {
    const sync = (event: Event) => {
      const state = (event as CustomEvent).detail;
      scanning = !!state.duplicateScanning;
      total = state.total || 0;
    };
    document.addEventListener('tagrex:filter-state', sync);
    const frame = requestAnimationFrame(() => document.dispatchEvent(new Event('tagrex:request-filter-state')));
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener('tagrex:filter-state', sync);
    };
  }
</script>

<div class="duplicate-scan-actions" {@attach connect}>
  <Button id="query-duplicate-cancel" class="btn" variant="outline" size="sm" hidden={!scanning}
    onclick={() => document.dispatchEvent(new Event('tagrex:query-scan-cancel'))}>Cancel scan</Button>
  <Button id="query-duplicate-scan" class="btn primary" variant="default" size="sm" disabled={scanning || !total}
    onclick={() => document.dispatchEvent(new Event('tagrex:query-scan'))}>
    <svg class="ico" aria-hidden="true"><use href="#i-dedup" /></svg>{scanning ? 'Scanning…' : 'Scan files'}
  </Button>
</div>
