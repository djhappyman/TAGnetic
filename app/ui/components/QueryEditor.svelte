<script lang="ts">
  import { untrack } from 'svelte';
  import { QUERY_FIELDS, QUERY_OPERATORS, newCondition, normalizeRoot, queryId, defaultOperator } from '../js/query-model.js';
  let { node, onsave, oncancel }: { node: any; onsave: (value: any) => void; oncancel: () => void } = $props();
  let draft = $state<any>(untrack(() => JSON.parse(JSON.stringify(node))));
  let error = $state('');
  function fieldChanged(item: any, field: string) { item.field = field; item.operator = defaultOperator(field); item.values = [field === 'cover' ? 'missing' : '']; }
  function alternative(item: any) {
    const group = { kind: 'group', id: queryId(), mode: 'any', children: [JSON.parse(JSON.stringify(item)), newCondition()] };
    function replace(parent: any): any { return parent.id === item.id ? group : parent.kind === 'group' ? { ...parent, children: parent.children.map(replace) } : parent; }
    draft = replace(draft);
  }
  function save(event: SubmitEvent) {
    event.preventDefault();
    try { onsave(normalizeRoot({ kind: 'group', id: '__validate__', mode: 'all', children: [JSON.parse(JSON.stringify(draft))] }).children[0]); }
    catch (e) { error = String((e as Error).message); }
  }
</script>
{#snippet editor(item: any, depth: number)}
  {#if item.kind === 'group'}
    <fieldset class="query-group-editor">
      <legend>Condition group</legend>
      <label for={`mode-${item.id}`}>Combine conditions</label>
      <select id={`mode-${item.id}`} bind:value={item.mode}><option value="all">Match all</option><option value="any">Match any</option></select>
      {#each item.children as child (child.id)}
        <div class="query-rule">
          {@render editor(child, depth + 1)}
          <button type="button" aria-label="Remove condition from group" onclick={() => { item.children = item.children.filter((n: any) => n.id !== child.id); }}>Remove</button>
        </div>
      {/each}
      <button type="button" onclick={() => item.children.push(newCondition())}>Add condition</button>
      <button type="button" disabled={depth >= 7} onclick={() => item.children.push({ kind: 'group', id: queryId(), mode: 'any', children: [newCondition()] })}>Add group</button>
    </fieldset>
  {:else}
    <div class="query-rule-fields">
      <label for={`field-${item.id}`}>Field</label>
      <select id={`field-${item.id}`} value={item.field} onchange={e => fieldChanged(item, e.currentTarget.value)}>{#each QUERY_FIELDS as field (field.key)}<option value={field.key}>{field.label}</option>{/each}</select>
      <label for={`operator-${item.id}`}>Match</label>
      <select id={`operator-${item.id}`} bind:value={item.operator}>
        {#each QUERY_OPERATORS.filter(op => (op.key!=='member'||['artist','albumartist','genre'].includes(item.field)) && (QUERY_FIELDS.find(f => f.key === item.field)?.type === 'number' || !['gt','gte','lt','lte','between'].includes(op.key))) as op (op.key)}<option value={op.key}>{op.label}</option>{/each}
      </select>
      {#if !['empty', 'present'].includes(item.operator)}
        <label for={`value-${item.id}`}>{item.operator === 'between' ? 'Lower; upper bound' : 'Values (separate alternatives with a semicolon)'}</label>
        <input id={`value-${item.id}`} maxlength="2048" value={item.values.join('; ')} oninput={e => { item.values = e.currentTarget.value.split(';').map(v => v.trim()); }} />
      {/if}
      <label class="query-negate"><input type="checkbox" bind:checked={item.negate} /> Exclude matching files</label>
      <button type="button" disabled={depth >= 7} onclick={() => alternative(item)}>Add alternative</button>
    </div>
  {/if}
{/snippet}
<form onsubmit={save} class="query-editor-form">
  {@render editor(draft, 0)}
  {#if error}<p role="alert" class="filter-error">{error}</p>{/if}
  <div class="filter-builder-actions"><button type="button" onclick={oncancel}>Cancel</button><button type="submit">Save filter</button></div>
</form>
