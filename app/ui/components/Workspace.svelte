<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '../js/i18n.js';
  import { FolderOpen, FolderPlus, Undo2, Redo2, ListChecks, ListX, ArrowDown, ArrowUp, Group, ChevronDown, PanelRightOpen, PanelRightClose } from '@lucide/svelte';
  import RibbonTools from './RibbonTools.svelte';
  import BrandWordmark from './BrandWordmark.svelte';
  import { subscribeWorkspaceState, setWorkspaceMode, setWorkspaceSubtab, type WorkspaceMode, type WorkspaceSubtab } from '$lib/workspace-state';
  import DockWorkspace from './DockWorkspace.svelte';
  import CompactPlayer from './CompactPlayer.svelte';
  import DockModule from './DockModule.svelte';
  import type { WorkspaceLayout } from '$lib/workspace-layout';
  import FilterPanel from './FilterPanel.svelte';
  import QueryChips from './QueryChips.svelte';
  import QueryFeedback from './QueryFeedback.svelte';
  import DuplicateScanControls from './DuplicateScanControls.svelte';
  import DuplicateHelp from './DuplicateHelp.svelte';
  import InspectorHelp from './InspectorHelp.svelte';
  import ImportFields from './ImportFields.svelte';
  import NativeCheckbox from './controls/NativeCheckbox.svelte';
  import NativeSlider from './controls/NativeSlider.svelte';
  import * as Tabs from '$lib/components/ui/tabs/index.js';
  import * as Table from '$lib/components/ui/table/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Textarea } from '$lib/components/ui/textarea/index.js';
  import { NativeSelect, NativeSelectOption } from '$lib/components/ui/native-select/index.js';
  let dock = $state<WorkspaceLayout>();
  let mode = $state<WorkspaceMode>('tagger');
  let subtab = $state<WorkspaceSubtab>('online');
  const primaryTab = $derived(mode === 'tagger' && subtab === 'online' ? 'online' : mode);
  function changePrimaryTab(value: string) {
    if (value === 'online') {
      setWorkspaceSubtab('online');
      changeTab('tagger');
    } else {
      if (value === 'tagger' && subtab === 'online') setWorkspaceSubtab('editor');
      changeTab(value);
    }
  }
  // Narrow windows show one working pane at a time. Dock modules stay mounted, so
  // a cell edit, staged field or scroll position survives the switch.
  let sidebarClosed = $state(false);
  function setSidebarClosed(value: boolean, persist = true) {
    sidebarClosed = value;
    document.body.classList.toggle('panel-collapsed', value);
    if (value) { dock?.reveal('files'); document.getElementById('panel-toggle')?.focus({preventScroll:true}); }
    if (persist) try { localStorage.setItem('tagrex.sidebarCollapsed', value ? '1' : '0'); } catch { /* Session-only preference. */ }
  }
  onMount(() => {
    try { setSidebarClosed(localStorage.getItem('tagrex.sidebarCollapsed') === '1', false); } catch { /* Open by default. */ }
  });
  function toggleSidebar() {
    const closed = !sidebarClosed;
    setSidebarClosed(closed);
    if (!closed) dock?.reveal('inspector');
  }
  let moreOpen = $state(false);
  let applying = $state(false);
  function openPresetLibrary() { document.getElementById('presets-edit')?.click(); }
  let generatorJob = $state<'transform' | 'number' | 'sides'>('transform');
  function attachPaneRouting(node: HTMLElement) {
    const onClick = (event: MouseEvent) => {
      // Choosing a tool is a deliberate edit/search move: on a narrow window it
      // reveals that tool instead of leaving it below an unusable table slice.
      if ((event.target as Element).closest('.mode-tab')) { setSidebarClosed(false); dock?.reveal('inspector'); }
      if ((event.target as Element).closest('#gen-job-transform')) showTransformations();
    };
    const revealModule = (event: Event) => {
      const module = (event as CustomEvent<string>).detail;
      if (module === 'transformations') { showTransformations(); return; }
      if (module !== 'files') setSidebarClosed(false);
      dock?.reveal(module);
    };
    node.addEventListener('click', onClick);
    document.addEventListener('tagrex:reveal-module', revealModule);
    return () => { node.removeEventListener('click', onClick); document.removeEventListener('tagrex:reveal-module', revealModule); };
  }
  // Keep the permanent player below the file pane without moving its native
  // controls into a dock module (which can be hidden in the compact layout).
  function selectionSummarySpacing(node: HTMLElement) {
    const pane = node.closest<HTMLElement>('.files-col');
    let previous = 0;
    const resize = new ResizeObserver(() => {
      const height = node.offsetHeight;
      if (height === previous) return;
      previous = height;
      pane?.style.setProperty('--selection-summary-space', `${height + 16}px`);
    });
    resize.observe(node);
    return () => resize.disconnect();
  }
  // The overflow panel stays in the DOM so the controller-owned buttons inside
  // keep their stable IDs. They are inert while closed rather than destroyed.
  $effect(() => {
    if (!moreOpen) return;
    const onPointer = (event: PointerEvent) => {
      const target = event.target as Element | null;
      if (target?.closest('#more-panel, #more-btn, .col-menu')) return;
      moreOpen = false;
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      if (moreOpen) { moreOpen = false; requestAnimationFrame(() => document.getElementById('more-btn')?.focus()); }
    };
    document.addEventListener('pointerdown', onPointer, true);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onPointer, true);
      document.removeEventListener('keydown', onKey);
    };
  });
  function openPanel() {
    moreOpen = !moreOpen;
    if (!moreOpen) return;
    requestAnimationFrame(() => {
      document.getElementById('more-panel')?.querySelector<HTMLElement>('button:not([hidden]), input, select')?.focus();
    });
  }
  onMount(() => {
    const bar = document.getElementById('diff-actionbar');
    if (!bar) return;
    const sync = () => { applying = bar.classList.contains('applying'); };
    const observer = new MutationObserver(sync);
    observer.observe(bar, { attributes:true, attributeFilter:['class'] });
    sync();
    return () => observer.disconnect();
  });
  onMount(() => subscribeWorkspaceState(next => {
    mode = next.mode;
    subtab = next.subtab;
  }));
  function showTransformations() {
    setWorkspaceSubtab('transformations');
    changeTab('tagger');
  }
  function changeTab(value: string, isSubtab = false) {
    if (isSubtab) setWorkspaceSubtab(value);
    else {
      setWorkspaceMode(value);
      setSidebarClosed(false);
      dock?.reveal('inspector');
    }
  }

</script>
<div class="controller-tabs-layout" {@attach attachPaneRouting}>
<Tabs.Root bind:value={mode} class="controller-tabs-root" onValueChange={(value) => changeTab(value)}>

<header class="topbar" data-tauri-drag-region>
  <div class="topbar-main" data-tauri-drag-region>
    <div class="brand" role="img" aria-label="Tagnetic">
      <BrandWordmark />
    </div>

    <Tabs.Root value={primaryTab} onValueChange={changePrimaryTab} class="primary-navigation">
    <Tabs.List class="mode-tabs" aria-label="Tools">
      <Tabs.Trigger value="tagger">
{#snippet child({ props })}
<Button {...props} data-slot="button" variant="ghost" size="sm" class="mode-tab" id="workspace-tags-tab" aria-controls="panel-tagger" data-mode="tagger" data-i18n-title="mode.tagger.title" title="Edit tags manually and manage cover art" data-i18n-aria-label="mode.tagger.label" aria-label="Tags"><svg class="ico"><use href="#i-tag"/></svg><span class="lbl" data-i18n="mode.tagger.label">Tags</span></Button>
{/snippet}
</Tabs.Trigger>
      <Tabs.Trigger value="online">
{#snippet child({ props })}
<Button {...props} data-slot="button" variant="ghost" size="sm" class="mode-tab" id="workspace-online-tab" aria-controls="panel-tagger" data-mode="online" data-i18n-title="inspector.onlineHeading" title="Find a release" data-i18n-aria-label="tagger.online" aria-label="Online"><svg class="ico"><use href="#i-search"/></svg><span class="lbl" data-i18n="tagger.online">Online</span></Button>
{/snippet}
</Tabs.Trigger>
      <Tabs.Trigger value="renamer">
{#snippet child({ props })}
<Button {...props} data-slot="button" variant="ghost" size="sm" class="mode-tab" id="workspace-renamer-tab" aria-controls="panel-renamer" data-mode="renamer" data-i18n-title="mode.renamer.title" title="Rename files and move them into folders" data-i18n-aria-label="mode.renamer.label" aria-label="Rename"><svg class="ico"><use href="#i-rename"/></svg><span class="lbl" data-i18n="mode.renamer.label">Rename</span></Button>
{/snippet}
</Tabs.Trigger>
      <Tabs.Trigger value="deduplicator">
{#snippet child({ props })}
<Button {...props} data-slot="button" variant="ghost" size="sm" class="mode-tab" id="query-mode-toggle" aria-controls="panel-deduplicator" aria-pressed={mode === 'deduplicator'} data-mode="deduplicator" data-i18n-title="mode.deduplicator.title" title="Find likely duplicate files in the open library" data-i18n-aria-label="mode.deduplicator.label" aria-label="Duplicates"><svg class="ico"><use href="#i-dedup"/></svg><span class="lbl" data-i18n="mode.deduplicator.label">Duplicates</span></Button>
{/snippet}
</Tabs.Trigger>
      <Tabs.Trigger value="exporter">
{#snippet child({ props })}
<Button {...props} data-slot="button" variant="ghost" size="sm" class="mode-tab" id="workspace-exporter-tab" aria-controls="panel-exporter" data-mode="exporter" data-i18n-title="mode.exporter.title" title="Export playlists, tables, and reports" data-i18n-aria-label="mode.exporter.label" aria-label="Export"><svg class="ico"><use href="#i-export"/></svg><span class="lbl" data-i18n="mode.exporter.label">Export</span></Button>
{/snippet}
</Tabs.Trigger>
    </Tabs.List>
    </Tabs.Root>
    <FilterPanel>
      <div class="filter-ctl field is-empty">
        <span class="query-mode-label" aria-hidden="true"><svg class="ico"><use href="#i-search"/></svg></span>
        <QueryChips />
        <Input id="filter" type="search" placeholder="Start typing…" aria-label="Search files" spellcheck="false" />
        <div class="field-tools">
          <Button variant="ghost" size="icon-sm" id="filter-clear" class="icon field-clear" type="button" data-i18n-title="toolbar.clearFilter" title="Clear the filter" data-i18n-aria-label="toolbar.clearFilter" aria-label="Clear the filter"><svg class="ico"><use href="#i-close"/></svg></Button>
      <div class="filter-presets">
        <div class="col-picker">
          <Button variant="ghost" size="icon-sm" id="presets-btn" class="tray-action" data-i18n-title="toolbar.presetsTitle" title="Save and reuse filter and sort presets" data-i18n-aria-label="toolbar.presets" aria-label="Filter and sort presets"><svg class="ico"><use href="#i-presets"/></svg></Button>
          <div id="presets-menu" class="col-menu" hidden></div>
        </div>
      </div>
        </div>
      </div>
      {#if mode !== 'deduplicator'}<QueryFeedback />{/if}
    </FilterPanel>
    <div class="topbar-actions" data-tauri-drag-region>
    <span id="update-pill" class="update-pill" hidden>
      <Button variant="outline" size="sm" id="update-open" class="update-open" type="button"></Button>
      <Button variant="ghost" size="icon-sm" id="update-notes" class="update-notes icon" type="button" hidden data-i18n-title="update.notes" title="What's new in this version" data-i18n-aria-label="update.notes" aria-label="What's new in this version"><svg class="ico"><use href="#i-doc"/></svg></Button>
      <Button variant="ghost" size="icon-sm" id="update-skip" class="update-skip icon" type="button" data-i18n-title="update.skip" title="Don't tell me about this version" data-i18n-aria-label="update.skip" aria-label="Don't tell me about this version"><svg class="ico"><use href="#i-close"/></svg></Button>
    </span>

    <Button variant="ghost" size="icon-sm" id="agent-activity" class="icon" hidden title="External agent connections and work" aria-label="External agent connections and work"><svg class="ico"><use href="#i-agent"/></svg></Button>
    <Button variant="ghost" size="icon-sm" id="settings-open" class="icon gear" data-i18n-title="topbar.settings" title="Settings" data-i18n-aria-label="topbar.settings" aria-label="Settings"><svg class="ico"><use href="#i-settings"/></svg></Button>
    <Button variant="ghost" size="icon-sm" id="panel-toggle" class="icon" aria-expanded={!sidebarClosed} aria-controls="mode-col" disabled={applying} onclick={toggleSidebar} data-i18n-title={sidebarClosed ? 'topbar.expandPanelTitle' : 'topbar.collapsePanelTitle'} title={t(sidebarClosed ? 'topbar.expandPanelTitle' : 'topbar.collapsePanelTitle')} data-i18n-aria-label={sidebarClosed ? 'topbar.expandPanel' : 'topbar.collapsePanel'} aria-label={t(sidebarClosed ? 'topbar.expandPanel' : 'topbar.collapsePanel')}>{#if sidebarClosed}<PanelRightOpen class="ico" size={17}/>{:else}<PanelRightClose class="ico" size={17}/>{/if}</Button>
    </div>
  </div>
  <!-- Legacy controller targets stay mounted during ribbon iteration. -->
  <div hidden inert aria-hidden="true">
    <Button id="preset-library" variant="ghost" size="sm" disabled={applying} onclick={openPresetLibrary} data-i18n-title="presets.title" title="Transformation presets"><svg class="ico"><use href="#i-wand"/></svg><span data-i18n="toolbar.presetsShort">Presets</span></Button>
    <Button variant="ghost" size="sm" id="detect-bpm" class="toolbar-command search-toggle" title="Detect BPM" aria-label="Detect BPM"><svg class="ico glyph-search"><use href="#i-metronome"/></svg><svg class="ico glyph-stop"><use href="#i-stop"/></svg><span class="tray-label" data-i18n="toolbar.bpmShort">BPM</span></Button>

    <div class="toolbar-overflow">
      <Button id="more-btn" variant="ghost" size="icon-sm" class="toolbar-trigger" aria-expanded={moreOpen} aria-controls="more-panel" onclick={openPanel} data-i18n-aria-label="toolbar.more" aria-label="More"><svg class="ico"><use href="#i-more"/></svg></Button>
      <div id="more-panel" class={['toolbar-panel more-panel', { open: moreOpen }]} role="group" aria-label="Selection actions" data-i18n-aria-label="toolbar.morePanel" aria-hidden={!moreOpen} inert={!moreOpen}>
        <div class="tray-row transform-row">
          <Button variant="ghost" size="sm" id="transform-btn" class="tray-action" data-i18n-title="toolbar.transformTitle" title="Rules to run on the values this panel produces" data-i18n-aria-label="toolbar.transform" aria-label="Transform values"><svg class="ico"><use href="#i-wand"/></svg><span class="tray-label" data-i18n="toolbar.transform">Transform values</span></Button>
        </div>

        <div class="tray-row">
          <Button variant="ghost" size="sm" id="clear-tags" class="tray-action danger" data-i18n-title="toolbar.clearTagsTitle" title="Clears text tags on the selected files. Cover art and DJ cue points are kept. Previewed before anything is written." data-i18n-aria-label="toolbar.clearTags" aria-label="Clear text tags on the selected files"><svg class="ico"><use href="#i-eraser"/></svg><span class="tray-label" data-i18n="editor.clearTags">Clear tags</span></Button>
        </div>
      </div>
    </div>

  </div>
</header>

<main class="workarea">

  <DockWorkspace bind:controller={dock} drawer closed={sidebarClosed}>
  <DockModule moduleId="files" title="Files" defaultArea="left" headerless movable={false}>
  <div class="ribbon-strip">
  <div class="command-toolbar command-ribbon" role="group" aria-label="File commands">
    <div class="ribbon-group" role="group" aria-label="Folders">
      <Button id="ribbon-open-folder" variant="ghost" size="icon-sm" class="icon ribbon-command" title="Open folder" aria-label="Open folder" disabled={applying}><FolderOpen class="ico" size={17} strokeWidth={1.7} /></Button>
      <Button id="ribbon-add-folder" variant="ghost" size="icon-sm" class="icon ribbon-command" title="Add folder to the current list" aria-label="Add folder" disabled={applying}><FolderPlus class="ico" size={17} strokeWidth={1.7} /></Button>
    </div>
    <span class="ribbon-divider" aria-hidden="true"></span>
    <div class="ribbon-group" role="group" aria-label="History">
    <Button variant="ghost" size="icon-sm" id="undo" class="icon ribbon-command" disabled data-i18n-title="topbar.undo" title="Undo the last applied batch" data-i18n-aria-label="topbar.undo" aria-label="Undo the last applied batch"><Undo2 class="ico" size={17} strokeWidth={1.7} /></Button>
    <Button variant="ghost" size="icon-sm" id="redo" class="icon ribbon-command" disabled data-i18n-title="topbar.redo" title="Redo the last undone batch" data-i18n-aria-label="topbar.redo" aria-label="Redo the last undone batch"><Redo2 class="ico" size={17} strokeWidth={1.7} /></Button>
    </div>
    <span class="ribbon-divider" aria-hidden="true"></span>
    <div class="ribbon-group" role="group" aria-label="Selection">
      <Button id="ribbon-select-all" variant="ghost" size="icon-sm" class="icon ribbon-command" title="Select all listed files" aria-label="Select all listed files" disabled><ListChecks class="ico" size={17} strokeWidth={1.7} /></Button>
      <Button id="ribbon-clear-selection" variant="ghost" size="icon-sm" class="icon ribbon-command" title="Clear selection" aria-label="Clear selection" disabled><ListX class="ico" size={17} strokeWidth={1.7} /></Button>
    </div>
    <span class="ribbon-divider" aria-hidden="true"></span>
    <div class="ribbon-group" role="group" aria-label="File order">
      <Button id="ribbon-move-down" variant="ghost" size="icon-sm" class="icon ribbon-command" title="Move selected files down within their group" aria-label="Move selected files down" disabled><ArrowDown class="ico" size={17} strokeWidth={1.7} /></Button>
      <Button id="ribbon-move-up" variant="ghost" size="icon-sm" class="icon ribbon-command" title="Move selected files up within their group" aria-label="Move selected files up" disabled><ArrowUp class="ico" size={17} strokeWidth={1.7} /></Button>
    </div>
    <span class="ribbon-divider" aria-hidden="true"></span>
    <div class="ribbon-group" role="group" aria-label="List organisation">
      <div class="col-picker group-ctl">
        <Button id="group-btn" variant="ghost" size="icon-sm" class="icon ribbon-command ribbon-menu-trigger" title="Group files" aria-label="Group files" aria-expanded="false" aria-controls="group-menu"><Group class="ico" size={17} strokeWidth={1.7} /><ChevronDown class="ribbon-chevron" size={10} /></Button>
        <Button variant="ghost" size="icon-sm" id="toggle-groups" class="icon ribbon-command" hidden data-i18n-title="toolbar.collapseGroups" title="Collapse every group" data-i18n-aria-label="toolbar.collapseGroups" aria-label="Collapse every group"><svg class="ico"><use href="#i-collapse-all"/></svg></Button>
        <div id="group-menu" class="col-menu" hidden></div>
      </div>

    </div>
    <RibbonTools />
  </div>
  </div>
  <section class="files-col">
    <div class="file-view-tabs" role="tablist" aria-label="File views" data-i18n-aria-label="staging.views">
      <Button id="files-tab" variant="ghost" size="sm" role="tab" aria-selected="true" aria-controls="files-view" aria-labelledby="files-tab-label" aria-describedby="status-sel"><span id="files-tab-label" data-i18n="staging.files">Files</span><span class="files-tab-counts" aria-hidden="true"><span id="files-total-count">0</span></span><span id="status-sel" class="sr-only"></span></Button>
      <Button id="staged-tab" variant="ghost" size="sm" role="tab" aria-selected="false" aria-controls="files-view" tabindex={-1}><span data-i18n="staging.tab">Staged</span><span id="staged-tab-count" class="staged-tab-count"></span></Button>
    </div>
    <div class="library-header">
      <div class="lib">

        <div id="root-field" class="field">
          <Button variant="ghost" size="icon-sm" id="lib-action" class="icon" data-i18n-title="topbar.chooseFolder" title="Choose a folder to open" data-i18n-aria-label="topbar.chooseFolder" aria-label="Choose a folder to open"><svg class="ico"><use id="lib-action-glyph" href="#i-folder"/></svg></Button>
          <div id="root-box" class="path">
            <Button variant="ghost" size="sm" id="root-display" class="path-open field-value" type="button" data-i18n-title="topbar.chooseFolder" title="Choose a folder to open">
              <span class="path-parent"></span><span class="path-name">No folder open</span>
            </Button>
          </div>
          <Input id="root" type="text" data-i18n-placeholder="topbar.pathPlaceholder" placeholder="/path/to/music/library" spellcheck="false" hidden />
          <div class="field-tools">
            <Button variant="ghost" size="icon-sm" id="root-recent" class="icon" type="button" data-i18n-title="topbar.recent" title="Recently opened folders" data-i18n-aria-label="topbar.recent" aria-label="Recently opened folders" aria-expanded="false" hidden><svg class="ico"><use href="#i-caret-down"/></svg></Button>

            <Button variant="ghost" size="icon-sm" id="lib-refresh" class="icon" data-i18n-title="topbar.reread" title="Re-read the open folder" data-i18n-aria-label="topbar.reread" aria-label="Re-read the open folder" disabled><svg class="ico"><use href="#i-refresh"/></svg></Button>
          </div>
          <div id="root-recent-menu" class="col-menu" hidden></div>
        </div>

      </div>

    </div>

    <div id="files-view" class="table-wrap" role="tabpanel" aria-labelledby="files-tab">
      <Table.Root id="tracks" containerClass="contents">
        <Table.Header>
          <tr>
            <th class="sel"><NativeCheckbox id="select-all" /></th>
            <th data-sort="file" class="sortable"><span data-i18n="column.file">File</span><span class="sort-ind"></span></th>
            <th data-sort="artist" class="sortable"><span data-i18n="field.artist">Artist</span><span class="sort-ind"></span></th>
            <th data-sort="title" class="sortable"><span data-i18n="field.title">Title</span><span class="sort-ind"></span></th>
            <th data-sort="album" class="sortable"><span data-i18n="field.album">Album</span><span class="sort-ind"></span></th>
            <th data-sort="year" class="sortable"><span data-i18n="field.year">Year</span><span class="sort-ind"></span></th>
          </tr>
        </Table.Header>
        <Table.Body id="tracks-body" />
      </Table.Root>
      <p id="tracks-empty" class="empty inert-panel" data-i18n="table.empty">Open a folder to list its tracks.</p>
    </div>



    <div id="selection-summary" class="table-selection-summary" hidden {@attach selectionSummarySpacing}>
      <span class="selection-summary-text" role="status" aria-live="polite" aria-atomic="true"><span class="selection-summary-primary"><span id="files-selected-count">0</span> <span id="selection-summary-kind">selected</span></span><span id="selection-summary-detail" class="selection-summary-detail"></span></span>
      <Button id="selection-summary-clear" variant="ghost" size="icon-sm" class="icon" type="button" title="Clear selection" aria-label="Clear selection"><svg class="ico"><use href="#i-close"/></svg></Button>
    </div>

    <div id="duplicates-view" hidden>
      <div class="dup-bar">
        <span id="dup-summary" class="hint muted" style="padding: 0"></span>
        <div class="spacer"></div>
        <span class="ro-note"><svg class="ico"><use href="#i-lock"/></svg> <span data-i18n="dedup.readonly">Read-only. Nothing is changed.</span></span>
      </div>
      <div id="dup-results" class="dup-wrap"></div>
    </div>


  </section>

  </DockModule>

  <Button variant="ghost" id="col-splitter" hidden class="col-splitter" role="separator" tabindex={0} aria-valuenow={480} aria-valuemin={240} aria-valuemax={1200} aria-orientation="vertical" aria-label="Resize editor panel" aria-controls="panel-tagger panel-renamer panel-generator panel-deduplicator panel-exporter" data-i18n-title="toolbar.dragResize" title="Drag to resize" />

  <DockModule moduleId="inspector" title="Inspector" defaultArea="right" defaultIndex={1} headerless movable={false}>
  <aside class="mode-col" id="mode-col">


    <Tabs.Content value="renamer" id="panel-renamer" aria-labelledby="workspace-renamer-tab" class="mode-panel" hidden>
      <div class="panel-block">
        <h2 data-i18n="renamer.heading">Rename files</h2>
        <div class="mask-row">
          <div class="field">
            <Input id="mask" type="text" value="%artist% - %title%" spellcheck="false" placeholder="%artist% - %title%" />
            <div class="field-tools">
              <Button variant="ghost" size="icon-sm" class="icon ph-btn" type="button" data-ph-target="mask" data-i18n-title="mask.placeholders" title="Placeholders" data-i18n-aria-label="mask.placeholdersAria" aria-label="Placeholder reference"><svg class="ico"><use href="#i-help"/></svg></Button>
              <Button variant="ghost" size="icon-sm" class="icon preset-btn" type="button" data-preset-target="mask" data-i18n-title="preset.title" title="Mask presets" data-i18n-aria-label="preset.title" aria-label="Mask presets"><svg class="ico"><use href="#i-presets"/></svg></Button>

              <Button variant="ghost" size="icon-sm" id="mask-builder-open" class="icon" type="button" data-i18n-title="maskBuilder.open" title="Open the mask builder" data-i18n-aria-label="maskBuilder.open" aria-label="Open the mask builder"><svg class="ico"><use href="#i-builder"/></svg></Button>
            </div>
          </div>
        </div>

        <p id="mask-example" class="hint mono mask-example" aria-live="polite"></p>

        <details class="panel-help">
          <summary data-i18n="renamer.hintSummary">How masks work</summary>
          <p class="hint mask-hint" data-i18n="renamer.maskHint" data-i18n-html>The <span class="hint-tool">?</span> button lists placeholders, optional parts, widths and folders. The builder tries the mask on every selected file. Add <code>/</code> to sort files into folders.</p>
        </details>

        <label class="rule-flag" data-i18n-title="settings.sidecarsHint" title="Same-named files (lyrics, cue sheets, per-track covers) move with a renamed or moved track, and undo restores them together. A file already at the destination is never overwritten.">
          <NativeCheckbox id="carry-sidecars" checked /><span data-i18n="renamer.carrySidecars">Carry same-named files (lyrics, cue sheets…)</span>
        </label>
        <label class="rule-flag" data-i18n-title="renamer.reorganizeToggleTitle" title="Move or copy the renamed files into another folder">
          <NativeCheckbox id="reorganize-toggle" /><span data-i18n="renamer.reorganizeToggle">Move or copy into another folder</span>
        </label>

        <div id="reorganize-options" hidden>

          <label class="export-row"
            ><span data-i18n="renamer.into">Into</span>

            <span class="field is-path">
              <Input id="move-dest" type="text" spellcheck="false" data-i18n-placeholder="renamer.intoPlaceholder" placeholder="the opened library" readonly data-i18n-title="renamer.chooseTitle" title="Choose a destination folder" />
              <span class="field-tools">
                <Button variant="ghost" size="icon-sm" id="move-dest-clear" class="icon field-clear" type="button" data-i18n-title="renamer.backToLibrary" title="Back to the opened library" data-i18n-aria-label="renamer.backToLibrary" aria-label="Back to the opened library"><svg class="ico"><use href="#i-close"/></svg></Button>
                <span class="field-sep" aria-hidden="true"></span>
                <Button variant="ghost" size="icon-sm" id="move-dest-pick" class="icon" type="button" data-i18n-title="renamer.chooseTitle" title="Choose a destination folder" data-i18n-aria-label="renamer.chooseTitle" aria-label="Choose a destination folder"><svg class="ico"><use href="#i-folder"/></svg></Button>
              </span>
            </span>
          </label>
          <div class="export-row move-mode-row">
            <span data-i18n="renamer.files">Files</span>
            <div class="seg" id="move-mode" role="tablist" data-i18n-aria-label="renamer.moveOrCopy" aria-label="Move or copy">
              <Button variant="ghost" size="sm" class="seg-btn active" type="button" data-move-mode="move" data-i18n="renamer.move">Move</Button>
              <Button variant="ghost" size="sm" class="seg-btn" type="button" data-move-mode="copy" data-i18n="renamer.copy">Copy</Button>
            </div>
            <label class="rule-flag" data-i18n-title="renamer.pruneTitle" title="Remove folders this move leaves empty. Undo puts them back.">
              <NativeCheckbox id="move-prune" /><span data-i18n="renamer.prune">Tidy up empty folders</span>
            </label>
          </div>

          <label class="rule-flag carry-extras-flag" data-i18n-title="settings.extrasHint" title="When a reorganize empties a folder, everything left in it (a rip log, loose artwork, a Scans subfolder) moves to the same destination. A folder that still holds other music, or files that scatter to several destinations, is left alone.">
            <NativeCheckbox id="carry-extras" checked /><span data-i18n="renamer.carryExtras">Carry the rest of the folder (art, rip log, scans)</span>
          </label>
        </div>

        <div class="panel-actions">
          <Button variant="default" size="sm" id="preview" class="primary" data-i18n="renamer.previewRename">Preview</Button>
        </div>
      </div>
    </Tabs.Content>

    <Tabs.Content value="tagger" id="panel-tagger" aria-labelledby={subtab === 'online' ? 'workspace-online-tab' : 'workspace-tags-tab'} class="mode-panel">
<Tabs.Root bind:value={subtab} class="controller-tabs-root" onValueChange={(value) => changeTab(value, true)}>
      <div class="source-navigation" hidden={subtab === 'online'}>
      <Tabs.List class="subtabs" data-i18n-aria-label="tagger.mode" aria-label="Tagging mode">
        <Tabs.Trigger value="editor">
{#snippet child({ props })}
<Button {...props} data-slot="button" variant="ghost" size="sm" class="subtab" data-subtab="editor" type="button" data-i18n="tagger.editor">Editor</Button>
{/snippet}
</Tabs.Trigger>
        <Tabs.Trigger value="filename">
{#snippet child({ props })}
<Button {...props} data-slot="button" variant="ghost" size="sm" class="subtab" data-subtab="filename" type="button" data-i18n="tagger.fromName">From name</Button>
{/snippet}
</Tabs.Trigger>
        <Tabs.Trigger value="fromtext">
{#snippet child({ props })}
<Button {...props} data-slot="button" variant="ghost" size="sm" class="subtab" data-subtab="fromtext" type="button" data-i18n="tagger.fromText">From text</Button>
{/snippet}
</Tabs.Trigger>
        <Tabs.Trigger value="transformations">
{#snippet child({ props })}
<Button {...props} data-slot="button" variant="ghost" size="sm" class="subtab" data-subtab="transformations" type="button" data-i18n="workspace.transformations">Transformations</Button>
{/snippet}
</Tabs.Trigger>
      </Tabs.List>
        <div class="source-tools">
          {#if subtab === 'editor'}<InspectorHelp labelKey="inspector.editorInfo" helpKey="inspector.editorHelp" />
          {:else if subtab === 'filename'}<InspectorHelp labelKey="inspector.nameInfo" helpKey="inspector.nameHelp" />
          {:else if subtab === 'fromtext'}<InspectorHelp labelKey="inspector.textInfo" helpKey="inspector.textHelp" />
          {:else if subtab === 'transformations'}<InspectorHelp labelKey="inspector.transformInfo" helpKey="generator.transformHint" />{/if}
          <Button variant="ghost" size="icon-sm" id="presets-edit" hidden={subtab !== 'transformations'} class="transformations-edit icon" type="button" data-i18n-title="presets.edit" title="Edit transformation presets" data-i18n-aria-label="presets.title" aria-label="Transformation presets"><svg class="ico"><use href="#i-rename"/></svg></Button>
        </div>
      </div>

      <Tabs.Content value="transformations" id="subtab-transformations" class="subtab-panel transformations" hidden>
        <div class="transformations-scroll docked-transformations">
      <div id="transform-home"></div>
      <div id="transform-block" class="panel-block">
        <span id="transform-count" hidden></span>
        <p id="transform-context-note" class="hint inert-panel" style="padding: 6px 8px" hidden></p>

        <p id="transform-over-plan" class="hint inert-panel" style="padding: 6px 8px" hidden data-i18n="generator.overPlan">
          This runs on the staged changes, not the files. The result replaces the plan, so one
          Apply and one undo still cover everything.
        </p>

        <div id="groups-menu" class="groups-list"></div>
        <div class="panel-actions" hidden>

          <div class="spacer"></div>
          <Button variant="default" size="sm" id="transform-preview" class="primary" hidden data-i18n="generator.previewChanges">Preview</Button>
        </div>
      </div>

        </div>
      </Tabs.Content>

      <Tabs.Content value="online" id="subtab-online" aria-labelledby="workspace-online-tab" class="subtab-panel online">
        <div class="online-head">
          <div class="inspector-heading"><h2 data-i18n="inspector.onlineHeading">Find a release</h2><InspectorHelp labelKey="inspector.onlineInfo" helpKey="inspector.onlineHelp" /></div>
          <div class="online-head-row">
            <div class="source-label">
              <NativeSelect size="sm" id="online-source" data-i18n-aria-label="online.source" aria-label="Source" data-i18n-title="online.sourceTitle" title="Metadata source">
                <NativeSelectOption value="discogs">Discogs</NativeSelectOption>
                <NativeSelectOption value="musicbrainz">MusicBrainz</NativeSelectOption>
                <NativeSelectOption value="beatport">Beatport</NativeSelectOption>
                <NativeSelectOption value="bandcamp">Bandcamp</NativeSelectOption>
                <NativeSelectOption value="soundeo">Soundeo</NativeSelectOption>
              </NativeSelect>
            </div>

            <NativeSelect size="sm" id="search-format" data-i18n-aria-label="online.mediaFilter" aria-label="Filter results by media type" data-i18n-title="online.mediaFilter" title="Filter results by media type">
              <NativeSelectOption value="" data-i18n="online.allMedia">All media</NativeSelectOption>
              <NativeSelectOption value="CD">CD</NativeSelectOption>
              <NativeSelectOption value="Vinyl" data-i18n="media.vinyl">Vinyl</NativeSelectOption>
              <NativeSelectOption value="LP">LP</NativeSelectOption>
              <NativeSelectOption value="Cassette" data-i18n="media.cassette">Cassette</NativeSelectOption>
              <NativeSelectOption value="File" data-i18n="media.file">File</NativeSelectOption>
            </NativeSelect>
          </div>
          <div class="mask-row">

            <div id="query-box" class="query field">
              <ImportFields />
              <Input id="discogs-query" type="text" data-i18n-aria-label="online.queryPlaceholder" aria-label="Artist, album, or catalogue number" data-i18n-placeholder="online.queryPlaceholder" placeholder="artist / album / catalog no." spellcheck="false" />
              <div class="field-tools">
                <Button variant="ghost" size="icon-sm" id="discogs-clear" class="icon query-clear" type="button" data-i18n-title="online.clearSearch" title="Clear search and results" data-i18n-aria-label="online.clearSearch" aria-label="Clear search and results" hidden><svg class="ico" aria-hidden="true"><use href="#i-close"/></svg></Button>
                <Button variant="ghost" size="icon-sm" id="query-presets" class="icon" type="button" data-i18n-title="online.presets" title="Queries this selection can build" data-i18n-aria-label="online.presets" aria-label="Queries this selection can build" aria-expanded="false"><svg class="ico"><use href="#i-caret-down"/></svg></Button>
                <Button variant="ghost" size="icon-sm" id="discogs-search" class="icon search-toggle" data-i18n-title="online.search" title="Search" data-i18n-aria-label="online.search" aria-label="Search">
                  <svg class="glyph glyph-search" viewBox="0 0 16 16" aria-hidden="true"><circle cx="7" cy="7" r="4.3" fill="none" stroke="currentColor" stroke-width="1.5"/><line x1="10.3" y1="10.3" x2="14" y2="14" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>
                  <svg class="glyph glyph-stop" viewBox="0 0 16 16" aria-hidden="true"><rect x="4" y="4" width="8" height="8" rx="1" fill="currentColor"/></svg>
                </Button>
              </div>
            </div>
          </div>
          <div class="release-toolbar" id="release-toolbar" hidden>
            <span class="hint muted" id="release-found" data-i18n="online.foundNone">Found 0 entries</span>
          </div>
        </div>
        <div class="online-scroll">

          <div id="release-list" class="release-list"></div>
          <div class="online-empty" role="status">
            <svg class="ico online-empty-icon" aria-hidden="true"><use href="#i-search" /></svg>
            <h3 id="online-empty-heading" data-i18n="inspector.searchEmptyHeading">Find the right release</h3>
            <p id="discogs-empty" class="empty" data-i18n="online.empty">Search by artist, album or catalogue number.</p>
          </div>
          <div id="release-more" class="release-more" hidden>
            <span id="release-loading" class="muted" role="status" data-i18n="inspector.searchingHeading" hidden>Searching releases…</span>
            <Button variant="ghost" size="sm" id="load-more" type="button" data-i18n="online.retry" hidden>Retry</Button>
          </div>
        </div>

      </Tabs.Content>

      <Tabs.Content value="editor" id="subtab-editor" class="subtab-panel editor" hidden>
        <div class="editor-scroll">
          <div class="panel-block">
            <span id="fields-count" hidden></span>

            <div id="fields-body" class="fe"></div>

          </div>

          <div class="panel-block">
            <div id="cover-well"></div>

          </div>
        </div>

      </Tabs.Content>

      <Tabs.Content value="filename" id="subtab-filename" class="subtab-panel from-name" hidden>
        <div class="from-name-scroll"><div class="panel-block">
          <div class="mask-row">
            <div class="field">
              <Input id="from-name-mask" aria-label="Pattern" data-i18n-aria-label="inspector.pattern" type="text" value="%artist% - %title%" spellcheck="false" placeholder="%artist% - %title%" />
              <div class="field-tools">
                <Button variant="ghost" size="icon-sm" class="icon ph-btn" type="button" data-ph-target="from-name-mask" data-i18n-title="mask.placeholders" title="Placeholders" data-i18n-aria-label="mask.placeholdersAria" aria-label="Placeholder reference"><svg class="ico"><use href="#i-help"/></svg></Button>
                <Button variant="ghost" size="icon-sm" class="icon preset-btn" type="button" data-preset-target="from-name-mask" data-i18n-title="preset.title" title="Mask presets" data-i18n-aria-label="preset.title" aria-label="Mask presets"><svg class="ico"><use href="#i-presets"/></svg></Button>

                <Button variant="ghost" size="icon-sm" id="from-name-builder-open" class="icon" type="button" data-i18n-title="maskBuilder.open" title="Open the mask builder" data-i18n-aria-label="maskBuilder.open" aria-label="Open the mask builder"><svg class="ico"><use href="#i-builder"/></svg></Button>
              </div>
            </div>
          </div>

          <div id="from-name-probe" class="probe"></div>


        </div></div>
        <div class="panel-actions from-name-actions">
          <Button variant="default" size="sm" id="from-name-preview" class="primary" data-i18n="fromName.preview">Preview</Button>
        </div>
      </Tabs.Content>

      <Tabs.Content value="fromtext" id="subtab-fromtext" class="subtab-panel from-name" hidden>
        <div class="from-name-scroll"><div class="panel-block">
          <div class="mask-row">
            <div class="field">
              <Input id="from-text-mask" aria-label="Pattern" data-i18n-aria-label="inspector.pattern" type="text" value="%track%. %artist% - %title%" spellcheck="false" placeholder="%track%. %artist% - %title%" />
              <div class="field-tools">
                <Button variant="ghost" size="icon-sm" class="icon ph-btn" type="button" data-ph-target="from-text-mask" data-i18n-title="mask.placeholders" title="Placeholders" data-i18n-aria-label="mask.placeholdersAria" aria-label="Placeholder reference"><svg class="ico"><use href="#i-help"/></svg></Button>
                <Button variant="ghost" size="icon-sm" class="icon preset-btn" type="button" data-preset-target="from-text-mask" data-i18n-title="preset.title" title="Mask presets" data-i18n-aria-label="preset.title" aria-label="Mask presets"><svg class="ico"><use href="#i-presets"/></svg></Button>

                <Button variant="ghost" size="icon-sm" id="from-text-builder-open" class="icon" type="button" data-i18n-title="maskBuilder.open" title="Open the mask builder" data-i18n-aria-label="maskBuilder.open" aria-label="Open the mask builder"><svg class="ico"><use href="#i-builder"/></svg></Button>
              </div>
            </div>
          </div>
          <div class="from-text-input">
            <div class="from-text-head">
              <label for="from-text-lines" data-i18n="inspector.trackList">Track list</label>
              <Button variant="ghost" size="sm" id="from-text-load" type="button" data-i18n="fromText.load">Open file</Button>
            </div>
            <Textarea id="from-text-lines" class="from-text-lines" spellcheck="false" wrap="off" data-i18n-placeholder="fromText.placeholder" placeholder="Paste a tracklist, one line per track" />
            <div class="from-text-meta">
              <span id="from-text-count" class="muted" role="status"></span>
              <Button variant="ghost" size="sm" id="from-text-suggest" type="button" data-i18n="fromText.suggest" data-i18n-title="fromText.suggestTitle" title="Builds a pattern from the list layout.">Detect pattern</Button>
            </div>
          </div>
          <div id="from-text-probe" class="probe"></div>


        </div></div>
        <div class="panel-actions from-name-actions">
          <Button variant="default" size="sm" id="from-text-preview" class="primary" data-i18n="fromName.preview">Preview</Button>
        </div>
      </Tabs.Content>
    </Tabs.Root>
</Tabs.Content>

    <Tabs.Content value="generator" id="panel-generator" class="mode-panel" hidden>

      <Tabs.Root bind:value={generatorJob} class="generator-jobs-root" style="display: contents">
      <Tabs.List class="gen-jobs" aria-label="Generator job" data-i18n-aria-label="generator.jobs">
        <Tabs.Trigger value="transform" id="gen-job-transform">
          {#snippet child({ props })}<Button {...props} variant="ghost" size="sm" class="gen-job" data-i18n="generator.job.transform">Transform</Button>{/snippet}
        </Tabs.Trigger>
        <Tabs.Trigger value="number" id="gen-job-number">
          {#snippet child({ props })}<Button {...props} variant="ghost" size="sm" class="gen-job" data-i18n="generator.job.number">Number</Button>{/snippet}
        </Tabs.Trigger>
        <Tabs.Trigger value="sides" id="gen-job-sides">
          {#snippet child({ props })}<Button {...props} variant="ghost" size="sm" class="gen-job" data-i18n="generator.job.sides">Vinyl sides</Button>{/snippet}
        </Tabs.Trigger>
      </Tabs.List>

      <Tabs.Content value="transform" hidden={generatorJob !== 'transform'}><p class="hint muted">Choose transformations from the Tags sidebar.</p></Tabs.Content>

      <Tabs.Content value="number" class="panel-block" hidden={generatorJob !== 'number'}>
        <Button variant="outline" onclick={() => { const dialog = document.getElementById('ribbon-number-dialog'); if (dialog) dialog.hidden = false; }}>Open Autonumber</Button>
      </Tabs.Content>

      <Tabs.Content value="sides" class="panel-block" hidden={generatorJob !== 'sides'}>
        <h2><span data-i18n="generator.vinylSides">Vinyl sides</span> <span id="vinyl-count" class="muted"></span></h2>

        <div class="panel-actions">
          <p class="panel-note" data-i18n="generator.vinylHint" data-i18n-html>Side A becomes disc 1, side B disc 2. <b>A1</b> becomes track 1. Rows without a side letter stay as they are.</p>
          <Button variant="default" size="sm" id="vinyl-split" class="primary" data-i18n="generator.splitSide">Split</Button>
        </div>
      </Tabs.Content>
      </Tabs.Root>
    </Tabs.Content>

    <Tabs.Content value="deduplicator" id="panel-deduplicator" class="mode-panel" hidden>
      <div class="panel-block">
        <div class="duplicate-heading"><h2>Find duplicates</h2><DuplicateHelp /></div>
        {#if mode === 'deduplicator'}<DuplicateScanControls />{/if}
        <Button id="dup-scan" hidden>Scan</Button>
      </div>
    </Tabs.Content>

    <Tabs.Content value="exporter" id="panel-exporter" aria-labelledby="workspace-exporter-tab" class="mode-panel" hidden>
      <div class="panel-block">
        <h2><span data-i18n="exporter.heading">Export</span> <span id="export-count" class="muted"></span></h2>

        <div class="exp-format">
          <span class="exp-format-label" data-i18n="exporter.format">Format</span>
          <div class="exp-seg" id="export-kind" role="tablist" data-i18n-aria-label="exporter.format.aria" aria-label="Export format">
            <Button variant="ghost" size="sm" class="seg-btn active" type="button" data-fmt="playlist" data-i18n="exporter.format.playlist">Playlist</Button>
            <Button variant="ghost" size="sm" class="seg-btn" type="button" data-fmt="cue" data-i18n="exporter.format.cue">CUE</Button>
            <Button variant="ghost" size="sm" class="seg-btn" type="button" data-fmt="csv" data-i18n="exporter.format.csv">CSV</Button>
            <Button variant="ghost" size="sm" class="seg-btn" type="button" data-fmt="html" data-i18n="exporter.format.html">HTML</Button>
            <Button variant="ghost" size="sm" class="seg-btn" type="button" data-fmt="xml" data-i18n="exporter.format.xml">XML</Button>
            <Button variant="ghost" size="sm" class="seg-btn" type="button" data-fmt="report" data-i18n="exporter.format.report">Report</Button>
          </div>
          <p class="exp-format-hint" id="export-hint"></p>
        </div>

        <div class="exp-fields">

          <div class="exp-mask" id="export-split-row">
            <label for="export-split" data-i18n="exporter.split">One per</label>
            <NativeSelect size="sm" id="export-split">
              <NativeSelectOption value="" data-i18n="exporter.split.selection">Selection</NativeSelectOption>
              <NativeSelectOption value="folder" data-i18n="exporter.split.folder">Folder</NativeSelectOption>
              <NativeSelectOption value="album" data-i18n="exporter.split.album">Album</NativeSelectOption>
            </NativeSelect>
          </div>

          <div class="exp-mask" id="export-mask-row">
            <label for="export-mask" data-i18n="exporter.mask">Mask</label>

            <div class="field">
              <Input id="export-mask" type="text" value="%artist% - %title%" spellcheck="false" />
              <div class="field-tools">
                <Button variant="ghost" size="icon-sm" class="icon ph-btn" type="button" data-ph-target="export-mask" data-i18n-title="mask.placeholders" title="Placeholders" data-i18n-aria-label="mask.placeholdersAria" aria-label="Placeholder reference"><svg class="ico"><use href="#i-help"/></svg></Button>
                <Button variant="ghost" size="icon-sm" class="icon preset-btn" type="button" data-preset-target="export-mask" data-i18n-title="preset.title" title="Mask presets" data-i18n-aria-label="preset.title" aria-label="Mask presets"><svg class="ico"><use href="#i-presets"/></svg></Button>
              </div>
            </div>
          </div>
          <div class="exp-row">
            <label for="export-name" id="export-name-label">File name</label>
            <Input id="export-name" type="text" spellcheck="false" />
          </div>
        </div>

        <div class="panel-actions">
          <p class="exp-note panel-note" data-i18n="exporter.note" data-i18n-html><b>Read-only.</b> Writes into the opened library folder. Your audio files are not modified.</p>
          <Button variant="default" size="sm" id="export-run" class="primary" data-i18n="exporter.run">Export</Button>
        </div>
      </div>
    </Tabs.Content>


    <section id="staged-edits" class="staged-edits" tabindex="-1" aria-labelledby="staged-heading" hidden>
      <div class="staged-head"><div><h3 id="staged-heading" data-i18n="staging.heading">Staged edits</h3><span id="staged-count" class="staged-count"></span></div><Button id="diff-discard" variant="ghost" size="icon-sm" class="icon staged-remove" data-i18n-aria-label="staging.clear" aria-label="Clear staged edits" data-i18n-title="staging.clear" title="Clear staged edits"><svg class="ico"><use href="#i-close"/></svg></Button></div>
    </section>
  </aside>
  </DockModule>

  </DockWorkspace>

</main>

<div id="columns-menu" class="col-menu" hidden></div>

<input id="cover-file" type="file" accept="image/*" hidden />

<footer class="statusbar">

  <div class="sb-left">

    <CompactPlayer />
  </div>
  <div class="sb-right">
    <div id="diff-actionbar" class="diff-actionbar">
      <span class="ab-summary sr-only"><span id="ab-state">files ready</span><span class="ab-plan" id="ab-plan"></span></span>
      <Button variant="outline" size="sm" id="diff-preview" class="btn" disabled data-i18n="staging.preview">Preview</Button>
      <span class="ab-progress" id="ab-progress"><span class="ab-bar"><span class="ab-bar-fill" id="ab-bar-fill"></span></span><span class="ab-file" id="ab-file"></span></span>
      <Button variant="outline" size="sm" id="diff-cancel-apply" class="btn" data-i18n="action.cancel">Cancel</Button>
      <Button variant="default" size="sm" id="diff-apply" class="btn primary" disabled><span id="ab-apply-label">Apply changes</span><span id="ab-count" class="sr-only">0</span></Button>
    </div>

    <span id="busy-pill" class="busy-pill" hidden role="status"><span class="ab-bar"><span class="ab-bar-fill" id="busy-fill"></span></span><span id="busy-text"></span><Button variant="ghost" size="icon-sm" id="busy-stop" class="icon busy-stop" type="button" hidden data-i18n-title="action.stop" title="Stop" data-i18n-aria-label="action.stop" aria-label="Stop"><svg class="ico"><use href="#i-stop"/></svg></Button></span>
    <div class="footer-feedback">
      <span class="ab-locked" id="ab-locked" hidden></span>
      <span id="status-transform-scope" class="muted" hidden aria-live="polite"></span>
    </div>

  </div>
</footer>

</Tabs.Root>

</div>
