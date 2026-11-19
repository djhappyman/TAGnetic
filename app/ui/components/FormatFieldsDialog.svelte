<script lang="ts">
  import { fieldHue } from '../js/field-pattern.js';
  import { toast } from '../js/dom.js';
  import { X } from '@lucide/svelte';
  import ControllerDialog from './ControllerDialog.svelte';
  import FieldPatternInput from './FieldPatternInput.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { NativeSelect, NativeSelectOption } from '$lib/components/ui/native-select/index.js';
  import { runRibbonAction } from '../js/ribbon-actions.js';

  let { selected = 0, busy = false }: { selected?: number; busy?: boolean } = $props();
  let kind = $state('build');
  let field = $state('albumartist');
  let source = $state('title');
  let buildPattern = $state('%artist%');
  let extractPattern = $state('%artist% - %title%');
  let fields = $state<{key:string;label:string}[]>([]);
  let opened = $state(false);
  let working = $state(false);
  let loading = $state(false);
  let error = $state('');
  let result = $state<any>(null);
  let contextRevision = $state(0);
  let request = 0;
  let root: HTMLElement;
  const pattern = $derived(kind === 'build' ? buildPattern : extractPattern);
  const canStage = $derived(!!result && !result.invalid && !loading && !working && !busy && selected > 0 && result.unmatched < result.total);

  function close() { if (!working) root.hidden = true; }
  function switchTab(event: KeyboardEvent, other: string) {
    if (working || !['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) return;
    event.preventDefault();
    kind = event.key === 'Home' ? 'build' : event.key === 'End' ? 'extract' : other;
    document.getElementById(kind === 'build' ? 'format-build-tab' : 'format-extract-tab')?.focus();
  }
  function connect(node: HTMLElement) {
    root = node.closest('.controller-dialog-root')!;
    const guard = (event: Event) => { if (working) event.preventDefault(); };
    const changed = () => {
      const next = !root.hidden;
      if (next === opened) return;
      opened = next; request++; result = null; error = '';
      if (next) {
        kind = 'build'; field = 'albumartist'; source = 'title';
        buildPattern = '%artist%'; extractPattern = '%artist% - %title%'; fields = [];
        const epoch = request;
        void runRibbonAction('field-format-open').then(data => { if (opened && request === epoch) fields = data.fields; }).catch(reason => { if (opened) error = String(reason); });
      } else void runRibbonAction('field-format-cancel');
    };
    const invalidate = () => { contextRevision++; };
    const observer = new MutationObserver(changed); observer.observe(root, {attributes:true, attributeFilter:['hidden']});
    root.addEventListener('controller:before-close', guard);
    document.addEventListener('tagrex:live-input', invalidate);
    document.addEventListener('tagrex:manual-plan-changed', invalidate);
    document.addEventListener('tagrex:ribbon-state', invalidate);
    return () => { request++; observer.disconnect(); root.removeEventListener('controller:before-close', guard); document.removeEventListener('tagrex:live-input', invalidate); document.removeEventListener('tagrex:manual-plan-changed', invalidate); document.removeEventListener('tagrex:ribbon-state', invalidate); };
  }
  $effect(() => {
    const options = {kind, field, source, pattern};
    void contextRevision;
    if (!opened || working || !fields.length) return;
    const id = ++request;
    result = null; error = ''; loading = !!pattern.trim();
    if (!pattern.trim()) return;
    const timer = setTimeout(() => {
      void runRibbonAction('field-format-preview', options).then(data => {
        if (opened && request === id) { result = data; loading = false; }
      }).catch(reason => { if (opened && request === id) { error = String(reason); loading = false; } });
    }, 250);
    return () => { clearTimeout(timer); };
  });
  async function submit(mode: 'stage' | 'write') {
    if (!canStage) return;
    working = true; error = '';
    const token = result.token;
    if (mode === 'write') root.hidden = true;
    try {
      await runRibbonAction(mode === 'write' ? 'field-format-write' : 'field-format-stage', {token});
      root.hidden = true;
    } catch (reason) { root.hidden = false; error = String(reason); result = null; toast(error, true); }
    finally { working = false; }
  }
</script>

<ControllerDialog id="ribbon-format-dialog" labelledby="ribbon-format-title">
  <div class="modal ribbon-tool-body format-fields-dialog" {@attach connect}>
    <div class="ribbon-tool-header">
      <h2 id="ribbon-format-title">Format fields</h2>
      <Button variant="ghost" size="icon-sm" disabled={working} aria-label="Close dialog" onclick={close}><X size={17}/></Button>
    </div>
    <div class="format-mode-switch" role="tablist" aria-label="Format operation">
      <button id="format-build-tab" type="button" role="tab" aria-selected={kind === 'build'} aria-controls="format-fields-panel" tabindex={kind === 'build' ? 0 : -1} disabled={working}
        onclick={() => { kind = 'build'; }} onkeydown={event => switchTab(event, 'extract')}>Build a field</button>
      <button id="format-extract-tab" type="button" role="tab" aria-selected={kind === 'extract'} aria-controls="format-fields-panel" tabindex={kind === 'extract' ? 0 : -1} disabled={working}
        onclick={() => { kind = 'extract'; }} onkeydown={event => switchTab(event, 'build')}>Extract fields</button>
    </div>
    <div id="format-fields-panel" class="ribbon-dialog-fields format-fields-content" role="tabpanel" aria-labelledby={kind === 'build' ? 'format-build-tab' : 'format-extract-tab'}>
      <label class="ribbon-field">{kind === 'build' ? 'Apply to' : 'Pull from'}
        {#if kind === 'build'}
          <NativeSelect id="ribbon-format-field" bind:value={field} disabled={working}>
            {#each fields as item (item.key)}<NativeSelectOption value={item.key}>{item.label}</NativeSelectOption>{/each}
          </NativeSelect>
        {:else}
          <NativeSelect id="ribbon-format-source" bind:value={source} disabled={working}>
            {#each fields as item (item.key)}<NativeSelectOption value={item.key}>{item.label}</NativeSelectOption>{/each}
          </NativeSelect>
        {/if}
      </label>
      <div class="ribbon-field">
        <label for="ribbon-format-pattern" onpointerdown={() => document.getElementById('ribbon-format-pattern')?.focus()}>{kind === 'build' ? 'Values' : 'Pattern'}</label>
        {#key kind}
          <FieldPatternInput id="ribbon-format-pattern" label={kind === 'build' ? 'Values' : 'Pattern'} value={pattern} {fields} disabled={working || !opened}
            invalid={!!error} onchange={value => { if (kind === 'build') buildPattern = value; else extractPattern = value; }}/>
        {/key}
      </div>
      <div class="format-result" aria-busy={loading} aria-live="polite">
        <span class="format-result-label">Preview{result?.total > 1 ? ` · First ${result.rows.length} of ${result.total}` : ''}</span>
        {#if loading}<p class="format-result-empty">Preparing preview…</p>
        {:else if result}
          {#each result.rows as row (row.path)}
            <div class="format-result-row">
              {#if result.total > 1}<span class="format-result-file" title={row.path}>{row.path.split(/[\\/]/).pop()}</span>{/if}
              {#if kind === 'extract'}<p class="format-source-value">{row.source || '(empty)'}</p>{/if}
              {#if row.unmatched}<p class="format-result-empty">Doesn’t match this pattern. Kept unchanged.</p>
              {:else}{#each row.values as item (item.field)}
                <div class="format-result-value"><span class="field-token-tone format-result-title" style:--field-hue={fieldHue(item.field)}>{item.label}</span><span class="format-result-text">{item.value || '(empty)'}</span></div>
              {/each}{/if}
            </div>
          {/each}
        {:else if !error}<p class="format-result-empty">Add tags or text to see the result.</p>{/if}
      </div>
      <div class="format-status" aria-live="polite">
        {#if result?.unmatched}<span>{result.unmatched} of {result.total} files don’t match.</span>{/if}
        {#if result?.locked.length}<span>Locked: {result.locked.join(', ')}.</span>{/if}
      </div>
    </div>
    <div class="ribbon-tool-footer">
      <Button variant="ghost" disabled={working} onclick={close}>Cancel</Button>
      <div><Button id="ribbon-format-write" variant="ghost" data-tooltip="Bypass staging and write changes immediately." data-tooltip-delay="1200" aria-description="Bypass staging and write changes immediately." disabled={!canStage} onclick={() => submit('write')}>Write</Button><Button id="ribbon-format-stage" data-tooltip="Add these changes to Staged. Review them before using Apply to save your files." data-tooltip-delay="1600" aria-description="Add these changes to Staged. Review them before using Apply to save your files." disabled={!canStage} onclick={() => submit('stage')}>Stage Edits</Button></div>
    </div>
  </div>
</ControllerDialog>

<style>
  .format-fields-dialog.modal { --controller-modal-width:560px; height:min(470px,calc(100dvh - 48px)); gap:0; overflow:hidden; }
  .ribbon-tool-header { flex:none; margin-bottom:var(--sp-4); }
  .format-mode-switch { display:flex; flex:none; gap:4px; }
  .format-mode-switch button { flex:1; min-height:38px; padding:8px 12px; border:0; border-radius:var(--r-inset) var(--r-inset) 0 0; background:transparent; color:var(--muted); font:inherit; font-weight:500; cursor:pointer; }
  .format-mode-switch button[aria-selected=true] { background:var(--bg); color:var(--text); }
  .format-mode-switch button:hover:not(:disabled) { color:var(--text); }
  .format-mode-switch button:focus-visible { outline:2px solid var(--accent); outline-offset:-3px; }
  .format-fields-content { flex:1; min-height:0; gap:var(--sp-3); border-radius:0 0 var(--r-inset) var(--r-inset); }
  .format-fields-content > .ribbon-field { flex:none; }
  .format-result { display:flex; flex:1; flex-direction:column; gap:var(--sp-2); min-height:0; overflow:auto; overscroll-behavior:contain; scrollbar-width:thin; }
  .format-result-label { font-size:var(--text-xs); color:var(--muted); }
  .format-result-row { display:flex; flex-direction:column; gap:var(--sp-1); min-width:0; }
  .format-result-row + .format-result-row { margin-top:var(--sp-2); }
  .format-result-file { font-size:var(--text-xs); color:var(--muted); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
  .format-result-value { display:grid; grid-template-columns:minmax(76px,1fr) minmax(0,3fr); gap:var(--sp-3); }
  .format-result-title { color:color-mix(in srgb,var(--field-ink) 72%,var(--muted)); }
  .format-result-text { color:var(--text); font-weight:400; }
  .format-result-text, .format-source-value { white-space:pre-wrap; overflow-wrap:anywhere; }
  .format-source-value { margin:0 0 var(--sp-1); color:var(--muted); }
  .format-result-empty { margin:0; color:var(--muted); font-weight:400; }
  .format-status { display:flex; flex:none; gap:var(--sp-2); min-height:16px; color:var(--muted); font-size:var(--text-xs); }
  .ribbon-tool-footer { flex:none; padding-top:var(--sp-4); }
  @media (max-height:480px) { .format-fields-dialog { padding:var(--sp-3); } .ribbon-tool-header { margin-bottom:var(--sp-2); } .ribbon-tool-footer { padding-top:var(--sp-2); } }
</style>
