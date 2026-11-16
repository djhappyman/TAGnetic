// A native-shaped bridge over disposable browser DTOs. It exercises the same
// desktop event/controller path without reading or writing any real files.
const listeners = new Map();
const state = {
  enabled: true, revision: 0, manual: null, view: {}, recovered: false,
  shared: false, stage: null, humanRevision: 0, mediaWrites: 0,
  recovery: { recovery_required: false, operations: [] }, calls: [], clients: [],
};
const copy = value => value === undefined ? undefined : structuredClone(value);
const mock = async (command, args) => (await import('../../../app/ui/js/mock.js')).mockInvoke(command, args);
const sharedSnapshot = () => copy({ revision: state.revision, human_revision: state.humanRevision, accepted: true, plan: state.stage, human_plan: state.manual });
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
    if (command === 'mcp_status') return {
      available: true, listening: state.enabled, shared_staging: state.shared, clients: { clients: copy(state.clients) },
      operations: { operations: [] }, analysis: { jobs: [] }, recovery: copy(state.recovery),
    };
    if (command === 'mcp_set_manual_plan') { state.manual = copy(args.plan); state.stage = copy(args.plan); return ++state.revision; }
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
      state.manual = await patch(state.manual, copy(args.plan), args.touched);
      state.stage = await patch(state.stage, copy(args.plan), args.touched);
      state.humanRevision++; state.revision++;
      emit('shared_stage_changed', { revision: state.revision });
      return sharedSnapshot();
    }
    if (command === 'mcp_clear_shared_stage') {
      if (args.expectedRevision !== state.revision) throw new Error('stage_conflict: Staging changed');
      state.manual = state.stage = null; state.humanRevision++; state.revision++;
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
    if (command === 'mcp_set_enabled') { state.enabled = args.enabled; emit('service_changed', {}); return; }
    if (command === 'save_settings' && state.failNextSettingsSave) {
      state.failNextSettingsSave = false;
      const fresh = await mock('load_settings', {});
      fresh.mask_presets = [...(fresh.mask_presets || []).filter(item => item.name !== 'Agent mask'), { name: 'Agent mask', mask: '%album%/%filename%' }];
      await mock('save_settings', { settings: fresh });
      throw new Error('Settings changed elsewhere. Read them again before saving.');
    }
    if (command === 'mcp_pair_client') {
      state.clients.push({ id: 'client_fixture', label: args.label, grant: copy(args.grant), revoked: false });
      return { principal_id: 'client_fixture', executable: '/fixture/Tagnetic', profile: '/fixture/profile', credential_file: '/fixture/profile/mcp/credentials/client_fixture.token' };
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

window.finishMcpBridge();
