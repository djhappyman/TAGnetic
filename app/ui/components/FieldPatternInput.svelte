<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { placeFloating } from '../js/popup-position.js';
  import { connectPatternController } from '../js/pattern-input.js';
  import { patternParts, fieldSuggestions, fieldHue } from '../js/field-pattern.js';
  import FunctionComposer from './FunctionComposer.svelte';
  import { loadTextFunctions, loadFunctionLayout, functionParts, functionSummary } from '../js/function-pattern.js';
  type Field = { key: string; label: string; function?: any; description?: string };
  let { id, label, value = $bindable(''), fields = [], disabled = false, invalid = false, controller = false, multiline = false, functions = false,
    placeholder = 'Type a tag name', onchange, onpendingchange }: {
    id: string; label: string; value?: string; fields?: Field[]; disabled?: boolean;
    functions?: boolean; onpendingchange?: (pending:boolean)=>void; invalid?: boolean; controller?: boolean; multiline?: boolean; placeholder?: string; onchange?: (value: string) => void;
  } = $props();
  let editor = $state<HTMLDivElement>(null!);
  let choices = $state<Field[]>([]);
  let active = $state(0);
  let queryText = $state('');
  let timer: ReturnType<typeof setTimeout>;
  let composing = false;
  let queryRange: Range | null = null;
  let history: { value: string; caret: number }[] = [];
  let historyAt = -1;
  let renderedFields: Field[];
  let renderedFunctions = false;
  let availableFunctions=$state<any[]>([]);
  let layoutValue=$state('');
  let layoutActive=false;
  let layoutQueued:string|null=null;
  let alive=true;
  async function refreshLayout(raw:string) {
    layoutQueued=raw;if(layoutActive)return;
    layoutActive=true;
    while(layoutQueued!=null&&alive){
      const current=layoutQueued;layoutQueued=null;
      try{await loadFunctionLayout(current);if(alive&&editor&&text(editor)===current&&!composing&&!draft)draw(current,document.activeElement===editor?caret():undefined);}
      catch{/* Raw patterns remain editable if presentation metadata is unavailable. */}
    }
    layoutActive=false;
  }
  let draft = $state<{name:string;args:string[];start:number;end:number;base:string;original?:string;display?:string[]}|null>(null);
  function parts(raw:string): any[] { return functionParts(raw,fields,patternParts,functions); }
  function placeComposer(node:HTMLDivElement) {
    placeFloating(node,editor,{width:Math.min(Math.max(editor.getBoundingClientRect().width,560),innerWidth-16)});
  }
  async function cancelFunction() { draft=null; await tick(); editor.focus(); }
  async function confirmFunction(raw:string) {
    if(!draft)return;
    const item=draft;
    if(text(editor)!==item.base){await cancelFunction();return;}
    draft=null; editor.focus();draw(item.base.slice(0,item.start)+raw+item.base.slice(item.end),item.start+raw.length);publish();
    await tick();editor.focus();
  }
  function editFunction(chip:HTMLElement,start:number) {
    const call=JSON.parse(chip.dataset.functionCall!);if(!call)return;
    hide();draft={name:call.key,args:call.args,start,end:start+call.raw.length,base:text(editor),original:call.raw,display:call.display_args};
  }
  let savedSelection = { start:0, end:0 };
  let composingValue = '';
  function selectionOffsets() {
    const range = selection();
    return range ? {start:textBefore(range), end:caret()} : savedSelection;
  }
  function selectRange(start: number, end: number) {
    putCaret(start); const from = selection()?.cloneRange();
    putCaret(end); const to = selection();
    if (from && to) { from.setEnd(to.endContainer, to.endOffset); window.getSelection()?.removeAllRanges(); window.getSelection()?.addRange(from); }
    savedSelection = {start, end};
  }
  function selectionChanged() { if (editor && selection()) savedSelection = selectionOffsets(); normalizeCaret(); }
  function blur() { savedSelection = selectionOffsets(); hide(); }
  const listId = $derived(`${id}-suggestions`);

  function placeSuggestions(node: HTMLDivElement) {
    placeFloating(node, editor, {width:Math.max(editor.getBoundingClientRect().width,240)});
  }
  // A marked, zero-width text position gives WebKit a real caret outside an
  // atomic span. Strip only this marker, never literal whitespace in patterns.
  const CARET = '\uFEFF';
  function caretStop() {
    const stop = document.createElement('span'); stop.dataset.caretStop = ''; stop.textContent = CARET; return stop;
  }
  function text(node: Node): string {
    if (node instanceof HTMLElement && node.dataset.pattern) return node.dataset.pattern;
    if (node instanceof HTMLElement && node.hasAttribute('data-caret-stop')) return (node.textContent || '').replace(CARET, '');
    if (node.nodeName === 'BR') return '\n';
    return node.nodeType === Node.TEXT_NODE ? node.textContent || '' : [...node.childNodes].map(text).join('');
  }
  function selection() {
    const range = window.getSelection()?.rangeCount ? window.getSelection()!.getRangeAt(0) : null;
    return range && editor.contains(range.startContainer) && editor.contains(range.endContainer) ? range : null;
  }
  function caret() {
    const range = selection();
    if (!range) return text(editor).length;
    const before = range.cloneRange(); before.selectNodeContents(editor); before.setEnd(range.endContainer, range.endOffset);
    return text(before.cloneContents()).length;
  }
  function pointAt(node: Node, end = false) {
    const range = document.createRange();
    if (node instanceof HTMLElement && node.hasAttribute('data-caret-stop')) {
      range.setStart(node.firstChild!, end ? node.textContent!.length : (node.textContent?.startsWith(CARET) ? 1 : 0));
    } else if (node.nodeType === Node.TEXT_NODE) range.setStart(node, end ? node.textContent!.length : 0);
    else { range.selectNodeContents(node); range.collapse(!end); }
    range.collapse(true); window.getSelection()?.removeAllRanges(); window.getSelection()?.addRange(range);
  }
  function putCaret(offset: number) {
    let remaining = offset;
    for (const node of editor.childNodes) {
      const length = text(node).length;
      if (node instanceof HTMLElement && node.dataset.pattern) {
        if (remaining === 0) { pointAt(node.previousSibling!, true); return; }
        if (remaining <= length) { pointAt(node.nextSibling!); return; }
      } else if (remaining <= length) {
        if (node.nodeType === Node.TEXT_NODE) {
          const range = document.createRange(); range.setStart(node, remaining); range.collapse(true);
          window.getSelection()?.removeAllRanges(); window.getSelection()?.addRange(range);
        } else {
          const range = document.createRange();
          const marker = node.textContent?.startsWith(CARET) ? 1 : 0;
          range.setStart(node.firstChild!, Math.min(remaining + marker, node.textContent!.length)); range.collapse(true);
          window.getSelection()?.removeAllRanges(); window.getSelection()?.addRange(range);
        }
        return;
      }
      remaining -= length;
    }
    if (editor.lastChild) pointAt(editor.lastChild, true);
    else { const range = document.createRange(); range.selectNodeContents(editor); range.collapse(false); window.getSelection()?.removeAllRanges(); window.getSelection()?.addRange(range); }
  }
  function normalizeCaret() {
    if (composing || !editor || document.activeElement !== editor) return;
    const range = selection(); if (!range?.collapsed) return;
    const parent = range.startContainer.nodeType === Node.TEXT_NODE ? range.startContainer.parentElement : range.startContainer as Element;
    const chip = parent?.closest<HTMLElement>('[data-pattern]');
    if (chip && editor.contains(chip)) {
      let start = 0;
      for (const node of editor.childNodes) { if (node === chip) break; start += text(node).length; }
      putCaret(start + (range.startOffset ? text(chip).length : 0));
    } else if (parent?.hasAttribute('data-caret-stop') && range.startOffset === 0 && parent.textContent?.startsWith(CARET)) pointAt(parent);
  }
  function pointerdown(event: PointerEvent) {
    hide();
    if(draft) { void cancelFunction(); return; }
    const chip = (event.target as Element).closest<HTMLElement>('[data-pattern]');
    if (!chip || !editor.contains(chip) || disabled) return;
    event.preventDefault(); editor.focus();
    let start = 0;
    for (const node of editor.childNodes) { if (node === chip) break; start += text(node).length; }
    if(chip.dataset.function){ editFunction(chip,start);return; }
    const rect = chip.getBoundingClientRect();
    putCaret(start + (event.clientX < rect.left + rect.width / 2 ? 0 : text(chip).length));
  }
  function draw(pattern: string, offset?: number) {
    const nodes: Node[] = [];
    for (const part of parts(pattern)) {
      if (part.field || part.fn) {
        if (!nodes.length || (nodes.at(-1) instanceof HTMLElement && (nodes.at(-1) as HTMLElement).dataset.pattern)) nodes.push(caretStop());
        const chip = document.createElement('span');
        chip.className = 'field-pattern-chip field-value-chip field-token-tone'; chip.contentEditable = 'false';
        chip.style.setProperty('--field-hue', String(fieldHue(part.field?.key||'function')));
        chip.dataset.pattern = part.text;
        chip.textContent = part.fn ? functionSummary(part.call,fields) : part.field.label + (part.width ? ` · ${part.width}` : '');
        if(part.fn){chip.dataset.function=part.fn.key;chip.dataset.functionCall=JSON.stringify(part.call);chip.classList.add('function-expression-chip');chip.setAttribute('aria-label',chip.textContent);chip.dataset.tooltip=part.text+' · Click or press F2 to edit';}
        chip.title = part.text;
        nodes.push(chip);
      } else nodes.push(document.createTextNode(part.text));
    }
    if (nodes.at(-1) instanceof HTMLElement && (nodes.at(-1) as HTMLElement).dataset.pattern) nodes.push(caretStop());
    layoutValue=pattern; editor.replaceChildren(...nodes); renderedFields = fields; renderedFunctions=functions;
    if (offset != null) putCaret(offset);
  }
  function hide() { clearTimeout(timer); choices = []; queryRange = null; active = 0; }
  function remember() {
    const next = { value:text(editor), caret:caret() };
    if (history[historyAt]?.value === next.value) return;
    history = [...history.slice(0, historyAt + 1), next].slice(-100); historyAt = history.length - 1;
  }
  function publish(emit = true) {
    const raw = text(editor);layoutValue=raw;
    const offset = caret();
    const expected = parts(raw).filter(part => part.field || part.fn).map(part => part.text);
    const actual = [...editor.querySelectorAll<HTMLElement>('[data-pattern]')].map(node => node.dataset.pattern);
    if (JSON.stringify(expected) !== JSON.stringify(actual)) draw(raw, offset);
    remember();
    if (controller) value = raw;
    onchange?.(raw);
    if (controller && emit) editor.dispatchEvent(new Event('input', {bubbles:true}));
  }
  function suggest() {
    hide();
    const range = selection();
    if (!range?.collapsed || range.startContainer.nodeType !== Node.TEXT_NODE) return;
    const match = (range.startContainer.textContent || '').slice(0, range.startOffset).match(/[%$]?[\p{L}\p{N}_:]{2,}$/u);
    if (!match) return;
    const pending = range.cloneRange(); pending.setStart(range.startContainer, range.startOffset - match[0].length);
    const term = match[0];
    timer = setTimeout(() => {
      if (composing || disabled || document.activeElement !== editor || !selection()?.collapsed) return;
      if (caret() !== textBefore(pending) + term.length || pending.toString() !== term) return;
      queryRange = pending; queryText = term.replace(/^[%$]/, ''); choices = fieldSuggestions([...fields,...(functions?availableFunctions.map(fn=>({key:fn.key,label:fn.label,function:fn,description:fn.description})):[])], term.replace(/^\$/, '')); active = 0;
    }, 200);
  }
  function highlight(label: string) {
    const at = label.toLocaleLowerCase().indexOf(queryText.toLocaleLowerCase());
    return at < 0 ? [label, '', ''] : [label.slice(0, at), label.slice(at, at + queryText.length), label.slice(at + queryText.length)];
  }
  function textBefore(range: Range) {
    const before = range.cloneRange(); before.selectNodeContents(editor); before.setEnd(range.startContainer, range.startOffset);
    return text(before.cloneContents()).length;
  }
  async function choose(field: Field) {
    if (!queryRange || disabled) return;
    const offset = textBefore(queryRange);
    const raw = text(editor);
    const end = offset + queryRange.toString().length;
    if(field.function) {
      const prefix=raw.slice(0,offset).match(/(%[^%]+%)\s*$/);
      const previous=prefix&&fields.some(item=>`%${item.key}%`.toLowerCase()===prefix[1].toLowerCase());
      hide();draft={name:field.function.key,args:field.function.args.map((_:string,index:number)=>index===0&&previous?prefix![1]:''),
        start:previous?prefix!.index!:offset,end,base:raw};return;
    }
    const token = `%${field.key}%`;
    hide(); editor.focus(); draw(raw.slice(0, offset) + token + raw.slice(end), offset + token.length); publish();
    await tick(); editor.focus();
  }
  function input() {
    if (composing) return;
    // Browsers leave a caret-holder BR after Select All/Delete in an empty
    // contenteditable. It is not a literal newline in the user's pattern.
    if (editor.childNodes.length === 1 && editor.firstChild?.nodeName === 'BR') editor.replaceChildren();
    publish(false); suggest();
  }
  function moveHistory(direction: number) {
    const at = historyAt + direction;
    if (at < 0 || at >= history.length) return;
    historyAt = at; const entry = history[at]; hide(); draw(entry.value, entry.caret); if (controller) value = entry.value; onchange?.(entry.value); if (controller) editor.dispatchEvent(new Event('input', {bubbles:true}));
  }
  function keydown(event: KeyboardEvent) {
    if (composing || event.isComposing || disabled) return;
    if ((event.ctrlKey || event.metaKey) && ['z', 'y'].includes(event.key.toLowerCase())) {
      event.preventDefault(); event.stopImmediatePropagation(); moveHistory(event.key.toLowerCase() === 'y' || event.shiftKey ? 1 : -1); return;
    }
    if (choices.length && ['ArrowDown', 'ArrowUp', 'Enter', 'Escape'].includes(event.key)) {
      event.preventDefault(); event.stopImmediatePropagation();
      if (event.key === 'Escape') hide();
      else if (event.key === 'Enter') void choose(choices[active]);
      else {
        active = (active + (event.key === 'ArrowDown' ? 1 : -1) + choices.length) % choices.length;
        void tick().then(() => document.getElementById(`${listId}-${active}`)?.scrollIntoView({block:'nearest'}));
      }
      return;
    }
    if(event.key==='F2'&&functions) {
      let start=0;const offset=caret();
      for(const part of parts(text(editor))) {const end=start+part.text.length;
        if(part.fn&&(offset===start||offset===end)){event.preventDefault();event.stopImmediatePropagation();draft={name:part.call.key,args:part.call.args,start,end,base:text(editor),original:part.text,display:part.call.display_args};return;}start=end;}
    }
    if (event.key === 'Enter') { event.preventDefault(); return; }
    if (event.key === 'Backspace' || event.key === 'Delete') {
      if (deleteChip(event.key === 'Backspace' ? -1 : 1)) event.preventDefault();
    }
    if (['ArrowLeft','ArrowRight'].includes(event.key) && !event.shiftKey && !event.metaKey && !event.ctrlKey && !event.altKey && selection()?.collapsed) {
      const offset = caret(); let start = 0;
      for (const part of parts(text(editor))) {
        const end = start + part.text.length;
        if ((part.field || part.fn) && ((event.key === 'ArrowRight' && offset === start) || (event.key === 'ArrowLeft' && offset === end))) {
          event.preventDefault(); hide(); putCaret(event.key === 'ArrowRight' ? end : start); return;
        }
        start = end;
      }
    }
    if (['ArrowLeft', 'ArrowRight', 'Home', 'End', 'Tab'].includes(event.key)) hide();
  }
  function deleteChip(direction: number) {
    if (!selection()?.collapsed) return false;
    const offset = caret(); let start = 0;
    for (const part of parts(text(editor))) {
      const end = start + part.text.length;
      if ((part.field || part.fn) && ((direction < 0 && offset === end) || (direction > 0 && offset === start))) {
        hide(); const raw = text(editor); draw(raw.slice(0, start) + raw.slice(end), start); publish(); return true;
      }
      start = end;
    }
    return false;
  }
  function beforeinput(event: InputEvent) {
    if (composing || event.isComposing || disabled) return;
    normalizeCaret();
    if ((event.inputType === 'deleteContentBackward' && deleteChip(-1)) || (event.inputType === 'deleteContentForward' && deleteChip(1))) event.preventDefault();
  }
  function paste(event: ClipboardEvent) {
    event.preventDefault();
    if (disabled) return;
    const range = selection(); if (!range) return;
    const raw = text(editor); const start = textBefore(range); const end = caret();
    const pasted = (event.clipboardData?.getData('text/plain') || '').replace(/[\r\n]+/g, '');
    hide(); draw(raw.slice(0, start) + pasted + raw.slice(end), start + pasted.length); publish();
  }
  function copy(event: ClipboardEvent, cut = false) {
    const range = selection(); if (!range || range.collapsed) return;
    event.preventDefault(); event.clipboardData?.setData('text/plain', text(editor).slice(textBefore(range), caret()));
    if (cut && !disabled) { hide(); const start = textBefore(range), end = caret(), raw = text(editor); draw(raw.slice(0, start) + raw.slice(end), start); publish(); }
  }
  function connect(node: HTMLDivElement) {
    // Attach once: tracking value here would remount the editable DOM on every
    // keystroke, losing the caret and the local undo history.
    untrack(() => { editor = node; draw(value); history = [{value, caret:value.length}]; historyAt = 0; });
    node.addEventListener('keydown', keydown);
    const addFunction=(event:Event)=>{
      const name=(event as CustomEvent).detail;const fn=availableFunctions.find(fn=>fn.key===name);if(!functions||!fn||disabled)return;
      const raw=text(editor),offset=selectionOffsets();
      const before=raw.slice(0,offset.start).match(/(%[^%]+%)\s*$/);
      const source=offset.start!==offset.end?raw.slice(offset.start,offset.end):before?.[1]||'';
      hide();draft={name,args:fn.args.map((_:string,index:number)=>index===0?source:''),start:source&&offset.start===offset.end?before!.index!:offset.start,end:offset.end,base:raw};
    };
    node.addEventListener('pattern:add-function',addFunction);
    const contexts:HTMLElement[]=[];
    for(let parent=node.parentElement;parent;parent=parent.parentElement)if(parent.matches('.controller-dialog-root,[role="tabpanel"],.mode-panel,.subtab-panel'))contexts.push(parent);
    const visibility=new MutationObserver(()=>{if(draft&&contexts.some(parent=>parent.hidden||parent.inert))draft=null;});
    for(const parent of contexts)visibility.observe(parent,{attributes:true,attributeFilter:['hidden','inert']});
    const disconnect = controller ? connectPatternController(node, {
      read: () => text(editor),
      write: (next: string) => { hide(); draft=null; value = next; draw(next); remember(); },
      selection: selectionOffsets, select: selectRange,
    }) : () => {};
    return () => { clearTimeout(timer); visibility.disconnect(); alive=false; onpendingchange?.(false); node.dispatchEvent(new CustomEvent('pattern:pending',{detail:false})); node.removeEventListener('keydown', keydown); node.removeEventListener('pattern:add-function',addFunction); disconnect(); };
  }
  $effect(() => {
    // Read both dependencies before branching: an external value reset must
    // still subscribe to the asynchronously arriving field catalogue.
    const nextValue = value, nextFields = fields, nextFunctions = functions;
    if (editor && (nextValue !== text(editor) || nextFields !== renderedFields || nextFunctions !== renderedFunctions) && !composing) {
      untrack(() => { if(nextValue!==text(editor))draft=null; hide(); draw(nextValue); history = [{value:nextValue, caret:nextValue.length}]; historyAt = 0; });
    }
  });
  $effect(()=>{if(functions)void loadTextFunctions().then((items:any[])=>{if(alive)availableFunctions=items;}).catch(()=>{});});
  $effect(()=>{const raw=layoutValue;if(!functions||!raw.includes('$'))return;const timer=setTimeout(()=>{void refreshLayout(raw);},120);return()=>clearTimeout(timer);});
  $effect(()=>{
    const pending=!!draft;
    onpendingchange?.(pending);
    if(editor){editor.dataset.patternPending=String(pending);editor.dispatchEvent(new CustomEvent('pattern:pending',{detail:pending,bubbles:true}));}
  });
</script>

<svelte:document onselectionchange={selectionChanged} />

<div data-search-editor={draft ? true : undefined} class={['field-pattern-control', {'field-pattern-invalid':invalid, 'field-pattern-disabled':disabled, 'field-pattern-multiline':multiline}]}>
  <div {id} class="field-pattern-editor" data-pattern-input={controller || undefined} data-pattern-functions={functions || undefined} role="combobox" aria-label={label}
    aria-autocomplete="list" aria-haspopup="listbox" aria-expanded={choices.length > 0}
    aria-controls={listId} aria-activedescendant={choices.length ? `${listId}-${active}` : undefined}
    aria-invalid={invalid} aria-disabled={disabled}
    contenteditable={!disabled && !draft} aria-readonly={!!draft} tabindex={disabled ? -1 : 0} spellcheck="false" data-placeholder={placeholder}
    {@attach connect} onbeforeinput={beforeinput} oninput={input} onpaste={paste}
    ondrop={event => { event.preventDefault(); }}
    oncopy={event => copy(event)} oncut={event => copy(event, true)}
    onpointerdown={pointerdown} onblur={blur}
    oncompositionstart={() => { composingValue = text(editor); composing = true; hide(); }}
    oncompositionend={() => { composing = false; if (text(editor) !== composingValue) { input(); if (controller) editor.dispatchEvent(new Event('input', {bubbles:true})); } }}></div>
  {#if draft}
    {#key draft}
      <FunctionComposer id={`${id}-function`} name={draft.name} initial={draft.args} original={draft.original} display={draft.display} functions={availableFunctions} {fields} onconfirm={confirmFunction} oncancel={cancelFunction} place={placeComposer} />
    {/key}
  {/if}
  {#if choices.length}
    <div id={listId} class="field-pattern-suggestions" data-controller-popup {@attach placeSuggestions} role="listbox" aria-label="Tag suggestions">
      {#each choices as field, index (field.key)}
        {@const parts = highlight(field.label)}
        <div id={`${listId}-${index}`} role="option" class="field-token-tone" style:--field-hue={fieldHue(field.key)} aria-label={field.label} aria-selected={active === index} tabindex="-1"
          onpointerenter={() => { active = index; }}
          onpointerdown={event => { event.preventDefault(); void choose(field); }}
          onkeydown={event => { if (event.key === 'Enter') void choose(field); }}>
          <span class="field-suggestion-copy"><span class="field-suggestion-label">{parts[0]}{#if parts[1]}<mark>{parts[1]}</mark>{/if}{parts[2]}</span>{#if field.description}<span class="function-suggestion-description">{field.description}</span>{/if}</span><small>{field.function ? 'Function' : field.key.startsWith('custom:') ? 'Custom Tag' : 'Tag'}</small>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .field-pattern-control { position:relative; min-width:0; background:var(--panel); border-radius:var(--input-radius,10px); color:var(--text); flex:1; width:100%; }
  .field-pattern-multiline > .field-pattern-editor { height:auto; min-height:36px; max-height:140px; white-space:pre-wrap; overflow-wrap:anywhere; overflow-y:auto; }
  :global(.field) > .field-pattern-control { background:transparent; }
  :global(.field) > .field-pattern-control:focus-within { outline:none; }
  .field-pattern-control:focus-within { outline:2px solid var(--accent); outline-offset:1px; }
  .field-pattern-invalid { outline:2px solid var(--error); }
  .field-pattern-disabled { opacity:.55; }
  .field-pattern-editor { height:36px; overflow-x:auto; overflow-y:hidden; padding:6px 10px; box-sizing:border-box; outline:none; white-space:pre; font:inherit; font-family:var(--font-literal); line-height:24px; cursor:text; caret-color:var(--accent); scrollbar-width:thin; }
  .field-pattern-editor:empty::before { content:attr(data-placeholder); color:var(--muted); pointer-events:none; }
  .field-pattern-editor::selection, .field-pattern-editor :global(::selection) { background:var(--accent); color:var(--on-accent,#fff); }
</style>
