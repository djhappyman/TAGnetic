//! The single live staging area shared by the desktop and all paired agents.
//! Handles name scopes in this area, never competing copies of desired values.
use super::*;
use tagrex_commands::CoverArtDto;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StageFieldRef {
    pub path: String,
    pub field: String,
}
#[derive(Clone, Serialize)]
pub struct SharedStageSnapshot {
    pub revision: u64,
    pub human_revision: u64,
    pub accepted: bool,
    pub plan: Option<PlanDto>,
    pub human_plan: Option<PlanDto>,
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
}
#[derive(Default)]
pub(super) struct StageState {
    claims: BTreeMap<StageKey, Claim>,
    human_protected: BTreeMap<StageKey, u64>,
    human_plan: Option<PlanDto>,
    human_updated_at: u64,
}
const HUMAN: &str = "human";
fn owner(id: &str) -> String {
    format!("draft:{id}")
}

fn art_fingerprint(
    cover: &CoverArtDto,
    pool: &[Arc<str>],
    cache: &mut HashMap<(usize, usize), String>,
) -> ServiceResult<Value> {
    let data = if let Some(index) = cover.data_ref {
        pool.get(index)
            .ok_or_else(|| ServiceError::invalid("Invalid staged artwork asset reference"))?
            .as_ref()
    } else {
        cover.data_base64.as_ref()
    };
    let identity = (data.as_ptr() as usize, data.len());
    let digest = cache
        .entry(identity)
        .or_insert_with(|| hash(data.as_bytes()));
    Ok(
        json!({"data":digest,"mime":cover.mime,"kind":tagrex_core::model::CoverKind::from_storage_key(&cover.kind).to_storage_key(),"description":cover.description}),
    )
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
                hash(&serde_json::to_vec(
                    &json!({"value":field.new,"invalid":field.invalid}),
                )?),
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
        let any = source
            .tag_changes
            .iter()
            .any(|field| keep(&StageKey::tag(&source.path, &field.field)))
            || source.cover_change.is_some() && keep(&StageKey::special(&source.path, "artwork"))
            || source.rename_to.is_some() && keep(&StageKey::special(&source.path, "filename"))
            || source.block_changes.iter().any(|block| {
                keep(&StageKey::special(
                    &source.path,
                    &format!("block:{}", block.kind),
                ))
            });
        if !any {
            continue;
        }
        let mut change = source.clone();
        change
            .tag_changes
            .retain(|field| keep(&StageKey::tag(&change.path, &field.field)));
        if !keep(&StageKey::special(&change.path, "artwork")) {
            change.cover_change = None;
        }
        if !keep(&StageKey::special(&change.path, "filename")) {
            change.rename_to = None;
            change.copy = false;
            change.rename_root = None;
            change.creates_folder = false;
            change.sidecar_renames.clear();
            change.folder_extras.clear();
        }
        change.block_changes.retain(|block| {
            keep(&StageKey::special(
                &change.path,
                &format!("block:{}", block.kind),
            ))
        });
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
fn snapshot(state: &State, accepted: bool) -> SharedStageSnapshot {
    SharedStageSnapshot {
        revision: state.manual_revision,
        human_revision: state.stage.human_updated_at,
        accepted,
        plan: state.manual_plan.clone(),
        human_plan: state.stage.human_plan.clone(),
    }
}
fn event(state: &State, changed: &BTreeSet<StageKey>) -> Value {
    let paths = changed
        .iter()
        .map(|key| key.path.as_ref())
        .collect::<BTreeSet<_>>();
    json!({"revision":state.manual_revision,"files":state.manual_plan.as_ref().map(|plan|plan.changes.len()).unwrap_or(0),"changed_paths":paths.iter().take(100).collect::<Vec<_>>(),"resync":paths.len()>100})
}
fn stage_budget(plan: Option<&PlanDto>) -> ServiceResult<()> {
    if let Some(plan) = plan {
        if serde_json::to_vec(plan)?.len() > 128 * 1024 * 1024 {
            return Err(ServiceError::new(
                "quota",
                "The shared staged plan exceeds its 128 MiB budget",
            ));
        }
    }
    Ok(())
}

impl Service {
    pub fn shared_stage(&self) -> ServiceResult<SharedStageSnapshot> {
        Ok(snapshot(&*lock(&self.0.state)?, true))
    }
    pub fn shared_stage_status(&self) -> ServiceResult<Value> {
        let state = lock(&self.0.state)?;
        Ok(
            json!({"shared_staging":true,"revision":state.manual_revision,"files":state.manual_plan.as_ref().map(|plan|plan.changes.len()).unwrap_or(0)}),
        )
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
        let previous = state
            .stage
            .human_plan
            .as_ref()
            .map(atoms)
            .transpose()?
            .unwrap_or_default();
        let mut changed = incoming.keys().cloned().collect::<BTreeSet<_>>();
        changed.extend(touched.iter().map(StageKey::from_public));
        if replace_layer {
            changed.extend(previous.keys().cloned());
        }
        if changed.is_empty() {
            return Ok(snapshot(&state, true));
        }
        let human = combine(state.stage.human_plan.as_ref(), plan.as_ref(), &changed)?;
        let combined = combine(state.manual_plan.as_ref(), plan.as_ref(), &changed)?;
        stage_budget(combined.as_ref())?;
        let revision = state.manual_revision.wrapping_add(1);
        for key in &changed {
            state.stage.human_protected.insert(key.clone(), revision);
            if let Some(fingerprint) = incoming.get(key) {
                state.stage.claims.insert(
                    key.clone(),
                    Claim {
                        fingerprint: fingerprint.clone(),
                        owners: [HUMAN.into()].into(),
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
        let changed = state.stage.claims.keys().cloned().collect();
        state.stage = StageState::default();
        state.manual_plan = None;
        state.manual_revision = state.manual_revision.wrapping_add(1);
        state.stage.human_updated_at = state.manual_revision;
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
    ) -> ServiceResult<Vec<StageFieldRef>> {
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
        stage_budget(combined.as_ref())?;
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
        state.manual_plan = combined;
        state.manual_revision = state.manual_revision.wrapping_add(1);
        Ok(excluded.iter().map(StageKey::public).collect())
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
            if let Some(plan) = combined.as_mut() {
                if !existing.is_empty() {
                    *plan = app.rebase_staged_plan(plan, &existing)?;
                }
                if plan.changes.is_empty() {
                    combined = None;
                }
            }
            if let Some(plan) = human.as_mut() {
                if !existing.is_empty() {
                    *plan = app.rebase_staged_plan(plan, &existing)?;
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
            let empty = combined.is_none();
            state.stage.human_protected.retain(|key, revision| {
                *revision > applied_revision
                    || (!empty && !written_paths.contains(key.path.as_ref()))
            });
            state.manual_plan = combined;
            state.stage.human_plan = human;
            state.manual_revision = state.manual_revision.wrapping_add(1);
            state.stage.human_updated_at = state.manual_revision;
            let value = event(&state, &removed);
            let result = snapshot(&state, true);
            drop(state);
            self.emit("shared_stage_changed", value);
            return Ok(result);
        }
    }

    pub(super) fn stage_get(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let grant = self.grant(principal, "read")?;
        let state = lock(&self.0.state)?;
        let revision = state.manual_revision;
        let offset = number(args, "offset", 0);
        let limit = number(args, "limit", 100).clamp(1, MAX_PAGE);
        let Some(plan) = state.manual_plan.as_ref() else {
            return Ok(json!({"revision":revision,"files":0,"changes":[]}));
        };
        let allowed = |key: &StageKey| {
            destination(&grant, Path::new(key.path.as_ref())).is_ok()
                && (grant.fields.is_empty()
                    || grant
                        .fields
                        .contains(key.field.strip_prefix("tag:").unwrap_or(&key.field)))
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
