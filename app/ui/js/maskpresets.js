import { upgradeComponentMarkup } from "../lib/component-elements.js";
// Named mask presets (#360).
//
// A small pool of saved mask patterns the user can drop into any mask input —
// the rename mask, the reorganize folder pattern, FROM NAME, the export mask.
// One shared pool: a pattern saved from one input is offered in all of them,
// because the mask vocabulary is the same everywhere.
//
// The pool lives in the backend settings, like the transform action groups
// (#57): read once from `load_settings`, rewritten with `save_settings`. The UI
// is a delegated popover shared by every mask input, the same shape as the
// placeholder reference (#148) it sits beside.
import { placeFloating, el, escapeHtml, toast } from "./dom.js";
import { catalogue } from './placeholders.js';
import { fieldLabel } from './fields.js';
import { patternParts, fieldHue } from './field-pattern.js';
import { functionParts, functionSummary, loadFunctionLayout } from './function-pattern.js';
import { initTableScrollbars } from './overlay-scrollbars.js';
import { t } from "./i18n.js";
import { invoke } from "./invoke.js";
import { maskPresets, savedSettings, setMaskPresets, setSavedSettings } from "./state.js";

// The input the open popover reads from / writes to, and the button it hangs off.
let target = null;
let anchor = null;
let fields = [];
let busy = false;
// Which preset each mask input last took its mask from (#599), so the menu can
// offer to update that preset rather than ask for its name again.
const origin = new WeakMap();

// Save reads Update when the typed name is an existing preset whose mask differs
// from the field's (#599): saving under that name replaces the saved mask.
function paintSave() {
  const button = el("preset-save");
  const name = el("preset-name").value.trim();
  const existing = maskPresets.find((preset) => preset.name === name);
  const update = !!(existing && target && existing.mask !== target.value.trim());
  const empty = !target?.value.trim();
  button.disabled = busy || !name || empty;
  el('preset-new').disabled = busy || empty;
  el('preset-new').title = empty ? t('preset.noMask') : '';
  button.textContent = t(update ? "preset.update" : "preset.save");
  button.title = update ? t("preset.updateTitle", { name }) : "";
}

async function persist(next) {
  // Save the whole settings back, carrying every other slice (action groups,
  // sidecars, …) untouched — the same rewrite-the-file model the settings use.
  busy = true;
  paintBusy();
  try {
    const settings = { ...savedSettings, mask_presets: next };
    await invoke("save_settings", { settings });
    setSavedSettings(settings);
    setMaskPresets(next);
  } finally {
    busy = false;
    paintBusy();
  }
}

async function savedFailed(error) {
  // Preserve the typed name and pattern for retry. A concurrent agent's saved
  // masks replace the stale pool, never the unsaved field the person is editing.
  try {
    const fresh = await invoke("load_settings", {});
    setSavedSettings(fresh);
    setMaskPresets(Array.isArray(fresh.mask_presets) ? fresh.mask_presets : []);
  } catch { /* Keep the last confirmed pool when storage is unavailable. */ }
  render();
  toast(String(error), true);
}

function paintBusy() {
  el('preset-list').setAttribute('aria-busy', String(busy));
  el('preset-name').disabled = busy;
  for (const button of el('preset-list').querySelectorAll('.preset-del')) button.disabled = busy;
  paintSave();
}

function preview(mask) {
  return functionParts(mask, fields, patternParts).map(part => {
    if (!part.field && !part.fn) return `<span class="preset-literal">${escapeHtml(part.text)}</span>`;
    const label = part.fn ? functionSummary(part.call, fields) : `${part.field.label}${part.width ? ` · ${part.width}` : ''}`;
    return `<span class="field-value-chip field-token-tone" style="--field-hue:${fieldHue(part.field?.key || 'function')}" title="${escapeHtml(part.text)}">${escapeHtml(label)}</span>`;
  }).join('');
}

function render() {
  const list = el("preset-list");
  // Async chip decoration must never replace a row under keyboard focus.
  const focused = list.contains(document.activeElement) ? document.activeElement : null;
  const focusedName = focused?.closest('.preset-row')?.dataset.name;
  const focusedClass = focused?.classList.contains('preset-del') ? 'preset-del' : 'preset-apply';
  const query = el('preset-filter').value.trim().toLocaleLowerCase();
  const shown = maskPresets.filter(preset => `${preset.name} ${preset.mask}`.toLocaleLowerCase().includes(query));
  list.innerHTML = shown.length ? shown.map(preset => {
    const current = preset.mask === target?.value.trim();
    return `<div class="preset-row${current ? ' is-current' : ''}" data-name="${escapeHtml(preset.name)}" data-mask="${escapeHtml(preset.mask)}">
      <button type="button" class="preset-apply" title="${escapeHtml(preset.mask)}"${current ? ' aria-current="true"' : ''}>
        <span class="preset-title-line"><span class="preset-name">${escapeHtml(preset.name)}</span></span>
        <span class="preset-mask" aria-hidden="true">${preview(preset.mask)}</span>
      </button>
      <button type="button" class="preset-del icon" data-name="${escapeHtml(preset.name)}" title="${escapeHtml(t('preset.delete'))}" aria-label="${escapeHtml(t('preset.delete'))}: ${escapeHtml(preset.name)}"${busy ? ' disabled' : ''}>
        <svg class="ico" aria-hidden="true"><use href="#i-close"/></svg>
      </button>
    </div>`;
  }).join('') : `<p class="preset-empty muted">${escapeHtml(t(maskPresets.length ? 'preset.noResults' : 'preset.empty'))}</p>`;
  upgradeComponentMarkup(list);
  if (focusedName) [...list.querySelectorAll('.preset-row')].find(row => row.dataset.name === focusedName)?.querySelector(`.${focusedClass}`)?.focus({preventScroll:true});
}

function showSave(show) {
  paintSave();
  el('preset-save-form').hidden = !show;
  el('preset-new').hidden = show;
  el('preset-new').setAttribute('aria-expanded', String(show));
  if (anchor) place(el('preset-menu'), anchor);
  if (show) { el('preset-name').focus(); el('preset-name').select(); }
}

// Place the popover against its button, mirroring the placeholder reference: a
// tall menu whose panel scrolls, so fixed positioning rather than the absolute
// `.col-menu` shell.
function place(menu, button) {
  placeFloating(menu, button, { align: "right", width: Math.min(360, innerWidth - 16) });
}

export function closePresets() {
  el("preset-menu").hidden = true;
  anchor?.setAttribute("aria-expanded", "false");
  target = null;
  anchor = null;
}

function openPresets(button, input) {
  const menu = el("preset-menu");
  target = input;
  anchor = button;
  menu.returnFocusTo = button;
  button.setAttribute('aria-expanded', 'true');
  button.setAttribute('aria-controls', 'preset-menu');
  el('preset-filter').value = '';
  showSave(false);
  // The preset this field's mask came from, if it still exists (#599).
  const from = origin.get(input);
  const known = from && maskPresets.some((preset) => preset.name === from);
  el("preset-name").value = known ? from : "";
  paintSave();
  render();
  menu.hidden = false;
  place(menu, button);
  el('preset-filter').focus();
  // Use the same native-backed presentation cache as editable function chips.
  void catalogue().then(entries => {
    fields = entries.filter(entry => /^%[^%]+%$/.test(entry.token)).map(entry => ({key:entry.name, label:fieldLabel(entry.name)}));
    if (target === input) render();
    return Promise.allSettled(maskPresets.filter(preset => preset.mask.includes('$')).map(preset => loadFunctionLayout(preset.mask)));
  }).then(() => { if (target === input) render(); }).catch(() => {});
}

// Save the mask currently in the target input under the typed name. An existing
// name is overwritten (renaming a preset is just saving over it), and the list
// stays sorted so it reads the same wherever it opens.
async function saveCurrent() {
  if (!target || busy) return;
  const input = target;
  const name = el("preset-name").value.trim();
  const mask = input.value.trim();
  if (!name) {
    el("preset-name").focus();
    return;
  }
  if (!mask) {
    toast(t("preset.noMask"), true);
    return;
  }
  const replaced = maskPresets.find((preset) => preset.name === name);
  const rest = maskPresets.filter((preset) => preset.name !== name);
  rest.push({ name, mask });
  rest.sort((a, b) => a.name.localeCompare(b.name));
  try { await persist(rest); }
  catch (error) { await savedFailed(error); return; }
  // The field's mask now is that preset's (#599).
  origin.set(input, name);
  if (target === input) { paintSave(); showSave(false); el("preset-new").focus(); }
  render();
  toast(t(replaced && replaced.mask !== mask ? "preset.updated" : "preset.saved", { name }));
}

function applyPreset(mask, name) {
  if (!target) return;
  target.value = mask;
  origin.set(target, name);
  // The live previews (FROM NAME's read-out, the export name) listen for input.
  target.dispatchEvent(new Event("input", { bubbles: true }));
  target.focus();
  closePresets();
}

async function deletePreset(name) {
  if (busy) return;
  const rows = [...el("preset-list").querySelectorAll(".preset-row")];
  const index = rows.findIndex(row => row.dataset.name === name);
  try { await persist(maskPresets.filter((preset) => preset.name !== name)); }
  catch (error) { await savedFailed(error); return; }
  render();
  const remaining = el('preset-list').querySelectorAll('.preset-apply');
  (remaining[Math.min(index, remaining.length - 1)] || el('preset-filter')).focus();
}

export async function initMaskPresets() {
  try {
    setSavedSettings((await invoke("load_settings", {})) || {});
    setMaskPresets(Array.isArray(savedSettings.mask_presets) ? savedSettings.mask_presets : []);
  } catch (e) {
    setMaskPresets([]);
  }

  // One delegated listener for every trigger, so a new mask input needs only the
  // button markup and no wiring.
  document.addEventListener("click", (e) => {
    const button = e.target.closest?.(".preset-btn");
    if (button) {
      e.preventDefault();
      const input = el(button.dataset.presetTarget);
      if (!input) return;
      if (anchor === button && !el("preset-menu").hidden) closePresets();
      else openPresets(button, input);
      return;
    }
    if (e.target.closest?.('#preset-new')) { showSave(true); return; }
    if (e.target.closest?.('#preset-save-cancel')) { showSave(false); el('preset-new').focus(); return; }
    if (e.target.closest?.("#preset-save")) {
      saveCurrent();
      return;
    }
    const del = e.target.closest?.("#preset-menu .preset-del");
    if (del) {
      deletePreset(del.dataset.name);
      return;
    }
    const apply = e.target.closest?.("#preset-menu .preset-apply");
    if (apply) {
      const row = apply.closest(".preset-row");
      applyPreset(row.dataset.mask, row.querySelector(".preset-del").dataset.name);
      return;
    }
    if (!e.target.closest?.("#preset-menu")) closePresets();
  });

  initTableScrollbars(el('preset-list'), {host:el('preset-list').parentElement});
  for (const rail of el('preset-list').parentElement.querySelectorAll('[role="scrollbar"]')) rail.setAttribute('aria-label', t('preset.heading'));
  el('preset-filter').addEventListener('input', render);
  el('preset-filter').addEventListener('keydown', event => {
    if (event.key === 'ArrowDown' || event.key === 'Enter') {
      event.preventDefault();
      const first = el('preset-list').querySelector('.preset-apply');
      if (event.key === 'Enter') first?.click(); else first?.focus();
    }
  });
  el("preset-name").addEventListener("input", paintSave);

  // Enter in the name field saves the current mask under it.
  el("preset-name").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      saveCurrent();
    }
  });

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !el("preset-menu").hidden) {
      const button = anchor;
      closePresets();
      button?.focus();
    }
  });

  // The popover is anchored to a button that moves when the panel or window
  // resizes; re-place rather than leaving it floating somewhere wrong.
  window.addEventListener("resize", () => {
    if (!el("preset-menu").hidden && anchor) place(el("preset-menu"), anchor);
  });
}
