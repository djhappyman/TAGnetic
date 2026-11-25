<script lang="ts">
  import { onMount } from 'svelte';
  import { Bookmark, Copy, FileOutput, FolderOpen, Info, Play, Settings2, Square } from '@lucide/svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import * as DropdownMenu from '$lib/components/ui/dropdown-menu/index.js';
  import * as Popover from '$lib/components/ui/popover/index.js';
  import FieldPatternInput from './FieldPatternInput.svelte';
  import { fieldLabel } from '../js/fields.js';
  import { catalogue } from '../js/placeholders.js';
  import { maskPresets } from '../js/state.js';
  import { defaultSplitFilePattern, directory, leaf, type Format, type Progress, type Review, type SaveMode, type SplitMethod } from '../js/audio-split';

  let {
    split, canCopy, copying, treated, blocked, sampleRate, sourcePaths, saveMode = $bindable(), destination = $bindable(),
    backupDirectory = $bindable(), createFolder = $bindable(), folderPattern = $bindable(),
    format = $bindable(), splitMethod = $bindable(), bitrate = $bindable(), namePattern = $bindable(),
    review, progress, previewing, finished, backups, restored, exportWarnings, outputAudition, onbrowse, onchange, onaudition,
  }: {
    split: boolean; canCopy: boolean; copying: boolean; treated: boolean; blocked: boolean; sampleRate: number; sourcePaths: string[];
    saveMode: SaveMode; destination: string; backupDirectory: string; createFolder: boolean; folderPattern: string;
    format: Format; bitrate: number; namePattern: string; splitMethod: SplitMethod;
    review: Review | null; progress: Progress | null; previewing: boolean; finished: string[]; backups: string[];
    restored: string[]; exportWarnings: string[]; outputAudition: number | null;
    onbrowse: (backup?: boolean) => void; onchange: (delayed?: boolean) => void; onaudition: (index: number) => void;
  } = $props();

  const saveChoices = [
    { value: 'replace', label: 'Replace original', icon: FileOutput },
    { value: 'beside', label: 'Save alongside original', icon: Copy },
  ] as const;
  const commonFields = ['track', 'title', 'artist', 'album', 'albumartist', 'disc', 'year', 'genre', 'comment'];
  let fields = $state.raw(commonFields.map(key => ({ key, label: fieldLabel(key) })));
  let settingsOpen = $state(false);
  const sourceFolders = $derived([...new Set(sourcePaths.map(directory))]);
  const sourceFolder = $derived(sourceFolders.length === 1 ? sourceFolders[0] : 'Each original’s folder');
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
</script>

{#snippet patternEditor(target: 'file' | 'folder', label: string, value: string, placeholder: string)}
  <div class="pattern">
    <span class="field-label">{label}</span>
    <div class="action-field pattern-field">
      <FieldPatternInput id={'split-output-' + target + '-pattern'} {label} {value} {placeholder} disabled={blocked}
        {fields} onchange={value => choosePattern(target, value, true)}/>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          {#snippet child({ props })}
            <Button {...props} variant="ghost" size="icon-sm" class="field-action" disabled={blocked}
              aria-label={label + ' presets'} title={label + ' presets'}><Bookmark size={15}/></Button>
          {/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Content class="split-pattern-presets" align="end">
          {#if target === 'folder'}
            <DropdownMenu.Item onSelect={() => choosePattern(target, '%album%')}>Album</DropdownMenu.Item>
            <DropdownMenu.Item onSelect={() => choosePattern(target, '%albumartist% - %album%')}>Album Artist – Album</DropdownMenu.Item>
          {:else}
            <DropdownMenu.Item onSelect={() => choosePattern(target, split ? defaultSplitFilePattern : '')}>{split ? 'Track number – Title (default)' : 'Source filename (default)'}</DropdownMenu.Item>
            <DropdownMenu.Item onSelect={() => choosePattern(target, '%artist% - %title%')}>Artist – Title</DropdownMenu.Item>
          {/if}
          {#if maskPresets.length}
            <DropdownMenu.Separator />
            {#each maskPresets as preset (preset.name)}
              <DropdownMenu.Item onSelect={() => choosePattern(target, preset.mask)}>{preset.name}</DropdownMenu.Item>
            {/each}
          {/if}
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    </div>
  </div>
{/snippet}

{#if !finished.length}
  <section class="save-settings" aria-labelledby="studio-save-heading">
    <h3 id="studio-save-heading" tabindex="-1">Review &amp; save</h3>

    <fieldset disabled={blocked}>
      <legend><label for="split-save-address">Save location</label></legend>
      <div class={['action-field', 'address', { inactive: saveMode !== 'folder' }]}>
        <input id="split-save-address" class="address-input" value={saveMode === 'folder' ? destination : sourceFolder}
          placeholder="Type a folder path" disabled={saveMode !== 'folder' || blocked} spellcheck="false"
          title={saveMode === 'folder' ? destination : sourceFolder}
          oninput={event => { destination = event.currentTarget.value; onchange(true); }}/>
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
                  <Popover.Root>
                    <Popover.Trigger>
                      {#snippet child({ props })}
                        <Button {...props} variant="ghost" size="icon-sm" class="field-action" title="About split methods" aria-label="About split methods" disabled={blocked}>
                          <Info size={15}/>
                        </Button>
                      {/snippet}
                    </Popover.Trigger>
                    <Popover.Content class="split-method-info" align="start" collisionPadding={12}>
                      <p><strong>Fast split</strong> copies the audio as it is. It’s quick and keeps the same quality. Cuts may move slightly to the nearest spot the file can be cut. Some older MP3 players may play a little extra audio at a cut.</p>
                      <p><strong>Exact cuts</strong> split right where you put the markers. This takes longer because the app makes new audio files. Creating a new MP3 can reduce quality.</p>
                      {#if treated}<p>Audio treatments require encoding new files, so exact cuts are used.</p>
                      {:else if !canCopy}<p>Changing the file type requires making new audio files, so exact cuts are used.</p>{/if}
                    </Popover.Content>
                  </Popover.Root>
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
    <label class="new-folder">
      <input type="checkbox" bind:checked={createFolder} disabled={blocked} onchange={() => onchange()}/> Create a new folder
    </label>
    {#if createFolder}
      {@render patternEditor('folder', 'Folder name', folderPattern, 'Type a name or a tag such as %album%')}
    {/if}
  </section>
{/if}

{#if review && !finished.length}
  <section class="review output-review" aria-label="Output review" aria-busy={previewing}>
    <h3 id="audio-split-review-heading" tabindex="-1">{review.outputs.length} {split ? (review.outputs.length === 1 ? 'track' : 'tracks') : (review.outputs.length === 1 ? 'file' : 'files')} to save</h3>
    <ol aria-label="Output filenames">
      {#each review.outputs as output, index (output.path)}
        {@const filename = leaf(output.path)}
        {@const listening = outputAudition === index}
        <li class="output-row">
          <span title={output.path}>{filename}</span>
          <Button variant="ghost" size="icon-sm" title={listening ? 'Stop audition' : 'Audition track'}
            aria-label={(listening ? 'Stop audition of ' : 'Audition ') + filename} disabled={blocked || previewing}
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
    {#if progress.framesTotal && progress.framesDone !== undefined}
      <progress max={progress.framesTotal} value={progress.framesDone} aria-label="Audio export progress"></progress>
    {:else if progress.total}
      <progress max={progress.total} value={progress.completed} aria-label="Audio export progress"></progress>
    {:else}
      <progress aria-label="Audio export progress"></progress>
    {/if}
  </div>
{/if}

{#if finished.length}
  <section class="review" aria-label="Completed files">
    {#if restored.length}
      <h3 id="audio-split-done-heading" tabindex="-1">Restored {restored.length} {restored.length === 1 ? 'original' : 'originals'}</h3>
      <ul>{#each restored as file (file)}<li class="output-path">{file}</li>{/each}</ul>
    {:else}
      <h3 id="audio-split-done-heading" tabindex="-1">Saved {finished.length} {finished.length === 1 ? 'file' : 'files'}</h3>
      {#each exportWarnings as warning, index (index)}<p class="warning">{warning}</p>{/each}
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

<style>
  .save-settings { display:grid; gap:var(--sp-3); }
  fieldset { min-width:0; border:0; padding:0; margin:0; }
  legend, .field-label { color:var(--muted); font-size:var(--text-xs); }
  legend, h3 { margin:0 0 var(--sp-1); }
  h3 { color:var(--text); font-weight:600; font-size:var(--text-sm); }
  .save-settings > h3 { margin:0; font-size:var(--text-lg); }
  .action-field { display:flex; align-items:center; min-width:0; min-height:var(--ctl-h); border-radius:var(--input-radius); background:var(--panel); }
  .action-field:has(:is(input, select, :global(.app-select-trigger), :global(.field-pattern-editor)):focus-visible) { outline:2px solid var(--accent); }
  .action-field :global(.field-action) { flex:none; margin-inline-end:2px; }
  .action-field :global(.field-action:focus-visible) { outline:2px solid var(--accent); outline-offset:-2px; }
  .action-field :global(.location-mode[aria-pressed="true"]) { background:var(--row-sel); color:var(--text); }
  .address { transition:background-color 140ms ease; }
  .address.inactive { background:color-mix(in srgb, var(--panel) 55%, var(--bg)); }
  .address-input { flex:1; width:100%; outline:none; background:transparent; }
  .address-input:disabled { color:var(--muted); }
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
  .new-folder { display:flex; align-items:center; gap:var(--sp-1); color:var(--text); width:fit-content; }
  .new-folder input { margin:0; accent-color:var(--accent); }
  select, .address-input { min-width:0; height:32px; padding:0 var(--sp-2); border-radius:var(--input-radius); border:0; color:var(--text); }
  select { width:100%; background:transparent; outline:none; }
  :global(.split-pattern-presets) { z-index:140; }
  :global(.split-method-info) { z-index:140; width:min(21rem, calc(100vw - 24px)); background:var(--bg); color:var(--text); }
  :global(.split-method-info p) { margin:0; line-height:1.45; }
  .review { background:var(--panel); border-radius:var(--r-inset); padding:var(--sp-3); }
  .output-review { margin-block-start:var(--sp-1); }
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
  progress { width:100%; accent-color:var(--accent); }
  @media (prefers-reduced-motion: reduce) { .address { transition:none; } }
</style>
