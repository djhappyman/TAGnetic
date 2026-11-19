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
  let tab = $state<'tracks' | 'disc'>('tracks');
  let start = $state<number | undefined>(1);
  let disc = $state('');
  let trackWidth = $state('1');
  let discWidth = $state('1');
  let working = $state(false);
  let root: HTMLElement;
  let preserveOnOpen = false;
  const valid = $derived(tab === 'tracks'
    ? start != null && Number.isSafeInteger(start) && start >= 1
    : /^\d+$/.test(disc.trim()) && Number.isSafeInteger(Number(disc)) && Number(disc) >= 1);
  const canRun = $derived(valid && selected > 0 && !busy && !working);

  function close() { if (!working) root.hidden = true; }
  function switchTab(event: KeyboardEvent) {
    if (working || !['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) return;
    event.preventDefault();
    tab = event.key === 'Home' ? 'tracks' : event.key === 'End' ? 'disc' : tab === 'tracks' ? 'disc' : 'tracks';
    document.getElementById(tab === 'tracks' ? 'number-tracks-tab' : 'number-disc-tab')?.focus();
  }
  function connect(node: HTMLElement) {
    root = node.closest('.controller-dialog-root')!;
    const guard = (event: Event) => { if (working) event.preventDefault(); };
    const observer = new MutationObserver(() => {
      if (root.hidden) return;
      if (preserveOnOpen) { preserveOnOpen = false; return; }
      tab = 'tracks'; start = 1; disc = '';
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
      const result = await runRibbonAction(tab === 'disc' ? 'number-disc' : 'number', {mode,disc});
      if (result?.dialog) {
        if (tab === 'disc') setNumberWidth('disc', discWidth);
        else setNumberWidth('track', trackWidth);
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
      <h2 id="ribbon-number-title">Numbering <span id="autonum-count" class="ribbon-selection-count"></span></h2>
      <Button variant="ghost" size="icon-sm" aria-label="Close dialog" disabled={working} onclick={close}><X size={17}/></Button>
    </div>
    <div class="number-mode-switch" role="tablist" aria-label="Numbering action">
      <button id="number-tracks-tab" type="button" role="tab" aria-selected={tab === 'tracks'} aria-controls="number-tracks-panel" tabindex={tab === 'tracks' ? 0 : -1} disabled={working} onclick={() => { tab = 'tracks'; }} onkeydown={switchTab}>Autonumber</button>
      <button id="number-disc-tab" type="button" role="tab" aria-selected={tab === 'disc'} aria-controls="number-disc-panel" tabindex={tab === 'disc' ? 0 : -1} disabled={working} onclick={() => { tab = 'disc'; }} onkeydown={switchTab}>Disc</button>
    </div>
    <div class="ribbon-dialog-fields autonumber-content" data-number-tab={tab}>
      <div id="number-tracks-panel" class="autonumber-pane" role="tabpanel" aria-labelledby="number-tracks-tab" hidden={tab !== 'tracks'}>
        <div class="autonumber-fields">
          <label class="ribbon-field" for="autonum-start">Start at<Input id="autonum-start" type="number" min="1" step="1" required bind:value={start} disabled={working}/></label>
          <label class="ribbon-field"><span>Track display</span>
            <NativeSelect id="autonum-track-format" bind:value={trackWidth} disabled={working} aria-describedby="autonum-format-help">
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
      <div id="number-disc-panel" class="autonumber-pane" role="tabpanel" aria-labelledby="number-disc-tab" hidden={tab !== 'disc'}>
        <div class="autonumber-fields">
          <label class="ribbon-field" for="autonum-disc">Disc number<Input id="autonum-disc" type="text" inputmode="numeric" bind:value={disc} disabled={working} placeholder="1" spellcheck="false"/></label>
          <label class="ribbon-field"><span>Disc display</span>
            <NativeSelect id="autonum-disc-format" bind:value={discWidth} disabled={working} aria-describedby="autonum-format-help">
              <NativeSelectOption value="1">1</NativeSelectOption><NativeSelectOption value="2">01</NativeSelectOption><NativeSelectOption value="3">001</NativeSelectOption>
            </NativeSelect>
          </label>
        </div>
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
  .autonumber-dialog.modal { height:min(420px,calc(100dvh - 48px)); gap:0; overflow:hidden; }
  .ribbon-tool-header { flex:none; margin-bottom:var(--sp-4); }
  .ribbon-tool-header h2 { min-width:0; }
  .autonumber-content { flex:1; border-radius:0 0 var(--r-inset) var(--r-inset); gap:var(--sp-4); min-height:0; overflow:auto; overscroll-behavior:contain; scrollbar-width:thin; }
  .autonumber-content[data-number-tab=tracks] { border-top-right-radius:var(--r-inset); }
  .autonumber-content[data-number-tab=disc] { border-top-left-radius:var(--r-inset); }
  .autonumber-pane { display:flex; flex-direction:column; gap:var(--sp-4); }
  .number-mode-switch { display:flex; flex:none; gap:4px; }
  .number-mode-switch button { position:relative; flex:1; min-height:38px; padding:8px 12px; border:0; border-radius:var(--r-inset); background:transparent; color:var(--muted); font:inherit; font-weight:500; cursor:pointer; }
  .number-mode-switch button[aria-selected=true] { border-radius:var(--r-inset) var(--r-inset) 0 0; background:var(--bg); color:var(--text); }
  .number-mode-switch button:focus-visible { outline:2px solid var(--accent); outline-offset:-3px; }
  #number-tracks-tab[aria-selected=true]::after, #number-disc-tab[aria-selected=true]::before { content:''; position:absolute; bottom:0; width:var(--r-inset); height:var(--r-inset); pointer-events:none; }
  #number-tracks-tab[aria-selected=true]::after { right:calc(-1 * var(--r-inset)); background:radial-gradient(circle at 100% 0,transparent calc(var(--r-inset) - .5px),var(--bg) var(--r-inset)); }
  #number-disc-tab[aria-selected=true]::before { left:calc(-1 * var(--r-inset)); background:radial-gradient(circle at 0 0,transparent calc(var(--r-inset) - .5px),var(--bg) var(--r-inset)); }
  .autonumber-fields { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:var(--sp-3); }
  .autonumber-options { display:flex; flex-direction:column; gap:var(--sp-1); }
  .autonumber-option { display:flex; align-items:center; justify-content:space-between; gap:var(--sp-3); min-height:32px; color:var(--label); font-size:var(--text-sm); cursor:pointer; }
  .autonumber-option > span { min-width:0; }
  .ribbon-tool-footer { flex:none; padding-top:var(--sp-4); }
  @media (max-width:440px) { .autonumber-fields { column-gap:var(--sp-2); } }
</style>
