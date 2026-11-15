<script lang="ts">
  import type { Snippet } from 'svelte';
  import '../filter-panel.css';

  let { children }: { children: Snippet } = $props();
  function attachPanel(root: HTMLElement) {
    const focus = () => {
      // A shortcut can be invoked while the compact Inspector pane is showing.
      const library = document.getElementById('pane-library');
      if (library?.getAttribute('aria-selected') === 'false') library.click();
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
      const anchor = button.getBoundingClientRect();
      if (!menu.classList.contains('floating')) menu.classList.add('floating');
      menu.style.top = 'auto';
      menu.style.bottom = `${window.innerHeight - anchor.top + 4}px`;
      menu.style.maxHeight = `${Math.max(80, anchor.top - 12)}px`;
      menu.style.width = '';
      const width = menu.offsetWidth;
      menu.style.left = `${Math.max(8, Math.min(anchor.left, innerWidth - width - 8))}px`;
    }
    const clicked = (event: MouseEvent) => {
      if ((event.target as Element).closest('#presets-btn')) requestAnimationFrame(placePresets);
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
