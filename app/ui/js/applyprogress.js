// Progress and Cancel for a running Apply (#437).
//
// The backend emits `tagrex://apply-progress` while it writes and stops at the
// next file after `cancel_apply`. The staged surface carries checking, writing
// and the confirmed result; row feedback still receives every completed step.
import { el } from "./dom.js";
import { invoke } from "./invoke.js";
import { t } from "./i18n.js";
import { beginStagedActivity, paintStagedActivity, stopStagedActivity, finishStagedActivity, restoreDraft } from './staged-activity.js';

const EVENT = "tagrex://apply-progress";

let unlisten = null;

function baseName(path) {
  return String(path || "").split(/[\\/]/).pop();
}

let frame = null, latest = null, fileCount = 0, percent = 0, acceptingProgress = false;
let historyOperation = null;
let required = new Map(), completed = new Map(), completedFiles = 0;
function flushProgress() {
  frame = null;
  if (!latest) return;
  const progress = latest; latest = null;
  if (historyOperation) {
    if (progress.total > 0) percent = Math.max(percent, Math.min(99, Math.round(progress.done / progress.total * 100)));
    paintStagedActivity({
      phase: progress.total > 0 ? 'writing' : 'checking',
      percent: progress.total > 0 ? percent : null,
      detail: baseName(progress.path),
    });
    return;
  }
  const checking = !progress.completed && progress.done < fileCount;
  const steps = Math.max(1, progress.total - fileCount);
  if (!checking) percent = Math.max(percent, Math.min(99, Math.round((progress.done - fileCount) / steps * 100)));
  const name = baseName(progress.path);
  paintStagedActivity({ phase: checking ? 'checking' : 'writing', percent: checking ? null : percent,
    detail: checking ? name : t('apply.filesProgress', { done: completedFiles, total: fileCount, file: name }) });
}
function queueProgress(progress) {
  latest = progress;
  if (frame === null) frame = requestAnimationFrame(flushProgress);
}
function receive(progress, onProgress) {
  if (!acceptingProgress) return;
  const bit = progress.completed === 'tags' ? 1 : progress.completed === 'transfer' ? 2 : 0;
  if (bit && required.has(progress.path)) {
    const old = completed.get(progress.path) || 0, next = old | bit;
    completed.set(progress.path, next);
    const mask = required.get(progress.path);
    if ((old & mask) !== mask && (next & mask) === mask) completedFiles++;
  }
  queueProgress(progress);
  onProgress(progress); // Row feedback retains every confirmed step.
}
export async function beginApplyProgress(onProgress = () => {}, changes = []) {
  acceptingProgress = true;
  fileCount = changes.length; percent = 0; completedFiles = 0;
  required = new Map(changes.map(change => [change.path,
    (change.tag_changes?.length || change.cover_change || change.block_changes?.length ? 1 : 0) | (change.rename_to ? 2 : 0)]));
  completed = new Map();
  el('diff-apply').setAttribute('aria-busy', 'true');
  beginStagedActivity('apply', { stop: cancelApply });
  unlisten = await listenProgress(progress => receive(progress, onProgress));
}
async function listenProgress(callback) {
  const events = window.__TAURI__?.event;
  if (events) return events.listen(EVENT, event => callback(event.payload));
  const listener = event => callback(event.detail);
  document.addEventListener(EVENT, listener);
  return () => document.removeEventListener(EVENT, listener);
}
export function finishingWriteProgress() {
  acceptingProgress = false;
  if (frame !== null) cancelAnimationFrame(frame);
  frame = null; latest = null;
  el('activity-stop').disabled = true;
  paintStagedActivity({ phase: 'finishing', percent: 99, detail: t('apply.refreshing') });
}

// History operations share the card and event stream. The backend does not
// offer cancellation here, so the card omits Stop.
export async function beginHistoryProgress(operation) {
  historyOperation = operation;
  acceptingProgress = true;
  percent = 0;
  beginStagedActivity(operation);
  unlisten = await listenProgress(progress => {
    if (acceptingProgress) queueProgress(progress);
  });
}

export function endHistoryProgress(outcome, detail) {
  if (!historyOperation) return;
  historyOperation = null;
  stopListening();
  // A history conflict hands control to the decision dialog. Cancel returns
  // directly to the draft, while a retry starts a fresh activity.
  if (outcome) finishStagedActivity(outcome, detail);
  else void restoreDraft();
}

function stopListening() {
  acceptingProgress = false;
  if (unlisten) unlisten();
  unlisten = null;
  if (frame !== null) cancelAnimationFrame(frame);
  frame = null;
  latest = null;
}

export function endApplyProgress(outcome = "error", detail = "") {
  stopListening();
  required.clear(); completed.clear();
  finishStagedActivity(outcome, detail);
  el("diff-apply").removeAttribute("aria-busy");
}

// The backend stops at the next file and rolls back; until it answers, the
// button is spent and the bar says so.
function cancelApply() {
  stopStagedActivity();
  invoke('cancel_apply', {}).catch(error => {
    // A failed cancellation request does not mean the executor has stopped.
    el('activity-detail').textContent = String(error);
  });
}
