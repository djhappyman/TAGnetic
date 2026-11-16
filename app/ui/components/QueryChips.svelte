<script lang="ts">
  import { placeFloating } from '../js/popup-position.js';
  import { QUERY_FIELDS, emptyRoot, newCondition, queryLabel, compactQueryLabel, queryTone, replaceNode, removeNode, addSuggestion, scopedText, appendQueryNodes, defaultOperator } from '../js/query-model.js';
  import QueryEditor from './QueryEditor.svelte';
  import QueryRuleLabel from './QueryRuleLabel.svelte';
  let root = $state<any>(emptyRoot());
  let suggestions = $state<any[]>([]);
  let proposals = $state<any[]>([]);
  let consumeDraft = $state(false);
  let highlightTerm=$state('');
  let text = $state('');
  let shown = $state(false);
  let active = $state(-1);
  let editing = $state<any>(null);
  let editingId = $state<string | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let anchor: HTMLElement;
  let popup: HTMLElement | undefined;
  let choices = $derived.by(() => {
    const out = suggestions.map(s => ({ ...s, label: s.nodes?s.nodes.map(queryLabel).join(' · '):queryLabel({...newCondition(s.field,s.value),operator:s.operator||defaultOperator(s.field)}), action: 'suggestion' }));
    for(const proposal of proposals){const label=proposal.nodes.map(queryLabel).join(' · ');if(!out.some(s=>s.label.toLowerCase()===label.toLowerCase()))out.unshift({...proposal,label,action:'rule'});}
    const raw = text.trim();
    const scoped = scopedText(raw);
    const field = scoped && QUERY_FIELDS.find(f => f.key === scoped.field);
    if (field && scoped?.text.trim()&&!out.some(s=>s.field===field.key&&s.operator==='contains'&&s.value.toLowerCase()===scoped.text.trim().toLowerCase())) out.push({ field: field.key, value: scoped.text.trim(), label: `${field.label} contains: ${scoped.text.trim()}`, action: 'contains' });
    if (raw) out.push({ action: 'text', label: `Search everywhere: ${raw}` });
    out.push({ action: 'build', label: 'Build a condition or group…' });
    return out;
  });
  function previews(choice:any){return choice.nodes||(choice.field?[{kind:'condition',id:'preview',field:choice.field,operator:choice.action==='contains'?'contains':choice.operator||defaultOperator(choice.field),values:[choice.value],negate:false}]:[]);}
  function publish(next: any) { return document.dispatchEvent(new CustomEvent('tagrex:filter-root', { cancelable:true,detail: { root: next } })); }
  function locate() {
    const field = anchor?.closest<HTMLElement>('.filter-ctl');
    if (field && popup) placeFloating(popup, field, { width:Math.max(field.offsetWidth, 320) });
  }
  function placePopup(node: HTMLElement) {
    popup = node;
    const resize = new ResizeObserver(locate);
    resize.observe(node);
    locate();
    return () => { resize.disconnect(); if (popup === node) popup = undefined; };
  }
  function focusInput() { requestAnimationFrame(() => input?.focus()); }
  function openEditor(item: any = newCondition(), consume = false) {
    consumeDraft=consume;
    shown = false; editingId = root.children.some((n: any) => n.id === item.id) ? item.id : null;
    editing = JSON.parse(JSON.stringify(item)); locate();
    requestAnimationFrame(() => document.querySelector<HTMLElement>('#query-editor select')?.focus());
  }
  function choose(choice: any) {
    shown = false;
    if (choice.action === 'build') return openEditor(newCondition(),true);
    if (choice.action !== 'text') {
      const next = choice.nodes?appendQueryNodes(root,choice.nodes):choice.action === 'suggestion' ? addSuggestion(root, choice) : { ...root, children: [...root.children, { ...newCondition(choice.field, choice.value), operator: choice.action === 'empty' ? 'empty' : 'contains' }] };
      if(!publish(next))return;
      if (input) { input.value = ''; input.dispatchEvent(new Event('input', { bubbles: true })); }
    }
    focusInput();
  }
  function request() {
    text = input?.value || '';shown=false;suggestions=[];proposals=[];active=-1;locate();
    document.dispatchEvent(new CustomEvent('tagrex:query-suggest', { detail: { text } }));
  }
  function keydown(event: KeyboardEvent) {
    if(event.key==='Escape'||(event.key==='Enter'&&event.target===input))document.dispatchEvent(new Event('tagrex:query-suggest-dismiss'));
    if (event.key === 'Escape' && (shown || editing || event.target === input)) { event.preventDefault(); event.stopPropagation(); shown = false; editing = null; focusInput(); return; }
    if (event.target !== input) return;
    if(event.altKey&&event.key==='Enter'){event.preventDefault();event.stopPropagation();openEditor();return;}
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();if(!shown){request();active=event.key==='ArrowDown'?0:choices.length-1;}else active=active<0?(event.key==='ArrowDown'?0:choices.length-1):(active+(event.key==='ArrowDown'?1:-1)+choices.length)%choices.length;
    } else if (event.key === 'Enter') { if(shown){event.preventDefault();event.stopPropagation();if(choices[active])choose(choices[active]);else shown=false;} }
    else if (event.key === 'Tab') shown = false;
    else if (event.key === 'Backspace' && !input?.value && root.children.length) { event.preventDefault(); document.querySelector<HTMLElement>('.filter-condition-chip:last-of-type button')?.focus(); }
  }
  function outside(event: PointerEvent) { if ((event.target as Element).closest('#presets-btn') || !(event.target as Element).closest('.filter-ctl, #query-editor, #query-suggestions')) { shown = false; editing = null; } }
  function receive(node: HTMLElement) {
    anchor = node;
    const changed = (event: Event) => {
      const s = (event as CustomEvent).detail;
      root = s.root || emptyRoot();
      requestAnimationFrame(locate);
    };
    const suggested = (event: Event) => { const detail=(event as CustomEvent).detail;if(detail.text!==input?.value)return;suggestions=detail.suggestions||[];proposals=detail.proposals||[];highlightTerm=detail.highlight||text;shown=!!text.trim()&&detail.ready!==false&&document.activeElement===input&&!editing;active=Math.min(active,choices.length-1);requestAnimationFrame(locate); };
    document.addEventListener('tagrex:filter-state', changed); document.addEventListener('tagrex:query-suggestions', suggested);
    const resize=new ResizeObserver(locate);const panel=node.closest('#filter-panel');if(panel)resize.observe(panel);
    const frame = requestAnimationFrame(() => {
      input = document.getElementById('filter') as HTMLInputElement;
      input?.addEventListener('input', request);input?.setAttribute('aria-description','Type to search or choose a filter. Alt+Enter builds a condition or group.');
      input?.setAttribute('role', 'combobox'); input?.setAttribute('aria-autocomplete', 'list'); input?.setAttribute('aria-controls', 'query-suggestions');
      document.dispatchEvent(new Event('tagrex:request-filter-state'));
    });
    return () => { resize.disconnect();cancelAnimationFrame(frame); input?.removeEventListener('input', request);  document.removeEventListener('tagrex:filter-state', changed); document.removeEventListener('tagrex:query-suggestions', suggested); };
  }
  $effect(() => { input?.setAttribute('aria-expanded', String(shown)); if (shown&&active>=0) input?.setAttribute('aria-activedescendant', `query-choice-${active}`); else input?.removeAttribute('aria-activedescendant'); });
</script>
<svelte:document onkeydown={keydown} onpointerdown={outside} />
<svelte:window onresize={locate} />
<span class="query-command-chips" {@attach receive}>
  {#if root.children.length > 1}<select data-size="sm" aria-label="Combine filter conditions" value={root.mode} onchange={e => publish({ ...root, mode: e.currentTarget.value })}><option value="all">Match all</option><option value="any">Match any</option></select>{/if}
  {#each root.children as item (item.id)}
    <span class="filter-condition-chip query-rule-label" data-query-tone={queryTone(item.field)} title={queryLabel(item)}><button type="button" aria-label={`Edit ${queryLabel(item)}`} onclick={() => openEditor(item)}><span class="query-committed-value">{compactQueryLabel(item)}</span></button><button type="button" aria-label={`Remove ${queryLabel(item)}`} onclick={() => { publish(removeNode(root, item.id)); focusInput(); }}><svg class="ico" aria-hidden="true"><use href="#i-close" /></svg></button></span>
  {/each}
</span>
{#if shown && !editing}
  <div id="query-suggestions" class="query-popup" role="listbox" aria-label="Search suggestions" {@attach placePopup}>
    {#each choices as choice, index (`${choice.action}:${choice.field}:${choice.value}:${index}`)}
      <div id={`query-choice-${index}`} role="option" aria-selected={active === index} aria-label={choice.label} tabindex="-1" onpointerdown={e => { e.preventDefault(); choose(choice); }} onkeydown={e => { if (e.key === 'Enter') choose(choice); }}><span class="query-proposal">{#if previews(choice).length}{#each previews(choice) as proposal, at (at)}<QueryRuleLabel node={proposal} highlight={highlightTerm} suggestion />{/each}{:else}{choice.label}{/if}</span>{#if choice.count != null}<span class="muted">{choice.count.toLocaleString()}</span>{/if}</div>
    {/each}
  </div>
{/if}
{#if editing}
  <div id="query-editor" class="query-popup" role="dialog" aria-label="Edit filter" tabindex="-1" {@attach placePopup}>
    {#key editing.id}<QueryEditor node={editing} oncancel={() => { editing = null; focusInput(); }} onsave={next => { if(!publish(editingId ? replaceNode(root, editingId, next) : { ...root, children: [...root.children, next] }))return;editing=null;if(consumeDraft&&input){input.value='';input.dispatchEvent(new Event('input',{bubbles:true}));}focusInput(); }} />{/key}
  </div>
{/if}
