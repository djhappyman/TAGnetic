<script lang="ts">
  import { tick, onMount } from 'svelte';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import catalogue from '../../../crates/core/src/search-fields.tsv?raw';
  import { parseSearchFields, filterSuggestions, validFilterValue, onlineSearchDrafts } from '../js/search-filter.js';
  import { fieldHue } from '../js/field-pattern.js';
  import { placeFloating } from '../js/popup-position.js';
  type Field = {key:string; parameter:string; label:string; kind:string; values:string[]};
  type Filter = {key:string; value:string};
  let source = $state('discogs');
  let text = $state('');
  let filter = $state<Filter | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let entry = $state<HTMLInputElement | null>(null);
  let pending = $state<Field | null>(null);
  let value = $state('');
  let choices = $state<Field[]>([]);
  let word = $state('');
  let active = $state(0);
  let composing = false;
  let timer: ReturnType<typeof setTimeout>;
  let insertion = { start:0, end:0 };
  let history: {text:string; filter:Filter | null}[] = [];
  let historyAt = -1;
  const fields: Field[] = $derived(parseSearchFields(catalogue, source));
  const applied = $derived(fields.find(field => field.key === filter?.key));
  const hueAliases: Record<string,string> = {label:'publisher', release_title:'album', medium:'media', classification:'genre', primary_type:'genre', secondary_type:'genre'};
  const hueKey = (key:string) => hueAliases[key] || key;
  const hue = $derived(fieldHue(hueKey(filter?.key || pending?.key || '')));
  const valid = $derived(pending && validFilterValue(pending, value));

  function notify(reason:string) {
    onlineSearchDrafts.set(source, input?.value ?? text, filter);
    document.dispatchEvent(new CustomEvent('tagrex:online-search-change', {detail:{reason, source}}));
  }
  function hide() { clearTimeout(timer); choices = []; active = 0; }
  function record() {
    history = [...history.slice(0, historyAt + 1), {text:input?.value ?? text, filter:filter ? {...filter} : null}].slice(-100);
    historyAt = history.length - 1;
  }
  function updateText() {
    if (!input) return;
    text = input.value; notify('text'); hide();
    if (composing) return;
    const at = input.selectionStart;
    if (at == null || at !== input.selectionEnd) return;
    const match = text.slice(0, at).match(/[\p{L}\p{N}_]{2,}$/u);
    if (!match) return;
    const term = match[0], currentSource = source;
    insertion = {start:at - term.length, end:at};
    timer = setTimeout(() => {
      if (composing || pending || source !== currentSource || document.activeElement !== input || input?.selectionStart !== at || input.value !== text) return;
      word = term; choices = filterSuggestions(fields, term); active = 0;
    }, 200);
  }
  function highlight(label:string) {
    const at = label.toLocaleLowerCase().indexOf(word.toLocaleLowerCase());
    return at < 0 ? [label,'',''] : [label.slice(0,at),label.slice(at,at + word.length),label.slice(at + word.length)];
  }
  async function choose(field:Field, edit = false) {
    hide(); pending = field; value = edit ? filter?.value || '' : '';
    if (edit) insertion = {start:-1,end:-1};
    document.dispatchEvent(new CustomEvent('tagrex:online-filter-edit', {detail:true}));
    await tick(); entry?.focus(); entry?.select();
  }
  function position(node:HTMLElement) {
    const box = document.getElementById('query-box');
    if (box) placeFloating(node, box, {width:box.getBoundingClientRect().width});
  }
  async function cancel() {
    pending = null; document.dispatchEvent(new CustomEvent('tagrex:online-filter-edit', {detail:false}));
    await tick(); input?.focus();
  }
  async function confirm(event:SubmitEvent) {
    event.preventDefault();
    if (!pending || !valid) return;
    record();
    if (insertion.start >= 0) text = (input?.value || '').slice(0,insertion.start) + (input?.value || '').slice(insertion.end);
    filter = {key:pending.key, value:value.trim()};
    // Sync synchronously before notifying the legacy search controller.
    if (input) input.value = text;
    pending = null; record(); notify('filter');
    document.dispatchEvent(new CustomEvent('tagrex:online-filter-edit', {detail:false}));
    await tick(); input?.focus(); input?.setSelectionRange(text.length,text.length);
  }
  function remove() { record(); filter = null; record(); notify('filter'); input?.focus(); }
  function restoreHistory(direction:number) {
    const next = historyAt + direction;
    if (next < 0 || next >= history.length) return false;
    historyAt = next; const snapshot = history[next]; text = snapshot.text; filter = snapshot.filter ? {...snapshot.filter} : null;
    if (input) input.value = text;
    notify('filter'); return true;
  }
  function keydown(event:KeyboardEvent) {
    if (composing || event.isComposing) return;
    if (pending) { event.preventDefault(); if (event.key === 'Escape') void cancel(); return; }
    if ((event.ctrlKey || event.metaKey) && ['z','y'].includes(event.key.toLowerCase()) && history.length && input?.value === history[historyAt]?.text) {
      if (restoreHistory(event.shiftKey || event.key.toLowerCase() === 'y' ? 1 : -1)) { event.preventDefault(); event.stopPropagation(); hide(); return; }
    }
    if (choices.length && ['ArrowDown','ArrowUp','Enter','Escape'].includes(event.key)) {
      event.preventDefault(); event.stopPropagation();
      if (event.key === 'Escape') hide();
      else if (event.key === 'Enter') void choose(choices[active]);
      else { active = (active + (event.key === 'ArrowDown' ? 1 : -1) + choices.length) % choices.length; void tick().then(() => document.getElementById(`online-field-${active}`)?.scrollIntoView({block:'nearest'})); }
      return;
    }
    if (event.key === 'Backspace' && input?.selectionStart === 0 && input.selectionEnd === 0 && filter) { event.preventDefault(); remove(); return; }
    if (event.key === 'ArrowLeft' && input?.selectionStart === 0 && filter) { event.preventDefault(); document.getElementById('online-filter-chip')?.focus(); }
    if (event.key === 'Enter') { event.preventDefault(); hide(); document.dispatchEvent(new Event('tagrex:online-submit')); }
    if (['ArrowLeft','ArrowRight','Home','End','Tab','Escape'].includes(event.key)) hide();
  }
  function sourceChanged(event:Event) {
    if (!(event.target instanceof HTMLSelectElement)) return;
    if (event.target.id === 'search-format') {
      const medium = fields.find(field => field.key === 'medium');
      if (medium && event.target.value) {
        record(); filter = {key:'medium', value:source === 'musicbrainz' ? ({File:'Digital Media', LP:'Vinyl'} as Record<string,string>)[event.target.value] || event.target.value : event.target.value};
        record(); notify('filter');
      }
      return;
    }
    if (event.target.id !== 'online-source') return;
    hide(); pending = null;
    onlineSearchDrafts.set(source, input?.value || '', filter);
    source = event.target.value;
    const draft = onlineSearchDrafts.get(source); text = draft.text; filter = draft.filter;
    if (input) input.value = text;
    history = []; historyAt = -1;
    document.dispatchEvent(new CustomEvent('tagrex:online-filter-edit', {detail:false}));
    notify('source');
  }
  function clear() {
    hide(); pending = null; filter = null; text = ''; history = []; historyAt = -1;
    if (input) input.value = '';
    notify('text'); document.dispatchEvent(new CustomEvent('tagrex:online-filter-edit', {detail:false}));
  }
  function outside(event:PointerEvent) {
    if (!(event.target instanceof Element) || event.target.closest('.online-filter-popover, .field-pattern-suggestions, #online-filter-chip')) return;
    hide(); if (pending) void cancel();
  }
  onMount(() => {
    document.addEventListener('tagrex:online-clear', clear);
    return () => { clearTimeout(timer); document.removeEventListener('tagrex:online-clear', clear); };
  });
  $effect(() => {
    const box = document.getElementById('query-box');
    box?.classList.toggle('search-filtered', !!filter);
    box?.style.setProperty('--field-hue', String(hue));
    const medium = document.getElementById('search-format') as HTMLSelectElement | null;
    // All chips count as the single filter, including the legacy medium select.
    if (medium) { medium.disabled = !!filter || !['discogs','musicbrainz'].includes(source); if (filter || medium.disabled) medium.value = ''; medium.dispatchEvent(new Event('tagrex:sync-select')); }
  });
</script>

<svelte:document onchange={sourceChanged} onpointerdown={outside} />
{#if filter && applied}
  <button id="online-filter-chip" data-search-editor type="button" class="field-value-chip field-token-tone" style:--field-hue={hue}
    aria-label={`${applied.label}: ${filter.value}. Edit filter`} title={`${applied.label}: ${filter.value}`}
    onclick={() => choose(applied, true)} onkeydown={event => { if (['Backspace','Delete'].includes(event.key)) {event.preventDefault();event.stopPropagation();remove();} if (event.key === 'ArrowRight') input?.focus(); if ((event.ctrlKey || event.metaKey) && ['z','y'].includes(event.key.toLowerCase())) keydown(event); }}>
    <span>{filter.value}</span>
  </button>
{/if}
<Input id="discogs-query" type="text" bind:ref={input} bind:value={text} class="online-search-text" role="combobox"
  aria-label={applied && filter ? `Search within ${applied.label}: ${filter.value}` : 'Artist, release title, or catalogue number'}
  aria-autocomplete="list" aria-haspopup="listbox" aria-expanded={choices.length > 0} aria-controls="online-field-suggestions"
  aria-activedescendant={choices.length ? `online-field-${active}` : undefined}
  placeholder={filter ? 'Search within filter…' : 'artist / album / catalog no.'} spellcheck="false"
  oninput={updateText} onkeydown={keydown} onblur={hide} onclick={() => {hide(); if(pending) void cancel();}}
  oncompositionstart={() => {composing = true;hide();}} oncompositionend={() => {composing = false;updateText();}} />
{#if choices.length}
  <div id="online-field-suggestions" data-controller-popup class="field-pattern-suggestions" role="listbox" aria-label="Search field suggestions" {@attach position}>
    {#each choices as field, index (field.key)}
      {@const parts = highlight(field.label)}
      <div id={`online-field-${index}`} role="option" class="field-token-tone" style:--field-hue={fieldHue(hueKey(field.key))}
        aria-label={field.label} aria-selected={index === active} tabindex="-1"
        onpointerenter={() => {active = index;}} onpointerdown={event => {event.preventDefault();void choose(field);}}
        onkeydown={event => {if(event.key === 'Enter') void choose(field);}}>
        <span class="field-suggestion-label">{parts[0]}{#if parts[1]}<mark>{parts[1]}</mark>{/if}{parts[2]}</span><small>Filter</small>
      </div>
    {/each}
  </div>
{/if}
{#if pending}
  <div class="online-filter-popover field-token-tone" data-controller-popup data-search-editor style:--field-hue={fieldHue(hueKey(pending.key))}
    role="dialog" aria-label={`${pending.label} filter`} tabindex="-1" {@attach position}
    onkeydown={event => {event.stopPropagation();if(event.key === 'Escape'){event.preventDefault();void cancel();}}}>
    <form onsubmit={confirm}>
    <label for="online-filter-value" class="field-suggestion-label">{pending.label}</label>
    <div class="online-filter-entry">
      <Input id="online-filter-value" bind:ref={entry} bind:value list={pending.values.length ? 'online-filter-values' : undefined}
        aria-label={pending.label} placeholder={pending.values.length ? pending.values.join(' / ') : pending.key === 'country' ? 'Country code, e.g. GB' : pending.key === 'language' ? 'Language code, e.g. eng' : pending.key.includes('date') ? 'YYYY-MM-DD' : `Enter ${pending.label.toLowerCase()}`} autocomplete="off" spellcheck="false"
        onkeydown={event => { if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); event.currentTarget.form?.requestSubmit(); } }} />
      {#if pending.values.length}<datalist id="online-filter-values">{#each pending.values as option (option)}<option value={option}></option>{/each}</datalist>{/if}
      <Button size="sm" type="submit" disabled={!valid}>OK</Button>
      <Button size="sm" variant="ghost" type="button" aria-label="Cancel filter" onclick={cancel}>Cancel</Button>
      {#if filter}<Button size="sm" variant="ghost" type="button" onclick={() => {remove();void cancel();}}>Remove</Button>{/if}
    </div>
    </form>
  </div>
{/if}
