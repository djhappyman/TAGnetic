<script lang="ts">
  import { tick, untrack, onDestroy } from 'svelte';
  import FieldPatternInput from './FieldPatternInput.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { NativeSelect, NativeSelectOption } from '$lib/components/ui/native-select/index.js';
  import { buildFunction } from '../js/function-pattern.js';
  let { id, name, initial, original, display, functions, fields, onconfirm, oncancel, place }: {
    id:string;name:string;initial:string[];original?:string;display?:string[];functions:any[];fields:{key:string;label:string}[];
    onconfirm:(value:string)=>void;oncancel:()=>void;place:(node:HTMLDivElement)=>void;
  }=$props();
  let selected=$state(untrack(()=>name));
  let args=$state(untrack(()=>[...(display||initial)]));
  let pending=$state<Record<number,boolean>>({});
  const fn=$derived(functions.find(item=>item.key===selected)!);
  let building=$state(false);
  let failure=$state('');
  let alive=true;
  onDestroy(()=>{alive=false;});
  const ready=$derived(!!args[0]?.length&&!building&&!Object.values(pending).some(Boolean));
  const cache=new Map<string,string[]>();
  function change(event:Event) {
    cache.set(selected,[...args]);
    selected=(event.currentTarget as HTMLSelectElement).value;
    args=cache.get(selected)||functions.find(item=>item.key===selected)!.args.map((_:string,index:number)=>index===0?args[0]||'':'');
    pending={};
  }
  async function accept() {
    if(!ready)return;building=true;failure='';
    try{const raw=await buildFunction(selected,args,original);if(alive)onconfirm(raw);}
    catch(error){if(alive)failure=String(error);}
    finally{if(alive)building=false;}
  }
  function keydown(event:KeyboardEvent) {
    if(event.isComposing)return;
    if(event.key==='Escape'){event.preventDefault();event.stopPropagation();oncancel();}
    if(event.key==='Enter'&&!(event.target as Element).closest('select,button')&&!Object.values(pending).some(Boolean)){
      event.preventDefault();event.stopPropagation();accept();
    }
  }
  function connect(node:HTMLDivElement) {
    place(node);
    void tick().then(()=>document.getElementById(`${id}-arg-0`)?.focus());
  }
</script>

<div class="function-composer" data-function-composer data-controller-popup data-search-editor role="dialog" tabindex="-1" aria-modal="false" aria-label="Edit function" {@attach connect} onkeydown={keydown}>
  <div class="function-argument-row" style:grid-template-columns={`minmax(100px,1fr) 110px ${fn.args.slice(1).map(()=>'minmax(80px,1fr)').join(' ')} auto`}>
    {#each fn.args as prompt,index (index)}
      {#if index===1}
          <NativeSelect class="function-operation" aria-label="Operation" value={selected} disabled={building||Object.values(pending).some(Boolean)} onchange={change}>
            {#each functions as option (option.key)}<NativeSelectOption value={option.key}>{option.label}</NativeSelectOption>{/each}
          </NativeSelect>
      {/if}
      <div class="function-argument">
        <FieldPatternInput id={`${id}-arg-${index}`} label={prompt} placeholder={prompt} disabled={building} value={args[index]||''} {fields} functions
          onchange={value=>{args[index]=value;}} onpendingchange={value=>{pending[index]=value;}} />
      </div>
      {#if index===0 && fn.args.length===1}
        <NativeSelect class="function-operation" aria-label="Operation" value={selected} disabled={building||Object.values(pending).some(Boolean)} onchange={change}>
          {#each functions as option (option.key)}<NativeSelectOption value={option.key}>{option.label}</NativeSelectOption>{/each}
        </NativeSelect>
      {/if}
    {/each}
    <div class="function-composer-actions"><Button variant="ghost" size="sm" onclick={oncancel}>Cancel</Button><Button size="sm" disabled={!ready} onclick={accept}>OK</Button></div>
  </div>
  {#if failure}<p class="function-composer-error" role="alert">{failure}</p>{/if}
</div>
