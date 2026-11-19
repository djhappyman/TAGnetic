<script lang="ts">
  import { X } from '@lucide/svelte';
  import ControllerDialog from './ControllerDialog.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { NativeSelect, NativeSelectOption } from '$lib/components/ui/native-select/index.js';
  import NativeCheckbox from './controls/NativeCheckbox.svelte';
  import { runRibbonAction } from '../js/ribbon-actions.js';
  import { numberWidth, setNumberWidth } from '../js/number-format.js';

  let { selected = 0, busy = false }: {selected?: number; busy?: boolean} = $props();
  let start = $state<number | undefined>(1);
  let disc = $state('');
  let trackWidth = $state('1');
  let discWidth = $state('1');
  let working = $state(false);
  let root: HTMLElement;
  let preserveOnOpen = false;
  const valid = $derived(start != null && Number.isSafeInteger(start) && start >= 1 &&
    (!disc.trim() || (/^\d+$/.test(disc.trim()) && Number.isSafeInteger(Number(disc)))));
  const canRun = $derived(valid && selected > 0 && !busy && !working);

  function close() { if (!working) root.hidden = true; }
  function connect(node: HTMLElement) {
    root = node.closest('.controller-dialog-root')!;
    const guard = (event: Event) => { if (working) event.preventDefault(); };
    const observer = new MutationObserver(() => {
      if (root.hidden) return;
      if (preserveOnOpen) { preserveOnOpen = false; return; }
      start = 1; disc = '';
      trackWidth = String(numberWidth('track')); discWidth = String(numberWidth('disc'));
      for (const [id, checked] of [['autonum-total',false],['autonum-per-group',false],['autonum-keep-sides',true]] as const) {
        (document.getElementById(id) as HTMLInputElement).checked = checked;
      }
    });
    observer.observe(root, {attributes:true, attributeFilter:['hidden']});
    root.addEventListener('controller:before-close', guard);
    return () => { observer.disconnect(); root.removeEventListener('controller:before-close', guard); };
  }
  async function submit(mode: 'stage' | 'write') {
    if (!canRun) return;
    working = true;
    if (mode === 'write') root.hidden = true;
    try {
      const result = await runRibbonAction('number', {mode});
      if (result?.dialog) {
        setNumberWidth('track', trackWidth); setNumberWidth('disc', discWidth);
        root.hidden = true;
      }
    } catch (reason) {
      preserveOnOpen = true;
      root.hidden = false;
      // Legacy controllers mount after the Svelte shell. Load their notification
      // helper only on an actual failure, never during component startup.
      const { toast } = await import('../js/dom.js'); toast(String(reason), true);
    } finally { working = false; }
  }
</script>

<ControllerDialog id="ribbon-number-dialog" labelledby="ribbon-number-title">
  <div class="modal ribbon-tool-body autonumber-dialog" {@attach connect}>
    <div class="ribbon-tool-header">
      <h2 id="ribbon-number-title">Autonumber <span id="autonum-count" class="ribbon-selection-count"></span></h2>
      <Button variant="ghost" size="icon-sm" aria-label="Close dialog" disabled={working} onclick={close}><X size={17}/></Button>
    </div>
    <div class="ribbon-dialog-fields autonumber-content">
      <div class="autonumber-fields">
        <label class="ribbon-field" for="autonum-start">Start at<Input id="autonum-start" type="number" min="1" step="1" required bind:value={start} disabled={working}/></label>
        <label class="ribbon-field"><span>Track display</span>
          <NativeSelect id="autonum-track-format" bind:value={trackWidth} disabled={working} aria-describedby="autonum-format-help">
            <NativeSelectOption value="1">1</NativeSelectOption><NativeSelectOption value="2">01</NativeSelectOption><NativeSelectOption value="3">001</NativeSelectOption>
          </NativeSelect>
        </label>
        <label class="ribbon-field" for="autonum-disc">Disc number<Input id="autonum-disc" type="text" inputmode="numeric" bind:value={disc} disabled={working} placeholder="Keep existing" spellcheck="false"/></label>
        <label class="ribbon-field"><span>Disc display</span>
          <NativeSelect id="autonum-disc-format" bind:value={discWidth} disabled={working} aria-describedby="autonum-format-help">
            <NativeSelectOption value="1">1</NativeSelectOption><NativeSelectOption value="2">01</NativeSelectOption><NativeSelectOption value="3">001</NativeSelectOption>
          </NativeSelect>
        </label>
      </div>
      <div class="autonumber-options">
        <label class="autonumber-option"><span>Include track total</span><NativeCheckbox id="autonum-total" toggle disabled={working}/></label>
        <label class="autonumber-option"><span>Restart for each group</span><NativeCheckbox id="autonum-per-group" toggle disabled={working}/></label>
        <label class="autonumber-option"><span>Keep A1 / B2 positions</span><NativeCheckbox id="autonum-keep-sides" checked toggle disabled={working}/></label>
      </div>
    </div>
    <span id="autonum-format-help" class="sr-only">Display formatting adds leading zeroes in the table. Saved number tags stay numeric.</span>
    <span id="autonum-write-help" class="sr-only">Bypass staging and write numbers immediately.</span>
    <span id="autonum-stage-help" class="sr-only">Add these numbers to Staged. Review them before using Apply to save your files.</span>
    <div class="ribbon-tool-footer">
      <Button variant="ghost" disabled={working} onclick={close}>Cancel</Button>
      <div>
        <Button id="autonum-write" variant="ghost" disabled={!canRun} data-tooltip="Bypass staging and write numbers immediately." data-tooltip-delay="1200" aria-describedby="autonum-write-help" onclick={() => submit('write')}>Write</Button>
        <Button id="autonum-run" disabled={!canRun} data-tooltip="Add these numbers to Staged. Review them before using Apply to save your files." data-tooltip-delay="1600" aria-describedby="autonum-stage-help" onclick={() => submit('stage')}>Stage Edits</Button>
      </div>
    </div>
  </div>
</ControllerDialog>

<style>
  .autonumber-dialog.modal { gap:var(--sp-4); overflow:hidden; }
  .ribbon-tool-header { flex:none; }
  .ribbon-tool-header h2 { min-width:0; }
  .autonumber-content { gap:var(--sp-4); min-height:0; overflow:auto; overscroll-behavior:contain; scrollbar-width:thin; }
  .autonumber-fields { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:var(--sp-3); }
  .autonumber-options { display:flex; flex-direction:column; gap:var(--sp-1); }
  .autonumber-option { display:flex; align-items:center; justify-content:space-between; gap:var(--sp-3); min-height:32px; color:var(--label); font-size:var(--text-sm); cursor:pointer; }
  .autonumber-option > span { min-width:0; }
  .ribbon-tool-footer { flex:none; }
  @media (max-width:440px) { .autonumber-fields { column-gap:var(--sp-2); } }
</style>
