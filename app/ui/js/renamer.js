// The RENAMER mode (#143 split it out of app.js; unified in #382).
//
// One mask over the selection: a `/` or `\` in it restructures folders
// wherever the file already is, the same way renaming it in place always
// has — no second pattern to keep in sync with the first. Reorganizing is an
// on/off extra on top: move or copy the same result to a different folder
// instead of leaving files where they sit. Both directions produce an
// ordinary plan, so preview, apply and undo work exactly like every other
// change.
import { el, toast, fileName } from "./dom.js";
import { t, tn } from "./i18n.js";
import { invoke } from "./invoke.js";
import { onChainChanged, runChainOverPlan } from "./chains.js";
import { hooks } from "./hooks.js";
import {
  previewPlan,
  previewSource,
  savedSettings,
  selectedPaths,
  setPreviewPlan,
  setPreviewSource,
  setSavedSettings,
} from "./state.js";

// The destination, mode, and whether reorganizing is even on are working
// state for the panel, not backend settings — and re-picking the same
// library folder every session is exactly the friction this feature exists
// to remove.
const MOVE_DEST_STORAGE_KEY = "tagrex.moveDestination";
const MOVE_MODE_STORAGE_KEY = "tagrex.moveMode";
const MOVE_PRUNE_STORAGE_KEY = "tagrex.movePrune";
const REORGANIZE_STORAGE_KEY = "tagrex.reorganizeEnabled";
// The mask itself (#402): the pattern last typed or picked, the way FROM NAME
// keeps its own (#141) — a relaunch used to put the markup's default back.
const MASK_STORAGE_KEY = "tagrex.renameMask";

let moveMode = "move";
let reorganize = false;

function renamePlan(paths, mask = el("mask").value) {
  return invoke("preview_rename", { mask, paths });
}

function movePlan(paths, mask = el("mask").value) {
  return invoke("preview_move", {
    mask,
    paths,
    destination: el("move-dest").value || null,
    copy: moveMode === "copy",
    pruneEmptyDirs: el("move-prune").checked,
  });
}

async function preview() {
  const paths = selectedPaths();
  if (paths.length === 0) {
    toast(reorganize ? t("renamer.selectToMove") : t("toast.selectOneTrack"), true);
    return;
  }
  try {
    // RENAMER's own chain runs on the names this just produced (#237) — where a
    // space becoming an underscore is the usual wish, and exactly the wish that
    // must not reach the tags FROM NAME reads.
    const plan = await (reorganize ? movePlan(paths) : renamePlan(paths));
    const staged = await runChainOverPlan(plan, "renamer");
    setPreviewPlan(staged);
    setPreviewSource("rename");
    hooks.renderPreview(previewPlan);
    // Files the mask couldn't render because a placeholder it requires was
    // empty (#411) — the reason a rename "did nothing", said instead of left
    // to guess.
    const missing = unrenderedParts(staged);
    if (!staged.changes.length && missing) {
      toast(t(reorganize ? "renamer.nothingUnrenderedMove" : "renamer.nothingUnrendered", { parts: missing }), true);
    } else if (reorganize) {
      // Reported from the plan just built, not the staged one (#410): an empty
      // plan makes renderPreview leave the diff state, which clears
      // previewPlan out from under this message — the same trap #145 fixed
      // for GENERATOR.
      const copy = moveMode === "copy";
      const done = staged.changes.length
        ? t(copy ? "toast.previewingCopy" : "toast.previewingMove", {
            files: tn("unit.file", staged.changes.length),
          })
        : t("toast.nothingToMove");
      toast(missing ? `${done} · ${t("renamer.someUnrendered", { parts: missing })}` : done, staged.changes.length === 0);
    } else if (missing) {
      toast(t("renamer.someUnrendered", { parts: missing }));
    }
  } catch (e) {
    toast(String(e), true);
  }
}

// What `mask` makes of `paths`, the way Preview builds it: rename or reorganize
// as the panel is set, then the panel's ticked rules. The mask builder (#540)
// shows this for a mask that is not in the box yet, so its list and the plan
// it stages can never disagree.
async function planForMask(mask, paths) {
  const plan = await (reorganize ? movePlan(paths, mask) : renamePlan(paths, mask));
  return runChainOverPlan(plan, "renamer");
}

// "%disc% empty on 2 files, %year% empty on 1 file" — or "" when the mask
// rendered for every file it was given (#411).
function unrenderedParts(plan) {
  return (plan?.unrendered || [])
    .map((u) => t("renamer.unrenderedPart", { field: `%${u.field}%`, files: tn("unit.file", u.files) }))
    .join(", ");
}

// ---- reorganize on/off (#153 destination + copy, unified in #382) ----

function updatePreviewLabel() {
  const key = !reorganize
    ? "renamer.previewRename"
    : moveMode === "copy"
      ? "renamer.previewCopy"
      : "renamer.previewMove";
  // The button's data-i18n follows the label: the static-text pass at startup
  // (and on every language change) rewrites it from that key, so a fixed
  // "previewRename" there overwrote "Preview move" right after it was set.
  const btn = el("preview");
  btn.dataset.i18n = key;
  btn.textContent = t(key);
}

function setReorganize(on) {
  reorganize = on;
  syncDestinationMode();
  updatePreviewLabel();
  writeStored(REORGANIZE_STORAGE_KEY, on ? "1" : "");
  scheduleMaskExample();
}

// Move or copy. Pruning only means anything for a move — a copy empties
// nothing — so its cleanup option is hidden and disabled for copies.
function setMoveMode(mode) {
  moveMode = mode;
  syncDestinationMode();
  updatePreviewLabel();
  writeStored(MOVE_MODE_STORAGE_KEY, mode);
  scheduleMaskExample();
}

function syncDestinationMode() {
  const input = el('rename-destination-mode');
  const value = reorganize ? moveMode : 'keep';
  if (input.value !== value) {
    input.value = value;
    // Keep the native select binding and its visible proxy in sync on restore.
    input.dispatchEvent(new CustomEvent('change', {bubbles:true, detail:{renamerSync:true}}));
  }
  el('reorganize-options').hidden = !reorganize;
  el('carry-extras-option').hidden = !reorganize;
  el('move-prune-option').hidden = !reorganize || moveMode === 'copy';
  el('move-prune').disabled = !reorganize || moveMode === 'copy';
}

function writeStored(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch (e) {
    /* localStorage unavailable — preference just won't persist */
  }
}

function readStored(key) {
  try {
    return localStorage.getItem(key);
  } catch (e) {
    return null;
  }
}

// The native folder chooser, the same plugin the library Browse button uses.
// Outside Tauri there is none, so browser-dev gets a typed path instead — it is
// the only way to exercise the flow there.
async function pickDestination() {
  const dialog = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dialog) {
    const typed = prompt("Destination folder (browser dev only)");
    if (typed) setDestination(typed);
    return;
  }
  try {
    const picked = await dialog.open({ directory: true, multiple: false });
    if (!picked) return; // user cancelled
    setDestination(picked);
  } catch (e) {
    toast(String(e), true);
  }
}

function setDestination(path) {
  el("move-dest").value = path || "";
  syncDestinationField();
  writeStored(MOVE_DEST_STORAGE_KEY, path || "");
  scheduleMaskExample();
}

// The field's × only while a destination is set: with none, the library is
// already where things go (#547).
function syncDestinationField() {
  el("move-dest").parentElement.classList.toggle("is-empty", !el("move-dest").value);
}

// ---- what travels with a track (#405) ----
// Same-named files and a folder's leftovers are backend settings (they change
// what a plan carries), set here because this is where that plan is made. The
// panel writes them straight to settings.json, the same file Settings saves.
async function setCarry(key, on) {
  setSavedSettings({ ...savedSettings, [key]: on });
  try {
    await invoke("save_settings", { settings: savedSettings });
  } catch (e) {
    toast(String(e), true);
    return;
  }
  scheduleMaskExample();
  // A staged rename was built with the old setting; build it again.
  if (previewSource === "rename" && previewPlan?.changes.length) preview();
}

// Show the saved values once settings.json is loaded (app.js calls this).
function syncCarryToggles() {
  el("carry-sidecars").checked = savedSettings.carry_sidecars !== false;
  el("carry-extras").checked = savedSettings.carry_folder_extras !== false;
}

// ---- live single-file example (#382) ----
// The read-out under the mask: what it renders for the first selected file,
// the same idea as FROM NAME's own probe (extraction rather than rendering,
// but the same "show, don't make me click Preview" point) — a per-keystroke
// round trip, debounced.
let maskExampleVersion = 0;
async function refreshMaskExample() {
  const box = el('mask-example');
  const sample = box.closest('.rename-example');
  const note = el('mask-example-note');
  const version = maskExampleVersion;
  const path = selectedPaths()[0];
  note.hidden = true;
  box.removeAttribute('title');
  sample.dataset.state = path ? 'loading' : 'empty';
  sample.setAttribute('aria-busy', String(!!path));
  box.textContent = t(path ? 'renamer.exampleLoading' : 'renamer.exampleSelect');
  if (!path) return;
  try {
    const plan = await (reorganize ? movePlan([path]) : renamePlan([path]));
    const out = await runChainOverPlan(plan, 'renamer');
    if (version !== maskExampleVersion || selectedPaths()[0] !== path) return;
    const change = out.changes[0];
    const to = change?.rename_to;
    const missing = !to && out.unrendered?.[0]?.field;
    box.textContent = to ? maskPart(to, change.rename_root) : fileName(path);
    box.title = to || path;
    sample.dataset.state = missing ? 'error' : to ? 'changed' : 'unchanged';
    note.textContent = missing ? t('renamer.exampleUnrendered', {field:`%${missing}%`})
      : '';
    note.hidden = !note.textContent;
  } catch (error) {
    if (version !== maskExampleVersion || selectedPaths()[0] !== path) return;
    sample.dataset.state = 'error';
    box.textContent = String(error);
  } finally {
    if (version === maskExampleVersion) sample.setAttribute('aria-busy','false');
  }
}

// Exactly what the mask produced (#384): the target below the folder it was
// rendered under — the file's own folder, or the destination — the way an
// inline example in a tagger usually reads. No root to split at (an older plan)
// falls back to the last few segments.
function maskPart(fullPath, root) {
  if (root) {
    const base = root.replace(/[\\/]+$/, "");
    if (fullPath.startsWith(base) && /[\\/]/.test(fullPath[base.length] || "")) {
      return fullPath.slice(base.length + 1);
    }
  }
  const sep = fullPath.includes("\\") && !fullPath.includes("/") ? "\\" : "/";
  return fullPath.split(/[\\/]/).filter(Boolean).slice(-3).join(sep);
}

let maskExampleTimer = null;
function scheduleMaskExample() {
  maskExampleVersion += 1;
  if (el("panel-renamer").hidden) return;
  clearTimeout(maskExampleTimer);
  maskExampleTimer = setTimeout(refreshMaskExample, 180);
}

// ---- wire up ----
el("preview").addEventListener("click", preview);
el("mask").addEventListener("input", () => {
  writeStored(MASK_STORAGE_KEY, el("mask").value);
  scheduleMaskExample();
});
el('rename-destination-mode').addEventListener('change', (event) => {
  if (event.detail?.renamerSync) return;
  const value = event.target.value;
  if (value !== 'keep') setMoveMode(value === 'copy' ? 'copy' : 'move');
  setReorganize(value !== 'keep');
});
el("move-dest-pick").addEventListener("click", pickDestination);
// The path is read-only: clicking it picks a folder, like the folder button
// beside it (#547).
el("move-dest").addEventListener("click", pickDestination);
el("move-dest-clear").addEventListener("click", () => setDestination(""));
el("carry-sidecars").addEventListener("change", (e) => setCarry("carry_sidecars", e.target.checked));
el("carry-extras").addEventListener("change", (e) => setCarry("carry_folder_extras", e.target.checked));
el("move-prune").addEventListener("change", () => {
  writeStored(MOVE_PRUNE_STORAGE_KEY, el("move-prune").checked ? "1" : "");
  scheduleMaskExample();
});

// Last session's mask (#402), destination, mode and whether reorganizing was on
// (#153, #382). An empty stored mask is not restored: the markup's default is a
// better start than a blank field.
el("mask").value = readStored(MASK_STORAGE_KEY) || el("mask").value;
el("move-dest").value = readStored(MOVE_DEST_STORAGE_KEY) || "";
syncDestinationField();
el("move-prune").checked = !!readStored(MOVE_PRUNE_STORAGE_KEY);
setMoveMode(readStored(MOVE_MODE_STORAGE_KEY) === "copy" ? "copy" : "move");
setReorganize(!!readStored(REORGANIZE_STORAGE_KEY));

// RENAMER has no read-out under its pattern beyond the single-file example: the
// staged diff IS the full example, so a chain change redoes the preview that
// produced it (#248) and refreshes the example the same way.
onChainChanged("renamer", () => {
  scheduleMaskExample();
  if (previewSource === "rename" && previewPlan?.changes.length) preview();
});

export { maskPart, planForMask, preview, scheduleMaskExample, syncCarryToggles };
