// Presentation of the native stage plus the freshest raw local input. The
// existing edit ledger remains the publication source; this module owns no writes.
import { edits, diffByPath, trackAt, tracks } from './state.js';
let subject = null;
const rawFields = new Map();
export function presentRawField(path, field, value, valid = true) {
  if (subject !== tracks) { rawFields.clear(); subject = tracks; }
  if (!rawFields.has(path)) rawFields.set(path, new Map());
  rawFields.get(path).set(field, {value, valid});
}
export function clearRawFields(paths) {
  if (!paths) rawFields.clear(); else for (const path of paths) rawFields.delete(path);
}

export function retireRevertedRawFields() {
  for (const [path, fields] of rawFields) {
    for (const [field, input] of fields) if (input.valid && !edits.get(path)?.has(field)) fields.delete(field);
    if (!fields.size) rawFields.delete(path);
  }
}

export function workingField(path, field) {
  const saved = trackAt(path)?.tags?.[field] ?? '';
  const accepted = diffByPath?.get(path)?.tag_changes?.find(item => item.field === field);
  const local = edits.get(path);
  const input = subject === tracks ? rawFields.get(path)?.get(field) : undefined;
  const raw = input ? input.value : local?.has(field) ? local.get(field) : undefined;
  const value = raw ?? (accepted ? accepted.new ?? '' : saved);
  return { saved, value, raw, dirty: value !== saved, invalid: input ? !input.valid || ((accepted?.new ?? '') === raw && !!accepted?.invalid) : !!accepted?.invalid };
}

// Change Text.data only when necessary; retain the app-owned text node and
// never touch a browser-owned active contenteditable subtree.
export function patchCellText(cell, value, old = '') {
  if (cell.isContentEditable) return;
  let text = cell.firstChild;
  if (!text || text.nodeType !== 3) {
    text = document.createTextNode('');
    cell.prepend(text);
  }
  if (text.data !== value) text.data = value;
  let prior = cell.querySelector('.cell-old');
  if (old) {
    if (!prior) { prior = document.createElement('span'); prior.className = 'cell-old'; cell.append(prior); }
    if (prior.textContent !== old) prior.textContent = old;
  } else if (prior) prior.remove();
}
