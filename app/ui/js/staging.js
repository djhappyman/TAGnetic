import { el } from './dom.js';
import { createStagingSummary } from './staging-summary.js';
import { syncStagedVisibility, initStagedActivity } from './staged-activity.js';
import { tn } from './i18n.js';

const summarize = createStagingSummary();

// One compact summary of the actual pending plan. Preview lives beside Apply;
// clearing here only discards the draft, never tags or files on disk.
export function renderStaging(plan, isNativeTransformed, delta) {
  const count = plan?.changes?.length || 0;
  const hidden = count === 0;
  const label = tn('unit.file', count);
  const { transformations, tags } = summarize(plan, isNativeTransformed, delta);
  for (const [id, amount, unit] of [['staged-transformations', transformations, 'staging.transformations'], ['staged-tags', tags, 'staging.tagChanges']]) {
    const node = el(id), text = tn(unit, amount);
    if (node.hidden !== (amount === 0)) node.hidden = amount === 0;
    if (node.textContent !== text) node.textContent = text;
  }
  const badge = count ? String(count) : '';
  syncStagedVisibility(!hidden);
  if (el('staged-count').textContent !== label) el('staged-count').textContent = label;
  if (el('staged-tab-count').textContent !== badge) el('staged-tab-count').textContent = badge;
}
export function initStaging({ onClear, isBusy }) {
  el('diff-discard').addEventListener('click', () => { if (!isBusy()) onClear(); });
  initStagedActivity();
  renderStaging(null);
}
