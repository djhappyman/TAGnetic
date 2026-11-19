import { readableTagName } from './tag-labels.js';
import { fieldOperation, operationFields } from './field-pattern.js';

// Own one preview per dialog. Never retain a second stage or a write path.
export function createFieldFormatActions({ invoke, captureSelection, isSelectionCurrent,
  getPlan, inputRevision, workingField, trackAt, isFieldLocked,
  adoptTagOnlyPreview, recordLiveFields, writeQuickPlan, fieldCatalogue, labelField = readableTagName }) {
  let generation = 0;
  let accepted = null;
  function current(item) {
    return item.revision === inputRevision() && item.base === getPlan() && isSelectionCurrent(item.subject)
      && item.targets.every(field => item.locks.get(field) === isFieldLocked(field));
  }
  return async function run(action, options = {}) {
    if (action === 'field-format-cancel') { generation++; accepted = null; return; }
    if (action === 'field-format-stage' || action === 'field-format-write') {
      const item = accepted;
      if (!item || item.token !== options.token || !current(item)) throw new Error('The files or staged values changed. Preview again.');
      if (item.invalid) throw new Error('Resolve the invalid values before staging.');
      if (action === 'field-format-write') {
        const selected = new Set(item.subject.paths);
        const changes = item.plan.changes.filter(change => selected.has(change.path) && !item.unmatched.has(change.path)).map(change => ({
          ...change, rename_to:null, cover_change:null, block_changes:[], sidecar_renames:[], copy:false, folder_extras:[],
          tag_changes:change.tag_changes.filter(edit => item.targets.includes(edit.field) && !isFieldLocked(edit.field)),
        }));
        await writeQuickPlan({...item.plan, changes});
        accepted = null; generation++;
        return { dialog:'ribbon-format-dialog' };
      }
      const entries = [];
      const changes = new Map(item.plan.changes.map(change => [change.path, change]));
      for (const path of item.subject.paths) {
        if (item.unmatched.has(path)) continue;
        for (const field of item.targets) {
          if (isFieldLocked(field)) continue;
          const edit = changes.get(path)?.tag_changes.find(edit => edit.field === field);
          entries.push({ path, field, value: edit ? edit.new ?? '' : trackAt(path)?.tags?.[field] ?? '' });
        }
      }
      // Adopt unrelated proposals before applying only this operation's fields.
      adoptTagOnlyPreview();
      recordLiveFields(entries);
      accepted = null; generation++;
      return { dialog: 'ribbon-format-dialog' };
    }
    const subject = captureSelection();
    if (!subject.paths.length) throw new Error('Select files first.');
    const custom = new Set();
    const baseChanges = new Map((getPlan()?.changes || []).map(change => [change.path, change]));
    for (const path of subject.paths) {
      for (const key of Object.keys(trackAt(path)?.tags || {})) if (key.startsWith('custom:')) custom.add(key);
      for (const edit of baseChanges.get(path)?.tag_changes || []) if (edit.field.startsWith('custom:')) custom.add(edit.field);
    }
    const fields = [...fieldCatalogue(), ...[...custom].sort().map(key => ({key, label:labelField(key)}))];
    if (action === 'field-format-open') return { fields };
    if (action !== 'field-format-preview') throw new Error('Unknown field formatting action.');
    const token = ++generation;
    accepted = null;
    const operation = fieldOperation(options.kind, options.field, options.source, options.pattern);
    const targets = operationFields(operation, fields);
    if (!targets.length) throw new Error('Add a field to extract into.');
    const item = { token, subject, targets, revision:inputRevision(), base:getPlan(), locks:new Map(targets.map(field => [field, isFieldLocked(field)])) };
    const plan = await invoke('preview_field_operations', { paths:subject.paths, operations:[operation], working:item.base });
    if (token !== generation || !current(item)) throw new Error('The files or staged values changed. Preview again.');
    // Only this extraction's mismatches, not inherited warnings from other tools.
    const unmatched = new Set((plan.unrendered || []).slice(item.base?.unrendered?.length || 0).filter(entry => entry.field === 'extraction pattern').flatMap(entry => entry.paths || []));
    const changes = new Map(plan.changes.map(change => [change.path, change]));
    const selected = new Set(subject.paths);
    const invalid = plan.changes.some(change => selected.has(change.path) && change.tag_changes.some(edit => targets.includes(edit.field) && edit.invalid));
    accepted = { ...item, plan, unmatched, invalid };
    return { token, total:subject.paths.length, unmatched:unmatched.size, locked:targets.filter(isFieldLocked).map(labelField), invalid,
      rows:subject.paths.slice(0, 3).map(path => ({ path, source:workingField(path, options.source).value,
        unmatched:unmatched.has(path), values:targets.map(field => {
          const edit = changes.get(path)?.tag_changes.find(edit => edit.field === field);
          return { field, label:labelField(field), old:workingField(path, field).value,
            value:isFieldLocked(field) ? workingField(path, field).value : edit ? edit.new ?? '' : trackAt(path)?.tags?.[field] ?? '' };
        }) })) };
  };
}
