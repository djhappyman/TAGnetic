// Progress and Cancel for a running Apply (#437).
//
// The backend emits `tagrex://apply-progress` while it writes and stops at the
// next file after `cancel_apply`. This only paints that in the diff bar; the
// bar shows nothing for the first moment, so a quick apply does not flash it.
import { el } from "./dom.js";
import { invoke } from "./invoke.js";
import { t } from "./i18n.js";

const EVENT = "tagrex://apply-progress";
const SHOW_AFTER_MS = 250;

let unlisten = null;
let showTimer = null;

function baseName(path) {
  return String(path || "").split(/[\\/]/).pop();
}

function paintMeter(id, done, total, label) {
  const fill = el(id), meter = fill.parentElement;
  const percent = total > 0 ? Math.max(0, Math.min(100, Math.round(done / total * 100))) : 0;
  fill.style.width = `${percent}%`;
  meter.setAttribute('role', 'progressbar');
  meter.setAttribute('aria-label', label);
  meter.setAttribute('aria-valuemin', '0');
  meter.setAttribute('aria-valuemax', '100');
  if (total > 0) meter.setAttribute('aria-valuenow', String(percent));
  else meter.removeAttribute('aria-valuenow');
  return percent;
}

function paint(progress) {
  const total = progress.total || 0;
  paintMeter("ab-bar-fill", progress.done, total, el("diff-apply").getAttribute("aria-label") || t("apply.checking"));
  if (!el("diff-cancel-apply").disabled) el("ab-file").textContent = baseName(progress.path);
}

export async function beginApplyProgress(onProgress = () => {}) {
  const bar = el("diff-actionbar");
  el("diff-apply").setAttribute("aria-busy", "true");
  paintMeter("ab-bar-fill", 0, 0, t("apply.checking"));
  el("ab-file").textContent = t("apply.checking");
  el("diff-cancel-apply").disabled = false;
  showTimer = setTimeout(() => bar.classList.add("applying"), SHOW_AFTER_MS);
  const events = window.__TAURI__ && window.__TAURI__.event;
  if (events) unlisten = await events.listen(EVENT, (e) => { paint(e.payload); onProgress(e.payload); });
  else {
    const listener = event => { paint(event.detail); onProgress(event.detail); };
    document.addEventListener(EVENT, listener);
    unlisten = () => document.removeEventListener(EVENT, listener);
  }
}

// The same events while an undo runs (#448), painted in the status bar since there
// may be no diff bar to put them in. No Cancel: a rollback stopped half-way would
// leave the batch partly undone.
let undoUnlisten = null;
let undoTimer = null;

export async function beginUndoProgress(labelKey = "undo.running") {
  // A rollback can't be stopped, so the pill shows no Stop (#558).
  el("busy-stop").hidden = true;
  paintMeter("busy-fill", 0, 0, t(labelKey));
  el("busy-text").textContent = t(labelKey);
  undoTimer = setTimeout(() => (el("busy-pill").hidden = false), SHOW_AFTER_MS);
  const events = window.__TAURI__ && window.__TAURI__.event;
  if (events) {
    undoUnlisten = await events.listen(EVENT, (e) => {
      const { done, total } = e.payload;
      const percent = paintMeter("busy-fill", done, total, t(labelKey));
      el("busy-text").textContent = `${t(labelKey)} ${percent}%`;
    });
  }
}

export function endUndoProgress() {
  clearTimeout(undoTimer);
  undoTimer = null;
  if (undoUnlisten) undoUnlisten();
  undoUnlisten = null;
  el("busy-pill").hidden = true;
}

export function endApplyProgress() {
  clearTimeout(showTimer);
  showTimer = null;
  if (unlisten) unlisten();
  unlisten = null;
  el("diff-apply").removeAttribute("aria-busy");
  el("diff-actionbar").classList.remove("applying", "cancelling");
}

// The backend stops at the next file and rolls back; until it answers, the
// button is spent and the bar says so.
el("diff-cancel-apply").addEventListener("click", () => {
  el("diff-cancel-apply").disabled = true;
  // Cancelling reads as stopping does (#558): the bar goes muted.
  el("diff-actionbar").classList.add("cancelling");
  el("ab-file").textContent = t("apply.cancelling");
  invoke("cancel_apply", {});
});
