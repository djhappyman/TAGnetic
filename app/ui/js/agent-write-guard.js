// Capture desired values without copying or serializing image payloads. Saved
// baselines may change after an agent write; the person's intentions may not.
const LARGE_PLAN = Symbol('refresh large plans through the native preview');
const MAX_FINGERPRINT_ROWS = 2048;
function covers(images, assets) {
  return (images || []).map(image => [image.mime, image.kind || '', image.description || '',
    image.data_ref == null ? image.data_base64 || '' : assets[image.data_ref]]);
}

export function desiredPlanFingerprint(plan) {
  if (!plan) return null;
  // A library-sized draft must not allocate another forest of arrays on the
  // input thread. Such drafts take the conservative native-refresh path.
  if (plan.changes?.length > MAX_FINGERPRINT_ROWS) return LARGE_PLAN;
  const assets = plan.cover_assets || [];
  return (plan.changes || []).map(change => [change.path, change.rename_to || null, Boolean(change.copy),
    (change.tag_changes || []).map(field => [field.field, field.new ?? null]),
    change.cover_change ? covers(change.cover_change.new, assets) : null,
    (change.block_changes || []).map(block => [block.kind, block.revision || null, Boolean(block.text_to_unicode),
      block.new ? [Object.entries(block.new.tags || {}).sort(([a], [b]) => a.localeCompare(b)), covers(block.new.covers, assets)] : null]),
    (change.sidecar_renames || []).map(pair => [...pair]),
  ]);
}

export function equalDesiredFingerprints(left, right) {
  if (left === right) return true;
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
  for (let index = 0; index < left.length; index++) if (!equalDesiredFingerprints(left[index], right[index])) return false;
  return true;
}

export function canAdoptAgentRebase(publication, { plan, generation, inputRevision, event }) {
  return Boolean(publication && publication.desired !== LARGE_PLAN && event.manual_rebased !== false
    && publication.generation === generation && publication.inputRevision === inputRevision
    && equalDesiredFingerprints(publication.desired, desiredPlanFingerprint(plan)));
}
