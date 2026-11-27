<script lang="ts">
  import * as Tooltip from '$lib/components/ui/tooltip/index.js';
  import { connectControllerSurface, registerControllerTooltip } from '$lib/controller-surfaces.js';
  let open = $state(false);
  let anchor = $state<HTMLElement | null>(null);
  let above = $state<number>();
  let text = $state('');
  let hint = $state('');
  let root: HTMLElement;
  function connect(node: HTMLElement) {
    root = node;
    const bridge = connectControllerSurface(root, (value) => { open = value; });
    const unregister = registerControllerTooltip((target, label, shortcut) => {
      anchor = target;
      const footer = target.closest('[data-tooltip-side="top"]');
      above = footer ? Math.max(6, target.getBoundingClientRect().top - footer.getBoundingClientRect().top + 6) : undefined;
      text = label;
      hint = shortcut;
      root.hidden = false;
    });
    return { destroy() { unregister(); bridge.destroy(); } };
  }
  function changed(value: boolean) { if (!value && open) root.hidden = true; }
</script>

<div id="tooltip" class="controller-tooltip-root" hidden use:connect>
  <Tooltip.Provider delayDuration={0}>
    <Tooltip.Root {open} onOpenChange={changed} disableHoverableContent>
      <Tooltip.Content forceMount portalProps={{ disabled: true }} customAnchor={anchor} side={above === undefined ? 'bottom' : 'top'} sideOffset={above ?? 6}
        class="controller-tooltip-content" arrowClasses="hidden" id="controller-tooltip-content">
        {#if text}<span>{text}</span>{/if}
        {#if hint}<kbd class="tip-kbd">{hint}</kbd>{/if}
      </Tooltip.Content>
    </Tooltip.Root>
  </Tooltip.Provider>
</div>
