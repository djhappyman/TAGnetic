<script lang="ts">
  import { untrack } from 'svelte';
  import { SegmentedControl, SegmentedControlItem } from '@ui-sdk/svelte/segmented-control';

  let { id, label, labelKey, optionAttribute, initial, options }: {
    id: string;
    label: string;
    labelKey: string;
    optionAttribute: string;
    initial: string;
    options: { value: string; label: string; labelKey?: string }[];
  } = $props();

  let value = $state<string | null>(untrack(() => initial));
  let root: HTMLDivElement;

  function connect(node: HTMLDivElement) {
    root = node;
    function reflect(event: Event) {
      value = (event as CustomEvent<string>).detail;
    }
    node.addEventListener('tagnetic:reflect-setting-choice', reflect);
    return () => node.removeEventListener('tagnetic:reflect-setting-choice', reflect);
  }

  function choose(next: string | null) {
    if (next !== null) {
      root.dispatchEvent(new CustomEvent('tagnetic:setting-choice', { detail: next, bubbles: true }));
    }
  }
</script>

<SegmentedControl {id} class="seg" aria-label={label} data-i18n-aria-label={labelKey}
  bind:value onValueChange={choose}>
  {#snippet render({ props, state, children })}
    <div {...props} {@attach connect}>{@render children?.(state)}</div>
  {/snippet}
  {#each options as option (option.value)}
    <SegmentedControlItem value={option.value} type="button" data-slot="button"
      class={['seg-btn', { active: value === option.value }]}
      data-i18n={option.labelKey} {...{ [optionAttribute]: option.value }}>
      {option.label}
    </SegmentedControlItem>
  {/each}
</SegmentedControl>
