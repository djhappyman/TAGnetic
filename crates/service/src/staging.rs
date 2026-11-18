//! The single live staging area shared by the desktop and all paired agents.
//! Handles name scopes in this area, never competing copies of desired values.
//!
//! The stage also projects bounded, revisioned deltas (changed fields, removals
//! and counts) for the desktop and accepts narrow human field intents. Both are
//! views over this same claim ledger: no competing plan, index of edits or
//! second write authority is introduced. See
//! `docs/plans/native-live-fields-contract.md`.
use std::collections::VecDeque;
use std::time::Instant;

use super::*;
use tagrex_commands::{ActionGroupDto, CoverArtDto, FileChangeDto, LockedSkipDto, TagEditDto};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StageFieldRef {
    pub path: String,
    pub field: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedStageSnapshot {
    pub revision: u64,
    pub human_revision: u64,
    pub accepted: bool,
    pub plan: Option<PlanDto>,
    pub human_plan: Option<PlanDto>,
    #[serde(default)]
    pub derived_fields: Vec<StageFieldRef>,
}

/// One human field intent: set `field` on `path` to `value`. `None` or an empty
/// string clears the field. A same-value intent still claims the field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StageIntent {
    pub path: String,
    pub field: String,
    pub value: Option<String>,
}
/// A complete trusted-desktop intent request. `recompute` allows an empty
/// intent list: it re-runs `groups` over `scope` from the current raw inputs.
#[derive(Debug, Clone)]
pub struct StageIntentRequest {
    pub intents: Vec<StageIntent>,
    pub expected_revision: u64,
    pub groups: Vec<ActionGroupDto>,
    pub scope: Vec<String>,
    pub recompute: bool,
    pub session: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StageTagDelta {
    pub old: Option<String>,
    pub new: Option<String>,
    pub invalid: bool,
    pub old_chars: usize,
    pub new_chars: usize,
    pub truncated: bool,
}
/// One staged image, named by its stable content-addressed asset reference. The
/// bytes are fetched only through `mcp_shared_stage_asset`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageArtworkCover {
    pub asset: String,
    pub mime: String,
    pub kind: String,
    pub description: String,
    pub bytes: usize,
    /// The payload cannot be embedded within the native image budget or is not
    /// an image MIME. Present so a consumer can mark the exact bad image.
    pub invalid: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageArtworkDelta {
    pub empty: bool,
    /// Any proposed cover failed the native artwork validity check.
    pub invalid: bool,
    /// Saved baseline covers, as stable references. `covers` remains the new
    /// (proposed) set for compatibility with the first delta shape.
    pub old: Vec<StageArtworkCover>,
    pub covers: Vec<StageArtworkCover>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageRenameDelta {
    pub to: String,
    pub copy: bool,
    pub creates_folder: bool,
    pub rename_root: Option<String>,
    pub sidecars: Vec<(String, String)>,
    pub folder_extras: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageBlockDelta {
    pub kind: String,
    pub revision: Option<String>,
    pub text_to_unicode: bool,
    pub chars: usize,
    pub truncated: bool,
    pub invalid: bool,
    pub text: Option<String>,
    pub covers: Vec<StageArtworkCover>,
}
/// One currently staged field that changed after the caller's revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageFieldDelta {
    pub path: String,
    pub field: String,
    /// Whether a human touch currently owns/protects this field. Agent-only
    /// fields are false; the value itself is always the merged stage truth.
    pub human: bool,
    #[serde(default)]
    pub derived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<StageTagDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork: Option<StageArtworkDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename: Option<StageRenameDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<StageBlockDelta>,
}
/// A bounded projection of the shared stage between two revisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageDelta {
    pub session: String,
    pub revision: u64,
    pub human_revision: u64,
    pub from: u64,
    pub resync: bool,
    pub resync_reason: Option<String>,
    /// Whole-stage totals.
    pub files: usize,
    pub staged_fields: usize,
    /// This delta only.
    pub changed_files: usize,
    pub changed_fields: usize,
    pub removed_fields: usize,
    pub changes: Vec<StageFieldDelta>,
    pub removed: Vec<StageFieldRef>,
    pub removed_paths: Vec<String>,
    /// Present only on a resync the native side could inline within its bounds.
    pub snapshot: Option<SharedStageSnapshot>,
    /// A resync was required but too large to inline: call `shared_stage`.
    pub snapshot_required: bool,
    pub elapsed_ms: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageIntentOutcome {
    pub accepted: bool,
    pub revision: u64,
    pub human_revision: u64,
    pub session: String,
    pub files: usize,
    pub staged_fields: usize,
    pub locked: Vec<LockedSkipDto>,
    pub delta: StageDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct StageKey {
    pub path: Arc<str>,
    pub field: String,
}
impl StageKey {
    pub fn tag(path: &str, field: &str) -> Self {
        Self {
            path: path.into(),
            field: format!(
                "tag:{}",
                tagrex_core::model::TagField::from_storage_key(field).to_storage_key()
            ),
        }
    }
    pub fn special(path: &str, field: &str) -> Self {
        Self {
            path: path.into(),
            field: field.into(),
        }
    }
    fn public(&self) -> StageFieldRef {
        StageFieldRef {
            path: self.path.to_string(),
            field: self
                .field
                .strip_prefix("tag:")
                .unwrap_or(&self.field)
                .into(),
        }
    }
    fn from_public(value: &StageFieldRef) -> Self {
        if value.field == "artwork"
            || value.field == "filename"
            || value.field.starts_with("block:")
        {
            Self::special(&value.path, &value.field)
        } else {
            Self::tag(&value.path, &value.field)
        }
    }
}
#[derive(Clone)]
struct Claim {
    fingerprint: String,
    owners: BTreeSet<String>,
    /// A transformed value must never become its own fallback input after
    /// bounded basis eviction. This bit belongs to the existing field ledger.
    derived: bool,
}
/// One committed revision boundary, so a delta can be projected without walking
/// the whole stage. Bounded both by count and by recorded fields.
struct StageHistory {
    revision: u64,
    keys: Vec<StageKey>,
    /// A rebase moved file identities; an incremental walk over old paths could
    /// resurrect them, so a delta crossing this boundary must resync.
    structural: bool,
}
#[derive(Clone)]
struct StageAssetEntry {
    data: Arc<str>,
    mime: String,
    kind: String,
    description: String,
    bytes: usize,
    revision: u64,
    seq: u64,
}
/// One bounded raw-input or derived-basis record. Raw inputs are what the person
/// typed; a derived basis is the value a transform ran on. Neither is a plan or
/// a second write authority: both only let a later recompute start from the same
/// input instead of the previous transform output, so non-idempotent transforms
/// cannot compound.
#[derive(Clone)]
pub(super) struct RecordedInput {
    value: Option<String>,
    seq: u64,
}
#[derive(Default)]
pub(super) struct StageState {
    claims: BTreeMap<StageKey, Claim>,
    human_protected: BTreeMap<StageKey, u64>,
    human_plan: Option<PlanDto>,
    human_updated_at: u64,
    /// Opaque per-process stage identity. A delta from another session resyncs.
    pub(super) session: String,
    history: VecDeque<StageHistory>,
    history_fields: usize,
    history_overflowed: bool,
    /// Derived caches of the two plans' path positions, never value authority.
    manual_index: Option<HashMap<u64, usize>>,
    human_index: Option<HashMap<u64, usize>>,
    /// Conservative serialized-size estimate of the combined plan.
    bytes: usize,
    /// Bounded raw human inputs, keyed by tag field. Used only to seed a
    /// transform recompute; never read as staged truth.
    pub(super) raw_inputs: BTreeMap<StageKey, RecordedInput>,
    /// Bounded pre-transform bases for fields a grouped publication produced.
    pub(super) derived_inputs: BTreeMap<StageKey, RecordedInput>,
    input_seq: u64,
    pub(super) input_bytes: usize,
    assets: HashMap<String, StageAssetEntry>,
    asset_bytes: usize,
    asset_seq: u64,
}
impl StageState {
    fn invalidate_indexes(&mut self) {
        self.manual_index = None;
        self.human_index = None;
    }
    /// Reset the staged values/layers while keeping the session, the revision
    /// history and the bounded artwork asset cache.
    fn clear_values(&mut self) {
        self.claims.clear();
        self.human_protected.clear();
        self.human_plan = None;
        self.raw_inputs.clear();
        self.derived_inputs.clear();
        self.input_bytes = 0;
        self.invalidate_indexes();
        self.bytes = 0;
    }
    fn input_size(value: &Option<String>) -> usize {
        value.as_ref().map_or(0, |value| value.len())
    }
    fn insert_input(
        map: &mut BTreeMap<StageKey, RecordedInput>,
        key: StageKey,
        value: Option<String>,
        seq: u64,
    ) -> usize {
        map.insert(key, RecordedInput { value, seq })
            .map(|previous| Self::input_size(&previous.value))
            .unwrap_or(0)
    }
    /// Record a raw human input. It supersedes any derived basis for the same
    /// field, because the raw value is the input a transform should rerun on.
    fn remember_raw(&mut self, key: StageKey, value: Option<String>) {
        self.input_seq = self.input_seq.wrapping_add(1);
        let seq = self.input_seq;
        let size = Self::input_size(&value);
        let previous = Self::insert_input(&mut self.raw_inputs, key.clone(), value, seq);
        self.input_bytes = self
            .input_bytes
            .saturating_sub(previous)
            .saturating_add(size);
        if let Some(previous) = self.derived_inputs.remove(&key) {
            self.input_bytes = self
                .input_bytes
                .saturating_sub(Self::input_size(&previous.value));
        }
        self.maybe_prune_inputs();
    }
    fn remember_derived(&mut self, key: StageKey, value: Option<String>) {
        self.input_seq = self.input_seq.wrapping_add(1);
        let seq = self.input_seq;
        let size = Self::input_size(&value);
        let previous = Self::insert_input(&mut self.derived_inputs, key, value, seq);
        self.input_bytes = self
            .input_bytes
            .saturating_sub(previous)
            .saturating_add(size);
        self.maybe_prune_inputs();
    }
    fn forget_derived(&mut self, key: &StageKey) {
        if let Some(previous) = self.derived_inputs.remove(key) {
            self.input_bytes = self
                .input_bytes
                .saturating_sub(Self::input_size(&previous.value));
        }
    }
    fn forget_input(&mut self, key: &StageKey) {
        if let Some(previous) = self.raw_inputs.remove(key) {
            self.input_bytes = self
                .input_bytes
                .saturating_sub(Self::input_size(&previous.value));
        }
        self.forget_derived(key);
    }
    fn retain_inputs(&mut self, keep: impl Fn(&StageKey) -> bool) {
        self.raw_inputs.retain(|key, _| keep(key));
        self.derived_inputs.retain(|key, _| keep(key));
        self.input_bytes = self
            .raw_inputs
            .values()
            .chain(self.derived_inputs.values())
            .map(|record| Self::input_size(&record.value))
            .sum();
    }
    /// Oldest-first eviction keeps the ledger bounded without ever treating the
    /// dropped fallback as authoritative. Sorting one batch is cheaper than a
    /// min-scan per eviction, which matters when a grouped publication records a
    /// whole visible scope.
    fn maybe_prune_inputs(&mut self) {
        if self.raw_inputs.len() + self.derived_inputs.len() > MAX_INPUT_FIELDS.saturating_mul(2)
            || self.input_bytes > MAX_INPUT_BYTES.saturating_mul(2)
        {
            self.prune_inputs();
        }
    }
    fn prune_inputs(&mut self) {
        let total = self.raw_inputs.len() + self.derived_inputs.len();
        if total <= MAX_INPUT_FIELDS && self.input_bytes <= MAX_INPUT_BYTES {
            return;
        }
        let mut entries = Vec::with_capacity(total);
        entries.extend(
            self.raw_inputs
                .iter()
                .map(|(key, record)| (record.seq, key.clone(), true)),
        );
        entries.extend(
            self.derived_inputs
                .iter()
                .map(|(key, record)| (record.seq, key.clone(), false)),
        );
        entries.sort_unstable_by_key(|(seq, _, _)| *seq);
        for (_, key, raw) in entries {
            if self.raw_inputs.len() + self.derived_inputs.len() <= MAX_INPUT_FIELDS
                && self.input_bytes <= MAX_INPUT_BYTES
            {
                break;
            }
            let record = if raw {
                self.raw_inputs.remove(&key)
            } else {
                self.derived_inputs.remove(&key)
            };
            if let Some(record) = record {
                self.input_bytes = self
                    .input_bytes
                    .saturating_sub(Self::input_size(&record.value));
            }
        }
    }
}

const HUMAN: &str = "human";
const STAGE_BUDGET: usize = 128 * 1024 * 1024;
pub(super) const MAX_HISTORY_REVISIONS: usize = 128;
const MAX_DELTA_FIELDS: usize = 2048;
const MAX_HISTORY_FIELDS: usize = 8_192;
const MAX_RESYNC_INLINE_FILES: usize = 4_096;
const MAX_RESYNC_INLINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_DELTA_TEXT_CHARS: usize = 4_096;
const MAX_EVENT_ITEMS: usize = 100;
const MAX_EVENT_BYTES: usize = 4 * 1024;
const MAX_ASSETS: usize = 512;
const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_INPUT_FIELDS: usize = 8_192;
pub(super) const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_INTENTS: usize = 512;
const MAX_INTENT_CHARS: usize = 1024 * 1024;
const MAX_INTENT_FIELD_BYTES: usize = 64;
const MAX_INTENT_PATH_BYTES: usize = 8 * 1024;

fn owner(id: &str) -> String {
    format!("draft:{id}")
}

fn art_data(cover: &CoverArtDto, pool: &[Arc<str>]) -> ServiceResult<Arc<str>> {
    if let Some(index) = cover.data_ref {
        Ok(pool
            .get(index)
            .ok_or_else(|| ServiceError::invalid("Invalid staged artwork asset reference"))?
            .clone())
    } else {
        Ok(cover.data_base64.clone())
    }
}
fn art_claim(digest: &str, cover: &CoverArtDto) -> Value {
    json!({"data":digest,"mime":cover.mime,"kind":tagrex_core::model::CoverKind::from_storage_key(&cover.kind).to_storage_key(),"description":cover.description})
}
fn art_fingerprint(
    cover: &CoverArtDto,
    pool: &[Arc<str>],
    cache: &mut HashMap<(usize, usize), String>,
) -> ServiceResult<Value> {
    let data = art_data(cover, pool)?;
    let identity = (data.as_ptr() as usize, data.len());
    let digest = cache
        .entry(identity)
        .or_insert_with(|| hash(data.as_bytes()))
        .clone();
    Ok(art_claim(&digest, cover))
}
struct ArtIdentity {
    id: String,
    mime: String,
    kind: String,
    description: String,
    bytes: usize,
    data: Arc<str>,
}
/// The stable reference for one staged image: the content, mime and picture
/// role, addressed by a digest. The same value is the artwork field's claim
/// fingerprint, so identity, protection and lazy fetch cannot drift apart.
fn art_identity(cover: &CoverArtDto, pool: &[Arc<str>]) -> ServiceResult<ArtIdentity> {
    let data = art_data(cover, pool)?;
    let value = art_claim(&hash(data.as_bytes()), cover);
    Ok(ArtIdentity {
        id: format!("art_{}", hash(&serde_json::to_vec(&value)?)),
        mime: cover.mime.clone(),
        kind: tagrex_core::model::CoverKind::from_storage_key(&cover.kind)
            .to_storage_key()
            .to_string(),
        description: cover.description.clone(),
        bytes: data.len(),
        data,
    })
}
fn remember_asset(stage: &mut StageState, identity: ArtIdentity, revision: u64) {
    stage.asset_seq = stage.asset_seq.wrapping_add(1);
    let ArtIdentity {
        id,
        mime,
        kind,
        description,
        bytes,
        data,
    } = identity;
    if let Some(previous) = stage.assets.remove(&id) {
        stage.asset_bytes = stage.asset_bytes.saturating_sub(previous.bytes);
    }
    stage.asset_bytes = stage.asset_bytes.saturating_add(bytes);
    stage.assets.insert(
        id,
        StageAssetEntry {
            data,
            mime,
            kind,
            description,
            bytes,
            revision,
            seq: stage.asset_seq,
        },
    );
    while stage.assets.len() > MAX_ASSETS || stage.asset_bytes > MAX_ASSET_BYTES {
        let oldest = stage
            .assets
            .iter()
            .min_by_key(|(_, entry)| entry.seq)
            .map(|(id, _)| id.clone());
        match oldest.and_then(|id| stage.assets.remove(&id)) {
            Some(entry) => stage.asset_bytes = stage.asset_bytes.saturating_sub(entry.bytes),
            None => break,
        }
    }
}

/// The exact JSON a staged tag value is digested from. Borrowed so a long value
/// is streamed into the hash instead of being copied into a temporary buffer and
/// then into a serialized one, per field. Field order matches the sorted keys
/// `json!` produced, so the digest of an unchanged field is unchanged.
#[derive(Serialize)]
struct TagAtom<'a> {
    invalid: bool,
    value: &'a Option<String>,
}

/// Hash the JSON encoding of `value` without materializing it. The digest equals
/// hashing `serde_json::to_vec(value)`, including escaping and unicode.
fn digest_json(value: &impl Serialize) -> ServiceResult<String> {
    struct Sink(Sha256);
    impl std::io::Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink(Sha256::new());
    serde_json::to_writer(&mut sink, value)?;
    Ok(format!("{:x}", sink.0.finalize()))
}

pub(super) fn atoms(plan: &PlanDto) -> ServiceResult<BTreeMap<StageKey, String>> {
    let mut result = BTreeMap::new();
    let mut cache = HashMap::new();
    for change in &plan.changes {
        let path: Arc<str> = change.path.as_str().into();
        for field in &change.tag_changes {
            result.insert(
                StageKey {
                    path: path.clone(),
                    field: format!("tag:{}", field.field),
                },
                digest_json(&TagAtom {
                    invalid: field.invalid,
                    value: &field.new,
                })?,
            );
        }
        if let Some(cover) = &change.cover_change {
            let value = cover
                .new
                .iter()
                .map(|cover| art_fingerprint(cover, &plan.cover_assets, &mut cache))
                .collect::<ServiceResult<Vec<_>>>()?;
            result.insert(
                StageKey {
                    path: path.clone(),
                    field: "artwork".into(),
                },
                hash(&serde_json::to_vec(&value)?),
            );
        }
        if change.rename_to.is_some() || !change.sidecar_renames.is_empty() {
            result.insert(StageKey{path:path.clone(),field:"filename".into()},hash(&serde_json::to_vec(&json!({"to":change.rename_to,"copy":change.copy,"sidecars":change.sidecar_renames,"extras":change.folder_extras}))?));
        }
        for block in &change.block_changes {
            let content = block
                .new
                .as_ref()
                .map(|content| {
                    let covers = content
                        .covers
                        .iter()
                        .map(|cover| art_fingerprint(cover, &plan.cover_assets, &mut cache))
                        .collect::<ServiceResult<Vec<_>>>()?;
                    Ok::<_, ServiceError>(json!({"tags":content.tags,"covers":covers}))
                })
                .transpose()?;
            result.insert(StageKey{path:path.clone(),field:format!("block:{}",block.kind)},hash(&serde_json::to_vec(&json!({"new":content,"revision":block.revision,"text_to_unicode":block.text_to_unicode}))?));
        }
    }
    Ok(result)
}
pub(super) fn filtered(plan: &PlanDto, keep: impl Fn(&StageKey) -> bool) -> PlanDto {
    let mut result = PlanDto {
        cover_assets: Vec::new(),
        description: plan.description.clone(),
        message: plan.message.clone(),
        notes: plan.notes.clone(),
        changes: Vec::new(),
        prune_empty_dirs: plan.prune_empty_dirs,
        locked_skipped: Vec::new(),
        unrendered: Vec::new(),
    };
    for source in &plan.changes {
        let tags = source
            .tag_changes
            .iter()
            .filter(|field| keep(&StageKey::tag(&source.path, &field.field)))
            .cloned()
            .collect::<Vec<_>>();
        let artwork =
            source.cover_change.is_some() && keep(&StageKey::special(&source.path, "artwork"));
        let filename = (source.rename_to.is_some() || !source.sidecar_renames.is_empty())
            && keep(&StageKey::special(&source.path, "filename"));
        let blocks = source
            .block_changes
            .iter()
            .filter(|block| {
                keep(&StageKey::special(
                    &source.path,
                    &format!("block:{}", block.kind),
                ))
            })
            .cloned()
            .collect::<Vec<_>>();
        if tags.is_empty() && !artwork && !filename && blocks.is_empty() {
            continue;
        }
        let change = tagrex_commands::FileChangeDto {
            path: source.path.clone(),
            tag_changes: tags,
            cover_change: if artwork {
                source.cover_change.clone()
            } else {
                None
            },
            block_changes: blocks,
            rename_to: if filename {
                source.rename_to.clone()
            } else {
                None
            },
            copy: filename && source.copy,
            rename_root: if filename {
                source.rename_root.clone()
            } else {
                None
            },
            creates_folder: filename && source.creates_folder,
            sidecar_renames: if filename {
                source.sidecar_renames.clone()
            } else {
                Vec::new()
            },
            folder_extras: if filename {
                source.folder_extras.clone()
            } else {
                Vec::new()
            },
        };
        result.changes.push(change);
    }
    let mut pool = Vec::new();
    let mut mapping = HashMap::new();
    for change in &mut result.changes {
        let mut remap = |cover: &mut CoverArtDto| {
            if let Some(old) = cover.data_ref {
                if let Some(asset) = plan.cover_assets.get(old) {
                    let index = *mapping.entry(old).or_insert_with(|| {
                        pool.push(asset.clone());
                        pool.len() - 1
                    });
                    cover.data_ref = Some(index);
                }
            }
        };
        if let Some(covers) = &mut change.cover_change {
            for cover in covers.old.iter_mut().chain(&mut covers.new) {
                remap(cover);
            }
        }
        for block in &mut change.block_changes {
            for content in block.old.iter_mut().chain(&mut block.new) {
                for cover in &mut content.covers {
                    remap(cover);
                }
            }
        }
    }
    result.cover_assets = pool;
    prune(&mut result);
    result
}
fn prune(plan: &mut PlanDto) {
    plan.changes.retain(|change| {
        !change.tag_changes.is_empty()
            || change.cover_change.is_some()
            || change.rename_to.is_some()
            || !change.block_changes.is_empty()
            || !change.sidecar_renames.is_empty()
    });
}
fn combine(
    current: Option<&PlanDto>,
    incoming: Option<&PlanDto>,
    replace: &BTreeSet<StageKey>,
) -> ServiceResult<Option<PlanDto>> {
    let base = current.map(|plan| filtered(plan, |key| !replace.contains(key)));
    let next = incoming.map(|plan| filtered(plan, |key| replace.contains(key)));
    let mut result = match (base, next) {
        (Some(base), Some(next)) => Some(super::plans::merge(base, next)?),
        (Some(base), None) => Some(base),
        (None, Some(next)) => Some(next),
        (None, None) => None,
    };
    if let Some(plan) = &mut result {
        prune(plan);
        plan.description = "Staged edits".into();
        plan.message = None;
        if plan.changes.is_empty() {
            result = None;
        }
    }
    Ok(result)
}
fn empty_plan() -> PlanDto {
    PlanDto {
        cover_assets: Vec::new(),
        description: "Staged edits".into(),
        message: None,
        notes: Vec::new(),
        changes: Vec::new(),
        prune_empty_dirs: false,
        locked_skipped: Vec::new(),
        unrendered: Vec::new(),
    }
}
fn snapshot(state: &State, accepted: bool) -> SharedStageSnapshot {
    let derived_fields = derived_fields(state);
    snapshot_with(state, accepted, derived_fields)
}
/// The derived-field references a snapshot reports, borrowed from the claim
/// ledger rather than copied out of a whole snapshot value.
fn derived_fields(state: &State) -> Vec<StageFieldRef> {
    state
        .stage
        .claims
        .iter()
        .filter(|(_, claim)| claim.derived)
        .map(|(key, _)| key.public())
        .collect()
}
fn snapshot_with(
    state: &State,
    accepted: bool,
    derived_fields: Vec<StageFieldRef>,
) -> SharedStageSnapshot {
    SharedStageSnapshot {
        revision: state.manual_revision,
        human_revision: state.stage.human_updated_at,
        accepted,
        plan: state.manual_plan.clone(),
        human_plan: state.stage.human_plan.clone(),
        derived_fields,
    }
}
/// The exact field set a snapshot serializes to, borrowed so the inline budget
/// can be measured before any of the plans is cloned.
#[derive(Serialize)]
struct SnapshotView<'a> {
    revision: u64,
    human_revision: u64,
    accepted: bool,
    plan: Option<&'a PlanDto>,
    human_plan: Option<&'a PlanDto>,
    derived_fields: BorrowedDerivedFields<'a>,
}
/// Stream derived references directly from the ledger. Even a non-inline
/// snapshot must not first clone a potentially large list of field/path strings.
struct BorrowedDerivedFields<'a>(&'a BTreeMap<StageKey, Claim>);
impl Serialize for BorrowedDerivedFields<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        #[derive(Serialize)]
        struct Field<'a> {
            path: &'a str,
            field: &'a str,
        }
        let mut seq = serializer.serialize_seq(None)?;
        for (key, claim) in self.0 {
            if claim.derived {
                seq.serialize_element(&Field {
                    path: &key.path,
                    field: key.field.strip_prefix("tag:").unwrap_or(&key.field),
                })?;
            }
        }
        seq.end()
    }
}

/// Counts serialized bytes and stops as soon as a budget is crossed, so a
/// projection that cannot be kept costs no full JSON copy at all.
struct ByteBudget {
    bytes: usize,
    limit: usize,
    exceeded: bool,
}
impl std::io::Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes) {
            self.exceeded = true;
            return Err(std::io::Error::other("serialized budget exceeded"));
        }
        self.bytes += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
/// Serialized length of `value` without materializing it, refusing as soon as it
/// would cross `limit`. The count is exactly `serde_json::to_vec(value).len()`.
fn json_len_within(value: &impl Serialize, limit: usize) -> ServiceResult<usize> {
    let mut budget = ByteBudget {
        bytes: 0,
        limit,
        exceeded: false,
    };
    if let Err(error) = serde_json::to_writer(&mut budget, value) {
        return Err(if budget.exceeded {
            ServiceError::new("quota", "Serialized plan exceeds its accounting budget")
        } else {
            error.into()
        });
    }
    Ok(budget.bytes)
}
pub(super) fn event(state: &State, changed: &BTreeSet<StageKey>) -> Value {
    // The event is a wake-up, not the payload: a delta read supplies the
    // fields. Keep the two reference lists inside a small byte and count budget
    // so a large agent plan cannot serialize the whole changed set here.
    let mut fields = Vec::new();
    let mut paths = Vec::new();
    let mut listed = BTreeSet::new();
    let mut bytes = 0usize;
    for key in changed {
        let cost = key.path.len() + key.field.len() + 32;
        if fields.len() >= MAX_EVENT_ITEMS || bytes.saturating_add(cost) > MAX_EVENT_BYTES {
            break;
        }
        bytes = bytes.saturating_add(cost);
        fields.push(key.public());
        if listed.insert(key.path.as_ref()) {
            paths.push(key.path.to_string());
        }
    }
    json!({
        "session":state.stage.session,
        "revision":state.manual_revision,
        "human_revision":state.stage.human_updated_at,
        "files":state.manual_plan.as_ref().map(|plan|plan.changes.len()).unwrap_or(0),
        "staged_fields":state.stage.claims.len(),
        "changed_fields":changed.len(),
        "changed":fields,
        "changed_paths":paths,
        "resync":fields.len()<changed.len()
    })
}
fn stage_budget(plan: Option<&PlanDto>) -> ServiceResult<usize> {
    let Some(plan) = plan else {
        return Ok(0);
    };
    // Same byte count as `serde_json::to_vec(plan)`, streamed: a large staged
    // plan is measured without materializing a second complete copy of it.
    json_len_within(plan, STAGE_BUDGET).map_err(|error| match error.code.as_str() {
        "quota" => ServiceError::new("quota", "The shared staged plan exceeds its 128 MiB budget"),
        _ => error,
    })
}
/// Positions are keyed by a 64-bit path hash: building and updating the index
/// allocates nothing per file, which matters at 100k-row stages. A collision
/// only costs the linear fallback in `indexed_change`, never correctness.
fn path_key(path: &str) -> u64 {
    // Word-at-a-time mixing keeps a 100k-row index build in the low
    // milliseconds even in an unoptimised build.
    let mut key = 0x9e37_79b9_7f4a_7c15u64 ^ (path.len() as u64);
    let (words, rest) = path.as_bytes().as_chunks::<8>();
    for word in words {
        key = (key ^ u64::from_le_bytes(*word))
            .wrapping_mul(0x0000_0100_0000_01b3)
            .rotate_left(27);
    }
    if !rest.is_empty() {
        let mut word = 0u64;
        for (index, byte) in rest.iter().enumerate() {
            word |= u64::from(*byte) << (index * 8);
        }
        key = (key ^ word)
            .wrapping_mul(0x0000_0100_0000_01b3)
            .rotate_left(27);
    }
    key ^ (key >> 29)
}
fn build_index(plan: &PlanDto) -> HashMap<u64, usize> {
    let mut index = HashMap::with_capacity(plan.changes.len());
    for (position, change) in plan.changes.iter().enumerate() {
        index.insert(path_key(&change.path), position);
    }
    index
}
fn ensure_manual_index(state: &mut State) {
    let State {
        manual_plan, stage, ..
    } = state;
    if stage.manual_index.is_none() {
        stage.manual_index = Some(
            manual_plan
                .as_ref()
                .map(build_index)
                .unwrap_or_else(HashMap::new),
        );
    }
}
fn indexed_change<'a>(
    plan: &'a PlanDto,
    index: Option<&HashMap<u64, usize>>,
    path: &str,
) -> Option<&'a FileChangeDto> {
    if let Some(index) = index {
        if let Some(change) = index
            .get(&path_key(path))
            .and_then(|index| plan.changes.get(*index))
            .filter(|change| change.path == path)
        {
            return Some(change);
        }
    }
    plan.changes.iter().find(|change| change.path == path)
}
fn record_change(state: &mut State, keys: BTreeSet<StageKey>, structural: bool) {
    let revision = state.manual_revision;
    state.stage.history_fields = state.stage.history_fields.saturating_add(keys.len());
    state.stage.history.push_back(StageHistory {
        revision,
        keys: keys.into_iter().collect(),
        structural,
    });
    while state.stage.history.len() > MAX_HISTORY_REVISIONS
        || state.stage.history_fields > MAX_HISTORY_FIELDS
    {
        match state.stage.history.pop_front() {
            Some(entry) => {
                state.stage.history_fields =
                    state.stage.history_fields.saturating_sub(entry.keys.len());
                state.stage.history_overflowed = true;
            }
            None => break,
        }
    }
}
/// Test hook: record a structural revision boundary without inventing a move.
#[cfg(test)]
pub(super) fn record_structural_for_test(state: &mut State) {
    state.manual_revision = state.manual_revision.wrapping_add(1);
    record_change(state, BTreeSet::new(), true);
}
fn live_field_capabilities(state: &State) -> Value {
    json!({
        "version":1,
        "delta":true,
        "intents":true,
        "recompute":true,
        "session_check":true,
        "derived_scope":true,
        "asset_refs":true,
        "session":state.stage.session,
        "history_revisions":MAX_HISTORY_REVISIONS,
        "history_fields":MAX_HISTORY_FIELDS,
        "max_delta_fields":MAX_DELTA_FIELDS
    })
}

fn clipped(value: Option<&str>) -> (Option<String>, usize, bool) {
    match value {
        None => (None, 0, false),
        Some(value) => {
            let chars = value.chars().count();
            if chars > MAX_DELTA_TEXT_CHARS {
                (
                    Some(value.chars().take(MAX_DELTA_TEXT_CHARS).collect()),
                    chars,
                    true,
                )
            } else {
                (Some(value.to_string()), chars, false)
            }
        }
    }
}
fn artwork_cover(
    cover: &CoverArtDto,
    pool: &[Arc<str>],
    stage: &mut StageState,
    revision: u64,
) -> ServiceResult<StageArtworkCover> {
    let identity = art_identity(cover, pool)?;
    // Cheap native validity: an empty payload, a payload over the embed budget
    // or a non-image MIME cannot be written, so the lazy consumer can mark the
    // exact image instead of silently treating it as valid.
    let invalid = identity.bytes == 0
        || identity.bytes > MAX_IMAGE_BYTES
        || !identity.mime.starts_with("image/");
    let reference = StageArtworkCover {
        asset: identity.id.clone(),
        mime: identity.mime.clone(),
        kind: identity.kind.clone(),
        description: identity.description.clone(),
        bytes: identity.bytes,
        invalid,
    };
    remember_asset(stage, identity, revision);
    Ok(reference)
}
fn artwork_covers(
    covers: &[CoverArtDto],
    plan: &PlanDto,
    stage: &mut StageState,
    revision: u64,
) -> ServiceResult<Vec<StageArtworkCover>> {
    covers
        .iter()
        .map(|cover| artwork_cover(cover, &plan.cover_assets, stage, revision))
        .collect()
}
fn field_delta(
    change: &FileChangeDto,
    key: &StageKey,
    plan: &PlanDto,
    stage: &mut StageState,
    revision: u64,
) -> ServiceResult<Option<StageFieldDelta>> {
    let path = key.path.to_string();
    let human = stage.human_protected.contains_key(key);
    let derived = stage.claims.get(key).is_some_and(|claim| claim.derived);
    let public_field = key
        .field
        .strip_prefix("tag:")
        .unwrap_or(&key.field)
        .to_string();
    if let Some(field) = key.field.strip_prefix("tag:") {
        let Some(value) = change.tag_changes.iter().find(|value| value.field == field) else {
            return Ok(None);
        };
        let (old, old_chars, old_truncated) = clipped(value.old.as_deref());
        let (new, new_chars, new_truncated) = clipped(value.new.as_deref());
        return Ok(Some(StageFieldDelta {
            path,
            field: public_field.clone(),
            human,
            derived,
            tag: Some(StageTagDelta {
                old,
                new,
                invalid: value.invalid,
                old_chars,
                new_chars,
                truncated: old_truncated || new_truncated,
            }),
            artwork: None,
            rename: None,
            block: None,
        }));
    }
    if key.field == "artwork" {
        let Some(cover) = change.cover_change.as_ref() else {
            return Ok(None);
        };
        let old = artwork_covers(&cover.old, plan, stage, revision)?;
        let covers = artwork_covers(&cover.new, plan, stage, revision)?;
        let invalid = covers.iter().any(|cover| cover.invalid);
        return Ok(Some(StageFieldDelta {
            path,
            field: public_field.clone(),
            human,
            derived,
            tag: None,
            artwork: Some(StageArtworkDelta {
                empty: covers.is_empty(),
                invalid,
                old,
                covers,
            }),
            rename: None,
            block: None,
        }));
    }
    if key.field == "filename" {
        let Some(to) = change.rename_to.clone() else {
            return Ok(None);
        };
        return Ok(Some(StageFieldDelta {
            path,
            field: public_field.clone(),
            human,
            derived,
            tag: None,
            artwork: None,
            rename: Some(StageRenameDelta {
                to,
                copy: change.copy,
                creates_folder: change.creates_folder,
                rename_root: change.rename_root.clone(),
                sidecars: change.sidecar_renames.clone(),
                folder_extras: change.folder_extras.clone(),
            }),
            block: None,
        }));
    }
    let Some(kind) = key.field.strip_prefix("block:") else {
        return Ok(None);
    };
    let Some(block) = change.block_changes.iter().find(|block| block.kind == kind) else {
        return Ok(None);
    };
    let mut covers = Vec::new();
    if let Some(content) = block.new.as_ref() {
        covers = artwork_covers(&content.covers, plan, stage, revision)?;
    }
    let invalid = covers.iter().any(|cover| cover.invalid);
    let text = block.new.as_ref().and_then(|content| {
        content.tags.iter().find_map(|(key, value)| {
            (*key == "LYRICS" || *key == "UNSYNCEDLYRICS" || *key == "lyrics")
                .then(|| value.clone())
        })
    });
    let (text, chars, truncated) = clipped(text.as_deref());
    Ok(Some(StageFieldDelta {
        path,
        field: public_field.clone(),
        human,
        derived,
        tag: None,
        artwork: None,
        rename: None,
        block: Some(StageBlockDelta {
            kind: block.kind.clone(),
            revision: block.revision.clone(),
            text_to_unicode: block.text_to_unicode,
            chars,
            truncated,
            invalid,
            text,
            covers,
        }),
    }))
}

fn stage_delta_locked(
    state: &mut State,
    since: Option<u64>,
    session: Option<&str>,
    started: Instant,
) -> ServiceResult<StageDelta> {
    let revision = state.manual_revision;
    let files = state
        .manual_plan
        .as_ref()
        .map(|plan| plan.changes.len())
        .unwrap_or(0);
    let staged_fields = state.stage.claims.len();
    let from = since.unwrap_or(0);
    let mut reason = None;
    if since.is_none() || session.is_none() {
        reason = Some("initial");
    } else if session != Some(state.stage.session.as_str()) || from > revision {
        reason = Some("session");
    } else if from < revision {
        match state.stage.history.front() {
            None => reason = Some("gap"),
            Some(front) if from < front.revision.saturating_sub(1) => {
                reason = Some(if state.stage.history_overflowed {
                    "overflow"
                } else {
                    "gap"
                })
            }
            _ => {}
        }
        if reason.is_none()
            && state
                .stage
                .history
                .iter()
                .any(|entry| entry.revision > from && entry.structural)
        {
            reason = Some("structural");
        }
    }
    let Some(reason) = reason else {
        let mut keys = BTreeSet::new();
        for entry in state
            .stage
            .history
            .iter()
            .filter(|entry| entry.revision > from)
        {
            keys.extend(entry.keys.iter().cloned());
        }
        if keys.len() <= MAX_DELTA_FIELDS {
            return incremental_delta(state, from, keys, files, staged_fields, started);
        }
        return Ok(resync_delta(
            state,
            from,
            "overflow",
            files,
            staged_fields,
            started,
        ));
    };
    Ok(resync_delta(
        state,
        from,
        reason,
        files,
        staged_fields,
        started,
    ))
}
/// A resync discards incremental state: inline the full snapshot when it is
/// bounded, otherwise tell the caller to read it explicitly.
fn resync_delta(
    state: &mut State,
    from: u64,
    reason: &str,
    files: usize,
    staged_fields: usize,
    started: Instant,
) -> StageDelta {
    let revision = state.manual_revision;
    let human_revision = state.stage.human_updated_at;
    let session = state.stage.session.clone();
    let mut snapshot_value = None;
    let mut required = false;
    if files > MAX_RESYNC_INLINE_FILES {
        required = true;
    } else {
        // Measure the exact bytes an inline snapshot would carry from borrowed
        // data, stopping at the inline budget. The previous shape serialized a
        // full clone, so a stage too large to inline paid two complete copies
        // (and their allocation) inside the shared state lock to discover that.
        let fits = {
            let view = SnapshotView {
                revision,
                human_revision,
                accepted: true,
                plan: state.manual_plan.as_ref(),
                human_plan: state.stage.human_plan.as_ref(),
                derived_fields: BorrowedDerivedFields(&state.stage.claims),
            };
            json_len_within(&view, MAX_RESYNC_INLINE_BYTES).is_ok()
        };
        if fits {
            snapshot_value = Some(snapshot(state, true));
        } else {
            required = true;
        }
    }
    StageDelta {
        session,
        revision,
        human_revision,
        from,
        resync: true,
        resync_reason: Some(reason.into()),
        files,
        staged_fields,
        changed_files: files,
        changed_fields: staged_fields,
        removed_fields: 0,
        changes: Vec::new(),
        removed: Vec::new(),
        removed_paths: Vec::new(),
        snapshot: snapshot_value,
        snapshot_required: required,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    }
}
fn incremental_delta(
    state: &mut State,
    from: u64,
    keys: BTreeSet<StageKey>,
    files: usize,
    staged_fields: usize,
    started: Instant,
) -> ServiceResult<StageDelta> {
    ensure_manual_index(state);
    let revision = state.manual_revision;
    let human_revision = state.stage.human_updated_at;
    let session = state.stage.session.clone();
    let State {
        manual_plan, stage, ..
    } = state;
    let mut changes = Vec::new();
    let mut removed = Vec::new();
    let mut removed_paths = BTreeSet::new();
    let mut changed_paths = BTreeSet::new();
    for key in &keys {
        changed_paths.insert(key.path.to_string());
        let delta = match manual_plan.as_ref() {
            Some(plan) => indexed_change(plan, stage.manual_index.as_ref(), key.path.as_ref())
                .map(|change| field_delta(change, key, plan, stage, revision))
                .transpose()?
                .flatten(),
            None => None,
        };
        match delta {
            Some(delta) => changes.push(delta),
            None => {
                removed.push(key.public());
                if !path_has_claims(&stage.claims, key.path.as_ref()) {
                    removed_paths.insert(key.path.to_string());
                }
            }
        }
    }
    Ok(StageDelta {
        session,
        revision,
        human_revision,
        from,
        resync: false,
        resync_reason: None,
        files,
        staged_fields,
        changed_files: changed_paths.len(),
        changed_fields: changes.len(),
        removed_fields: removed.len(),
        changes,
        removed,
        removed_paths: removed_paths.into_iter().collect(),
        snapshot: None,
        snapshot_required: false,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}
/// Whether any claim remains for this path, using the ordered ledger rather
/// than a scan. Used to report a file leaving the stage.
fn path_has_claims(claims: &BTreeMap<StageKey, Claim>, path: &str) -> bool {
    let start = StageKey {
        path: path.into(),
        field: String::new(),
    };
    claims
        .range(start..)
        .next()
        .is_some_and(|(key, _)| key.path.as_ref() == path)
}
fn keeps_field(value: &tagrex_commands::FieldChangeDto) -> bool {
    value.invalid || value.old.as_deref().unwrap_or("") != value.new.as_deref().unwrap_or("")
}
fn is_empty_change(change: &FileChangeDto) -> bool {
    change.tag_changes.is_empty()
        && change.cover_change.is_none()
        && change.rename_to.is_none()
        && change.sidecar_renames.is_empty()
        && change.block_changes.is_empty()
}
/// Patch only the tag fields named by a human intent into one layer. This keeps
/// the ordinary one-field publication proportional to the affected keys rather
/// than to the whole stage, while producing exactly what `combine` would.
fn patch_tag_plan(
    plan: &mut Option<PlanDto>,
    index: &mut Option<HashMap<u64, usize>>,
    incoming: &PlanDto,
    changed: &BTreeSet<StageKey>,
) {
    let mut by_path: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for key in changed {
        if let Some(field) = key.field.strip_prefix("tag:") {
            by_path.entry(key.path.as_ref()).or_default().push(field);
        }
    }
    if by_path.is_empty() {
        return;
    }
    let incoming_by_path = incoming
        .changes
        .iter()
        .map(|change| (change.path.as_str(), change))
        .collect::<HashMap<_, _>>();
    let target = plan.get_or_insert_with(empty_plan);
    for (path, fields) in by_path {
        if index.is_none() {
            *index = Some(build_index(target));
        }
        let position = index.as_ref().and_then(|index| {
            index
                .get(&path_key(path))
                .copied()
                .filter(|position| target.changes[*position].path == path)
        });
        let incoming_fields = incoming_by_path
            .get(path)
            .map(|change| {
                change
                    .tag_changes
                    .iter()
                    .map(|value| (value.field.as_str(), value))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        match position {
            Some(position) => {
                let change = &mut target.changes[position];
                for field in fields {
                    match incoming_fields
                        .get(field)
                        .filter(|value| keeps_field(value))
                    {
                        Some(value) => {
                            if let Some(existing) = change
                                .tag_changes
                                .iter_mut()
                                .find(|existing| existing.field == *field)
                            {
                                existing.new = value.new.clone();
                                existing.invalid = value.invalid;
                            } else {
                                change.tag_changes.push((*value).clone());
                            }
                        }
                        None => change.tag_changes.retain(|value| value.field != field),
                    }
                }
                if is_empty_change(change) {
                    // A file left the stage: keep the surviving order but the
                    // positions after it shift, so the index is rebuilt lazily.
                    target.changes.remove(position);
                    *index = None;
                }
            }
            None => {
                let Some(source) = incoming_by_path.get(path) else {
                    continue;
                };
                let mut change = (*source).clone();
                change
                    .tag_changes
                    .retain(|value| fields.contains(&value.field.as_str()) && keeps_field(value));
                if change.tag_changes.is_empty() {
                    continue;
                }
                change.cover_change = None;
                change.rename_to = None;
                change.copy = false;
                change.rename_root = None;
                change.creates_folder = false;
                change.sidecar_renames.clear();
                change.folder_extras.clear();
                change.block_changes.clear();
                let key = path_key(&change.path);
                target.changes.push(change);
                // Appending keeps every earlier position valid: no rebuild.
                if let Some(index) = index.as_mut() {
                    index.insert(key, target.changes.len() - 1);
                }
            }
        }
    }
    for skip in &incoming.locked_skipped {
        if !target
            .locked_skipped
            .iter()
            .any(|item| item.field == skip.field)
        {
            target.locked_skipped.push(skip.clone());
        }
    }
    target.description = "Staged edits".into();
    target.message = None;
    if target.changes.is_empty() {
        *plan = None;
        *index = Some(HashMap::new());
    }
}
/// Values currently staged for the tag fields of `scope` paths. O(staged fields
/// in scope), not the whole stage, and never a value authority.
fn staged_values_at(
    state: &mut State,
    scope: &BTreeSet<String>,
) -> BTreeMap<StageKey, Option<String>> {
    ensure_manual_index(state);
    let State {
        manual_plan, stage, ..
    } = state;
    let mut values = BTreeMap::new();
    if let Some(plan) = manual_plan.as_ref() {
        for path in scope {
            if let Some(change) = indexed_change(plan, stage.manual_index.as_ref(), path) {
                for field in &change.tag_changes {
                    values.insert(StageKey::tag(&change.path, &field.field), field.new.clone());
                }
            }
        }
    }
    values
}
/// The saved/proposed values of one plan's tag fields, keyed by stage identity.
fn plan_tag_values(plan: &PlanDto) -> BTreeMap<StageKey, (Option<String>, Option<String>)> {
    let mut values = BTreeMap::new();
    for change in &plan.changes {
        for field in &change.tag_changes {
            values.insert(
                StageKey::tag(&change.path, &field.field),
                (field.old.clone(), field.new.clone()),
            );
        }
    }
    values
}
fn intent_value(intent: &StageIntent) -> Option<String> {
    // The intent route never trims: whitespace and invalid drafts stay as typed.
    intent.value.clone().filter(|value| !value.is_empty())
}
fn intent_edit(intent: &StageIntent) -> TagEditDto {
    TagEditDto {
        path: intent.path.clone(),
        field: intent.field.clone(),
        value: intent.value.clone(),
    }
}
fn seed_edit(key: &StageKey, value: &Option<String>) -> TagEditDto {
    TagEditDto {
        path: key.path.to_string(),
        field: key
            .field
            .strip_prefix("tag:")
            .unwrap_or(&key.field)
            .to_string(),
        value: value.clone(),
    }
}
/// Everything one human publication needs after its preview has run.
struct HumanPatch {
    plan: PlanDto,
    touched: Vec<StageFieldRef>,
    delta_from: u64,
    /// Raw intent values to remember for a later non-compounding recompute.
    raw: BTreeMap<StageKey, Option<String>>,
    /// The pre-transform input used for each seeded field.
    base: BTreeMap<StageKey, Option<String>>,
    /// Fields the recompute inspected. Fields with no new change are removed.
    scope: BTreeSet<StageKey>,
    /// A transform group set ran over the scope.
    transformed: bool,
    /// An explicit recompute, even with no transform groups.
    recompute: bool,
}

impl Service {
    pub fn shared_stage(&self) -> ServiceResult<SharedStageSnapshot> {
        Ok(snapshot(&*lock(&self.0.state)?, true))
    }
    pub fn shared_stage_status(&self) -> ServiceResult<Value> {
        let state = lock(&self.0.state)?;
        Ok(json!({
            "shared_staging":true,
            "revision":state.manual_revision,
            "files":state.manual_plan.as_ref().map(|plan|plan.changes.len()).unwrap_or(0),
            "staged_fields":state.stage.claims.len(),
            "live_fields":live_field_capabilities(&state)
        }))
    }
    pub fn live_field_capabilities(&self) -> ServiceResult<Value> {
        Ok(live_field_capabilities(&*lock(&self.0.state)?))
    }
    /// Bounded revisioned projection of the shared stage.
    pub fn shared_stage_delta(
        &self,
        since: Option<u64>,
        session: Option<&str>,
    ) -> ServiceResult<StageDelta> {
        let started = Instant::now();
        let mut state = lock(&self.0.state)?;
        stage_delta_locked(&mut state, since, session, started)
    }
    /// Fetch one staged image by the reference a delta reported. The bytes are
    /// the exact pooled payload the stage would embed.
    pub fn shared_stage_asset(&self, asset: &str) -> ServiceResult<Value> {
        if !asset.starts_with("art_") || asset.len() > 96 {
            return Err(ServiceError::new(
                "invalid_handle",
                "Not a staged artwork reference",
            ));
        }
        let mut state = lock(&self.0.state)?;
        let revision = state.manual_revision;
        state.stage.asset_seq = state.stage.asset_seq.wrapping_add(1);
        let seq = state.stage.asset_seq;
        let entry = state.stage.assets.get_mut(asset).ok_or_else(|| {
            ServiceError::new(
                "asset_expired",
                "This staged artwork is no longer cached; refresh the staged change",
            )
        })?;
        entry.revision = revision;
        entry.seq = seq;
        if entry.bytes > MAX_IMAGE_BYTES {
            return Err(ServiceError::new(
                "too_large",
                "The staged image exceeds the service image budget",
            ));
        }
        Ok(json!({
            "asset":asset,
            "mime":entry.mime,
            "kind":entry.kind,
            "description":entry.description,
            "data_base64":entry.data,
            "bytes":entry.bytes,
            "revision":revision
        }))
    }
    /// Apply narrow human field intents. Preview work is limited to the named
    /// fields (or the explicit transform scope), and only the affected keys are
    /// patched into the stage. Everything else about publication is unchanged:
    /// the same protection, revision CAS and claim ledger.
    ///
    /// Compatibility wrapper for callers written against the first contract.
    /// Trusted desktop calls `publish_human_intents_request` so it can pass the
    /// stage session and the explicit `recompute` operation.
    pub fn publish_human_intents(
        &self,
        intents: Vec<StageIntent>,
        expected_revision: u64,
        groups: &[ActionGroupDto],
        scope: &[String],
    ) -> ServiceResult<StageIntentOutcome> {
        self.publish_human_intents_request(StageIntentRequest {
            intents,
            expected_revision,
            groups: groups.to_vec(),
            scope: scope.to_vec(),
            recompute: false,
            session: None,
        })
    }
    pub fn publish_human_intents_request(
        &self,
        request: StageIntentRequest,
    ) -> ServiceResult<StageIntentOutcome> {
        let StageIntentRequest {
            intents,
            expected_revision,
            groups,
            scope,
            recompute,
            session,
        } = request;
        if intents.len() > MAX_INTENTS {
            return Err(ServiceError::invalid("Provide at most 512 field intents"));
        }
        if intents.is_empty() && !recompute {
            return Err(ServiceError::invalid(
                "Provide between 1 and 512 field intents",
            ));
        }
        for intent in &intents {
            if intent.path.is_empty() || intent.path.len() > MAX_INTENT_PATH_BYTES {
                return Err(ServiceError::invalid(
                    "An intent path is missing or too long",
                ));
            }
            if intent.field.is_empty() || intent.field.len() > MAX_INTENT_FIELD_BYTES {
                return Err(ServiceError::invalid(
                    "An intent field is missing or too long",
                ));
            }
            if intent.field == "artwork"
                || intent.field == "filename"
                || intent.field.starts_with("block:")
            {
                return Err(ServiceError::invalid(
                    "Use the full staged plan for artwork, filename or tag-block changes",
                ));
            }
            // NUL handling is the existing preview rule: it strips them rather
            // than refusing, so one pasted value behaves the same on both routes.
            if intent
                .value
                .as_ref()
                .is_some_and(|value| value.chars().count() > MAX_INTENT_CHARS)
            {
                return Err(ServiceError::invalid("An intent value is too large"));
            }
        }
        let mut scope_paths = BTreeSet::new();
        for path in scope {
            if path.is_empty() || path.len() > MAX_INTENT_PATH_BYTES {
                return Err(ServiceError::invalid(
                    "A recompute scope path is missing or too long",
                ));
            }
            scope_paths.insert(path);
        }
        if recompute && scope_paths.is_empty() {
            return Err(ServiceError::invalid(
                "A recompute needs a non-empty paths scope",
            ));
        }
        let transformed = !groups.is_empty();
        if groups.iter().any(|group| {
            group.rules.iter().any(|rule| {
                rule.enabled
                    && matches!(
                        rule.scope.as_deref().unwrap_or(&group.scope),
                        "filename" | "fileext"
                    )
            })
        }) {
            return Err(ServiceError::new(
                "full_plan_required",
                "Filename rules require the full staged-plan route",
            ));
        }
        let seeded = transformed || recompute;
        let intent_keys: BTreeSet<_> = intents
            .iter()
            .map(|intent| StageKey::tag(&intent.path, &intent.field))
            .collect();
        let mut base = {
            let mut state = lock(&self.0.state)?;
            if let Some(session) = session.as_deref() {
                if session != state.stage.session {
                    return Err(ServiceError::new(
                        "stage_session_mismatch",
                        "The shared stage session changed; reload the staged changes",
                    ));
                }
            }
            let mut base = if seeded {
                staged_values_at(&mut state, &scope_paths)
            } else {
                BTreeMap::new()
            };
            if seeded {
                if base.keys().any(|key| {
                    state
                        .stage
                        .claims
                        .get(key)
                        .is_some_and(|claim| claim.derived)
                        && !state.stage.raw_inputs.contains_key(key)
                        && !state.stage.derived_inputs.contains_key(key)
                        && !intent_keys.contains(key)
                }) {
                    return Err(ServiceError::new("raw_basis_expired", "The original input for a transformed field expired. Apply or discard this stage before recomputing it."));
                }
                // Raw human input wins over the pre-transform basis, which wins
                // over the current staged value. A derived result is never fed
                // back as the next raw input.
                for (key, record) in &state.stage.raw_inputs {
                    if scope_paths.contains(key.path.as_ref()) {
                        base.insert(key.clone(), record.value.clone());
                    }
                }
                for (key, record) in &state.stage.derived_inputs {
                    if scope_paths.contains(key.path.as_ref())
                        && !state.stage.raw_inputs.contains_key(key)
                    {
                        base.insert(key.clone(), record.value.clone());
                    }
                }
            }
            base
        };
        let mut raw = BTreeMap::new();
        for intent in &intents {
            let key = StageKey::tag(&intent.path, &intent.field);
            let value = intent_value(intent);
            base.insert(key.clone(), value.clone());
            raw.insert(key, value);
        }
        let touched = intents
            .iter()
            .map(|intent| StageFieldRef {
                path: intent.path.clone(),
                field: intent.field.clone(),
            })
            .collect::<Vec<_>>();
        let scope_keys = base.keys().cloned().collect::<BTreeSet<_>>();
        let edits: Vec<TagEditDto> = if seeded {
            base.iter()
                .map(|(key, value)| seed_edit(key, value))
                .collect()
        } else {
            intents.iter().map(intent_edit).collect()
        };
        let plan = {
            let guard = lock(&self.0.app)?;
            let app = guard.as_ref().ok_or_else(no_library)?;
            if transformed {
                let paths = scope_paths.iter().map(PathBuf::from).collect::<Vec<_>>();
                app.preview_live_edits_with_covers(&edits, &paths, &groups, &[])?
            } else {
                app.preview_tag_edits(&edits)?
            }
        };
        self.publish_human_patch(
            HumanPatch {
                plan,
                touched,
                delta_from: expected_revision,
                raw,
                base,
                scope: scope_keys,
                transformed,
                recompute,
            },
            expected_revision,
            session,
        )
    }
    fn publish_human_patch(
        &self,
        patch: HumanPatch,
        expected_revision: u64,
        session: Option<String>,
    ) -> ServiceResult<StageIntentOutcome> {
        let HumanPatch {
            plan,
            touched,
            delta_from,
            raw,
            base,
            scope,
            transformed,
            recompute,
        } = patch;
        let incoming = atoms(&plan)?;
        let plan_tags = plan_tag_values(&plan);
        let locked = plan.locked_skipped.clone();
        // Streamed estimate, so a scope-sized preview is never serialized into a
        // throwaway buffer just to learn how large it is.
        let added = json_len_within(&plan.changes, STAGE_BUDGET)
            .map_err(|error| match error.code.as_str() {
                "quota" => {
                    ServiceError::new("quota", "The shared staged plan exceeds its 128 MiB budget")
                }
                _ => error,
            })?
            .saturating_add(256);
        let mut state = lock(&self.0.state)?;
        let current_session = state.stage.session.clone();
        // Authoritative session check under the mutation lock: a numerically
        // plausible revision from another stage lifecycle still cannot stage.
        if session
            .as_deref()
            .is_some_and(|session| session != current_session)
        {
            return Err(ServiceError::new(
                "stage_session_mismatch",
                "The shared stage session changed; reload the staged changes",
            ));
        }
        if expected_revision < state.stage.human_updated_at {
            // A newer human revision already exists. Apply nothing; the delta
            // still shows what this caller missed, so it can retire stale input.
            let delta = stage_delta_locked(
                &mut state,
                Some(delta_from),
                Some(&current_session),
                Instant::now(),
            )?;
            return Ok(StageIntentOutcome {
                accepted: false,
                revision: delta.revision,
                human_revision: delta.human_revision,
                session: current_session,
                files: delta.files,
                staged_fields: delta.staged_fields,
                locked,
                delta,
            });
        }
        if expected_revision > state.manual_revision {
            return Err(ServiceError::invalid(
                "Shared stage revision is from a different session",
            ));
        }
        let touched_keys = touched
            .iter()
            .map(StageKey::from_public)
            .collect::<BTreeSet<_>>();
        let mut changed = BTreeSet::new();
        changed.extend(touched_keys.iter().cloned());
        if recompute || transformed {
            // Publish only a field whose staged fingerprint actually changes.
            // Unchanged scope fields keep their existing owner and value, so an
            // agent contribution is not silently claimed by a preset refresh.
            let mut candidates = scope;
            candidates.extend(incoming.keys().cloned());
            for key in candidates {
                if touched_keys.contains(&key) {
                    changed.insert(key);
                    continue;
                }
                let next = incoming.get(&key);
                let current = state.stage.claims.get(&key).map(|claim| &claim.fingerprint);
                if next == current {
                    continue;
                }
                if next.is_none() && current.is_none() {
                    continue;
                }
                changed.insert(key);
            }
        } else {
            changed.extend(incoming.keys().cloned());
        }
        if changed.is_empty() {
            let delta = stage_delta_locked(
                &mut state,
                Some(delta_from),
                Some(&current_session),
                Instant::now(),
            )?;
            return Ok(StageIntentOutcome {
                accepted: true,
                revision: delta.revision,
                human_revision: delta.human_revision,
                session: current_session,
                files: delta.files,
                staged_fields: delta.staged_fields,
                locked,
                delta,
            });
        }
        if state.stage.bytes.saturating_add(added) > STAGE_BUDGET {
            // Re-measure exactly once before refusing: the estimate only ever
            // over-counts, so this is rare and never loosens the budget.
            state.stage.bytes = stage_budget(state.manual_plan.as_ref())?;
            if state.stage.bytes.saturating_add(added) > STAGE_BUDGET {
                return Err(ServiceError::new(
                    "quota",
                    "The shared staged plan exceeds its 128 MiB budget",
                ));
            }
        }
        let revision = state.manual_revision.wrapping_add(1);
        for key in &changed {
            state.stage.human_protected.insert(key.clone(), revision);
            if let Some(fingerprint) = incoming.get(key) {
                state.stage.claims.insert(
                    key.clone(),
                    Claim {
                        fingerprint: fingerprint.clone(),
                        owners: [HUMAN.into()].into(),
                        derived: transformed
                            && plan_tags
                                .get(key)
                                .is_some_and(|(old, new)| base.get(key).unwrap_or(old) != new),
                    },
                );
            } else {
                state.stage.claims.remove(key);
            }
        }
        {
            let State {
                manual_plan, stage, ..
            } = &mut *state;
            patch_tag_plan(manual_plan, &mut stage.manual_index, &plan, &changed);
            patch_tag_plan(
                &mut stage.human_plan,
                &mut stage.human_index,
                &plan,
                &changed,
            );
            stage.bytes = stage.bytes.saturating_add(added);
        }
        state.manual_revision = revision;
        state.stage.human_updated_at = revision;
        // Raw intents are the input a later recompute must rerun on. The
        // derived basis is what the transform ran on now, never its output.
        for (key, value) in &raw {
            state.stage.remember_raw(key.clone(), value.clone());
        }
        if transformed || recompute {
            for key in &changed {
                match incoming.get(key) {
                    Some(_) if transformed => {
                        let basis = base
                            .get(key)
                            .cloned()
                            .unwrap_or_else(|| plan_tags.get(key).and_then(|(old, _)| old.clone()));
                        state.stage.remember_derived(key.clone(), basis);
                    }
                    Some(_) => state.stage.forget_derived(key),
                    None => {
                        // A raw clear keeps its raw record so a later recompute
                        // still knows the person wanted it cleared.
                        if state.stage.raw_inputs.contains_key(key) {
                            state.stage.forget_derived(key);
                        } else {
                            state.stage.forget_input(key);
                        }
                    }
                }
            }
        } else {
            for key in &changed {
                state.stage.forget_derived(key);
            }
        }
        state.stage.prune_inputs();
        record_change(&mut state, changed.clone(), false);
        let value = event(&state, &changed);
        let delta = stage_delta_locked(
            &mut state,
            Some(delta_from),
            Some(&current_session),
            Instant::now(),
        )?;
        drop(state);
        self.emit("shared_stage_changed", value);
        Ok(StageIntentOutcome {
            accepted: true,
            revision,
            human_revision: revision,
            session: current_session,
            files: delta.files,
            staged_fields: delta.staged_fields,
            locked,
            delta,
        })
    }
    pub fn shared_apply_plan(&self, paths: &BTreeSet<String>) -> ServiceResult<PlanDto> {
        Ok(self.shared_apply_plan_with_revision(paths)?.0)
    }
    pub fn shared_apply_plan_with_revision(
        &self,
        paths: &BTreeSet<String>,
    ) -> ServiceResult<(PlanDto, u64)> {
        let state = lock(&self.0.state)?;
        let plan = state
            .manual_plan
            .as_ref()
            .ok_or_else(|| ServiceError::new("no_changes", "The shared staging area is empty"))?;
        Ok((
            filtered(plan, |key| paths.contains(key.path.as_ref())),
            state.manual_revision,
        ))
    }
    pub fn publish_manual_plan(
        &self,
        plan: Option<PlanDto>,
        expected_revision: u64,
    ) -> ServiceResult<SharedStageSnapshot> {
        self.publish_human(plan, expected_revision, Vec::new(), true)
    }
    pub fn publish_manual_plan_with_intents(
        &self,
        plan: Option<PlanDto>,
        expected_revision: u64,
        touched: Vec<StageFieldRef>,
    ) -> ServiceResult<SharedStageSnapshot> {
        self.publish_human(plan, expected_revision, touched, false)
    }
    fn publish_human(
        &self,
        plan: Option<PlanDto>,
        expected_revision: u64,
        touched: Vec<StageFieldRef>,
        replace_layer: bool,
    ) -> ServiceResult<SharedStageSnapshot> {
        let incoming = plan.as_ref().map(atoms).transpose()?.unwrap_or_default();
        let mut state = lock(&self.0.state)?;
        // Agent-only revision changes are merged transparently. An older human
        // generation cannot rewind a newer human input that already arrived.
        if expected_revision < state.stage.human_updated_at {
            return Ok(snapshot(&state, false));
        }
        if expected_revision > state.manual_revision {
            return Err(ServiceError::invalid(
                "Shared stage revision is from a different session",
            ));
        }
        // The whole human layer is only re-digested when it is being replaced.
        let previous = if replace_layer {
            state
                .stage
                .human_plan
                .as_ref()
                .map(atoms)
                .transpose()?
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let mut changed = incoming.keys().cloned().collect::<BTreeSet<_>>();
        changed.extend(touched.iter().map(StageKey::from_public));
        if replace_layer {
            changed.extend(previous.keys().cloned());
        }
        if changed.is_empty() {
            return Ok(snapshot(&state, true));
        }
        let previous_values = state
            .manual_plan
            .as_ref()
            .map(plan_tag_values)
            .unwrap_or_default();
        let incoming_values = plan
            .as_ref()
            .map(plan_tag_values)
            .unwrap_or_default()
            .into_iter()
            .map(|(key, (_, new))| (key, new))
            .collect::<BTreeMap<_, _>>();
        let human = combine(state.stage.human_plan.as_ref(), plan.as_ref(), &changed)?;
        let combined = combine(state.manual_plan.as_ref(), plan.as_ref(), &changed)?;
        state.stage.bytes = stage_budget(combined.as_ref())?;
        let revision = state.manual_revision.wrapping_add(1);
        for key in &changed {
            state.stage.human_protected.insert(key.clone(), revision);
            if let Some(fingerprint) = incoming.get(key) {
                let derived = state
                    .stage
                    .claims
                    .get(key)
                    .is_some_and(|claim| claim.derived && claim.fingerprint == *fingerprint);
                state.stage.claims.insert(
                    key.clone(),
                    Claim {
                        fingerprint: fingerprint.clone(),
                        owners: [HUMAN.into()].into(),
                        derived,
                    },
                );
            } else {
                state.stage.claims.remove(key);
            }
        }
        state.stage.human_plan = human;
        state.stage.human_updated_at = revision;
        state.manual_plan = combined;
        state.manual_revision = revision;
        state.stage.invalidate_indexes();
        // A full-plan publication is still a human input. Keep the recorded raw
        // basis while it restates the same working value (artwork/import/
        // structural routes); for a tag value it actually changes, the new
        // staged value becomes the basis, so the stale record is retired rather
        // than re-recorded (bounded, no duplicate of a whole imported plan).
        for key in &changed {
            if !key.field.starts_with("tag:") {
                continue;
            }
            match incoming_values.get(key) {
                Some(next) => {
                    let unchanged = previous_values
                        .get(key)
                        .is_some_and(|(_, previous)| previous == next);
                    if !unchanged {
                        state.stage.forget_input(key);
                    }
                }
                None => state.stage.forget_input(key),
            }
        }
        record_change(&mut state, changed.clone(), false);
        let value = event(&state, &changed);
        let result = snapshot(&state, true);
        drop(state);
        self.emit("shared_stage_changed", value);
        Ok(result)
    }
    pub fn clear_shared_stage(&self, expected_revision: u64) -> ServiceResult<SharedStageSnapshot> {
        let mut state = lock(&self.0.state)?;
        if expected_revision != state.manual_revision {
            return Ok(snapshot(&state, false));
        }
        let changed: BTreeSet<StageKey> = state.stage.claims.keys().cloned().collect();
        state.stage.clear_values();
        state.manual_plan = None;
        state.manual_revision = state.manual_revision.wrapping_add(1);
        state.stage.human_updated_at = state.manual_revision;
        record_change(&mut state, changed.clone(), false);
        let value = event(&state, &changed);
        let result = snapshot(&state, true);
        drop(state);
        self.emit("shared_stage_changed", value);
        Ok(result)
    }
    /// A new library/file-set session cannot inherit another session's stage.
    /// Clears the in-memory stage, advances both revisions, rotates the opaque
    /// session and records a structural boundary, so a delayed intent from the
    /// previous session can never stage stale paths into the new one even when
    /// its revision number would otherwise look plausible.
    pub fn reset_shared_stage_for_library(&self) -> ServiceResult<SharedStageSnapshot> {
        let mut state = lock(&self.0.state)?;
        let changed: BTreeSet<StageKey> = state.stage.claims.keys().cloned().collect();
        state.stage.clear_values();
        state.stage.session = opaque("stage")?;
        state.stage.history.clear();
        state.stage.history_fields = 0;
        state.stage.history_overflowed = true;
        state.manual_plan = None;
        state.manual_revision = state.manual_revision.wrapping_add(1);
        state.stage.human_updated_at = state.manual_revision;
        record_change(&mut state, changed.clone(), true);
        let value = event(&state, &changed);
        let result = snapshot(&state, true);
        drop(state);
        self.emit("shared_stage_changed", value);
        Ok(result)
    }
    pub(super) fn stage_agent_locked(
        state: &mut State,
        id: &str,
        parent: Option<&str>,
        plan: &PlanDto,
        scope: &BTreeSet<StageKey>,
        principal: &str,
    ) -> ServiceResult<(Vec<StageFieldRef>, BTreeSet<StageKey>)> {
        let incoming = atoms(plan)?;
        let mut excluded = BTreeSet::new();
        let mut replace = BTreeSet::new();
        for key in scope {
            let current = state.stage.claims.get(key);
            let desired = incoming.get(key);
            if state.stage.human_protected.contains_key(key)
                && current.map(|claim| &claim.fingerprint) != desired
            {
                excluded.insert(key.clone());
                continue;
            }
            if desired.is_some() || current.is_some() {
                replace.insert(key.clone());
            }
        }
        for key in &replace {
            if let Some(claim) = state.stage.claims.get(key) {
                if incoming.get(key) != Some(&claim.fingerprint)
                    && claim.owners.iter().any(|existing| {
                        existing != HUMAN
                            && !parent.is_some_and(|id| existing == &owner(id))
                            && existing
                                .strip_prefix("draft:")
                                .and_then(|id| state.plans.get(id))
                                .is_none_or(|plan| plan.owner != principal)
                    })
                {
                    return Err(ServiceError::new("stage_field_conflict",format!("A different agent has staged {} on {}; inspect the current staging field before replacing it",key.field,key.path)));
                }
            }
        }
        let combined = combine(state.manual_plan.as_ref(), Some(plan), &replace)?;
        state.stage.bytes = stage_budget(combined.as_ref())?;
        let contribution = owner(id);
        if let Some(parent) = parent {
            let parent = owner(parent);
            for claim in state.stage.claims.values_mut() {
                claim.owners.remove(&parent);
            }
        }
        for key in &replace {
            if let Some(fingerprint) = incoming.get(key) {
                if let Some(claim) = state
                    .stage
                    .claims
                    .get_mut(key)
                    .filter(|claim| claim.fingerprint == *fingerprint)
                {
                    if !state.stage.human_protected.contains_key(key) {
                        claim.owners.insert(contribution.clone());
                    }
                } else {
                    state.stage.claims.insert(
                        key.clone(),
                        Claim {
                            fingerprint: fingerprint.clone(),
                            owners: [contribution.clone()].into(),
                            derived: false,
                        },
                    );
                }
            } else {
                state.stage.claims.remove(key);
            }
        }
        // Superseded contribution metadata does not carry any alternate value.
        state
            .stage
            .claims
            .retain(|_, claim| !claim.owners.is_empty());
        // An agent-owned field has no human raw basis to preserve. Protected
        // fields keep theirs (an agent may only restate the same fingerprint).
        let replaced_unprotected = replace
            .iter()
            .filter(|key| !state.stage.human_protected.contains_key(*key))
            .cloned()
            .collect::<BTreeSet<_>>();
        state
            .stage
            .retain_inputs(|key| !replaced_unprotected.contains(key));
        state.manual_plan = combined;
        state.manual_revision = state.manual_revision.wrapping_add(1);
        state.stage.invalidate_indexes();
        record_change(state, replace.clone(), false);
        Ok((excluded.iter().map(StageKey::public).collect(), replace))
    }
    pub(super) fn discard_stage_contribution(&self, id: &str) -> ServiceResult<()> {
        let contribution = owner(id);
        let mut state = lock(&self.0.state)?;
        let mut remove = BTreeSet::new();
        for (key, claim) in &mut state.stage.claims {
            if claim.owners.remove(&contribution) && claim.owners.is_empty() {
                remove.insert(key.clone());
            }
        }
        if remove.is_empty() {
            return Ok(());
        }
        state.stage.claims.retain(|key, _| !remove.contains(key));
        state.stage.retain_inputs(|key| !remove.contains(key));
        state.manual_plan = state
            .manual_plan
            .as_ref()
            .map(|plan| filtered(plan, |key| !remove.contains(key)));
        if state
            .manual_plan
            .as_ref()
            .is_some_and(|plan| plan.changes.is_empty())
        {
            state.manual_plan = None;
        }
        state.manual_revision = state.manual_revision.wrapping_add(1);
        state.stage.invalidate_indexes();
        record_change(&mut state, remove.clone(), false);
        let value = event(&state, &remove);
        drop(state);
        self.emit("shared_stage_changed", value);
        Ok(())
    }
    pub(super) fn live_projection(
        &self,
        scope: &BTreeSet<StageKey>,
    ) -> ServiceResult<Option<PlanDto>> {
        let state = lock(&self.0.state)?;
        Ok(state
            .manual_plan
            .as_ref()
            .map(|plan| filtered(plan, |key| scope.contains(key))))
    }
    pub fn complete_shared_apply(
        &self,
        applied_plan: &PlanDto,
        applied_revision: u64,
    ) -> ServiceResult<SharedStageSnapshot> {
        let mut valid = filtered(applied_plan, |_| true);
        for change in &mut valid.changes {
            change.tag_changes.retain(|field| !field.invalid);
        }
        let applied = atoms(&valid)?;
        let written = applied_plan
            .changes
            .iter()
            .map(|change| PathBuf::from(&change.path))
            .collect::<Vec<_>>();
        // App is acquired before state, the same order as native write paths.
        let guard = lock(&self.0.app)?;
        let app = guard.as_ref().ok_or_else(no_library)?;
        self.complete_shared_apply_with_app(app, &applied, &written, applied_revision)
    }
    pub(super) fn complete_shared_apply_with_app(
        &self,
        app: &App,
        applied: &BTreeMap<StageKey, String>,
        written: &[PathBuf],
        applied_revision: u64,
    ) -> ServiceResult<SharedStageSnapshot> {
        // Build/rebase outside the state lock. A user can keep editing while
        // filesystem snapshots refresh; retry the short CAS if they did.
        self.refresh_live_identities(written)?;
        loop {
            let (revision, removed, mut combined, mut human) = {
                let state = lock(&self.0.state)?;
                let removed = applied
                    .iter()
                    .filter_map(|(key, fingerprint)| {
                        state
                            .stage
                            .claims
                            .get(key)
                            .filter(|claim| claim.fingerprint == *fingerprint)
                            .map(|_| key.clone())
                    })
                    .collect::<BTreeSet<_>>();
                (
                    state.manual_revision,
                    removed.clone(),
                    state
                        .manual_plan
                        .as_ref()
                        .map(|plan| filtered(plan, |key| !removed.contains(key))),
                    state
                        .stage
                        .human_plan
                        .as_ref()
                        .map(|plan| filtered(plan, |key| !removed.contains(key))),
                )
            };
            let existing = written
                .iter()
                .filter(|path| path.exists())
                .cloned()
                .collect::<Vec<_>>();
            // A written path that no longer exists means Apply renamed or moved
            // it: the stage's file identities changed, so an incremental delta
            // over old paths could resurrect them. Rebasing itself only
            // rewrites saved baselines; a changed path set is the same signal.
            let mut rebased_identities = written.iter().any(|path| !path.exists());
            if let Some(plan) = combined.as_mut() {
                if !existing.is_empty() {
                    let before = plan
                        .changes
                        .iter()
                        .map(|change| change.path.clone())
                        .collect::<BTreeSet<_>>();
                    *plan = app.rebase_staged_plan(plan, &existing)?;
                    rebased_identities |= plan
                        .changes
                        .iter()
                        .any(|change| !before.contains(&change.path));
                }
                if plan.changes.is_empty() {
                    combined = None;
                }
            }
            if let Some(plan) = human.as_mut() {
                if !existing.is_empty() {
                    let before = plan
                        .changes
                        .iter()
                        .map(|change| change.path.clone())
                        .collect::<BTreeSet<_>>();
                    *plan = app.rebase_staged_plan(plan, &existing)?;
                    rebased_identities |= plan
                        .changes
                        .iter()
                        .any(|change| !before.contains(&change.path));
                }
                if plan.changes.is_empty() {
                    human = None;
                }
            }
            let mut state = lock(&self.0.state)?;
            if state.manual_revision != revision {
                drop(state);
                continue;
            }
            state.stage.claims.retain(|key, _| !removed.contains(key));
            let written_paths = written
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<BTreeSet<_>>();
            // An applied field is saved now; its raw/derived input is retired
            // so a later recompute cannot resurrect it as pending work.
            state
                .stage
                .retain_inputs(|key| !written_paths.contains(key.path.as_ref()));
            let empty = combined.is_none();
            state.stage.human_protected.retain(|key, revision| {
                *revision > applied_revision
                    || (!empty && !written_paths.contains(key.path.as_ref()))
            });
            state.manual_plan = combined;
            state.stage.human_plan = human;
            state.manual_revision = state.manual_revision.wrapping_add(1);
            state.stage.human_updated_at = state.manual_revision;
            state.stage.invalidate_indexes();
            record_change(&mut state, removed.clone(), rebased_identities);
            let value = event(&state, &removed);
            let result = snapshot(&state, true);
            drop(state);
            self.emit("shared_stage_changed", value);
            return Ok(result);
        }
    }

    pub(super) fn stage_get(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        self.stage_get_with_path_check(principal, args, |grant, path| {
            destination(grant, path).is_ok()
        })
    }

    pub(super) fn stage_get_with_path_check(
        &self,
        principal: &Principal,
        args: &Value,
        path_allowed: impl Fn(&Grant, &Path) -> bool,
    ) -> ServiceResult<Value> {
        let offset = number(args, "offset", 0);
        let limit = number(args, "limit", 100).clamp(1, MAX_PAGE);
        // Copy only paths under the lock. Canonicalization may block on remote
        // disks; perform it once per path outside shared state ownership.
        let (grant, revision, paths) = {
            let state = lock(&self.0.state)?;
            let grant = self.grant_in_state(&state, principal, "read")?.clone();
            let paths = state
                .manual_plan
                .as_ref()
                .into_iter()
                .flat_map(|plan| &plan.changes)
                .filter(|change| {
                    change
                        .tag_changes
                        .iter()
                        .any(|field| visible_field(&grant, &field.field))
                        || change.cover_change.is_some() && visible_field(&grant, "artwork")
                        || change.rename_to.is_some() && visible_field(&grant, "filename")
                        || change
                            .block_changes
                            .iter()
                            .any(|block| visible_field(&grant, &format!("block:{}", block.kind)))
                })
                .map(|change| change.path.clone())
                .collect::<BTreeSet<_>>();
            (grant, state.manual_revision, paths)
        };
        let paths = paths
            .into_iter()
            .filter(|path| path_allowed(&grant, Path::new(path)))
            .collect::<BTreeSet<_>>();
        let state = lock(&self.0.state)?;
        if self.grant_in_state(&state, principal, "read")? != &grant
            || state.manual_revision != revision
        {
            return Err(ServiceError::conflict(
                "The shared stage or access scope changed; request the view again",
            ));
        }
        let Some(plan) = state.manual_plan.as_ref() else {
            return Ok(json!({"revision":revision,"files":0,"changes":[]}));
        };
        let allowed = |key: &StageKey| {
            visible_field(&grant, key.field.strip_prefix("tag:").unwrap_or(&key.field))
                && paths.contains(key.path.as_ref())
        };
        let mut total = 0;
        let mut page_paths = BTreeSet::new();
        for change in &plan.changes {
            let visible = change
                .tag_changes
                .iter()
                .any(|field| allowed(&StageKey::tag(&change.path, &field.field)))
                || change.cover_change.is_some()
                    && allowed(&StageKey::special(&change.path, "artwork"))
                || change.rename_to.is_some()
                    && allowed(&StageKey::special(&change.path, "filename"))
                || change.block_changes.iter().any(|block| {
                    allowed(&StageKey::special(
                        &change.path,
                        &format!("block:{}", block.kind),
                    ))
                });
            if visible {
                if total >= offset && page_paths.len() < limit {
                    page_paths.insert(change.path.clone());
                }
                total += 1;
            }
        }
        let visible = filtered(plan, |key| {
            page_paths.contains(key.path.as_ref()) && allowed(key)
        });
        drop(state);
        let rows=visible.changes.iter().map(|change|json!({"path":change.path,"tags":change.tag_changes.iter().map(|field|json!({"field":field.field,"old":field.old.as_ref().map(|value|value.chars().take(2048).collect::<String>()),"new":field.new.as_ref().map(|value|value.chars().take(2048).collect::<String>()),"old_chars":field.old.as_ref().map(|value|value.chars().count()),"new_chars":field.new.as_ref().map(|value|value.chars().count()),"invalid":field.invalid})).collect::<Vec<_>>(),"rename_to":change.rename_to,"artwork":change.cover_change.as_ref().map(|cover|super::artwork_policy::artwork_diff(cover,&visible.cover_assets)),"blocks":change.block_changes.iter().map(|block|json!({"kind":block.kind})).collect::<Vec<_>>()})).collect::<Vec<_>>();
        Ok(
            json!({"revision":revision,"files":total,"offset":offset,"next_offset":(offset+rows.len()<total).then_some(offset+rows.len()),"changes":rows}),
        )
    }
}

/// Pure field visibility; path admission is evaluated once per request.
fn visible_field(grant: &Grant, field: &str) -> bool {
    grant.fields.is_empty() || grant.fields.contains(field)
}

/// Projection accounting and digest compatibility: bounded serialization must
/// produce the same bytes, digests and inline/required decisions as the owned
/// path it replaced, without materializing a second full copy of a large stage.
#[cfg(test)]
mod stage_projection_accounting_tests {
    use super::*;
    use tagrex_commands::FieldChangeDto;

    fn change(index: usize, value_bytes: usize) -> FileChangeDto {
        FileChangeDto {
            path: format!("/music/{index:05}.flac"),
            rename_to: None,
            copy: false,
            tag_changes: ["title", "artist", "album", "genre"]
                .iter()
                .enumerate()
                .map(|(field, name)| FieldChangeDto {
                    field: name.to_string(),
                    old: Some(format!("old {index} {field}")),
                    new: Some("y".repeat(value_bytes)),
                    invalid: field == 1,
                })
                .collect(),
            cover_change: None,
            sidecar_renames: Vec::new(),
            block_changes: Vec::new(),
            rename_root: None,
            creates_folder: index.is_multiple_of(7),
            folder_extras: Vec::new(),
        }
    }
    fn plan(files: usize, value_bytes: usize) -> PlanDto {
        PlanDto {
            cover_assets: Vec::new(),
            description: "Staged edits".into(),
            message: None,
            notes: Vec::new(),
            changes: (0..files).map(|i| change(i, value_bytes)).collect(),
            prune_empty_dirs: false,
            locked_skipped: Vec::new(),
            unrendered: Vec::new(),
        }
    }
    fn state(plan: Option<PlanDto>, human: Option<PlanDto>) -> State {
        let mut claims = BTreeMap::new();
        claims.insert(
            StageKey::tag("/music/00000.flac", "genre"),
            Claim {
                fingerprint: "fp".into(),
                owners: [HUMAN.into()].into(),
                derived: true,
            },
        );
        let stage = StageState {
            session: "stage_projection".into(),
            human_plan: human,
            human_updated_at: 7,
            claims,
            ..Default::default()
        };
        State {
            security: SecurityStore::default(),
            snapshots: HashMap::new(),
            snapshot_cache: None,
            selections: HashMap::new(),
            query_lanes: HashMap::new(),
            plans: HashMap::new(),
            assets: HashMap::new(),
            releases: HashMap::new(),
            image_receipts: HashMap::new(),
            operations: HashMap::new(),
            manual_plan: plan,
            manual_revision: 11,
            stage,
            recovery_required: false,
            redo_revision: 0,
        }
    }

    #[test]
    fn the_borrowed_snapshot_view_serializes_exactly_the_snapshot_bytes() {
        let manual = plan(3, 32);
        let human = plan(2, 16);
        let mut state = state(Some(manual), Some(human));
        state.stage.claims.insert(
            StageKey::special("/music/cover.flac", "artwork"),
            Claim {
                fingerprint: "art".into(),
                owners: [HUMAN.into()].into(),
                derived: true,
            },
        );
        state.stage.claims.insert(
            StageKey::tag("/music/plain.flac", "title"),
            Claim {
                fingerprint: "plain".into(),
                owners: [HUMAN.into()].into(),
                derived: false,
            },
        );
        let derived = derived_fields(&state);
        let owned = snapshot(&state, true);
        assert_eq!(owned.derived_fields, derived);
        let view = SnapshotView {
            revision: owned.revision,
            human_revision: owned.human_revision,
            accepted: owned.accepted,
            plan: state.manual_plan.as_ref(),
            human_plan: state.stage.human_plan.as_ref(),
            derived_fields: BorrowedDerivedFields(&state.stage.claims),
        };
        assert_eq!(
            serde_json::to_vec(&view).unwrap(),
            serde_json::to_vec(&owned).unwrap(),
            "the inline decision must measure the bytes the answer carries"
        );
        assert_eq!(
            json_len_within(&view, MAX_RESYNC_INLINE_BYTES).unwrap(),
            serde_json::to_vec(&owned).unwrap().len()
        );
    }

    #[test]
    fn a_small_resync_still_inlines_the_same_snapshot() {
        let mut state = state(Some(plan(2, 8)), None);
        let delta = stage_delta_locked(&mut state, None, None, Instant::now()).unwrap();
        assert!(delta.resync);
        assert_eq!(delta.resync_reason.as_deref(), Some("initial"));
        assert!(!delta.snapshot_required);
        let inline = delta.snapshot.expect("bounded snapshot is inlined");
        assert_eq!(inline.revision, 11);
        assert_eq!(inline.human_revision, 7);
        assert_eq!(inline.plan.as_ref().map(|plan| plan.changes.len()), Some(2));
        assert_eq!(inline.derived_fields.len(), 1);
    }

    #[test]
    fn an_oversized_resync_requires_the_explicit_snapshot_without_building_it() {
        // 3000 files stay inside the inline file bound, so only the byte budget
        // can decide; each file is far over it.
        let mut state = state(Some(plan(3_000, 1_024)), None);
        let serialized = serde_json::to_vec(state.manual_plan.as_ref().unwrap())
            .unwrap()
            .len();
        assert!(serialized > MAX_RESYNC_INLINE_BYTES);
        let started = Instant::now();
        let delta = stage_delta_locked(&mut state, None, None, Instant::now()).unwrap();
        assert!(delta.snapshot_required);
        assert!(delta.snapshot.is_none());
        assert_eq!(delta.resync_reason.as_deref(), Some("initial"));
        assert_eq!(delta.files, 3_000);
        // The bounded decision is a read, not a full JSON render: it must stay
        // well inside the work a full serialize of this stage would take.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );

        // A source change still resyncs with the same bounded decision.
        let delta = stage_delta_locked(
            &mut state,
            Some(1),
            Some("stage_projection"),
            Instant::now(),
        )
        .unwrap();
        assert!(delta.resync);
        assert!(delta.snapshot_required);
        assert!(delta.snapshot.is_none());
        // Another session can never receive this stage incrementally.
        let delta =
            stage_delta_locked(&mut state, Some(11), Some("other"), Instant::now()).unwrap();
        assert_eq!(delta.resync_reason.as_deref(), Some("session"));
        assert!(delta.snapshot.is_none());
    }

    #[test]
    fn stage_budget_matches_the_wire_length_and_refuses_before_copying() {
        let plan = plan(40, 64);
        let wire = serde_json::to_vec(&plan).unwrap().len();
        assert_eq!(stage_budget(None).unwrap(), 0);
        assert_eq!(stage_budget(Some(&plan)).unwrap(), wire);
        assert_eq!(json_len_within(&plan, STAGE_BUDGET).unwrap(), wire);
        let refused = json_len_within(&plan, wire - 1).expect_err("over budget");
        assert_eq!(refused.code, "quota");
        assert_eq!(
            stage_budget(Some(&plan)).unwrap(),
            wire,
            "the staged-plan budget must not drift from the wire length"
        );
    }

    #[test]
    fn over_budget_accounting_stops_instead_of_rendering_every_element() {
        use serde::ser::SerializeSeq;
        struct Many(std::cell::Cell<usize>);
        impl Serialize for Many {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(1_000_000))?;
                for _ in 0..1_000_000 {
                    self.0.set(self.0.get() + 1);
                    seq.serialize_element(&"expansion\n\"é")?;
                }
                seq.end()
            }
        }
        let many = Many(std::cell::Cell::new(0));
        assert_eq!(
            json_len_within(&many, 128).expect_err("over budget").code,
            "quota"
        );
        assert!(many.0.get() < 20, "visited {}", many.0.get());
    }

    #[test]
    fn atoms_digest_the_wire_json_and_keep_stage_keys() {
        let mut plan = plan(2, 0);
        plan.changes[0].tag_changes[0].new = Some("Café \"quoted\"\nline".into());
        plan.changes[0].tag_changes[2].new = None;
        plan.changes[1].tag_changes[1].new = Some("ジャングル".into());
        let atoms = atoms(&plan).unwrap();
        assert_eq!(atoms.len(), 8);
        for change in &plan.changes {
            for field in &change.tag_changes {
                let key = StageKey::tag(&change.path, &field.field);
                let expected = hash(
                    &serde_json::to_vec(&json!({"value":field.new,"invalid":field.invalid}))
                        .unwrap(),
                );
                assert_eq!(atoms.get(&key), Some(&expected), "{key:?}");
            }
        }
    }
}
