import { createNumberingActions } from './js/numbering-actions.js';
import { createFieldFormatActions } from './js/field-format-actions.js';
import { createLiveStageTransport } from './js/live-stage-transport.js';
import { nativeLiveFields, setNativeLiveFields } from './js/live-preview.js';
import { groupIcon } from './js/group-presentation.js';
import { liveIntentOptions, setNativeStageSession, isNativeTransformedTag } from './js/generator.js';
import { workingField, patchCellText, retireRevertedRawFields } from './js/working-values.js';
import { captureFileOrder, restoreFileOrder } from "./js/file-order.js";
import { interpretQueryText } from './js/query-language.js';
import { connectRibbonActions } from './js/ribbon-actions.js';
import { formatNumber } from './js/number-format.js';
import { indexedMovableDirections, movableDirections, shiftSelectedOrder } from './js/row-order.js';
import { createApplyFeedback } from './js/apply-feedback.js';
import { initStaging, renderStaging } from "./js/staging.js";
import { connectWorkspaceController, publishWorkspaceMode, publishWorkspaceSubtab } from './lib/workspace-state';
import { normalizeConditions, coverState } from "./js/filter-query.js";
import { emptyRoot, normalizeRoot, legacyRoot, normalizeQuery, queryLabel, registerQueryFields, scopedText, newCondition, defaultOperator } from "./js/query-model.js";
import { createLibraryQueryClient } from "./js/library-query-client.js";
import { createGroupMotion } from "./js/group-motion.js";
import { clearLiveInputs, hasLiveInputs, liveInputs, liveInputChanged, liveInputRevision, recordLiveField, recordLiveFields, hasInvalidLiveFields, hasInvalidPreview, removeAppliedLiveInputs, undoLiveImport, adoptLiveCoverPlan } from "./js/live-preview.js";
import { desiredPlanFingerprint, canAdoptAgentRebase } from './js/agent-write-guard.js';
import { createSharedStageSync } from './js/shared-stage.js';
"use strict";

import { TAURI, invoke } from "./js/invoke.js";
import { hydratePlan } from "./js/plan-codec.js";
import { createApplyReview } from './js/apply-review.js';
import {
  el,
  toast,
  fileName,
  folderName,
  escapeHtml,
  ico,
  confirmDialog,
  placeFloating,
  placeContextMenu,
  closePlanReport,
  dismissStickyToast,
  report,
  choiceDialog,
} from "./js/dom.js";
import { enablePointerReorder } from "./js/reorder.js";
import { initRowSelection } from "./js/row-selection.js";
import { initSpreadsheet } from "./js/spreadsheet.js";
import { RowHeights } from "./js/row-heights.js";
import { createComponentElement } from "./lib/component-elements.js";
import { refreshExporter, setExportKind } from "./js/exporters.js";
import { applyStaticText, message as backendMessage, planText, t as msg, tn, list as joinList } from "./js/i18n.js";
import { hooks } from "./js/hooks.js";
import { vinylPositionOf } from "./js/vinyl.js";
import { initLaunchOpen } from "./js/dragdrop.js";
import { readSession, saveSession, clearSession, beginResume, clearResumeAttempt } from "./js/session-resume.js";
import {
  beginApplyProgress,
  beginHistoryProgress,
  endApplyProgress,
  finishingWriteProgress,
  endHistoryProgress,
} from "./js/applyprogress.js";
import { initUpdateCheck } from "./js/updatecheck.js";
import "./js/tablegestures.js";
import { scheduleMaskExample, syncCarryToggles } from "./js/renamer.js";
import "./js/maskbuilder.js";
import { previewTagsFromName, refreshNameProbe, scheduleNameProbe } from "./js/fromname.js";
import { refreshTextProbe, scheduleTextProbe } from "./js/fromtext.js";
import { createDuplicateJobs } from "./js/duplicate-jobs.js";
import { duplicateGroupTitle, duplicateSelectionDetail } from "./js/duplicate-selection.js";
import {
  chooseCover,
  embedCoverFile,
  embedCoverFromPath,
  embedExternalCover,
  exportCover,
  onCoverChosen,
  previewClearTags,
  previewCoverRemove,
  refreshCoverWell,
  renderPendingCoverSummary,
} from "./js/cover.js";
import {
  allColumnKeys,
  tableColumnKeys, setReviewColumns, syncTableColumns,
  columnLabel,
  columnWidth,
  customColumnOf,
  customColumnValue,
  invalidateCustomColumns,
  loadColumnWidths,
  loadColumns,
  loadCustomColumns,
  maybeAutofit,
  refreshCustomColumnCells,
  refreshCustomColumnPath,
  populateGroupMenu,
  renderColumnsMenu,
  renderTableHead,
  setGroupBy,
} from "./js/columns.js";
import { dropGroupKey, folderGroupLabel, groupKeyOf, groupLabel } from "./js/grouping.js";
import {
  loadSavedToken,
  refreshReleaseMatches,
  retranslateOnline,
  searchBusy,
  stopLoading,
} from "./js/online.js";
import {
  numberTracks, numberingEdits, refreshGenerator, syncTransformPlacement, adoptTagOnlyPreview, adoptSharedPreview, isAutomaticTransformPreview, isAutomaticTransformRefresh,
  automaticTransformScope, scheduleAutomaticTransformPreview,
  dismissAutomaticTransformPreview, pauseAutomaticTransformPreview,
  resumeAutomaticTransformPreview, invalidateAutomaticTransformPreview,
} from "./js/generator.js";
import { initActionGroups, initBuiltinGroups, renderAllGroupsMenus } from "./js/chain.js";
import { migrateChains } from "./js/chains.js";
import { EXTENDED_FIELDS, KNOWN_CUSTOM_LABELS, VIRTUAL_COLUMNS, fieldLabel } from "./js/fields.js";
import { initPlaceholderReference } from "./js/placeholders.js";
import { initMaskPresets } from "./js/maskpresets.js";
import { initTooltips } from "./js/tooltip.js";
import { initWorkbenchMotion } from "./js/workbench-motion.js";
import { comboOf, formatCombo, initShortcuts } from "./js/shortcuts.js";
import { beginBackgroundActivity } from './js/staged-activity.js';
import { bpmRunning, invalidateBpm, retranslateBpm, stopBpm } from "./js/bpm.js";
import { isFieldLocked, loadFieldLocks, pushFieldLocks } from "./js/locks.js";
import {
  cellSuggestKey,
  endCellSuggest,
  openCellSuggest,
  refreshCellSuggest,
} from "./js/suggest.js";
import {
  currentFieldValue,
  previewRemoveTagBlock,
  refreshFieldEditor,
  refreshSharedStagedFields,
  refreshTagFormats,
  resetFieldEditor,
  validateFieldValue,
} from "./js/editor.js";

// Hand the table-side refreshers to the panels that were split out (#143); see
// js/hooks.js for why this seam exists and when it goes away.
Object.assign(hooks, {
  renderTracks,
  renderPreview,
  previewEdits,
  refreshCoverWell,
  refreshLibrary,
  openDrop,
  updateSortIndicators,
  mountMeasureRows,
  renderTableHead,
  revealPath,
  navigablePaths: () => navPaths,
  orderPathsForDisplay,
  orderSelectedFiles,
  // A language change repaints the static markup itself; the open panel builds
  // some of its own text, so it is asked to draw again (#50). The library
  // indicator is dynamic too (#287) — repaint it so an open folder's name and
  // path survive the language switch instead of being left on the placeholder.
  // The table's header row and the Group by menu are built from the field
  // catalogue at start-up, so they are rebuilt too (#423); the column picker
  // builds itself each time it opens.
  retranslate: () => {
    (MODE_REFRESH[currentMode] || (() => {}))();
    renderRootDisplay();
    renderTableHead();
    populateGroupMenu();
    // What is built from `t` at draw time stays in the old language until it is
    // drawn again (#476): the table (folder bands), the status line, the release
    // cards.
    syncCssLabels();
    syncFilterControls();
    renderTracks();
    updateStatus();
    retranslateOnline();
    retranslateBpm();
    renderSettingsNav();
  },
});
import { openSettings, cancelSettings, updateSettingsDot } from "./js/settings.js";
import { selectSettingsGroup, renderSettingsNav } from "./js/settingsui.js";
import {
  valueFont,
  applyValueFont,
  applyTheme,
  applyThemePreset,
  themeMode,
  paletteMode,
  checkboxColEnabled,
  applyCheckboxCol,
  tableFontPx,
  applyTableFont,
  tracklistFontPx,
  applyTracklistFont,
  badgeFont,
  applyBadgeFont,
  groupByPref,
  saveGroupBy,
} from "./js/prefs.js";
import {
  fmtTime,
  showPlayerBar,
  playTrack,
  isPlayingPath,
} from "./js/player.js";
import {
  selectionVersion, captureSelection, isSelectionCurrent,
  tracks, setTracks, reindexTracks, invalidateTrackContent, contentRevision,
  previewPlan, setPreviewPlan, resolvedPreviewPlan, setPreviewDecorator, withoutPreviewPublication, setSharedStagingEnabled,
  previewSource, setPreviewSource,
  pendingImportCover, setPendingImportCover,
  diffByPath, setDiffByPath,
  applySelection, setApplySelection,
  edits, selection, selectedPaths, tag, trackAt, trackByPath,
  DEFAULT_COLUMNS, TABLE_DATA_COLUMN_OFFSET, visibleColumns, setVisibleColumns,
  columnWidths, setColumnWidths,
  sortKey, setSortKey, sortDir, setSortDir,
  manualOrder, setManualOrder,
  filterText, setFilterText,
  filterError, setFilterError,
  filterQuery, setFilterQuery,
  groupBy, setGroupByValue, collapsedGroups,
  dropFolders, setDropFolders,
  sessionRoot, setSessionRoot,
  currentMode, setCurrentMode,
  activeRowPath, setActiveRowPath,
  savedSettings, setSavedSettings,
  actionGroups, setActionGroups,
  setMaskPresets,
  builtinGroups, setBuiltinGroups,
} from "./js/state.js";

// Overlay title bar (#362, macOS only): tauri.conf.json's titleBarStyle
// "Overlay" floats the native traffic lights over the page instead of
// giving them their own strip, so the top bar needs room reserved for
// them. Only inside that actual window — the browser-only mock (mock.js)
// has no traffic lights to clear, and other platforms keep their normal
// decorated title bar regardless of the config (Tauri ignores this style
// there), so there is nothing to clear for them either.
if (TAURI && /Mac/.test(navigator.platform)) {
  document.body.classList.add("mac-overlay-titlebar");
}

// The in-app windows close where the platform closes (#565): at the start of
// their heads on macOS, on the traffic-light side; at the end on Windows; on
// Linux where the desktop's own button layout puts close, the end when it
// can't be read. CSS keys only on `close-start`.
const PLATFORM = /Mac/.test(navigator.platform) ? "mac" : /Win/.test(navigator.platform) ? "win" : "linux";
document.body.classList.add(`platform-${PLATFORM}`);
if (PLATFORM === "mac") document.body.classList.add("close-start");
if (PLATFORM === "linux" && TAURI) {
  invoke("close_button_side")
    .then((side) => document.body.classList.toggle("close-start", side === "start"))
    .catch(() => {});
}

// ---- elements ----
const rootInput = el("root");
const tableWrap = el("files-view");
const tracksBody = el("tracks-body");
const tracksEmpty = el("tracks-empty");
let scrolledCellEdit = null;
let stagedView = false;
let stagedMembershipPending = false;
let activeApplyReview = null;
let excludedApplyPaths = new Set();
const fileViewScroll = { files: { top:0, left:0 }, staged: { top:0, left:0 } };
function editablePreview() {
  return true;
}
function setFileView(staged) {
  if (applying || undoing) return;
  if (stagedView === staged) return;
  scrolledCellEdit = null;
  const active = tracksBody.querySelector('[contenteditable="true"]');
  if (active) active.blur();
  fileViewScroll[stagedView ? 'staged' : 'files'] = { top:tableWrap.scrollTop, left:tableWrap.scrollLeft };
  stagedView = staged;
  stagedMembershipPending = false;
  syncFileView();
  renderTracks({ preserveDuplicateRows: true });
  patchMountedFields();
  const scroll = fileViewScroll[stagedView ? 'staged' : 'files'];
  tableWrap.scrollTop = scroll.top; tableWrap.scrollLeft = scroll.left;
  renderWindow({force:true, preserveDuplicateRows:true});
  document.dispatchEvent(new CustomEvent('tagrex:reveal-module',{detail:'files'}));
}
function syncFileView() {
  if (setReviewColumns(stagedView)) stagedMembershipPending = true;
  const columnsChanged = syncTableColumns();
  document.body.classList.toggle('staged-view',stagedView);
  el('tracks').classList.toggle('show-old',stagedView);
  for (const [id,on] of [['files-tab',!stagedView],['staged-tab',stagedView]]) {
    const tab=el(id); tab.setAttribute('aria-selected',String(on)); tab.tabIndex=on?0:-1;
  }
  tableWrap.setAttribute('aria-labelledby',stagedView?'staged-tab':'files-tab');
  tracksEmpty.textContent = stagedView ? msg('staging.empty') : msg('table.empty');
  return columnsChanged;
}
const applyBtn = el("diff-apply");
const undoBtn = el("undo");
const redoBtn = el("redo");
const selectAll = el("select-all");
const statusSel = el("status-sel");
let spreadsheet = null;
let groupMotion = null;
const sortAnimations = new Set();
const sortReducedMotion = matchMedia('(prefers-reduced-motion: reduce)');

let sortScrollTop = 0;
let sortScrollLeft = 0;
// Client-only operation context. Never serialize it into the backend plan or
// replace its composed description, which is also the journal fallback text.
let spreadsheetBatchKind = "";

// An open intent owns every awaited result. An OS hand-over or manual open
// supersedes startup resume before a late result can repaint another subject.
let openVersion = 0;
let resumableRoot = "";
let sessionSaveTimer = null;
let restoringView = false;
function sessionStorage() { try { return localStorage; } catch { return null; } }
function sessionSnapshot() {
  return {
    version: 1, root: resumableRoot,
    mode: currentMode,
    subtab: document.querySelector('.subtab-panel:not([hidden])')?.id.replace('subtab-', ''),
    filter: filterText, queryRoot: filterRoot, commandMode, duplicateKind, conditions: [], conditionMode: filterConditionMode,
    sortKey, sortDir, groupBy,
    collapsed: [...collapsedGroups], selection: [...selection],
    scrollTop: tableWrap.scrollTop, scrollLeft: tableWrap.scrollLeft,
  };
}
function flushSessionView() {
  clearTimeout(sessionSaveTimer);
  if (resumableRoot && !restoringView) saveSession(sessionStorage(), sessionSnapshot());
}
function scheduleSessionSave() {
  if (!resumableRoot || restoringView) return;
  clearTimeout(sessionSaveTimer);
  sessionSaveTimer = setTimeout(flushSessionView, 250);
}
// Only bounded view state is captured, once after a burst, never on each row
// in a marquee or each scroll frame. No drafts or write plans enter storage.
window.addEventListener('pagehide', flushSessionView);
document.addEventListener('visibilitychange', () => { if (document.hidden) flushSessionView(); });
tableWrap.addEventListener('scroll', scheduleSessionSave, { passive: true });
document.addEventListener('click', scheduleSessionSave);
document.addEventListener('input', scheduleSessionSave);
document.addEventListener('keydown', scheduleSessionSave);

// Remember an inner popup's Escape in capture phase: its content may already
// have unmounted by the time a document-level Settings/Discard handler runs.
const componentEscapeEvents = new WeakSet();
function componentOwnsEscape(event) {
  if (componentEscapeEvents.has(event)) return true;
  const target = event.target instanceof Element ? event.target : null;
  const owned = !!target?.closest("[data-controller-popup],[data-search-editor],[data-artwork-ui]") ||
    [...document.querySelectorAll('[data-slot="select-content"], [data-slot="popover-content"], [data-slot="dropdown-menu-content"]')]
      .some((node) => !node.matches('[hidden], [data-state="closed"], [data-closed]') && node.getClientRects().length > 0);
  if (owned) componentEscapeEvents.add(event);
  return owned;
}

// ---- helpers ----

// ---- rendering ----
// Renders the table, overlaying any pending edits on top of the on-disk
// values (edited cells shown and marked dirty). Does NOT clear `edits` — call
// `resetEdits()` for that when loading fresh disk state.
// The text of a read-only derived column — the vinyl position (#106) or the
// playing time (#172) — or null when the column is an ordinary tag. One place,
// so the row renderer, the diff renderer, sorting and filtering cannot drift on
// what a derived cell says.
const technicalColumnProperties = {
  path:'path', size:'size_bytes', bitrate:'bitrate_kbps', channels:'channels',
  sample_rate:'sample_rate_hz', format:'format', modified:'modified',
};
function virtualCellText(track, key, pending) {
  if (key === "position") return vinylPositionOf(track, new Map(['media','disc','track'].map(field => [field, workingField(track.path, field).value])));
  if (key === "length") return track.duration_secs == null ? "" : fmtTime(track.duration_secs);
  if (key === "tagtypes") return tagBlockSummary(track);
  if (Object.hasOwn(technicalColumnProperties, key)) {
    const value = track[technicalColumnProperties[key]];
    if (value == null) return "";
    if (key === "size") return `${(value / (1024 * 1024)).toFixed(2)} MiB`;
    return String(value);
  }
  return null;
}

// The Tag types cell (#47): every block the file carries, the one being read
// first. That order is the point — the rest are the file's other answers, and
// seeing them is how "the edit didn't take" becomes "you are reading the other
// block".
function tagBlockSummary(track) {
  const blocks = track.tag_blocks || [];
  if (!blocks.length) return "";
  const read = blocks.filter((b) => b.read_from).map((b) => b.label);
  const rest = blocks.filter((b) => !b.read_from).map((b) => b.label);
  return [...read, ...rest].join(" + ");
}

function sortValue(track, key) {
  if (key === "artwork") {
    const state = coverState(track);
    return state === "unknown" ? "2" : state === "embedded" ? "1" : "0";
  }
  // Length sorts on the seconds, not on "9:59" vs "10:01" as text.
  if (key === "length") return String(track.duration_secs ?? -1).padStart(9, "0");
  // Position sorts by (disc, track) numerically, so A1, A2, … B1 order holds
  // (and plain CD tracks sort numerically too) instead of by the display string.
  if (key === "position") {
    const disc = parseInt(track.tags.disc || "0", 10) || 0;
    const trk = parseInt(track.tags.track || "0", 10) || 0;
    return String(disc).padStart(4, "0") + String(trk).padStart(5, "0");
  }
  // Files carrying a second block sort together, ahead of the plain ones —
  // which is the way you would go looking for them (#47).
  if (key === "tagtypes") {
    const blocks = track.tag_blocks || [];
    return String(9 - Math.min(blocks.length, 9)) + tagBlockSummary(track).toLowerCase();
  }
  if (Object.hasOwn(technicalColumnProperties, key)) {
    return String(track[technicalColumnProperties[key]] ?? "").toLowerCase();
  }
  // A mask column sorts on what it renders, which is what the eye compares.
  if (customColumnOf(key)) return customColumnValue(key, track.path).toLowerCase();
  return (key === "file" ? fileName(track.path) : track.tags[key] || "").toLowerCase();
}

// The display value of one column for a track — the same source the columns and
// sort use, so a field-scoped filter (`position:B1`) sees what the eye sees.
function fieldValue(track, key) {
  if (key === "file") return fileName(track.path);
  const derived = virtualCellText(track, key, edits.get(track.path));
  if (derived !== null) return derived;
  if (customColumnOf(key)) return customColumnValue(key, track.path);
  return formatNumber(track.tags[key] || "", key);
}

// Compile one immutable query per edit; retain the previous predicate on error.
// Existing display values and cached free text stay in the table controller.
const numericCollator = new Intl.Collator(undefined, { numeric: true });
const trackSearchText = new WeakMap();
let filterRenderVersion = 0;
let filterConditions = [];
let filterConditionMode = 'all';
let filterPending = false;
let filterMessage = '';

let filterRoot = emptyRoot();
let commandMode = 'search';
let duplicateKind = 'exact';
let duplicateMessage = '';
let duplicateScanning=false;
let duplicateStatus='not_scanned';
let duplicateFoundCount=0;
let duplicateLatestRevision=0;
let duplicateRenderedRevision=0;
let duplicateRefreshPending=false;
let duplicateQueryKey='';
let duplicateGroupOrder=new Map();
let duplicateGroupLookup=new Map();
let duplicateGroups = [];
let duplicateScanStarted=false;
let selectedDuplicateGroupId=null;
let duplicateSelectionCache=null;
let duplicateSelectionRevision=-1;
let nativeMatchPaths = null;
let nativeMatchRevision = -1;
let scheduledQueryRevision = -1;
let queryRetry=0;
const libraryQueryClient = createLibraryQueryClient(invoke);
function resetDuplicatePublications(){duplicateLatestRevision=0;duplicateRenderedRevision=0;duplicateRefreshPending=false;duplicateQueryKey='';duplicateGroupOrder.clear();clearDuplicateSelection();}
function duplicateResultsActive(){return commandMode==='duplicates'&&duplicateScanStarted;}
function clearDuplicateSelection(){
  const hadSelection=selectedDuplicateGroupId!=null||duplicateSelectionCache!=null;
  selectedDuplicateGroupId=null;duplicateSelectionCache=null;duplicateSelectionRevision=-1;
  if(hadSelection)document.dispatchEvent(new CustomEvent('tagrex:duplicate-selection',{detail:{group:null}}));
}
function selectionMatchesDuplicate(group){return group.paths.length===selection.size&&group.paths.every(path=>selection.has(path));}
function publishDuplicateSelection(force=false){
  const group=duplicateResultsActive()&&selectedDuplicateGroupId!=null?duplicateGroupLookup.get(`duplicate:${selectedDuplicateGroupId}`):null;
  if(!group||!selectionMatchesDuplicate(group)){
    const hadSelection=selectedDuplicateGroupId!=null||duplicateSelectionCache!=null;
    clearDuplicateSelection();
    if(force&&!hadSelection)document.dispatchEvent(new CustomEvent('tagrex:duplicate-selection',{detail:{group:null}}));
    return;
  }
  if(force||duplicateSelectionCache?.source!==group||duplicateSelectionRevision!==contentRevision){
    duplicateSelectionCache={source:group,detail:duplicateSelectionDetail(group,trackAt)};
    duplicateSelectionRevision=contentRevision;
    document.dispatchEvent(new CustomEvent('tagrex:duplicate-selection',{detail:{group:duplicateSelectionCache.detail}}));
  }
}
function reconcileDuplicateSelection(groups){
  if(selectedDuplicateGroupId==null)return;
  const previous=duplicateGroupLookup.get(`duplicate:${selectedDuplicateGroupId}`);
  const next=groups.find(group=>group.id===selectedDuplicateGroupId);
  if(!next||!previous||!selectionMatchesDuplicate(previous)){clearDuplicateSelection();return;}
  // Selection belongs to this whole group, so newly verified copies join it.
  if(!selectionMatchesDuplicate(next)){selection.clear();for(const path of next.paths)selection.add(path);}
}
document.addEventListener('tagrex:request-duplicate-selection',()=>publishDuplicateSelection(true));
function refreshDuplicatePublications(){
  if(commandMode!=='duplicates'||filterError||duplicateLatestRevision<=duplicateRenderedRevision)return;
  if(filterPending){duplicateRefreshPending=true;return;}
  scheduleFilterRender();
}
function orderedDuplicateGroups(groups){
  for(const group of groups)if(!duplicateGroupOrder.has(group.id))duplicateGroupOrder.set(group.id,duplicateGroupOrder.size);
  if(groups.every((group,i)=>!i||duplicateGroupOrder.get(groups[i-1].id)<=duplicateGroupOrder.get(group.id)))return groups;
  return [...groups].sort((a,b)=>duplicateGroupOrder.get(a.id)-duplicateGroupOrder.get(b.id));
}
function hasLibraryQuery() { return commandMode==='duplicates'?duplicateScanStarted:!!filterText||!!filterRoot.children.length; }
function filterModel(extraField=null) {
  const keys=allColumnKeys();registerQueryFields(keys.map(key=>({key,label:columnLabel(key)})));
  const used=new Set(extraField?[extraField]:[]);
  const visit=node=>{if(node.kind==='group')node.children.forEach(visit);else used.add(node.field);};visit(filterRoot);
  const scoped=scopedText(filterText);if(scoped)used.add(scoped.field);
  return {...normalizeQuery({text:filterText,root:filterRoot,commandMode,duplicateKind}),customFields:[...used].flatMap(key=>{const field=customColumnOf(key);return field?[{key,mask:field.pattern}]:[];})};
}
function publishFilterState() {
  document.dispatchEvent(new CustomEvent('tagrex:filter-state', { detail: {
    // Feedback must also work for invalid input. Revalidating here would throw
    // before the error could be shown or Apply disabled.
    text:filterText, root:filterRoot, mode:commandMode,
    commandMode, duplicateKind, duplicateMessage, duplicateScanning, duplicateStatus, duplicateScanStarted,
    conditions:filterConditions, pending:filterPending, error:filterMessage,
    matched:matchingTrackCount, total:tracks.length,
    hidden:[...selection].filter(path=>!viewPathIndex.has(path)).length,
  } }));
}
function recompileFilter() {
  try {
    const model=filterModel();
    filterMessage='';queryRetry=0;setFilterError(false);setFilterQuery({model,match:null});
  } catch(error) {filterMessage=error.message;setFilterError(true);}
}
function matchesFilter(track) {
  return !hasLibraryQuery() || !nativeMatchPaths || nativeMatchPaths.has(track.path);
}

function updateSortIndicators() {
  document.querySelectorAll('[data-artwork-sort]').forEach(button => {
    button.setAttribute('aria-pressed', String(sortKey === 'artwork'));
    button.title = msg(sortKey === 'artwork' ? (sortDir > 0 ? 'table.artwork.missingFirst' : 'table.artwork.embeddedFirst') : 'table.artwork.sort');
  });
  document.querySelectorAll("th.sortable").forEach((th) => {
    const ind = th.querySelector(".sort-ind");
    const sorted = th.dataset.sort === sortKey;
    const direction = sorted ? (manualOrder ? 'other' : sortDir > 0 ? 'ascending' : 'descending') : null;
    if (th.getAttribute('aria-sort') !== direction) {
      if (direction) th.setAttribute('aria-sort', direction);
      else th.removeAttribute('aria-sort');
    }
    ind.classList.toggle('descending', sorted && sortDir < 0);
    // A hand-made order (a drag, a match) leaves the sort in force for
    // everything it didn't touch (#198). The indicator fades rather than
    // vanishing, so the column still says what orders the list while admitting
    // that some rows are where they were put.
    ind.classList.toggle("manual", sorted && manualOrder);
    const scope = sorted && groupBy && !duplicateResultsActive() ? msg('table.sortWithinGroups') : '';
    ind.title = sorted && manualOrder ? msg("app.reorderedByHand") : scope;
    if (scope) th.setAttribute('aria-description', scope);
    else th.removeAttribute('aria-description');
  });
}

// ---- filter mode + saved presets (#44) ----
// Revalidate query rules and reflect the input/error state before rendering.
function syncFilterControls() {
  recompileFilter();
  const box = el("filter");
  // The error and the empty state belong to the field around the box (#547):
  // the red border, and a clear button only when there is text to clear.
  box.parentElement.classList.toggle("has-error", filterError);
  box.parentElement.classList.toggle("is-empty", !box.value);
  box.title = filterMessage;
  box.setAttribute('aria-invalid', String(filterError));
  box.setAttribute('aria-describedby', 'filter-feedback');
  publishFilterState();
  updateDiffBar();
}

// A named preset captures the full view query: filter text and rules, the
// sort column/direction, and the grouping — everything the user tuned by hand.
// Older UIs must not see a structured preset and silently ignore its conditions.
const PRESETS_STORAGE_KEY = "tagrex.filterPresets";
const legacyConditionPresetsKey = `${PRESETS_STORAGE_KEY}.conditions.v1`;
const conditionPresetsKey = `${PRESETS_STORAGE_KEY}.conditions.v2`;
let nativeQueryPresets = null;
let queryPresetSave = Promise.resolve();
function loadPresets() {
  if (nativeQueryPresets) return nativeQueryPresets.slice();
  const byName = new Map();
  const stores=localStorage.getItem(conditionPresetsKey)!==null?[conditionPresetsKey]:[PRESETS_STORAGE_KEY,legacyConditionPresetsKey];
  for (const key of stores) {
    try {
      const list = JSON.parse(localStorage.getItem(key) || '[]');
      if (Array.isArray(list)) for (const preset of list) {
        if (preset && typeof preset.name === 'string') byName.set(preset.name, preset);
      }
    } catch { /* A damaged or unavailable optional store does not block the other. */ }
  }
  return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name));
}
function savePresets(list) {
  const before = loadPresets();
  try {
    const converted=list.map(p=>({...p,queryVersion:2,root:p.root?normalizeRoot(p.root):legacyRoot(normalizeConditions(p.conditions),p.conditionMode||'all'),conditions:[]}));
    localStorage.setItem(conditionPresetsKey,JSON.stringify(converted));
    if (TAURI) {
      nativeQueryPresets = converted;
      const upsert = converted.filter(preset => JSON.stringify(preset) !== JSON.stringify(before.find(old => old.name === preset.name)));
      const remove = before.filter(old => !converted.some(preset => preset.name === old.name)).map(preset => preset.name);
      queryPresetSave = queryPresetSave.then(async () => {
        const settings = await invoke('mcp_patch_query_presets', { upsert, remove });
        acceptSharedSettings(settings);
      }).catch(async error => {
        toast(String(error), true);
        try { acceptSharedSettings(await invoke('load_settings', {})); } catch { /* Keep the recoverable local copy. */ }
      });
    }
  } catch { /* Keep the original stores recoverable when validation/storage fails. */ }
}

function acceptSharedSettings(settings) {
  setSavedSettings(settings);
  setActionGroups(Array.isArray(settings.action_groups) ? settings.action_groups : []);
  setMaskPresets(Array.isArray(settings.mask_presets) ? settings.mask_presets : []);
  nativeQueryPresets = Array.isArray(settings.query_presets) ? settings.query_presets : [];
  try { localStorage.setItem(conditionPresetsKey, JSON.stringify(nativeQueryPresets)); } catch { /* Optional view cache. */ }
  renderPresetsMenu();
  renderAllGroupsMenus();
  document.dispatchEvent(new CustomEvent('tagrex:shared-presets-changed'));
}

document.addEventListener('tagrex:settings-saved', event => acceptSharedSettings(event.detail));

// Store the current view under `name` (replacing a same-named one).
function saveCurrentPreset(name) {
  name = name.trim();
  if (!name) return;
  if (filterError) { toast('Fix the query before saving a preset.', true); return false; }
  const list = loadPresets().filter((p) => p.name !== name);
  list.push({
    name,
    queryVersion: 2, root: filterRoot, commandMode, duplicateKind, conditions: [], conditionMode: filterRoot.mode,
    filter: filterText,
    sortKey,
    sortDir,
    group: groupBy,
  });
  list.sort((a, b) => a.name.localeCompare(b.name));
  savePresets(list);
  renderPresetsMenu();
}

// Re-apply a saved preset: restore the filter text/flags, sort, and grouping,
// then reflect them onto every control and repaint.
function applyPreset(p) {
  try {
    if ((p.queryVersion != null && ![1,2].includes(p.queryVersion)) || (p.conditionMode != null && !['all', 'any'].includes(p.conditionMode))) throw new Error('Unsupported saved filter version.');
    filterRoot = p.queryVersion === 2 ? normalizeRoot(p.root) : legacyRoot(normalizeConditions(p.conditions), p.conditionMode === 'any' ? 'any' : 'all');
    filterConditions = filterRoot.children;filterConditionMode=filterRoot.mode;
    const nextMode=p.commandMode==='duplicates'?'duplicates':'search';
    const nextKind=['exact','same_audio','likely'].includes(p.duplicateKind)?p.duplicateKind:'exact';
    if(nextMode!==commandMode||nextKind!==duplicateKind){
      if(commandMode==='duplicates'){duplicateJobs.cancel();invalidateDuplicateScan();}
      duplicateGroups=[];duplicateMessage='';duplicateScanStarted=false;duplicateScanning=false;duplicateStatus='not_scanned';duplicateFoundCount=0;resetDuplicatePublications();
    }
    commandMode=nextMode;duplicateKind=nextKind;
  } catch (error) { toast(error.message, true); return; }
  setFilterText(p.filter || "");
  el("filter").value = filterText;
  syncFilterControls();

  setGroupBy(p.group || "", { rerender: false });

  if (p.sortKey) applySort(p.sortKey, p.sortDir === -1 ? -1 : 1);
  else renderTracks();
  setMode(commandMode==='duplicates'?'deduplicator':currentMode==='deduplicator'?'tagger':currentMode);
  scheduleFilterRender();
}

let presetsMenuKey = null;
function renderPresetsMenu() {
  const menu = el("presets-menu");
  const list = loadPresets();
  const key = JSON.stringify([list, msg("app.noPresets"), msg("app.saveCurrentAs"), msg("action.save")]);
  if (presetsMenuKey === key && menu.childNodes.length) return;
  presetsMenuKey = key;
  menu.innerHTML = "";
  if (!list.length) {
    const empty = document.createElement("div");
    empty.className = "col-menu-sep";
    empty.textContent = msg("app.noPresets");
    menu.appendChild(empty);
  }
  for (const p of list) {
    const row = document.createElement("div");
    row.className = "col-menu-row preset-row";

    const apply = createComponentElement("button");
    apply.type = "button";
    apply.className = "text-btn preset-apply";
    apply.textContent = p.name;
    apply.title = presetSummary(p);
    apply.addEventListener("click", () => {
      applyPreset(p);
      menu.hidden = true;
    });

    const del = createComponentElement("button", { size: "icon-sm" });
    del.type = "button";
    del.className = "preset-del";
    del.innerHTML = ico("close");
    del.title = msg("app.deletePreset", { name: p.name });
    del.addEventListener("click", (e) => {
      e.stopPropagation();
      savePresets(loadPresets().filter((q) => q.name !== p.name));
      renderPresetsMenu();
    });

    row.append(apply, del);
    menu.appendChild(row);
  }

  // Save-current footer: name the current filter + sort and store it.
  const foot = document.createElement("div");
  foot.className = "col-menu-foot preset-save";
  const input = createComponentElement("input");
  input.type = "text";
  input.placeholder = msg("app.saveCurrentAs");
  input.spellcheck = false;
  input.className = "preset-name";
  const save = createComponentElement("button");
  save.type = "button";
  // The same Save as the mask presets' (#592): a button, not a link.
  save.className = "btn";
  save.textContent = msg("action.save");
  const commit = () => {
    if (input.value.trim()) {
      if (saveCurrentPreset(input.value) === false) return;
      input.value = "";
      menu.hidden = true;
    }
  };
  save.addEventListener("click", commit);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
    }
  });
  foot.append(input, save);
  menu.appendChild(foot);
}

// A human-readable one-liner for a preset's tooltip.
function presetSummary(p) {
  const bits = [];
  try { bits.push(...(p.root ? normalizeRoot(p.root) : legacyRoot(normalizeConditions(p.conditions),p.conditionMode||'all')).children.map(queryLabel)); } catch { bits.push('Invalid conditions'); }
  if(p.commandMode==='duplicates') bits.push(`Duplicates: ${p.duplicateKind||'exact'}`);
  if (p.filter) {
    bits.push(
      msg("app.presetFilter", { filter: p.filter }),
    );
  }
  if (p.sortKey) {
    bits.push(msg("app.presetSort", { column: columnLabel(p.sortKey), dir: p.sortDir === -1 ? "↓" : "↑" }));
  }
  if (p.group) bits.push(msg("app.presetGroup", { group: p.group }));
  return bits.length ? bits.join(" · ") : msg("app.presetEmpty");
}

// Build one track row. `groupKey` (when grouping) tags the row so a click on it
// can tell which folder it belongs to. Returned rather than appended: the window
// renderer (#189) decides which rows exist at all, and a single row is also
// rebuilt in place when only its own contents changed (#186).
// Row DOM contract shared with columns.js and js/spreadsheet.js:
// - data-path identifies a track; renderedRows owns only the virtual window.
// - cells[0] is selection (also when hidden); cells[1] is artwork status.
// - cells[TABLE_DATA_COLUMN_OFFSET + i] matches visibleColumns[i]; file stays
//   first in that logical list. Artwork is structural, never an editable range.
// - writable cells carry data-path/field/original and .editable; derived/mask
//   columns are read-only. Unreadable rows are inert and may span columns.
// - group/spacer rows are structural, never spreadsheet cell targets.
// - the legacy controller owns row selection and direct edit commits; the
//   spreadsheet owns cell focus/ranges and reapplies them after window paints.
// Keep this order/metadata when changing the renderer; never infer library
// membership or edit scope from the currently mounted DOM rows.
function decorateTrackArtwork(row, track) {
  row._columnKeys = [...tableColumnKeys()];
  const file = row.querySelector('td.file');
  if (!file) return row;
  row.querySelector('td.sel')?.setAttribute('aria-colindex', '1');
  file.setAttribute('aria-colindex', String(TABLE_DATA_COLUMN_OFFSET + 1));
  const stagedCover = diffByPath?.get(track.path)?.cover_change;
  const state = stagedCover && !stagedCover.invalid
    ? (stagedCover.new.length ? 'embedded' : 'missing')
    : coverState(track);
  const indicator = document.createElement('span');
  indicator.className = 'track-artwork';
  indicator.dataset.coverState = state;
  indicator.setAttribute('role', 'img');
  indicator.setAttribute('aria-label', msg(`table.artwork.${state}`));
  indicator.title = msg(`table.artwork.${state}`);
  if (stagedCover && !stagedCover.invalid) {
    const operation = !stagedCover.new.length ? 'removed' : !stagedCover.old.length ? 'added' : 'replaced';
    indicator.classList.add('artwork-changing');
    indicator.dataset.coverChange = operation;
    indicator.title = msg(`table.artwork.${operation}`);
    indicator.setAttribute('aria-label', indicator.title);
  }
  indicator.innerHTML = ico('image');
  const cell = document.createElement('td');
  cell.className = 'artwork-col';
  cell.setAttribute('role', 'gridcell');
  cell.setAttribute('aria-colindex', '2');
  cell.setAttribute('aria-readonly', 'true');
  cell.append(indicator);
  file.before(cell);
  const fill = document.createElement('td');
  fill.className = 'table-fill';
  fill.setAttribute('aria-hidden', 'true');
  row.append(fill);
  return row;
}

function buildTrackRow(track, groupKey) {
  const pending = edits.get(track.path);
  const tr = document.createElement("tr");
  tr.dataset.path = track.path;
  if (groupKey !== null) tr.dataset.group = groupKey;
  if (isPlayingPath(track.path)) tr.classList.add("playing");
  // An unreadable file (tags failed to parse) is shown but inert: it can't be
  // selected, played, or edited, and every mode's preview already skips it. It's
  // listed only so it never looks like the file vanished (#83).
  if (track.unreadable) {
    tr.classList.add("unreadable");
    const reason = backendMessage(track.read_error?.message) || track.read_error?.text || msg("app.unreadableNote");
    tr.innerHTML = `
      <td class="sel"></td>
      <td class="file" title="${escapeHtml(`${track.path}: ${reason}`)}">${escapeHtml(fileName(track.path))}</td>
      ${tableColumnKeys().length > 1 ? `<td class="unreadable-note" colspan="${tableColumnKeys().length - 1}">${escapeHtml(reason)}</td>` : ""}`;
    return decorateTrackArtwork(tr, track);
  }
  // Diff-state (#117): a staged plan renders its change into this same table in
  // place — no separate Preview view. Staged rows show the new values (dirty)
  // and the sel column becomes the per-row apply scope; other rows recede.
  if (diffByPath) {
    fillDiffRow(tr, track);
    return decorateTrackArtwork(tr, track);
  }
  // Checkbox + row highlight both reflect the `selection` set (source of truth),
  // so re-rendering never changes what's selected.
  const isSel = selection.has(track.path);
  if (isSel) tr.classList.add("selected");
  tr.innerHTML = `
      <td class="sel"></td>
      <td class="file" title="${escapeHtml(track.path)}">${escapeHtml(fileName(track.path))}</td>`;
  for (const field of tableColumnKeys()) {
    if (field === "file") continue; // rendered above (structural, always first)
    // A mask column (#150): computed, so read-only like Position — and it shows
    // what is on disk, since the backend renders it from the file.
    const custom = customColumnOf(field);
    if (custom) {
      const td = document.createElement("td");
      td.className = `position-cell align-${custom.align}`;
      td.textContent = customColumnValue(field, track.path);
      tr.appendChild(td);
      continue;
    }
    // A derived column (#106 Position, #172 Length): read-only, so it isn't
    // editable and carries no dirty state.
    const derived = virtualCellText(track, field, pending);
    if (derived !== null) {
      const td = document.createElement("td");
      td.className = "position-cell";
      td.textContent = derived;
      tr.appendChild(td);
      continue;
    }
    const original = tag(track, field);
    const edited = pending && pending.has(field);
    const value = edited ? pending.get(field) : original;
    const td = document.createElement("td");
    // A locked column keeps its cell shape and its dirty/error states, and
    // simply refuses to be edited (#48). Still marked `editable` so the diff and
    // the column machinery treat it like any other tag column — what `locked`
    // takes away is the double-click.
    td.className = "editable" + (isFieldLocked(field) ? " locked" : "");
    // Not editable until double-clicked (single click selects the row).
    td.contentEditable = "false";
    td.spellcheck = false;
    td.dataset.path = track.path;
    td.dataset.field = field;
    td.dataset.original = original;
    td.textContent = formatNumber(value, field);
    if (edited && value !== original) td.classList.add("dirty");
    tr.appendChild(td);
  }
  return decorateTrackArtwork(tr, track);
}

// One row of the in-table diff (#117). A staged row shows the plan's new values
// (dirty/error cells) and its sel checkbox is the per-row apply tick tracked in
// `applySelection`; an untouched row shows current values, inert. Reuses
// diffCell()/diffDir() from the old preview-table renderer.
function fillDiffRow(tr, track) {
  const change = diffByPath.get(track.path);
  const pending = edits.get(track.path);
  const automatic = editablePreview();
  if (rowSelectionSet().has(track.path)) tr.classList.add('selected');
  if (!change) {
    // Untouched row: current values, no apply tick, no editing/selection.
    tr.innerHTML =
      `<td class="sel"></td>` +
      `<td class="file" title="${escapeHtml(track.path)}">${escapeHtml(fileName(track.path))}</td>`;
    for (const field of tableColumnKeys()) {
      if (field === "file") continue;
      const td = document.createElement("td");
      const derived = virtualCellText(track, field, pending);
      if (derived !== null) {
        td.className = "position-cell";
        td.textContent = derived;
      } else {
        td.textContent = tag(track, field);
        if (automatic) markLiveEditable(td,track,field);
      }
      tr.appendChild(td);
    }
    return;
  }
  tr.classList.add("staged");
  tr.innerHTML =
    `<td class="sel"></td>` +
    diffFileCellHtml(change, track);
  for (const field of tableColumnKeys()) {
    if (field === "file") continue;
    const derived = virtualCellText(track, field, pending);
    if (derived !== null) {
      const td = document.createElement("td");
      td.className = "position-cell";
      td.textContent = derived;
      tr.appendChild(td);
      continue;
    }
    // Staged supplies every changed field, including fields hidden in Files.
    // A row without that field still gets a cell so the columns stay aligned.
    const cell = diffCell(change, field, track);
    const td = document.createElement("td");
    td.className = cell.cls;
    if (cell.title) td.title = cell.title;
    td.textContent = cell.text;
    if (automatic) markLiveEditable(td,track,field);
    if (cell.old && stagedView) {
      const old = document.createElement("span");
      old.className = "cell-old";
      old.textContent = cell.old;
      td.appendChild(old);
    }
    tr.appendChild(td);
  }
}

function markLiveEditable(cell,track,field) {
  if (customColumnOf(field) || isFieldLocked(field)) return;
  cell.classList.add('editable');
  cell.contentEditable='false'; cell.spellcheck=false;
  cell.dataset.path=track.path; cell.dataset.field=field;
  cell.dataset.original=tag(track,field);
}

// The File cell for a staged row: on a rename/move it shows the new name, the
// new relative folder beneath it (on a reorganize, #37), and the struck old name
// revealed under "Show old values". A non-rename change keeps the plain name.
function diffFileCellHtml(change, track) {
  if (!change.rename_to) {
    return `<td class="file" title="${escapeHtml(track.path)}">${escapeHtml(fileName(track.path))}</td>`;
  }
  const moved = diffDir(change.rename_to) !== diffDir(change.path);
  // Grouped by destination (#385), the header above already names the folder.
  const pathLine = moved && !destGrouping
    ? `<span class="fpath">${ico("corner")}${escapeHtml(diffDir(change.rename_to))}/</span>`
    : "";
  // Sidecars travelling with this rename/move (#58): a count badge, each pair
  // named in the tooltip so the plan is honest about what else will move.
  // Only the track's own: folder leftovers ride on one of a folder's changes
  // but belong to the folder, and the destination header counts them (#403).
  const own = (change.sidecar_renames || []).filter(
    ([from]) => !(change.folder_extras || []).includes(from),
  );
  const sc = own.length ? own : null;
  const scBadge = sc
    ? `<span class="fsidecars" title="${escapeHtml(
        sc.map(([from, to]) => `${fileName(from)}  →  ${fileName(to)}`).join("\n")
      )}">+${sc.length} sidecar${sc.length === 1 ? "" : "s"}</span>`
    : "";
  return (
    `<td class="file dirty" title="${escapeHtml(change.path)}  →  ${escapeHtml(change.rename_to)}">` +
    `<span class="fcell"><span class="fname">${escapeHtml(fileName(change.rename_to))}</span>${pathLine}${scBadge}` +
    `<span class="cell-old">${escapeHtml(fileName(change.path))}</span></span></td>`
  );
}

// A collapsible group header row spanning the table width.
function buildGroupHeader(key, count) {
  const collapsed = collapsedGroups.has(key);
  const tr = document.createElement("tr");
  tr.className = "group-head" + (collapsed ? " collapsed" : "");
  tr.dataset.group = key;
  tr.tabIndex = -1;
  tr.setAttribute('aria-expanded', String(!collapsed));
  const duplicate=duplicateGroupLookup.get(key);
  if(commandMode==='duplicates'&&duplicate&&!stagedView){
    tr.classList.add('duplicate-group');
    tr.innerHTML=`<td class="group-cell" colspan="${tableColumnKeys().length+2}"><span class="group-heading-content"><span class="group-caret">${ico('caret-down')}</span><span class="group-type-icon" aria-hidden="true">${ico('dedup')}</span><span class="group-label"></span><span class="group-count muted"></span></span></td>`;
    paintDuplicateHeader(tr,duplicate,count);
    return tr;
  }
  const dest = destGroups.get(key);
  if (dest || key === UNCHANGED_GROUP_KEY) {
    // A destination group (#385): where these files land, whether that folder
    // is new, and how many land there — the question a rename/move preview is
    // actually asked.
    tr.classList.add("dest");
    const label = dest
      ? `<span class="group-arrow">→</span><span class="group-label" title="${escapeHtml(dest.dir)}">${escapeHtml(dest.label)}</span>` +
        (dest.createsFolder ? `<span class="group-new">${escapeHtml(msg("diff.newFolder"))}</span>` : "")
      : `<span class="group-label unchanged">${escapeHtml(msg("diff.unchangedGroup"))}</span>`;
    // What else comes along from the source folder (#403): the rip log, loose
    // art, a Scans subfolder — named in the tooltip, below the destination.
    const extras = dest && dest.extras.length
      ? ` <span class="group-extras muted" title="${escapeHtml(
          dest.extras.map((to) => (to.startsWith(dest.dir) ? to.slice(dest.dir.length + 1) : to)).join("\n"),
        )}">+ ${escapeHtml(tn("unit.folderFile", dest.extras.length))}</span>`
      : "";
    tr.innerHTML = `<td class="group-cell" colspan="${tableColumnKeys().length + 2}"><span class="group-heading-content">
      <span class="group-caret">${ico("caret-down")}</span>
      <span class="group-type-icon" aria-hidden="true">${ico("folder")}</span>${label}
      ${extras}<span class="group-count">${escapeHtml(tn("unit.file", count))}</span>
    </span></td>`;
    return tr;
  }
  tr.innerHTML = `<td class="group-cell" colspan="${tableColumnKeys().length + 2}"><span class="group-heading-content">
      <span class="group-caret">${ico("caret-down")}</span>
      <span class="group-type-icon" aria-hidden="true">${ico(groupIcon(groupBy))}</span>
      <span class="group-label" title="${escapeHtml(key)}">${escapeHtml(groupLabel(key))}</span>
      <span class="group-count">${escapeHtml(tn("unit.file", count))}</span>
    </span></td>`;
  return tr;
}

function paintDuplicateHeader(row,group,count){
  const title=duplicateGroupTitle(group,trackAt);
  const label=row.querySelector('.group-label'),copies=row.querySelector('.group-count');
  if(label.textContent!==title)label.textContent=title;
  const text=`${count} copies`;if(copies.textContent!==text)copies.textContent=text;
  row.setAttribute('aria-label',`${title}, ${text}. ${group.reason}`);
}

// ---- the windowed table (#189) ----
// The table used to keep one DOM row per file, so a library of a few thousand
// paid for rows nobody could see: every render, every restyle, and every walk
// over the rows was priced against the library rather than against the window.
//
// What the table shows is now a MODEL — an ordered list of group headers and
// tracks — of which only the slice the viewport covers is built as rows, held in
// place by a spacer row above and below. Everything that used to read the rows
// (select-all, ranges, group selection, keyboard navigation, the group toggle)
// reads the model; everything that paints rows works on the rendered slice.
//
// The rule to keep: the DOM is a view of the model, never the source of truth
// for what the table contains. `selection` was already a set for exactly this
// reason (#20) — this extends the same discipline to order, grouping and counts.

// The rendered list: group headers plus the tracks of expanded groups, in order.
let viewItems = [];
// Selectable paths in visual order, INCLUDING files inside collapsed groups —
// collapsing a folder hides its rows, it doesn't take its files out of scope
// (select-all and Shift-ranges covered them when they were hidden rows too).
let viewPaths = [];
// Filter matches include collapsed/unreadable files, as the status bar always has.
let matchingTrackCount = 0;
let viewPathIndex = new Map(); // path -> index in viewPaths
// The paths the keyboard walks: what is actually on screen, so ↑/↓ skip a
// collapsed folder rather than stepping through it invisibly.
let navPaths = [];
let navPathIndex = new Map();
let groupPaths = new Map(); // group key -> its paths, in order
let displayedGroupByPath = new Map();
let viewGroups = []; // group keys in order
// Folder position belongs to the session, independently of column direction,
// filtering, collapse state, and the row data re-read after Apply/Undo.
const folderSortOrders = new Map();
function folderOrderForSort() {
  if (groupBy !== 'folder' && groupBy !== 'drop') return null;
  let order = folderSortOrders.get(groupBy);
  if (!order) folderSortOrders.set(groupBy, (order = new Map()));
  for (const track of tracks) {
    const key = groupKeyOf(track);
    if (!order.has(key)) order.set(key, order.size);
  }
  return order;
}
// A staged rename/move that sends files to another folder is grouped by where
// they land (#385), not by where they sit: `destGrouping` says the model is in
// that shape, `destGroups` describes each destination group by its key.
const DEST_GROUP_PREFIX = "\u0000dest:";
const UNCHANGED_GROUP_KEY = "\u0000unchanged";
let destGrouping = false;
let destGroups = new Map(); // key -> { dir, label, createsFolder }
let itemIndexByPath = new Map(); // path -> index in viewItems (rendered items only)
let groupItemIndices = new Map();
let groupItems = new Map(); // group -> all its track items, including collapsed tracks
let windowGroupChanges = false;
// Per-item height in px once measured; null until then, when the per-kind
// estimate stands in. Rows are uniform in practice, but a staged rename is two
// lines tall, so heights are learned rather than assumed.
let itemHeights = [];
const itemOffsets = new RowHeights();
let rowHeightEstimate = 18;
let groupHeightEstimate = 22;
let rowEstimateMeasured = false;
let groupEstimateMeasured = false;
// The sticky header's height, re-read once per model rebuild: it sits inside the
// scroller, so the furthest the list can be scrolled counts it too.
let headHeight = 0;
// The rows currently in the DOM, by path — the replacement for querying the
// table, and the set every painter iterates.
let renderedRows = new Map();
let renderedFirst = 0;
let renderedLast = -1;
let lastPaintedScrollTop = tableWrap.scrollTop;
// Rows built above and below the viewport, so a small scroll shows real rows
// rather than a gap waiting to be filled.
const OVERSCAN_ROWS = 10;
// Album-sized sets stay mounted: scrolling them requires no DOM work. Large
// collections retain the bounded window instead of allocating a whole library.
const MOUNTED_ROW_LIMIT = 512;
// Keep the final row above the bottom fade and horizontal overlay thumb.
const TABLE_END_SPACE = 20;

function itemHeightAt(i) {
  const measured = itemHeights[i];
  if (measured != null) return measured;
  return viewItems[i].kind === "group" ? groupHeightEstimate : rowHeightEstimate;
}

function recomputeItemTops() {
  itemOffsets.reset(viewItems.length, itemHeightAt);
}

// First item at or after `offset` px, by binary search over the prefix sums.
function itemIndexAt(offset) {
  return itemOffsets.indexAt(offset);
}

// The header text for each destination group (#385): the folder below the one
// the mask was rendered under. Renamed in place, that's just the new subfolder;
// sent elsewhere, it leads with the destination's own name — "Tagged/ › Album/".
// Two in-place groups that would read the same (the same subfolder made in two
// different albums) lead with their album's name too, so none is ambiguous.
function labelDestGroups() {
  const sepOf = (p) => (p.includes("\\") && !p.includes("/") ? "\\" : "/");
  const rel = (info) => {
    const root = (info.root || "").replace(/[\\/]+$/, "");
    if (!root || !(info.dir === root || info.dir.startsWith(root + sepOf(info.dir)))) return null;
    return info.dir.slice(root.length + 1).replace(/\\/g, "/");
  };
  const short = new Map();
  for (const [key, info] of destGroups) {
    const r = rel(info);
    if (r === null) short.set(key, `${folderGroupLabel(info.dir)}/`);
    else if (info.inPlace && r) short.set(key, `${r}/`);
    else short.set(key, null);
  }
  const counts = new Map();
  for (const label of short.values()) if (label) counts.set(label, (counts.get(label) || 0) + 1);
  for (const [key, info] of destGroups) {
    const label = short.get(key);
    if (label && (counts.get(label) === 1 || rel(info) === null)) {
      info.label = label;
    } else {
      const r = rel(info);
      const rootLeaf = fileName((info.root || "").replace(/[\\/]+$/, ""));
      info.label = r ? `${rootLeaf}/ › ${r}/` : `${rootLeaf}/`;
    }
  }
}

// Rebuild the model from the track list, the filter, the grouping and the staged
// plan. No DOM work at all — the window render is what touches the table.
function buildViewModel(preparedFilter = null) {
  const duplicateView=duplicateResultsActive()&&!stagedView;
  const folderOrder = duplicateView?null:folderOrderForSort();
  // A staged change always stays visible so the whole plan can be reviewed and
  // scoped, even if the current filter would otherwise hide it (#117).
  matchingTrackCount = duplicateView?(nativeMatchPaths?.size||0):0;
  // Native duplicate groups already define their full companion scope. Avoid
  // filtering and folder-ordering the entire library again on each publication.
  const visible = duplicateView?[]:tracks.filter((track) => {
    const matches = preparedFilter ? preparedFilter.has(track.path) : matchesFilter(track);
    if (matches) matchingTrackCount += 1;
    if (stagedView) return !!diffByPath?.has(track.path); // every pending file, regardless of the Files filter
    return matches;
  });
  viewItems = [];
  viewPaths = [];
  navPaths = [];
  groupPaths = new Map();
  displayedGroupByPath = new Map();
  viewGroups = [];
  groupItemIndices = new Map();
  groupItems = new Map();
  windowGroupChanges = false;
  viewPathIndex=new Map();navPathIndex=new Map();itemIndexByPath=new Map();
  const pushGroup = (key, count) => {
    const header = {kind:'group', key, count};
    groupItems.set(key, []);
    groupItemIndices.set(key, viewItems.length);
    viewGroups.push(key);
    viewItems.push(header);
    return header;
  };
  const pushTrack = (track, groupKey, shown) => {
    displayedGroupByPath.set(track.path, groupKey ?? '');
    const item = {kind:'track', track, groupKey};
    if (groupKey !== null) groupItems.get(groupKey).push(item);
    if (shown) {itemIndexByPath.set(track.path,viewItems.length);viewItems.push(item);}
    if (track.unreadable) return; // inert: never selectable, never navigable
    viewPathIndex.set(track.path,viewPaths.length);
    viewPaths.push(track.path);
    if (shown) {navPathIndex.set(track.path,navPaths.length);navPaths.push(track.path);}
  };
  destGrouping = Boolean(
    stagedView && diffByPath &&
      [...diffByPath.values()].some((c) => c.rename_to && diffDir(c.rename_to) !== diffDir(c.path)),
  );
  destGroups = new Map();
  if(duplicateView){
    duplicateGroupLookup=new Map();
    for(const group of duplicateGroups){
      const key=`duplicate:${group.id}`;duplicateGroupLookup.set(key,group);
      const shown=!collapsedGroups.has(key),paths=[];let header=null;
      for(const path of group.paths){
        const track=trackAt(path);if(!track)continue;
        if(!header)header=pushGroup(key,0);
        header.count++;pushTrack(track,key,shown);if(!track.unreadable)paths.push(path);
      }
      if(header)groupPaths.set(key,paths);
    }
  } else if (destGrouping) {
    // Destinations first, in first-appearance order; whatever the plan leaves
    // where it is follows in one "staying where they are" group.
    const destOrder = [];
    const restOrder = [];
    const byKey = new Map();
    for (const track of visible) {
      const change = diffByPath.get(track.path);
      let key;
      if (change && change.rename_to) {
        const dir = diffDir(change.rename_to);
        key = DEST_GROUP_PREFIX + dir;
        if (!destGroups.has(key)) {
          destGroups.set(key, { dir, root: change.rename_root || null, inPlace: true, createsFolder: false, extras: [] });
          destOrder.push(key);
        }
        const info = destGroups.get(key);
        if (change.creates_folder) info.createsFolder = true;
        // The folder's leftovers travelling here (#403), as their target paths.
        for (const [from, to] of change.sidecar_renames || []) {
          if ((change.folder_extras || []).includes(from)) info.extras.push(to);
        }
        if (!info.root || diffDir(change.path) !== info.root.replace(/[\\/]+$/, "")) info.inPlace = false;
      } else {
        // One group, whatever the grouping: its folder or artist would only
        // compete with the destinations for the question the preview asks.
        key = UNCHANGED_GROUP_KEY;
        if (!byKey.has(key)) restOrder.push(key);
      }
      if (!byKey.has(key)) byKey.set(key, []);
      byKey.get(key).push(track);
    }
    labelDestGroups();
    for (const key of [...destOrder, ...restOrder]) {
      pushGroup(key, byKey.get(key).length);
      const shown = !collapsedGroups.has(key);
      const paths = [];
      for (const track of byKey.get(key)) {
        pushTrack(track, key, shown);
        if (!track.unreadable) paths.push(track.path);
      }
      groupPaths.set(key, paths);
    }
  } else if (groupBy) {
    // Groups in first-appearance order over the (mapping-ordered) track list,
    // so grouping never reorders the underlying files.
    const order = [];
    const byKey = new Map();
    for (const track of visible) {
      const key = groupKeyOf(track);
      if (!byKey.has(key)) {
        byKey.set(key, []);
        order.push(key);
      }
      byKey.get(key).push(track);
    }
    // Folder positions do not follow the track comparator or its direction.
    // Other grouping modes retain their existing label-order behavior.
    if (folderOrder) order.sort((a, b) => folderOrder.get(a) - folderOrder.get(b));
    else if (sortKey) order.sort((a, b) => numericCollator.compare(groupLabel(a), groupLabel(b)) * sortDir);
    for (const key of order) {
      pushGroup(key, byKey.get(key).length);
      const shown = !collapsedGroups.has(key);
      const paths = [];
      for (const track of byKey.get(key)) {
        pushTrack(track, key, shown);
        if (!track.unreadable) paths.push(track.path);
      }
      groupPaths.set(key, paths);
    }
  } else {
    for (const track of visible) pushTrack(track, null, true);
  }
  itemHeights = new Array(viewItems.length).fill(null);
  rowEstimateMeasured = groupEstimateMeasured = false;
  recomputeItemTops();
}

function updateGroupVisibility(key, collapsing) {
  const index = groupItemIndices.get(key) + 1;
  const members = groupItems.get(key);
  const end = index + (collapsing ? members.length : 0);
  const inserted = collapsing ? [] : members;
  viewItems = viewItems.slice(0, index).concat(inserted, viewItems.slice(end));
  itemHeights = itemHeights.slice(0, index).concat(new Array(inserted.length).fill(null), itemHeights.slice(end));
  windowGroupChanges ||= members.length > 80;
  reindexGroupVisibility();
}

function reindexGroupVisibility() {
  // Preserve the filtered scope, group order and measured heights. Reindex
  // only the displayed projection, leaving selection of hidden files intact.
  navPaths = [];
  navPathIndex = new Map();
  itemIndexByPath = new Map();
  groupItemIndices = new Map();
  for (let i = 0; i < viewItems.length; i++) {
    const item = viewItems[i];
    if (item.kind === 'group') { groupItemIndices.set(item.key, i); continue; }
    const {path, unreadable} = item.track;
    itemIndexByPath.set(path, i);
    if (!unreadable) { navPathIndex.set(path, navPaths.length); navPaths.push(path); }
  }
  recomputeItemTops();
}

// Spans include artwork and trailing fill. The selection band stays in the
// DOM for the grid controller but is hidden, so it must not add a phantom column.
// A spacer stands in for the rows outside the window, so the table is as tall as
// the whole list and the scrollbar means what it says.
function spacerRow(height) {
  const tr = document.createElement("tr");
  tr.className = "spacer";
  tr.setAttribute("aria-hidden", "true");
  const td = document.createElement("td");
  td.colSpan = tableColumnKeys().length + 2;
  td.style.height = `${Math.max(0, Math.round(height))}px`;
  tr.appendChild(td);
  return tr;
}

function setSpacerHeights() {
  const top = tracksBody.firstElementChild;
  const bottom = tracksBody.lastElementChild;
  if (top && top.classList.contains("spacer")) {
    const height = `${Math.max(0, Math.round(itemOffsets.top(renderedFirst)))}px`;
    if (top.firstElementChild.style.height !== height) top.firstElementChild.style.height = height;
  }
  if (bottom && bottom.classList.contains("spacer")) {
    const below = itemOffsets.top(viewItems.length) - itemOffsets.top(Math.min(renderedLast + 1, viewItems.length));
    const height = `${Math.max(0, Math.round(below + TABLE_END_SPACE))}px`;
    if (bottom.firstElementChild.style.height !== height) bottom.firstElementChild.style.height = height;
  }
}

// Learn the real heights of the rows just built. A row that turns out taller
// than its estimate (a staged rename is two lines) moves everything below it, so
// the scroll offset is corrected by however much the rows ABOVE the window
// changed — the first rendered row then stays exactly where the eye left it.
function measureRenderedRows() {
  // Only new rows need a layout read. Recording an estimate-equal height must
  // not rebuild the O(library) prefix array on each one-row scroll step.
  const before = itemOffsets.top(renderedFirst);
  const changed = [];
  let baselineChanged = false;
  let index = renderedFirst;
  for (const tr of tracksBody.children) {
    if (tr.classList.contains("spacer")) continue;
    const itemIndex = index++;
    if (itemHeights[itemIndex] != null) continue;
    const item = viewItems[itemIndex];
    if (!item) continue;
    const height = tr.getBoundingClientRect().height;
    if (!height) continue;
    itemHeights[itemIndex] = height;
    if (height !== itemOffsets.values[itemIndex]) changed.push([itemIndex, height]);
    // Learn the compact baseline once per model/font, then only lower it if a
    // shorter row appears. Tall diff rows remain individual measurements.
    if (item.kind === "group") {
      if (!groupEstimateMeasured || height < groupHeightEstimate) {
        if (height !== groupHeightEstimate) baselineChanged = true;
        groupHeightEstimate = height;
        groupEstimateMeasured = true;
      }
    } else if (!rowEstimateMeasured || height < rowHeightEstimate) {
      if (height !== rowHeightEstimate) baselineChanged = true;
      rowHeightEstimate = height;
      rowEstimateMeasured = true;
    }
  }
  if (!baselineChanged && !changed.length) return;
  if (baselineChanged) recomputeItemTops();
  else for (const [index, height] of changed) itemOffsets.set(index, height);
  const shift = itemOffsets.top(renderedFirst) - before;
  if (shift) tableWrap.scrollTop += shift;
  setSpacerHeights();
}

// One row of the model, ready to insert. The ARIA grid counts its header as row
// 1, then every viewItems entry (group bands and unreadable files included).
// Window spacers are aria-hidden and never consume a model row index.
function gridRowNumber(itemIndex) {
  return itemIndex + 2;
}

function buildItemRow(i) {
  const item = viewItems[i];
  const tr = item.kind === "group" ? buildGroupHeader(item.key, item.count) : buildTrackRow(item.track, item.groupKey);
  tr.setAttribute("aria-rowindex", String(gridRowNumber(i)));
  if (item.kind === "group") return tr;
  activeApplyReview?.paint(tr);
  tr.tabIndex = item.track.path === activeRowPath ? 0 : -1;
  renderedRows.set(item.track.path, tr);
  return tr;
}

function dropRow(tr, scrollOnly) {
  const active = tr.querySelector('[contenteditable="true"]');
  const preserveCancel = scrollOnly && active && active === document.activeElement;
  if (preserveCancel) {
    // Recycling ends the DOM edit. Escape still belongs to that cell while
    // the user is scrolling, so the rest of the staged plan stays intact.
    const {path, field, editOriginal} = active.dataset;
    scrolledCellEdit = {path, field, value: editOriginal ?? active.dataset.original};
  }
  if (active) {
    endCellSuggest();
    active.contentEditable = 'false';
    commitCellEdit(active);
  }
  if (tr.dataset.path && renderedRows.get(tr.dataset.path) === tr) renderedRows.delete(tr.dataset.path);
  tr.remove();
  if (preserveCancel) el('tracks').focus({preventScroll: true});
}

// Build the rows the viewport covers (plus the overscan). Cheap enough to run on
// every scroll event: it is bounded by the window, not by the library — and a
// scroll that moves the window by a row or two only builds those rows, because
// an overlapping window is trimmed and extended rather than rebuilt.
function renderWindow({ force = false, refreshGrid = true, scrollOnly = false, preserveDuplicateRows = false, reorderRows = false, toggledGroups = null } = {}) {
  const windowed = viewItems.length > MOUNTED_ROW_LIMIT || (windowGroupChanges && viewItems.length > 96);
  if (!force && scrollOnly && !windowed) {
    lastPaintedScrollTop = tableWrap.scrollTop;
    return;
  }
  // Before the table is shown for the first time it has no height yet; a nominal
  // one keeps the first render from being a single row.
  const viewportHeight = tableWrap.clientHeight || 600;
  // A rebuilt model can leave the scroller past the end of what is now a much
  // shorter list — a filter narrowing to a handful, a collapse-all. The browser
  // will clamp the offset when it next lays the table out, so clamp it here
  // first: cutting the window for a position that is about to be corrected
  // leaves the rows built somewhere the eye isn't.
  // Native scrolling owns its measured limit. Only clamp a rebuilt model;
  // clamping ordinary scrolls to an estimate fights the thumb near the end.
  if (!scrollOnly) {
    const maxTop = Math.max(0, Math.ceil(headHeight + itemOffsets.top(viewItems.length) + TABLE_END_SPACE - viewportHeight));
    if (tableWrap.scrollTop > maxTop) tableWrap.scrollTop = maxTop;
  }
  const viewportTop = tableWrap.scrollTop;
  lastPaintedScrollTop = viewportTop;
  const visibleFirst = itemIndexAt(viewportTop);
  const visibleLast = itemIndexAt(viewportTop + viewportHeight);
  // Scrolling within the mounted buffer has no table paint work.
  if (!force && scrollOnly && visibleFirst >= renderedFirst && visibleLast <= renderedLast) return;
  const first = windowed ? Math.max(0, visibleFirst - OVERSCAN_ROWS) : 0;
  const last = windowed ? Math.min(viewItems.length - 1, visibleLast + OVERSCAN_ROWS) : viewItems.length - 1;
  if (!force && first === renderedFirst && last === renderedLast) return;
  const rowCount = renderedLast - renderedFirst + 1;
  const intact = rowCount > 0 && tracksBody.children.length === rowCount + 2;
  const overlaps = intact && first <= renderedLast && last >= renderedFirst;
  const jumpingWindow = !force && intact && !overlaps;
  if (force || jumpingWindow) spreadsheet?.beforePaint();
  if ((force && (preserveDuplicateRows || reorderRows)) || toggledGroups || jumpingWindow) {
    const trackKey = (path, group) => (toggledGroups || reorderRows) ? `track:${(group || '').length}:${group || ''}${path}` : `track:${path}`;
    const rowKey = row => row.dataset.path ? trackKey(row.dataset.path, row.dataset.group) : `group:${row.dataset.group}`;
    const prior=new Map([...tracksBody.children].filter(row=>!row.classList.contains('spacer')).map(row=>[rowKey(row),row]));
    const spacers=[...tracksBody.children].filter(row=>row.classList.contains('spacer'));
    const desired=[spacers[0]||spacerRow(0)];
    renderedFirst=first;renderedLast=last;renderedRows=new Map();
    for(let i=first;i<=last;i++){
      const item=viewItems[i],key=item.kind==='track'?trackKey(item.track.path, item.groupKey):`group:${item.key}`;
      const row=prior.get(key)||buildItemRow(i);
      if (item.kind === 'track' && row._columnKeys?.join('\0') !== tableColumnKeys().join('\0')) {
        const fresh = buildTrackRow(item.track, item.groupKey);
        const oldCells = new Map(row._columnKeys.map((field, index) => [field, row.cells[index + TABLE_DATA_COLUMN_OFFSET]]));
        const fill = row.querySelector('.table-fill');
        const freshCells = [...fresh.cells];
        for (const [index, field] of tableColumnKeys().entries()) row.insertBefore(oldCells.get(field) || freshCells[index + TABLE_DATA_COLUMN_OFFSET], fill);
        for (const [field, cell] of oldCells) if (!tableColumnKeys().includes(field)) cell.remove();
        row._columnKeys = [...tableColumnKeys()];
      }
      const rowNumber=String(gridRowNumber(i));
      if(row.getAttribute('aria-rowindex')!==rowNumber)row.setAttribute('aria-rowindex',rowNumber);
      if(item.kind==='track')renderedRows.set(item.track.path,row);
      else if(toggledGroups){
        if(toggledGroups.has(item.key)){
          const collapsed=collapsedGroups.has(item.key);
          row.classList.toggle('collapsed',collapsed);
          row.setAttribute('aria-expanded',String(!collapsed));
        }
      } else {const group=duplicateGroupLookup.get(item.key);if(group)paintDuplicateHeader(row,group,item.count);}
      desired.push(row);
    }
    desired.push(spacers[1]||spacerRow(0));
    const wanted=new Set(desired);
    if(toggledGroups)for(const row of [...tracksBody.children])if(!wanted.has(row))dropRow(row);
    let cursor=tracksBody.firstElementChild;
    for(const row of desired){if(row===cursor)cursor=cursor.nextElementSibling;else tracksBody.insertBefore(row,cursor);}
    if(!toggledGroups)for(const row of [...tracksBody.children])if(!wanted.has(row))dropRow(row, scrollOnly);
    if(force&&preserveDuplicateRows)patchMountedFields();
    setSpacerHeights();
    // WebKit can clamp scrolling while the table temporarily contains fewer
    // rows. Restore the captured viewport after its spacers are consistent,
    // before measured-height corrections adjust that same viewport.
    if (tableWrap.scrollTop !== viewportTop) tableWrap.scrollTop = viewportTop;
    measureRenderedRows();if(refreshGrid)spreadsheet?.refresh({scrollOnly});return;
  }
  if (!force && overlaps) {
    spreadsheet?.beforePaint();
    const topSpacer = tracksBody.firstElementChild;
    const bottomSpacer = tracksBody.lastElementChild;
    while (renderedFirst < first) {
      dropRow(topSpacer.nextElementSibling, scrollOnly);
      renderedFirst += 1;
    }
    while (renderedLast > last) {
      dropRow(bottomSpacer.previousElementSibling, scrollOnly);
      renderedLast -= 1;
    }
    const enteringRows = [];
    while (renderedFirst > first) {
      renderedFirst -= 1;
      const row = buildItemRow(renderedFirst);
      topSpacer.after(row);
      enteringRows.push(row);
    }
    while (renderedLast < last) {
      renderedLast += 1;
      const row = buildItemRow(renderedLast);
      bottomSpacer.before(row);
      enteringRows.push(row);
    }
    setSpacerHeights();
    if (tableWrap.scrollTop !== viewportTop) tableWrap.scrollTop = viewportTop;
    measureRenderedRows();
    if (refreshGrid) spreadsheet?.refresh({scrollOnly, enteringRows});
    return;
  }
  renderedFirst = first;
  renderedLast = last;
  renderedRows = new Map();
  const frag = document.createDocumentFragment();
  frag.appendChild(spacerRow(itemOffsets.top(first)));
  for (let i = first; i <= last; i++) frag.appendChild(buildItemRow(i));
  frag.appendChild(spacerRow(TABLE_END_SPACE + itemOffsets.top(viewItems.length) - itemOffsets.top(Math.min(last + 1, viewItems.length))));
  tracksBody.replaceChildren(frag);
  if (tableWrap.scrollTop !== viewportTop) tableWrap.scrollTop = viewportTop;
  measureRenderedRows();
  if (refreshGrid) spreadsheet?.refresh({scrollOnly});
}

// The row for a path, when it is on screen. Off-window paths have no row — that
// is the point — so every caller has to tolerate `undefined`.
function renderedRow(path) {
  return renderedRows.get(path);
}

// Fit-to-content (#151) against a windowed table. Measuring what is rendered
// would fit a column to whatever happens to be on screen, and the widest value
// is usually elsewhere — so the measurer borrows a few real rows per column,
// the longest values in the list, and the browser lays those out for real.
// Collapsed folders are excluded for the same reason they were when their rows
// were `display: none`: a hidden value never set a column's width.
const MEASURE_CANDIDATES = 3;

function mountMeasureRows(keys) {
  const wanted = new Map();
  for (const key of keys) {
    // Longest first, by character count — a stand-in for width good enough to
    // pick the handful of candidates the browser then measures properly.
    const best = [];
    for (const item of viewItems) {
      if (item.kind !== "track") continue;
      const length = fieldValue(item.track, key).length;
      if (best.length < MEASURE_CANDIDATES) {
        best.push({ length, item });
        best.sort((a, b) => a.length - b.length);
      } else if (length > best[0].length) {
        best[0] = { length, item };
        best.sort((a, b) => a.length - b.length);
      }
    }
    for (const { item } of best) wanted.set(item.track.path, item);
  }
  const borrowed = [];
  for (const [path, item] of wanted) {
    if (renderedRows.has(path)) continue; // already in the table, already measured
    const tr = buildTrackRow(item.track, item.groupKey);
    tracksBody.appendChild(tr);
    borrowed.push(tr);
  }
  return () => borrowed.forEach((tr) => tr.remove());
}

let scrollPaintFrame = 0;
tableWrap.addEventListener("scroll", () => {
  // Compare with the last render, not the last event: a model rebuild can
  // paint another offset before a coalesced scroll event resets it to zero.
  if (tableWrap.scrollTop === lastPaintedScrollTop) return;
  if (!scrollPaintFrame) scrollPaintFrame = requestAnimationFrame(() => {
    scrollPaintFrame = 0;
    renderWindow({scrollOnly:true});
  });
}, {passive:true});
// The window is a function of the viewport, so anything that resizes it — the
// OS window, the mode-panel splitter, showing the table for the first time —
// has to re-cut it.
let tableResizeFrame = 0;
let tableResizeNeedsMeasure = false;
function scheduleTableResize({ measure = false } = {}) {
  tableResizeNeedsMeasure ||= measure;
  if (tableResizeFrame) return;
  tableResizeFrame = requestAnimationFrame(() => {
    tableResizeFrame = 0;
    if (document.body.classList.contains("resizing-dock")) {
      // A width-only live resize keeps the existing rows and their components.
      // Height changes recut only the entering/leaving part of the window.
      renderWindow({scrollOnly:true});
      return;
    }
    if (tableResizeNeedsMeasure) {
      tableResizeNeedsMeasure = false;
      for (let index = renderedFirst; index <= renderedLast; index++) itemHeights[index] = null;
      headHeight = el("tracks").tHead?.offsetHeight || headHeight;
      // A layout change alters geometry, not loaded values. Measure the
      // existing cells in place, preserving focus, raw edits and row identity.
      measureRenderedRows();
    }
    renderWindow({scrollOnly:true});
  });
}
let lastTableLayoutWidth = 0;
let lastTableViewportHeight = 0;
const tableGeometryObserver = new ResizeObserver(entries => {
  const viewport = entries.find(entry => entry.target === tableWrap)?.contentRect;
  // Group header widths use the native scrollport container. Do not publish
  // a per-frame inherited CSS variable through every table cell on resize.
  // Only data widths affect row heights. The trailing fill can animate with
  // the pane without remeasuring every loaded row on every drawer frame.
  const layoutWidth = tableColumnKeys().reduce((total, key) => total + columnWidth(key), 0);
  const widthChanged = layoutWidth !== lastTableLayoutWidth;
  const heightChanged = viewport && viewport.height !== lastTableViewportHeight;
  lastTableLayoutWidth = layoutWidth;
  if (viewport) lastTableViewportHeight = viewport.height;
  // A drawer changes the visible scrollport, not cell widths or row heights.
  // Update sticky group bounds without replacing rows on every animation frame.
  // Filter reveal/collapse changes only viewport height. Recut the bounded
  // window without rebuilding loaded rows on every animation frame.
  if (widthChanged || heightChanged) scheduleTableResize({measure:widthChanged});
});
tableGeometryObserver.observe(tableWrap);
tableGeometryObserver.observe(el('tracks'));
document.addEventListener("tagrex:dock-resize-end", () => {
  if (tableResizeNeedsMeasure) scheduleTableResize();
});

function renderTracks({ preparedFilter = null, preserveDuplicateRows = false, reorderRows = false } = {}) {
  groupMotion?.cancel();
  cancelSortMotion();
  const anchorIndex=preserveDuplicateRows?itemIndexAt(tableWrap.scrollTop):-1;
  const anchor=anchorIndex>=0?viewItems[anchorIndex]:null;
  const anchorOffset=anchor?itemOffsets.top(anchorIndex)-tableWrap.scrollTop:0;
  setReviewColumns(stagedView);
  syncTableColumns();
  if(hasLibraryQuery() && nativeMatchRevision!==contentRevision && scheduledQueryRevision!==contentRevision && !filterError) scheduleFilterRender();
  updateSortIndicators();
  tracksEmpty.hidden = stagedView ? !!diffByPath?.size : tracks.length > 0;
  tracksEmpty.textContent = stagedView ? msg('staging.empty') : msg('table.empty');
  headHeight = el("tracks").tHead?.offsetHeight || headHeight;
  buildViewModel(preparedFilter);
  if(anchor){const index=anchor.kind==='track'?itemIndexByPath.get(anchor.track.path):viewItems.findIndex(item=>item.kind==='group'&&item.key===anchor.key);if(index!=null&&index>=0)tableWrap.scrollTop=itemOffsets.top(index)-anchorOffset;}
  renderWindow({ force: true, refreshGrid: false, preserveDuplicateRows, reorderRows });

  // The group toggle only makes sense while grouped (#32).
  el("toggle-groups").hidden = !groupBy && !duplicateResultsActive();
  syncGroupToggle();

  // While diffing the sel column is the apply scope, not the selection, so the
  // selection UI must not repaint the checkboxes — refresh the action bar instead.
  if (diffByPath) updateDiffBar();
  if (!usingReviewSelection()) syncSelectionUI({ refreshGrid: false, refreshPanels:true });
  refreshRoving();
  spreadsheet?.refresh();
  // Sticky autofit (#151): re-fit only when what the cells show has actually
  // changed — this runs on every paint, including a plain row click.
  maybeAutofit();
  if(commandMode!=='duplicates') scheduleAutomaticTransformPreview();
  updateTransformScope();
  publishFilterState();
}

// Files owns the live counts. The full filtered summary remains available
// through its tooltip and accessible description, without another toolbar row.
function updateStatus(selected = selectedPaths().length, publish = true) {
  const total = tracks.length;
  updateSelectionSummary(selected);
  const totalCount = el('files-total-count');
  const totalText = total.toLocaleString();
  if (totalCount.textContent !== totalText) totalCount.textContent = totalText;
  const files = filterText || filterConditions.length
    ? `${matchingTrackCount}/${tn("unit.file", total)}` : tn("unit.file", total);
  const summary = `${files} · ${tn("count.selected", selected)}`;
  if (statusSel.textContent !== summary) statusSel.textContent = summary;
  const filesTab = el('files-tab');
  if (filesTab.title !== summary) filesTab.title = summary;
  if (publish) publishFilterState();
}

// Coalesce pointer/keyboard selection phases into one final update per frame.
// Only visibility changes animate; ordinary count edits never replay entry.
let selectionSummaryFrame = 0;
let selectionSummaryCount = 0;
let selectionSummaryVisible = false;
let selectionSummaryAnimation = null;
let selectionSummaryWidthAnimation = null;
let selectionSummaryCountAnimation = null;
let selectionSummarySizeSignature = '';
let selectionSummaryEpoch = 0;
let selectionMarqueeActive = false;
function updateSelectionSummary(count, immediate = false) {
  selectionSummaryCount = count;
  if (immediate) {
    cancelAnimationFrame(selectionSummaryFrame);
    selectionSummaryFrame = 0;
    paintSelectionSummary(count);
    return;
  }
  if (selectionSummaryFrame) return;
  selectionSummaryFrame = requestAnimationFrame(() => {
    selectionSummaryFrame = 0;
    paintSelectionSummary(selectionSummaryCount);
  });
}
function setSelectionSummaryVisible(show) {
  if (show === selectionSummaryVisible) return;
  const summary = el('selection-summary');
  const reversing = !selectionMarqueeActive && !!selectionSummaryAnimation;
  const from = reversing ? {transform:getComputedStyle(summary).transform, opacity:getComputedStyle(summary).opacity} : null;
  selectionSummaryVisible = show;
  summary.inert = !show;
  summary.setAttribute('aria-hidden', String(!show));
  if (!show) { selectionSummaryCountAnimation?.cancel(); selectionSummaryCountAnimation = null; }
  const epoch = ++selectionSummaryEpoch;
  selectionSummaryAnimation?.cancel();
  selectionSummaryAnimation = null;
  if (selectionMarqueeActive || matchMedia('(prefers-reduced-motion: reduce)').matches || !summary.animate) {
    summary.hidden = !show;
    return;
  }
  summary.hidden = false;
  const rest = 'translateX(-50%) translateY(0) scale(1)';
  const frames = show
    ? [from || {transform:'translateX(-50%) translateY(12px) scale(.9)',opacity:0,easing:'cubic-bezier(.16,.8,.24,1)'}, {transform:'translateX(-50%) translateY(-2px) scale(1.02)',opacity:1,offset:.7,easing:'ease-in-out'}, {transform:rest,opacity:1}]
    : [from || {transform:rest,opacity:1}, {transform:'translateX(-50%) translateY(8px) scale(.96)',opacity:0}];
  const animation = summary.animate(frames, {duration:show?260:140, easing:show?'linear':'cubic-bezier(.3,0,.8,.2)',fill:'both'});
  selectionSummaryAnimation = animation;
  animation.finished.then(() => {
    if (epoch !== selectionSummaryEpoch) return;
    summary.hidden = !show;
    if (!show) { selectionSummaryWidthAnimation?.cancel(); selectionSummaryWidthAnimation = null; }
    selectionSummaryAnimation = null;
    animation.cancel();
  }, () => {});
}
// Width layout is confined to this small absolute overlay, never the table.
// Re-target from the current width on rapid updates and discard finished fills.
function animateSelectionSummaryWidth(oldWidth) {
  const summary = el('selection-summary');
  selectionSummaryWidthAnimation?.cancel();
  selectionSummaryWidthAnimation = null;
  if (!oldWidth || summary.hidden || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const width = summary.offsetWidth;
  if (Math.abs(width - oldWidth) < 2) return;
  const overshoot = Math.sign(width - oldWidth) * Math.min(3, Math.abs(width - oldWidth) * .04);
  const animation = summary.animate([{width:`${oldWidth}px`},{width:`${width + overshoot}px`,offset:.8},{width:`${width}px`}], {duration:240,easing:'cubic-bezier(.2,.8,.3,1)',fill:'both'});
  selectionSummaryWidthAnimation = animation;
  animation.finished.then(() => {
    if (selectionSummaryWidthAnimation !== animation) return;
    selectionSummaryWidthAnimation = null;
    animation.cancel();
  }, () => {});
}
function paintSelectionSummary(count) {
  const kind = usingReviewSelection() ? 'included for Apply' : 'selected';
  const signature = `${count}|${kind}`;
  const summary = el('selection-summary');
  const sizeChanged = selectionSummarySizeSignature !== signature;
  const oldWidth = !selectionMarqueeActive && sizeChanged && count > 1 && !summary.hidden ? summary.offsetWidth : 0;
  // Keep the last multiple-selection label throughout exit. Writing 0 or 1
  // before the fade ends makes an otherwise correct hidden state flash on screen.
  if (count > 1) {
    const number = el('files-selected-count');
    const label = count.toLocaleString();
    if (number.textContent !== label) {
      const increasing = count > Number(number.dataset.count || 0);
      selectionSummaryCountAnimation?.cancel();
      selectionSummaryCountAnimation = null;
      number.textContent = label;
      number.dataset.count = String(count);
      if (!selectionMarqueeActive && selectionSummaryVisible && !matchMedia('(prefers-reduced-motion: reduce)').matches && number.animate) {
        const animation = number.animate([
          { opacity: .35, transform: `translateY(${increasing ? 4 : -4}px)` },
          { opacity: 1, transform: 'translateY(0)' },
        ], { duration: 140, easing: 'cubic-bezier(.2,.8,.2,1)' });
        selectionSummaryCountAnimation = animation;
        animation.finished.then(() => {
          if (selectionSummaryCountAnimation === animation) selectionSummaryCountAnimation = null;
        }, () => {});
      }
    }
  }
  if (el('selection-summary-kind').textContent !== kind) el('selection-summary-kind').textContent = kind;
  if (el('selection-summary-detail').textContent) el('selection-summary-detail').textContent = '';
  setSelectionSummaryVisible(count > 1);
  el('selection-summary-clear').disabled = applying || undoing || filterPending;
  if (sizeChanged) {
    selectionSummarySizeSignature = signature;
    animateSelectionSummaryWidth(oldWidth);
  }
}

function resetEdits({ preserveEditor = false } = {}) {
  scrolledCellEdit = null;
  clearLiveInputs();
  resetFieldEditor({ preserveSubject: preserveEditor });
}

// Move pending edits from old paths to new paths after a rename is applied, so
// tag edits survive a rename instead of being orphaned by the path change.
function remapEditsAfterRename(plan) {
  for (const change of plan.changes) {
    if (change.rename_to && edits.has(change.path)) {
      edits.set(change.rename_to, edits.get(change.path));
      edits.delete(change.path);
    }
  }
}

function patchMountedFields(keys = null, {rawInput = true} = {}) {
  const affected = keys ? new Map() : null;
  if (affected) for (const {path,field} of keys) {
    if (!renderedRows.has(path)) continue;
    let fields=affected.get(path);
    if (!fields) affected.set(path,(fields=new Set()));
    fields.add(field);
  }
  if (affected?.size === 0) return;
  const columns = tableColumnKeys();
  for (const [path, row] of renderedRows) {
    const fields=affected?.get(path);
    if (affected && !fields) continue;
    const track = trackAt(path); if (!track || track.unreadable) continue;
    const change = diffByPath?.get(path);
    if (!activeApplyReview && row.dataset.applyState) {
      delete row.dataset.applyState; row.removeAttribute('aria-disabled');
      row.classList.remove('apply-complete','applying-current');
    }
    row.classList.toggle('selected', rowSelectionSet().has(path));
    row.classList.toggle('staged', !!change);
    if (!keys) {
      const indicator = row.querySelector('.track-artwork');
      if (indicator) {
        const cover = change?.cover_change;
        const state = cover && !cover.invalid ? (cover.new.length ? 'embedded' : 'missing') : coverState(track);
        const operation = cover && !cover.invalid ? (!cover.new.length ? 'removed' : !cover.old.length ? 'added' : 'replaced') : '';
        const title = msg(`table.artwork.${operation || state}`);
        if (indicator.dataset.coverState !== state) indicator.dataset.coverState = state;
        indicator.classList.toggle('artwork-changing', !!operation);
        if (operation && indicator.dataset.coverChange !== operation) indicator.dataset.coverChange = operation;
        else if (!operation && indicator.hasAttribute('data-cover-change')) indicator.removeAttribute('data-cover-change');
        if ((indicator.getAttribute('title') ?? indicator.getAttribute('aria-description') ?? '') !== title) indicator.title = title;
        if (indicator.getAttribute('aria-label') !== title) indicator.setAttribute('aria-label', title);
      }
      const file = row.querySelector('td.file');
      const destination = change?.rename_to || '';
      if (file && (file.dataset.destination || '') !== destination) {
        const template = document.createElement('template');
        template.innerHTML = diffFileCellHtml(change || {path}, track);
        const next = template.content.firstChild;
        file.replaceChildren(...next.childNodes); file.className = next.className; file.title = next.title;
        file.dataset.destination = destination;
      }
    }
    for (const [index, field] of columns.entries()) {
      if (field === 'file' || customColumnOf(field)) continue;
      const cell = row.cells[index + TABLE_DATA_COLUMN_OFFSET]; if (!cell) continue;
      if (fields && !fields.has(field) && !(field==='position' && ['track','disc','media'].some(key=>fields.has(key)))) continue;
      const derived = virtualCellText(track, field, edits.get(path));
      if (derived !== null) { patchCellText(cell, derived); continue; }
      const projected = workingField(path, field);
      const presentation = change ? diffCell(change, field, track) : { text: formatNumber(projected.value, field), cls: projected.dirty ? 'dirty' : '' };
      // The raw overlay wins until native validation returns. Computed output
      // remains separate while the active editor owns its raw text.
      if (keys && rawInput && projected.raw !== undefined) {
        presentation.text = projected.raw;
        presentation.cls = projected.invalid ? 'error' : projected.dirty ? `dirty${projected.raw === '' ? ' cleared' : ''}` : '';
      }
      patchCellText(cell, presentation.text, stagedView ? presentation.old : '');
      const classes = presentation.cls.split(' ');
      for (const name of ['dirty','error','unchanged','cleared','empty']) cell.classList.toggle(name, classes.includes(name));
      if ((cell.getAttribute('title') ?? cell.getAttribute('aria-description') ?? '') !== (presentation.title || '')) cell.title = presentation.title || '';
    }
  }
}

document.addEventListener('tagrex:working-fields', event => {
  patchMountedFields(event.detail);
  if (nativeLiveFields && sharedStageSync) {
    const entries = event.detail.map(({path,field}) => ({path,field,value:workingField(path,field).value}));
    applyBtn.disabled = true;
    void sharedStageSync.intents(entries, liveIntentOptions()).then(() => updateDiffBar());
  }
});
document.addEventListener('tagrex:native-recompute', event => {
  if (!sharedStageSync) return;
  applyBtn.disabled=true;
  void sharedStageSync.intents(event.detail.entries,event.detail.options).then(()=>updateDiffBar());
});

function onCellEdit(td) {
  prepareTagCellEdit();
  const {path,field,original} = td.dataset;
  const value = td.textContent.replace(/\u00a0/g, ' ');
  const valid = validateFieldValue(field,value,[path]).ok;
  td.classList.toggle('error',!valid);
  td.classList.toggle('dirty',valid && value!==original);
  recordLiveField([path],field,value,valid);
}

// Enter or blur finishes the cell's raw input; Apply is the only writer.
function commitCellEdit(td) {
  if (!td.isConnected || !td.dataset.editingSession) return;
  delete td.dataset.editingSession;
  // Leaving an invalid cell keeps its visible raw draft and the Apply gate.
  // It is never silently filtered out of an otherwise applicable plan.
  // Input already scheduled this revision. Ending focus must not start another
  // revision while a preview is patching rows (which also produces blur).
  patchMountedFields([{path:td.dataset.path, field:td.dataset.field}]);
  if (stagedView && stagedMembershipPending) {
    stagedMembershipPending = false;
    queueMicrotask(() => { if (!tracksBody.querySelector('[contenteditable="true"]')) { if (!diffByPath?.size) exitDiffState(); else renderTracks({preserveDuplicateRows:true}); } });
  }
  if (diffByPath && !diffByPath.size) queueMicrotask(() => { if (!tracksBody.querySelector('[contenteditable="true"]')) exitDiffState(); });
  updateDiffBar();
}

// Escape leaves a cell as it was: the typed value is dropped and nothing is
// written.
function cancelCellEdit(td) {
  prepareTagCellEdit();
  const {path,field,original} = td.dataset;
  const value = td.dataset.editOriginal ?? original;
  td.textContent = value;
  td.classList.remove('dirty','error');
  recordRestoredCellValue(path, field, value);
}

function recordRestoredCellValue(path, field, value) {
  recordLiveField([path],field,value,validateFieldValue(field,value,[path]).ok);
}

function prepareTagCellEdit() {
  const excluded = previewPlan && !isAutomaticTransformPreview()
    ? [...(diffByPath?.keys() || [])].filter(path => !applySelection.has(path)) : null;
  adoptTagOnlyPreview();
  // Changing review mode must not silently re-include files the user excluded.
  if (excluded && !previewPlan) excludedApplyPaths = new Set(excluded);
}

// Reveal the file table. The Preview view is gone (#117): a staged plan now
// renders into this same table via the diff-state, and the duplicate scan is the
// DEDUPLICATOR mode driven from setMode (#118). With no sibling views left, the
// old `Files` tab is now a plain label (#121); this just toggles the file table.
function showView(which) {
  el("files-view").hidden = which !== "files";
  spreadsheet?.refresh();
}

// ---- duplicate finder (#40): a read-only library scan, grouped ----
function discardPreview() {
  if (applying || undoing) return;
  const discard = () => {
    resetEdits({preserveEditor:true});
    dismissAutomaticTransformPreview();
    exitDiffState();
    refreshSharedStagedFields();
  };
  if (sharedStageSync) {
    clearingSharedStage = true;
    try { withoutPreviewPublication(discard); }
    finally { clearingSharedStage = false; }
    void sharedStageSync.clear();
  } else discard();
}

// Path -> track lookup, so the diff can show the current value of a file's
// unchanged main/extra columns. Rebuilt per render from the live `tracks`.

// Stage `previewPlan` into the file table's diff-state (#117). Named renderPreview
// still because every mutating mode funnels its plan here; there is no separate
// Preview view any more — the same #tracks table shows the change in place with a
// persistent Apply/Discard footer. An empty plan just leaves the table untouched.
function renderPreview(plan, { preserveView = false, delta = null } = {}) {
  const inputPlan = plan;
  plan = resolvedPreviewPlan(plan);
  if (sharedStageSync && inputPlan !== plan && plan?.changes.length > 2048 && isAutomaticTransformRefresh()) {
    // Keep a library-sized stage stable while native validation patches the
    // typed field. Its raw editor node is already current; no whole-map pass.
    applyBtn.disabled = true;
    el('diff-actionbar').dataset.blocked = 'true';
    el('diff-actionbar').title = '';
    el('ab-locked').hidden = true;
    return;
  }
  renderStaging(plan, isNativeTransformedTag, delta);
  if (!plan?.changes.length && tracksBody.querySelector('[contenteditable="true"]')) {
    setDiffByPath(new Map()); setApplySelection(new Set());
    stagedMembershipPending = true; updateDiffBar(); return;
  }
  if (!plan?.changes.length) retireRevertedRawFields();
  const previous = diffByPath;
  const deltaMembershipChanged = delta?.nativeDelta && delta.changedKeys.some(({path}) => !!diffByPath?.has(path) !== !!delta.changeAt(path));
  if (plan?.changes.length) {
    if (delta?.nativeDelta && diffByPath) {
      for (const {path} of delta.changedKeys) {
        const change = delta.changeAt(path);
        if (change) { diffByPath.set(path,change); if (!excludedApplyPaths.has(path)) applySelection.add(path); }
        else { diffByPath.delete(path); applySelection.delete(path); }
      }
    } else {
      setDiffByPath(new Map(plan.changes.map(change => [change.path, change])));
      setApplySelection(new Set([...diffByPath.keys()].filter(path => !excludedApplyPaths.has(path))));
    }
    document.body.classList.add('diffing');
    document.body.classList.toggle('automatic-transform-preview', isAutomaticTransformPreview());
    const columnsChanged = delta?.nativeDelta && !delta.columnsChanged ? false : syncFileView();
    const membershipChanged = delta?.nativeDelta ? deltaMembershipChanged : !previous || previous.size !== diffByPath.size || [...previous.keys()].some(path => !diffByPath.has(path));
    const destinationsChanged = delta?.nativeDelta ? delta.changedKeys.some(({field})=>field==='filename') : previous && [...diffByPath].some(([path,change])=>change.rename_to!==previous.get(path)?.rename_to);
    if (stagedView && (membershipChanged || columnsChanged || destinationsChanged)) {
      if (tracksBody.querySelector('[contenteditable="true"]')) stagedMembershipPending = true;
      else renderTracks({ preserveDuplicateRows: true });
    }
    const scalarKeys = delta?.nativeDelta && delta.changedKeys.every(({field}) => field !== 'artwork' && field !== 'filename' && !field.startsWith('block:')) ? delta.changedKeys : null;
    patchMountedFields(scalarKeys, {rawInput:false});
    if (!delta?.nativeDelta || delta.changedKeys.some(({field}) => field === 'artwork' || field.startsWith('block:'))) renderPendingCoverSummary();
    const kind = previewSource === 'spreadsheet' ? spreadsheetBatchKind : '';
    const description = [kind, planText(previewPlan)].filter(Boolean).join(' · ');
    const label = description ? ` · ${shortPlanDescription(description)}` : '';
    if (el('ab-plan').textContent !== label) el('ab-plan').textContent = label;
    if (el('diff-actionbar').title !== description) el('diff-actionbar').title = description;
    el('diff-actionbar').hidden = false;
    renderLockedSkips(); updateDiffBar(); updateRefreshButton();
    return;
  }
  if (!plan || plan.changes.length === 0) {
    // A plan a lock emptied would otherwise land as "nothing to change", which
    // is the one message it must not give: nothing changed BECAUSE of the lock,
    // and the difference is the whole point of having one (#48). Said here
    // rather than in each caller — every mode's plan comes through this.
    const skips = (plan && plan.locked_skipped) || [];
    if (skips.length) {
      const parts = skips.map((s) =>
        msg("locks.fieldOnFiles", { field: diffLabel(s.field), files: tn("unit.file", s.files) })
      );
      toast(msg("toast.nothingLeftLocked", { fields: parts.join(", ") }));
    }
    exitDiffState();
    return;
  }
}

function shortPlanDescription(description) {
  const [head, ...rest] = description.split(": ");
  if (!rest.length) return description;
  const marks = rest.join(": ").split(" · ").slice(1);
  return [head, ...marks].join(" · ");
}

// What a lock kept out of the staged plan (#48).
//
// A change that silently did not happen is the one thing a lock must never be,
// so the bar names the fields and how many files each would have touched. It
// reads as a note rather than an error: nothing went wrong, the plan did
// exactly what the locks said it could.
function renderLockedSkips() {
  const note = el("ab-locked");
  const skips = (previewPlan && previewPlan.locked_skipped) || [];
  note.hidden = skips.length === 0;
  if (!skips.length) return;
  const parts = skips.map((s) =>
    msg("locks.fieldOnFiles", { field: diffLabel(s.field), files: tn("unit.file", s.files) })
  );
  note.textContent = msg("locks.lockedList", { fields: parts.join(", ") });
  note.title = msg(parts.length === 1 ? "locks.lockedTitleOne" : "locks.lockedTitleMany");
}

// Leave the diff-state: drop the plan + apply scope and put the plain rows back.
function exitDiffState() {
  excludedApplyPaths.clear();
  renderStaging(null);
  spreadsheetBatchKind = "";
  setPreviewPlan(null);
  setPreviewSource(null);
  setDiffByPath(null);
  renderPendingCoverSummary();
  setApplySelection(new Set());
  // A validated proposal may already have enabled Apply before a revert. The
  // empty plan must clear that state too, not merely hide its action footer.
  applyBtn.disabled = true;
  document.body.classList.remove("diffing", 'automatic-transform-preview');
  syncFileView();
  el("diff-actionbar").hidden = false; updateDiffBar();
  closePlanReport(); // a report about this plan goes with it (#562)
  if (stagedView) renderTracks({ preserveDuplicateRows: true });
  else patchMountedFields();
  refreshGenerator();
  updateRefreshButton(); // the re-read is reachable again (#212)
  if (currentMode === 'deduplicator') {
    el('files-view').hidden = true;
    el('duplicates-view').hidden = false;
    document.body.classList.add('mode-deduplicator');
  }
}

// The few words CSS draws (::before content) come from a custom property, so they
// follow the interface language like everything else (#475).
function syncCssLabels() {
  document.documentElement.style.setProperty("--cleared-label", JSON.stringify(msg("app.cleared")));
  syncPanelCellMin();
}

// The side panels' closing grid (#579) is three equal cells with the run button
// in the last; a cell must hold the longest run button in this language, or
// "Preview changes" in Russian wraps onto two lines. Measured off the labels,
// since most panels are hidden; a button alone takes the row and is left out.
function syncPanelCellMin() {
  const buttons = [...document.querySelectorAll(".panel-actions > .primary:not(:only-child)")];
  if (!buttons.length) return;
  const style = getComputedStyle(buttons[0]);
  const ctx = document.createElement("canvas").getContext("2d");
  ctx.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  const chrome =
    parseFloat(style.paddingLeft) +
    parseFloat(style.paddingRight) +
    parseFloat(style.borderLeftWidth) +
    parseFloat(style.borderRightWidth);
  const widest = Math.max(...buttons.map((b) => ctx.measureText(b.textContent.trim()).width));
  document.documentElement.style.setProperty("--panel-cell-min", `${Math.ceil(widest + chrome + 2)}px`);
}

// Sync the action footer (apply count + enabled) and the header select-all
// tri-state to `applySelection`. Every changed file starts ticked; unticking
// some narrows what a single Apply writes (#81).
function syncRenamePreviewButton() {
  el('preview').disabled = currentMode !== 'renamer' || !selectedPaths().length || applying || undoing || filterPending;
}
function updateDiffBar(scope) {
  updateSelectionSummary(rowSelectionSet().size);
  const hasWork = !!diffByPath?.size;
  el('diff-actionbar').dataset.hasWork = String(hasWork);
  el('diff-preview').disabled = !hasWork || applying || undoing || filterPending || commandMode === 'duplicates';
  syncRenamePreviewButton();
  if (!hasWork) {
    applyBtn.disabled = true;
    const label = msg('action.apply');
    el('ab-apply-label').textContent = label;
    applyBtn.setAttribute('aria-label',label);
    el('diff-actionbar').dataset.blocked = 'true';
    return;
  }
  syncRibbonSelection();
  scope ??= automaticTransformScope();
  const staged = diffByPath.size;
  const n = applySelection.size;
  el("ab-count").textContent = String(n);
  const applyLabel = msg("diff.applyFiles", { files: tn("unit.file", n) });
  if (applyBtn.getAttribute("aria-label") !== applyLabel) applyBtn.setAttribute("aria-label", applyLabel);
  const label = el("ab-apply-label");
  if (label.textContent !== applyLabel) label.textContent = applyLabel;
  applyBtn.disabled = applying || undoing || commandMode==='duplicates' || filterPending || !!filterError || n === 0 || !!sharedStageSync?.pending() || !!tracksBody.querySelector('[contenteditable="true"]') || scope.planInvalid || (isAutomaticTransformPreview() && (scope.busy || scope.invalid || scope.failed));
  const pending = filterPending || (isAutomaticTransformPreview() && scope.busy);
  const blocked = applyBtn.disabled;
  el("diff-actionbar").dataset.blocked = String(blocked);
  const statusKey = blocked ? (pending ? "" : "diff.checkChanges") : (n === 1 ? "diff.fileReady" : "diff.filesReady");
  el("ab-state").dataset.i18n = statusKey;
  const stateText = statusKey ? msg(statusKey) : '';
  if (el("ab-state").textContent !== stateText) el("ab-state").textContent = stateText;
  if (isAutomaticTransformPreview()) return; // header ticks still select tracks
  selectAll.checked = n > 0 && n === staged;
  selectAll.indeterminate = n > 0 && n < staged;
}

/* ---- per-cell diff helpers (#80, reused by the in-table diff-state #117) ----
   The Preview view's mirror table is gone; the file table now shows the diff in
   place (fillDiffRow). These helpers still compute the state + text of one
   changed cell, so they are shared by that renderer. */

// Main columns (mirror the file table, minus play).
const DIFF_MAIN_COLS = ["file", "artist", "title", "album", "year"];

function diffLabel(field) {
  return fieldLabel(field);
}

function diffDir(path) {
  const i = Math.max(String(path).lastIndexOf("/"), String(path).lastIndexOf("\\"));
  return i >= 0 ? path.slice(0, i) : "";
}

// The state + text of one field cell for one file: unchanged | dirty | error |
// cleared. `track` supplies the current value for an unchanged cell.
function diffCell(change, field, track) {
  const tc = (change.tag_changes || []).find((t) => t.field === field);
  const current = track ? (track.tags[field] || "") : "";
  if (!tc) return { text: current, cls: "unchanged" + (current ? "" : " empty") };
  const nv = tc.new == null ? "" : String(tc.new);
  const ov = tc.old == null ? "" : String(tc.old);
  if (tc.invalid) {
    return { text: nv, cls: "error", title: msg("app.invalidValue", { field: diffLabel(field), value: nv, old: ov || "∅" }), old: ov };
  }
  if (nv === "") return { text: "", cls: "dirty cleared", title: msg("app.clearedWas", { old: ov }), old: ov };
  return { text: nv, cls: "dirty", title: msg("app.was", { old: ov || "∅" }), old: ov };
}

let latestBatchId = null;
let redoCount = 0;
const redoViews = [];
const batchHandOrders = new Map();
let historyLoaded = false;
let historyReadVersion = 0;
async function refreshHistory() {
  const version = ++historyReadVersion;
  const root = sessionRoot;
  try {
    const [batches, count] = await Promise.all([invoke("history", {}), invoke("redo_count", {})]);
    if (version !== historyReadVersion || root !== sessionRoot) return;
    latestBatchId = batches[0]?.id ?? null;
    redoCount = count;
    if (redoViews.length > count) redoViews.splice(0, redoViews.length - count);
    updateRedoControl();
    historyLoaded = true;
    undoBtn.disabled = batches.length === 0;
    // Undo is icon-only now (#115) — reflect the batch count in the tooltip /
    // aria-label instead of overwriting the SVG with text.
    const label = batches.length
      ? msg("topbar.undoCount", { batches: tn("unit.batch", batches.length) })
      : msg("topbar.undo");
    undoBtn.title = label;
    undoBtn.setAttribute("aria-label", label);
  } catch (e) {
    /* history is best-effort in the toolbar */
  }
}



function updateRedoControl() {
  redoBtn.disabled = !redoCount || commandMode === 'duplicates' || applying || undoing || Boolean(diffByPath && !isAutomaticTransformPreview());
  const label = redoCount ? msg('topbar.redoCount', {batches:tn('unit.batch', redoCount)}) : msg('topbar.redo');
  redoBtn.title = label;
  redoBtn.setAttribute('aria-label', label);
}

// A path as a file manager hands it over (#179). Finder's "Copy as Pathname"
// wraps anything containing a space in single quotes, a shell copy uses double
// ones, and a path dragged into a terminal comes back with its spaces escaped.
// Pasted verbatim, none of those name a folder — and what followed was an empty
// table rather than a word about why.
function cleanPastedPath(raw) {
  let path = (raw || "").trim();
  // A matching pair only: a quote on one side alone is part of the name.
  const quote = path[0];
  if ((quote === "'" || quote === '"') && path.endsWith(quote) && path.length > 1) {
    path = path.slice(1, -1);
  }
  // A Windows path (a drive letter or a UNC share) uses the backslash as its
  // separator, not as an escape: unescaping turned C:\Users\me into C:Usersme and
  // the folder chooser's own answer into "No such folder" (#464).
  if (/^[A-Za-z]:[\\/]/.test(path) || path.startsWith("\\\\")) return path;
  return path.replace(/\\(.)/g, "$1").trim();
}


// ---- recently opened libraries (#180) ----
//
// A display preference like the column set, so localStorage rather than
// settings.json. Most recent first, no duplicates, and only real libraries: a
// drop of loose files opens a session rooted at their common ancestor, which is
// not a folder anyone chose to open.
const RECENT_ROOTS_KEY = "tagrex.recentRoots";
const RECENT_ROOTS_MAX = 8;

function recentRoots() {
  try {
    const saved = JSON.parse(localStorage.getItem(RECENT_ROOTS_KEY) || "[]");
    return Array.isArray(saved) ? saved.filter((p) => typeof p === "string" && p) : [];
  } catch (e) {
    return [];
  }
}

function writeRecentRoots(list) {
  try {
    localStorage.setItem(RECENT_ROOTS_KEY, JSON.stringify(list.slice(0, RECENT_ROOTS_MAX)));
  } catch (e) {
    /* storage unavailable — the list just won't persist */
  }
  el("root-recent").hidden = list.length === 0;
}

// Reopening a folder moves it up rather than adding a second row.
function rememberRoot(path) {
  if (!path) return;
  writeRecentRoots([path, ...recentRoots().filter((p) => p !== path)]);
}

function closeRecentMenu() {
  el("root-recent-menu").hidden = true;
  el("root-recent").setAttribute("aria-expanded", "false");
}

let recentMenuKey = null;
function toggleRecentMenu() {
  const menu = el("root-recent-menu");
  if (!menu.hidden) {
    closeRecentMenu();
    return;
  }
  const list = recentRoots();
  const key = JSON.stringify(list);
  if (key !== recentMenuKey || !menu.childNodes.length) {
  recentMenuKey = key;
  menu.innerHTML = "";
  for (const path of list) {
    const parts = path.split(/[\\/]/).filter(Boolean);
    const name = parts.pop() || path;
    const row = createComponentElement("button");
    row.type = "button";
    row.className = "col-menu-row tk-menu-item recent-row";
    // The tail is what tells two of these apart; the head is the tooltip's job.
    row.innerHTML =
      `<span class="path-parent">${escapeHtml(parts.length ? `${parts.length > 1 ? "…/" : "/"}${parts[parts.length - 1]}/` : "")}</span>` +
      `<span class="path-name">${escapeHtml(name)}</span>`;
    row.title = path;
    row.addEventListener("click", () => {
      closeRecentMenu();
      rootInput.value = path;
      openLibrary();
    });
    menu.appendChild(row);
  }
  }
  menu.hidden = false;
  el("root-recent").setAttribute("aria-expanded", "true");
  // `floating` is what neutralizes .col-menu's own absolute placement so the
  // inline offsets from placeFloating are the ones that count (#160).
  menu.classList.add("floating");
  placeRecentMenu();
}

// The list is as wide as the path box it drops from (#407), not as wide as its
// longest row: it belongs to that box, and a narrower one read as a separate
// popup hanging off its left end.
function placeRecentMenu() {
  // The whole field (#547), which the caret is now part of, and which is still
  // there while the path is being typed and the indicator is hidden.
  const box = el("root-field");
  placeFloating(el("root-recent-menu"), box, {
    align: "left",
    width: box.getBoundingClientRect().width,
  });
}
window.addEventListener("resize", () => {
  if (!el("root-recent-menu").hidden) placeRecentMenu();
});

// ---- the library indicator and its one button (#177) ----
//
// `openedRoot` is what is actually open; the input is what the user is saying.
// The button's role follows the difference between them, NOT focus: pasting a
// path and reaching for the button blurs the field, and a focus-driven toggle
// would turn Open back into Browse just in time for the click to open a folder
// dialog instead of the pasted path.
let openedRoot = "";

function libraryIsDirty() {
  const typed = rootInput.value.trim();
  return typed !== "" && typed !== openedRoot;
}

// Name the open folder: the folder itself in full, its parent path dimmed and
// elided from the left (the tail is the half that answers "which one?"), the
// whole path in the tooltip.
function renderRootDisplay() {
  const display = el("root-display");
  const parent = display.querySelector(".path-parent");
  const name = display.querySelector(".path-name");
  if (!openedRoot) {
    parent.textContent = "";
    // Not a `data-i18n` on the span (#287): the static-text pass would then
    // overwrite an open folder's name with this placeholder on every language
    // change. The name is dynamic, so its empty-state text is set here instead.
    name.textContent = msg("topbar.noFolder");
    display.classList.add("empty");
    display.title = msg("topbar.chooseFolder");
    return;
  }
  const parts = openedRoot.split(/[\\/]/).filter(Boolean);
  name.textContent = parts.pop() || openedRoot;
  // Only the folder it sits in, with a leading ellipsis for everything above:
  // "…/Temp music/" says where you are, "/Users/oleg…" says nothing, and that
  // is what a plain end-elided path gives you. The whole path is the tooltip.
  parent.textContent = parts.length
    ? `${parts.length > 1 ? "…/" : "/"}${parts[parts.length - 1]}/`
    : "";
  display.classList.remove("empty");
  display.title = openedRoot;
  fitRootDisplay();
}

// The parent segment is worth showing whole or not at all (#182): shaved down to
// `…/Temp m…` it is noise, and a second ellipsis in a line this short reads as
// damage. CSS can only shave, so the choice is made here — measure, and drop the
// segment when the line overflows, leaving the name the whole width.
function fitRootDisplay() {
  const display = el("root-display");
  const parent = display.querySelector(".path-parent");
  const name = display.querySelector(".path-name");
  if (!parent.textContent) return;
  parent.hidden = false;
  // Not `scrollWidth > clientWidth` on the button: the name clamps itself with
  // an ellipsis, so the content always "fits" and the overflow never shows up
  // there. What matters is the width the name WANTS — its own scrollWidth —
  // next to the parent it would have to share with.
  const style = getComputedStyle(display);
  const inner =
    display.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
  if (parent.offsetWidth + name.scrollWidth > inner) parent.hidden = true;
}

// Browse… normally, Open while the field says something else. Label, tooltip and
// accessible name move together, as the search/stop button's do (#111).
function updateLibAction() {
  const dirty = libraryIsDirty();
  const btn = el("lib-action");
  // The button lost its word (#230), so the glyph is what carries the state:
  // a folder to go and choose one, the import arrow to open what was typed.
  el("lib-action-glyph").setAttribute("href", dirty ? "#i-import" : "#i-folder");
  btn.classList.toggle("opening", dirty);
  btn.title = dirty ? msg("app.openTyped") : msg("topbar.chooseFolder");
  btn.setAttribute("aria-label", btn.title);
  // The title is off the element while the pointer is on it; put the new one
  // where the tooltip will find it rather than losing the change (#230).
  if (btn.dataset.tipText) btn.dataset.tipText = btn.title;
  updateRefreshButton();
}

// Whether the folder can be re-read right now (#212). Not before anything is
// open, and not while a plan is staged: the diff describes files as they were
// when it was built, and a re-scan under it would leave it talking about a
// library that has moved. Discarding a plan is a decision, not a side effect of
// pressing Refresh.
function updateRefreshButton() {
  const btn = el("lib-refresh");
  const staged = Boolean(diffByPath);
  btn.disabled = !openedRoot || staged;
  btn.title = staged
    ? msg("app.rereadBlocked")
    : msg("topbar.reread");
  btn.setAttribute("aria-label", msg("topbar.reread"));
}

// Re-read the open folder (#212).
//
// Deliberately not a reopen: sort, grouping, filter, the column layout and the
// mode you are in all stay exactly as they were — the point is to see what the
// folder says now, not to start again. What cannot survive is anything keyed to
// a path that has gone: a selection entry and a pending edit for a file no
// longer there are dropped, and everything still present keeps both.
async function refreshLibrary(expectedPaths = [], replacedPaths = []) {
  if (!openedRoot || diffByPath) return false;
  const btn = el("lib-refresh");
  btn.disabled = true;
  try {
    const fresh = await invoke("list_tracks", {});
    const present = new Set(fresh.map((track) => track.path));
    const goneFromSelection = [...selection].filter((path) => !present.has(path));
    const goneFromEdits = [...edits.keys()].filter((path) => !present.has(path));
    for (const path of goneFromSelection) selection.delete(path);
    for (const path of goneFromEdits) edits.delete(path);
    const before = tracks.length;
    const order = handOrder();
    setTracks(fresh);
    sortKeepingHandOrder(order); // the sort stays, as everything else does (#526)
    if (replacedPaths.length) {
      for (const path of expectedPaths) {
        if (present.has(path)) selection.add(path);
      }
    }
    // Nothing left selected but files to select: the same rule the open path
    // follows, so an operation is never aimed at nothing without saying so.
    if (selection.size === 0) {
      const firstReadable = fresh.find((track) => !track.unreadable);
      if (firstReadable) selection.add(firstReadable.path);
    }
    renderTracks();
    refreshCustomColumnCells();
    syncSelectionUI();
    const delta = fresh.length - before;
    const change =
      delta === 0
        ? msg("reread.noChange")
        : delta > 0
          ? msg("reread.more", { tracks: tn("unit.track", delta) })
          : msg("reread.gone", { tracks: tn("unit.track", -delta) });
    const dropped = goneFromEdits.length
      ? msg("reread.dropped", { edits: tn("unit.pendingEdit", goneFromEdits.length) })
      : "";
    toast(
      msg("toast.reread", {
        folder: folderName(openedRoot),
        tracks: tn("unit.track", fresh.length),
        change,
        dropped,
      })
    );
    return expectedPaths.every(path => present.has(path));
  } catch (e) {
    toast(String(e), true);
    return false;
  } finally {
    updateRefreshButton();
  }
}

// Show the input over the indicator, with the current path selected so a paste
// replaces it outright.
function editRootPath() {
  // The BOX, not the button inside it (#181): the border and the recent-folders
  // caret belong to the indicator and have to leave with it, and the inner
  // button carries a `display` of its own that would outrank `hidden` anyway.
  closeRecentMenu();
  el("root-box").hidden = true;
  rootInput.hidden = false;
  rootInput.value = openedRoot;
  rootInput.focus();
  rootInput.select();
  updateLibAction();
}

// Back to the indicator. Only when the field agrees with what is open — while it
// says something else, it has to stay visible, or the Open button would be
// offering to open something the user can no longer see.
function leaveRootEdit() {
  if (libraryIsDirty()) return;
  rootInput.hidden = true;
  el("root-box").hidden = false;
  renderRootDisplay();
  updateLibAction();
}

// ---- actions ----
async function openLibrary(options = {}) {
  const root = cleanPastedPath(rootInput.value);
  if (!root) {
    toast(msg("app.enterPath"), true);
    return;
  }
  invalidateBpm();
  flushSessionView();
  const version = ++openVersion;
  resumableRoot = "";
  const savedView = options.restore || null;
  pauseAutomaticTransformPreview();
  // Show what is actually about to be opened, not what was pasted.
  rootInput.value = root;
  let timeout;
  try {
    const operation = (async () => {
      await invoke("open_library", { root });
      if (version !== openVersion) return;
      setDropFolders(null);
      await afterOpen(root, version, savedView);
    })();
    await (savedView ? Promise.race([
      operation,
      new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error('resume-timeout')), 10000); }),
    ]) : operation);
    if (version !== openVersion) return;

    rememberRoot(root);
    clearResumeAttempt(sessionStorage());
    resumableRoot = root;
    restoringView = false;
    flushSessionView();
  } catch (e) {
    if (version !== openVersion) return;
    if (savedView) {
      // The native filesystem request may still be running. Ignore its result;
      // later work cannot publish a different selection or plan after timeout.
      openVersion++;
      restoringView = false;
      resumeAutomaticTransformPreview();
      rootInput.hidden = false;
      el("root-box").hidden = true;
      updateLibAction();
      toast(msg(String(e).includes('resume-timeout') ? 'session.slow' : 'session.unavailable', { folder: folderName(root) }), true);
      return;
    }
    // Missing, disconnected and permission-denied folders can recover. Keep
    // their recent entry instead of treating every I/O failure as deletion.
    toast(String(e), true);
  } finally {
    clearTimeout(timeout);
    if (version === openVersion) {
      restoringView = false;
      resumeAutomaticTransformPreview();
    }
  }

}

// Shared tail of every "open a session" path (Open, Browse, or a drag-and-drop):
// pull the track list and reset all per-session view state. `label` is what the
// success toast names. When `dropFolders` is set (a file-set drop, #127) the
// table defaults to drop-origin grouping; otherwise the saved group pref.
async function afterOpen(label, version = openVersion, savedView = null) {
  if (version !== openVersion) return;
  restoringView = !!savedView;
  latestBatchId = null;
  historyLoaded = false;
  historyReadVersion++;
  resetFieldEditor();
  automaticRenameBatches.clear();
  batchRefreshScopes.clear();
  setSessionRoot(label); // the opened/dropped root — folder group labels hang off it
  // What is open is now what the field says, so the button falls back to
  // Browse… and the indicator takes over from the input (#177).
  openedRoot = label;
  rootInput.value = label;
  leaveRootEdit();
  // Opening builds a new backend session with nothing locked, and the locks
  // belong to the person rather than to the folder they happen to have open —
  // so they are handed to it again (#48).
  await pushFieldLocks();
  if (version !== openVersion) return;
  const loadedTracks = await invoke("list_tracks", {});
  if (version !== openVersion) return;
  folderSortOrders.clear();
  setTracks(loadedTracks);
  // Only the first readable track is selected on open (#128), so an operation
  // never silently hits the whole library — the user picks what to work on
  // (a row, a range, a whole folder via its group header, or the select-all
  // box). The set (not the DOM) holds the selection. Unreadable placeholders
  // (#83) can't be operated on, so the first readable one is chosen.
  selection.clear();
  const firstReadable = tracks.find((t) => !t.unreadable);
  if (firstReadable) selection.add(firstReadable.path);
  // Opening a session drops any staged plan and leaves the diff-state.
  setPreviewPlan(null);
  setPreviewSource(null);
  setDiffByPath(null);
  setApplySelection(new Set());
  document.body.classList.remove("diffing", 'automatic-transform-preview');
  el("diff-actionbar").hidden = false; updateDiffBar();
  stagedView = false; stagedMembershipPending = false; excludedApplyPaths.clear();
  fileViewScroll.files = { top:0, left:0 }; fileViewScroll.staged = { top:0, left:0 };
  renderStaging(null); syncFileView();
  resetEdits();
  setSortKey(null);
  setSortDir(1);
  setManualOrder(false);
  redoViews.length = 0; batchHandOrders.clear(); redoCount = 0; updateRedoControl();
  filterConditions = [];
  filterRoot=emptyRoot();nativeMatchPaths=null;duplicateGroups=[];invalidateDuplicateScan();duplicateScanStarted=false;duplicateScanning=false;duplicateStatus='not_scanned';duplicateFoundCount=0;duplicateMessage='';resetDuplicatePublications();libraryQueryClient.invalidate();nativeMatchRevision=-1;scheduledQueryRevision=-1;
  filterConditionMode = "all";
  setFilterText("");
  el("filter").value = "";
  syncFilterControls(); // clears invalid query state
  setGroupBy(dropFolders ? "drop" : groupByPref(), { persist: false, rerender: false });
  folderOrderForSort();
  if (savedView) {
    filterRoot=savedView.queryRoot?normalizeRoot(savedView.queryRoot):legacyRoot(normalizeConditions(savedView.conditions),savedView.conditionMode==='any'?'any':'all');
    filterConditions=filterRoot.children;filterConditionMode=filterRoot.mode;commandMode=savedView.commandMode||'search';duplicateKind=savedView.duplicateKind||'exact';
    setFilterText(savedView.filter);
    el("filter").value = savedView.filter;
    syncFilterControls();
    const validColumns = new Set(allColumnKeys());
    setSortKey(validColumns.has(savedView.sortKey) || savedView.sortKey === 'artwork' ? savedView.sortKey : null);
    setSortDir(savedView.sortDir);
    // Group fields are checked against the current menu, not trusted from storage.
    const groups = [...el('group-menu').querySelectorAll('[data-group]')].map(option => option.dataset.group);
    if (groups.includes(savedView.groupBy)) setGroupBy(savedView.groupBy, { persist: false, rerender: false });
    sortTracks();
    collapsedGroups.clear();
    for (const key of savedView.collapsed) collapsedGroups.add(key);
    selection.clear();
    for (const path of savedView.selection) if (trackAt(path) && !trackAt(path).unreadable) selection.add(path);
    // A removed file is never replaced by another selected target implicitly.

  }
  renderTracks();
  if (savedView) {
    tableWrap.scrollTop = savedView.scrollTop;
    tableWrap.scrollLeft = savedView.scrollLeft;
    renderWindow();
  }
  // Mask columns (#150) are rendered by the backend, so they arrive a beat after
  // the rows do. The table paints immediately with the cells blank and repaints
  // once the values land — a visible column is never worth blocking the open on.
  refreshCustomColumnCells();
  showView("files");
  if (savedView) {
    setMode(savedView.mode);
    setSubtab(savedView.subtab);
  }
  showPlayerBar();
  await refreshHistory();
  if (version !== openVersion) return;
  // The folder's own name, not the path to it: the path field above the table
  // is already showing that, and spelling it out again is most of the toast
  // (#267).
  const opened = msg("toast.opened", { folder: folderName(label), tracks: tn("unit.track", tracks.length) });
  // A damaged undo journal was set aside on open (#676): say so, and where it went.
  let setAside = null;
  try {
    setAside = await invoke("take_journal_notice", {});
  } catch (e) {
    setAside = null; // an older backend without the command: nothing to report
  }
  if (version !== openVersion) return;
  if (setAside) {
    report(opened, [
      { text: msg("app.journalReset"), problem: true },
      { text: msg("app.journalSetAside", { path: setAside }), problem: false },
    ]);
  } else {
    toast(opened);
  }
  void sharedStageSync?.refresh();
}

// Open a drag-and-drop of `paths` (#127). The backend resolves a lone folder to
// a library and anything else to a file-set; `dropFolders` (the dropped dirs)
// drives the table's drop-origin grouping, and is null for library mode.
async function openDrop(paths, options = {}) {
  if (!paths || !paths.length) return;
  invalidateBpm();
  flushSessionView();
  const version = ++openVersion;
  resumableRoot = "";
  pauseAutomaticTransformPreview();
  try {
    const result = await invoke("open_drop", { paths });
    if (version !== openVersion) return;
    setDropFolders(result.mode === "files" ? result.folders || [] : null);
    rootInput.value = result.root;
    // A dropped FOLDER is a library and worth remembering (#180); a drop of
    // loose files is rooted at their common ancestor, which nobody chose.
    await afterOpen(result.root, version, options.restore || null);
    if (version !== openVersion) return;
    clearResumeAttempt(sessionStorage());
    if (result.mode !== "files") {
      rememberRoot(result.root);
      resumableRoot = result.root;
      restoringView = false;
      flushSessionView();
    } else clearSession(sessionStorage());
  } catch (e) {
    if (version === openVersion) toast(String(e), true);
  } finally {
    if (version === openVersion) {
      restoringView = false;
      resumeAutomaticTransformPreview();
    }
  }
}


async function previewEdits() {
  liveInputChanged();
}

// True while an Apply is running (#437), or an Undo (#448).
let applying = false;
let undoing = false;
let sharedStageSync = null;
let clearingSharedStage = false;
// This session's automatic rename batches retain the inverse path map needed
// to keep selection/keyboard anchors on the same files after a later Undo.
const automaticRenameBatches = new Map();
// In-session batches expose their exact source paths already. Unknown history
// and moves keep the full scanner fallback so membership/sidecars stay correct.
const batchRefreshScopes = new Map();
function rememberBatchScope(batch, plan) {
  if (batch?.id == null) return;
  if (manualOrder) batchHandOrders.set(batch.id, handOrder());
  while (batchHandOrders.size > 128) batchHandOrders.delete(batchHandOrders.keys().next().value);
  const moved = plan.changes.some(change => change.rename_to || change.sidecar_renames?.length);
  batchRefreshScopes.set(batch.id, moved ? null : [...new Set(plan.changes.map(change => change.path))]);
  // Older batches remain undoable through the scanner fallback; this memo is
  // only a bounded optimization, never the source of recovery information.
  while (batchRefreshScopes.size > 128) batchRefreshScopes.delete(batchRefreshScopes.keys().next().value);
}

async function refreshBatchTracks(paths, source) {
  if (source !== tracks) return false;
  const root = sessionRoot;
  if (paths) {
    try {
      const fresh = await invoke("read_track_rows", { paths });
      if (source !== tracks || root !== sessionRoot) return false;
      const updates = new Map(fresh.map(track => [track.path, track]));
      if (updates.size !== paths.length || paths.some(path => !updates.has(path))) {
        throw new Error("Incomplete track refresh");
      }
      setTracks(source.map(track => updates.get(track.path) || track));
      return "scoped";
    } catch {
      // Files can disappear between the completed write and its readback. The
      // scanner owns membership/unreadable placeholders, so recover through it.
      if (source !== tracks || root !== sessionRoot) return false;
    }
  }
  const fresh = await invoke("list_tracks", {});
  if (source !== tracks || root !== sessionRoot) return false;
  setTracks(fresh);
  return "full";
}
function remapTransformSelection(paths) {
  const next = [...selection].map(path => paths.get(path) || path);
  selection.clear();
  for (const path of next) selection.add(path);
  if (paths.has(activeRowPath)) setActiveRowPath(paths.get(activeRowPath));
  if (paths.has(selAnchor)) selAnchor = paths.get(selAnchor);
  selBase = new Set([...selBase].map(path => paths.get(path) || path));
}

function freezeMutationSurfaces({ scrollableFiles = false } = {}) {
  const surfaces = [...document.querySelectorAll('.mode-col > :not(#staged-edits), #files-view')];
  const states = surfaces.map(node => [node, node.inert]);
  const controls = surfaces.flatMap(node => [...node.querySelectorAll('input,textarea,select,button')]).map(node => [node,node.disabled]);
  for (const [node] of states) node.inert = scrollableFiles && node.id === 'files-view' ? false : true;
  for (const [node] of controls) node.disabled = true;
  // Keep native scrolling and scrollbar dragging, while blocking table edits,
  // sorting and selection for the duration of the transaction.
  const block = event => {
    if (event.type === 'pointerdown' && event.target === tableWrap) return;
    if (event.type === 'keydown') {
      const distance = { PageDown:tableWrap.clientHeight * .85, PageUp:-tableWrap.clientHeight * .85, ArrowDown:40, ArrowUp:-40 }[event.key];
      if (distance) tableWrap.scrollTop += distance;
      else if (event.key === 'Home') tableWrap.scrollTop = 0;
      else if (event.key === 'End') tableWrap.scrollTop = tableWrap.scrollHeight;
    }
    event.preventDefault(); event.stopImmediatePropagation();
  };
  const blocked = ['pointerdown','click','dblclick','contextmenu','keydown'];
  if (scrollableFiles) for (const type of blocked) tableWrap.addEventListener(type, block, true);
  return () => {
    if (scrollableFiles) for (const type of blocked) tableWrap.removeEventListener(type, block, true);
    for (const [node,disabled] of controls) if (node.isConnected) node.disabled = disabled;
    for (const [node,inert] of states) node.inert = inert;
  };
}

async function apply() {
  if(applying || undoing || commandMode==='duplicates' || sharedStageSync?.pending())return;
  if (!previewPlan || previewPlan.changes.length === 0) return;
  if (isAutomaticTransformPreview() && (automaticTransformScope().busy || automaticTransformScope().failed || automaticTransformScope().invalid)) return;
  if (filterPending || filterError || hasInvalidPreview(previewPlan)) return;
  const automaticRename = isAutomaticTransformPreview() && previewPlan.changes.some(change => change.rename_to);
  const wasRename = previewSource === "rename" || automaticRename;
  const wasEdits = previewSource === "edits";
  // Only the ticked rows are applied (#81). The apply scope is the sel-column
  // tick set (`applySelection`) while diffing. The plan the backend gets — and
  // undo journals — is exactly this subset.
  const changesByPath = new Map();
  for (const change of previewPlan.changes) {
    if (applySelection.has(change.path)) changesByPath.set(change.path, change);
  }
  const appliedPlan = {
    ...previewPlan,
    changes: orderPathsForDisplay([...changesByPath.keys()]).map(path => changesByPath.get(path)),
  };
  if (appliedPlan.changes.length === 0) {
    toast(msg("app.tickOneRow"), true);
    return;
  }
  const appliedPaths = new Set(appliedPlan.changes.map((c) => c.path));
  // A second Apply (the shortcut, a double click) would queue behind the first.
  if (applying || undoing) return;
  scrolledCellEdit = null;
  applying = true;
  el('diff-preview').disabled = true;
  applyBtn.disabled = true;
  spreadsheet?.refresh();
  const releaseMutationSurfaces = freezeMutationSurfaces({ scrollableFiles: true });
  const sourceTracks = tracks;
  const remainingSelected = new Set(rowSelectionSet());
  const review = createApplyReview(appliedPlan.changes, {
    columns: tableColumnKeys().slice(), offset: TABLE_DATA_COLUMN_OFFSET,
    rowForPath: renderedRow, artworkLabel: state => msg(`table.artwork.${state}`),
    onComplete: path => { if (sourceTracks === tracks && remainingSelected.delete(path)) updateStatus(remainingSelected.size, false); },
  });
  let outcome = "error", activityError = "";
  const feedback = createApplyFeedback(review, appliedPlan.changes);
  activeApplyReview = review;
  for (const row of renderedRows.values()) review.paint(row);
  pauseAutomaticTransformPreview();
  try {
    await beginApplyProgress(progress => {
      // The initial pass validates every file without writing. Follow rows only
      // once the executor starts the sequential write pass.
      if (!progress.completed && progress.done < appliedPlan.changes.length) return;
      feedback.progress(progress);
    }, appliedPlan.changes);
    if (sharedStageSync) await sharedStageSync.flush();
    const batch = await invoke("apply_plan", { plan: appliedPlan, ...(sharedStageSync ? { stageRevision: sharedStageSync.revision() } : {}) });
    finishingWriteProgress();
    sharedStageSync?.completed(appliedPlan);
    await feedback.finish();
    review.completeAll();
    if (sourceTracks !== tracks) return;
    rememberBatchScope(batch, appliedPlan);
    toast(msg("toast.applied", { files: tn("unit.file", appliedPlan.changes.length) }));
    if (wasRename) {
      remapEditsAfterRename(appliedPlan); // keep pending tag edits, new paths
      if (automaticRename) {
        const renamed = appliedPlan.changes.filter(change => change.rename_to);
        remapTransformSelection(new Map(renamed.map(change => [change.path, change.rename_to])));
        automaticRenameBatches.set(batch.id, new Map(renamed.map(change => [change.rename_to, change.path])));
      }
    }
    if (wasEdits) {
      // Drop only the applied files' edits; unticked files keep their staged
      // edits so a follow-up apply can still write them.
      removeAppliedLiveInputs(new Set([...appliedPaths, ...appliedPlan.changes.map(change => change.rename_to).filter(Boolean)]));
      resetFieldEditor();
    }
    // Cover/tag-only batches read back their exact targets. Moves use the
    // scanner because they can change folder/file-set membership.
    const order = handOrder(new Map(appliedPlan.changes.filter(change => change.rename_to && !change.copy).map(change => [change.path, change.rename_to])));
    const readback = await refreshBatchTracks(batchRefreshScopes.get(batch.id) ?? null, sourceTracks);
    if (!readback) return;
    sortKeepingHandOrder(order); // retain explicit alignment across writes
    // Mask columns render from disk, and an apply is exactly when what is on
    // disk changed under unchanged paths (#150).
    const scopedFields = readback === 'scoped' && !appliedPlan.changes.some(change => change.block_changes?.length)
      ? new Map(appliedPlan.changes.map(change => [change.path, new Set((change.tag_changes || []).map(field => field.field))])) : undefined;
    invalidateCustomColumns(readback === "scoped" ? batchRefreshScopes.get(batch.id) : null, scopedFields);
    // exitDiffState() drops the plan + apply scope and repaints the table.
    // Successful work leaves a clean selection for the next operation. Keep
    // these anchors on rollback so the same preview remains available to retry.
    selection.clear();
    selAnchor = null; selBase.clear();
    reviewAnchor = null; reviewBase.clear();
    spreadsheet?.clearRange({ refresh:false });
    activeApplyReview = null;
    if (sharedStageSync) withoutPreviewPublication(exitDiffState);
    else exitDiffState();
    refreshCustomColumnCells();
    await refreshHistory();
    outcome = "success";
  } catch (e) {
    activityError = String(e);
    feedback.cancel();
    activeApplyReview = null;
    review.restore();
    // A cancel is the user's own doing and left everything as it was: say so
    // rather than raising it as a failure (#437).
    const code = e && e.failure && e.failure.message && e.failure.message.code;
    if (code === "error.plan.cancelled") { outcome = "cancelled"; activityError = ""; toast(msg("toast.applyCancelled")); }
    else toast(String(e), true);
  } finally {
    feedback.cancel();
    activeApplyReview = null;
    endApplyProgress(outcome, activityError);
    applying = false;
    void sharedStageSync?.refresh();
    updateStatus(rowSelectionSet().size);
    releaseMutationSurfaces();
    spreadsheet?.refresh();
    resumeAutomaticTransformPreview({ dismiss: !hasLiveInputs() });
  }
}

// After "undo what was not touched", each field another edit changed is named,
// a line per file, so the user knows which files still hold a newer value. Kept
// on screen until dismissed, but not in the problem colour: keeping them is
// what the user asked for (#683).
const UNDO_KEPT_FILES_SHOWN = 5;
function undoKeptParts(kept) {
  const byFile = new Map();
  for (const { path, field } of kept) {
    if (!byFile.has(path)) byFile.set(path, []);
    const label = field.startsWith("custom:") ? field.slice("custom:".length) : fieldLabel(field);
    byFile.get(path).push(label);
  }
  const parts = [{ text: tn("app.undidKept", kept.length), keep: true }];
  for (const [path, labels] of [...byFile].slice(0, UNDO_KEPT_FILES_SHOWN)) {
    parts.push({ text: `${fileName(path)}: ${joinList(labels)}`, problem: false });
  }
  const more = byFile.size - UNDO_KEPT_FILES_SHOWN;
  if (more > 0) parts.push({ text: tn("app.undidKeptMore", more), problem: false });
  return parts;
}

async function undo() {
  // The backend takes one job at a time; a second click would only queue (#448).
  if (applying || undoing) return;
  undoing = true;
  const releaseMutationSurfaces = freezeMutationSurfaces();
  const sourceTracks = tracks;
  let outcome = null, activityError = "";
  pauseAutomaticTransformPreview();
  try {
    if (!historyLoaded) await refreshHistory();
    const batchId = latestBatchId;
    if (batchId === null) return;
    const redoView = {scope:batchRefreshScopes.get(batchId) ?? null, order:handOrder()};
    const order = batchHandOrders.get(batchId) ?? handOrder();
    await beginHistoryProgress('undo');
    let result;
    try {
      result = await invoke("undo", { batchId });
    } catch (e) {
      const failure = e && e.failure && e.failure.message;
      if (!failure || failure.code !== "error.plan.changedSinceApplied") throw e;
      endHistoryProgress();
      const path = (failure.args && failure.args.path) || "";
      const choice = await choiceDialog(msg("undo.changed.title"), {
        subject: fileName(path),
        detail: msg("undo.changed.detail"),
        primary: msg("undo.changed.keep"),
        secondary: msg("undo.changed.overwrite"),
        cancel: msg("undo.changed.cancel"),
      });
      if (!choice) return;
      await beginHistoryProgress('undo');
      result = await invoke("undo", {
        batchId,
        mode: choice === "primary" ? "keep_changed" : "overwrite",
      });
    }
    finishingWriteProgress();
    if (sourceTracks !== tracks) return;
    const kept = (result && result.kept) || [];
    const renamed = automaticRenameBatches.get(batchId);
    if (renamed) {
      remapTransformSelection(renamed);
      automaticRenameBatches.delete(batchId);
    }
    if (kept.length > 0) report(msg("app.undid"), undoKeptParts(kept));
    resetEdits();
    const readback = await refreshBatchTracks(batchRefreshScopes.get(batchId) ?? null, sourceTracks);
    if (!readback) return;
    sortKeepingHandOrder(order);
    if (!kept.length) redoViews.push(redoView);
    invalidateCustomColumns(readback === "scoped" ? batchRefreshScopes.get(batchId) : null);
    batchRefreshScopes.delete(batchId);
    // exitDiffState() also clears previewPlan/previewSource and repaints.
    exitDiffState();
    refreshCustomColumnCells();
    await refreshHistory();
    outcome = "success";
  } catch (e) {
    outcome = "error"; activityError = String(e);
    toast(activityError, true);
  } finally {
    endHistoryProgress(outcome, activityError);
    undoing = false;
    releaseMutationSurfaces();
    syncRibbonSelection();
    resumeAutomaticTransformPreview({ dismiss: !hasLiveInputs() });
  }
}



async function redo() {
  if (redoBtn.disabled || applying || undoing) return;
  undoing = true;
  const releaseMutationSurfaces = freezeMutationSurfaces();
  const sourceTracks = tracks;
  const view = redoViews.at(-1);
  let outcome = null, activityError = "";
  pauseAutomaticTransformPreview();
  try {
    await beginHistoryProgress('redo');
    const batch = await invoke('redo', {});
    finishingWriteProgress();
    if (!batch || tracks !== sourceTracks) return;
    const readback = await refreshBatchTracks(view?.scope ?? null, sourceTracks);
    if (!readback) return;
    batchRefreshScopes.set(batch.id, view?.scope ?? null);
    if (view?.order) batchHandOrders.set(batch.id, handOrder());
    sortKeepingHandOrder(view?.order ?? handOrder());
    if (view) redoViews.pop();
    invalidateCustomColumns(readback === 'scoped' ? view?.scope : null);
    invalidateAutomaticTransformPreview();
    if (!hasLiveInputs()) exitDiffState();
    else renderTracks();
    refreshCustomColumnCells();
    outcome = 'success';
  } catch (error) {
    outcome = 'error'; activityError = String(error);
    toast(activityError, true);
  } finally {
    endHistoryProgress(outcome, activityError);
    undoing = false;
    releaseMutationSurfaces();
    syncRibbonSelection();
    resumeAutomaticTransformPreview({dismiss:!hasLiveInputs()});
    await refreshHistory();
  }
}

// ---- mode tabs ----
// The active mode's panel is the only one shown; entering a mode refreshes its
// panel against the current selection. The table (subject) never changes — only
// the right-hand panel (verb) swaps.
const MODE_REFRESH = {
  // The mask example reads the selection, so it goes stale like TAGGER's own
  // field grid does while a different mode was open (#382).
  renamer: scheduleMaskExample,
  tagger: refreshTagger,
  generator: refreshGenerator,
  exporter: refreshExporter,
  deduplicator: () => {},
};
// TAGGER is the primary tool, so it's the default tab (see index.html). Keep
// this in sync with the tab/panel marked active/visible there.

function setMode(name) {
  const queryToggle=el('query-mode-toggle'),fromQueryToggle=document.activeElement===queryToggle;
  publishWorkspaceMode(name);
  setCurrentMode(name);
  // The dock borrows the GENERATOR panel's chain block (#149, #233); this puts
  // it wherever the new mode says it belongs — back in that panel on the way
  // into GENERATOR, into the dock on the way out of it if it is pinned.
  // Which also decides whether the wand is offered at all: the chain belongs to
  // some jobs and not others (#236).
  syncTransformPlacement();
  document.querySelectorAll(".mode-panel").forEach((panel) => {
    panel.hidden = panel.id !== `panel-${name}`;
  });
  // DEDUPLICATOR (#118) is the one mode that takes over the main area: its
  // read-only results replace the file table (and the Files/Preview strip), the
  // way its controls sit in the right panel like every other mode. Leaving it
  // restores the normal table view.
  const dedup=name==='deduplicator';
  const changed=commandMode!==(dedup?'duplicates':'search');
  if(commandMode==='duplicates'&&!dedup){duplicateJobs.cancel();invalidateDuplicateScan();duplicateGroups=[];duplicateMessage='';}
  commandMode=dedup?'duplicates':'search';
  if(changed){
    filterRoot=emptyRoot();filterConditions=[];filterConditionMode='all';
    nativeMatchPaths=null;duplicateGroups=[];duplicateScanStarted=false;duplicateScanning=false;duplicateStatus='not_scanned';duplicateFoundCount=0;resetDuplicatePublications();
    libraryQueryClient.cancel();
    setFilterText('');el('filter').value='';
    el('filter').dispatchEvent(new Event('input',{bubbles:true}));
    document.dispatchEvent(new Event('tagrex:query-suggest-dismiss'));
  }
  document.body.classList.toggle('mode-deduplicator',dedup);
  document.body.classList.toggle('mode-tagger',name==='tagger');
  el('duplicates-view').hidden=true;
  showView('files');
  el('diff-actionbar').hidden=false;
  updateDiffBar();
  if(dedup){
    document.dispatchEvent(new CustomEvent('tagrex:reveal-module',{detail:'inspector'}));
    // A compact layout can hide the file pane containing the mode toggle.
    // Keep keyboard focus in the newly revealed workflow in that case.
    if(fromQueryToggle)requestAnimationFrame(()=>requestAnimationFrame(()=>{
      if(document.querySelector('.dock-frame')?.dataset.dockCompact==='true'||!queryToggle.getClientRects().length){
        const scan=el('query-duplicate-scan');
        (scan&&!scan.disabled?scan:el('query-duplicate-kind'))?.focus({preventScroll:true});
      }
    }));
  }
  if(changed){syncFilterControls();scheduleFilterRender();}
  (MODE_REFRESH[name] || (() => {}))();
}

// Refresh the TAGGER field grid for the selection. The Discogs card list
// persists across mode switches (a search isn't thrown away when you leave).
function refreshTagger() {
  refreshFieldEditor();
  scheduleNameProbe(); // no-op unless FROM NAME is the open sub-tab
  scheduleTextProbe(); // the same for FROM TEXT, whose counts follow the selection
}



// Drop the mode-tab labels to icon-only when the labelled tabs no longer fit the
// top bar (#116, #280). Measured with labels shown (compact removed first), so the
// natural width is the yardstick — no oscillation. Keeps a fifth/longer mode
// (DEDUPLICATOR) from ever truncating the bar.
function updateCompactTabs() {
  // Measure row 1 (#343): the tabs now share their row only with the brand and
  // the app actions — the path box dropped to its own row — so the tab-collapse
  // signal is that row overflowing, not the whole (two-row) top bar.
  const bar = document.querySelector(".topbar-main");
  const tabs = document.querySelector(".mode-tabs");
  if (!bar || !tabs) return;
  document.body.classList.remove("compact-tabs");
  // On a shrink the spacer collapses first; once it is spent the row overflows
  // its own width — that overflow, measured with labels shown, is the signal to
  // fall back to icon-only tabs (#280).
  if (bar.scrollWidth > bar.clientWidth) document.body.classList.add("compact-tabs");
}
// The bar can overflow rather than shrink, so observing it never fires on a
// viewport shrink — track the viewport (the root element) instead, plus the window
// resize event, and run once at startup.
window.addEventListener("resize", updateCompactTabs);
if (window.ResizeObserver) {
  new ResizeObserver(updateCompactTabs).observe(document.documentElement);
}
updateCompactTabs();

// Right-click handling (#132). The webview's native menu (Reload / Inspect
// Element, plus a wall of macOS text services) looks out of place, so it's
// always suppressed. Over a text input or an editing tag cell we show our own
// minimal Cut/Copy/Paste/Select-All menu instead; elsewhere right-click does
// nothing. On macOS a Ctrl-click is an OS-level right-click, so this also keeps
// it from popping a menu (the additive-select modifier there is ⌘).
const editCtx = el("edit-ctx");
let ctxField = null; // the input / editing cell the menu acts on

function hideEditCtx() {
  editCtx.hidden = true;
  ctxField = null;
}

function showEditCtx(x, y, field) {
  ctxField = field;
  // The keys on the right, in this platform's notation (#560); a read-only
  // field can be copied from but not cut or pasted into.
  for (const key of editCtx.querySelectorAll(".ctx-key")) key.textContent = formatCombo(key.dataset.combo);
  const readOnly = !!(field.readOnly || field.disabled || field.getAttribute("contenteditable") === "false");
  editCtx.querySelector('[data-cmd="cut"]').disabled = readOnly;
  const paste = editCtx.querySelector('[data-cmd="paste"]');
  paste.disabled = readOnly;
  // Paste goes muted when there is nothing to paste. Only through the plugin:
  // the web API would raise a permission prompt just to open a menu.
  if (TAURI && !readOnly) {
    TAURI.invoke("plugin:clipboard-manager|read_text")
      .then((res) => {
        const text = typeof res === "string" ? res : (res && res.plainText && res.plainText.text) || "";
        if (ctxField === field) paste.disabled = !text;
      })
      .catch(() => {});
  }
  editCtx.returnFocusTo = field;
  editCtx.hidden = false;
  placeContextMenu(editCtx, x, y);
}

document.addEventListener("contextmenu", (e) => {
  if (e.target.closest("[data-artwork-ui]")) return;
  e.preventDefault(); // never show the webview's native menu
  const header = e.target.closest('#tracks thead th');
  if (header) { hideEditCtx(); hideFileCtx(); openColumnsMenu({x:e.clientX,y:e.clientY}, header); return; }
  el('columns-menu').hidden = true;
  const group = e.target.closest('#tracks-body tr.group-head');
  if (group) {
    hideEditCtx(); hideFileCtx(); el('columns-menu').hidden = true;
    if (!applying && !undoing) selectGroup(group.dataset.group, e.ctrlKey || e.metaKey ? 'toggle' : 'replace');
    return;
  }
  const field = e.target.closest('input, textarea, [contenteditable="true"]');
  if (field) {
    hideFileCtx();
    showEditCtx(e.clientX, e.clientY, field);
    return;
  }
  hideEditCtx();
  // The file column carries its own menu (#213). Inert while a plan is staged,
  // like every other row gesture — the diff names files, and taking one out
  // from under it would leave the plan talking about something that is gone.
  const fileCell = diffByPath ? null : e.target.closest("td.file");
  const tr = fileCell && fileCell.closest("tr");
  if (tr && tr.dataset.path) showFileCtx(e.clientX, e.clientY, tr);
  else hideFileCtx();
});

// A multi-row selection is a file operation, even over an editable column.
// Explicit cell ranges and a single data cell retain the spreadsheet menu.
document.addEventListener('contextmenu', event => {
  const row = event.target.closest('#tracks-body tr[data-path]:not(.unreadable)');
  if (!row || applying || undoing || event.target.closest('input, textarea, [contenteditable="true"]')) return;
  if (spreadsheet?.hasRangeAt(event.target)) return;
  if (!event.target.closest('td.file') && !(selection.size > 1 && selection.has(row.dataset.path))) return;
  event.preventDefault(); event.stopPropagation();
  hideEditCtx(); el('columns-menu').hidden = true;
  const keyboard = event.clientX === 0 && event.clientY === 0;
  const rect = row.getBoundingClientRect();
  showFileCtx(keyboard ? rect.left : event.clientX, keyboard ? rect.bottom : event.clientY, row);
}, true);

// ---- the file column's context menu (#213) ----
const fileCtx = el("file-ctx");
// The paths the open menu acts on, decided when it opens rather than when an
// item is clicked: what the menu is about must be what was on screen when it
// appeared, even if something else moves in between.
let fileCtxPaths = [];

function hideFileCtx() {
  fileCtx.hidden = true;
  fileCtxPaths = [];
}

let fileCtxAudioOpener = null;
function showFileCtx(x, y, tr) {
  const path = tr.dataset.path;
  // Right-clicking inside the selection acts on all of it; right-clicking
  // outside narrows to that row and SELECTS it, so nothing is ever done to
  // files the user cannot see are involved.
  if (!selection.has(path)) {
    selection.clear();
    selection.add(path);
    setActiveRowByPath(path, false);
    syncSelectionUI();
  }
  fileCtxPaths = selectedPaths();
  for (const command of ['hide', 'trash']) {
    const item = fileCtx.querySelector(`[data-cmd="${command}"]`);
    item.disabled = !!previewPlan;
    item.title = previewPlan ? msg('staging.applyFirst') : '';
  }
  const many = fileCtxPaths.length > 1;
  fileCtx.querySelector('[data-cmd="hide"] .ctx-label').textContent = many
    ? msg("ctx.removeN", { files: tn("unit.file", fileCtxPaths.length) })
    : msg("ctx.removeFromList");
  fileCtx.querySelector('[data-cmd="trash"] .ctx-label').textContent = many
    ? msg("ctx.trashN", { files: tn("unit.file", fileCtxPaths.length) })
    : msg("ctx.trash");
  // The toolbar's own actions (#563) follow their buttons: Detect BPM goes when
  // it is switched off and rests while a run is on, Clear tags rests with its
  // button. The keys come from the shortcut map, so a rebinding shows here.
  const bpmButton = el("detect-bpm");
  const bpmItem = fileCtx.querySelector('[data-cmd="bpm"]');
  bpmItem.hidden = bpmButton.hidden;
  bpmItem.disabled = bpmRunning() || bpmButton.disabled;
  fileCtx.querySelector('[data-cmd="clear"]').disabled = el("clear-tags").disabled;
  for (const key of fileCtx.querySelectorAll(".ctx-key")) key.textContent = formatCombo(comboOf(key.dataset.action));
  fileCtxAudioOpener = tr;
  fileCtx.returnFocusTo = document.activeElement;
  fileCtx.hidden = false;
  placeContextMenu(fileCtx, x, y);
}

// Drop rows from the table without touching disk. The library is the model, so
// this is a filter over it — and a re-read (#212) brings them all back, which is
// what makes "remove from the list" safe to offer beside a deletion.
function dropFromList(paths) {
  if(commandMode==='duplicates')return;
  const gone = new Set(paths);
  for (const path of gone) {
    selection.delete(path);
    edits.delete(path);
  }
  setTracks(tracks.filter((track) => !gone.has(track.path)));
  if (selection.size === 0) {
    const firstReadable = tracks.find((track) => !track.unreadable);
    if (firstReadable) selection.add(firstReadable.path);
  }
  renderTracks();
  syncSelectionUI();
}

async function trashFiles(paths) {
  if(commandMode==='duplicates'){toast('Return to Search before changing files.',true);return;}
  const ok = await confirmDialog(
    msg("confirm.trash", { files: tn("unit.file", paths.length) }),
    msg("action.moveToTrash"),
    { danger: true }
  );
  if (!ok) return;
  try {
    const trashed = await invoke("trash_files", { paths });
    dropFromList(trashed);
    toast(msg("toast.trashed", { files: tn("unit.file", trashed.length) }));
  } catch (e) {
    toast(String(e), true);
  }
}

fileCtx.addEventListener("click", (e) => {
  const item = e.target.closest(".ctx-item");
  if (!item) return;
  const paths = fileCtxPaths;
  const opener = fileCtx.returnFocusTo === document.body ? fileCtxAudioOpener : fileCtx.returnFocusTo;
  hideFileCtx();
  if (!paths.length) return;
  if (item.dataset.cmd === "hide") {
    dropFromList(paths);
    toast(msg("toast.removedFromList", { files: tn("unit.file", paths.length) }));
  } else if (item.dataset.cmd === "trash") {
    trashFiles(paths);
  } else if (item.dataset.cmd === "bpm") {
    // Opening the menu made `paths` the selection, which is what these act on.
    el("detect-bpm").click();
  } else if (item.dataset.cmd === "split-audio" || item.dataset.cmd === "convert-audio") {
    launchAudioTools(item.dataset.cmd === "split-audio" ? "split" : "convert", paths, opener);
  } else if (item.dataset.cmd === "formats") {
    refreshTagFormats();
    el("tag-formats-modal").hidden = false;
  } else if (item.dataset.cmd === "clear") {
    el("clear-tags").click();
  }
});

document.addEventListener("mousedown", (e) => {
  if (!fileCtx.hidden && !e.target.closest("#file-ctx")) hideFileCtx();
});
window.addEventListener("scroll", hideFileCtx, true);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !componentOwnsEscape(e)) hideFileCtx();
});

// Paste at the caret. Read the clipboard through the Tauri plugin — reading it
// with navigator.clipboard.readText() makes WKWebView pop a system "Paste"
// permission pill (#132). In browser-dev there's no plugin, so fall back to the
// web API there. The insert goes through execCommand so it lands at the caret
// and feeds the cell's normal edit handling.
async function pasteIntoField(field) {
  let text = "";
  try {
    if (TAURI) {
      const res = await TAURI.invoke("plugin:clipboard-manager|read_text");
      text = typeof res === "string" ? res : (res && res.plainText && res.plainText.text) || "";
    } else if (navigator.clipboard && navigator.clipboard.readText) {
      text = (await navigator.clipboard.readText()) || "";
    }
  } catch (_) {
    text = "";
  }
  if (field && field.focus) field.focus();
  if (text) document.execCommand("insertText", false, text);
}

// Act on mousedown (fires before the field loses its selection) and keep the
// field focused so execCommand targets it.
editCtx.addEventListener("mousedown", (e) => {
  const item = e.target.closest(".ctx-item");
  if (!item || item.disabled) return;
  e.preventDefault();
  const field = ctxField;
  if (field && field.focus) field.focus();
  const cmd = item.dataset.cmd;
  if (cmd === "cut") document.execCommand("cut");
  else if (cmd === "copy") document.execCommand("copy");
  else if (cmd === "selectall") document.execCommand("selectAll");
  else if (cmd === "paste") pasteIntoField(field);
  hideEditCtx();
});

// Dismiss on any outside interaction.
document.addEventListener("mousedown", (e) => {
  if (!editCtx.hidden && !e.target.closest("#edit-ctx")) hideEditCtx();
});
window.addEventListener("scroll", hideEditCtx, true);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !componentOwnsEscape(e)) hideEditCtx();
});

// ---- diff-state action bar (#117) ----
el("dup-scan").addEventListener("click", runDuplicateScan);
el('diff-preview').addEventListener('click', () => { if (!el('diff-preview').disabled) setFileView(true); });
for (const [id,staged] of [['files-tab',false],['staged-tab',true]]) {
  el(id).addEventListener('click', () => setFileView(staged));
  el(id).addEventListener('keydown', event => {
    if (!['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) return;
    event.preventDefault();
    const next=event.key==='Home'?false:event.key==='End'?true:!staged;
    setFileView(next); el(next?'staged-tab':'files-tab').focus();
  });
}

// The ribbon reuses the table selection and native folder-open paths. A folder
// addition builds a file set from the current files plus the chosen directory;
// the backend resolves/deduplicates these paths without writing media.
let ribbonFolderBusy = false;
let appendTypedFolder = false;
function folderCommandBlocked() {
  if (applying || undoing || ribbonFolderBusy) return true;
  if (edits.size || pendingImportCover || hasLiveInputs() || previewPlan?.changes?.length) {
    toast("Apply or clear staged edits before opening or adding a folder.", true);
    return true;
  }
  return false;
}
async function addFolderToList(root) {
  if (folderCommandBlocked()) return;
  const savedView = sessionSnapshot();
  const existingPaths = tracks.map(track => track.path);
  const paths = [...new Set([...existingPaths, cleanPastedPath(root)])];
  if (!paths.at(-1)) return;
  ribbonFolderBusy = true;
  try { await openDrop(paths, { restore: savedView }); }
  finally { ribbonFolderBusy = false; appendTypedFolder = false; }
}
function launchAudioTools(mode, paths, opener) {
  let ordered = paths;
  if (mode === 'convert' && paths.length > 1) {
    const remaining = new Set(paths);
    ordered = [...viewPaths.filter(path => remaining.delete(path)), ...remaining];
  }
  const pendingPaths = ordered.filter(path => edits.has(path) || diffByPath?.has(path));
  const libraryVersion = openVersion;
  const refresh = (exported, replaced = []) => libraryVersion === openVersion &&
    (!dropFolders || replaced.length > 0) && refreshLibrary(exported, replaced);
  const canReplace = () => libraryVersion === openVersion && !diffByPath && !applying && !undoing &&
    ordered.every(path => !edits.has(path));
  document.dispatchEvent(new CustomEvent('tagrex:audio-studio', {
    detail: { mode, paths: [...ordered], opener, pendingPaths,
      canReplace, refreshLibrary: openedRoot ? refresh : undefined },
  }));
}
document.addEventListener('tagrex:audio-entry', event => {
  const { mode, opener } = event.detail;
  launchAudioTools(mode, selectedPaths(), opener);
});
function syncRibbonSelection() {
  syncRenamePreviewButton();
  updateRedoControl();
  const reviewing = usingReviewSelection();
  const count = reviewing ? applySelection.size : selection.size;
  const total = reviewing ? diffByPath.size : viewPaths.length;
  el('ribbon-select-all').disabled = applying || undoing || filterPending || !total;
  el('ribbon-clear-selection').disabled = applying || undoing || filterPending || !count;
  const blocked = commandMode==='duplicates' || applying || undoing || filterPending || Boolean(diffByPath && !isAutomaticTransformPreview());
  const moves = blocked || !selection.size ? null
    : selection.size < viewPaths.length / 4
      ? indexedMovableDirections(viewPaths, viewPathIndex, selection, groupBy ? displayedGroupByPath : null)
      : movableDirections(ribbonOrderGroups(), selection);
  el('ribbon-move-up').disabled = !moves?.up;
  el('ribbon-move-down').disabled = !moves?.down;
  el('group-btn').disabled = applying || undoing;
  const blocks = new Map();
  const paths = selectedPaths();
  for (const path of paths) for (const block of trackAt(path)?.tag_blocks || []) blocks.set(block.kind, {kind:block.kind, label:block.label});
  document.dispatchEvent(new CustomEvent('tagrex:ribbon-state', {detail:{count:paths.length, busy:applying || undoing || commandMode==='duplicates', blocks:[...blocks.values()]}}));
}
function ribbonOrderGroups() {
  return groupBy ? [...groupPaths.values()] : [viewPaths];
}
function moveRibbonSelection(direction) {
  if (applying || undoing || filterPending || (diffByPath && !isAutomaticTransformPreview())) return;
  const next = shiftSelectedOrder(tracks, ribbonOrderGroups(), selection, direction);
  if (!next) return;
  for (let i = 0; i < next.length; i++) tracks[i] = next[i];
  reindexTracks();
  setManualOrder(true);
  renderTracks();
  const moved = navPaths.find(path => selection.has(path));
  if (moved) scrollPathIntoView(moved);
}
el('ribbon-move-up').addEventListener('click', () => moveRibbonSelection(-1));
el('ribbon-move-down').addEventListener('click', () => moveRibbonSelection(1));

el('ribbon-open-folder').addEventListener('click', () => {
  if (!folderCommandBlocked()) { appendTypedFolder = false; browseForFolder(); }
});
el('ribbon-add-folder').addEventListener('click', () => {
  if (!folderCommandBlocked()) browseForFolder({ append: true });
});
el('ribbon-select-all').addEventListener('click', () => {
  if (applying || undoing) return;
  selectAllRows();
});
el('ribbon-clear-selection').addEventListener('click', () => {
  if (applying || undoing) return;
  if (usingReviewSelection()) setApplySelection(new Set());
  else selection.clear();
  syncSelectionUI();
});
el('selection-summary-clear').addEventListener('click', () => {
  if (applying || undoing || filterPending) return;
  el('ribbon-clear-selection').click();
  el('tracks').focus({preventScroll:true});
});


// Immediate command writes have their own transaction and never consume live
// inputs or the global Apply scope. The native result refreshes only the staged
// old snapshots; its desired values and included rows stay exactly as chosen.
async function writeQuickPlan(plan) {
  if (!plan.changes.length) { toast('No changes to write.'); return; }
  if (hasInvalidPreview(plan)) throw new Error('Correct invalid values before writing.');
  applying = true;
  el('diff-preview').disabled = true;
  applyBtn.disabled = true;
  pauseAutomaticTransformPreview();
  const staged = previewPlan;
  const source = previewSource;
  const included = new Set(applySelection);
  const sourceTracks = tracks;
  const remainingSelected = new Set(selection);
  const release = freezeMutationSurfaces({scrollableFiles:true});
  const review = createApplyReview(plan.changes, {
    columns:tableColumnKeys().slice(), offset:TABLE_DATA_COLUMN_OFFSET,
    rowForPath:renderedRow, artworkLabel:state => msg(`table.artwork.${state}`),
    onComplete:path => { if (sourceTracks === tracks && remainingSelected.delete(path)) updateStatus(remainingSelected.size, false); },
  });
  let outcome = "error", activityError = "";
  const feedback = createApplyFeedback(review, plan.changes);
  activeApplyReview = review;
  for (const row of renderedRows.values()) review.paint(row);
  el('diff-actionbar').hidden = false;
  syncRibbonSelection();
  try {
    await beginApplyProgress(progress => {
      if (!progress.completed && progress.done < plan.changes.length) return;
      feedback.progress(progress);
    }, plan.changes);
    const result = await invoke('quick_write', {plan, stagedPlan:staged});
    finishingWriteProgress();
    await feedback.finish();
    review.completeAll();
    activeApplyReview = null;
    rememberBatchScope(result.batch, plan);
    for (const change of plan.changes) selection.delete(change.path);
    spreadsheet?.clearRange({refresh:false});
    const updated = new Map(result.tracks.map(track => [track.path, track]));
    setTracks(tracks.map(track => updated.get(track.path) || track));
    invalidateCustomColumns([...updated.keys()]);
    setPreviewPlan(result.plan); setPreviewSource(source);
    if (result.plan) {
      renderPreview(result.plan);
      setApplySelection(included); syncSelectionUI();
    } else renderTracks();
    refreshCustomColumnCells();
    refreshFieldEditor();
    await refreshHistory();
    outcome = 'success';
    toast(`Wrote changes to ${plan.changes.length} ${plan.changes.length === 1 ? 'file' : 'files'}. Staged edits kept.`);
  } catch (error) {
    feedback.cancel(); activeApplyReview = null; review.restore();
    activityError = String(error);
    if (error?.failure?.message?.code === 'error.plan.cancelled') { outcome = 'cancelled'; activityError = ''; toast(msg('toast.applyCancelled')); }
    else throw error;
  } finally {
    feedback.cancel(); activeApplyReview = null; endApplyProgress(outcome, activityError);
    applying = false; release();
    el('diff-actionbar').hidden = false;
    updateDiffBar();
    updateStatus(rowSelectionSet().size);
    syncRibbonSelection(); spreadsheet?.refresh();
    resumeAutomaticTransformPreview({dismiss:!hasLiveInputs()});
  }
}

document.addEventListener('tagrex:number-format', () => renderTracks());
const fieldFormatActions = createFieldFormatActions({
  invoke, captureSelection, isSelectionCurrent, getPlan:() => previewPlan, inputRevision:liveInputRevision,
  workingField, trackAt, isFieldLocked, adoptTagOnlyPreview, recordLiveFields, writeQuickPlan,
  fieldCatalogue:() => EXTENDED_FIELDS.map(([key]) => ({key,label:fieldLabel(key)})), labelField:fieldLabel,
});
const numberingActions = createNumberingActions({
  invoke,captureSelection,isSelectionCurrent,getPlan:()=>previewPlan,inputRevision:liveInputRevision,
  read:workingField,
  group:path=>{
    if(displayedGroupByPath.has(path))return displayedGroupByPath.get(path);
    // Selected files hidden by filtering retain the same saved-tag grouping;
    // destination previews also retain their destination when filtered out.
    const change=diffByPath.get(path);
    if(destGrouping)return change?.rename_to?DEST_GROUP_PREFIX+diffDir(change.rename_to):UNCHANGED_GROUP_KEY;
    // Duplicate result members are already indexed above; unmatched retained
    // selections use ordinary grouping without rescanning every result group.
    return groupBy?groupKeyOf(trackAt(path)):'';
  },
  order:paths=>hooks.orderPathsForDisplay(paths),context:()=>groupBy,
  isFieldLocked,adoptTagOnlyPreview,recordLiveFields,writeQuickPlan,
});
connectRibbonActions(async (action, options) => {
  if (action === 'numbering-cancel') return numberingActions(action, options);
  if (action === 'field-format-cancel') return fieldFormatActions(action, options);
  if (applying || undoing) throw new Error('Wait for the current operation to finish.');
  if (action.startsWith('numbering-')) return numberingActions(action, options);
  if (action.startsWith('field-format-')) return fieldFormatActions(action, options);
  if (action === 'artwork-settings') {
    await openSettings();
    const groups = [...document.querySelectorAll('.settings-group')];
    const at = groups.findIndex(group => group.contains(el('set-cover-max')));
    if (at >= 0) selectSettingsGroup(at);
    el('set-cover-max').scrollIntoView({block:'center'}); el('set-cover-max').focus();
    return;
  }
  const subject = captureSelection();
  const paths = subject.paths;
  if (!paths.length) throw new Error('Select files first.');
  if (action === 'rename') {
    setMode('renamer');
    document.dispatchEvent(new CustomEvent('tagrex:reveal-module', {detail:'inspector'}));
    el('mask').focus();
    toast('Use a folder pattern such as %album%/%filename% to preserve filenames.');
  } else if (action === 'export-artwork') {
    await exportCover();
  } else if (action === 'number') {
    if (options.mode === 'write') {
      const edits = numberingEdits({includeDisc:false});
      const plan = await invoke('preview_tag_edits', {edits, paths, cover:null});
      if (!isSelectionCurrent(subject)) throw new Error('The selection changed. Try again.');
      await writeQuickPlan(plan);
    } else if (await numberTracks({includeDisc:false}) === false) return;
    return {dialog:'ribbon-number-dialog'};
  } else if (action === 'number-disc') {
    // Disc assignment is an independent action. Never run the track-numbering
    // engine or inspect its hidden controls, grouping or vinyl options here.
    const raw = String(options.disc ?? '').trim();
    const value = Number(raw);
    if (!/^\d+$/.test(raw) || !Number.isSafeInteger(value) || value < 1) throw new Error('Enter a whole disc number of 1 or more.');
    if (isFieldLocked('disc')) throw new Error('Disc is locked. Unlock it before setting disc numbers.');
    const revision = liveInputRevision(), base = previewPlan;
    const edits = paths.map(path => ({path,field:'disc',value:String(value)}));
    const plan = await invoke('preview_tag_edits', {edits,paths,cover:null});
    if (!isSelectionCurrent(subject)) throw new Error('The selection changed. Try again.');
    if (revision !== liveInputRevision() || base !== previewPlan) throw new Error('The staged values changed. Try again.');
    if (isFieldLocked('disc') || plan.locked_skipped?.some(item => item.field === 'disc')) throw new Error('Disc was locked during the preview. Nothing was changed.');
    if (plan.changes.some(change => change.tag_changes?.some(edit => edit.field === 'disc' && edit.invalid))) throw new Error('This disc number is not supported by the selected files.');
    if (options.mode === 'write') await writeQuickPlan(plan);
    else { adoptTagOnlyPreview(); recordLiveFields(edits); }
    return {dialog:'ribbon-number-dialog'};
  } else if (action === 'remove-tags') {
    if (options.mode === 'write') {
      const plan = await invoke(options.kind === 'text' ? 'preview_clear_tags' : 'preview_remove_tag_block', {paths, kind:options.kind});
      if (!isSelectionCurrent(subject)) throw new Error('The selection changed. Try again.');
      if (plan.changes.some(change => change.block_changes?.some(block => !block.exact))) {
        el('ribbon-remove-dialog').hidden = true;
        const ok = await confirmDialog('Remove the selected tag blocks now? Undo can restore text and artwork, but some private data in these blocks cannot be restored.', 'Write', {danger:true});
        if (!ok) return {dialog:'ribbon-remove-dialog'};
        if (!isSelectionCurrent(subject)) throw new Error('The selection changed. Try again.');
      }
      await writeQuickPlan(plan);
    } else if (options.kind === 'text') await previewClearTags();
    else {
      const label = trackAt(paths[0])?.tag_blocks?.find(block => block.kind === options.kind)?.label || options.kind;
      // Close the chooser before the existing detailed block-removal confirmation.
      el('ribbon-remove-dialog').hidden = true;
      await previewRemoveTagBlock(options.kind, label);
    }
    return {dialog:'ribbon-remove-dialog'};
  } else if (action === 'format-values') {
    const inputRevision = liveInputRevision();
    const inputPlan = previewPlan;
    const plan = await invoke('preview_field_operations', {
      paths, operations: [{kind:'format', field:options.field, pattern:options.pattern}],
      working: inputPlan,
    });
    if (inputRevision !== liveInputRevision() || inputPlan !== previewPlan) throw new Error('The staged values changed. Try again.');
    if (!isSelectionCurrent(subject)) throw new Error('The selection changed. Try again.');
    if (options.mode === 'write') {
      // Preserve the shared stage: immediate Write saves only this operation's
      // target field, while its expression still reads current staged inputs.
      const selected = new Set(paths);
      const changes = plan.changes.filter(change => selected.has(change.path)).map(change => ({
        ...change, rename_to:null, cover_change:null, block_changes:[], sidecar_renames:[],
        copy:false, folder_extras:[], tag_changes:change.tag_changes.filter(edit => edit.field === options.field),
      }));
      await writeQuickPlan({...plan, changes});
    } else {
      adoptTagOnlyPreview();
      const values = new Map(plan.changes.map(change => [change.path, change.tag_changes.find(edit => edit.field === options.field)]));
      recordLiveFields(paths.map(path => ({path, field:options.field,
        value:values.has(path) && values.get(path) ? values.get(path).new ?? '' : trackAt(path)?.tags?.[options.field] ?? '',
      })));
    }
    return {dialog:'ribbon-format-dialog'};
  } else if (action === 'import-artwork') {
    const result = await invoke('artwork_import_sources', {paths, pattern:options.pattern, firstImage:options.firstImage, replaceExisting:options.replaceArtwork});
    if (!isSelectionCurrent(subject)) throw new Error('The selection changed. Try again.');
    const plans = [];
    for (const assignment of result.assignments) {
      plans.push(await invoke('preview_cover_embed', {paths:assignment.paths, cover:assignment.cover}));
      if (!isSelectionCurrent(subject)) throw new Error('The selection changed. Try again.');
    }
    if (plans.length) {
      const plan = {...plans[0], changes:plans.flatMap(plan => plan.changes)};
      if (options.mode === 'write') await writeQuickPlan(plan);
      else adoptLiveCoverPlan(plan, result.assignments.flatMap(assignment => assignment.paths));
    }
    report('Artwork import prepared', [
      {text:`${result.assignments.reduce((sum,item) => sum + item.paths.length,0)} files matched`},
      result.missing.length && {text:`${result.missing.length} files had no matching image`, problem:true},
      result.skipped_existing.length && {text:`${result.skipped_existing.length} files kept their existing artwork`},
    ]);
    return {dialog:'ribbon-import-dialog'};
  }
});

// ---- wire up ----
// One button, two roles (#177): what it does is what its label says.
el("lib-action").addEventListener("click", () =>
  libraryIsDirty() ? (appendTypedFolder ? addFolderToList(rootInput.value) : openLibrary()) : browseForFolder()
);
el("lib-refresh").addEventListener("click", () => refreshLibrary());
el("root-display").addEventListener("click", editRootPath);
el("root-recent").addEventListener("click", (e) => {
  e.stopPropagation();
  toggleRecentMenu();
});
document.addEventListener("click", (e) => {
  const menu = el("root-recent-menu");
  if (!menu.hidden && !menu.contains(e.target)) closeRecentMenu();
});

applyBtn.addEventListener("click", apply);
undoBtn.addEventListener("click", undo);
redoBtn.addEventListener("click", redo);

// Same flow from the table toolbar, so clearing doesn't require opening EDITOR.
el("clear-tags").addEventListener("click", previewClearTags);
el("export-kind").addEventListener("click", (e) => {
  const btn = e.target.closest("[data-fmt]");
  if (btn) setExportKind(btn.dataset.fmt);
});
// TAGGER sub-tabs: ONLINE (a metadata source), EDITOR (tag fields + cover),
// FROM NAME (the file's own name, #139) — three ways to the same outcome.
function setSubtab(name) {
  if (name === "transformations") name = "editor";
  publishWorkspaceSubtab(name);
  requestAnimationFrame(() => {
    const tab = document.querySelector(`.subtab[data-subtab="${name}"]`);
    const rail = tab?.closest('.subtabs');
    if (!tab || !rail) return;
    const bounds = rail.getBoundingClientRect(), active = tab.getBoundingClientRect();
    if (active.left < bounds.left) rail.scrollLeft -= bounds.left - active.left;
    else if (active.right > bounds.right) rail.scrollLeft += active.right - bounds.right;
  });
  document.querySelectorAll(".subtab").forEach((t) => t.classList.toggle("active", t.dataset.subtab === name));
  document.querySelectorAll(".subtab-panel").forEach((p) => {
    p.hidden = p.id !== `subtab-${name}`;
  });
  // The probe is only computed while it's on screen; opening the tab is when it
  // has to catch up with the current selection and pattern.
  if (name === "editor") refreshFieldEditor();
  if (name === "filename") refreshNameProbe();
  if (name === "fromtext") refreshTextProbe();
  // Each sub-tab is its own chain context — ONLINE and FROM NAME have one,
  // EDITOR deliberately has none (#236).
  syncTransformPlacement();
}
connectWorkspaceController({ setMode, setSubtab });


document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || e.defaultPrevented || componentOwnsEscape(e)) return;
  // Settings sheet takes precedence.
  if (!el("settings").hidden) {
    cancelSettings();
    return;
  }
  // Esc interrupts an in-progress search sweep (#108) — e.g. the wanted release
  // is already visible. Don't hijack Esc while editing a cell.
  const editing = document.activeElement && document.activeElement.isContentEditable;
  if (!editing && searchBusy()) {
    stopLoading();
    e.preventDefault();
  }
});

// Esc lets go of a staged plan, as Discard (⌘⌫) does (#533), when nothing else
// Esc closes is open. Decided in the capture phase: the menus and popovers close
// themselves on Esc in their own handlers, and checked after them one press
// would close a menu and throw the plan away with it.
//
// While BPM detection runs, Esc stops it instead, as its Stop does (#539), and
// leaves alone any plan already staged: the run stages its own when it ends,
// and the next Esc lets go of that.
const ESC_POPUPS = [
  "file-ctx",
  "edit-ctx",
  "group-menu",
  "columns-menu",
  "ph-menu",
  "preset-menu",
  "query-preset-menu",
];
// Focusing or interacting with another control leaves the scrolled-out edit
// staged. Scrollbar interactions alone keep its cell-level Escape available.
document.addEventListener('focusin', event => {
  if (scrolledCellEdit && event.target !== el('tracks') && !event.target.closest('.table-scrollbar')) scrolledCellEdit = null;
}, true);
document.addEventListener('pointerdown', event => {
  if (scrolledCellEdit && !event.target.closest?.('.table-scrollbar')) scrolledCellEdit = null;
}, true);
window.addEventListener(
  "keydown",
  (e) => {
    if (e.key !== "Escape" || e.isComposing || componentOwnsEscape(e)) return;
    const detecting = bpmRunning();
    // A report or error waiting to be dismissed closes first (#562), so the
    // plan it describes isn't thrown away under it.
    const sticky = !el("toast").hidden && !el("toast-close").hidden;
    if (!diffByPath && !detecting && !sticky && !scrolledCellEdit) return;
    const active = document.activeElement;
    const typing =
      active && (active.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(active.tagName));
    const popupOpen =
      ESC_POPUPS.some((id) => el(id) && !el(id).hidden) ||
      [...document.querySelectorAll(".sg-menu, .modal-backdrop")].some((surface) => !surface.hidden) ||
      !el("settings").hidden;
    if (typing || popupOpen || searchBusy()) return;
    if (scrolledCellEdit) {
      const {path, field, value} = scrolledCellEdit;
      scrolledCellEdit = null;
      e.preventDefault();
      e.stopPropagation();
      if (trackAt(path)) {
        prepareTagCellEdit();
        recordRestoredCellValue(path, field, value);
      }
      return;
    }
    if (sticky) {
      e.preventDefault();
      dismissStickyToast();
      return;
    }
    if (detecting) {
      e.preventDefault();
      stopBpm();
      return;
    }
    const discard = el("diff-discard");
    if (discard && !discard.disabled) {
      e.preventDefault();
      discard.click();
    }
  },
  true,
);

// ⌘/Ctrl+A selects every row the table lists, not the whole page (#345). Without
// this the browser's own select-all fires and text-highlights the entire UI.
// The shortcut registry runs it (#432): it leaves a text field its own
// select-all, stays out while Settings or a dialog is open, and matches the
// physical key, so a Cyrillic layout no longer breaks it.
function selectAllRows() {
  // While diffing, the sel column is the apply scope (#117), so select all of it.
  if (usingReviewSelection()) {
    setApplySelection(new Set(diffByPath.keys()));
    syncSelectionUI();
    return;
  }
  for (const path of viewPaths) selection.add(path);
  syncSelectionUI();
}

// Open a native folder chooser (Tauri dialog plugin). The scanner recurses into
// subfolders, so picking a folder loads everything under it. Outside Tauri
// (browser dev) there's no native dialog — fall back to focusing the path field.
async function browseForFolder({ append = false } = {}) {
  appendTypedFolder = append;
  const dialog = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dialog) {
    // Browser development has no native chooser: open the field instead, which
    // is the only way in there.
    toast(msg("app.typePath"));
    editRootPath();
    return;
  }
  try {
    const picked = await dialog.open({ directory: true, multiple: false });
    if (!picked) return; // user cancelled
    rootInput.value = picked;
    if (append) await addFolderToList(picked);
    else await openLibrary();
  } catch (e) {
    toast(String(e), true);
  }
}

rootInput.addEventListener("input", updateLibAction);
rootInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") appendTypedFolder ? addFolderToList(rootInput.value) : openLibrary();
  // Escape abandons the edit: back to what is open, and out of the field.
  if (e.key === "Escape") {
    appendTypedFolder = false;
    rootInput.value = openedRoot;
    leaveRootEdit();
  }
});
// Leaving the field returns the indicator, unless what was typed is still
// unopened — that has to stay visible while the button offers to open it.
rootInput.addEventListener("blur", leaveRootEdit);
renderRootDisplay();
updateLibAction();
// The bar narrows with the window, so what fits changes with it (#182).
window.addEventListener("resize", fitRootDisplay);
el("root-recent").hidden = recentRoots().length === 0;
selectAll.addEventListener("change", () => {
  const on = selectAll.checked;
  // While diffing the header box toggles the whole apply scope, not selection.
  if (usingReviewSelection()) {
    setApplySelection(new Set(on ? diffByPath.keys() : []));
    syncSelectionUI();
    return;
  }
  // Every file the table lists, not just the rows on screen (#189) — including
  // the ones inside a collapsed folder, which select-all covered when they were
  // hidden rows too.
  for (const path of viewPaths) {
    if (on) selection.add(path);
    else selection.delete(path);
  }
  syncSelectionUI();
});
// ---- row selection ----
// Track selection uses file-list gestures and an immediate highlight.
// Tag editing and keyboard cell ranges remain separate from track selection.
let selAnchor = null; // path of the last row clicked, for Shift-range
// What a Shift-range is added to: the selection as a ⌘/Ctrl click left it,
// without the anchor. A file list keeps an earlier block when a second one is
// started with ⌘ and extended with Shift — select 1–25, ⌘-click 27, Shift-click
// 51 and both blocks stay; only the range from the anchor moves as Shift-clicks
// repeat. A plain click starts over with nothing under it.
let selBase = new Set();

// Select `base` plus the rows from the anchor through index `to`. Only what is
// still selected of the base counts, so a selection changed some other way since
// (Select all, a folder opened) is not brought back by a stale base.
function selectRangeFromAnchor(from, to) {
  const keep = [...selBase].filter((p) => selection.has(p));
  selection.clear();
  for (const p of keep) selection.add(p);
  const [lo, hi] = from <= to ? [from, to] : [to, from];
  for (let i = lo; i <= hi; i++) selection.add(viewPaths[i]);
}

// Make `path` the anchor of the next Shift-range, with the rest of the current
// selection as what that range is added to.
function anchorSelection(path) {
  selAnchor = path;
  selBase = new Set(selection);
  selBase.delete(path);
}

function usingReviewSelection() {
  return commandMode!=='duplicates'&&Boolean(diffByPath && !isAutomaticTransformPreview() && (stagedView || !editablePreview()));
}

function rowSelectionSet() {
  return usingReviewSelection() ? applySelection : selection;
}

let selectionPanelFrame = 0;
let selectionPanelTimer = 0;
let selectionPanelVersion = 0;
let selectionPanelSubject = null;
let selectionPanelsPending = false;
function cancelSelectionPanels() {
  selectionPanelVersion++;
  cancelAnimationFrame(selectionPanelFrame);
  clearTimeout(selectionPanelTimer);
  selectionPanelFrame = selectionPanelTimer = 0;
}
function scheduleSelectionPanels() {
  if (selectionPanelFrame || selectionPanelTimer) return;
  const version = selectionPanelVersion;
  // Let the highlight paint before mounting fields or preparing native work.
  selectionPanelFrame = requestAnimationFrame(() => {
    selectionPanelFrame = 0;
    selectionPanelTimer = setTimeout(() => {
      selectionPanelTimer = 0;
      if (version !== selectionPanelVersion || applying || undoing) return;
      selectionPanelsPending = false;
      try {
        if (currentMode === 'tagger') {
          refreshFieldEditor(); scheduleNameProbe(); scheduleTextProbe();
          if (!el('subtab-online').hidden) refreshReleaseMatches();
        }
        if (currentMode === 'renamer') scheduleMaskExample();
        scheduleAutomaticTransformPreview();
      } finally {
        el('fields-body').inert = false;
        el('fields-body').removeAttribute('aria-busy');
      }
    }, 0);
  });
}
function paintRowSelection() {
  const selected = rowSelectionSet();
  for (const [path, row] of renderedRows) {
    const on = selected.has(path);
    if (row.classList.contains('selected') !== on) row.classList.toggle('selected', on);
    if (row.getAttribute('aria-selected') !== String(on)) row.setAttribute('aria-selected', String(on));
  }
}
function syncSelectionUI({ deferPanels = false, refreshGrid = true, refreshPanels = false } = {}) {
  syncRibbonSelection();
  paintRowSelection();
  publishDuplicateSelection();
  updateStatus(rowSelectionSet().size);
  if (usingReviewSelection()) {
    updateDiffBar();
    if (refreshGrid) spreadsheet?.refresh({selectionOnly:true});
    return;
  }
  if (!deferPanels) {
    const paths = selectedPaths();
    let checked = 0;
    for (const path of paths) if (viewPathIndex.has(path)) checked++;
    selectAll.checked = checked > 0 && checked === viewPaths.length;
    selectAll.indeterminate = checked > 0 && checked < viewPaths.length;
    updatePanelCounts(paths.length);
  }
  if (refreshGrid) spreadsheet?.refresh({selectionOnly:true});
  if(commandMode==='duplicates'){
    // Reviewing a duplicate group changes selection, never a staged Apply scope
    // or an editor preview that the user left in another mode.
    if(!deferPanels)scheduleSessionSave();
    return;
  }
  let scopeAnnounced = false;
  if (!isAutomaticTransformRefresh()) {
    if (refreshPanels) selectionPanelsPending = true;
    const subject = `${selectionVersion()}:${contentRevision}`;
    if (subject !== selectionPanelSubject) {
      selectionPanelSubject = subject;
      selectionPanelsPending = true;
      cancelSelectionPanels();
      invalidateAutomaticTransformPreview({selectionOnly:true});
      scopeAnnounced = true; // Its public event updates the scope synchronously.
      // Old controls cannot edit a previous subject while the new view is pending.
      el('fields-body').inert = true;
      el('fields-body').setAttribute('aria-busy', 'true');
      el('cover-well').inert = true;
    }
    if (selectionPanelsPending) {
      if (deferPanels) cancelSelectionPanels();
      else scheduleSelectionPanels();
    }
  }
  if (!deferPanels) {
    if (!scopeAnnounced) updateTransformScope();
    scheduleSessionSave();
  }
}

// Scope belongs to the app's quiet status rail, not a second transform toolbar.
function updateTransformScope() {
  const note = el('status-transform-scope');
  if (!note) return;
  const scope = automaticTransformScope();
  const text = scope.busy ? '' : scope.invalid ? 'Fix invalid values' : scope.failed ? 'Preview failed' : scope.blocked ? 'Review current preview first' : '';
  // Preview state controls validation, never a second notification strip.
  note.hidden = true;
  note.dataset.detail = String(Boolean(text));
  if (note.textContent !== text) note.textContent = text;
  note.setAttribute('aria-busy', String(scope.busy));
  if (isAutomaticTransformPreview()) updateDiffBar(scope);
}

document.addEventListener('tagrex:transform-scope', updateTransformScope);

// Selection-dependent counts in the GENERATOR/EXPORTER panel headings. Cheap
// text updates safe to run on every selection change (the panels stay in the
// DOM even while hidden); the full panel refresh still happens on mode entry.
function updatePanelCounts(count = selectedPaths().length) {
  // One noun for one selection in every head (#593).
  el("transform-count").textContent = count ? `· ${tn("count.selected", count)}` : "";
  el("autonum-count").textContent = count ? `· ${tn("count.selected", count)}` : "";
  el("vinyl-count").textContent = count ? `· ${tn("count.selected", count)}` : "";
  el("export-count").textContent = count ? `· ${tn("count.selected", count)}` : "";
}

function selectRow(tr, e, { deferPanels = false } = {}) {
  if(duplicateResultsActive()){
    const key=viewItems[itemIndexByPath.get(tr.dataset.path)]?.groupKey;
    if(selectDuplicateGroup(key)){setActiveRow(tr,!deferPanels);if(deferPanels)tr.focus({preventScroll:true});}
    return;
  }
  if (usingReviewSelection()) { selectReviewRow(tr, e); return; }
  if (tr.classList.contains("unreadable")) return; // inert — can't be selected
  const path = tr.dataset.path;
  if (e.shiftKey && selAnchor) {
    // The range runs over the model's order, so it spans files whose rows are
    // off screen or inside a collapsed folder (#189).
    const b = viewPathIndex.get(path);
    const a = viewPathIndex.has(selAnchor) ? viewPathIndex.get(selAnchor) : b;
    selectRangeFromAnchor(a, b);
  } else if (e.metaKey || e.ctrlKey) {
    if (selection.has(path)) selection.delete(path);
    else selection.add(path);
    anchorSelection(path);
  } else {
    if (selection.size !== 1 || !selection.has(path)) { selection.clear(); selection.add(path); }
    anchorSelection(path);
  }
  syncSelectionUI({deferPanels, refreshGrid:!deferPanels});
  setActiveRow(tr, !deferPanels);
  if (deferPanels) tr.focus({preventScroll:true}); // immediate focus without a window/layout pass
}

let reviewAnchor = null;
let reviewBase = new Set();
function selectReviewRow(tr, event) {
  const path = tr.dataset.path;
  if (!diffByPath.has(path)) return;
  if (event.shiftKey && reviewAnchor && viewPathIndex.has(reviewAnchor)) {
    const first = viewPathIndex.get(reviewAnchor), last = viewPathIndex.get(path);
    const keep = [...reviewBase].filter(path => applySelection.has(path));
    applySelection.clear();
    for (const path of keep) applySelection.add(path);
    for (const path of viewPaths.slice(Math.min(first,last), Math.max(first,last)+1)) if (diffByPath.has(path)) applySelection.add(path);
  } else {
    if (event.ctrlKey || event.metaKey) {
      if (applySelection.has(path)) applySelection.delete(path); else applySelection.add(path);
    } else { applySelection.clear(); applySelection.add(path); }
    reviewAnchor = path; reviewBase = new Set(applySelection); reviewBase.delete(path);
  }
  syncSelectionUI({refreshGrid:false});
  setActiveRow(tr, false); tr.focus({preventScroll:true});
}

// A plain group right-click replaces; Ctrl/Cmd-click toggles the complete
// filtered group, including collapsed/offscreen members, and preserves others.
function selectGroup(key, mode = 'replace') {
  if(duplicateResultsActive()){selectDuplicateGroup(key);return;}
  const manual = usingReviewSelection();
  const paths = (groupPaths.get(key) || []).filter(path => !manual || diffByPath.has(path));
  if (!paths.length) return;
  const target = rowSelectionSet();
  if (mode === 'toggle') {
    const remove = paths.every(path => target.has(path));
    for (const path of paths) { if (remove) target.delete(path); else target.add(path); }
  } else {
    if (target.size !== paths.length || paths.some(path => !target.has(path))) { target.clear(); for (const path of paths) target.add(path); }
    if (manual) { reviewAnchor = paths[0]; reviewBase = new Set(target); reviewBase.delete(reviewAnchor); }
    else anchorSelection(paths[0]);
  }
  spreadsheet?.clearRange({refresh:false});
  syncSelectionUI();
}

function selectDuplicateGroup(key){
  const group=duplicateGroupLookup.get(key);
  if(!duplicateResultsActive()||!group)return false;
  selectedDuplicateGroupId=group.id;
  if(!selectionMatchesDuplicate(group)){selection.clear();for(const path of group.paths)selection.add(path);}
  anchorSelection(group.paths[0]);
  spreadsheet?.clearRange({refresh:false});
  syncSelectionUI();
  return true;
}

// Enter edit mode on a cell and select its text (double-click, per the hint).
function beginCellEdit(cell) {
  if(commandMode==='duplicates')return;
  if (cell.dataset.field) {
    const field = diffByPath?.get(cell.dataset.path)?.tag_changes.find(change => change.field === cell.dataset.field);
    cell.textContent = field ? field.new ?? '' : currentFieldValue(cell.dataset.path,cell.dataset.field);
  }
  cell.dataset.editOriginal = cell.textContent;
  cell.dataset.editingSession = 'active';
  cell.contentEditable = "true";
  cell.focus();
  // Read the library's values for this column once per edit, not per keystroke
  // (#63). Nothing shows until something is typed.
  openCellSuggest(cell);
  const range = document.createRange();
  range.selectNodeContents(cell);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
}

tracksBody.addEventListener("click", (e) => {
  if (e.target.closest("button, input, textarea, select, a")) return;
  if (e.target.closest("td.sel")) return; // checkbox toggle → change listener
  const groupHead = e.target.closest("tr.group-head");
  if (groupHead) {
    // Group clicks have their own collapse/selection handler.
    return;
  }
  const tr = e.target.closest("tr");
  if (!tr || !tr.dataset.path) return;
  const cell = e.target.closest("td.editable");
  if (cell && cell.isContentEditable) return; // mid-edit: don't reselect
  if (e.detail === 0||duplicateResultsActive()) selectRow(tr, e);
});

// A plain click in the empty area below the rows deselects, like a file manager
// (#346) — the counterpart to ⌘/Ctrl+A. The click lands on the `#files-view`
// background (few files, no bottom spacer) or a windowing spacer row; clicks on
// a real row, group header, checkbox or the column header are left to their own
// handlers, and a modified click keeps the selection for range/add.
el("files-view").addEventListener("click", (e) => {
  if (usingReviewSelection()) return;
  if (e.metaKey || e.ctrlKey || e.shiftKey) return;
  if (e.target.closest("thead") || e.target.closest("td.sel")) return;
  const tr = e.target.closest("tr");
  if (tr && (tr.dataset.path || tr.classList.contains("group-head"))) return;
  if (selection.size) {
    rowSelectionSet().clear();
    selAnchor = null;
    syncSelectionUI();
  }
});

// Calibrate model offsets from a real rendered row, then binary-search the
// virtual model. The drag can therefore span rows that have left the DOM.
function marqueePaths(top, bottom, viewportTop, scrollTop) {
  const reference = renderedRows.entries().next().value;
  if (!reference) return [];
  const [path, row] = reference;
  const index = itemIndexByPath.get(path);
  if (index === undefined) return [];
  const origin = row.getBoundingClientRect().top - viewportTop + scrollTop - itemOffsets.top(index);
  const from = Math.max(0, top - origin);
  const to = bottom - origin;
  if (to < 0) return [];
  const paths = [];
  const first = itemIndexAt(from);
  let itemTop = itemOffsets.top(first);
  for (let i = first; i < viewItems.length && itemTop <= to; i++) {
    const item = viewItems[i];
    itemTop += itemHeightAt(i);
    if (item.kind === "track" && !item.track.unreadable && itemTop >= from) paths.push(item.track.path);
  }
  return paths;
}
initRowSelection({
  wrap: tableWrap, getSelection: rowSelectionSet,
  enabled: () => !applying && !undoing && !duplicateResultsActive(),
  pathsInRange: (top, bottom, viewportTop, scrollTop) => {
    const paths = marqueePaths(top, bottom, viewportTop, scrollTop);
    return usingReviewSelection() ? paths.filter(path => diffByPath.has(path)) : paths;
  },
  onPress: (event) => {
    const before = {selAnchor,selBase,reviewAnchor,reviewBase,activeRowPath};
    const row = event.target.closest('tr[data-path]');
    if (row) selectRow(row, event, {deferPanels:true});
    else if (!usingReviewSelection()) { rowSelectionSet().clear(); syncSelectionUI({deferPanels:true,refreshGrid:false}); }
    return before;
  },
  onScroll: () => renderWindow({scrollOnly:true}),
  onStart: () => {
    selectionMarqueeActive = true;
    selectionSummaryAnimation?.finish();
    selectionSummaryWidthAnimation?.cancel();
    selectionSummaryWidthAnimation = null;
    selectionSummaryCountAnimation?.cancel();
    selectionSummaryCountAnimation = null;
    spreadsheet?.clearRange();
  },
  onChange: () => {
    // The press invalidates stale editor controls. Publish the command scope
    // on release and update only the table and its selection count during drag.
    paintRowSelection();
    updateSelectionSummary(rowSelectionSet().size, true);
  },
  onCommit: ({ cancelled, beforePress, paths, startPath, focusPath }) => {
    selectionMarqueeActive = false;
    if (cancelled && beforePress) {
      ({selAnchor,selBase,reviewAnchor,reviewBase} = beforePress);
      setActiveRowByPath(beforePress.activeRowPath, false);
    }
    if (!cancelled && paths.length) {
      if (usingReviewSelection()) { reviewAnchor = startPath || paths[0]; reviewBase = new Set(applySelection); reviewBase.delete(reviewAnchor); }
      else anchorSelection(startPath || paths[0]);
      setActiveRowByPath(focusPath, false);
      renderedRow(focusPath)?.focus({ preventScroll: true });
    }
    syncSelectionUI();
  },
});

tracksBody.addEventListener("dblclick", (e) => {
  if (!editablePreview()) return;
  if (e.target.closest("button, input, textarea, select, a")) return;
  if (e.target.closest('tr.group-head') || e.ctrlKey || e.metaKey || e.shiftKey) return;
  // Double-clicking the file name plays the track (the per-row play button was
  // removed, #99 redesign); tag cells still double-click to edit.
  const fileCell = e.target.closest("td.file");
  if (fileCell) {
    const tr = e.target.closest("tr");
    if (tr && tr.dataset.path && !tr.classList.contains("unreadable")) {
      playTrack(tr.dataset.path);
    }
    return;
  }
  const cell = e.target.closest("td.editable");
  if (cell && !cell.classList.contains("locked")) {
    beginCellEdit(cell);
  }
});

// contentEditable is turned off again when a cell loses focus (blur doesn't
// bubble, so listen in the capture phase).
tracksBody.addEventListener(
  "blur",
  (e) => {
    if (e.target.classList?.contains("editable") && e.target.isContentEditable) {
      e.target.contentEditable = "false";
      endCellSuggest(); // the suggestion list belongs to the edit (#63)
      commitCellEdit(e.target);
    }
  },
  true,
);

// ---- keyboard row navigation (roving tabindex) ----
// Exactly one data row is tabbable (tabindex 0); ↑/↓ move focus between visible
// rows and Space toggles the focused row's selection. This makes the row focus
// ring (states.css) reachable for a keyboard-heavy tool.

// Keep exactly one row tabbable; called after every render. The anchor is a
// PATH, not a row (#189): the row it names may be scrolled out of the window,
// and it has to survive that.
function refreshRoving() {
  if (navPaths.length === 0) {
    setActiveRowPath(null);
    return;
  }
  if (!navPathIndex.has(activeRowPath)) setActiveRowPath(navPaths[0]);
  for (const [path, tr] of renderedRows) tr.tabIndex = path === activeRowPath ? 0 : -1;
}

// Use the complete view model rather than mounted rows: grouping can place
// files in a different order from the filename-sorted library array. Selected
// paths outside the current filter retain their captured order at the end.
function orderPathsForDisplay(paths) {
  return [...paths].sort((a, b) =>
    (viewPathIndex.get(a) ?? Infinity) - (viewPathIndex.get(b) ?? Infinity));
}

// Scroll `path` into view when it is outside the window, so a row reached by
// keyboard exists to be focused. The sticky header sits over the top of the
// scroller, so a row arriving from above is placed below it rather than under it.
function scrollPathIntoView(path) {
  const index = itemIndexByPath.get(path);
  if (index === undefined) return;
  const headHeight = el("tracks").querySelector("thead")?.offsetHeight || 0;
  const top = itemOffsets.top(index);
  const bottom = top + itemHeightAt(index);
  const viewTop = tableWrap.scrollTop + headHeight;
  const viewBottom = tableWrap.scrollTop + tableWrap.clientHeight;
  if (top < viewTop) tableWrap.scrollTop = Math.max(0, top - headHeight);
  else if (bottom > viewBottom) tableWrap.scrollTop += bottom - viewBottom;
  renderWindow();
}

// Make `path` the keyboard anchor, and focus its row when asked — bringing it on
// screen first if the window doesn't hold it.
function setActiveRowByPath(path, focus) {
  setActiveRowPath(path);
  if (focus && path) scrollPathIntoView(path);
  for (const [p, tr] of renderedRows) {
    const tabIndex = p === activeRowPath ? 0 : -1;
    if (tr.tabIndex !== tabIndex) tr.tabIndex = tabIndex;
  }
  if (focus && path) renderedRow(path)?.focus();
}

function setActiveRow(tr, focus) {
  setActiveRowByPath(tr ? tr.dataset.path : null, focus);
}

// Point the table at one file: select it alone, make it the keyboard anchor and
// bring it on screen. What the player's title does when clicked (#216) — after
// a few minutes of listening, "which row is this?" is a real question, and the
// bar is the only thing that knows the answer.
//
// Returns false when the file is not in the open library at all; a file that is
// there but hidden by the filter or inside a collapsed folder still selects,
// since the selection is the model's and not the DOM's (#189) — it simply has no
// row to scroll to.
function revealPath(path) {
  if (!path || !trackAt(path)) return false;
  selection.clear();
  selection.add(path);
  setActiveRowByPath(path, true);
  syncSelectionUI();
  return true;
}

tracksBody.addEventListener("keydown", (e) => {
  // Don't hijack keys while editing a cell or typing in a control.
  if (e.target.isContentEditable || e.target.matches("input, textarea, select")) return;
  const tr = e.target.closest("tr");
  if (!tr || !tr.dataset.path) return;
  if(duplicateResultsActive()&&(e.key==='Enter'||e.key===' ')){
    e.preventDefault();e.stopImmediatePropagation();selectRow(tr,e);return;
  }
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const i = navPathIndex.get(tr.dataset.path);
    const next = i === undefined ? undefined : navPaths[e.key === "ArrowDown" ? i + 1 : i - 1];
    if (next) {
      if (!usingReviewSelection() && !e.ctrlKey && !e.metaKey) {
        if (e.shiftKey && selAnchor) selectRangeFromAnchor(navPathIndex.get(selAnchor) ?? i, navPathIndex.get(next));
        else { selection.clear(); selection.add(next); anchorSelection(next); }
        syncSelectionUI({refreshGrid:false});
      }
      setActiveRowByPath(next, true);
    }
  } else if (e.key === " ") {
    e.preventDefault(); // Space would otherwise scroll
    const path = tr.dataset.path;
    // While diffing, Space toggles the focused staged row's apply tick.
    if (usingReviewSelection()) {
      if (!diffByPath.has(path)) return; // untouched rows aren't tickable
      if (applySelection.has(path)) applySelection.delete(path);
      else applySelection.add(path);
      syncSelectionUI();
      return;
    }
    if (selection.has(path)) selection.delete(path);
    else selection.add(path);
    anchorSelection(path);
    syncSelectionUI();
  }
});


function stopWatchingSortMotion() {
  tableWrap.removeEventListener('scroll', cancelSortOnScroll);
  tableWrap.removeEventListener('wheel', cancelSortMotion);
  tableWrap.removeEventListener('pointerdown', cancelSortMotion, true);
  window.removeEventListener('resize', cancelSortMotion);
  sortReducedMotion.removeEventListener('change', cancelSortMotion);
}
function cancelSortMotion() {
  if (!sortAnimations.size) return;
  for (const animation of sortAnimations) animation.cancel();
  sortAnimations.clear();
  stopWatchingSortMotion();
}
function cancelSortOnScroll() {
  if (tableWrap.scrollTop !== sortScrollTop || tableWrap.scrollLeft !== sortScrollLeft) cancelSortMotion();
}

function visibleSortRows() {
  const bounds = tableWrap.getBoundingClientRect();
  const top = Math.max(bounds.top + tableWrap.clientTop, tableWrap.querySelector('thead').getBoundingClientRect().bottom);
  const bottom = bounds.top + tableWrap.clientTop + tableWrap.clientHeight;
  const rows = new Map();
  for (const row of motionViewportRows()) {
    if (!row?.dataset.path) continue;
    const rect = row.getBoundingClientRect();
    if (rect.bottom > top && rect.top < bottom) {
      rows.set(row.dataset.path, {row, top: rect.top, height: rect.height});
    }
  }
  return {rows, top, bottom};
}

// Commit the final file order immediately. Only the visible rows play back
// their displacement, with newly visible rows starting at the viewport edge.
function sortBy(key) {
  cancelSortMotion();
  const before = !sortReducedMotion.matches && typeof Element.prototype.animate === 'function' ? visibleSortRows() : null;
  const previousLayout = before?.rows.size ? motionRowLayout() : null;
  applySort(key, sortKey === key ? -sortDir : 1, {reorderRows:true});
  if (!previousLayout) return;
  sortScrollTop = tableWrap.scrollTop;
  sortScrollLeft = tableWrap.scrollLeft;
  const after = visibleSortRows();
  let moving = 0;
  for (const [path, {row, top, height}] of after.rows) {
    const prior = before.rows.get(path);
    const previousTop = prior?.top ?? previousLayout.rowTop(row);
    if (previousTop === undefined) continue;
    const start = prior ? previousTop : Math.min(after.bottom, Math.max(after.top - height, previousTop));
    const offset = start - top;
    if (Math.abs(offset) <= 1) continue;
    const animation = row.animate(
      [{transform:`translateY(${offset}px)`}, {transform:'translateY(0)'}],
      {duration:230, delay:Math.min(moving++ * 4, 48), easing:'cubic-bezier(.3,0,.25,1)', fill:'backwards'},
    );
    sortAnimations.add(animation);
    const finished = () => {
      if (sortAnimations.delete(animation) && !sortAnimations.size) stopWatchingSortMotion();
    };
    animation.finished.then(finished, finished);
  }
  if (!sortAnimations.size) return;
  tableWrap.addEventListener('scroll', cancelSortOnScroll, {passive:true});
  tableWrap.addEventListener('wheel', cancelSortMotion, {passive:true});
  tableWrap.addEventListener('pointerdown', cancelSortMotion, {capture:true, passive:true});
  window.addEventListener('resize', cancelSortMotion, {passive:true});
  sortReducedMotion.addEventListener('change', cancelSortMotion);
}

document.addEventListener('tagrex:sort-artwork', () => sortBy('artwork'));

// The toggle-free core is shared with preset restore.
function applySort(key, dir, {reorderRows = false} = {}) {
  setSortKey(key);
  setSortDir(dir);
  sortTracks();
  renderTracks({reorderRows});
}

// Put `tracks` in the active sort's order. Separate from applySort because a
// freshly read list has to be sorted again without re-deciding what the sort is
// (#196): an apply or an undo replaces the list with what is on disk, in scan
// order, while the header still claims a sort — so everything jumped, the folder
// being worked on included, and nothing said why.
function sortTracks() {
  if (!sortKey) return;
  // Whatever was placed by hand is placed by the sort now (#198) — a re-read
  // from disk is where a hand-made order ends, and so is a fresh sort.
  setManualOrder(false);
  // Build expensive display keys once for this sort, without retaining stale
  // keys after edits. Preserve the library array and each track object.
  const folderOrder = folderOrderForSort();
  const keyed = tracks.map((track) => ({
    track,
    folderRank: folderOrder?.get(groupKeyOf(track)),
    value: sortValue(track, sortKey),
    unknown: sortKey === "artwork" && coverState(track) === "unknown",
  }));
  keyed.sort((a, b) => {
    if (folderOrder && a.folderRank !== b.folderRank) return a.folderRank - b.folderRank;
    if (a.unknown !== b.unknown) return Number(a.unknown) - Number(b.unknown);
    return numericCollator.compare(a.value, b.value) * sortDir;
  });
  for (let i = 0; i < keyed.length; i++) tracks[i] = keyed[i].track;
  reindexTracks();
}

function orderSelectedFiles(paths, order) {
  if (applying || undoing || paths.length !== order.length) return false;
  const requested = new Set(paths);
  if (requested.size !== order.length || new Set(order).size !== order.length || order.some(path => !requested.has(path) || !trackAt(path))) return false;
  const byPath = new Map(tracks.map(track => [track.path, track]));
  let next = 0;
  setTracks(tracks.map(track => requested.has(track.path) ? byPath.get(order[next++]) : track));
  setManualOrder(true);
  renderTracks();
  syncSelectionUI();
  return true;
}

function handOrder(renamed) {
  return captureFileOrder(tracks, manualOrder, renamed);
}

function sortKeepingHandOrder(order) {
  sortTracks();
  if (!order) return;
  const placed = restoreFileOrder(tracks, order);
  for (let i = 0; i < placed.length; i++) tracks[i] = placed[i];
  reindexTracks();
  setManualOrder(true);
}

// Sort clicks are delegated on the header so dynamically-built columns (#43)
// stay sortable.
el("tracks").querySelector("thead").addEventListener("click", (e) => {
  if (e.target.closest(".col-resize")) return; // grip click is a resize, not a sort
  const th = e.target.closest("th.sortable");
  if (th) sortBy(th.dataset.sort);
});

// Column picker popover (#43): toggle, drag-reorder, and outside-click close.
let columnsMenuOrigin = null;
let columnsMenuResize = 0;
function positionColumnsMenu() {
  const menu = el('columns-menu');
  if (menu.hidden || !columnsMenuOrigin) return;
  placeFloating(menu, columnsMenuOrigin);
  menu.style.setProperty('--column-menu-height', menu.style.maxHeight);
}
function openColumnsMenu(point = null, anchor = el('tracks').tHead.querySelector('th.sortable')) {
  const menu = el('columns-menu');
  renderColumnsMenu();
  document.body.append(menu);
  menu.classList.add('columns-at-pointer');
  menu.returnFocusTo = anchor;
  menu.hidden = false;
  columnsMenuOrigin = point ? {getBoundingClientRect:() => {
    const x = Math.max(8, Math.min(point.x, innerWidth - 8));
    const y = Math.max(8, Math.min(point.y, innerHeight - 8));
    return {left:x,right:x,top:y,bottom:y};
  }} : anchor;
  queueMicrotask(positionColumnsMenu);
}
window.addEventListener('resize', () => {
  if (el('columns-menu').hidden || columnsMenuResize) return;
  columnsMenuResize = requestAnimationFrame(() => { columnsMenuResize = 0; positionColumnsMenu(); });
}, {passive:true});
el('tracks').tHead.addEventListener('keydown', event => {
  const header = event.target.closest('th.sortable');
  if (!header) return;
  if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
    event.preventDefault(); event.stopPropagation(); openColumnsMenu(null, header);
  } else if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault(); sortBy(header.dataset.sort);
  }
});
document.addEventListener("click", (e) => {
  const menu = el("columns-menu");
  // A visibility toggle rebuilds its row during this click. The original event
  // path still identifies an inside click after its target is disconnected.
  if (!menu.hidden && !e.composedPath().includes(menu)) {
    menu.hidden = true;
  }
});
// Grouping is a view overlay — changing it only re-renders, never reorders
// `tracks`. Collapsed state is per grouping, so setGroupBy resets it.
el("group-btn").addEventListener("click", (e) => {
  e.stopPropagation();
  const menu = el("group-menu");
  menu.hidden = !menu.hidden;
  el('group-btn').setAttribute('aria-expanded', String(!menu.hidden));
  if (!menu.hidden) placeFloating(menu, el('group-btn'));
});
el("group-menu").addEventListener("click", (e) => {
  const more = e.target.closest('.group-more');
  if (more) {
    const options = el('group-more-options');
    options.hidden = !options.hidden;
    more.setAttribute('aria-expanded', String(!options.hidden));
    placeFloating(el('group-menu'), el('group-btn'));
    return;
  }
  const opt = e.target.closest(".group-opt");
  if (!opt) return;
  el("group-menu").hidden = true;
  el('group-btn').setAttribute('aria-expanded', 'false');
  const previous = groupBy;
  setGroupBy(opt.dataset.group, {rerender:false});
  // A folder sort orders the model by folder before the column. Leaving that
  // mode must reapply the active column globally, including unmounted rows.
  const folderSortChanged = (previous === 'folder' || previous === 'drop') !== (groupBy === 'folder' || groupBy === 'drop');
  if (folderSortChanged && !manualOrder) sortTracks();
  renderTracks();
  el("group-btn").focus();
});
document.addEventListener("click", (e) => {
  const menu = el("group-menu");
  if (!menu.hidden && !menu.contains(e.target) && !e.target.closest("#group-btn")) menu.hidden = true;
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !componentOwnsEscape(e)) el("group-menu").hidden = true;
});

// Collapse/expand a group by clicking its header. Which rows exist is a model
// question now (#189): a collapsed folder's tracks leave the rendered list
// entirely rather than being rows with `display: none`. The selection is a set,
// so it survives the rebuild untouched.
function motionViewportRows() {
  const first = Math.max(renderedFirst, itemIndexAt(tableWrap.scrollTop) - 1);
  const last = Math.min(renderedLast, itemIndexAt(tableWrap.scrollTop + tableWrap.clientHeight) + 1);
  const rows = [];
  for (let index = first; index <= last; index++) rows.push(tracksBody.children[index - renderedFirst + 1]);
  return rows;
}
function motionRowLayout() {
  const offsets = new RowHeights();
  offsets.tree = itemOffsets.tree;
  const items = viewItems, tracks = itemIndexByPath, groups = groupItemIndices;
  const origin = tracksBody.getBoundingClientRect().top;
  const top = index => index === undefined ? undefined : origin + offsets.top(index);
  return {
    rowTop(row) {
      if (!row.dataset.path) return top(groups.get(row.dataset.group));
      const index = tracks.get(row.dataset.path);
      if (items[index]?.groupKey === (row.dataset.group ?? null)) return top(index);
    },
    groupTop(key) { return top(groups.get(key)); },
  };
}
groupMotion = createGroupMotion(tableWrap, motionViewportRows, motionRowLayout);
function toggleGroup(key) {
  cancelSortMotion();
  const collapse = !collapsedGroups.has(key);
  const affected = new Set([key]);
  groupMotion.run(() => {
    spreadsheet?.clearRange({refresh:false});
    if (collapse) collapsedGroups.add(key); else collapsedGroups.delete(key);
    updateGroupVisibility(key, collapse);
    renderWindow({force:true,toggledGroups:affected}); syncGroupToggle();
    const index = groupItemIndices.get(key);
    if (index >= renderedFirst && index <= renderedLast) tracksBody.children[index - renderedFirst + 1]?.focus({preventScroll:true});
  }, {group:key, collapsing:collapse, affected});
}
tracksBody.addEventListener('click', event => {
  const head = event.target.closest('tr.group-head');
  if (!head || applying || undoing) return;
  if(duplicateResultsActive()){
    if(event.target.closest('.group-caret'))toggleGroup(head.dataset.group);
    else {selectDuplicateGroup(head.dataset.group);head.focus({preventScroll:true});}
  }
  else if (event.ctrlKey || event.metaKey) selectGroup(head.dataset.group, 'toggle');
  else toggleGroup(head.dataset.group);
});
tracksBody.addEventListener('keydown', event => {
  const head = event.target.closest('tr.group-head');
  if (!head || applying || undoing) return;
  if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
    event.preventDefault(); event.stopPropagation(); selectGroup(head.dataset.group, 'replace');
  } else if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault(); event.stopPropagation();
    if(duplicateResultsActive())selectDuplicateGroup(head.dataset.group);
    else if (event.ctrlKey || event.metaKey) selectGroup(head.dataset.group, 'toggle');
    else toggleGroup(head.dataset.group);
  } else if(duplicateResultsActive()&&(event.key==='ArrowLeft'||event.key==='ArrowRight')){
    event.preventDefault();event.stopPropagation();
    if(collapsedGroups.has(head.dataset.group)===(event.key==='ArrowRight'))toggleGroup(head.dataset.group);
  }
});

// Expand/collapse every group at once (#32), through the same model rebuild as
// an individual header.
function setAllGroupsCollapsed(collapse) {
  const affected = new Set(viewGroups.filter(key => collapsedGroups.has(key) !== collapse));
  if (!affected.size) return;
  groupMotion.run(() => {
    const items = [], heights = [];
    for (const key of viewGroups) {
      const index = groupItemIndices.get(key);
      items.push(viewItems[index]);
      heights.push(itemHeights[index]);
      if (collapse) continue;
      const wasOpen = !collapsedGroups.has(key);
      for (const [offset, item] of groupItems.get(key).entries()) {
        items.push(item);
        heights.push(wasOpen ? itemHeights[index + offset + 1] : null);
      }
    }
    collapsedGroups.clear();
    if (collapse) for (const key of viewGroups) collapsedGroups.add(key);
    viewItems = items;
    itemHeights = heights;
    reindexGroupVisibility();
    windowGroupChanges = true;
    renderWindow({force:true,toggledGroups:affected});
  }, {collapsing:collapse, affected});
}
// The two Expand all / Collapse all buttons collapsed into one state toggle:
// while any group is still open it offers "collapse", and once everything is
// shut it flips to "expand". Icon + tooltip are re-derived from the model rather
// than a remembered flag, so per-group clicks keep it honest.
function syncGroupToggle() {
  const btn = el("toggle-groups");
  if (!btn || btn.hidden) return;
  const anyOpen = viewGroups.some((key) => !collapsedGroups.has(key));
  const label = anyOpen ? msg("toolbar.collapseGroups") : msg("toolbar.expandGroups");
  const collapse = anyOpen ? "1" : "0";
  if (btn.dataset.collapse !== collapse) {
    btn.innerHTML = ico(anyOpen ? "collapse-all" : "expand-all");
    btn.dataset.collapse = collapse;
  }
  if (btn.title !== label) btn.title = label;
  if (btn.getAttribute("aria-label") !== label) btn.setAttribute("aria-label", label);
}
el("toggle-groups").addEventListener("click", () => {
  cancelSortMotion();
  setAllGroupsCollapsed(el("toggle-groups").dataset.collapse !== "0");
  syncGroupToggle();
});

// Build large query matches in bounded chunks, keeping the current table
// usable until a complete latest-generation replacement is ready. Any table
// mutation, query change or content revision cancels the pending derivation.
async function renderFilteredTracks() {
  if(filterError){filterPending=false;publishFilterState();updateDiffBar();return;}
  const version=++filterRenderVersion, revision=contentRevision, source=tracks;
  scheduledQueryRevision=revision;
  if(!hasLibraryQuery()) {
    libraryQueryClient.cancel();nativeMatchPaths=null;duplicateGroups=[];duplicateMessage='';nativeMatchRevision=revision;filterPending=false;renderTracks();return;
  }
  filterPending=true;publishFilterState();updateDiffBar();
  try {
    const model={...filterModel(),coverLabels:{embedded:msg('table.artwork.embedded'),missing:msg('table.artwork.missing'),unknown:msg('table.artwork.unknown')}};
    const key=JSON.stringify([revision,model]);
    const result=await libraryQueryClient.run(model);
    if(!result||version!==filterRenderVersion||revision!==contentRevision||source!==tracks)return;
    const preserve=commandMode==='duplicates'&&duplicateQueryKey===key&&['running','complete'].includes(result.duplicateStatus);
    nativeMatchPaths=new Set(result.matchedPaths);duplicateGroups=commandMode==='duplicates'?orderedDuplicateGroups(result.groups):result.groups;duplicateStatus=result.duplicateStatus;
    if(duplicateResultsActive())reconcileDuplicateSelection(duplicateGroups);
    if(commandMode==='duplicates'){
      duplicateQueryKey=key;duplicateRenderedRevision=result.groupsRevision??duplicateLatestRevision;
      duplicateLatestRevision=Math.max(duplicateLatestRevision,duplicateRenderedRevision);
      duplicateFoundCount=result.groupsCount??Math.max(duplicateFoundCount,result.groups.length);
      const count=duplicateFoundCount===result.groups.length?`${duplicateFoundCount} duplicate groups`:`${result.groups.length} of ${duplicateFoundCount} groups match`;
      if(result.duplicateStatus!=='running')duplicateMessage=result.duplicateStatus==='not_scanned'?'Not scanned. Choose a match type and Scan.':result.duplicateStatus==='failed'?`Scan failed · ${result.errors||0} errors. Retry the scan.`:result.duplicateStatus==='cancelled'?'Scan cancelled.':`${count} · ${result.skipped||0} skipped · ${result.errors||0} errors`;
    }else duplicateMessage='';
    nativeMatchRevision=revision;filterPending=false;filterMessage='';setFilterError(false);renderTracks({preserveDuplicateRows:preserve});
    if(duplicateRefreshPending){duplicateRefreshPending=false;refreshDuplicatePublications();}
  } catch(error) {
    if(version!==filterRenderVersion)return;
    if(/supersed|cancelled|snapshot|library changed|library revision/i.test(error.message||'')&&queryRetry++<3){libraryQueryClient.invalidate();setFilterError(false);scheduleFilterRender();return;}
    filterMessage=`${error.message || error}. Showing the last valid results.`;setFilterError(true);filterPending=false;
    el('filter').setAttribute('aria-invalid','true');el('filter').parentElement.classList.add('has-error');el('filter').title=filterMessage;
    publishFilterState();updateDiffBar();
  }
}

// Typing re-renders the whole table, which on a large library is far slower
// than the typing (#184) — so the render waits for a pause. The controls follow
// the keystroke, so the box itself never feels laggy.
let filterTimer = null;
el("filter").addEventListener("input", (e) => {
  // Preserve the user's spelling; matching is literal and case-insensitive.
  filterRenderVersion++;
  setFilterText(e.target.value.trim());
  syncFilterControls();
  scheduleFilterRender();
});
function scheduleFilterRender() {
  filterRenderVersion++;
  filterPending = !filterError;
  scheduledQueryRevision=contentRevision;
  clearTimeout(filterTimer);
  publishFilterState();
  updateDiffBar();
  filterTimer = setTimeout(renderFilteredTracks, 120);
}
document.addEventListener('tagrex:filter-conditions', event => {
  try {
    const conditions = normalizeConditions(event.detail.conditions);
    filterRoot=legacyRoot(conditions,event.detail.mode==='any'?'any':'all');
    filterConditions=filterRoot.children;filterConditionMode=filterRoot.mode;
    syncFilterControls();
    scheduleFilterRender();
  } catch (error) { toast(error.message, true); }
});
let duplicateActivity = null;
function invalidateDuplicateScan() {
  duplicateJobs.invalidate();
  duplicateActivity?.finish();
  duplicateActivity = null;
}
function stopDuplicateScan() {
  duplicateActivity?.stopping(msg('bpm.stopping'), '');
  return duplicateJobs.cancel();
}
const duplicateJobs=createDuplicateJobs(invoke,state=>{
  duplicateScanning=state.status==='running';
  duplicateStatus=state.status==='error'?'failed':state.status;
  duplicateFoundCount=state.groupsCount??state.groups?.length??0;
  const phase={stamping:'Checking files',hashing:'Checking file contents',decoding:'Reading decoded audio',fingerprinting:'Reading audio fingerprints',comparing:'Comparing recordings'}[state.phase]||'Scanning files';
  const progress=state.phase==='comparing'?`${phase} · ${duplicateFoundCount} groups found`:`${phase}: ${state.processed||0} of ${state.total||tracks.length} files · ${duplicateFoundCount} groups found`;
  duplicateMessage=commandMode!=='duplicates'?'':state.status==='error'?state.error:state.status==='failed'?(state.message||'Scan failed. Retry with a narrower library.'):state.status==='running'?progress:state.status==='cancelled'?'Scan cancelled.':`${duplicateFoundCount} groups · ${state.skipped||0} skipped · ${state.errors||0} errors`;
  if (state.status === 'running' && commandMode === 'duplicates') {
    duplicateActivity ??= beginBackgroundActivity({ title: msg('duplicates.scan'), stop: stopDuplicateScan });
    // Native scan phases have different denominators. Show the current phase
    // and count without presenting them as an overall percentage.
    duplicateActivity.progress(null, progress);
  } else if (duplicateActivity) {
    const outcome = state.status === 'cancelled' ? 'cancelled'
      : (state.status === 'error' || state.status === 'failed') ? 'error' : 'success';
    const title = outcome === 'success' ? msg('duplicates.complete')
      : outcome === 'cancelled' ? msg('duplicates.cancelled') : msg('duplicates.failed');
    duplicateActivity.finish(commandMode === 'duplicates' ? outcome : null, title, duplicateMessage);
    duplicateActivity = null;
  }
  if(state.groupsRevision!=null)duplicateLatestRevision=Math.max(duplicateLatestRevision,state.groupsRevision);
  if(state.status==='cancelled'||state.status==='error'||state.status==='failed'){
    // A query may already hold a pre-cancel publication. Reject its response
    // before clearing the visible scope, then fetch the authoritative revision.
    duplicateGroups=[];duplicateRefreshPending=false;clearDuplicateSelection();
    if(duplicateResultsActive()){
      clearTimeout(filterTimer);filterRenderVersion++;libraryQueryClient.cancel();filterPending=false;
      nativeMatchPaths=new Set();renderTracks();
    }
  }
  publishFilterState();
  if(state.groupsRevision!=null)refreshDuplicatePublications();
},()=>{if(commandMode==='duplicates')scheduleFilterRender();});
function runDuplicateScan(){
  if(commandMode!=='duplicates')setMode('deduplicator');
  clearTimeout(filterTimer);filterRenderVersion++;libraryQueryClient.cancel();filterPending=false;
  duplicateScanStarted=true;
  resetDuplicatePublications();duplicateFoundCount=0;duplicateGroups=[];nativeMatchPaths=new Set();
  nativeMatchRevision=scheduledQueryRevision=contentRevision;renderTracks();
  return duplicateJobs.start(duplicateKind);
}
let previousQueryMode='tagger';
document.addEventListener('tagrex:query-mode',event=>{
  if(event.detail.mode==='duplicates'){if(currentMode!=='deduplicator')previousQueryMode=currentMode;setMode('deduplicator');}
  else setMode(previousQueryMode==='deduplicator'?'tagger':previousQueryMode);
});
document.addEventListener('tagrex:query-duplicate-kind',event=>{
  duplicateJobs.cancel();invalidateDuplicateScan();duplicateKind=event.detail.kind;duplicateScanStarted=false;duplicateScanning=false;duplicateGroups=[];duplicateStatus='not_scanned';duplicateFoundCount=0;duplicateMessage='';resetDuplicatePublications();syncFilterControls();scheduleFilterRender();
});
document.addEventListener('tagrex:query-scan',runDuplicateScan);
document.addEventListener('tagrex:query-scan-cancel', stopDuplicateScan);
document.addEventListener('tagrex:filter-root',event=>{
  try{filterRoot=normalizeRoot(event.detail.root);filterConditions=filterRoot.children;filterConditionMode=filterRoot.mode;syncFilterControls();scheduleFilterRender();}
  catch(error){event.preventDefault();toast(error.message,true);}
});
let suggestionTimer=null, suggestionVersion=0;
document.addEventListener('tagrex:query-suggest-dismiss',()=>{clearTimeout(suggestionTimer);suggestionVersion++;libraryQueryClient.cancelSuggestions();});
document.addEventListener('tagrex:query-suggest',event=>{
  const version=++suggestionVersion,text=event.detail.text;
  clearTimeout(suggestionTimer);
  if(!text.trim()||(commandMode==='duplicates'&&!['running','complete'].includes(duplicateStatus))||(commandMode==='duplicates'&&!duplicateFoundCount)){libraryQueryClient.cancelSuggestions();document.dispatchEvent(new CustomEvent('tagrex:query-suggestions',{detail:{text,suggestions:[],proposals:[],ready:false}}));return;}
  suggestionTimer=setTimeout(async()=>{
    const parsed=interpretQueryText(text),prefix=scopedText(text);
    const proposals=parsed.complete&&parsed.nodes.length?[{nodes:parsed.nodes}]:[];
    const respond=suggestions=>{if(version===suggestionVersion)document.dispatchEvent(new CustomEvent('tagrex:query-suggestions',{detail:{text,suggestions,proposals,highlight:parsed.remaining||prefix?.text||text,ready:true}}));};
    if(parsed.complete&&!parsed.remaining)return respond([]);
    const field=parsed.field||prefix?.field;
    const needle=parsed.nodes.length?parsed.remaining:prefix?prefix.text.trim():text;
    try{
      const model={...filterModel(field),text:''};
      const extra=parsed.field?parsed.nodes.slice(0,-1):parsed.nodes;
      if(extra.length)model.root=normalizeRoot(model.root.mode==='all'?{...model.root,children:[...model.root.children,...extra]}:{kind:'group',id:`suggest-root-${version}`,mode:'all',children:[model.root,...extra]});
      const suggestions=await libraryQueryClient.suggest(model,needle,field);
      if(suggestions)respond(suggestions.map(s=>extra.length?{...s,nodes:[...extra,{...newCondition(s.field,s.value),operator:s.operator||defaultOperator(s.field)}]}:s));
    }catch{respond([]);}
  },160);
});
document.addEventListener('tagrex:request-filter-state', publishFilterState);
// The field's clear button (#547): the same as deleting the text by hand.
el("filter-clear").addEventListener("click", () => {
  const box = el("filter");
  box.value = "";
  box.dispatchEvent(new Event("input", { bubbles: true }));
  box.focus();
});
// Presets popover (#44): mirrors the columns picker — toggle + outside-click close.
el("presets-btn").addEventListener("click", (e) => {
  e.stopPropagation();
  const menu = el("presets-menu");
  if (menu.hidden) renderPresetsMenu();
  menu.hidden = !menu.hidden;
});
document.addEventListener("click", (e) => {
  const menu = el("presets-menu");
  if (!menu.hidden && !menu.contains(e.target) && e.target !== el("presets-btn")) {
    menu.hidden = true;
  }
});
// The action-groups popovers (#57) wire their own toggle and outside-click
// close, in chain.js, since there is one per rule chain (#144).
// Track edits on any editable cell (event delegation).
tracksBody.addEventListener("input", (e) => {
  if (!e.target.classList.contains("editable")) return;
  onCellEdit(e.target);
  refreshCellSuggest(e.target); // offer what the library already says (#63)
});
// Enter commits a cell instead of inserting a newline, and leaves edit mode.
tracksBody.addEventListener("keydown", (e) => {
  if (!e.target.classList.contains("editable")) return;
  // Enter can confirm an IME composition; let it finish before cell shortcuts.
  if (e.isComposing || e.keyCode === 229) return;
  // The suggestion list gets first refusal on the arrows, and on Enter/Escape
  // only once an arrow has stepped into it (#63) — so a cell nobody is reading
  // suggestions for still commits on Enter exactly as it always has.
  if (cellSuggestKey(e)) return;
  if (e.key === "Enter" || e.key === "Escape") {
    e.preventDefault();
    // Escape is the cell's own: it must not also stop a search or close
    // something else.
    e.stopPropagation(); // the grid must not re-enter edit on the same Enter
    // Ending the edit ends the suggestions with it, said here rather than left
    // to the blur listener below: turning contentEditable off moves focus to
    // the body WITHOUT firing a blur on the cell, so the list would stay on
    // screen over a cell nobody is editing any more (#63) — and for the same
    // reason the write is asked for here too (#507).
    endCellSuggest();
    if (e.key === "Escape") cancelCellEdit(e.target);
    e.target.contentEditable = "false";
    e.target.blur();
    commitCellEdit(e.target);
    spreadsheet?.focusActive();
  }
});

// Range edits use one validated preview and the existing Apply/Undo gate. They
// never feed the immediate single-cell write queue or replace an existing draft.
function spreadsheetBatchProblem() {
  if(commandMode==='duplicates')return 'Return to Search before editing files.';
  if (applying || undoing) return "Wait for the current write or undo to finish.";
  if (diffByPath || previewPlan) return "Apply or discard the current preview first.";
  if (edits.size || pendingImportCover) return "Review or discard the existing tag edits before pasting.";
  return "";
}

async function stageSpreadsheetEdits(list, kind) {
  const problem = spreadsheetBatchProblem();
  if (problem) throw new Error(problem);
  const origin = tracks;
  const plan = await invoke("preview_tag_edits", { edits: list, cover: null });
  if (tracks !== origin || spreadsheetBatchProblem()) {
    throw new Error("The library changed while preparing the preview. Paste again.");
  }
  if (plan.locked_skipped?.length) {
    throw new Error("A field was locked during the preview. Nothing was staged. Unlock the field and paste again.");
  }
  // Other producers intentionally display backend-invalid values and apply the
  // valid subset. A spreadsheet batch promises all targets are valid, so keep
  // that stricter rule at this boundary without changing shared plan semantics.
  const invalidCells = [];
  for (const change of plan.changes) {
    const file = fileName(change.path);
    for (const field of change.tag_changes || []) {
      if (field.invalid && invalidCells.length < 3) invalidCells.push(`${diffLabel(field.field)} (${file})`);
    }
    const cover = change.cover_change;
    if (cover && (cover.invalid || [...(cover.old || []), ...(cover.new || [])].some((image) => image.invalid)) && invalidCells.length < 3) {
      invalidCells.push(`Artwork (${file})`);
    }
  }
  if (invalidCells.length) {
    throw new Error(`Nothing was staged. Rejected ${invalidCells.join(" · ")}. Correct these values and paste again.`);
  }
  if (!plan.changes.length) {
    toast("The selected values already match. Nothing to change.");
    return;
  }
  setPreviewPlan(plan);
  setPreviewSource("spreadsheet");
  spreadsheetBatchKind = kind;
  renderPreview(previewPlan);
}

spreadsheet = initSpreadsheet({
  body: tracksBody,
  wrap: tableWrap,
  table: el("tracks"),
  getRows: () => navPaths,
  getRowIndex: (path) => navPathIndex.get(path),
  getColumns: () => tableColumnKeys(),
  getRow: renderedRow,
  rowCount: () => viewItems.length,
  rowNumber: (path) => gridRowNumber(itemIndexByPath.get(path)),
  rowLabel: fileName,
  label: columnLabel,
  value: (path, field) => {
    const track = trackAt(path);
    if (!track) return "";
    if (field === 'file' || customColumnOf(field) || VIRTUAL_COLUMNS.some(([key]) => key === field)) return fieldValue(track, field);
    const changed = diffByPath?.get(path)?.tag_changes.find(change => change.field === field);
    return changed ? changed.new ?? '' : currentFieldValue(path, field);
  },
  readOnly: (field) => {
    if(commandMode==='duplicates')return 'Duplicate review is read-only.';
    if (isFieldLocked(field)) return `${columnLabel(field)} is locked. Unlock it before batch editing.`;
    if (field === "file" || customColumnOf(field) || VIRTUAL_COLUMNS.some(([key]) => key === field)) {
      return `${columnLabel(field)} is read-only.`;
    }
    return "";
  },
  validate: (field, value, path) => validateFieldValue(field, value, [path]),
  isReviewing: () => applying || !editablePreview(),
  rowActivation: event => duplicateResultsActive()&&(event.key==='Enter'||event.key===' '),
  batchProblem: spreadsheetBatchProblem,
  stage: stageSpreadsheetEdits,
  beginEdit: beginCellEdit,
  finishEdit: (cell) => {
    endCellSuggest();
    cell.contentEditable = "false";
    cell.blur();
    commitCellEdit(cell);
  },
  reveal: scrollPathIntoView,
  setActiveRow: (path, {extend=false,followSelection=false}={}) => {
    setActiveRowByPath(path, false);
    if (followSelection && !extend && commandMode!=='duplicates' && !usingReviewSelection() && (selection.size!==1 || !selection.has(path))) {
      selection.clear();selection.add(path);anchorSelection(path);
      syncSelectionUI({refreshGrid:false});
    }
  },
  focusRow: (path) => setActiveRowByPath(path, true),
  selectionSummary: () => {
    const hidden = [...selection].filter((path) => !navPathIndex.has(path)).length;
    return `${tracks.length} files · ${selection.size} selected${hidden ? ` · ${hidden} hidden` : ""}`;
  },
  hideMenus: () => { hideFileCtx(); hideEditCtx(); },
  error: (message) => toast(message, true),
});

loadSavedToken();
// What the session locks (#48). Nothing is open yet at start-up, so this
// normally answers "nothing"; after a window reload with a library still open it
// is what stops the padlocks from disagreeing with the backend enforcing them.
loadFieldLocks();

// Apply the saved column choice (#43) and build the header before any library
// is opened.
loadCustomColumns(); // before loadColumns: it validates keys against the pool
loadColumns();
loadColumnWidths();
renderTableHead();
applyValueFont(valueFont());
applyCheckboxCol(checkboxColEnabled());
// --panel-inset (#374) is the right padding every mode-panel and TAGGER/
// ONLINE's own head/scroll use — 14px, matching the topbar's own right
// inset above them, with none of these elements reserving a
// scrollbar-gutter of their own any more (an earlier version of this fix
// tried scrollbar-gutter: stable, but WKWebView reserves the *platform's*
// default scrollbar metrics there rather than respecting our own
// ::-webkit-scrollbar styling — wider than the 14px budget, with no way for
// CSS to ask it what that width actually is and compensate). This is a
// defensive safety net, not the fix itself: after the first layout pass,
// nudge --panel-inset by however far TAGGER/ONLINE's own search-format
// select (default-visible at startup, no mode switch needed) still sits
// from the topbar's rightmost icon (the panel toggle since #421), in case
// some other engine has its own equivalent quirk (a <select> chrome inset,
// for instance) that plain 14px doesn't already account for.
(function calibratePanelInset() {
  requestAnimationFrame(() => {
    const ref = el("panel-toggle");
    const target = el("search-format");
    if (!ref || !target) return;
    // A hidden window can hold this frame back until after a mode switch; the
    // select then measures as all zeros and the "gap" is the whole window,
    // which set the inset to 0 (#580). Nothing to calibrate against: keep 14px.
    if (!target.getClientRects().length) return;
    const delta = ref.getBoundingClientRect().right - target.getBoundingClientRect().right;
    if (Math.abs(delta) < 0.5) return;
    const current = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--panel-inset")) || 14;
    // target sitting delta px left of ref means too much padding — less
    // padding pushes its content right to close the gap, hence current -
    // delta (padding and the gap move in the same direction, not opposite).
    document.documentElement.style.setProperty("--panel-inset", `${Math.max(0, current - delta)}px`);
  });
})();
applyTableFont(tableFontPx());
applyTracklistFont(tracklistFontPx());
applyBadgeFont(badgeFont());
// Reflect saved defaults onto the grouping selector.
// Grouping options are built from the modeled fields (#43), then the saved
// choice is applied.
populateGroupMenu();
// Load saved action groups (#57) from settings.json into the Groups popover.
// The per-job chains become presets once the saved ones are in (#392).
initActionGroups().then(() => {
  migrateChains();
  // RENAMER's carry toggles show the saved settings (#405).
  syncCarryToggles();
});
initBuiltinGroups();
// Initialize the literal, case-insensitive query controls.
syncFilterControls();
// Wire the placeholder reference (#148) and warm its catalogue, so the column
// headers can name their placeholder without an await per header. The head is
// already drawn by then, so it is redrawn once the names are available.
initPlaceholderReference().then(renderTableHead);
initMaskPresets();
// Native settings own reusable workflows shared with agents; localStorage is
// retained as a recoverable migration source and a browser-fixture fallback.
if (TAURI) {
  let agentsAvailable = false;
  const publishViewPreferences = () => !agentsAvailable ? Promise.resolve() : invoke('mcp_set_view_preferences', { preferences: {
    theme: themeMode(), palette: paletteMode(), value_font: valueFont(), table_font_px: tableFontPx(),
  } }).catch(error => toast(String(error), true));
  document.addEventListener('tagrex:view-preferences-changed', publishViewPreferences);
  const localPresets = loadPresets();
  invoke('load_settings', {}).then(settings => {
    const migrate = !settings.query_presets?.length && localPresets.length;
    acceptSharedSettings(settings);
    if (migrate) savePresets(localPresets);
  }).catch(error => toast(String(error), true));
  let agentsEnabled = false;
  let manualPlanSync = Promise.resolve();
  let manualGeneration = 0;
  const manualPublications = new Map();
  function mirrorManualPlan(plan) {
    if (!agentsEnabled || sharedStageSync) return;
    const publication = { generation: manualGeneration, inputRevision: liveInputRevision(), desired: desiredPlanFingerprint(plan) };
    manualPlanSync = manualPlanSync.then(async () => {
      const revision = await invoke('mcp_set_manual_plan', { plan });
      manualPublications.set(revision, publication);
      while (manualPublications.size > 4) manualPublications.delete(manualPublications.keys().next().value);
    }).catch(error => toast(String(error), true));
  }
  const touchedFields = () => liveInputs().edits.map(({ path, field }) => ({ path, field }));
  document.addEventListener('tagrex:manual-plan-changed', event => {
    manualGeneration++;
    if (clearingSharedStage) return;
    if (sharedStageSync) {
      sharedStageSync.touched(touchedFields());
      void sharedStageSync.changed(event.detail, event.stageTouched || []);
    }
    else mirrorManualPlan(event.detail);
  });
  document.addEventListener('tagrex:live-input', () => {
    if (!sharedStageSync || clearingSharedStage || nativeLiveFields) return;
    const touched = touchedFields();
    sharedStageSync.touched(touched);
    // Claim a directly typed field before an older native/agent response can
    // refresh that cell. The validated automatic preview follows normally.
    if (touched.length) void sharedStageSync.changed(sharedStageSync.humanPlan(), touched);
  });
  function connectSharedStage(capabilities) {
    if (sharedStageSync) return;
    const live = capabilities?.version >= 1 && capabilities.delta && capabilities.intents ? createLiveStageTransport(invoke, capabilities) : null;
    setNativeLiveFields(!!live, capabilities);
    setNativeStageSession(capabilities?.session);
    sharedStageSync = createSharedStageSync({
      intents: live?.publish,
      read: live?.read || (() => invoke('mcp_shared_stage', {})),
      publish: async (plan, expectedRevision, touched) => {
        const value = await invoke('mcp_publish_shared_stage', { plan, expectedRevision, touched });
        return live ? live.reconcile(value) : value;
      },
      clear: async expectedRevision => {
        const value = await invoke('mcp_clear_shared_stage', { expectedRevision });
        return live ? live.reconcile(value) : value;
      },
      context: () => ({ inputRevision: liveInputRevision(), library: tracks, busy: applying || undoing }),
      adopt: snapshot => {
        if (!tracks.length) return false;
        setNativeStageSession(snapshot.session,snapshot.nativeDelta ? null : (snapshot.derived_fields || []),snapshot.derivedChanges || []);
        const automatic = true; // Shared fields stay editable inside mixed artwork/rename plans.
        const plan = snapshot.nativeDelta ? snapshot.plan : snapshot.plan ? hydratePlan(snapshot.plan) : null;
        withoutPreviewPublication(() => {
          setPreviewPlan(plan);
          adoptSharedPreview(plan, snapshot.human_plan, automatic);
          if (!previewSource && plan) setPreviewSource(plan.changes.some(change => change.rename_to) ? 'rename' : 'edits');
        });
        // Rendering can synchronously refresh automatic inputs. Those updates
        // must still pass through the shared-stage decorator rather than
        // replacing the canonical stage with a local null/partial preview.
        renderPreview(plan, { preserveView: true, delta: snapshot });
        refreshSharedStagedFields(snapshot.nativeDelta ? snapshot.changedKeys : null);
        // A staging acknowledgement changes desired fields, not saved custom
        // columns or the selection. Their existing open/write/view handlers
        // own refreshes; scanning the library here defeats scalar deltas.
        if (!snapshot.nativeDelta) syncSelectionUI({refreshGrid:false});
        if (!snapshot.nativeDelta) scheduleAutomaticTransformPreview();
      },
      error: failure => toast(String(failure), true),
    });
    setSharedStagingEnabled(true);
    setPreviewDecorator((plan, touched) => {
      const shared = sharedStageSync.decorate(plan, touched);
      adoptSharedPreview(shared, plan, true);
      return shared;
    });
    void sharedStageSync.start(previewPlan);
  }
  async function refreshCurrentManualPlan(paths) {
    if (hasLiveInputs() || isAutomaticTransformPreview()) { liveInputChanged(); return; }
    if (!previewPlan) { renderTracks(); return; }
    const current = previewPlan, generation = manualGeneration, source = tracks;
    const included = new Set(applySelection);
    const rebased = await invoke('mcp_rebase_manual_plan', { plan: current, paths });
    if (generation !== manualGeneration || source !== tracks || current !== previewPlan) return;
    setPreviewPlan(rebased); renderPreview(rebased); setApplySelection(included);
  }
  async function refreshAgentState() {
    const status = await invoke('mcp_status', {});
    const wasAvailable = agentsAvailable;
    agentsAvailable = status.available !== false;
    if (agentsAvailable && !wasAvailable) void publishViewPreferences();
    const wasEnabled = agentsEnabled;
    agentsEnabled = status.listening;
    // During source iteration the old native binary remains usable. Only a
    // backend advertising this contract receives shared-stage commands.
    if (agentsAvailable && status.shared_staging === true) connectSharedStage(status.live_fields);
    if (agentsEnabled && !wasEnabled) mirrorManualPlan(previewPlan);
  }
  document.addEventListener('tagrex:show-shared-stage', () => {
    el('settings').hidden = true;
    setFileView(true);
    el('staged-tab').focus({ preventScroll: true });
  });
  document.addEventListener('tagrex:adopt-agent-draft', async event => {
    if (applying || undoing || previewPlan?.changes.length || edits.size) {
      toast('Finish or discard your current edits before adopting an agent draft.', true);
      return;
    }
    try {
      const plan = await invoke('mcp_adopt_draft', event.detail);
      if (previewPlan?.changes.length || edits.size) throw new Error('Your edits changed while the agent draft was loading. Try again when they are complete.');
      setPreviewSource('agent'); setPreviewPlan(plan); renderPreview(plan);
      el('settings').hidden = true;
    } catch (error) { toast(String(error), true); }
  });
  void refreshAgentState().catch(error => toast(String(error), true));
  const events = window.__TAURI__?.event;
  events?.listen('tagrex://agent-settings-changed', async () => {
    try { acceptSharedSettings(await invoke('load_settings', {})); }
    catch (error) { toast(String(error), true); }
  });
  events?.listen('tagrex://agent-event', async event => {
    const { kind, data } = event.payload || {};
    try {
      if (kind === 'service_changed') await refreshAgentState();
      if (kind === 'shared_stage_changed') await sharedStageSync?.refresh();
      if (kind === 'shared_stage_refresh_failed') {
        toast('Files were saved, but the remaining staged edits could not refresh. Review the staging area before applying again.', true);
        await sharedStageSync?.refresh();
      }
      if (kind === 'write_completed') {
        const updated = new Map((data.tracks || []).map(track => [track.path, track]));
        const included = new Set(applySelection);
        setTracks(tracks.map(track => updated.get(track.path) || track));
        invalidateCustomColumns([...updated.keys()]);
        if (sharedStageSync) {
          if (hasLiveInputs()) liveInputChanged();
          await sharedStageSync.refresh();
          refreshCustomColumnCells(); refreshFieldEditor(); refreshSharedStagedFields(); syncSelectionUI();
          await refreshHistory();
          return;
        }
        // A newer local edit can precede its native preview/publication. The
        // server revision, raw-input revision and desired values must all match
        // before an older operation may replace the person's current plan.
        const adopt = canAdoptAgentRebase(manualPublications.get(data.manual_revision), {
          plan: previewPlan, generation: manualGeneration, inputRevision: liveInputRevision(), event: data,
        });
        manualPublications.delete(data.manual_revision);
        if (adopt) {
          const plan = data.plan ? hydratePlan(data.plan) : null;
          setPreviewPlan(plan);
          if (plan) { renderPreview(plan); setApplySelection(included); }
          else renderTracks();
        } else await refreshCurrentManualPlan([...updated.keys()]);
        refreshCustomColumnCells(); refreshFieldEditor(); syncSelectionUI();
        await refreshHistory();
      } else if (kind === 'session_changed' && data?.root) {
        if (!edits.size && !previewPlan) await afterOpen(data.root, ++openVersion);
      } else if (kind === 'session_changed' && !diffByPath) {
        const fresh = await invoke('list_tracks', {});
        const order = handOrder();
        setTracks(fresh); sortKeepingHandOrder(order); renderTracks();
        refreshCustomColumnCells(); refreshFieldEditor(); syncSelectionUI(); await refreshHistory();
      } else if (kind === 'ui_present') {
        if (data?.module) setMode(data.module);
        if (data?.path) hooks.revealPath(data.path);
        if (data?.draft) {
          document.dispatchEvent(new CustomEvent('tagrex:review-agent-draft', { detail: data }));
        }
      } else if (kind === 'ui_preferences') {
        if (data?.theme) applyTheme(data.theme);
        if (data?.palette) applyThemePreset(data.palette);
        if (data?.value_font) applyValueFont(data.value_font);
        if (data?.table_font_px) applyTableFont(data.table_font_px);
        await publishViewPreferences();
      }
    } catch (error) { toast(String(error), true); }
  });
}
// Put the interface into the chosen language before anything else paints (#50).
// Early, because the panels below read their own labels back out of the DOM.
applyStaticText();
syncCssLabels();
// Keep useful title text accessible without hover bubbles covering controls.
initTooltips();
initWorkbenchMotion();
// Keyboard shortcuts (#432): one registry, rebindable in Settings. Select-all
// has no button to press, so it hands its own handler in.
initShortcuts({
  selectAll: selectAllRows,
  // The same as the file menu's Move to Trash (#600), with its confirmation, on
  // the selection; inert while a plan is staged, like the menu.
  trash: () => {
    if (diffByPath) return;
    const paths = selectedPaths();
    if (paths.length) trashFiles(paths);
  },
});
initUpdateCheck();
// Open whatever the OS handed the app on the way in (#51) — a folder from
// "Open With", from the Dock icon, or from a second launch. After the hooks
// above, since it opens through the same one a drop does.
initLaunchOpen().then(async handedOver => {
  if (handedOver || openVersion !== 0) return;
  const saved = readSession(sessionStorage());
  if (!saved) return;
  if (!beginResume(sessionStorage(), saved.root)) {
    rootInput.value = saved.root;
    rootInput.hidden = false;
    el('root-box').hidden = true;
    updateLibAction();
    toast(msg('session.interrupted', { folder: folderName(saved.root) }), true);
    return;
  }
  rootInput.value = saved.root;
  await openLibrary({ restore: saved });
});
// Put the transform chain where the saved preference says it belongs (#233):
// nothing calls setMode at startup, so without this a pinned chain would come
// back unpinned after a restart.
syncTransformPlacement();

// ---- dev surface (#143) ----
// app.js is a module now, so nothing in it is global by accident. The
// browser-served verification path drives the UI by calling into it, so the few
// entry points it needs are exposed deliberately, in one place, instead of the
// whole file leaking into `window`. Harmless in the native app: it is a handle
// on functions the UI itself already calls.
initStaging({ isBusy: () => applying || undoing, onClear:discardPreview });
syncFileView();

window.tagrex = {
  invoke,
  openLibrary,
  setMode,
  setSubtab,
  selectedPaths,
  renderTracks,
  get tracks() {
    return tracks;
  },
  get selection() {
    return selection;
  },
  get previewPlan() {
    return previewPlan;
  },
  // Keep diagnostics attached to the active controller after Vite replaces a
  // module; importing the bare state URL can observe an older module instance.
  get querySession() {
    return { root: sessionRoot, edits: edits.size, preview: diffByPath?.size || 0 };
  },
};

// Outside editable fields Ctrl/Cmd+Z first reverts an un-applied import choice.
document.addEventListener('keydown', event => {
  if (applying || undoing || event.target.closest('[data-audio-studio]')) return;
  if (!(event.ctrlKey || event.metaKey) || event.shiftKey || event.key.toLowerCase() !== 'z') return;
  if (event.target.closest('input,textarea,[contenteditable="true"],[data-search-editor]')) return;
  if (undoLiveImport()) { event.preventDefault(); event.stopImmediatePropagation(); refreshFieldEditor({force:true}); }
}, true);
