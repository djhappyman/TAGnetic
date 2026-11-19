<script lang="ts">
  import { Eraser, FolderPen, TextCursorInput, ListOrdered, ImagePlus, ImageUp, Image as ImageIcon, SlidersHorizontal, FileMusic, X } from '@lucide/svelte';
  import ControllerDialog from './ControllerDialog.svelte';
  import FormatFieldsDialog from './FormatFieldsDialog.svelte';
  import AutonumberDialog from './AutonumberDialog.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import NativeCheckbox from './controls/NativeCheckbox.svelte';
  import { NativeSelect, NativeSelectOption } from '$lib/components/ui/native-select/index.js';
  import { runRibbonAction } from '../js/ribbon-actions.js';
  let selected = $state(0);
  let busy = $state(false);
  let working = $state(false);
  let error = $state('');
  let pattern = $state('folder.jpg');
  let firstImage = $state(false);
  let replaceArtwork = $state(false);
  let removeKind = $state('text');
  let blocks = $state<{kind:string;label:string}[]>([]);
  function attachState() {
    const update = (event: Event) => { const state = (event as CustomEvent).detail; selected = state.count; busy = state.busy; blocks = state.blocks; };
    document.addEventListener('tagrex:ribbon-state', update);
    return () => document.removeEventListener('tagrex:ribbon-state', update);
  }
  function attachDialog(node: HTMLElement) {
    const root = node.closest('.controller-dialog-root')!;
    const guard = (event: Event) => { if (working) event.preventDefault(); };
    root.addEventListener('controller:before-close', guard);
    return () => root.removeEventListener('controller:before-close', guard);
  }
  function close(name: string) { if (working) return; const root = document.getElementById('ribbon-' + name + '-dialog'); if (root) root.hidden = true; }
  function open(name: string) {
    if (working) return;
    error = '';
    pattern = 'folder.jpg'; firstImage = false; replaceArtwork = false;
    removeKind = 'text';
    const root = document.getElementById('ribbon-' + name + '-dialog');
    if (root) root.hidden = false;
  }
  async function perform(action: string, options: Record<string, unknown> = {}) {
    if (working) return;
    error = ''; working = true;
    const dialog = options.mode === 'write' ? document.getElementById('ribbon-' + ({number:'number', 'import-artwork':'import', 'remove-tags':'remove', 'format-values':'format'}[action]) + '-dialog') : null;
    if (dialog) dialog.hidden = true;
    try {
      const result = await runRibbonAction(action, options);
      if (result?.dialog) { const root = document.getElementById(result.dialog); if (root) root.hidden = true; }
    } catch (reason) { error = String(reason); if (dialog) dialog.hidden = false; }
    finally { working = false; }
  }
</script>

<span class="ribbon-divider" aria-hidden="true"></span>
<div class="ribbon-group" role="group" aria-label="Tag tools" {@attach attachState}>
  <Button id="ribbon-remove-tags" variant="ghost" size="icon-sm" class="icon ribbon-command" disabled={!selected || busy || working} title="Remove tags from selected files" aria-label="Remove tags" onclick={() => open('remove')}><Eraser class="ico" size={17} strokeWidth={1.7}/></Button>
  <Button id="ribbon-rename-folder" variant="ghost" size="icon-sm" class="icon ribbon-command" disabled={!selected || busy || working} title="Rename folders using tag values" aria-label="Rename folders" onclick={() => perform('rename')}><FolderPen class="ico" size={17} strokeWidth={1.7}/></Button>
  <Button id="ribbon-format-values" variant="ghost" size="icon-sm" class="icon ribbon-command" disabled={!selected || busy || working} title="Build or extract tag fields" aria-label="Format fields" onclick={() => open('format')}><TextCursorInput class="ico" size={17} strokeWidth={1.7}/></Button>
</div>
<span class="ribbon-divider" aria-hidden="true"></span>
<div class="ribbon-group" role="group" aria-label="Numbering">
  <Button id="ribbon-autonumber" variant="ghost" size="icon-sm" class="icon ribbon-command" disabled={!selected || busy || working} title="Autonumber tracks and discs" aria-label="Autonumber" onclick={() => open('number')}><ListOrdered class="ico" size={17} strokeWidth={1.7}/></Button>
</div>
<span class="ribbon-divider" aria-hidden="true"></span>
<div class="ribbon-group" role="group" aria-label="Artwork">
  <Button id="ribbon-import-artwork" variant="ghost" size="icon-sm" class="icon ribbon-command" disabled={!selected || busy || working} title="Import artwork from each track’s folder" aria-label="Import artwork" onclick={() => open('import')}><ImagePlus class="ico" size={17} strokeWidth={1.7}/></Button>
  <Button id="ribbon-export-artwork" variant="ghost" size="icon-sm" class="icon ribbon-command" disabled={!selected || busy || working} title="Export embedded front covers to image files" aria-label="Export artwork" onclick={() => perform('export-artwork')}><ImageUp class="ico" size={17} strokeWidth={1.7}/></Button>
  <Button id="ribbon-artwork-options" variant="ghost" size="icon-sm" class="icon ribbon-command" disabled={busy || working} title="Artwork import size and quality settings" aria-label="Artwork settings" onclick={() => perform('artwork-settings')}><span class="ribbon-artwork-settings-icon"><ImageIcon class="ico" size={17} strokeWidth={1.7}/><SlidersHorizontal class="ribbon-artwork-sliders" size={9} strokeWidth={2}/></span></Button>
</div>
<span class="ribbon-divider" aria-hidden="true"></span>
<div class="ribbon-group" role="group" aria-label="Lyrics">
  <Button id="ribbon-lyrics" variant="ghost" size="icon-sm" class="icon ribbon-command" disabled title="Import lyrics — coming later" aria-label="Import lyrics — coming later"><FileMusic class="ico" size={17} strokeWidth={1.7}/></Button>
</div>

<AutonumberDialog {selected} {busy} />
<ControllerDialog id="ribbon-import-dialog" labelledby="ribbon-import-title">
  <div class="modal ribbon-tool-body" {@attach attachDialog}>
    <div class="ribbon-tool-header">
      <div><h2 id="ribbon-import-title">Import artwork</h2><p class="ribbon-tool-description">Find a cover beside each selected track.</p></div>
      <Button variant="ghost" size="icon-sm" disabled={working} aria-label="Close dialog" onclick={() => close('import')}><X size={17}/></Button>
    </div>
    <div class="ribbon-dialog-fields">
      <label class="ribbon-field" for="artwork-import-pattern">Filename pattern<Input id="artwork-import-pattern" bind:value={pattern} disabled={firstImage || working} placeholder="folder.jpg" spellcheck="false"/></label>
      <p class="ribbon-field-help">Use a filename or tags, such as %album%.jpg.</p>
      <div class="ribbon-options-grid">
        <label class="ribbon-option"><NativeCheckbox id="artwork-import-first" checked={firstImage} onchange={event => { firstImage = (event.currentTarget as HTMLInputElement).checked; }} disabled={working}/><span><strong>First image in folder</strong><small>Use filename order instead of a pattern.</small></span></label>
        <label class="ribbon-option"><NativeCheckbox id="artwork-import-replace" checked={replaceArtwork} onchange={event => { replaceArtwork = (event.currentTarget as HTMLInputElement).checked; }} disabled={working}/><span><strong>Replace existing artwork</strong><small>Update front covers; keep other images.</small></span></label>
      </div>
    </div>
    {#if error}<p class="ribbon-tool-error" role="alert">{error}</p>{/if}
    <div class="ribbon-tool-footer"><span>Stage to review. Write saves now.</span><div><Button variant="ghost" disabled={working} onclick={() => close('import')}>Cancel</Button><Button id="ribbon-import-stage" variant="secondary" disabled={working || !selected || (!firstImage && !pattern.trim())} onclick={() => perform('import-artwork', {pattern, firstImage, replaceArtwork, mode:'stage'})}>Stage</Button><Button id="ribbon-import-write" disabled={working || !selected || (!firstImage && !pattern.trim())} onclick={() => perform('import-artwork', {pattern, firstImage, replaceArtwork, mode:'write'})}>Write</Button></div></div>
  </div>
</ControllerDialog>
<ControllerDialog id="ribbon-remove-dialog" labelledby="ribbon-remove-title">
  <div class="modal ribbon-tool-body" {@attach attachDialog}>
    <div class="ribbon-tool-header">
      <div><h2 id="ribbon-remove-title">Remove tags</h2><p class="ribbon-tool-description">Clear text fields or remove a specific tag block.</p></div>
      <Button variant="ghost" size="icon-sm" disabled={working} aria-label="Close dialog" onclick={() => close('remove')}><X size={17}/></Button>
    </div>
    <div class="ribbon-dialog-fields">
      <label class="ribbon-field">Remove<NativeSelect id="ribbon-remove-kind" bind:value={removeKind}><NativeSelectOption value="text">Text fields</NativeSelectOption>{#each blocks as block (block.kind)}<NativeSelectOption value={block.kind}>{block.label} tag block</NativeSelectOption>{/each}</NativeSelect></label>
      <p class="ribbon-field-help">{removeKind === 'text' ? 'Artwork and cue data are kept.' : 'The whole block is removed, including artwork and private data inside it.'}</p>
    </div>
    {#if error}<p class="ribbon-tool-error" role="alert">{error}</p>{/if}
    <div class="ribbon-tool-footer"><span>Stage to review. Write saves now.</span><div><Button variant="ghost" disabled={working} onclick={() => close('remove')}>Cancel</Button><Button id="ribbon-remove-stage" variant="secondary" disabled={working || !selected} onclick={() => perform('remove-tags', {kind:removeKind, mode:'stage'})}>Stage</Button><Button id="ribbon-remove-write" disabled={working || !selected} onclick={() => perform('remove-tags', {kind:removeKind, mode:'write'})}>Write</Button></div></div>
  </div>
</ControllerDialog>
<FormatFieldsDialog {selected} {busy} />
