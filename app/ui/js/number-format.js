function saved(key) {
  try { return Math.max(1, Math.min(3, Number(localStorage.getItem('tagrex.' + key + 'Digits')) || 1)); }
  catch { return 1; }
}
const widths = { track: saved('track'), disc: saved('disc') };
export function numberWidth(key) { return widths[key] || 1; }
export function setNumberWidth(key, value) {
  if (!(key in widths)) return;
  widths[key] = Math.max(1, Math.min(3, Number(value) || 1));
  try { localStorage.setItem('tagrex.' + key + 'Digits', String(widths[key])); } catch { /* Optional preference. */ }
  document.dispatchEvent(new CustomEvent('tagrex:number-format'));
}
export function formatNumber(value, key) {
  const text = String(value ?? '');
  return key in widths && /^\d+$/.test(text) ? text.padStart(widths[key], '0') : text;
}
