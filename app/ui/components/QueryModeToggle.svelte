<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';

  let active = $state(false);
  function connect() {
    const sync = (event: Event) => {
      active = (event as CustomEvent).detail.commandMode === 'duplicates';
    };
    document.addEventListener('tagrex:filter-state', sync);
    const frame = requestAnimationFrame(() => document.dispatchEvent(new Event('tagrex:request-filter-state')));
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener('tagrex:filter-state', sync);
    };
  }
  function toggle() {
    document.dispatchEvent(new Event('tagrex:query-suggest-dismiss'));
    document.dispatchEvent(new CustomEvent('tagrex:query-mode', { detail: { mode: active ? 'search' : 'duplicates' } }));
  }
</script>

<span class="query-mode-control" {@attach connect}>
  <Button id="query-mode-toggle" type="button" variant="ghost" size="sm"
    aria-label="Duplicates mode" aria-pressed={active}
    title={active ? 'Return to search' : 'Show duplicate files'} onclick={toggle}>
    <svg class="ico" aria-hidden="true"><use href="#i-dedup" /></svg><span>Duplicates</span>
  </Button>
</span>
