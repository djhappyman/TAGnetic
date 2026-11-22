// A native-shaped bridge over disposable browser DTOs. It exercises the same
// desktop event/controller path without reading or writing any real files.
const listeners = new Map();
const state = {
  enabled: true, revision: 0, manual: null, view: {}, recovered: false,
  liveFields: false, shared: false, stage: null, humanRevision: 0, mediaWrites: 0,
  recovery: { recovery_required: false, operations: [] }, calls: [], clients: [],
  activity: { connections: [], work: [], writes: [], staged: null },
};
const rawInputs = new Map(), transformBases = new Map(), derivedFields = new Map();
const copy = value => value === undefined ? undefined : structuredClone(value);
const mock = async (command, args) => (await import('../../../app/ui/js/mock.js')).mockInvoke(command, args);
const sharedSnapshot = () => copy({ revision: state.revision, human_revision: state.humanRevision, accepted: true, plan: state.stage, human_plan: state.manual,derived_fields:[...derivedFields.values()] });
const patch = async (prior, incoming, touched = []) => (await import('../../../app/ui/js/shared-stage.js')).optimisticSharedPlan(prior, incoming, touched);
const fields = async plan => (await import('../../../app/ui/js/shared-stage.js')).stagedFields(plan);
const emit = (kind, data) => {
  for (const listener of listeners.get('tagrex://agent-event') || []) listener({ payload: { kind, data: copy(data) } });
};

window.__TAURI__ = {
  event: { listen: async (name, listener) => {
    if (!listeners.has(name)) listeners.set(name, new Set());
    listeners.get(name).add(listener);
    return () => listeners.get(name).delete(listener);
  } },
  core: { invoke: async (command, args = {}) => {
    state.calls.push(command);
    if (command === 'mcp_agent_status') return {
      enabled: state.enabled,
      can_reconnect: state.clients.some(client => !client.revoked),
      recovery_required: state.recovery.recovery_required,
      ...copy(state.activity),
      connections: state.activity.connections.map(connection => ({ id: connection.principal, state: 'active', controllable: true, ...copy(connection) })),
    };
    if (command === 'mcp_control_agent') {
      const connection = state.activity.connections.find(connection => (connection.id || connection.principal) === args.sessionId);
      if (!connection) throw new Error('The agent session changed');
      connection.state = { pause: 'paused', resume: 'active', disconnect: 'disconnected', reconnect: 'active' }[args.action];
      (state.agentControls ||= []).push(copy(args));
      emit('agent_activity_changed', {});
      return;
    }
    if (command === 'mcp_cancel_operation') { state.lastStoppedOperation = args.operationId; return; }
    if (command === 'mcp_status') return {
      live_fields: state.liveFields ? {version:1,delta:true,intents:true,recompute:!!state.liveRecompute,derived_scope:!!state.liveRecompute,session_check:!!state.liveRecompute,session:'fixture'} : undefined,
      available: true, listening: state.enabled, shared_staging: state.shared, clients: { clients: copy(state.clients) },
      operations: { operations: [] }, analysis: { jobs: [] }, recovery: copy(state.recovery),
    };
    if (command === 'mcp_set_manual_plan') { state.manual = copy(args.plan); state.stage = copy(args.plan); return ++state.revision; }
    if (command === 'mcp_shared_stage_delta') return {session:'fixture',revision:state.revision,resync:true,snapshot:sharedSnapshot()};
    if (command === 'mcp_publish_shared_intents') {
      (state.intentRequests ??= []).push(copy(args));
      if(state.liveRecompute && args.session!=='fixture')throw Error('stage_session_mismatch');
      const scope=new Set(args.paths || args.intents.map(entry=>entry.path));
      const seeded=state.liveRecompute && (args.recompute || args.groups?.length);
      const values=new Map();
      const prior=[];
      if(seeded) {
        for(const change of state.stage?.changes || [])if(scope.has(change.path))for(const tag of change.tag_changes || []) {
          const key=change.path+'\0'+tag.field;
          values.set(key,{path:change.path,field:tag.field,value:tag.new});prior.push({path:change.path,field:tag.field});
        }
        for(const entries of [transformBases,rawInputs])for(const [key,entry]of entries)if(scope.has(entry.path))values.set(key,entry);
      }
      for(const entry of args.intents){const key=entry.path+'\0'+entry.field;rawInputs.set(key,copy(entry));values.set(key,entry);}
      const plan = await mock('preview_live_edits', {edits:seeded?[...values.values()]:args.intents,groups:args.groups || [],paths:args.paths || []});
      if(seeded && args.groups?.length)for(const change of plan.changes)for(const tag of change.tag_changes){const key=change.path+'\0'+tag.field;transformBases.set(key,copy(values.get(key)||{path:change.path,field:tag.field,value:tag.old}));}
      const touched=seeded?[...prior,...await fields(plan),...args.intents]:args.intents;
      state.manual = await patch(state.manual, plan, touched);
      state.stage = await patch(state.stage, plan, touched);
      state.humanRevision = ++state.revision;
      for(const entry of touched)derivedFields.delete(entry.path+'\0'+entry.field);
      if(seeded && args.groups?.length)for(const change of plan.changes)for(const tag of change.tag_changes){const key=change.path+'\0'+tag.field;if((values.get(key)?.value ?? tag.old)!==tag.new)derivedFields.set(key,{path:change.path,field:tag.field});}
      const delta = {session:'fixture',revision:state.revision,human_revision:state.humanRevision,changes:plan.changes.flatMap(change=>change.tag_changes.map(tag=>({path:change.path,field:tag.field,tag,derived:derivedFields.has(change.path+'\0'+tag.field)}))),removed:touched.filter(entry=>!plan.changes.some(change=>change.path===entry.path&&change.tag_changes.some(tag=>tag.field===entry.field))),removed_paths:[...new Set(touched.map(entry=>entry.path))].filter(path=>!state.stage?.changes.some(change=>change.path===path))};
      return {accepted:true,revision:state.revision,delta};
    }
    if (command === 'mcp_shared_stage') {
      const result = sharedSnapshot();
      if (state.holdNextStageRead) {
        state.holdNextStageRead = false;
        await new Promise(resolve => state.releaseStageRead = resolve);
      }
      return result;
    }
    if (command === 'mcp_publish_shared_stage') {
      if (args.expectedRevision !== state.revision) throw new Error('stage_conflict: Staging changed');
      for(const change of args.plan?.changes || [])for(const tag of change.tag_changes || []) {
        const previous=state.stage?.changes.find(row=>row.path===change.path)?.tag_changes.find(field=>field.field===tag.field);
        if(previous?.new!==tag.new){rawInputs.delete(change.path+'\0'+tag.field);transformBases.delete(change.path+'\0'+tag.field);derivedFields.delete(change.path+'\0'+tag.field);}
      }
      state.manual = await patch(state.manual, copy(args.plan), args.touched);
      state.stage = await patch(state.stage, copy(args.plan), args.touched);
      state.humanRevision++; state.revision++;
      emit('shared_stage_changed', { revision: state.revision });
      return sharedSnapshot();
    }
    if (command === 'mcp_clear_shared_stage') {
      if (args.expectedRevision !== state.revision) throw new Error('stage_conflict: Staging changed');
      state.manual = state.stage = null; state.humanRevision++; state.revision++;
      rawInputs.clear();transformBases.clear();derivedFields.clear();
      emit('shared_stage_changed', { revision: state.revision });
      return sharedSnapshot();
    }
    if (command === 'apply_plan' && state.shared) {
      const selected = new Set(args.plan.changes.map(change => change.path));
      const current = { ...(state.stage || args.plan), changes: (state.stage?.changes || []).filter(change => selected.has(change.path)) };
      const result = await mock('apply_plan', { plan: current });
      state.mediaWrites++;
      const touched = await fields(current);
      state.stage = await patch(state.stage, null, touched);
      state.manual = await patch(state.manual, null, touched);
      state.revision++;
      emit('shared_stage_changed', { revision: state.revision });
      return result;
    }
    if (command === 'mcp_set_view_preferences') { state.view = copy(args.preferences); return; }
    if (command === 'mcp_set_enabled') { state.enabled = args.enabled; if (!state.enabled) state.activity.connections = []; emit('service_changed', {}); return; }
    if (command === 'save_settings' && state.failNextSettingsSave) {
      state.failNextSettingsSave = false;
      const fresh = await mock('load_settings', {});
      fresh.mask_presets = [...(fresh.mask_presets || []).filter(item => item.name !== 'Agent mask'), { name: 'Agent mask', mask: '%album%/%filename%' }];
      await mock('save_settings', { settings: fresh });
      throw new Error('Settings changed elsewhere. Read them again before saving.');
    }
    if (command === 'mcp_pair_client') {
      const id = state.clients.length ? `client_fixture_${state.clients.length + 1}` : 'client_fixture';
      state.clients.push({ id, label: args.label, grant: copy(args.grant), revoked: false });
      return { principal_id: id, executable: '/fixture/Tagnetic', profile: '/fixture/profile', credential_file: `/fixture/profile/mcp/credentials/${id}.token` };
    }
    if (command === 'mcp_revoke_client') { state.clients.find(client => client.id === args.principalId).revoked = true; return; }
    if (command === 'mcp_patch_query_presets') return { ...(await mock('load_settings', {})), query_presets: args.upsert };
    if (command === 'mcp_rebase_manual_plan') {
      const plan = copy(args.plan);
      for (const change of plan.changes) if (args.paths.includes(change.path)) {
        const track = await mock('read_track', { path: change.path });
        for (const field of change.tag_changes || []) field.old = track.tags[field.field] || null;
      }
      return plan;
    }
    if (command === 'mcp_reconcile_operation') {
      if (args.proof !== 'verified-fixture-proof' || args.action !== 'accept_completed') {
        state.recovery.operations[0].proof = 'verified-fixture-proof';
        throw new Error('The recovery evidence changed. Review the refreshed scope.');
      }
      state.recovered = true; state.recovery = { recovery_required: false, operations: [] }; return { reconciled: true };
    }
    return copy(await mock(command, args));
  } },
};

let mutationQueue = Promise.resolve();
function mutate(work) {
  const next = mutationQueue.then(work);
  mutationQueue = next.catch(() => {});
  return next;
}
const nativeInvoke = window.__TAURI__.core.invoke;
const mutations = new Set(['mcp_publish_shared_stage','mcp_publish_shared_intents','mcp_clear_shared_stage','mcp_set_manual_plan','apply_plan']);
window.__TAURI__.core.invoke = (command, args) => mutations.has(command) ? mutate(() => nativeInvoke(command,args)) : nativeInvoke(command,args);

window.mcpFixture = {
  state,
  emit,
  async stageAgent(path, values, cover = null) {
    const track = await mock('read_track', { path });
    const next = { description: 'Live agent changes', changes: [{ path, tag_changes: Object.entries(values).map(([field, value]) => ({ field, old: track.tags[field] || null, new: value })), cover_change: cover, rename_to: null }] };
    state.stage = await patch(state.stage, next); state.revision++;
    emit('shared_stage_changed', { revision: state.revision });
  },
  async discardAgent(path, keys) {
    state.stage = await patch(state.stage, null, keys.map(field => ({ path, field })));
    state.revision++; emit('shared_stage_changed', { revision: state.revision });
  },
  async commitAgent(paths) {
    const selected = new Set(paths);
    const current = { ...state.stage, changes: state.stage.changes.filter(change => selected.has(change.path)) };
    await mock('apply_plan', { plan: current }); state.mediaWrites++;
    const touched = await fields(current);
    state.stage = await patch(state.stage, null, touched); state.manual = await patch(state.manual, null, touched);
    state.revision++;
    const tracks = await Promise.all(paths.map(path => mock('read_track', { path })));
    emit('shared_stage_changed', { revision: state.revision });
    emit('write_completed', { tracks });
  },
  async holdAgentWrite(path, title) {
    const revision = state.revision;
    const manual = copy(state.manual);
    const track = await mock('read_track', { path });
    await mock('apply_plan', { plan: { description: 'Agent fixture write', changes: [{ path, tag_changes: [{ field: 'title', old: track.tags.title, new: title }], rename_to: null }] } });
    const saved = await mock('read_track', { path });
    if (manual) for (const change of manual.changes) if (change.path === path) for (const field of change.tag_changes || []) field.old = saved.tags[field.field] || null;
    state.complete = () => emit('write_completed', { tracks: [saved], plan: manual, manual_revision: revision, manual_rebased: state.revision === revision });
    return revision;
  },
};

for (const name of ['stageAgent','discardAgent','commitAgent']) {
  const operation = window.mcpFixture[name];
  window.mcpFixture[name] = (...args) => mutate(() => operation(...args));
}

window.finishMcpBridge();
