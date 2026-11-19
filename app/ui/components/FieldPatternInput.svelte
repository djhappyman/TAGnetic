<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { placeFloating } from '../js/popup-position.js';
  import { patternParts, fieldSuggestions, fieldHue } from '../js/field-pattern.js';
  type Field = { key: string; label: string };
  let { id, label, value = '', fields = [], disabled = false, invalid = false,
    placeholder = 'Type a tag name', onchange }: {
    id: string; label: string; value?: string; fields?: Field[]; disabled?: boolean;
    invalid?: boolean; placeholder?: string; onchange: (value: string) => void;
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
  const listId = $derived(`${id}-suggestions`);

  function placeSuggestions(node: HTMLDivElement) {
    placeFloating(node, editor, {width:editor.getBoundingClientRect().width});
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
    const chip = (event.target as Element).closest<HTMLElement>('[data-pattern]');
    if (!chip || !editor.contains(chip) || disabled) return;
    event.preventDefault(); editor.focus();
    let start = 0;
    for (const node of editor.childNodes) { if (node === chip) break; start += text(node).length; }
    const rect = chip.getBoundingClientRect();
    putCaret(start + (event.clientX < rect.left + rect.width / 2 ? 0 : text(chip).length));
  }
  function draw(pattern: string, offset?: number) {
    const nodes: Node[] = [];
    for (const part of patternParts(pattern, fields)) {
      if (part.field) {
        if (!nodes.length || (nodes.at(-1) instanceof HTMLElement && (nodes.at(-1) as HTMLElement).dataset.pattern)) nodes.push(caretStop());
        const chip = document.createElement('span');
        chip.className = 'field-pattern-chip field-token-tone'; chip.contentEditable = 'false';
        chip.style.setProperty('--field-hue', String(fieldHue(part.field.key)));
        chip.dataset.pattern = part.text; chip.textContent = part.field.label;
        nodes.push(chip);
      } else nodes.push(document.createTextNode(part.text));
    }
    if (nodes.at(-1) instanceof HTMLElement && (nodes.at(-1) as HTMLElement).dataset.pattern) nodes.push(caretStop());
    editor.replaceChildren(...nodes); renderedFields = fields;
    if (offset != null) putCaret(offset);
  }
  function hide() { clearTimeout(timer); choices = []; queryRange = null; active = 0; }
  function remember() {
    const next = { value:text(editor), caret:caret() };
    if (history[historyAt]?.value === next.value) return;
    history = [...history.slice(0, historyAt + 1), next].slice(-100); historyAt = history.length - 1;
  }
  function publish() { remember(); onchange(text(editor)); }
  function suggest() {
    hide();
    const range = selection();
    if (!range?.collapsed || range.startContainer.nodeType !== Node.TEXT_NODE) return;
    const match = (range.startContainer.textContent || '').slice(0, range.startOffset).match(/%?[\p{L}\p{N}_:]{2,}$/u);
    if (!match) return;
    const pending = range.cloneRange(); pending.setStart(range.startContainer, range.startOffset - match[0].length);
    const term = match[0];
    timer = setTimeout(() => {
      if (composing || disabled || document.activeElement !== editor || !selection()?.collapsed) return;
      if (caret() !== textBefore(pending) + term.length || pending.toString() !== term) return;
      queryRange = pending; queryText = term.replace(/^%/, ''); choices = fieldSuggestions(fields, term); active = 0;
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
    const token = `%${field.key}%`;
    hide(); editor.focus(); draw(raw.slice(0, offset) + token + raw.slice(end), offset + token.length); publish();
    await tick(); editor.focus();
  }
  function input() { if (composing) return; publish(); suggest(); }
  function moveHistory(direction: number) {
    const at = historyAt + direction;
    if (at < 0 || at >= history.length) return;
    historyAt = at; const entry = history[at]; hide(); draw(entry.value, entry.caret); onchange(entry.value);
  }
  function keydown(event: KeyboardEvent) {
    if (composing || event.isComposing || disabled) return;
    if ((event.ctrlKey || event.metaKey) && ['z', 'y'].includes(event.key.toLowerCase())) {
      event.preventDefault(); event.stopPropagation(); moveHistory(event.key.toLowerCase() === 'y' || event.shiftKey ? 1 : -1); return;
    }
    if (choices.length && ['ArrowDown', 'ArrowUp', 'Enter', 'Escape'].includes(event.key)) {
      event.preventDefault(); event.stopPropagation();
      if (event.key === 'Escape') hide();
      else if (event.key === 'Enter') void choose(choices[active]);
      else {
        active = (active + (event.key === 'ArrowDown' ? 1 : -1) + choices.length) % choices.length;
        void tick().then(() => document.getElementById(`${listId}-${active}`)?.scrollIntoView({block:'nearest'}));
      }
      return;
    }
    if (event.key === 'Enter') { event.preventDefault(); return; }
    if (event.key === 'Backspace' || event.key === 'Delete') {
      if (deleteChip(event.key === 'Backspace' ? -1 : 1)) event.preventDefault();
    }
    if (['ArrowLeft','ArrowRight'].includes(event.key) && !event.shiftKey && !event.metaKey && !event.ctrlKey && !event.altKey && selection()?.collapsed) {
      const offset = caret(); let start = 0;
      for (const part of patternParts(text(editor), fields)) {
        const end = start + part.text.length;
        if (part.field && ((event.key === 'ArrowRight' && offset === start) || (event.key === 'ArrowLeft' && offset === end))) {
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
    for (const part of patternParts(text(editor), fields)) {
      const end = start + part.text.length;
      if (part.field && ((direction < 0 && offset === end) || (direction > 0 && offset === start))) {
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
    const pasted = event.clipboardData?.getData('text/plain') || '';
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
    return () => clearTimeout(timer);
  }
  $effect(() => {
    // Read both dependencies before branching: an external value reset must
    // still subscribe to the asynchronously arriving field catalogue.
    const nextValue = value, nextFields = fields;
    if (editor && (nextValue !== text(editor) || nextFields !== renderedFields) && !composing) {
      untrack(() => { hide(); draw(nextValue); history = [{value:nextValue, caret:nextValue.length}]; historyAt = 0; });
    }
  });
</script>

<svelte:document onselectionchange={normalizeCaret} />

<div class={['field-pattern-control', {'field-pattern-invalid':invalid, 'field-pattern-disabled':disabled}]}>
  <div {id} class="field-pattern-editor" role="combobox" aria-label={label}
    aria-autocomplete="list" aria-haspopup="listbox" aria-expanded={choices.length > 0}
    aria-controls={listId} aria-activedescendant={choices.length ? `${listId}-${active}` : undefined}
    aria-invalid={invalid} aria-disabled={disabled}
    contenteditable={!disabled} tabindex={disabled ? -1 : 0} spellcheck="false" data-placeholder={placeholder}
    {@attach connect} onbeforeinput={beforeinput} oninput={input} onkeydown={keydown} onpaste={paste}
    ondrop={event => { event.preventDefault(); }}
    oncopy={event => copy(event)} oncut={event => copy(event, true)}
    onpointerdown={pointerdown} onblur={hide}
    oncompositionstart={() => { composing = true; hide(); }}
    oncompositionend={() => { composing = false; input(); }}></div>
  {#if choices.length}
    <div id={listId} class="field-pattern-suggestions" {@attach placeSuggestions} role="listbox" aria-label="Tag suggestions">
      {#each choices as field, index (field.key)}
        {@const parts = highlight(field.label)}
        <div id={`${listId}-${index}`} role="option" class="field-token-tone" style:--field-hue={fieldHue(field.key)} aria-label={field.label} aria-selected={active === index} tabindex="-1"
          onpointerenter={() => { active = index; }}
          onpointerdown={event => { event.preventDefault(); void choose(field); }}
          onkeydown={event => { if (event.key === 'Enter') void choose(field); }}>
          <span class="field-suggestion-label">{parts[0]}{#if parts[1]}<mark>{parts[1]}</mark>{/if}{parts[2]}</span><small>{field.key.startsWith('custom:') ? 'Custom Tag' : 'Tag'}</small>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .field-pattern-control { position:relative; min-width:0; background:var(--panel); border-radius:var(--input-radius,10px); color:var(--text); }
  .field-pattern-control:focus-within { outline:2px solid var(--accent); outline-offset:1px; }
  .field-pattern-invalid { outline:2px solid var(--error); }
  .field-pattern-disabled { opacity:.55; }
  .field-pattern-editor { height:36px; overflow-x:auto; overflow-y:hidden; padding:6px 10px; box-sizing:border-box; outline:none; white-space:pre; font:inherit; line-height:24px; cursor:text; caret-color:var(--accent); scrollbar-width:thin; }
  .field-pattern-editor:empty::before { content:attr(data-placeholder); color:var(--muted); pointer-events:none; }
  .field-pattern-editor::selection, .field-pattern-editor :global(::selection) { background:var(--accent); color:var(--on-accent,#fff); }
  .field-pattern-editor :global(.field-pattern-chip) { display:inline-flex; align-items:center; justify-content:center; height:24px; vertical-align:top; padding:0 8px; margin-inline:1px; border-radius:var(--r-compact,8px); color:var(--field-ink); background:color-mix(in srgb,var(--field-ink) 12%,var(--panel)); font-weight:500; line-height:1; white-space:nowrap; caret-color:transparent; user-select:all; -webkit-user-select:all; cursor:default; }
  .field-pattern-suggestions { position:absolute; z-index:20; max-height:224px; overflow:auto; overscroll-behavior:contain; padding:7px; border:1px solid var(--border); border-radius:var(--r-menu,16px); background:var(--panel); box-shadow:0 8px 24px #0003; }
  .field-pattern-suggestions [role=option] { display:flex; align-items:center; justify-content:space-between; gap:12px; min-height:32px; padding:4px 8px; box-sizing:border-box; border-radius:var(--r-menu-item,8px); cursor:pointer; }
  .field-pattern-suggestions [role=option]:hover, .field-pattern-suggestions [aria-selected=true] { background:var(--row-hover); }
  .field-suggestion-label { color:var(--field-ink); }
  .field-pattern-suggestions mark { color:inherit; background:color-mix(in srgb,var(--field-ink) 12%,transparent); border-radius:3px; font-weight:600; }
  .field-pattern-suggestions small { font-size:var(--text-xs); color:var(--muted); }
</style>
