<script lang="ts">
  import { Info, X } from '@lucide/svelte';
  import ControllerDialog from './ControllerDialog.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { NativeSelect, NativeSelectOption } from '$lib/components/ui/native-select/index.js';
  import NativeCheckbox from './controls/NativeCheckbox.svelte';
  import { runRibbonAction } from '../js/ribbon-actions.js';
  import { numberWidth, setNumberWidth } from '../js/number-format.js';
  let {selected=0,busy=false}:{selected?:number;busy?:boolean}=$props();
  const tabs=['tracks','disc','vinyl'] as const;
  let tab=$state<'tracks'|'disc'|'vinyl'>('tracks');
  let start=$state<number|undefined>(1),disc=$state('1');
  let trackWidth=$state('1'),discWidth=$state('1');
  let trackTotal=$state(false),restartGroup=$state(false);
  let discTotal=$state(false),advanceGroup=$state(true),discResetAlbum=$state(true);
  let source=$state('track'),keepPosition=$state(true),vinylTrackTotal=$state(false),vinylDiscTotal=$state(false);
  let opened=$state(false),working=$state(false),pending=$state(false),revision=$state(0);
  let result=$state<any>(null);
  let root:HTMLElement,request=0,preserveOnOpen=false,submitting=false;
  let previewPromise:Promise<any>=Promise.resolve(null);
  const inputValid=$derived(tab==='tracks'?start!=null&&Number.isSafeInteger(start)&&start>=1:tab==='disc'?/^\d+$/.test(disc.trim())&&Number.isSafeInteger(Number(disc))&&Number(disc)>=1:true);
  const canRun=$derived(inputValid&&selected>0&&!busy&&!working&&(pending?(tab!=='vinyl'||!!result?.matched&&!result.invalid):!!result?.matched&&!result.invalid));
  function close(){if(!working)root.hidden=true;}
  function switchTab(event:KeyboardEvent){
    if(working||!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;
    event.preventDefault();tab=event.key==='Home'?'tracks':event.key==='End'?'vinyl':tabs[(tabs.indexOf(tab)+(event.key==='ArrowLeft'?2:1))%tabs.length];
    document.getElementById(`number-${tab}-tab`)?.focus();
  }
  function connect(node:HTMLElement){
    root=node.closest('.controller-dialog-root')!;
    const guard=(event:Event)=>{if(working)event.preventDefault();};
    const observer=new MutationObserver(()=>{
      if(root.hidden){opened=false;request++;void runRibbonAction('numbering-cancel').catch(()=>{});return;}
      opened=true;if(preserveOnOpen){preserveOnOpen=false;return;}
      tab='tracks';start=1;disc='1';trackTotal=false;restartGroup=false;
      discTotal=false;advanceGroup=true;discResetAlbum=true;source='track';keepPosition=true;vinylTrackTotal=false;vinylDiscTotal=false;result=null;
      trackWidth=String(numberWidth('track'));discWidth=String(numberWidth('disc'));
    });
    observer.observe(root,{attributes:true,attributeFilter:['hidden']});root.addEventListener('controller:before-close',guard);
    const invalidate=()=>{revision++;};
    const events=['tagrex:live-input','tagrex:manual-plan-changed','tagrex:ribbon-state'];
    for(const event of events)document.addEventListener(event,invalidate);
    return ()=>{request++;observer.disconnect();root.removeEventListener('controller:before-close',guard);for(const event of events)document.removeEventListener(event,invalidate);};
  }
  $effect(()=>{
    const options=tab==='tracks'?{kind:tab,start,trackTotal,restartGroup,resetAlbum:false}:
      tab==='disc'?{kind:tab,start:disc,discTotal,advanceGroup,resetAlbum:discResetAlbum}:
      {kind:tab,source,keepPosition,trackTotal:vinylTrackTotal,discTotal:vinylDiscTotal};
    void revision;if(!opened||working)return;
    const id=++request;pending=true;
    let settle:(value:any)=>void=()=>{};
    previewPromise=new Promise(resolve=>{settle=resolve;});
    const timer=setTimeout(()=>{
      void runRibbonAction('numbering-preview',options).then(next=>{if(opened&&id===request){result=next;pending=false;}settle(next);}).catch(()=>{if(id===request){result=null;pending=false;}settle(null);});
    },150);
    return ()=>{clearTimeout(timer);settle(null);};
  });
  async function submit(mode:'stage'|'write'){
    if(!canRun||submitting)return;
    submitting=true;
    const expected=request,fresh=await previewPromise;
    submitting=false;
    if(!opened||expected!==request||!fresh?.matched||fresh.invalid||!inputValid||busy)return;
    working=true;const token=fresh.token;if(mode==='write')root.hidden=true;
    try{
      const next=await runRibbonAction(mode==='write'?'numbering-write':'numbering-stage',{token});
      if(next?.dialog){if(tab==='tracks')setNumberWidth('track',trackWidth);else if(tab==='disc')setNumberWidth('disc',discWidth);root.hidden=true;}
    }catch(reason){preserveOnOpen=true;root.hidden=false;const {toast}=await import('../js/dom.js');toast(String(reason),true);}
    finally{working=false;}
  }
</script>

<ControllerDialog id="ribbon-number-dialog" labelledby="ribbon-number-title">
  <div class="modal ribbon-tool-body autonumber-dialog" {@attach connect}>
    <div class="ribbon-tool-header">
      <h2 id="ribbon-number-title">Numbering <span id="autonum-count" class="ribbon-selection-count"></span></h2>
      <Button variant="ghost" size="icon-sm" aria-label="Close dialog" disabled={working} onclick={close}><X size={17}/></Button>
    </div>
    <div class="number-mode-switch" role="tablist" aria-label="Numbering action">
      <button id="number-tracks-tab" type="button" role="tab" aria-selected={tab==='tracks'} aria-controls="number-tracks-panel" tabindex={tab==='tracks'?0:-1} disabled={working} onclick={()=>{tab='tracks';}} onkeydown={switchTab}>Autonumber</button>
      <button id="number-disc-tab" type="button" role="tab" aria-selected={tab==='disc'} aria-controls="number-disc-panel" tabindex={tab==='disc'?0:-1} disabled={working} onclick={()=>{tab='disc';}} onkeydown={switchTab}>Disc</button>
      <button id="number-vinyl-tab" type="button" role="tab" aria-selected={tab==='vinyl'} aria-controls="number-vinyl-panel" tabindex={tab==='vinyl'?0:-1} disabled={working} onclick={()=>{tab='vinyl';}} onkeydown={switchTab}>Vinyl</button>
    </div>
    <div class="ribbon-dialog-fields autonumber-content" data-number-tab={tab}>
      <div id="number-tracks-panel" class="autonumber-pane" role="tabpanel" aria-labelledby="number-tracks-tab" hidden={tab!=='tracks'}>
        <div class="autonumber-fields">
          <label class="ribbon-field" for="autonum-start">Start at<Input id="autonum-start" type="number" min="1" step="1" required bind:value={start} disabled={working}/></label>
          <label class="ribbon-field"><span>Track display</span><NativeSelect id="autonum-track-format" bind:value={trackWidth} disabled={working}><NativeSelectOption value="1">1</NativeSelectOption><NativeSelectOption value="2">01</NativeSelectOption><NativeSelectOption value="3">001</NativeSelectOption></NativeSelect></label>
        </div>
        <div class="autonumber-options">
          <label class="autonumber-option"><span><strong>Reset per group</strong><small>Restart track sequence</small></span><NativeCheckbox id="autonum-per-group" toggle checked={restartGroup} disabled={working} onchange={e=>{restartGroup=(e.currentTarget as HTMLInputElement).checked;}}/></label>
          <label class="autonumber-option"><span><strong>Track total</strong><small>Show 12/20</small></span><NativeCheckbox id="autonum-total" toggle checked={trackTotal} disabled={working} onchange={e=>{trackTotal=(e.currentTarget as HTMLInputElement).checked;}}/></label>
        </div>
      </div>
      <div id="number-disc-panel" class="autonumber-pane" role="tabpanel" aria-labelledby="number-disc-tab" hidden={tab!=='disc'}>
        <div class="autonumber-fields">
          <label class="ribbon-field" for="autonum-disc">Start at<Input id="autonum-disc" type="text" inputmode="numeric" bind:value={disc} disabled={working} placeholder="1" spellcheck="false"/></label>
          <label class="ribbon-field"><span>Disc display</span><NativeSelect id="autonum-disc-format" bind:value={discWidth} disabled={working}><NativeSelectOption value="1">1</NativeSelectOption><NativeSelectOption value="2">01</NativeSelectOption><NativeSelectOption value="3">001</NativeSelectOption></NativeSelect></label>
        </div>
        <div class="autonumber-options">
          <label class="autonumber-option"><span><strong>New disc per group</strong><small>Advance disc number</small></span><NativeCheckbox id="autonum-disc-per-group" toggle checked={advanceGroup} disabled={working} onchange={e=>{advanceGroup=(e.currentTarget as HTMLInputElement).checked;}}/></label>
          <label class="autonumber-option"><span><strong>Reset per album</strong><small>Restart disc sequence</small></span><NativeCheckbox id="autonum-disc-reset-album" toggle checked={discResetAlbum} disabled={working} onchange={e=>{discResetAlbum=(e.currentTarget as HTMLInputElement).checked;}}/></label>
          <label class="autonumber-option"><span><strong>Disc total</strong><small>Show 2/3</small></span><NativeCheckbox id="autonum-disc-total" toggle checked={discTotal} disabled={working} onchange={e=>{discTotal=(e.currentTarget as HTMLInputElement).checked;}}/></label>
        </div>
      </div>
      <div id="number-vinyl-panel" class="autonumber-pane" role="tabpanel" aria-labelledby="number-vinyl-tab" hidden={tab!=='vinyl'}>
        <div class="ribbon-field">
          <div class="vinyl-source-heading"><label for="vinyl-source">Which field contains A1/B2?</label><button type="button" class="vinyl-info" aria-label="About vinyl positions" aria-describedby="vinyl-position-help" data-tooltip="B3 becomes Disc 2, Track 3. Choose the field that currently holds B3. Plain numbers stay unchanged." data-tooltip-delay="500"><Info size={14}/></button></div>
          <NativeSelect id="vinyl-source" bind:value={source} disabled={working}><NativeSelectOption value="track">Track field</NativeSelectOption><NativeSelectOption value="disc">Disc field</NativeSelectOption></NativeSelect>
          <span id="vinyl-position-help" class="sr-only">B3 becomes Disc 2, Track 3. Choose the field that currently holds B3. Plain numbers stay unchanged.</span>
        </div>
        <div class="autonumber-options">
          <label class="autonumber-option"><span><strong>Keep original</strong><small>Save in Position</small></span><NativeCheckbox id="vinyl-keep-position" toggle checked={keepPosition} disabled={working} onchange={e=>{keepPosition=(e.currentTarget as HTMLInputElement).checked;}}/></label>
          <label class="autonumber-option"><span><strong>Track totals</strong><small>Count each side</small></span><NativeCheckbox id="vinyl-track-total" toggle checked={vinylTrackTotal} disabled={working} onchange={e=>{vinylTrackTotal=(e.currentTarget as HTMLInputElement).checked;}}/></label>
          <label class="autonumber-option"><span><strong>Disc totals</strong><small>Count each album</small></span><NativeCheckbox id="vinyl-disc-total" toggle checked={vinylDiscTotal} disabled={working} onchange={e=>{vinylDiscTotal=(e.currentTarget as HTMLInputElement).checked;}}/></label>
        </div>
      </div>
    </div>
    <span id="autonum-write-help" class="sr-only">Bypass staging and write numbers immediately.</span><span id="autonum-stage-help" class="sr-only">Add these numbers to Staged. Review them before using Apply to save your files.</span>
    <div class="ribbon-tool-footer"><Button variant="ghost" disabled={working} onclick={close}>Cancel</Button><div><Button id="autonum-write" variant="ghost" disabled={!canRun} data-tooltip="Bypass staging and write numbers immediately." data-tooltip-delay="1200" aria-describedby="autonum-write-help" onclick={()=>submit('write')}>Write</Button><Button id="autonum-run" disabled={!canRun} data-tooltip="Add these numbers to Staged. Review them before using Apply to save your files." data-tooltip-delay="1600" aria-describedby="autonum-stage-help" onclick={()=>submit('stage')}>Stage Edits</Button></div></div>
  </div>
</ControllerDialog>

<style>
  .autonumber-dialog.modal { height:min(400px,calc(100dvh - 48px)); gap:0; overflow:hidden; }
  .ribbon-tool-header { flex:none; margin-bottom:var(--sp-4); }
  .ribbon-tool-header h2 { min-width:0; }
  .autonumber-content { flex:1; border-radius:0 0 var(--r-inset) var(--r-inset); gap:var(--sp-4); min-height:0; overflow:auto; overscroll-behavior:contain; scrollbar-width:thin; }
  .autonumber-content[data-number-tab=tracks] { border-top-right-radius:var(--r-inset); }
  .autonumber-content[data-number-tab=disc] { border-top-left-radius:var(--r-inset); border-top-right-radius:var(--r-inset); }
  .autonumber-content[data-number-tab=vinyl] { border-top-left-radius:var(--r-inset); }
  .autonumber-pane { display:flex; flex-direction:column; gap:var(--sp-4); }
  .number-mode-switch { display:flex; flex:none; gap:4px; }
  .number-mode-switch button { position:relative; flex:1; min-height:38px; padding:8px 12px; border:0; border-radius:var(--r-inset); background:transparent; color:var(--muted); font:inherit; font-weight:500; cursor:pointer; }
  .number-mode-switch button[aria-selected=true] { border-radius:var(--r-inset) var(--r-inset) 0 0; background:var(--bg); color:var(--text); }
  .number-mode-switch button:focus-visible { outline:2px solid var(--accent); outline-offset:-3px; }
  #number-tracks-tab[aria-selected=true]::after, #number-disc-tab[aria-selected=true]::before, #number-disc-tab[aria-selected=true]::after, #number-vinyl-tab[aria-selected=true]::before { content:''; position:absolute; bottom:0; width:var(--r-inset); height:var(--r-inset); pointer-events:none; }
  #number-tracks-tab[aria-selected=true]::after, #number-disc-tab[aria-selected=true]::after { right:calc(-1 * var(--r-inset)); background:radial-gradient(circle at 100% 0,transparent calc(var(--r-inset) - .5px),var(--bg) var(--r-inset)); }
  #number-disc-tab[aria-selected=true]::before, #number-vinyl-tab[aria-selected=true]::before { left:calc(-1 * var(--r-inset)); background:radial-gradient(circle at 0 0,transparent calc(var(--r-inset) - .5px),var(--bg) var(--r-inset)); }
  .autonumber-fields { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:var(--sp-3); }
  .autonumber-options { display:grid; grid-template-columns:minmax(0,1fr) minmax(0,1fr); gap:var(--sp-3) var(--sp-4); }
  .autonumber-option { display:flex; align-items:center; justify-content:space-between; gap:var(--sp-2); min-height:44px; color:var(--label); font-size:var(--text-sm); cursor:pointer; }
  .autonumber-option > span { display:flex; flex-direction:column; gap:3px; min-width:0; }
  .autonumber-option strong { font-size:var(--text-sm); font-weight:500; }
  .autonumber-option small { color:var(--muted); font-size:var(--text-xs); line-height:1.4; font-weight:400; }
  .vinyl-source-heading { display:flex; align-items:center; gap:var(--sp-1); }
  .vinyl-info { display:inline-flex; align-items:center; justify-content:center; width:24px; height:24px; padding:0; border:0; border-radius:var(--r-compact); background:transparent; color:var(--muted); cursor:help; }
  .vinyl-info:hover { color:var(--text); }
  .vinyl-info:focus-visible { outline:2px solid var(--accent); outline-offset:1px; }
  .ribbon-tool-footer { flex:none; padding-top:var(--sp-4); }
  @media (max-width:440px) { .autonumber-fields { column-gap:var(--sp-2); } }
</style>
