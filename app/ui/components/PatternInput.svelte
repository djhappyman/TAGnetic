<script lang="ts">
  import { onMount } from 'svelte';
  import FieldPatternInput from './FieldPatternInput.svelte';
  import { patternFields } from '../js/pattern-input.js';
  import { fieldLabel } from '../js/fields.js';
  let { id, label, value = $bindable(''), direction = 'render', enabled = true,
    disabled = false, multiline = false, placeholder = 'Type a tag name', onchange }: {
    id:string; label:string; value?:string; direction?:string; enabled?:boolean;
    disabled?:boolean; multiline?:boolean; placeholder?:string; onchange?:(value:string)=>void;
  } = $props();
  let entries = $state<any[]>([]);
  let mode = $state<string | null>(null);
  const fields = $derived(enabled ? patternFields(entries, mode || direction, fieldLabel) : []);
  onMount(() => {
    let alive = true;
    // A failed catalogue must not disable editing an existing raw expression.
    void import('../js/placeholders.js').then(module => module.catalogue()).then(data => { if (alive) entries = data; }).catch(() => {});
    const changed = (event: Event) => { mode = (event as CustomEvent).detail; };
    const node = document.getElementById(id)!;
    node.addEventListener('pattern:direction', changed);
    return () => { alive = false; node.removeEventListener('pattern:direction', changed); };
  });
</script>

<FieldPatternInput {id} {label} bind:value {fields} {disabled} {multiline} {placeholder} {onchange} functions={enabled&&(mode||direction)==='render'} controller />
