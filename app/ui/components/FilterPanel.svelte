<script lang="ts">
  import type { Snippet } from 'svelte';
  import '../filter-panel.css';
  import { placeFloating } from '../js/popup-position.js';

  let { children }: { children: Snippet } = $props();
  function attachPanel(root: HTMLElement) {
    const focus = () => {
      requestAnimationFrame(() => {
        const input = root.querySelector<HTMLInputElement>('#filter');
        input?.focus();
        input?.select();
      });
    };
    function placePresets() {
      const menu = root.querySelector<HTMLElement>('#presets-menu');
      const button = root.querySelector<HTMLElement>('#presets-btn');
      if (!menu || menu.hidden || !button) return;
      menu.classList.add('floating');
      placeFloating(menu, button, { align:'right' });
    }
    const clicked = (event: MouseEvent) => {
      if ((event.target as Element).closest('#presets-btn')) {
        document.dispatchEvent(new Event('tagrex:query-suggest-dismiss'));
        requestAnimationFrame(placePresets);
      }
    };
    // Only the floating preset menu needs observation; filter changes no
    // longer rebuild a duplicate summary above the command field.
    const observer = new MutationObserver(placePresets);
    const presets = root.querySelector('#presets-menu');
    if (presets) observer.observe(presets, { attributes: true, attributeFilter: ['hidden'], childList: true, characterData: true, subtree: true });
    const resize = new ResizeObserver(placePresets);
    resize.observe(root);
    root.addEventListener('click', clicked, true);
    window.addEventListener('resize', placePresets);
    document.addEventListener('tagrex:focus-filter', focus);
    return () => {
      observer.disconnect();
      resize.disconnect();
      root.removeEventListener('click', clicked, true);
      window.removeEventListener('resize', placePresets);
      document.removeEventListener('tagrex:focus-filter', focus);
    };
  }
</script>

<div id="filter-panel" class="filter-panel" {@attach attachPanel}>
  <div id="filter-panel-content" class="filter-panel-content">
    <div class="filter-panel-inner">{@render children()}</div>
  </div>
</div>
