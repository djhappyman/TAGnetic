// BPM detection (#536): measure the tempo of the selected files from the audio
// and stage it in the BPM field, through the ordinary preview/apply/undo path.
//
// The measuring takes seconds a file, so it runs in the backend while the window
// stays usable. The toolbar button is one toggling slot, like the online search
// (#111): it starts a run, and while one is going it is a Stop that ends it at
// the current file, staging what was measured before. The shared activity
// card tracks its progress and offers the same Stop action.
//
// Settings can switch the whole thing off (#538), which hides the button; the
// shortcut presses the button, so it goes quiet with it.
import { el, report, toast } from "./dom.js";
import { invoke } from "./invoke.js";
import { t, tn } from "./i18n.js";
import { beginBackgroundActivity } from './staged-activity.js';
import { hooks } from "./hooks.js";
import { applyColumns } from "./columns.js";
import { liveInputRevision } from "./live-preview.js";
import {
  captureSelection,
  isSelectionCurrent,
  previewPlan,
  selectedPaths,
  setPreviewPlan,
  setPreviewSource,
  visibleColumns,
} from "./state.js";

const EVENT = "tagrex://bpm-progress";
const button = el("detect-bpm");

let running = false;
let stopping = false;
let operationRevision = 0;
let activeRun = 0;
let activity = null;

// Title and label come from here rather than from data-i18n attributes: they
// change with the run, and a language switch would otherwise put the idle text
// back on a Stop button (#287).
function paintButton() {
  button.classList.toggle("busy", running);
  button.disabled = stopping;
  button.title = running ? t("toolbar.detectBpmStop") : t("toolbar.detectBpmTitle");
  button.setAttribute("aria-label", running ? t("toolbar.detectBpmStop") : t("toolbar.detectBpm"));
}

function paintProgress(done, total, path) {
  if (stopping) return;
  const filename = path?.split(/[\\/]/).pop();
  const count = t('bpm.progress', { done, total });
  activity?.progress(total > 0 ? Math.min(99, Math.round(done / total * 100)) : null,
    filename ? `${count} · ${filename}` : count);
}

// The report after a run (#558): what was staged as the headline, then a line
// for each other thing worth knowing. Stopping and skipped mixes are fine;
// no steady beat and files that would not decode need a look.
function reportRun(result, total) {
  const staged = result.plan.changes.length;
  report(staged ? t("toast.bpm.staged", { files: tn("unit.file", staged) }) : t("toast.bpm.unchanged"), [
    result.cancelled && { text: t("toast.bpm.stopped", { done: result.analysed, total }) },
    result.too_long && { text: tn("toast.bpm.tooLong", result.too_long) },
    result.no_tempo && { text: tn("toast.bpm.noTempo", result.no_tempo), problem: true },
    result.failed.length && { text: tn("toast.bpm.failed", result.failed.length), problem: true },
  ]);
}

// Stop the run at the current file, keeping what was measured. From the button
// and from Escape (#539).
export function stopBpm() {
  if (!running || stopping) return;
  stopping = true;
  paintButton();
  activity?.stopping(t('bpm.stopping'), '');
  void invoke('cancel_detect_bpm', {}).catch(error => toast(String(error), true));
}

export function bpmRunning() {
  return running;
}

// Invalidate before a folder-open request replaces the track list. Wait for the
// old run to settle before allowing another to reset native cancellation state.
export function invalidateBpm() {
  operationRevision++;
  stopBpm();
}

async function detectBpm() {
  if (running) {
    stopBpm();
    return;
  }
  const paths = selectedPaths();
  if (paths.length === 0) {
    toast(t("toast.bpm.select"), true);
    return;
  }
  const run = ++operationRevision;
  activeRun = run;
  const subject = captureSelection();
  const inputRevision = liveInputRevision();
  const startingPlan = previewPlan;
  const current = () => run === operationRevision && isSelectionCurrent(subject) &&
    liveInputRevision() === inputRevision && previewPlan === startingPlan;
  running = true;
  stopping = false;
  paintButton();
  activity = beginBackgroundActivity({ title: t('toolbar.detectBpm'), stop: stopBpm });
  paintProgress(0, paths.length);
  const events = window.__TAURI__ && window.__TAURI__.event;
  let unlisten = null, outcome = null, title, detail;
  try {
    unlisten = events
      ? await events.listen(EVENT, (e) => { if (current()) paintProgress(e.payload.done, e.payload.total, e.payload.path); })
      : null;
    if (!current()) return;
    const result = await invoke("detect_bpm", { paths });
    if (!current()) return;
    // A run that measured nothing new leaves whatever is staged alone.
    if (result.plan.changes.length || !previewPlan) {
      // The diff shows a change in the column of its field, and a BPM column
      // the table does not have would leave the preview with nothing in it. It
      // stays afterwards, like any column picked in the menu.
      if (result.plan.changes.length && !visibleColumns.includes("bpm")) {
        applyColumns([...visibleColumns, "bpm"]);
      }
      setPreviewPlan(result.plan);
      setPreviewSource("bpm");
      hooks.renderPreview(previewPlan);
    }
    reportRun(result, paths.length);
    const staged = result.plan.changes.length;
    title = staged ? t('toast.bpm.staged', { files: tn('unit.file', staged) }) : t('toast.bpm.unchanged');
    if (result.cancelled) detail = t('toast.bpm.stopped', { done: result.analysed, total: paths.length });
    outcome = result.cancelled ? 'cancelled' : 'success';
  } catch (e) {
    if (current()) { outcome = 'error'; title = t('toolbar.detectBpm'); detail = String(e); }
  } finally {
    if (unlisten) unlisten();
    if (activeRun !== run) return;
    activeRun = 0;
    running = false;
    stopping = false;
    activity?.finish(outcome, title, detail);
    activity = null;
    paintButton();
  }
}

export function retranslateBpm() {
  paintButton();
}

export function setBpmEnabled(on) {
  button.hidden = !on;
}

button.addEventListener("click", detectBpm);
paintButton();
invoke("load_settings", {})
  .then((settings) => setBpmEnabled(!settings || settings.bpm_detection !== false))
  .catch(() => {});
