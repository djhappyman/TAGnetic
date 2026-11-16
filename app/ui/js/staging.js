import { el } from './dom.js';
import { t, tn } from './i18n.js';

// One compact summary of the actual pending plan. Preview lives beside Apply;
// clearing here only discards the draft, never tags or files on disk.
export function renderStaging(plan) {
  const count = plan?.changes?.length || 0;
  const hidden = count === 0;
  const label = t('staging.fileCount', { files: tn('unit.file', count) });
  const badge = count ? String(count) : '';
  if (el('staged-edits').hidden !== hidden) el('staged-edits').hidden = hidden;
  if (el('staged-count').textContent !== label) el('staged-count').textContent = label;
  if (el('staged-tab-count').textContent !== badge) el('staged-tab-count').textContent = badge;
}
export function initStaging({ onClear, isBusy }) {
  el('diff-discard').addEventListener('click', () => { if (!isBusy()) onClear(); });
  renderStaging(null);
}
