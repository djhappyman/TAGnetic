// The native service owns the stage. This cache only paints local typing while
// its revision-checked publication is in flight; it never authorizes a write.
const MAX_OPTIMISTIC_ROWS = 2048;
const hasChanges = change => change.tag_changes?.length || change.rename_to || change.cover_change || change.block_changes?.length || change.sidecar_renames?.length;
const key = (path, field) => `${path}\0${field}`;

function withoutFields(change, fields) {
  if (!fields?.size) return change;
  const result = { ...change, tag_changes: (change.tag_changes || []).filter(tag => !fields.has(tag.field)) };
  if (fields.has('artwork')) result.cover_change = null;
  if (fields.has('filename')) { result.rename_to = null; result.sidecar_renames = []; result.copy = false; }
  if (change.block_changes?.length) result.block_changes = change.block_changes.filter(block => !fields.has(`block:${block.kind}`));
  return result;
}

export function stagedFields(plan) {
  const fields = [];
  for (const change of plan?.changes || []) {
    for (const tag of change.tag_changes || []) fields.push({ path: change.path, field: tag.field });
    if (change.cover_change) fields.push({ path: change.path, field: 'artwork' });
    if (change.rename_to || change.sidecar_renames?.length) fields.push({ path: change.path, field: 'filename' });
    for (const block of change.block_changes || []) fields.push({ path: change.path, field: `block:${block.kind}` });
  }
  return fields;
}

// Automatic previews follow the current selection, but staging does not.
// Leaving one file for another never withdraws the first file's desired values.
export function automaticStageScope(previous, nextPlan, paths) {
  const scope = new Set(paths);
  const next = stagedFields(nextPlan);
  return {
    touched: [...previous.filter(entry => scope.has(entry.path)), ...next],
    fields: [...previous.filter(entry => !scope.has(entry.path)), ...next],
  };
}

// Native publication patches fields, so the current whole stage is the paint
// baseline. Image strings/unchanged rows retain identity; no image serialization.
export function stageRemainder(snapshot) {
  return snapshot?.plan || null;
}

export function optimisticSharedPlan(remainder, humanPlan, touched = []) {
  if (!remainder) return humanPlan;
  // Retain the canonical large stage until its native patch arrives. A local
  // raw input already owns its editor DOM; rebuilding 100k rows to show that
  // one keystroke would be slower and would briefly hide other staged files.
  if (remainder.changes.length > MAX_OPTIMISTIC_ROWS) return remainder;
  if ((humanPlan?.changes.length || 0) > MAX_OPTIMISTIC_ROWS) return humanPlan;
  const remove = new Map();
  for (const entry of [...stagedFields(humanPlan), ...touched]) {
    if (!remove.has(entry.path)) remove.set(entry.path, new Set());
    remove.get(entry.path).add(entry.field);
  }
  const changes = new Map(remainder.changes.map(change => [change.path, withoutFields(change, remove.get(change.path))]));
  for (const change of humanPlan?.changes || []) {
    const prior = changes.get(change.path);
    changes.set(change.path, !prior ? change : {
      ...prior, ...change,
      tag_changes: [...(prior.tag_changes || []), ...(change.tag_changes || [])],
      cover_change: change.cover_change || prior.cover_change,
      rename_to: change.rename_to || prior.rename_to,
      copy: change.rename_to ? change.copy : prior.copy,
      sidecar_renames: change.sidecar_renames?.length ? change.sidecar_renames : prior.sidecar_renames,
      block_changes: [...(prior.block_changes || []), ...(change.block_changes || [])],
    });
  }
  const list = [...changes.values()].filter(hasChanges);
  return list.length ? { ...(humanPlan || remainder), changes: list } : null;
}

export function createSharedStageSync({ read, publish, clear, context, adopt, intents, error = () => {} }) {
  let snapshot = null, remainder = null, pending = null, running = null, refresh = false, clearRequested = false;
  let generation = 0, discardGeneration = 0, started = false, lastFailure = null, publishing = false;
  let adoptedRevision = -1, adoptedGeneration = -1, adoptedSession = null;
  const touches = new Map();
  const capture = () => ({ ...context(), generation, discardGeneration });
  const current = before => {
    const after = context();
    return before.generation === generation && before.inputRevision === after.inputRevision
      && before.library === after.library && !after.busy;
  };
  const accept = (value, before) => {
    if (snapshot && value.revision < snapshot.revision && (!value.session || value.session === snapshot.session)) return;
    snapshot = value; remainder = stageRemainder(value);
    if (!pending && !clearRequested && current(before)
      && (adoptedRevision !== value.revision || adoptedGeneration !== generation || adoptedSession !== value.session)) {
      if (adopt(value) !== false) { adoptedRevision = value.revision; adoptedGeneration = generation; adoptedSession = value.session; }
    }
  };
  const hasPlan = value => value && Object.hasOwn(value,'plan');
  const conflict = failure => /stage_conflict|staging.*changed|stage.*changed|stage revision.*different session/i.test(String(failure));
  function restore(work) {
    if (work.clear) { clearRequested=true; return; }
    if (work.intents) {
      const replaced = new Set(hasPlan(pending) ? [...stagedFields(pending.plan), ...(pending.touched || [])].map(entry=>key(entry.path,entry.field)) : []);
      const merged = new Map([...work.intents.filter(entry=>!replaced.has(key(entry.path,entry.field))), ...(pending?.intents || [])].map(entry=>[key(entry.path,entry.field),entry]));
      pending = {...work,...pending};
      if (merged.size || pending.options?.recompute) pending.intents=[...merged.values()]; else delete pending.intents;
    } else if (!hasPlan(pending)) pending = {...work,...pending};
  }
  async function pump() {
    if (!started) return;
    while (pending || clearRequested || refresh) {
      if (!snapshot) { const before = capture(); accept(await read(), before); }
      if (pending || clearRequested) {
        let work;
        if (clearRequested) { work={clear:true};clearRequested=false; }
        else if (hasPlan(pending) && pending.intents && (pending.intents.length || pending.options?.recompute)) {
          work={plan:pending.plan,touched:pending.touched};
          pending={intents:pending.intents,options:pending.options};
        } else { work=pending;pending=null; }
        const before = capture();
        try {
          publishing = true;
          const value = work.clear ? await clear(snapshot.revision)
            : work.intents ? await intents(work.intents, snapshot.revision, work.options) : await publish(work.plan, snapshot.revision, [...new Map([...touches.values(), ...(work.touched || [])].map(entry => [key(entry.path, entry.field), entry])).values()]);
          if (value?.accepted === false) throw new Error('stage_conflict: Staging changed before publication');
          publishing = false;
          accept(value, before);
        } catch (failure) {
          publishing = false;
          // Discard is a barrier, including for requests whose failure arrives
          // late. Retrying their old intent after the clear would revive it.
          if (before.discardGeneration !== discardGeneration) continue;
          if (before.library !== context().library) { refresh = true; continue; }
          if (/stage_session_mismatch/i.test(String(failure))) {
            const atRead=capture();accept(await read(),atRead);
            throw failure; // Never replay an old library's intents into a new session.
          }
          if (!conflict(failure)) {
            restore(work);
            throw failure;
          }
          // A newer native contribution stays intact. Retry the latest local
          // human layer, never a stale merged snapshot from the browser.
          restore(work);
          const atRead = capture(); accept(await read(), atRead);
        }
      } else {
        refresh = false;
        const before = capture(); accept(await read(), before);
      }
    }
  }
  function schedule() {
    if (!started) return Promise.resolve();
    if (!running) {
      let failed = false;
      running = Promise.resolve().then(pump).catch(failure => { failed = true; lastFailure = failure; error(failure); }).finally(() => {
        running = null;
        if (!failed && (pending || clearRequested || refresh)) schedule();
      });
    }
    return running;
  }
  return {
    start(plan = undefined) {
      if (started) return schedule();
      started = true; refresh = true;
      if (plan !== undefined && plan !== null) pending = { plan };
      return schedule();
    },
    changed(plan, touched = []) {
      // An automatic preview can settle to no local edits after selection or
      // Inspector hydration. It has no patch to publish; feeding it back into
      // the native stage only creates another hydration/preview cycle.
      if (!pending && !clearRequested && !plan?.changes?.length && !touched.length && !touches.size) return running || Promise.resolve();
      lastFailure = null;
      generation++;
      const replaced=new Set([...stagedFields(plan),...touched,...touches.values()].map(entry=>key(entry.path,entry.field)));
      const remaining=pending?.intents?.filter(entry=>!replaced.has(key(entry.path,entry.field)));
      const withdrawals=[...new Map([...(pending?.touched || []),...touched].map(entry=>[key(entry.path,entry.field),entry])).values()];
      pending = { plan, touched:withdrawals, ...(remaining?.length ? {intents:remaining,options:pending.options} : {}) }; return schedule();
    },
    intents(entries, options = {}) {
      if (!entries.length && !options.recompute) return running || Promise.resolve();
      lastFailure = null; generation++;
      const merged = new Map((pending?.intents || []).map(entry => [key(entry.path,entry.field), entry]));
      for (const entry of entries) merged.set(key(entry.path,entry.field), entry);
      const prior = pending?.options;
      if (prior?.recompute || options.recompute) options = {...prior,...options,recompute:true,paths:[...new Set([...(prior?.paths || []),...(options.paths || []),...entries.map(entry=>entry.path)])]};
      pending = {...pending,intents:[...merged.values()], options}; return schedule();
    },
    touched(entries) {
      touches.clear();
      for (const entry of entries) touches.set(key(entry.path, entry.field), entry);
    },
    clear() { lastFailure = null; generation++; discardGeneration++; touches.clear(); pending = null; clearRequested = true; return schedule(); },
    completed(plan) { for (const entry of stagedFields(plan)) touches.delete(key(entry.path, entry.field)); },
    refresh() { lastFailure = null; refresh = true; return schedule(); },
    async flush() { while (running) await running; if (lastFailure) throw lastFailure; },
    pending: () => publishing || !!pending || clearRequested || !!lastFailure,
    revision: () => snapshot?.revision,
    humanPlan: () => pending ? pending.plan : (snapshot?.human_plan ?? null),
    snapshot: () => snapshot,
    decorate(plan, touched = []) { return optimisticSharedPlan(remainder, plan, [...touches.values(), ...touched]); },
  };
}
