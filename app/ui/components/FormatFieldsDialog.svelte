<script lang="ts">
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
    } catch (reason) { root.hidden = false; error = String(reason); result = null; }
    finally { working = false; }
  }
</script>

<ControllerDialog id="ribbon-format-dialog" labelledby="ribbon-format-title">
  <div class="modal ribbon-tool-body format-fields-dialog" {@attach connect}>
    <div class="ribbon-tool-header">
      <h2 id="ribbon-format-title">Format fields</h2>
      <Button variant="ghost" size="icon-sm" disabled={working} aria-label="Close dialog" onclick={close}><X size={17}/></Button>
    </div>
    <div class="format-mode-switch" role="group" aria-label="Format operation">
      <Button variant="ghost" aria-pressed={kind === 'build'} disabled={working} onclick={() => { kind = 'build'; }}>Build a field</Button>
      <Button variant="ghost" aria-pressed={kind === 'extract'} disabled={working} onclick={() => { kind = 'extract'; }}>Extract fields</Button>
    </div>
    <div class="ribbon-dialog-fields format-fields-content">
      <label class="ribbon-field">{kind === 'build' ? 'Put into' : 'Read from'}
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
          <FieldPatternInput id="ribbon-format-pattern" label={kind === 'build' ? 'Values' : 'Pattern'} value={pattern} {fields} disabled={working}
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
                <div class="format-result-value"><span>{item.label}</span><span class="format-result-text">{item.value || '(empty)'}</span></div>
              {/each}{/if}
            </div>
          {/each}
        {:else if !error}<p class="format-result-empty">Add tags or text to see the result.</p>{/if}
      </div>
      {#if result?.unmatched}<p class="ribbon-field-help" role="status">{result.unmatched} of {result.total} files don’t match and will stay unchanged.</p>{/if}
      {#if result?.locked.length}<p class="ribbon-field-help">Locked fields kept unchanged: {result.locked.join(', ')}.</p>{/if}
      {#if result?.invalid}<p class="ribbon-tool-error" role="alert">Some values aren’t supported by these files. Change the pattern before staging.</p>{/if}
      {#if error}<p class="ribbon-tool-error" role="alert">{error}</p>{/if}
    </div>
    <div class="ribbon-tool-footer">
      <Button variant="ghost" disabled={working} onclick={close}>Cancel</Button>
      <div><Button id="ribbon-format-write" variant="ghost" disabled={!canStage} onclick={() => submit('write')}>Write now</Button><Button id="ribbon-format-stage" disabled={!canStage} onclick={() => submit('stage')}>Add to Staged</Button></div>
    </div>
  </div>
</ControllerDialog>

<style>
  .format-fields-dialog { --controller-modal-width:560px; }
  .format-mode-switch { display:flex; gap:4px; padding:4px; background:var(--bg); border-radius:var(--input-radius,10px); }
  .format-mode-switch :global(button) { flex:1; border-radius:var(--r-compact,8px); }
  .format-mode-switch :global(button[aria-pressed=true]) { background:var(--panel); color:var(--text); }
  .format-fields-content { gap:var(--sp-4); }
  .format-result { display:flex; flex-direction:column; gap:var(--sp-2); min-height:64px; }
  .format-result-label { font-size:var(--text-xs); color:var(--muted); }
  .format-result-row { display:flex; flex-direction:column; gap:var(--sp-1); min-width:0; }
  .format-result-row + .format-result-row { margin-top:var(--sp-2); }
  .format-result-file { font-size:var(--text-xs); color:var(--muted); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
  .format-result-value { display:grid; grid-template-columns:minmax(76px,1fr) minmax(0,3fr); gap:var(--sp-3); }
  .format-result-value > span:first-child { color:var(--muted); }
  .format-result-text, .format-source-value { white-space:pre-wrap; overflow-wrap:anywhere; }
  .format-source-value { margin:0 0 var(--sp-1); }
  .format-result-empty { margin:0; color:var(--muted); }
</style>
