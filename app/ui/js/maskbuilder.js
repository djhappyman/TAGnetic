// The mask builder (#540): a window in which the RENAMER mask and what it makes
// of every selected file are on screen together.
//
// The box under the mask shows one file (#382) and the placeholder reference
// closes once something is inserted (#148), which leaves a longer expression to
// be written blind. Here the reference stays open beside the expression and the
// list below it is the whole selection, current name and new name, recomputed
// as you type. It is computed by the same call Preview makes, through the same
// rules, so the list and the plan OK stages cannot disagree. Nothing is written
// from the window: OK puts the mask in the box and stages the preview.
//
// The same window serves FROM NAME (#548), the grammar run the other way: the
// file name in, tags out. There the list is a table, the file name and then a
// column for every tag the mask names, each value marked as the preview would
// treat it. A mode says which box the mask comes from and goes back to, how the
// list is computed and what OK stages.
import { loadTextFunctions } from './function-pattern.js';
import { connectBuilderSizing } from './mask-builder-sizing.js';
import { initTableScrollbars } from './overlay-scrollbars.js';
import { el, escapeHtml } from "./dom.js";
import { t } from "./i18n.js";
import { invoke } from "./invoke.js";
import { chainHasRules, onChainChanged, runChainOverPlan } from "./chains.js";
import { columnLabel } from "./columns.js";
import { previewTagsFromName } from "./fromname.js";
import { previewTagsFromText, visibleSubject } from "./fromtext.js";
import { catalogue, insertAt, renderReference } from "./placeholders.js";
import { maskPart, planForMask, preview } from "./renamer.js";
import { captureSelection, isSelectionCurrent, tracks } from "./state.js";

// Rows the list shows. A list is for reading, and a few hundred names is past
// what anyone reads; the count says how many more there are.
const LIMIT = 300;
const DEBOUNCE_MS = 200;
// The size the window was last left at (#541), as "width,height" in pixels.
const SIZE_STORAGE_KEY = "tagrex.maskBuilderSize";

const modal = el("mask-builder");
const dialog = modal.querySelector(".mask-builder");
const expr = el("mb-expr");
const exprField = expr.closest(".mb-field");

let entries = [];
let referenceFunctions = [];
let timer = null;
// Which refresh is the latest, so a slow answer to an older expression never
// paints over the answer to a newer one.
let generation = 0;
let activeRefresh = false;
let queuedRefresh = null;

function baseName(path) {
  return String(path).split(/[\\/]/).pop();
}

// One row per file: what it is called now and what the mask calls it, marked
// when the name stays, when a required placeholder is empty for it, and when
// another file would end up with the same name.
function rowsFor(plan, paths) {
  const changes = new Map(plan.changes.map((change) => [change.path, change]));
  const missing = new Map();
  for (const entry of plan.unrendered || []) {
    for (const path of entry.paths || []) missing.set(path, entry.field);
  }
  // Where each file ends up, the ones that stay included, compared without case:
  // two names that differ only in case are one file on a case-insensitive disk.
  const landing = new Map();
  for (const path of paths) {
    const key = (changes.get(path)?.rename_to || path).toLowerCase();
    landing.set(key, (landing.get(key) || 0) + 1);
  }
  return paths.map((path) => {
    const change = changes.get(path);
    if (missing.has(path)) return { path, kind: "unrendered", field: missing.get(path) };
    if (!change?.rename_to || change.rename_to === path) return { path, kind: "unchanged" };
    const target = maskPart(change.rename_to, change.rename_root);
    const clash = landing.get(change.rename_to.toLowerCase()) > 1;
    return { path, kind: clash ? "clash" : "changed", target };
  });
}

function rowsHtml(rows) {
  const body = rows.map((row, index) => {
    const token = `%${row.field}%`;
    const after = row.kind === 'unrendered'
      ? escapeHtml(t('maskBuilder.needs', {field:token}))
      : escapeHtml(row.kind === 'unchanged' ? baseName(row.path) : row.target);
    const label = row.kind === 'changed' ? t('maskBuilder.statusChanged')
      : row.kind === 'clash' ? t('maskBuilder.statusClash')
      : row.kind === 'unrendered' ? t('maskBuilder.statusMissing') : t('maskBuilder.unchanged');
    return `<tr class="mb-row mb-${row.kind}">
      <td class="mb-index">${index + 1}</td>
      <td class="mb-old" data-tooltip="${escapeHtml(row.path)}" title="${escapeHtml(row.path)}">${escapeHtml(baseName(row.path))}</td>
      <td class="mb-new" data-tooltip="${row.kind === 'unchanged' ? escapeHtml(row.path) : row.kind === 'unrendered' ? '' : escapeHtml(row.target)}" title="${row.kind === 'unchanged' ? escapeHtml(row.path) : row.kind === 'unrendered' ? '' : escapeHtml(row.target)}">${after}</td>
      <td class="mb-result-status" title="${row.kind === 'clash' ? escapeHtml(t('maskBuilder.clash')) : escapeHtml(label)}">${escapeHtml(label)}</td>
    </tr>`;
  }).join('');
  return `<table class="mb-table mb-rename-table" aria-label="${escapeHtml(t('maskBuilder.renamePreview'))}">
    <thead><tr><th scope="col" class="mb-index">#</th><th scope="col">${escapeHtml(t('maskBuilder.currentName'))}</th>
    <th scope="col">${escapeHtml(t('maskBuilder.proposedName'))}</th><th scope="col">${escapeHtml(t('maskBuilder.status'))}</th></tr></thead>
    <tbody>${body}</tbody></table>`;
}

// Render mode (RENAMER): the plan Preview would stage, one row per file.
async function renderList(mask, shown) {
  const rows = rowsFor(await planForMask(mask, shown), shown);
  return { html: rowsHtml(rows) };
}

// Extract mode (FROM NAME #548, FROM TEXT #550): what the mask reads out of
// each subject, a file name or a line of the list, through the FROM NAME
// presets as Preview tags runs them (#237), against the tags the file carries
// now. A value is marked when it is the same as the tag already there, when a
// rule changed what was read (the reading in its tooltip), and when the field
// cannot hold it; a subject the mask does not fit is one row.
//
// `rows` are { subject (markup), title, path (the file it goes to, or null),
// matched, readings (Map), invalid }. `extraColumn` adds the File column FROM
// TEXT needs, since there the subject is the line, not the file.
async function readingsTable({ fields, rows, subjectHeader, extraColumn, missLabel }) {
  let tidied = rows.map((row) => row.readings);
  const toChain = rows.filter((row) => row.matched && row.path);
  if (chainHasRules("fromname") && toChain.length) {
    const plan = {
      description: "",
      changes: toChain.map((row) => ({
        path: row.path,
        rename_to: null,
        tag_changes: [...row.readings].map(([field, value]) => ({ field, old: null, new: value })),
      })),
    };
    const out = await runChainOverPlan(plan, "fromname");
    const byPath = new Map((out?.changes || []).map((change) => [change.path, change]));
    tidied = rows.map((row) => {
      const change = row.path && byPath.get(row.path);
      return change ? new Map(change.tag_changes.map((tc) => [tc.field, tc.new ?? ""])) : row.readings;
    });
  }
  const current = new Map(tracks.map((track) => [track.path, track.tags || {}]));
  const extraHead = extraColumn ? `<th>${escapeHtml(t("maskBuilder.fileHeader"))}</th>` : "";
  const head = `<tr><th class="mb-old">${escapeHtml(subjectHeader)}</th>${extraHead}${fields
    .map((field) => `<th>${escapeHtml(columnLabel(field))}</th>`)
    .join("")}</tr>`;
  const body = rows
    .map((row, index) => {
      const subject = `<td class="mb-old" title="${escapeHtml(row.title)}"><span>${row.subject}</span><span class="mb-arrow" aria-hidden="true">→</span></td>`;
      if (!row.matched) {
        const span = Math.max(1, fields.length + (extraColumn ? 1 : 0));
        return `<tr class="mb-nomatch">${subject}<td class="mb-miss" colspan="${span}">${escapeHtml(missLabel)}</td></tr>`;
      }
      const extra = extraColumn
        ? row.path
          ? `<td class="mb-target" title="${escapeHtml(row.path)}">${escapeHtml(baseName(row.path))}</td>`
          : `<td class="mb-target mb-nofile">${escapeHtml(t("maskBuilder.noFile"))}</td>`
        : "";
      const cells = fields.map((field) => {
        const raw = row.readings.get(field);
        if (raw === undefined) return `<td class="mb-none">—</td>`;
        const value = tidied[index].get(field) ?? raw;
        if (row.invalid.includes(field)) {
          return `<td class="mb-bad" title="${escapeHtml(t("maskBuilder.cantWrite"))}">${escapeHtml(value)}</td>`;
        }
        if (value !== raw) {
          return `<td class="mb-tidied" title="${escapeHtml(t("maskBuilder.readAs", { raw }))}">${escapeHtml(value)}</td>`;
        }
        const same = row.path && (current.get(row.path)?.[field] ?? "") === value;
        return `<td${same ? ' class="mb-same"' : ""}>${escapeHtml(value)}</td>`;
      });
      return `<tr>${subject}${extra}${cells.join("")}</tr>`;
    })
    .join("");
  return { html: `<table class="mb-table"><thead>${head}</thead><tbody>${body}</tbody></table>` };
}

// FROM NAME (#548): one row per selected file.
async function extractList(mask, shown) {
  const probes = await invoke("probe_tags_from_names", { mask, paths: shown });
  const rows = probes.files.map((probe, index) => ({
    subject: escapeHtml(baseName(shown[index])),
    title: shown[index],
    path: shown[index],
    matched: probe.matched,
    readings: new Map(probe.fields),
    invalid: probe.invalid || [],
  }));
  const { html } = await readingsTable({
    fields: probes.fields,
    rows,
    subjectHeader: t("maskBuilder.fileName"),
    extraColumn: false,
    missLabel: t("maskBuilder.noFit"),
  });
  return { html };
}

// FROM TEXT (#550): one row per line of the list, the nth track on the nth
// selected file as Preview tags pairs them; a heading or a "Side A" is a line
// the mask does not fit, skipped. Tracks left without a file and files left
// without a track are counted, since that is a list and a selection that do
// not match.
async function textList(mask, shown) {
  if (!el("from-text-lines").value.trim()) {
    return { html: `<p class="mb-empty muted">${escapeHtml(t("fromText.noLines"))}</p>` };
  }
  const probe = await invoke("probe_text_lines", { mask, text: el("from-text-lines").value, paths: shown });
  const rows = probe.lines.map((line) => ({
    subject: visibleSubject(line.line),
    title: line.line,
    path: line.path,
    matched: line.matched,
    readings: new Map(line.fields),
    invalid: line.invalid || [],
  }));
  const { html } = await readingsTable({
    fields: probe.fields,
    rows,
    subjectHeader: t("maskBuilder.lineHeader"),
    extraColumn: true,
    missLabel: t("maskBuilder.notTrack"),
  });
  return { html };
}

const MODES = {
  render: {
    context: "renamer",
    source: "mask",
    opener: "mask-builder-open",
    title: "maskBuilder.title",
    list: renderList,
    accept: () => preview(),
  },
  extract: {
    context: "fromname",
    source: "from-name-mask",
    opener: "from-name-builder-open",
    title: "maskBuilder.titleExtract",
    list: extractList,
    accept: () => previewTagsFromName(),
  },
  // The FROM TEXT presets are FROM NAME's own (#237): both read values out of a
  // string with a mask.
  text: {
    context: "fromname",
    source: "from-text-mask",
    opener: "from-text-builder-open",
    title: "maskBuilder.titleText",
    list: textList,
    accept: () => previewTagsFromText(),
  },
};
let mode = MODES.render;

function refresh() {
  if (modal.hidden) return;
  el("mb-list").setAttribute("aria-busy", "true");
  el("mb-ok").disabled = true;
  queuedRefresh = { mine: ++generation, subject: captureSelection(), mode, mask: expr.value };
  void drainRefresh();
}

// At most one native preview is active. Later typing replaces a single queued
// request rather than filling the native worker/lock queue with obsolete masks.
async function drainRefresh() {
  if (activeRefresh || !queuedRefresh) return;
  const request = queuedRefresh;
  queuedRefresh = null;
  activeRefresh = true;
  const { mine, subject, mode: requestedMode, mask } = request;
  const current = () => mine === generation && !modal.hidden && mode === requestedMode && isSelectionCurrent(subject);
  try {
    const paths = subject.paths;
    // FROM TEXT can inspect text even without files to pair it with.
    if (!paths.length && requestedMode !== MODES.text) {
      if (!current()) return;
      el("mb-error").hidden = true;
      exprField.classList.remove("has-error");
      el("mb-list").innerHTML = `<p class="mb-empty muted">${escapeHtml(t("maskBuilder.noSelection"))}</p>`;
      return;
    }
    const result = await requestedMode.list(mask, paths.slice(0, LIMIT));
    if (!current()) return;
    el("mb-error").hidden = true;
    exprField.classList.remove("has-error");
    el("mb-list").innerHTML = result.html;
    el("mb-ok").disabled = false;
  } catch (e) {
    if (!current()) return;
    // Keep the last parsed result while an expression is half-typed.
    el("mb-error").textContent = String(e);
    el("mb-error").hidden = false;
    exprField.classList.add("has-error");
  } finally {
    if (current()) el("mb-list").setAttribute("aria-busy", "false");
    activeRefresh = false;
    if (queuedRefresh && !modal.hidden) void drainRefresh();
  }
}

function schedule() {
  el("mb-ok").disabled = true;
  generation++; // Reject the previous answer immediately, during the debounce.
  queuedRefresh = null;
  clearTimeout(timer);
  timer = setTimeout(refresh, DEBOUNCE_MS);
}

// The window opens at the size it was left at, if that still fits; the
// stylesheet's maximum keeps a size from a larger screen inside this one.
function restoreSize() {
  let stored = null;
  try {
    stored = localStorage.getItem(SIZE_STORAGE_KEY);
  } catch (e) {
    /* storage unavailable: the default size */
  }
  const [width, height] = String(stored || "").split(",").map(Number);
  dialog.style.width = width > 0 ? `${width}px` : "";
  dialog.style.height = height > 0 ? `${height}px` : "";
}

// Kept only when the corner was dragged: a default size written back on every
// close would stop the window following the app window's own size.
let openedSize = null;

function rememberSize() {
  if (modal.hidden || !openedSize) return;
  const { width, height } = dialog.getBoundingClientRect();
  if (Math.abs(width - openedSize.width) < 2 && Math.abs(height - openedSize.height) < 2) return;
  try {
    localStorage.setItem(SIZE_STORAGE_KEY, `${Math.round(width)},${Math.round(height)}`);
  } catch (e) {
    /* storage unavailable: the size just isn't kept */
  }
}

// One line, then as many as the mask wraps to, up to MAX_LINES (#545). Also
// tells the field whether it is empty, which hides Clear (#546).
const MAX_LINES = 5;
function fitField() {
  if (expr.hasAttribute("data-pattern-input")) { exprField.classList.toggle("is-empty", !expr.value); return; }
  exprField.classList.toggle("is-empty", !expr.value);
  expr.style.height = "auto";
  const line = parseFloat(getComputedStyle(expr).lineHeight) || 20;
  const chrome = expr.offsetHeight - expr.clientHeight;
  const padding = parseFloat(getComputedStyle(expr).paddingTop) + parseFloat(getComputedStyle(expr).paddingBottom);
  const most = line * MAX_LINES + padding + chrome;
  const wanted = expr.scrollHeight + chrome;
  expr.style.height = `${Math.min(wanted, most)}px`;
  expr.style.overflowY = wanted > most ? "auto" : "hidden";
}

// The reference offers what this direction can use: reading a name back cannot
// run a function or fill a computed value (#548).
function referenceEntries() {
  return mode === MODES.render ? entries : entries.filter((entry) => entry.extract);
}

// The grammar the reference shows for this mode (#557).
function referenceDirection() {
  return mode === MODES.render ? "render" : mode === MODES.text ? "text" : "extract";
}

const sizing = connectBuilderSizing({dialog, body:dialog.querySelector('.mb-body'),
  sidebar:dialog.querySelector('.mb-ref'), divider:el('mb-pane-divider'),
  edges:[el('mb-resize-left'),el('mb-resize-right')], onSizeChange:rememberSize});
let scrollbarsReady = false;
function connectPreviewScrollbars() {
  if (scrollbarsReady) return;
  scrollbarsReady = true;
  for (const id of ['mb-list', 'mb-ref-body']) {
    const port = el(id);
    initTableScrollbars(port, {host:port.parentElement, fadeEdges:true});
  }
}

async function open(which) {
  mode = which;
  expr.dispatchEvent(new CustomEvent("pattern:direction", {detail:referenceDirection()}));
  dialog.classList.toggle("is-extract", mode !== MODES.render);
  el("mb-title").textContent = t(mode.title);
  restoreSize();
  expr.value = el(mode.source).value;
  el("mb-ref-filter").value = "";
  el("mb-ref-filter").parentElement.classList.add("is-empty");
  el("mb-error").hidden = true;
  modal.hidden = false;
  sizing.sync();
  connectPreviewScrollbars();
  fitField();
  const { width, height } = dialog.getBoundingClientRect();
  openedSize = { width, height };
  expr.focus();
  expr.setSelectionRange(expr.value.length, expr.value.length);
  refresh();
  try {
    entries = await catalogue();
    referenceFunctions = mode===MODES.render ? await loadTextFunctions().catch(()=>[]) : [];
    renderReference(referenceEntries(), "", el("mb-ref-body"), referenceDirection(), {tagChips:true, functions:referenceFunctions});
  } catch (e) {
    el("mb-ref-body").textContent = String(e);
  }
}

function close() {
  rememberSize();
  clearTimeout(timer);
  generation++;
  queuedRefresh = null;
  modal.hidden = true;
  el(mode.opener).focus();
}

// The mask goes into the box the way typing it there would (it is stored and
// the one-file example follows), and the preview is staged as Preview would.
function accept() {
  if (el("mb-ok").disabled) return;
  const mask = el(mode.source);
  mask.value = expr.value;
  mask.dispatchEvent(new Event("input", { bubbles: true }));
  close();
  mode.accept();
}

el("mask-builder-open").addEventListener("click", () => open(MODES.render));
el("from-name-builder-open").addEventListener("click", () => open(MODES.extract));
el("from-text-builder-open").addEventListener("click", () => open(MODES.text));
el("mb-close").addEventListener("click", close);
el("mb-cancel").addEventListener("click", close);
el("mb-ok").addEventListener("click", accept);
// Cleared as an edit of the field (#542), so Cmd+Z brings the mask back; the
// direct assignment is only for an engine without the editing command.
el("mb-clear").addEventListener("click", () => {
  expr.focus();
  expr.select();
  if (!expr.value || document.execCommand("delete")) return;
  expr.value = "";
  expr.dispatchEvent(new Event("input", { bubbles: true }));
});
expr.addEventListener("input", () => {
  // A mask is one line: a pasted line break would only ever be an error.
  if (/[\r\n]/.test(expr.value)) {
    const caret = expr.selectionStart;
    const before = expr.value.slice(0, caret).replace(/[\r\n]+/g, "");
    expr.value = before + expr.value.slice(caret).replace(/[\r\n]+/g, "");
    expr.setSelectionRange(before.length, before.length);
  }
  fitField();
  schedule();
});
expr.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.isComposing) {
    e.preventDefault();
    accept();
  }
});
// The filter's × (#549): shown only while there is text to clear.
el("mb-ref-clear").addEventListener("click", () => {
  const box = el("mb-ref-filter");
  box.value = "";
  box.dispatchEvent(new Event("input", { bubbles: true }));
  box.focus();
});
el("mb-ref-filter").addEventListener("input", (e) => {
  e.target.parentElement.classList.toggle("is-empty", !e.target.value);
  renderReference(referenceEntries(), e.target.value, el("mb-ref-body"), referenceDirection(), {tagChips:true, functions:referenceFunctions});
});
el("mb-ref-body").addEventListener("click", (e) => {
  const row = e.target.closest(".ph-row[data-token]");
  if (row?.dataset.textFunction) {
    expr.dispatchEvent(new CustomEvent('pattern:add-function',{detail:row.dataset.textFunction}));
  } else if (row) insertAt(expr, row.dataset.token);
});
// A narrower or wider window wraps the mask differently.
new ResizeObserver(() => {
  if (!modal.hidden) fitField();
}).observe(dialog);
// A preset ticked or edited in the Transform window changes every value here.
for (const context of ["renamer", "fromname"]) {
  onChainChanged(context, () => {
    if (modal.hidden || mode.context !== context) return;
    schedule();
  });
}
modal.addEventListener("keydown", (e) => {
  // Whatever is open on top of the builder (the presets menu, the Transform
  // window, the preset editor) closes on Escape first.
  const above =
    !el("preset-menu").hidden ||
    [...document.querySelectorAll(".modal-backdrop:not(#mask-builder)")].some((root) => !root.hidden);
  if (e.key === "Escape" && !above) {
    e.preventDefault();
    close();
  }
});
modal.addEventListener("mousedown", (e) => {
  if (e.target === modal) close();
});
