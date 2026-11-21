// One activity surface, shared with the staged draft. Layout is measured only
// at state boundaries; progress touches text and a compositor-scaled fill.
import { el } from './dom.js';
import { t } from './i18n.js';

const HOLD_MS = 1400;
const easing = 'cubic-bezier(.22,1,.36,1)';
const historyCopy = {
  undo: { running: 'undo.running', success: 'app.undid' },
  redo: { running: 'redo.running', success: 'app.redid' },
};
let operation = 'apply';
let onStop = null, backgroundOwner = null, backgroundTitle = '';
let state = 'draft', pending = false, timer = null, generation = 0;
const animations = new Set();
const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
function animate(node, frames, duration) {
  if (reduced() || !node.animate) return Promise.resolve();
  const player = node.animate(frames, { duration, easing });
  animations.add(player);
  return player.finished.catch(() => {}).finally(() => animations.delete(player));
}
function interrupt() {
  generation++;
  clearTimeout(timer); timer = null;
  for (const player of animations) player.cancel();
  animations.clear();
}
function swap(activity, fromHeight) {
  const card = el('staged-edits');
  const from = fromHeight ?? (card.hidden ? 0 : card.getBoundingClientRect().height);
  for (const player of card.getAnimations()) player.cancel();
  card.hidden = false;
  const outgoing = el(activity ? 'staged-draft' : 'staged-activity');
  const incoming = el(activity ? 'staged-activity' : 'staged-draft');
  const visible = !outgoing.hidden;
  outgoing.hidden = true;
  incoming.hidden = false;
  incoming.inert = false;
  const to = card.getBoundingClientRect().height;
  if (!reduced()) {
    const openBox = { height: `${to}px`, opacity: 1, overflow: 'hidden' };
    const startBox = from
      ? { ...openBox, height: `${from}px` }
      : { height: '0px', opacity: 0, paddingBlock: '0px', marginBlock: '0px', overflow: 'hidden' };
    void animate(card, [startBox, openBox], 430);
    void animate(incoming, [
      { opacity: 0, transform: 'translateY(6px)' },
      { opacity: .25, offset: .28 },
      { opacity: 1, transform: 'translateY(0)' },
    ], 380);
    if (visible) {
      // Keep the old content in the same surface just for the crossfade.
      outgoing.hidden = false;
      outgoing.inert = true;
      outgoing.style.cssText = 'position:absolute;inset:auto var(--sp-3) var(--sp-3);pointer-events:none';
      const version = generation;
      void animate(outgoing, [{ opacity: 1, transform: 'translateY(0)' }, { opacity: 0, transform: 'translateY(-3px)' }], 185).then(() => {
        if (version !== generation) return;
        outgoing.hidden = true; outgoing.style.cssText = '';
      });
    }
  }
}
function resetContent() {
  for (const id of ['staged-draft', 'staged-activity']) el(id).style.cssText = '';
  el('staged-draft').hidden = state !== 'draft';
  el('staged-activity').hidden = state === 'draft';
}
export async function restoreDraft() {
  const card = el('staged-edits');
  const height = card.getBoundingClientRect().height;
  interrupt();
  const version = generation;
  if (pending) {
    state = 'draft'; swap(false, height);
    delete el('staged-edits').dataset.activity;
    return;
  }
  await animate(card, [
    { height: `${height}px`, opacity: 1, marginTop: '8px', marginBottom: '12px', paddingTop: '12px', paddingBottom: '12px', overflow: 'hidden' },
    { height: '0px', opacity: 0, marginTop: '0px', marginBottom: '0px', paddingTop: '0px', paddingBottom: '0px', overflow: 'hidden' },
  ], 360);
  if (version !== generation) return;
  state = 'draft'; card.hidden = !pending; delete card.dataset.activity; resetContent();
}
export function syncStagedVisibility(hasPending) {
  pending = hasPending;
  if (state === 'draft') el('staged-edits').hidden = !pending;
}
export function beginStagedActivity(kind = 'apply', { title, stop } = {}) {
  const card = el('staged-edits');
  const from = card.hidden ? 0 : card.getBoundingClientRect().height;
  interrupt(); resetContent();
  operation = kind;
  backgroundOwner = null;
  backgroundTitle = title || '';
  onStop = stop || null;
  const label = backgroundTitle || t(historyCopy[operation]?.running || 'apply.checking');
  state = 'checking';
  el('staged-edits').dataset.activity = state;
  el('activity-title').textContent = label;
  el('activity-detail').textContent = operation === 'apply' ? t('apply.preflight') : '';
  el('activity-meter').setAttribute('aria-label', label);
  el('activity-percent').textContent = '';
  el('activity-fill').style.transform = 'scaleX(0)';
  el('activity-meter').removeAttribute('aria-valuenow');
  el('activity-stop').hidden = !onStop; el('activity-stop').disabled = false;
  el('activity-dismiss').hidden = true;
  // The inspector may be collapsed, or behind Files in the compact layout.
  document.dispatchEvent(new CustomEvent('tagrex:reveal-module', { detail: 'inspector' }));
  swap(true, from);
}
export function paintStagedActivity({ phase, percent, detail }) {
  if (!['checking', 'writing', 'finishing'].includes(state)) return;
  const title = phase === 'finishing' ? t('apply.finishing') : backgroundTitle || t(historyCopy[operation]?.running || `apply.${phase}`);
  if (state !== phase) {
    state = phase; el('staged-edits').dataset.activity = state;
    el('activity-meter').setAttribute('aria-label', title);
    if (el('activity-title').textContent !== title) {
      void animate(el('activity-title'), [{ opacity: .4 }, { opacity: 1 }], 260);
    }
  }
  if (el('activity-title').textContent !== title) el('activity-title').textContent = title;
  if (el('activity-detail').textContent !== detail) el('activity-detail').textContent = detail;
  const meter = el('activity-meter');
  const value = percent == null ? null : String(percent);
  if (meter.getAttribute('aria-valuenow') === value) return;
  el('activity-percent').textContent = value == null ? '' : `${value}%`;
  if (value == null) meter.removeAttribute('aria-valuenow');
  else {
    el('activity-fill').style.transform = `scaleX(${percent / 100})`;
    meter.setAttribute('aria-valuenow', value);
  }
}
export function stopStagedActivity(title = t('apply.cancelling'), detail = t('apply.stopping')) {
  state = 'cancelling'; el('staged-edits').dataset.activity = state;
  el('activity-stop').disabled = true;
  el('activity-title').textContent = title;
  el('activity-detail').textContent = detail;
}
export function finishStagedActivity(outcome, detail = '', outcomeTitle = '') {
  if (state === 'draft') return;
  const card = el('staged-edits'), from = card.getBoundingClientRect().height;
  for (const player of card.getAnimations()) player.cancel();
  state = outcome; card.dataset.activity = state;
  const history = historyCopy[operation];
  let titleKey = `apply.${outcome}`;
  if (history && outcome === 'success') titleKey = history.success;
  if (history && outcome === 'error') titleKey = `${operation}.failed`;
  const title = outcomeTitle || t(titleKey);
  el('activity-title').textContent = title;
  el('activity-meter').setAttribute('aria-label', title);
  el('activity-detail').textContent = detail || ((history || operation === 'background') && outcome === 'success' ? '' : t(`apply.${outcome}Detail`));
  el('activity-stop').hidden = true;
  el('activity-dismiss').hidden = outcome === 'success';
  if (outcome === 'success') {
    el('activity-fill').style.transform = 'scaleX(1)';
    el('activity-meter').setAttribute('aria-valuenow', '100');
    el('activity-percent').textContent = '100%';
    timer = setTimeout(() => void restoreDraft(), HOLD_MS);
    void animate(card.querySelector('.activity-check'), [{ opacity: 0, transform: 'scale(.75)' }, { opacity: 1, transform: 'scale(1)' }], 310);
  } else {
    el('activity-percent').textContent = '';
    el('activity-meter').removeAttribute('aria-valuenow');
  }
  const to = card.getBoundingClientRect().height;
  void animate(card, [{ height: `${from}px`, overflow: 'hidden' }, { height: `${to}px`, overflow: 'hidden' }], 320);
  void animate(el('activity-title'), [{ opacity: .35, transform: 'translateY(3px)' }, { opacity: 1, transform: 'translateY(0)' }], 290);
}
// Foreground writes own the card while they run. Each background run has its
// own handle, so late events cannot update or dismiss a newer operation.
export function beginBackgroundActivity({ title, stop }) {
  const owner = Symbol();
  if (!['checking', 'writing', 'finishing', 'cancelling'].includes(state) || operation === 'background') {
    beginStagedActivity('background', { title, stop });
    backgroundOwner = owner;
  }
  return {
    progress(percent, detail) {
      if (backgroundOwner === owner) paintStagedActivity({ phase: 'writing', percent, detail });
    },
    stopping(title, detail) {
      if (backgroundOwner === owner) stopStagedActivity(title, detail);
    },
    finish(outcome, title, detail) {
      if (backgroundOwner !== owner) return;
      backgroundOwner = null;
      if (outcome) finishStagedActivity(outcome, detail, title);
      else void restoreDraft();
    },
  };
}

export function initStagedActivity() {
  el('activity-stop').addEventListener('click', () => onStop?.());
  el('activity-dismiss').addEventListener('click', () => {
    const opener = historyCopy[operation] ? el(operation) : el('diff-apply');
    (opener.disabled ? el('panel-toggle') : opener).focus({ preventScroll: true });
    void restoreDraft();
  });
  const preference = matchMedia('(prefers-reduced-motion: reduce)');
  preference.addEventListener('change', () => {
    if (preference.matches) for (const player of animations) player.finish();
  });
  window.addEventListener('pagehide', interrupt, { once: true });
}
