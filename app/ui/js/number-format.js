function saved(key) {
  try { return Math.max(1, Math.min(3, Number(localStorage.getItem('tagrex.' + key + 'Digits')) || 1)); }
  catch { return 1; }
}
const widths = { track: saved('track'), disc: saved('disc') };
export function numberWidth(key) { return widths[key] || 1; }
export function setNumberWidth(key, value) {
  if (!(key in widths)) return;
  const width = Math.max(1, Math.min(3, Number(value) || 1));
  // The ribbon sets both widths after Numbering, usually to the values already
  // stored. Re-rendering the table for an unchanged preference is pure work.
  if (width === widths[key]) return;
  widths[key] = width;
  try { localStorage.setItem('tagrex.' + key + 'Digits', String(width)); } catch { /* Optional preference. */ }
  document.dispatchEvent(new CustomEvent('tagrex:number-format'));
}
export function formatNumber(value, key) {
  const text = String(value ?? '');
  return key in widths && /^\d+$/.test(text) ? text.padStart(widths[key], '0') : text;
}
