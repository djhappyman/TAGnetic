<script lang="ts">
  import type { Snippet } from 'svelte';
  import { createPopoverController, Popover, PopoverContent, PopoverTitle, PopoverTrigger } from '@ui-sdk/svelte/popover';

  let { label, icon, children, controller = createPopoverController(), side = 'left', disabled = false, active = false }: {
    label: string;
    icon: Snippet;
    children: Snippet;
    controller?: ReturnType<typeof createPopoverController<unknown, string>>;
    side?: 'left' | 'top';
    disabled?: boolean;
    active?: boolean;
  } = $props();
</script>

<Popover {controller}>
  <PopoverTrigger class="repair-popup-trigger" aria-label={label} title={label} {disabled} data-effect={active}>
    {@render icon()}
  </PopoverTrigger>
  <PopoverContent class="app-select-menu audio-tool-popup repair-popup" {side} align="start" sideOffset={7} collisionPadding={8}>
    <PopoverTitle class="repair-popup-title">{label}</PopoverTitle>
    {@render children()}
  </PopoverContent>
</Popover>

<style>
  :global(.repair-popup-trigger) {
    display:grid; place-items:center; inline-size:28px; block-size:28px; padding:0;
    color:var(--muted); background:transparent; border:0; border-radius:var(--r-compact);
  }
  :global(.repair-popup-trigger:hover:enabled) { color:var(--text); background:var(--row-hover); }
  :global(.repair-popup-trigger[aria-expanded=true]) { background:var(--sel-tint); }
  :global(.repair-popup-trigger:is([aria-expanded=true],[data-effect=true])) { color:var(--accent-ink); }
  :global(.repair-popup-trigger:disabled) { opacity:.45; }
  :global(.repair-popup-trigger:focus-visible) { outline:2px solid var(--accent); outline-offset:1px; }
  :global(.app-select-menu.repair-popup) {
    display:grid; gap:var(--sp-2); width:max-content; min-inline-size:160px;
    max-inline-size:min(340px, calc(100vw - 20px)); padding:var(--sp-3);
  }
  :global(.repair-popup-title) { margin:0; color:var(--text); font-size:var(--text-sm); font-weight:var(--fw-semibold,600); }
</style>
