<script lang="ts">
  import { onMount } from 'svelte';
  import { Bookmark, Check, Copy, FileOutput, FolderOpen, Info, Play, Settings2, Square } from '@lucide/svelte';
  import { Checkbox } from '@ui-sdk/svelte/checkbox';
  import { Input } from '@ui-sdk/svelte/input';
  import { createMenuController, Menu, MenuContent, MenuItem, MenuSeparator, MenuTrigger } from '@ui-sdk/svelte/menu';
  import { createPopoverController, Popover, PopoverContent, PopoverTrigger } from '@ui-sdk/svelte/popover';
  import { Progress as ProgressBar, ProgressIndicator, ProgressTrack } from '@ui-sdk/svelte/progress';
  import { buttonVariants } from '$lib/components/ui/button/button.svelte';
  import Button from './AudioSplitButton.svelte';
  import FieldPatternInput from './FieldPatternInput.svelte';
  import { fieldLabel } from '../js/fields.js';
  import { catalogue } from '../js/placeholders.js';
  import { maskPresets } from '../js/state.js';
  import { defaultSplitFilePattern, directory, leaf, type Format, type Progress, type Review, type SaveMode, type SplitMethod } from '../js/audio-split';

  let {
    split, canCopy, copying, treated, blocked, sampleRate, sourcePaths, saveMode = $bindable(), destination = $bindable(),
    backupDirectory = $bindable(), createFolder = $bindable(), folderPattern = $bindable(),
    format = $bindable(), splitMethod = $bindable(), bitrate = $bindable(), namePattern = $bindable(),
    review, progress, previewing, finished, backups, restored, exportWarnings, outputAudition, effects = [], onbrowse, onchange, onaudition,
  }: {
    split: boolean; canCopy: boolean; copying: boolean; treated: boolean; blocked: boolean; sampleRate: number; sourcePaths: string[];
    saveMode: SaveMode; destination: string; backupDirectory: string; createFolder: boolean; folderPattern: string;
    format: Format; bitrate: number; namePattern: string; splitMethod: SplitMethod;
    review: Review | null; progress: Progress | null; previewing: boolean; finished: string[]; backups: string[];
    restored: string[]; exportWarnings: string[]; outputAudition: number | null;
    effects?: string[];
    onbrowse: (backup?: boolean) => void; onchange: (delayed?: boolean) => void; onaudition: (index: number) => void;
  } = $props();

  const saveChoices = [
    { value: 'replace', label: 'Replace original', icon: FileOutput },
    { value: 'beside', label: 'Save alongside original', icon: Copy },
  ] as const;
  const commonFields = ['track', 'title', 'artist', 'album', 'albumartist', 'disc', 'year', 'genre', 'comment'];
  const patternMenu = createMenuController();
  const methodInfo = createPopoverController();
  let fields = $state.raw(commonFields.map(key => ({ key, label: fieldLabel(key) })));
  let settingsOpen = $state(false);
  const sourceFolders = $derived([...new Set(sourcePaths.map(directory))]);
  const sourceFolder = $derived(sourceFolders.length === 1 ? sourceFolders[0] : 'Each original’s folder');
  const fieldButton = buttonVariants({ variant: 'ghost', size: 'icon-sm', class: 'field-action' });
  const progressMax = $derived(progress?.framesTotal && progress.framesDone !== undefined ? progress.framesTotal : progress?.total || 1);
  const progressValue = $derived(progress?.framesTotal && progress.framesDone !== undefined ? progress.framesDone : progress?.total ? progress.completed : null);
  const warnings = $derived((review?.warnings || []).filter(warning =>
    warning !== 'MP3 players that ignore gapless timing tags may play a little extra audio at a cut.'
      && warning !== 'MP3 recordings will be encoded again.'
      && warning !== 'The selected audio treatments will be applied when encoding.'));

  onMount(() => {
    let mounted = true;
    void catalogue().then((entries: { name: string; token: string }[]) => {
      if (mounted) fields = entries.filter(entry => /^%[^%]+%$/.test(entry.token))
        .map(entry => ({ key: entry.name, label: fieldLabel(entry.name) }));
    }).catch(() => {});
    return () => { mounted = false; };
  });

  function toggleSaveMode(value: 'replace' | 'beside') {
    saveMode = saveMode === value ? 'folder' : value;
    onchange();
  }

  function choosePattern(target: 'file' | 'folder', value: string, delayed = false) {
    if (target === 'file') namePattern = value;
    else folderPattern = value;
    onchange(delayed);
  }

  function popupKeys(event: KeyboardEvent, popup: typeof methodInfo | typeof patternMenu) {
    if (event.key !== 'Escape') return;
    // Let Escape dismiss this popup without also closing the surrounding dialog.
    event.preventDefault();
    event.stopPropagation();
    popup.close();
    popup.focus();
  }
</script>

{#snippet patternEditor(target: 'file' | 'folder', label: string, value: string, placeholder: string)}
  <div class="pattern">
    <span class="field-label">{label}</span>
    <div class="action-field pattern-field">
      <FieldPatternInput id={'split-output-' + target + '-pattern'} {label} {value} {placeholder} disabled={blocked}
        {fields} onchange={value => choosePattern(target, value, true)}/>
      <MenuTrigger controller={patternMenu} payload={target} class={fieldButton} disabled={blocked}
        aria-label={label + ' presets'} title={label + ' presets'}><Bookmark size={15}/></MenuTrigger>
    </div>
  </div>
{/snippet}

{#if !finished.length}
  <section class="save-settings" aria-labelledby="studio-save-heading">
    <h3 id="studio-save-heading" tabindex="-1">Review &amp; save</h3>

    <fieldset disabled={blocked}>
      <legend><label for="split-save-address">Save location</label></legend>
      <div class={['action-field', 'address', { inactive: saveMode !== 'folder' }]}>
        <Input id="split-save-address" class="address-input" value={saveMode === 'folder' ? destination : sourceFolder}
          placeholder="Type a folder path" disabled={saveMode !== 'folder' || blocked} spellcheck="false"
          title={saveMode === 'folder' ? destination : sourceFolder}
          onValueChange={value => { destination = value; onchange(true); }}/>
        <Button variant="ghost" size="icon-sm" class="field-action" title="Choose output folder" aria-label="Choose output folder"
          disabled={saveMode !== 'folder' || blocked} onclick={() => onbrowse()}><FolderOpen size={15}/></Button>
        {#each saveChoices as choice (choice.value)}
          <Button variant="ghost" size="icon-sm" class="field-action location-mode" aria-pressed={saveMode === choice.value}
            title={choice.label} aria-label={choice.label} disabled={blocked} onclick={() => toggleSaveMode(choice.value)}><choice.icon size={17}/></Button>
        {/each}
      </div>
      {#if saveMode === 'replace'}
        <div class="recovery">
          <span title={backupDirectory}>Originals go to {backupDirectory ? leaf(backupDirectory) : 'TAGnetic Recovery'}.</span>
          <Button variant="ghost" size="sm" aria-label="Choose recovery folder" onclick={() => onbrowse(true)}>Change</Button>
          {#if backupDirectory}<Button variant="ghost" size="sm" onclick={() => { backupDirectory = ''; onchange(); }}>Use default</Button>{/if}
        </div>
      {/if}
    </fieldset>

    <fieldset disabled={blocked}>
      <legend class="sr-only">Audio output</legend>
      <div class="format-row">
        <div class="format-setting format">
          <label class="field-label" for="split-output-format">Output</label>
          <div class="action-field">
            <select id="split-output-format" bind:value={format} onchange={() => onchange()}>
              <option value="mp3">MP3</option>
              <option value="flac">FLAC (lossless)</option>
            </select>
            <Button variant="ghost" size="icon-sm" class="field-action" title={format.toUpperCase() + ' settings'} aria-label="Output settings"
              aria-expanded={settingsOpen} aria-controls={settingsOpen ? 'split-output-settings' : undefined}
              disabled={blocked} onclick={() => { settingsOpen = !settingsOpen; }}><Settings2 size={16}/></Button>
          </div>
        </div>
        {#if settingsOpen}
          <div id="split-output-settings" class="format-options">
            {#if split}
              <div class="format-setting method">
                {#if canCopy}<label class="field-label" for="split-output-method">Split method</label>
                {:else}<span class="field-label">Split method</span>{/if}
                <div class="action-field">
                  {#if canCopy}
                    <select id="split-output-method" bind:value={splitMethod} onchange={() => onchange()}>
                      <option value="copy">Fast split</option>
                      <option value="exact">Exact cuts</option>
                    </select>
                  {:else}<span class="fixed-method">Exact cuts</span>{/if}
                  <Popover controller={methodInfo}>
                    <PopoverTrigger class={fieldButton} title="About split methods" aria-label="About split methods" disabled={blocked}><Info size={15}/></PopoverTrigger>
                    <PopoverContent class="split-method-info" align="start" sideOffset={4} collisionPadding={12} aria-label="Split methods"
                      onkeydowncapture={event => popupKeys(event, methodInfo)}>
                      <p><strong>Fast split</strong> copies the audio as it is. It’s quick and keeps the same quality. Cuts may move slightly to the nearest spot the file can be cut. Some older MP3 players may play a little extra audio at a cut.</p>
                      <p><strong>Exact cuts</strong> split right where you put the markers. This takes longer because the app makes new audio files. Creating a new MP3 can reduce quality.</p>
                      {#if treated}<p>Audio treatments require encoding new files, so exact cuts are used.</p>
                      {:else if !canCopy}<p>Changing the file type requires making new audio files, so exact cuts are used.</p>{/if}
                    </PopoverContent>
                  </Popover>
                </div>
              </div>
            {/if}
            {#if format === 'mp3' && !copying}
              <div class="format-setting quality">
                <label class="field-label" for="split-output-bitrate">MP3 quality</label>
                <div class="action-field">
                  <select id="split-output-bitrate" bind:value={bitrate} onchange={() => onchange()}>
                    {#if !split || sampleRate >= 32000}
                      <option value={320}>320 kbps</option>
                      <option value={256}>256 kbps</option>
                      <option value={192}>192 kbps</option>
                    {/if}
                    <option value={160}>160 kbps</option>
                    <option value={128}>128 kbps</option>
                    <option value={96}>96 kbps</option>
                  </select>
                </div>
              </div>
            {:else if !split}
              <p class="format-mode">FLAC keeps the full audio quality.</p>
            {/if}
          </div>
        {:else if split}<span class="format-mode">{treated ? 'Exact cuts · Audio treated' : copying ? 'Fast split' : 'Exact cuts'}</span>{/if}
      </div>
    </fieldset>

    {@render patternEditor('file', 'Filename pattern', namePattern, split ? 'Track number – Title' : 'Keep source filenames')}
    <Checkbox checked={createFolder} class="new-folder" disabled={blocked}
      onCheckedChange={next => { createFolder = next === true; onchange(); }}>
      <span class="new-folder-check" aria-hidden="true">{#if createFolder}<Check size={12}/>{/if}</span>
      Create a new folder
    </Checkbox>
    {#if createFolder}
      {@render patternEditor('folder', 'Folder name', folderPattern, 'Type a name or a tag such as %album%')}
    {/if}

    <Menu controller={patternMenu}>
      <MenuContent class="app-select-menu split-pattern-presets" align="end" sideOffset={4}
        onkeydowncapture={event => popupKeys(event, patternMenu)}>
        {#snippet children({ payload })}
          {@const target = payload === 'folder' ? 'folder' : 'file'}
          {#if target === 'folder'}
            <MenuItem onSelect={() => choosePattern(target, '%album%')}>Album</MenuItem>
            <MenuItem onSelect={() => choosePattern(target, '%albumartist% - %album%')}>Album Artist – Album</MenuItem>
          {:else}
            <MenuItem onSelect={() => choosePattern(target, split ? defaultSplitFilePattern : '')}>{split ? 'Track number – Title (default)' : 'Source filename (default)'}</MenuItem>
            <MenuItem onSelect={() => choosePattern(target, '%artist% - %title%')}>Artist – Title</MenuItem>
          {/if}
          {#if maskPresets.length}
            <MenuSeparator />
            {#each maskPresets as preset (preset.name)}
              <MenuItem onSelect={() => choosePattern(target, preset.mask)}>{preset.name}</MenuItem>
            {/each}
          {/if}
        {/snippet}
      </MenuContent>
    </Menu>
  </section>
{/if}

{#if review && !finished.length}
  <section class="review output-review" aria-label="Output review" aria-busy={previewing}>
    <h3 id="audio-split-review-heading" tabindex="-1">{review.outputs.length} {split ? (review.outputs.length === 1 ? 'track' : 'tracks') : (review.outputs.length === 1 ? 'file' : 'files')} to save</h3>
    {#if effects.length}<p class="review-note"><strong>Effects on export</strong> · {effects.join(', ')}</p>{/if}
    <p id="audio-split-output-audition-note" class="review-note">Audition previews up to four seconds from the start of each {split ? 'track' : 'file'}{treated ? ' with your effects' : ''}.</p>
    <ol aria-label="Output filenames">
      {#each review.outputs as output, index (output.path)}
        {@const filename = leaf(output.path)}
        {@const listening = outputAudition === index}
        <li class="output-row">
          <span title={output.path}>{filename}</span>
          <Button variant="ghost" size="icon-sm" title={listening ? 'Stop audition' : 'Preview the first four seconds'}
            aria-label={(listening ? 'Stop audition of ' : 'Audition ') + filename} disabled={blocked || previewing}
            aria-describedby="audio-split-output-audition-note"
            onclick={() => onaudition(index)}>
            {#if listening}<Square size={13}/>{:else}<Play size={14}/>{/if}
          </Button>
        </li>
      {/each}
    </ol>
  </section>
  {#if !previewing}
    {#each [...new Set(warnings)] as warning (warning)}<p class="warning">{warning}</p>{/each}
  {/if}
{/if}

{#if progress}
  <div class="export-progress">
    <p role="status">{progress.phase === 'copying' ? 'Copying audio frames' : progress.phase} · {progress.completed} of {progress.total} files</p>
    <ProgressBar value={progressValue} max={progressMax} aria-label="Audio export progress">
      <ProgressTrack class="split-output-progress-track"><ProgressIndicator class="split-output-progress-fill"/></ProgressTrack>
    </ProgressBar>
  </div>
{/if}

{#if finished.length}
  <section class="review" aria-label="Completed files">
    {#if restored.length}
      <h3 id="audio-split-done-heading" tabindex="-1">Restored {restored.length} {restored.length === 1 ? 'original' : 'originals'}</h3>
      {@render completedWarnings()}
      <ul>{#each restored as file (file)}<li class="output-path">{file}</li>{/each}</ul>
    {:else}
      <h3 id="audio-split-done-heading" tabindex="-1">Saved {finished.length} {finished.length === 1 ? 'file' : 'files'}</h3>
      {@render completedWarnings()}
      <ul>{#each finished as file (file)}<li class="output-path">{file}</li>{/each}</ul>
      {#if backups.length}
        <div class="backups">
          <h4>Originals kept in recovery</h4>
          <ul>{#each backups as file (file)}<li class="output-path">{file}</li>{/each}</ul>
        </div>
      {/if}
    {/if}
  </section>
{/if}

{#snippet completedWarnings()}
  {#each [...new Set(exportWarnings)] as warning (warning)}<p class="warning">{warning}</p>{/each}
{/snippet}

<style>
  .save-settings { display:grid; gap:var(--sp-3); }
  fieldset { min-width:0; border:0; padding:0; margin:0; }
  legend, .field-label { color:var(--muted); font-size:var(--text-xs); }
  legend, h3 { margin:0 0 var(--sp-1); }
  h3 { color:var(--text); font-weight:600; font-size:var(--text-sm); }
  .save-settings > h3 { margin:0; font-size:var(--text-lg); }
  .action-field { display:flex; align-items:center; min-width:0; min-height:var(--ctl-h); border-radius:var(--input-radius); background:var(--panel); }
  .action-field:has(:is(:global(.address-input), select, :global(.app-select-trigger), :global(.field-pattern-editor)):focus-visible) { outline:2px solid var(--accent); }
  .action-field :global(.field-action) { flex:none; margin-inline-end:2px; }
  .action-field :global(.field-action:focus-visible) { outline:2px solid var(--accent); outline-offset:-2px; }
  .action-field :global(.location-mode[aria-pressed="true"]) { background:var(--row-sel); color:var(--text); }
  .address { transition:background-color 140ms ease; }
  .address.inactive { background:color-mix(in srgb, var(--panel) 55%, var(--bg)); }
  .address :global(.address-input) { flex:1; width:100%; min-width:0; height:32px; padding:0 var(--sp-2); border-radius:var(--input-radius); border:0; outline:none; background:transparent; color:var(--text); }
  .address :global(.address-input:disabled) { color:var(--muted); }
  .recovery { display:flex; align-items:center; flex-wrap:wrap; gap:var(--sp-1) var(--sp-2); margin-block-start:var(--sp-1); }
  .recovery, .format-mode { color:var(--muted); font-size:var(--text-xs); overflow-wrap:anywhere; }
  .format-mode { margin:0; align-self:end; line-height:var(--ctl-h); }
  .format-row { display:flex; align-items:end; gap:var(--sp-2); }
  .format-options { display:contents; }
  .format-setting { min-width:0; display:grid; gap:var(--sp-1); }
  .format { flex:0 1 166px; }
  .method { flex:0 1 142px; }
  .quality { flex:0 1 105px; }
  .format-setting .field-label { white-space:nowrap; }
  .format-setting .action-field :is(select, :global(.app-select), .fixed-method) { flex:1; min-width:0; }
  .fixed-method { padding-inline:var(--input-inset); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .format-setting .action-field :global(button.app-select-trigger) { border:0; background:transparent; padding-inline-end:var(--sp-1); }
  .format-setting .action-field :global(button.app-select-trigger:focus-visible) { outline:none; box-shadow:none; }
  .pattern { display:grid; gap:var(--sp-1); }
  .pattern-field :global(.field-pattern-control) { background:transparent; }
  .pattern-field :global(.field-pattern-control:focus-within) { outline:none; }
  .save-settings :global(button.new-folder) { display:flex; align-items:center; gap:var(--sp-2); color:var(--text); width:fit-content; padding:var(--sp-1) var(--sp-2); margin-inline-start:calc(-1 * var(--sp-2)); border:0; border-radius:var(--r-compact); background:transparent; }
  .save-settings :global(button.new-folder:hover:enabled) { background:var(--row-hover); }
  .save-settings :global(button.new-folder:focus-visible) { outline:2px solid var(--accent); }
  .save-settings :global(button.new-folder:disabled) { opacity:.5; }
  .new-folder-check { width:15px; height:15px; display:grid; place-items:center; border:1px solid var(--border); border-radius:4px; background:var(--bg); }
  :global(button.new-folder[aria-checked="true"]) .new-folder-check { border-color:var(--accent); background:var(--accent); color:var(--primary-foreground); }
  select { width:100%; min-width:0; height:32px; padding:0 var(--sp-2); border-radius:var(--input-radius); border:0; background:transparent; outline:none; color:var(--text); }
  :global(.app-select-menu.split-pattern-presets [data-slot='menu-item']) { display:flex; align-items:center; min-height:32px; border-radius:var(--r-menu-item); padding:var(--sp-1) var(--sp-3); cursor:default; outline:none; }
  :global(.split-pattern-presets [role='separator']) { border-top:1px solid var(--border); margin:var(--sp-1) var(--sp-2); }
  :global(.split-method-info) { z-index:140; display:grid; gap:var(--sp-2); width:min(21rem, calc(100vw - 24px)); padding:var(--sp-3); border:1px solid var(--border); border-radius:var(--r-inset); background:var(--bg); color:var(--text); box-shadow:0 12px 32px rgb(0 0 0 / .25); }
  :global(.split-method-info p) { margin:0; line-height:1.45; }
  .review { background:var(--panel); border-radius:var(--r-inset); padding:var(--sp-3); }
  .output-review { margin-block-start:var(--sp-1); }
  .review-note { color:var(--muted); margin:0 0 var(--sp-2); font-size:var(--text-xs); }
  ol, ul { list-style:none; margin:0; padding:0; max-height:240px; overflow:auto; overscroll-behavior:contain; }
  li { border-radius:var(--r-compact); padding:var(--sp-1) var(--sp-2); }
  .output-row { display:flex; align-items:center; justify-content:space-between; gap:var(--sp-2); padding-block:0; }
  .output-row span { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .output-row :global(button) { flex:none; }
  .output-row:nth-child(even) { background:color-mix(in srgb, var(--panel) 70%, var(--bg)); }
  li:hover, li:focus-within, .output-row:hover, .output-row:focus-within { background:var(--row-hover); }
  .warning { color:var(--text); margin:var(--sp-1) 0 var(--sp-2); overflow-wrap:anywhere; }
  .output-path { overflow-wrap:anywhere; color:var(--muted); font-size:var(--text-xs); }
  .backups { margin-block:var(--sp-2); }
  .backups h4 { margin:0; font-size:var(--text-sm); font-weight:var(--fw-medium); }
  .export-progress { display:grid; gap:var(--sp-1); }
  .export-progress p { margin:0; }
  .export-progress :global(.split-output-progress-track) { height:5px; overflow:hidden; border-radius:5px; background:var(--panel); }
  .export-progress :global(.split-output-progress-fill) { height:100%; width:var(--progress-value, 25%); border-radius:inherit; background:var(--accent); }
  .export-progress :global(.split-output-progress-fill[data-indeterminate]) { animation:progress-travel 1.2s linear infinite alternate; }
  @keyframes progress-travel { from { transform:translateX(0); } to { transform:translateX(300%); } }
  @media (prefers-reduced-motion: reduce) { .address { transition:none; } .export-progress :global(.split-output-progress-fill[data-indeterminate]) { animation:none; } }
</style>
