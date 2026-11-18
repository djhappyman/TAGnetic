// UI-only provenance. Never sent to the backend or persisted in audio tags.
const transformed = new WeakSet();
const key = (path, field) => `${path}\0${field}`;
const value = raw => raw ?? '';

export function recordTransformationChanges(command, args, plan) {
  if (!plan?.changes || !['preview_live_edits', 'preview_transform_groups', 'preview_transform_over_plan'].includes(command)) return;
  if (command === 'preview_live_edits' && !args?.groups?.length) return;
  const prior = new Map();
  for (const change of args?.plan?.changes || []) for (const tag of change.tag_changes || []) prior.set(key(change.path, tag.field), tag);
  const inputs = new Map((args?.edits || []).map(edit => [key(edit.path, edit.field), edit.value]));
  for (const change of plan.changes) for (const tag of change.tag_changes || []) {
    const id = key(change.path, tag.field), before = prior.get(id);
    const baseline = command === 'preview_live_edits'
      ? (inputs.has(id) ? inputs.get(id) : tag.old)
      : before ? before.new : tag.old;
    if (value(tag.new) !== value(baseline) || (before && transformed.has(before))) transformed.add(tag);
  }
}

export function summarizeStaging(plan, isNativeTransformed = () => false) {
  let transformations = 0, tags = 0;
  for (const change of plan?.changes || []) for (const tag of change.tag_changes || []) {
    // A field has one final value, regardless of how many rules changed it.
    if (value(tag.old) === value(tag.new)) continue;
    if (transformed.has(tag) || isNativeTransformed(change.path, tag.field)) transformations++;
    else tags++;
  }
  return { transformations, tags };
}

// Native deltas patch only their changed rows, preserving typing performance
// when the shared stage holds a large library. Full snapshots rebuild once.
export function createStagingSummary() {
  let current, rows = new Map(), totals = {transformations:0, tags:0};
  return (plan, isNativeTransformed, delta) => {
    if (!delta?.nativeDelta || current !== plan) {
      rows = new Map(); totals = {transformations:0, tags:0};
      for (const change of plan?.changes || []) {
        const counts = summarizeStaging({changes:[change]}, isNativeTransformed);
        rows.set(change.path, counts);
        totals.transformations += counts.transformations; totals.tags += counts.tags;
      }
    } else for (const path of new Set((delta.changedKeys || []).map(entry => entry.path))) {
      const before = rows.get(path) || {transformations:0, tags:0};
      const change = delta.changeAt(path);
      const after = summarizeStaging(change ? {changes:[change]} : null, isNativeTransformed);
      totals.transformations += after.transformations - before.transformations;
      totals.tags += after.tags - before.tags;
      if (change) rows.set(path,after); else rows.delete(path);
    }
    current = plan;
    return {...totals};
  };
}
