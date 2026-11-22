import { hasInvalidPreview, setPreviewValidity } from './live-preview.js';
import { hydratePlan } from './plan-codec.js';
// Incremental desktop adapter. The native plan remains authoritative. The
// array and path index are patched in place so scalar acknowledgements never
// copy the remainder of a large stage or its artwork.
export function createLiveStageTransport(invoke, capabilities = {}) {
  let session = null, snapshot = null;
  const positions = new Map();
  const invalidFields = new Set();
  const columnCounts = new Map();
  const derivedFields = new Map();
  const fieldKey = (path, field) => `${path}\0${field}`;
  function index(value) {
    snapshot = {...value, session, plan:hydratePlan(value.plan)};
    positions.clear(); derivedFields.clear();
    for(const entry of value.derived_fields || []) derivedFields.set(fieldKey(entry.path,entry.field),entry);
    invalidFields.clear(); columnCounts.clear();
    for (const change of snapshot.plan?.changes || []) {
      for (const tag of change.tag_changes || []) {
        columnCounts.set(tag.field,(columnCounts.get(tag.field) || 0) + 1);
        if (tag.invalid) invalidFields.add(fieldKey(change.path,tag.field));
      }
      if (change.invalid) invalidFields.add(fieldKey(change.path,'@structure'));
      if (change.cover_change && hasInvalidPreview({changes:[{cover_change:change.cover_change}]})) invalidFields.add(fieldKey(change.path,'artwork'));
      for (const block of change.block_changes || []) if (hasInvalidPreview({changes:[{block_changes:[block]}]})) invalidFields.add(fieldKey(change.path,`block:${block.kind}`));
    }
    setPreviewValidity(snapshot.plan,invalidFields.size > 0);
    snapshot.plan?.changes.forEach((change, at) => positions.set(change.path, at));
    return snapshot;
  }
  async function accept(delta) {
    if (snapshot && delta.session === session && delta.revision < snapshot.revision) return snapshot;
    if (delta.resync || !snapshot || delta.session !== session || (delta.from != null && delta.from > snapshot.revision) || delta.changes?.some(entry => entry.tag?.truncated)) {
      session = delta.session;
      return index(delta.snapshot || await invoke('mcp_shared_stage', {}));
    }
    if (delta.changes?.some(entry => entry.block || (entry.artwork && !Array.isArray(entry.artwork.old)))) return index(await invoke('mcp_shared_stage', {}));
    const assets = new Map(), artwork = new Map();
    const image = async cover => {
      if (!assets.has(cover.asset)) assets.set(cover.asset, invoke('mcp_shared_stage_asset', {asset:cover.asset}));
      const asset = await assets.get(cover.asset);
      return {mime:cover.mime,kind:cover.kind,description:cover.description,data_base64:asset.data_base64,...(cover.invalid ? {invalid:true} : {})};
    };
    // Hydrate before patching the cached plan, so expiry/error cannot expose a
    // partly adopted revision. The same reference is fetched once per delta.
    try {
      for (const entry of delta.changes || []) if (entry.artwork) {
        const old = await Promise.all((entry.artwork.old || []).map(image));
        const next = await Promise.all(entry.artwork.covers.map(image));
        artwork.set(entry,{old,new:next,...(entry.artwork.invalid ? {invalid:true} : {})});
      }
    } catch (error) {
      if (/asset_expired|artwork.*expired/i.test(String(error)) || error?.failure?.code === 'asset_expired') return index(await invoke('mcp_shared_stage', {}));
      throw error;
    }
    let columnsChanged = false;
    const plan = snapshot.plan || {description:'Edit tags',changes:[],notes:[],locked_skipped:[]};
    function row(path) {
      if (!positions.has(path)) { positions.set(path,plan.changes.length); plan.changes.push({path,tag_changes:[],rename_to:null,cover_change:null}); }
      return plan.changes[positions.get(path)];
    }
    function remove(entry) {
      const at = positions.get(entry.path); if (at === undefined) return;
      const change = plan.changes[at];
      if (entry.field === 'filename') {
        change.rename_to = null; change.copy = false; change.sidecar_renames = [];
        change.rename_root = null; change.creates_folder = false; change.folder_extras = [];
      }
      else if (entry.field === 'artwork') change.cover_change = null;
      else if (entry.field.startsWith('block:')) change.block_changes = (change.block_changes || []).filter(block=>block.kind !== entry.field.slice(6));
      else {
        if (change.tag_changes.some(tag => tag.field === entry.field)) {
          const count = (columnCounts.get(entry.field) || 1) - 1;
          columnCounts.set(entry.field,count); if (!count) columnsChanged = true;
        }
        change.tag_changes = change.tag_changes.filter(tag => tag.field !== entry.field);
      }
    }
    for (const entry of delta.removed || []) { invalidFields.delete(fieldKey(entry.path,entry.field)); remove(entry); }
    for (const entry of delta.changes || []) {
      const change = row(entry.path);
      if (entry.tag) {
        const key = fieldKey(entry.path,entry.field);
        if (entry.tag.invalid) invalidFields.add(key); else invalidFields.delete(key);
        const tag = {field:entry.field, ...entry.tag};
        const at = change.tag_changes.findIndex(tag => tag.field === entry.field);
        if (at < 0) {
          if (!columnCounts.get(entry.field)) columnsChanged = true;
          columnCounts.set(entry.field,(columnCounts.get(entry.field) || 0) + 1);
          change.tag_changes.push(tag);
        } else change.tag_changes[at] = tag;
      } else if (entry.rename) {
        change.rename_to = entry.rename.to; change.copy = entry.rename.copy;
        change.sidecar_renames = entry.rename.sidecars; change.creates_folder = entry.rename.creates_folder;
        change.rename_root = entry.rename.rename_root; change.folder_extras = entry.rename.folder_extras;
      }
      else if (entry.artwork) {
        change.cover_change = artwork.get(entry);
        const key=fieldKey(entry.path,'artwork');
        if (hasInvalidPreview({changes:[{cover_change:change.cover_change}]})) invalidFields.add(key); else invalidFields.delete(key);
      }
    }
    for (const path of delta.removed_paths || []) {
      const at = positions.get(path); if (at === undefined) continue;
      const last = plan.changes.pop(); positions.delete(path);
      if (at < plan.changes.length) { plan.changes[at] = last; positions.set(last.path,at); }
    }
    setPreviewValidity(plan,invalidFields.size > 0);
    const derivedChanges=[...(delta.changes || []),...(delta.removed || []).map(entry=>({...entry,derived:false}))].map(({path,field,derived})=>({path,field,derived}));
    for(const entry of derivedChanges){const key=fieldKey(entry.path,entry.field);if(entry.derived)derivedFields.set(key,{path:entry.path,field:entry.field});else if(entry.derived===false)derivedFields.delete(key);}
    snapshot = {...snapshot, columnsChanged, revision:delta.revision, human_revision:delta.human_revision, plan:plan.changes.length ? plan : null,
      changeAt: path => { const at = positions.get(path); return at === undefined ? undefined : plan.changes[at]; },
      changedKeys:[...(delta.changes || []), ...(delta.removed || [])].map(({path,field})=>({path,field})),
      derivedChanges, readDerivedFields: () => [...derivedFields.values()], nativeDelta:true};
    return snapshot;
  }
  return {
    read: async () => accept(await invoke('mcp_shared_stage_delta', {since:snapshot?.revision ?? null, session})),
    reconcile: value => index(value),
    publish: async (entries, expectedRevision, options = {}) => {
      let value, columnsChanged = false, fullAdoption = false;
      const changedKeys = new Map(), derivedChanges = new Map();
      if (options.recompute && !capabilities.recompute) throw new Error('Native transform recomputation is unavailable');
      const deferredGroups = entries.length > 512 && (options.groups?.length || options.recompute) && capabilities.recompute;
      const requests = [];
      for (let offset=0; offset<entries.length; offset+=512) requests.push({intents:entries.slice(offset,offset+512),...(deferredGroups ? {} : options)});
      if ((!entries.length && options.recompute) || deferredGroups) requests.push({...options,recompute:true,intents:[]});
      for (const request of requests) {
        const result = await invoke('mcp_publish_shared_intents', {...request,expectedRevision,...(capabilities.session_check ? {session} : {})});
        value = await accept(result.delta);
        fullAdoption ||= !value.nativeDelta;
        if (!result.accepted) throw new Error('stage_conflict: newer human revision');
        expectedRevision = value.revision;
        columnsChanged ||= value.columnsChanged;
        for (const key of value.changedKeys || []) changedKeys.set(`${key.path}\0${key.field}`,key);
        for (const key of value.derivedChanges || []) derivedChanges.set(fieldKey(key.path,key.field),key);
      }
      if (value?.nativeDelta && entries.length>512) value = {...value, nativeDelta:!fullAdoption, columnsChanged, changedKeys:[...changedKeys.values()],derivedChanges:[...derivedChanges.values()],...(fullAdoption?{derived_fields:[...derivedFields.values()]}:{})};
      return value;
    },
  };
}
