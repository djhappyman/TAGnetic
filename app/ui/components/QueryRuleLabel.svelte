<script lang="ts">
  import {fieldLabel, QUERY_OPERATORS, queryTone} from '../js/query-model.js';
  let {node, highlight='', suggestion=false}: {node:any; highlight?:string; suggestion?:boolean}=$props();
  function label(rule:any) {
    const prefix=rule.negate?'Not ':'';
    if(rule.kind==='group')return `${rule.mode==='any'?'Any':'All'} of:`;
    const name=fieldLabel(rule.field);
    return `${prefix}${name}${['member','equals'].includes(rule.operator)||(rule.field==='cover'&&['empty','present'].includes(rule.operator))?':':` ${QUERY_OPERATORS.find(op=>op.key===rule.operator)?.label||rule.operator}${['empty','present'].includes(rule.operator)?'':':'}`}`;
  }
  function values(rule:any):string[] {
    if(rule.field==='cover'&&['empty','present'].includes(rule.operator))return [rule.operator==='empty'?'missing':'embedded'];
    return ['empty','present'].includes(rule.operator)?[]:rule.values;
  }
  function marked(value:string) {
    const term=highlight.trim(),at=value.toLowerCase().indexOf(term.toLowerCase());
    return !term||at<0?{before:value,match:'',after:''}:{before:value.slice(0,at),match:value.slice(at,at+term.length),after:value.slice(at+term.length)};
  }
</script>

{#snippet ruleLabel(rule:any)}
  <span class="query-rule-label" data-query-tone={queryTone(rule.field)}>
    <span class="query-field-label">{label(rule)}</span>
    {#if rule.kind==='group'}
      {#each rule.children as child,index (child.id)}
        {#if index}<span class="query-value-join">{rule.mode==='any'?' or ':' and '}</span>{/if}
        {@render ruleLabel(child)}
      {/each}
    {:else}
      {#each values(rule) as value,index (index)}
        {#if index}<span class="query-value-join">{rule.operator==='between'?' and ':' or '}</span>{/if}
        {@const part=marked(value)}
        <span class={['query-value-chip',{'query-suggestion-chip':suggestion}]} data-field={rule.field}>{part.before}{#if part.match}<mark>{part.match}</mark>{/if}{part.after}</span>
      {/each}
    {/if}
  </span>
{/snippet}
{@render ruleLabel(node)}
