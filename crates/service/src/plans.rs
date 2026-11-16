use super::*;
use tagrex_commands::{ActionGroupDto, ImportSelectionDto, TagEditDto, TransformRuleDto};

#[cfg(windows)]
mod windows_identity;

/// Native progress counts preflight/write/transfer steps. The service exposes
/// completed media files instead, and keeps UI notifications bounded independently
/// of cancellation/authorization checks and the current operation counter.
struct FileProgress<'a> {
    expected: HashMap<&'a Path, (tagrex_core::plan::ApplyStep, bool)>,
    processed: usize,
    total: usize,
    last_emitted: Option<std::time::Duration>,
}
struct FileProgressUpdate {
    processed: usize,
    total: usize,
    changed: bool,
    emit: bool,
}
impl<'a> FileProgress<'a> {
    fn new(plan: &'a PlanDto) -> Self {
        use tagrex_core::plan::ApplyStep;
        let expected = plan
            .changes
            .iter()
            .map(|change| {
                let transfer = change
                    .rename_to
                    .as_deref()
                    .is_some_and(|target| target != change.path);
                let metadata = !change.tag_changes.is_empty()
                    || change.cover_change.is_some()
                    || !change.block_changes.is_empty();
                // Copies receive their tags after the transfer; a move receives its
                // tags before it. Each media path therefore has one final signal.
                let step = if transfer && !(change.copy && metadata) {
                    ApplyStep::Transfer
                } else {
                    ApplyStep::Tags
                };
                (Path::new(&change.path), (step, false))
            })
            .collect();
        Self {
            expected,
            processed: 0,
            total: plan.changes.len(),
            last_emitted: None,
        }
    }
    fn observe(
        &mut self,
        progress: &tagrex_core::plan::ApplyProgress,
        now: std::time::Duration,
    ) -> FileProgressUpdate {
        let previous = self.processed;
        if let Some(step) = progress.completed {
            if let Some((expected, completed)) = self.expected.get_mut(progress.path.as_path()) {
                if *expected == step && !*completed {
                    *completed = true;
                    self.processed += 1;
                }
            }
        }
        let changed = self.processed != previous;
        let first_confirmed = previous == 0 && self.processed > 0;
        let last_confirmed = changed && self.processed == self.total;
        let elapsed = self
            .last_emitted
            .is_none_or(|last| now.saturating_sub(last) >= std::time::Duration::from_millis(50));
        let emit = first_confirmed || last_confirmed || elapsed;
        if emit {
            self.last_emitted = Some(now);
        }
        FileProgressUpdate {
            processed: self.processed,
            total: self.total,
            changed,
            emit,
        }
    }
}

#[derive(Clone)]
pub(super) struct SealedPlan {
    pub owner: String,
    pub id: String,
    pub digest: String,
    pub plan: PlanDto,
    identities: BTreeMap<String, Identity>,
    created_at: u64,
    size_bytes: usize,
    library_root: PathBuf,
    pub(super) live: bool,
    pub(super) scope: BTreeSet<super::staging::StageKey>,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct Identity {
    canonical: PathBuf,
    size: u64,
    modified_nanos: u128,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    file_id: windows_identity::WindowsIdentity,
}
impl Identity {
    fn read(path: &Path) -> ServiceResult<Self> {
        let canonical = fs::canonicalize(path)?;
        #[cfg(windows)]
        let (metadata, file_id) = windows_identity::read(&canonical)?;
        #[cfg(not(windows))]
        let metadata = fs::metadata(path)?;
        Ok(Self {
            canonical,
            size: metadata.len(),
            modified_nanos: metadata
                .modified()?
                .duration_since(UNIX_EPOCH)
                .map(|t| t.as_nanos())
                .unwrap_or(0),
            #[cfg(unix)]
            device: {
                use std::os::unix::fs::MetadataExt;
                metadata.dev()
            },
            #[cfg(unix)]
            inode: {
                use std::os::unix::fs::MetadataExt;
                metadata.ino()
            },
            #[cfg(windows)]
            file_id,
        })
    }
}
#[derive(Default)]
struct PlanIntent {
    tags: HashMap<String, BTreeSet<String>>,
    all_tags: BTreeSet<String>,
    artwork: BTreeSet<String>,
    renames: BTreeSet<String>,
    blocks: HashMap<String, BTreeSet<String>>,
}
impl PlanIntent {
    fn tag(&mut self, path: &str, field: &str) {
        self.tags
            .entry(path.into())
            .or_default()
            .insert(tagrex_core::model::TagField::from_storage_key(field).to_storage_key());
    }
    fn clear_previous(&self, plan: &mut PlanDto) {
        for change in &mut plan.changes {
            if self.all_tags.contains(&change.path) {
                change.tag_changes.clear();
            } else if let Some(fields) = self.tags.get(&change.path) {
                change
                    .tag_changes
                    .retain(|field| !fields.contains(&field.field));
            }
            if self.artwork.contains(&change.path) {
                change.cover_change = None;
            }
            if self.renames.contains(&change.path) {
                change.rename_to = None;
                change.copy = false;
                change.rename_root = None;
                change.creates_folder = false;
                change.sidecar_renames.clear();
                change.folder_extras.clear();
            }
            if let Some(kinds) = self.blocks.get(&change.path) {
                change
                    .block_changes
                    .retain(|block| !kinds.contains(&block.kind));
            }
        }
    }
}
fn release_disc(track: &tagrex_commands::ReleaseTrackDto) -> Option<u32> {
    track.disc.filter(|disc| *disc > 0).or_else(|| {
        let (prefix, suffix) = track.position.trim().split_once(['.', '-'])?;
        if prefix.is_empty() || suffix.is_empty() || !suffix.as_bytes()[0].is_ascii_digit() {
            return None;
        }
        prefix.parse::<u32>().ok().filter(|disc| *disc > 0)
    })
}
fn release_track_totals(release: &ReleaseDto, indices: &[usize]) -> Vec<Option<usize>> {
    let single = release.disc_total.is_none_or(|discs| discs <= 1)
        && release
            .tracks
            .iter()
            .all(|track| release_disc(track).is_none());
    let disc =
        |track: &tagrex_commands::ReleaseTrackDto| release_disc(track).or(single.then_some(1));
    let mut counts = HashMap::new();
    for track in &release.tracks {
        if let Some(disc) = disc(track) {
            *counts.entry(disc).or_insert(0usize) += 1;
        }
    }
    indices
        .iter()
        .map(|index| {
            release
                .tracks
                .get(*index)
                .and_then(&disc)
                .and_then(|disc| counts.get(&disc).copied())
        })
        .collect()
}
fn check_locked_fields(app: &App, plan: &PlanDto) -> ServiceResult<()> {
    let locked = app.locked_fields().into_iter().collect::<BTreeSet<_>>();
    if plan.changes.iter().any(|change| {
        change
            .tag_changes
            .iter()
            .any(|field| locked.contains(&field.field))
            || change.block_changes.iter().any(|block| {
                block
                    .old
                    .iter()
                    .chain(&block.new)
                    .any(|content| content.tags.keys().any(|field| locked.contains(field)))
            })
    }) {
        return Err(ServiceError::new("field_locked","A field touched by this prepared plan is now locked; prepare it again respecting the current locks"));
    }
    Ok(())
}

impl Service {
    pub fn drafts(&self) -> ServiceResult<Value> {
        let state = lock(&self.0.state)?;
        Ok(
            json!({"drafts":state.plans.values().map(|p|json!({"id":p.id,"principal_id":p.owner,"description":p.plan.description,"files":p.plan.changes.len(),"digest":p.digest,"created_at":p.created_at})).collect::<Vec<_>>()}),
        )
    }
    pub fn cancel_operation(&self, id: &str) -> ServiceResult<Value> {
        let operation = lock(&self.0.state)?
            .operations
            .get(id)
            .cloned()
            .ok_or_else(missing_handle)?;
        operation.cancel.store(true, Ordering::Release);
        Ok(json!({"operation":id,"cancel_requested":true}))
    }
    pub(super) fn plan_call(
        &self,
        principal: &Principal,
        tool: &str,
        args: &Value,
    ) -> ServiceResult<Value> {
        match tool {
            "draft.get" => {
                let state = lock(&self.0.state)?;
                let plan = state
                    .plans
                    .get(required(args, "draft")?)
                    .filter(|p| p.owner == principal.id)
                    .ok_or_else(missing_handle)?;
                Ok(describe_live(
                    &state,
                    plan,
                    number(args, "offset", 0),
                    number(args, "limit", 100).clamp(1, MAX_PAGE),
                ))
            }
            "draft.discard" => {
                let mut state = lock(&self.0.state)?;
                let id = required(args, "draft")?;
                if !state.plans.get(id).is_some_and(|p| p.owner == principal.id) {
                    return Err(missing_handle());
                }
                state.plans.remove(id);
                drop(state);
                self.discard_stage_contribution(id)?;
                Ok(json!({"discarded":id}))
            }
            "plan.commit" => self.commit(principal, args, None),
            "files.write" => {
                self.grant(principal, "write")?;
                let mut intent = args.clone();
                intent
                    .as_object_mut()
                    .ok_or_else(|| ServiceError::invalid("Arguments must be an object"))?
                    .remove("idempotency_key");
                // A retry of the same direct intent is resolved before creating a
                // new snapshot/plan (which would otherwise change its digest).
                let key = required(args, "idempotency_key")?;
                let intent_digest = hash(&serde_json::to_vec(&intent)?);
                if let Some(existing) = self.find_operation(principal, key, &intent_digest)? {
                    return Ok(existing);
                }
                let mut created = self.create_plan(principal, "draft.create", args, false)?;
                created["idempotency_key"] = json!(key);
                self.commit(principal, &created, Some(&intent_digest))
            }
            "operations.status" => {
                let operations: Vec<_> =
                    lock(&self.0.state)?.operations.values().cloned().collect();
                let id = args.get("operation").and_then(Value::as_str);
                let mut records = Vec::new();
                for operation in operations {
                    let record = lock(&operation.record)?;
                    if record.owner == principal.id && id.is_none_or(|id| record.id == id) {
                        records.push(record.clone());
                    }
                }
                if id.is_some() && records.is_empty() {
                    return Err(missing_handle());
                }
                Ok(json!({"operations":records}))
            }
            "operations.cancel" => {
                let id = required(args, "operation")?;
                let operation = lock(&self.0.state)?
                    .operations
                    .get(id)
                    .cloned()
                    .ok_or_else(missing_handle)?;
                if lock(&operation.record)?.owner != principal.id {
                    return Err(missing_handle());
                }
                self.cancel_operation(id)
            }
            "history.list" => {
                self.grant(principal, "history")?;
                let app = lock(&self.0.app)?;
                let app = app.as_ref().ok_or_else(no_library)?;
                let state = lock(&self.0.state)?;
                let own_batches: BTreeSet<i64> = state
                    .operations
                    .values()
                    .filter_map(|o| {
                        let r = o.record.lock().ok()?;
                        (r.owner == principal.id)
                            .then(|| r.result.as_ref()?.get("batch")?.get("id")?.as_i64())
                            .flatten()
                    })
                    .collect();
                Ok(
                    json!({"batches":app.history()?.into_iter().filter(|b|own_batches.contains(&b.id)).collect::<Vec<_>>(),"redo_count":app.redo_count(),"redo_revision":app.redo_revision(),"redo_token":app.redo_token(),"redo_session_only":true}),
                )
            }
            "history.undo" | "history.redo" => self.history_mutation(principal, tool, args),
            _ => self.create_plan(principal, tool, args, true),
        }
    }
    fn create_plan(
        &self,
        principal: &Principal,
        tool: &str,
        args: &Value,
        publish: bool,
    ) -> ServiceResult<Value> {
        let grant = self.grant(principal, "stage")?;
        let paths = self.paths(principal, args, "stage")?;
        let previous = if let Some(id) = args.get("draft").and_then(Value::as_str) {
            Some(
                lock(&self.0.state)?
                    .plans
                    .get(id)
                    .filter(|p| p.owner == principal.id)
                    .cloned()
                    .ok_or_else(missing_handle)?,
            )
        } else {
            None
        };
        let mut previous = previous;
        if let Some(previous) = previous.as_mut() {
            if previous.live {
                previous.plan = self
                    .live_projection(&previous.scope)?
                    .unwrap_or_else(|| super::staging::filtered(&previous.plan, |_| false));
            }
        }
        let import_skip: BTreeSet<String> = if tool == "draft.import" {
            let backend = lock(&self.0.settings)?.clone();
            match backend {
                Some(settings) => {
                    let value = settings.read()?;
                    value
                        .get("import_skip_fields")
                        .map(|value| serde_json::from_value(value.clone()))
                        .transpose()?
                        .unwrap_or_default()
                }
                None => BTreeSet::new(),
            }
        } else {
            BTreeSet::new()
        };
        let mut intent = PlanIntent::default();
        let app_guard = lock(&self.0.app)?;
        let app = app_guard.as_ref().ok_or_else(no_library)?;
        // Membership validation is explicit even for legacy preview producers
        // which predate file-set sessions and read a supplied path directly.
        app.read_track_rows(&paths)?;
        let working = {
            let selected = paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<BTreeSet<_>>();
            let state = lock(&self.0.state)?;
            state.manual_plan.as_ref().map(|plan| {
                super::staging::filtered(plan, |key| selected.contains(key.path.as_ref()))
            })
        };
        let empty_working = app.preview_tag_edits(&[])?;
        let plan = match tool {
            "draft.create" => {
                let mut edits: Vec<TagEditDto> = args
                    .get("edits")
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()?
                    .unwrap_or_default();
                if let Some(values) = args.get("values").and_then(Value::as_object) {
                    for path in &paths {
                        for (field, value) in values {
                            edits.push(TagEditDto {
                                path: path.to_string_lossy().into_owned(),
                                field: field.clone(),
                                value: if value.is_null() {
                                    None
                                } else {
                                    Some(
                                        value
                                            .as_str()
                                            .ok_or_else(|| {
                                                ServiceError::invalid(
                                                    "Tag values must be strings or null",
                                                )
                                            })?
                                            .into(),
                                    )
                                },
                            });
                        }
                    }
                }
                let scope: BTreeSet<_> = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if edits.iter().any(|e| !scope.contains(&e.path)) {
                    return Err(ServiceError::invalid(
                        "Every edit must be in the explicit selection",
                    ));
                }
                for edit in &edits {
                    intent.tag(&edit.path, &edit.field);
                }
                app.preview_tag_edits(&edits)?
            }
            "draft.transform" => {
                let groups: Vec<ActionGroupDto> = if let Some(groups) = args.get("groups") {
                    serde_json::from_value(groups.clone())?
                } else {
                    let rules: Vec<TransformRuleDto> =
                        serde_json::from_value(args.get("rules").cloned().ok_or_else(|| {
                            ServiceError::invalid("rules or groups is required")
                        })?)?;
                    vec![ActionGroupDto {
                        name: "Agent transformations".into(),
                        scope: args
                            .get("scope")
                            .and_then(Value::as_str)
                            .unwrap_or("tags")
                            .into(),
                        rules,
                        note: String::new(),
                    }]
                };
                let mut plan = if let Some(previous) = &previous {
                    app.preview_transform_over_plan(&previous.plan, &groups)?
                } else {
                    let saved = app.preview_transform_groups(&paths, &groups)?;
                    if let Some(working) = &working {
                        let working = super::staging::filtered(working, |key| {
                            groups.iter().any(|group| {
                                if let Some(field) = key.field.strip_prefix("tag:") {
                                    group.scope == "tags" || group.scope == field
                                } else {
                                    key.field == "filename"
                                        && (group.scope == "filename" || group.scope == "extension")
                                }
                            })
                        });
                        let keys = super::staging::atoms(&working)?
                            .into_keys()
                            .collect::<BTreeSet<_>>();
                        for key in &keys {
                            if let Some(field) = key.field.strip_prefix("tag:") {
                                intent.tag(&key.path, field);
                            } else if key.field == "filename" {
                                intent.renames.insert(key.path.to_string());
                            }
                        }
                        let current = app.preview_transform_over_plan(&working, &groups)?;
                        merge(
                            super::staging::filtered(&saved, |key| !keys.contains(key)),
                            current,
                        )?
                    } else {
                        saved
                    }
                };
                plan.description = format!(
                    "Transform: {}",
                    groups
                        .iter()
                        .map(|group| group.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                plan.message = None;
                plan
            }
            "draft.rename" => {
                intent
                    .renames
                    .extend(paths.iter().map(|p| p.to_string_lossy().into_owned()));
                app.preview_rename_over_plan(
                    required(args, "mask")?,
                    &paths,
                    working.as_ref().unwrap_or(&empty_working),
                )?
            }
            "draft.move" => {
                intent
                    .renames
                    .extend(paths.iter().map(|p| p.to_string_lossy().into_owned()));
                let destination = args
                    .get("destination")
                    .and_then(Value::as_str)
                    .map(Path::new);
                if let Some(destination) = destination {
                    permitted(&grant, destination, false)?;
                }
                app.preview_move_over_plan(
                    required(args, "mask")?,
                    &paths,
                    destination,
                    flag(args, "copy"),
                    false,
                    working.as_ref().unwrap_or(&empty_working),
                )?
            }
            "draft.from_name" => {
                let mask = required(args, "mask")?;
                let probes = app.probe_tags_from_names(mask, &paths)?;
                for (path, probe) in paths.iter().zip(probes.files) {
                    if probe.matched {
                        for (field, value) in probe.fields {
                            if !value.is_empty() {
                                intent.tag(&path.to_string_lossy(), &field);
                            }
                        }
                    }
                }
                app.preview_tags_from_name_over_plan(
                    mask,
                    &paths,
                    working.as_ref().unwrap_or(&empty_working),
                )?
            }
            "draft.from_text" => {
                let mask = required(args, "mask")?;
                let text = required(args, "text")?;
                let probe = app.probe_text_lines(mask, text, &paths)?;
                for line in probe.lines {
                    if let Some(path) = line.path {
                        for (field, value) in line.fields {
                            if !value.is_empty() {
                                intent.tag(&path, &field);
                            }
                        }
                    }
                }
                app.preview_tags_from_text(mask, text, &paths)?
            }
            "draft.clear_tags" => {
                intent
                    .all_tags
                    .extend(paths.iter().map(|p| p.to_string_lossy().into_owned()));
                app.preview_clear_tags(&paths)?
            }
            "draft.remove_block" => {
                for path in &paths {
                    intent
                        .blocks
                        .entry(path.to_string_lossy().into_owned())
                        .or_default()
                        .insert(required(args, "kind")?.into());
                }
                app.preview_remove_tag_block(&paths, required(args, "kind")?)?
            }
            "draft.convert_block" => {
                for path in &paths {
                    intent
                        .blocks
                        .entry(path.to_string_lossy().into_owned())
                        .or_default()
                        .extend([required(args, "from")?.into(), required(args, "to")?.into()]);
                }
                app.preview_convert_tag_block(
                    &paths,
                    required(args, "from")?,
                    required(args, "to")?,
                    args.get("revision").and_then(Value::as_str),
                )?
            }
            "draft.fix_encoding" => app.preview_fix_encoding(&paths)?,
            "draft.number" => {
                let start = number(args, "start", 1);
                let step = number(args, "step", 1);
                let total = number(args, "total", paths.len());
                let width = number(args, "width", 0).min(12);
                if start > u32::MAX as usize || step > u32::MAX as usize {
                    return Err(ServiceError::invalid(
                        "Numbering values exceed supported range",
                    ));
                }
                let mut edits = Vec::new();
                for (index, path) in paths.iter().enumerate() {
                    let number = start
                        .checked_add(
                            index
                                .checked_mul(step)
                                .ok_or_else(|| ServiceError::invalid("Numbering overflow"))?,
                        )
                        .ok_or_else(|| ServiceError::invalid("Numbering overflow"))?;
                    edits.push(TagEditDto {
                        path: path.to_string_lossy().into_owned(),
                        field: "track".into(),
                        value: Some(format!("{number:0width$}")),
                    });
                    if flag(args, "write_total") {
                        edits.push(TagEditDto {
                            path: path.to_string_lossy().into_owned(),
                            field: "tracktotal".into(),
                            value: Some(total.to_string()),
                        });
                    }
                    if let Some(disc) = args.get("disc").and_then(Value::as_u64) {
                        edits.push(TagEditDto {
                            path: path.to_string_lossy().into_owned(),
                            field: "disc".into(),
                            value: Some(disc.to_string()),
                        });
                    }
                    if let Some(total) = args.get("disc_total").and_then(Value::as_u64) {
                        edits.push(TagEditDto {
                            path: path.to_string_lossy().into_owned(),
                            field: "disctotal".into(),
                            value: Some(total.to_string()),
                        });
                    }
                }
                for edit in &edits {
                    intent.tag(&edit.path, &edit.field);
                }
                app.preview_tag_edits(&edits)?
            }
            "draft.import" => {
                let mut per_file_totals = None;
                let selection: ImportSelectionDto = if let Some(receipt) =
                    args.get("release").and_then(Value::as_str)
                {
                    let state = lock(&self.0.state)?;
                    let receipt = state
                        .releases
                        .get(receipt)
                        .filter(|r| r.owner == principal.id)
                        .ok_or_else(missing_handle)?;
                    let release = &receipt.release;
                    let indices: Vec<usize> = args
                        .get("track_indices")
                        .map(|v| serde_json::from_value(v.clone()))
                        .transpose()?
                        .unwrap_or_else(|| (0..paths.len()).collect());
                    if indices.len() != paths.len() {
                        return Err(ServiceError::invalid(
                            "Provide one release-track index per selected file, in selection order",
                        ));
                    }
                    per_file_totals = Some(release_track_totals(release, &indices));
                    let tracks = indices
                        .iter()
                        .map(|index| {
                            let t = release.tracks.get(*index).ok_or_else(|| {
                                ServiceError::invalid("Release track index outside fetched release")
                            })?;
                            Ok(tagrex_commands::ImportTrackDto {
                                position: t.position.clone(),
                                disc: t.disc,
                                artist: t.artist.clone().unwrap_or_else(|| release.artist.clone()),
                                title: t.title.clone(),
                                duration_secs: t.duration_secs,
                                isrc: t.isrc.clone(),
                                bpm: t.bpm,
                                key: t.key.clone(),
                            })
                        })
                        .collect::<ServiceResult<Vec<_>>>()?;
                    ImportSelectionDto {
                        album: Some(release.title.clone()),
                        album_artist: Some(release.artist.clone()),
                        year: release.year.map(|v| v.to_string()),
                        genre: Some(if release.styles.is_empty() {
                            release.genres.join("; ")
                        } else {
                            release.styles.join("; ")
                        }),
                        tracks,
                        release_id: Some(release.id.clone()),
                        source: Some(receipt.source.clone()),
                        label: release.labels.first().map(|l| l.name.clone()),
                        catalog_number: release
                            .labels
                            .first()
                            .and_then(|l| l.catalog_number.clone()),
                        country: release.country.clone(),
                        track_total: None,
                        url: release.url.clone(),
                        media_type: release.format.clone(),
                        disc_total: release.disc_total.map(|d| d.to_string()),
                    }
                } else {
                    serde_json::from_value(args.get("import").cloned().ok_or_else(||ServiceError::invalid("A fetched release handle with explicit track mapping, or typed import values, is required"))?)?
                };
                for (index, path) in paths.iter().enumerate() {
                    let path = path.to_string_lossy();
                    let release_key = match selection.source.as_deref() {
                        Some("musicbrainz") => "custom:MUSICBRAINZ_ALBUMID",
                        Some("beatport") => "custom:BEATPORT_RELEASE_ID",
                        Some("bandcamp") => "custom:BANDCAMP_ALBUM_URL",
                        Some("soundeo") => "custom:SOUNDEO_RELEASE_URL",
                        _ => "custom:DISCOGS_RELEASE_ID",
                    };
                    for (field, value) in [
                        ("album", selection.album.as_ref()),
                        ("albumartist", selection.album_artist.as_ref()),
                        ("year", selection.year.as_ref()),
                        ("genre", selection.genre.as_ref()),
                        (release_key, selection.release_id.as_ref()),
                        ("publisher", selection.label.as_ref()),
                        ("catalognumber", selection.catalog_number.as_ref()),
                        ("custom:RELEASECOUNTRY", selection.country.as_ref()),
                        ("url", selection.url.as_ref()),
                        ("media", selection.media_type.as_ref()),
                        ("tracktotal", selection.track_total.as_ref()),
                    ] {
                        if value.is_some_and(|v| !v.is_empty()) && !import_skip.contains(field) {
                            intent.tag(&path, field);
                        }
                    }
                    if let Some(track) = selection.tracks.get(index) {
                        for (field, present) in [
                            ("title", !track.title.is_empty()),
                            (
                                "artist",
                                !track.artist.is_empty()
                                    || selection
                                        .album_artist
                                        .as_ref()
                                        .is_some_and(|value| !value.is_empty()),
                            ),
                            ("track", !track.position.is_empty()),
                            ("disc", track.disc.is_some()),
                            ("disctotal", selection.disc_total.is_some()),
                            ("isrc", track.isrc.as_ref().is_some_and(|v| !v.is_empty())),
                            ("bpm", track.bpm.is_some()),
                            ("key", track.key.as_ref().is_some_and(|v| !v.is_empty())),
                        ] {
                            if present && !import_skip.contains(field) {
                                intent.tag(&path, field);
                            }
                        }
                    }
                }
                let mut plan =
                    app.preview_import(&paths, &selection, flag(args, "vinyl_sides_to_disc"))?;
                if let Some(totals) = per_file_totals {
                    for (index, total) in totals.into_iter().enumerate() {
                        if let Some(total) = total {
                            if import_skip.contains("tracktotal") {
                                continue;
                            }
                            let mut one = selection.clone();
                            one.tracks = selection.tracks.get(index).cloned().into_iter().collect();
                            one.track_total = Some(total.to_string());
                            let mut overlay = app.preview_import(
                                &paths[index..index + 1],
                                &one,
                                flag(args, "vinyl_sides_to_disc"),
                            )?;
                            for change in &mut overlay.changes {
                                change
                                    .tag_changes
                                    .retain(|field| field.field == "tracktotal");
                            }
                            prune_empty_changes(&mut overlay);
                            intent.tag(&paths[index].to_string_lossy(), "tracktotal");
                            plan = merge(plan, overlay)?;
                        }
                    }
                }
                plan
            }
            "draft.artwork" => {
                intent
                    .artwork
                    .extend(paths.iter().map(|path| path.to_string_lossy().into_owned()));
                self.preview_artwork(principal, args, &paths, app, working.as_ref())?
            }
            _ => return Err(ServiceError::new("unknown_tool", "Tool is not implemented")),
        };
        let library_root = fs::canonicalize(app.library_root())?;
        drop(app_guard);
        let parent = previous.as_ref().map(|previous| previous.id.clone());
        let mut scope = previous
            .as_ref()
            .map(|previous| previous.scope.clone())
            .unwrap_or_default();
        scope.extend(super::staging::atoms(&plan)?.into_keys());
        for (path, fields) in &intent.tags {
            for field in fields {
                scope.insert(super::staging::StageKey::tag(path, field));
            }
        }
        for path in &intent.artwork {
            scope.insert(super::staging::StageKey::special(path, "artwork"));
        }
        for path in &intent.renames {
            scope.insert(super::staging::StageKey::special(path, "filename"));
        }
        for (path, blocks) in &intent.blocks {
            for block in blocks {
                scope.insert(super::staging::StageKey::special(
                    path,
                    &format!("block:{block}"),
                ));
            }
        }
        for path in &intent.all_tags {
            for source in previous
                .as_ref()
                .map(|p| &p.plan)
                .into_iter()
                .chain(working.as_ref())
            {
                scope.extend(
                    super::staging::atoms(source)?
                        .into_keys()
                        .filter(|key| key.path.as_ref() == path && key.field.starts_with("tag:")),
                );
            }
        }
        let plan = if tool == "draft.transform" && previous.is_some() {
            plan
        } else if let Some(mut previous) = previous {
            intent.clear_previous(&mut previous.plan);
            merge(previous.plan, plan)?
        } else {
            plan
        };
        self.seal(
            principal,
            plan,
            library_root,
            scope,
            parent.as_deref(),
            publish,
        )
    }
    fn seal(
        &self,
        principal: &Principal,
        mut plan: PlanDto,
        library_root: PathBuf,
        scope: BTreeSet<super::staging::StageKey>,
        parent: Option<&str>,
        publish: bool,
    ) -> ServiceResult<Value> {
        let grant = self.grant(principal, "stage")?;
        validate_plan(&grant, &plan, false)?;
        let mut identities = BTreeMap::new();
        for change in &plan.changes {
            identities.insert(
                change.path.clone(),
                Identity::read(Path::new(&change.path))?,
            );
            for (from, _) in &change.sidecar_renames {
                identities.insert(from.clone(), Identity::read(Path::new(from))?);
            }
        }
        for key in &scope {
            if !identities.contains_key(key.path.as_ref()) {
                identities.insert(
                    key.path.to_string(),
                    Identity::read(Path::new(key.path.as_ref()))?,
                );
            }
        }
        let id = opaque("draft")?;
        plan.description = format!("{} · MCP draft {id}", plan.description);
        let scope_fields = scope
            .iter()
            .map(|key| json!({"path":key.path,"field":key.field}))
            .collect::<Vec<_>>();
        let digest = if publish {
            hash(&serde_json::to_vec(
                &json!({"intent":"apply_current_shared_stage","scope":scope_fields,"root":library_root,"principal":principal.id,"epoch":self.0.epoch}),
            )?)
        } else {
            hash(&serde_json::to_vec(
                &json!({"plan":plan,"identities":identities,"principal":principal.id,"epoch":self.0.epoch}),
            )?)
        };
        let size_bytes = serde_json::to_vec(&plan)?.len();
        let mut sealed = SealedPlan {
            owner: principal.id.clone(),
            id: id.clone(),
            digest: digest.clone(),
            plan,
            identities,
            created_at: now(),
            size_bytes,
            library_root,
            live: publish,
            scope,
        };
        let mut state = lock(&self.0.state)?;
        quota(state.plans.len())?;
        let retained_bytes: usize = state.plans.values().map(|p| p.size_bytes).sum();
        let new_bytes = sealed.size_bytes;
        if retained_bytes.saturating_add(new_bytes) > 128 * 1024 * 1024 {
            return Err(ServiceError::new(
                "quota",
                "Sealed draft budget is 128 MiB; discard unused drafts",
            ));
        }
        let excluded = if publish {
            Self::stage_agent_locked(
                &mut state,
                &id,
                parent,
                &sealed.plan,
                &sealed.scope,
                &principal.id,
            )?
        } else {
            Vec::new()
        };
        let mut result = if publish {
            describe_live(&state, &sealed, 0, 100)
        } else {
            describe(&sealed, 0, 100)
        };
        result["protected_fields"] = json!(excluded);
        let revision = state.manual_revision;
        if publish {
            sealed.plan = super::staging::filtered(&sealed.plan, |_| false);
            sealed.size_bytes = serde_json::to_vec(&sealed.plan)?.len();
        }
        state.plans.insert(id, sealed);
        drop(state);
        if publish {
            self.emit(
                "shared_stage_changed",
                json!({"revision":revision,"resync":true}),
            );
        }
        self.emit("service_changed", json!({"drafts_changed":true}));
        Ok(result)
    }
    fn find_operation(
        &self,
        principal: &Principal,
        key: &str,
        digest: &str,
    ) -> ServiceResult<Option<Value>> {
        if key.is_empty() || key.len() > 256 {
            return Err(ServiceError::invalid(
                "Idempotency key must have 1..256 bytes",
            ));
        }
        let key_hash = hash(format!("{}:{key}", principal.id).as_bytes());
        let state = lock(&self.0.state)?;
        for operation in state.operations.values() {
            let record = lock(&operation.record)?;
            if record.owner == principal.id && record.key_hash == key_hash {
                if record.digest != digest {
                    return Err(ServiceError::conflict(
                        "Idempotency key was already used for different intent",
                    ));
                }
                return Ok(Some(
                    json!({"operation":record.id,"status":record.status,"reused":true}),
                ));
            }
        }
        Ok(None)
    }
    fn commit(
        &self,
        principal: &Principal,
        args: &Value,
        direct_intent_digest: Option<&str>,
    ) -> ServiceResult<Value> {
        let grant = self.grant(principal, "write")?;
        let plan_id = required(args, "draft")?;
        let expected = args
            .get("scope_digest")
            .and_then(Value::as_str)
            .or_else(|| args.get("digest").and_then(Value::as_str))
            .ok_or_else(|| ServiceError::invalid("scope_digest is required"))?;
        let key = required(args, "idempotency_key")?;
        let intent_digest = direct_intent_digest.unwrap_or(expected);
        if let Some(existing) = self.find_operation(principal, key, intent_digest)? {
            return Ok(existing);
        }
        if lock(&self.0.state)?.recovery_required {
            return Err(ServiceError::new(
                "recovery_required",
                "Resolve the interrupted operation before preparing a new write",
            ));
        }
        let current_locks = match self.0.app.try_lock() {
            Ok(app) => app.as_ref().map(|app| app.locked_fields()),
            Err(std::sync::TryLockError::WouldBlock) => None,
            Err(_) => {
                return Err(ServiceError::new(
                    "service_unavailable",
                    "Application session lock is poisoned",
                ))
            }
        };
        let mut state = lock(&self.0.state)?;
        // Atomic acceptance: a simultaneous retry can create a different
        // temporary plan but must reserve exactly one caller intent.
        let key_hash = hash(format!("{}:{key}", principal.id).as_bytes());
        for operation in state.operations.values() {
            let record = lock(&operation.record)?;
            if record.owner == principal.id && record.key_hash == key_hash {
                if record.digest != intent_digest {
                    return Err(ServiceError::conflict(
                        "Idempotency key was already used for different intent",
                    ));
                }
                return Ok(json!({"operation":record.id,"status":record.status,"reused":true}));
            }
        }
        if state.operations.len() >= 10_000 {
            return Err(ServiceError::new("quota","Durable operation receipt budget is full; retain recovery evidence and use trusted maintenance"));
        }
        if state.recovery_required {
            return Err(ServiceError::new("recovery_required","Resolve the interrupted service operation against journal evidence before new writes"));
        }
        let plan = state
            .plans
            .get(plan_id)
            .filter(|p| p.owner == principal.id)
            .cloned()
            .ok_or_else(missing_handle)?;
        if !constant_eq(plan.digest.as_bytes(), expected.as_bytes()) {
            return Err(ServiceError::new(
                "digest_mismatch",
                "Prepared plan digest does not match",
            ));
        }
        if now().saturating_sub(plan.created_at) > 24 * 60 * 60 {
            return Err(ServiceError::new(
                "expired_plan",
                "Prepare a new plan; this plan's 24-hour write window expired",
            ));
        }
        let preview = if plan.live {
            state
                .manual_plan
                .as_ref()
                .map(|current| super::staging::filtered(current, |key| plan.scope.contains(key)))
                .unwrap_or_else(|| super::staging::filtered(&plan.plan, |_| false))
        } else {
            plan.plan.clone()
        };
        validate_plan(&grant, &preview, true)?;
        if let Some(locks) = current_locks {
            let locked = locks.into_iter().collect::<BTreeSet<_>>();
            if preview.changes.iter().any(|change| {
                change
                    .tag_changes
                    .iter()
                    .any(|field| locked.contains(&field.field))
                    || change.block_changes.iter().any(|block| {
                        block
                            .old
                            .iter()
                            .chain(&block.new)
                            .any(|content| content.tags.keys().any(|field| locked.contains(field)))
                    })
            }) {
                return Err(ServiceError::new(
                    "field_locked",
                    "A prepared field is now locked; prepare the draft again",
                ));
            }
        }
        let running = state
            .operations
            .values()
            .filter(|operation| {
                operation.record.lock().is_ok_and(|record| {
                    matches!(
                        record.status.as_str(),
                        "queued" | "preflight" | "writing" | "rolling_back"
                    )
                })
            })
            .count();
        if running >= 8 {
            return Err(ServiceError::new(
                "busy",
                "Mutation queue is full; inspect existing operations before retrying",
            ));
        }
        let id = opaque("operation")?;
        let operation = Arc::new(Operation {
            record: Mutex::new(OperationRecord {
                id: id.clone(),
                owner: principal.id.clone(),
                key_hash: hash(format!("{}:{key}", principal.id).as_bytes()),
                digest: intent_digest.into(),
                plan_id: opaque("accepted_write")?,
                status: "queued".into(),
                processed: 0,
                total: preview.changes.len(),
                result: None,
                error: None,
                interrupted_phase: None,
                library_root: Some(plan.library_root.clone()),
                accepted_digest: None,
                scope_digest: plan.live.then(|| plan.digest.clone()),
                scope_id: Some(plan_id.into()),
            }),
            cancel: AtomicBool::new(false),
        });
        state.operations.insert(id.clone(), operation.clone());
        persist_operations(&self.0.directory, &state)?;
        drop(state);
        self.emit(
            "operation_changed",
            json!({"operation":id,"status":"queued"}),
        );
        let service = self.clone();
        let principal = principal.clone();
        let worker_operation = operation.clone();
        #[cfg(test)]
        let refuse_spawn = self.0.fail_next_spawn.swap(false, Ordering::AcqRel);
        #[cfg(not(test))]
        let refuse_spawn = false;
        let spawned = if refuse_spawn {
            Err(std::io::Error::other("injected spawn failure"))
        } else {
            std::thread::Builder::new()
                .name("tagnetic-agent-write".into())
                .spawn(move || {
                    let completed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        service.execute(principal, plan, worker_operation.clone())
                    }));
                    if completed.is_err() {
                        service.worker_failed(&worker_operation, true);
                    }
                })
        };
        if spawned.is_err() {
            self.worker_failed(&operation, false);
            return Ok(json!({"operation":id,"status":"failed_before_write"}));
        }
        Ok(json!({"operation":id,"status":"queued"}))
    }
    fn worker_failed(&self, operation: &Operation, uncertain: bool) {
        // This path also handles poisoned guards after unwinding. It never
        // claims rollback or replays the accepted intent automatically.
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        {
            let mut record = operation.record.lock().unwrap_or_else(|p| p.into_inner());
            record.interrupted_phase = Some(record.status.clone());
            record.status = if uncertain {
                "recovery_required"
            } else {
                "failed_before_write"
            }
            .into();
            record.error = Some(ServiceError::new(
                if uncertain {
                    "worker_panicked"
                } else {
                    "worker_unavailable"
                },
                if uncertain {
                    "The write worker panicked; inspect the journal and affected files before new writes"
                } else {
                    "The worker could not start; no files were written"
                },
            ));
        }
        if uncertain {
            state.recovery_required = true;
        }
        if persist_operations(&self.0.directory, &state).is_err() {
            state.recovery_required = true;
            let mut record = operation.record.lock().unwrap_or_else(|p| p.into_inner());
            record.status = "recovery_required".into();
        }
        drop(state);
        let mut terminal=operation.record.lock().ok().map(|record|json!({"completed":true,"operation":record.id,"status":record.status,"processed":record.processed,"total":record.total,"recovery_required":record.status=="recovery_required"})).unwrap_or_else(||json!({"completed":true,"recovery_required":true}));
        terminal["worker_failed"] = json!(true);
        self.emit("operation_changed", terminal);
    }
    fn execute(&self, principal: Principal, plan: SealedPlan, operation: Arc<Operation>) {
        let mutation_permit = lock(&self.0.mutation);
        let result = (|| -> ServiceResult<Value> {
            if let Err(error) = &mutation_permit {
                return Err(error.clone());
            }
            if lock(&self.0.state)?.recovery_required {
                return Err(ServiceError::new(
                    "recovery_required",
                    "A prior operation requires recovery before queued writes may start",
                ));
            }
            self.grant(&principal, "write")?;
            if operation.cancel.load(Ordering::Acquire) {
                return Err(ServiceError::new("cancelled", "Cancelled before mutation"));
            }
            {
                let mut record = lock(&operation.record)?;
                record.status = "preflight".into();
            }
            let grant = self.grant(&principal, "write")?;
            let mut app_guard = lock(&self.0.app)?;
            let app = app_guard.as_mut().ok_or_else(no_library)?;
            let (descriptor, mut accepted, manual, manual_revision) = {
                let state = lock(&self.0.state)?;
                let descriptor = if plan.live {
                    state
                        .plans
                        .get(&plan.id)
                        .filter(|p| p.owner == principal.id)
                        .cloned()
                        .ok_or_else(missing_handle)?
                } else {
                    plan.clone()
                };
                let accepted = if descriptor.live {
                    state
                        .manual_plan
                        .as_ref()
                        .map(|current| {
                            super::staging::filtered(current, |key| descriptor.scope.contains(key))
                        })
                        .unwrap_or_else(|| super::staging::filtered(&descriptor.plan, |_| false))
                } else {
                    descriptor.plan.clone()
                };
                (
                    descriptor,
                    accepted,
                    state.manual_plan.clone(),
                    state.manual_revision,
                )
            };
            validate_plan(&grant, &accepted, true)?;
            check_locked_fields(app, &accepted)?;
            for change in &accepted.changes {
                for path in std::iter::once(change.path.as_str())
                    .chain(change.sidecar_renames.iter().map(|(from, _)| from.as_str()))
                {
                    let expected=descriptor.identities.get(path).ok_or_else(||ServiceError::new("scope_changed","Current effects require an identity outside the original authorized scope"))?;
                    if &Identity::read(Path::new(path))? != expected {
                        return Err(ServiceError::new("stale_file","An authorized file changed or was replaced outside a known application write"));
                    }
                }
            }
            let stable_paths = accepted
                .changes
                .iter()
                .all(|change| change.rename_to.is_none() && change.sidecar_renames.is_empty());
            let applied_atoms = super::staging::atoms(&accepted)?;
            if !stable_paths {
                let remaining = manual.as_ref().map(|current| {
                    super::staging::filtered(current, |key| !applied_atoms.contains_key(key))
                });
                if accepted
                    .changes
                    .iter()
                    .filter(|change| change.rename_to.is_some() && !change.copy)
                    .any(|moved| {
                        remaining.as_ref().is_some_and(|other| {
                            other.changes.iter().any(|change| change.path == moved.path)
                        })
                    })
                {
                    return Err(ServiceError::conflict("This move would orphan other pending fields on the same file; include those fields in the shared scope or apply them first"));
                }
            }
            let (accepted_digest, accepted_id) = {
                let record = lock(&operation.record)?;
                (
                    hash(&serde_json::to_vec(
                        &json!({"scope_digest":descriptor.digest,"plan":accepted,"identities":descriptor.identities}),
                    )?),
                    record.plan_id.clone(),
                )
            };
            accepted.description = format!(
                "{} · MCP draft {}",
                accepted
                    .description
                    .split(" · MCP draft ")
                    .next()
                    .unwrap_or("Staged edits"),
                accepted_id
            );
            {
                let mut record = lock(&operation.record)?;
                record.accepted_digest = Some(accepted_digest.clone());
                record.total = accepted.changes.len();
            }
            if accepted.changes.is_empty() {
                return Ok(json!({"files":0,"no_changes":true,"accepted_digest":accepted_digest}));
            }
            {
                let mut record = lock(&operation.record)?;
                record.status = "writing".into();
            }
            self.persist_operations()?;
            let mut files = FileProgress::new(&accepted);
            let started = std::time::Instant::now();
            let operation_id = lock(&operation.record)?.id.clone();
            let mut progress = |progress: &tagrex_core::plan::ApplyProgress| {
                if self.write_is_authorized(&principal).is_err() {
                    operation.cancel.store(true, Ordering::Release);
                }
                let update = files.observe(progress, started.elapsed());
                if update.changed {
                    if let Ok(mut record) = operation.record.lock() {
                        record.processed = update.processed;
                    }
                }
                if update.emit {
                    self.emit("operation_changed",json!({"operation":operation_id,"processed":update.processed,"total":update.total}));
                }
            };
            let (batch, tracks) = if stable_paths {
                let written = app.with_write_scope(&grant.roots, |app| {
                    app.quick_write_with(&accepted, None, &mut progress, &operation.cancel)
                })?;
                #[cfg(test)]
                if self.0.panic_after_apply.swap(false, Ordering::AcqRel) {
                    panic!("injected panic after native apply before receipt");
                }
                (written.batch, Some(written.tracks))
            } else {
                (
                    app.with_write_scope(&grant.roots, |app| {
                        app.apply_with(&accepted, &mut progress, &operation.cancel)
                    })?,
                    None,
                )
            };
            let written = accepted
                .changes
                .iter()
                .map(|change| PathBuf::from(&change.path))
                .collect::<Vec<_>>();
            let applied_atoms =
                super::staging::atoms(&super::staging::filtered(&accepted, |key| {
                    !accepted.changes.iter().any(|change| {
                        change.path == key.path.as_ref()
                            && change.tag_changes.iter().any(|field| {
                                key.field == format!("tag:{}", field.field) && field.invalid
                            })
                    })
                }))?;
            let before_completion = lock(&self.0.state)?.manual_revision;
            let completion =
                self.complete_shared_apply_with_app(app, &applied_atoms, &written, manual_revision);
            if let Err(error) = &completion {
                self.emit(
                    "shared_stage_refresh_failed",
                    json!({"message":error.message,"saved":true}),
                );
            }
            lock(&self.0.state)?.redo_revision += 1;
            if let Some(tracks) = tracks {
                self.emit("write_completed",json!({"tracks":tracks,"manual_revision":manual_revision,"manual_rebased":false,"manual_changed":before_completion!=manual_revision,"shared_staging":true,"current_manual_revision":lock(&self.0.state)?.manual_revision}));
            } else {
                self.emit("session_changed", json!({"refresh":true}));
            }
            Ok(
                json!({"batch":batch,"files":accepted.changes.len(),"accepted_digest":accepted_digest}),
            )
        })();
        let mut requires_recovery = false;
        if let Ok(mut record) = operation.record.lock() {
            match result {
                Ok(value) => {
                    record.status = "succeeded".into();
                    record.processed = record.total;
                    record.result = Some(value);
                }
                Err(error) => {
                    let message = error.message.to_lowercase();
                    let cancelled =
                        error.code == "cancelled" || error.code == "cancelled_and_restored";
                    requires_recovery = error.code == "recovery_required"
                        || (message.contains("rollback of batch") && message.contains("failed"));
                    if requires_recovery {
                        record.interrupted_phase = Some(record.status.clone());
                    }
                    record.status = if requires_recovery {
                        "recovery_required"
                    } else if cancelled {
                        "cancelled_and_restored"
                    } else {
                        "failed"
                    }
                    .into();
                    record.error = Some(error);
                }
            }
        }
        // Never acquire state while holding an operation-record guard: durable
        // persistence deliberately takes those locks in the opposite direction.
        if requires_recovery {
            if let Ok(mut state) = self.0.state.lock() {
                state.recovery_required = true;
            }
        }
        if let Err(error) = self.persist_operations() {
            if let Ok(mut record) = operation.record.lock() {
                record.status = "recovery_required".into();
                record.error = Some(error);
            }
            if let Ok(mut state) = self.0.state.lock() {
                state.recovery_required = true;
            }
        }
        let terminal=operation.record.lock().ok().map(|record|json!({"completed":true,"operation":record.id,"status":record.status,"processed":record.processed,"total":record.total})).unwrap_or_else(||json!({"completed":true}));
        self.emit("operation_changed", terminal);
    }
    fn write_is_authorized(&self, principal: &Principal) -> ServiceResult<()> {
        let state = lock(&self.0.state)?;
        if !state.security.enabled
            || !state.security.clients.iter().any(|client| {
                client.id == principal.id
                    && client.epoch == principal.epoch
                    && !client.revoked
                    && !expired(&client.grant)
                    && client.grant.actions.contains("write")
            })
        {
            return Err(denied());
        }
        Ok(())
    }
    fn persist_operations(&self) -> ServiceResult<()> {
        persist_operations(&self.0.directory, &*lock(&self.0.state)?)
    }
    fn history_mutation(
        &self,
        principal: &Principal,
        tool: &str,
        args: &Value,
    ) -> ServiceResult<Value> {
        self.grant(principal, "history")?;
        let grant = self.grant(principal, "write")?;
        let operation_id = required(args, "operation")?;
        let operation = lock(&self.0.state)?
            .operations
            .get(operation_id)
            .cloned()
            .ok_or_else(missing_handle)?;
        let (owner, batch, plan_id, undone, redo_token) = {
            let record = lock(&operation.record)?;
            (
                record.owner.clone(),
                record
                    .result
                    .as_ref()
                    .and_then(|v| v.get("batch"))
                    .and_then(|v| v.get("id"))
                    .and_then(Value::as_i64),
                record.plan_id.clone(),
                record
                    .result
                    .as_ref()
                    .and_then(|v| v.get("undone"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                record
                    .result
                    .as_ref()
                    .and_then(|v| v.get("redo_token"))
                    .and_then(Value::as_str)
                    .map(String::from),
            )
        };
        if owner != principal.id {
            return Err(denied());
        }
        let batch =
            batch.ok_or_else(|| ServiceError::invalid("Operation has no committed batch"))?;
        let _mutation = lock(&self.0.mutation)?;
        if lock(&self.0.state)?.recovery_required {
            return Err(ServiceError::new(
                "recovery_required",
                "Use the trusted recovery panel before changing history",
            ));
        }
        self.grant(principal, "write")?;
        let mut app_guard = lock(&self.0.app)?;
        let app = app_guard.as_mut().ok_or_else(no_library)?;
        let mut state = lock(&self.0.state)?;
        if state
            .manual_plan
            .as_ref()
            .is_some_and(|p| !p.changes.is_empty())
        {
            return Err(ServiceError::conflict(
                "Finish the human's staged plan before changing history",
            ));
        }
        let expected = args.get("redo_revision").and_then(Value::as_u64);
        if tool == "history.redo"
            && (expected != Some(app.redo_revision())
                || !undone
                || redo_token.is_none()
                || redo_token != app.redo_token())
        {
            return Err(ServiceError::conflict(
                "This operation is no longer the current top Redo entry; inspect history again",
            ));
        }
        if tool == "history.redo" {
            for path in app.redo_scope() {
                destination(&grant, &path)?;
            }
        } else if let Some(plan) = state.plans.get(&plan_id) {
            for change in &plan.plan.changes {
                destination(&grant, Path::new(&change.path))?;
                if let Some(target) = &change.rename_to {
                    destination(&grant, Path::new(target))?;
                }
            }
        } else {
            let (damaged, batches) = app.recovery_batches(&plan_id)?;
            let evidence = batches.iter().find(|b| b.id == batch).ok_or_else(|| {
                ServiceError::new(
                    "expired_plan",
                    "The native journal cannot prove this receipt's exact batch",
                )
            })?;
            if damaged {
                return Err(ServiceError::new(
                    "recovery_required",
                    "Inspect the replaced journal before Undo",
                ));
            }
            for path in &evidence.paths {
                destination(&grant, path)?;
            }
        }
        if tool == "history.undo" && undone {
            return Ok(json!({"undone":batch,"reused":true}));
        }
        {
            let mut record = lock(&operation.record)?;
            record.status = if tool == "history.undo" {
                "undoing"
            } else {
                "redoing"
            }
            .into();
        }
        persist_operations(&self.0.directory, &state)?;
        let outcome: ServiceResult<Value> = if tool == "history.undo" {
            app.with_write_scope(&grant.roots, |app| app.undo(batch))
                .map(|report| json!({"undone":batch,"report":report}))
                .map_err(ServiceError::from)
        } else {
            app.with_write_scope(&grant.roots, |app| app.redo_with(&mut |_| {}))
                .map(|batch| json!({"batch":batch}))
                .map_err(ServiceError::from)
        };
        let result = match outcome {
            Ok(value) => value,
            Err(error) => {
                {
                    let mut record = lock(&operation.record)?;
                    let preflight = matches!(
                        error.code.as_str(),
                        "stale_file" | "permission_denied" | "collision"
                    );
                    if !preflight {
                        record.interrupted_phase = Some(record.status.clone());
                    }
                    record.status = if preflight {
                        "history_refused"
                    } else {
                        "recovery_required"
                    }
                    .into();
                    record.error = Some(error.clone());
                    if !preflight {
                        state.recovery_required = true;
                    }
                }
                persist_operations(&self.0.directory, &state)?;
                return Err(error);
            }
        };
        state.redo_revision += 1;
        {
            let mut record = lock(&operation.record)?;
            record.status = "succeeded".into();
            if let Some(saved) = record.result.as_mut() {
                saved["undone"] = json!(tool == "history.undo");
                saved["redo_token"] = json!(app.redo_token());
                if tool == "history.redo" {
                    if let Some(batch) = result.get("batch") {
                        saved["batch"] = batch.clone();
                    }
                }
            }
        }
        persist_operations(&self.0.directory, &state)?;
        drop(state);
        drop(app_guard);
        self.emit("session_changed", json!({"refresh":true}));
        Ok(result)
    }
}
fn persist_operations(directory: &Path, state: &State) -> ServiceResult<()> {
    let records = state
        .operations
        .values()
        .map(|op| lock(&op.record).map(|r| r.clone()))
        .collect::<ServiceResult<Vec<_>>>()?;
    write_json(&directory.join("operations.json"), &records)
}
fn validate_plan(grant: &Grant, plan: &PlanDto, mutation: bool) -> ServiceResult<()> {
    if plan.changes.len() > grant.max_files {
        return Err(ServiceError::new(
            "quota",
            "Plan exceeds granted file budget",
        ));
    }
    for change in &plan.changes {
        permitted(grant, Path::new(&change.path), mutation)?;
        if change.tag_changes.iter().any(|c| c.invalid) {
            return Err(ServiceError::invalid("Plan includes invalid tag values"));
        }
        if !grant.fields.is_empty() {
            for change in &change.tag_changes {
                if !grant.fields.contains(&change.field) {
                    return Err(denied());
                }
            }
            if change.cover_change.is_some() && !grant.fields.contains("artwork") {
                return Err(denied());
            }
            if change.rename_to.is_some() && !grant.fields.contains("filename") {
                return Err(denied());
            }
            if !change.block_changes.is_empty() {
                return Err(ServiceError::new(
                    "permission_denied",
                    "Whole tag-block changes require an unrestricted field grant",
                ));
            }
        }
        if mutation && !grant.write_files.is_empty() && change.rename_to.is_some() {
            return Err(ServiceError::new("permission_denied","Path-changing writes require a root workflow grant, not an exact-file metadata grant"));
        }
        if let Some(target) = &change.rename_to {
            destination(grant, Path::new(target))?;
        }
        for (source, target) in &change.sidecar_renames {
            permitted(grant, Path::new(source), mutation)?;
            destination(grant, Path::new(target))?;
        }
        if plan.prune_empty_dirs {
            return Err(ServiceError::new(
                "permission_denied",
                "Directory pruning is not an implicit part of an agent move",
            ));
        }
    }
    Ok(())
}
pub(super) fn describe_live(
    state: &State,
    scope: &SealedPlan,
    offset: usize,
    limit: usize,
) -> Value {
    if !scope.live {
        return describe(scope, offset, limit);
    }
    let mut current = scope.clone();
    current.plan = state
        .manual_plan
        .as_ref()
        .map(|plan| super::staging::filtered(plan, |key| scope.scope.contains(key)))
        .unwrap_or_else(|| super::staging::filtered(&scope.plan, |_| false));
    current.plan.description = scope.plan.description.clone();
    let mut result = describe(&current, offset, limit);
    result["scope_digest"] = json!(scope.digest);
    result["live_revision"] = json!(state.manual_revision);
    result["live"] = json!(true);
    result
}
impl Service {
    pub(super) fn refresh_live_identities(&self, paths: &[PathBuf]) -> ServiceResult<()> {
        let identities = paths
            .iter()
            .filter(|path| path.exists())
            .map(|path| {
                Identity::read(path).map(|identity| (path.to_string_lossy().into_owned(), identity))
            })
            .collect::<ServiceResult<BTreeMap<_, _>>>()?;
        let mut state = lock(&self.0.state)?;
        for plan in state.plans.values_mut().filter(|plan| plan.live) {
            for (path, identity) in &identities {
                if plan.identities.contains_key(path) {
                    plan.identities.insert(path.clone(), identity.clone());
                }
            }
        }
        Ok(())
    }
}

pub(super) fn describe(plan: &SealedPlan, offset: usize, limit: usize) -> Value {
    let rows:Vec<_>=plan.plan.changes.iter().skip(offset).take(limit).map(|c|json!({"path":c.path,"rename_to":c.rename_to,"copy":c.copy,"tags":c.tag_changes,"artwork":c.cover_change.as_ref().map(|v|super::artwork_policy::artwork_diff(v,&plan.plan.cover_assets)),"blocks":c.block_changes.iter().map(|b|json!({"kind":b.kind,"lost_fields":b.lost_fields,"lost_pictures":b.lost_pictures,"exact":b.exact})).collect::<Vec<_>>(),"sidecar_renames":c.sidecar_renames})).collect();
    json!({"draft":plan.id,"digest":plan.digest,"description":plan.plan.description,"files":plan.plan.changes.len(),"changes":rows,"offset":offset,"next_offset":(offset+rows.len()<plan.plan.changes.len()).then_some(offset+rows.len()),"locked_skipped":plan.plan.locked_skipped,"unrendered":plan.plan.unrendered})
}
pub(super) fn merge(mut previous: PlanDto, mut next: PlanDto) -> ServiceResult<PlanDto> {
    // Keep shared image buffers shared: JSON roundtrips copied every image and
    // linear row lookup made successive large stages quadratic.
    let mut assets = previous
        .cover_assets
        .iter()
        .enumerate()
        .map(|(i, asset)| (asset.clone(), i))
        .collect::<HashMap<_, _>>();
    let mut remap = Vec::with_capacity(next.cover_assets.len());
    for asset in &next.cover_assets {
        let key = asset.clone();
        let index = *assets.entry(key).or_insert_with(|| {
            previous.cover_assets.push(asset.clone());
            previous.cover_assets.len() - 1
        });
        remap.push(index);
    }
    for change in &mut next.changes {
        let update = |cover: &mut tagrex_commands::CoverArtDto| -> ServiceResult<()> {
            if let Some(index) = cover.data_ref {
                cover.data_ref = Some(
                    *remap
                        .get(index)
                        .ok_or_else(|| ServiceError::invalid("Invalid artwork reference"))?,
                );
            }
            Ok(())
        };
        if let Some(cover) = &mut change.cover_change {
            for cover in cover.old.iter_mut().chain(&mut cover.new) {
                update(cover)?;
            }
        }
        for block in &mut change.block_changes {
            for content in block.old.iter_mut().chain(&mut block.new) {
                for cover in &mut content.covers {
                    update(cover)?;
                }
            }
        }
    }
    let mut positions = previous
        .changes
        .iter()
        .enumerate()
        .map(|(i, change)| (change.path.clone(), i))
        .collect::<HashMap<_, _>>();
    for incoming in next.changes {
        if let Some(index) = positions.get(&incoming.path).copied() {
            let current = &mut previous.changes[index];
            for field in incoming.tag_changes {
                if let Some(old) = current
                    .tag_changes
                    .iter_mut()
                    .find(|f| f.field == field.field)
                {
                    old.new = field.new;
                    old.invalid = field.invalid;
                } else {
                    current.tag_changes.push(field);
                }
            }
            if incoming.cover_change.is_some() {
                current.cover_change = incoming.cover_change;
            }
            if incoming.rename_to.is_some() {
                current.rename_to = incoming.rename_to;
                current.copy = incoming.copy;
                current.rename_root = incoming.rename_root;
                current.sidecar_renames = incoming.sidecar_renames;
                current.folder_extras = incoming.folder_extras;
                current.creates_folder = incoming.creates_folder;
            }
            for block in incoming.block_changes {
                if let Some(old) = current
                    .block_changes
                    .iter_mut()
                    .find(|b| b.kind == block.kind)
                {
                    *old = block;
                } else {
                    current.block_changes.push(block);
                }
            }
        } else {
            positions.insert(incoming.path.clone(), previous.changes.len());
            previous.changes.push(incoming);
        }
    }
    previous.description = next.description;
    previous.message = next.message;
    previous.notes.extend(next.notes);
    previous.locked_skipped.extend(next.locked_skipped);
    previous.unrendered.extend(next.unrendered);
    prune_empty_changes(&mut previous);
    Ok(previous)
}
fn prune_empty_changes(plan: &mut PlanDto) {
    for change in &mut plan.changes {
        change.tag_changes.retain(|field| {
            field.invalid
                || field.old.as_deref().unwrap_or("") != field.new.as_deref().unwrap_or("")
        });
    }
    plan.changes.retain(|change| {
        !change.tag_changes.is_empty()
            || change.rename_to.is_some()
            || change.cover_change.is_some()
            || !change.block_changes.is_empty()
            || !change.sidecar_renames.is_empty()
    });
}

// Shared only with the trusted recovery module, never exposed as an MCP tool.
pub(super) fn persist_recovery_operations(directory: &Path, state: &State) -> ServiceResult<()> {
    persist_operations(directory, state)
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tagrex_core::plan::{ApplyProgress, ApplyStep};

    fn sample_plan(changes: Value) -> PlanDto {
        serde_json::from_value(json!({"description":"Progress fixture","message":null,"notes":[],"changes":changes,"locked_skipped":[],"unrendered":[]})).unwrap()
    }
    fn tag_change(path: &str) -> Value {
        json!({"path":path,"rename_to":null,"tag_changes":[{"field":"title","old":null,"new":"Changed","invalid":false}]})
    }
    fn signal(path: &str, done: usize, completed: Option<ApplyStep>) -> ApplyProgress {
        ApplyProgress {
            done,
            total: 100_000,
            path: PathBuf::from(path),
            completed,
        }
    }
    #[test]
    fn preflight_and_sidecars_never_count_as_completed_media_files() {
        let mut moved = tag_change("move.wav");
        moved["rename_to"] = json!("moved.wav");
        moved["sidecar_renames"] = json!([["move.lrc", "moved.lrc"]]);
        let plan = sample_plan(json!([tag_change("tags.wav"), moved]));
        let mut tracker = FileProgress::new(&plan);
        assert_eq!(
            tracker
                .observe(&signal("tags.wav", 8, None), Duration::ZERO)
                .processed,
            0
        );
        assert_eq!(
            tracker
                .observe(
                    &signal("move.wav", 9, Some(ApplyStep::Tags)),
                    Duration::ZERO
                )
                .processed,
            0
        );
        assert_eq!(
            tracker
                .observe(
                    &signal("move.lrc", 10, Some(ApplyStep::Transfer)),
                    Duration::ZERO
                )
                .processed,
            0
        );
        let first = tracker.observe(
            &signal("tags.wav", 11, Some(ApplyStep::Tags)),
            Duration::ZERO,
        );
        assert_eq!(first.processed, 1);
        assert!(
            first.emit,
            "The first confirmed file must bypass throttling"
        );
        let duplicate = tracker.observe(
            &signal("tags.wav", 12, Some(ApplyStep::Tags)),
            Duration::ZERO,
        );
        assert_eq!(duplicate.processed, 1);
        assert!(!duplicate.changed);
        assert!(!duplicate.emit);
        let last = tracker.observe(
            &signal("move.wav", 13, Some(ApplyStep::Transfer)),
            Duration::ZERO,
        );
        assert_eq!(last.processed, 2);
        assert_eq!(last.total, 2);
        assert!(last.emit, "The last confirmed file must bypass throttling");
    }
    #[test]
    fn a_copy_with_metadata_finishes_only_after_its_tags_are_written() {
        let mut copied = tag_change("copy.wav");
        copied["rename_to"] = json!("copied.wav");
        copied["copy"] = json!(true);
        let plan = sample_plan(json!([copied]));
        let mut tracker = FileProgress::new(&plan);
        assert_eq!(
            tracker
                .observe(
                    &signal("copy.wav", 4, Some(ApplyStep::Transfer)),
                    Duration::ZERO
                )
                .processed,
            0
        );
        assert_eq!(
            tracker
                .observe(
                    &signal("copy.wav", 5, Some(ApplyStep::Tags)),
                    Duration::ZERO
                )
                .processed,
            1
        );
    }
    #[test]
    fn a_hundred_thousand_fast_files_emit_a_bounded_time_based_stream() {
        let count = 100_000;
        let changes = (0..count)
            .map(|i| tag_change(&format!("{i}.wav")))
            .collect::<Vec<_>>();
        let plan = sample_plan(json!(changes));
        let mut tracker = FileProgress::new(&plan);
        let mut emitted = Vec::new();
        for (index, change) in plan.changes.iter().enumerate() {
            let now = Duration::from_micros(index as u64 * 10);
            for completed in [None, None, Some(ApplyStep::Tags)] {
                let update = tracker.observe(&signal(&change.path, count + index, completed), now);
                assert!(update.processed <= update.total);
                if update.emit {
                    emitted.push(update.processed);
                }
            }
        }
        assert_eq!(tracker.processed, count);
        assert_eq!(emitted.first(), Some(&0));
        assert_eq!(emitted.get(1), Some(&1));
        assert_eq!(emitted.last(), Some(&count));
        assert!(
            emitted.len() <= 23,
            "{} IPC notifications for a one-second burst",
            emitted.len()
        );
    }
    #[test]
    fn genuine_two_wav_apply_reports_files_instead_of_native_step_counts() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        fs::create_dir_all(&media).unwrap();
        let paths = (0..2)
            .map(|index| {
                let path = media.join(format!("{index}.wav"));
                write_wav(&path);
                path
            })
            .collect::<Vec<_>>();
        let mut app = App::open(&media, &dir.path().join("journal.sqlite")).unwrap();
        let edits = paths
            .iter()
            .map(|path| TagEditDto {
                path: path.to_string_lossy().into_owned(),
                field: "title".into(),
                value: Some("Written".into()),
            })
            .collect::<Vec<_>>();
        let plan = app.preview_tag_edits(&edits).unwrap();
        let mut tracker = FileProgress::new(&plan);
        let mut native_max = 0;
        let mut counts = Vec::new();
        app.apply_with(
            &plan,
            &mut |progress| {
                native_max = native_max.max(progress.done);
                let update = tracker.observe(progress, Duration::ZERO);
                counts.push(update.processed);
                assert_eq!(update.total, 2);
                assert!(update.processed <= 2);
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            native_max > 2,
            "Fixture must actually reproduce step counts exceeding the file count"
        );
        assert_eq!(counts.first(), Some(&0));
        assert_eq!(counts.last(), Some(&2));
        assert_eq!(
            counts.iter().copied().collect::<BTreeSet<_>>(),
            [0, 1, 2].into()
        );
    }
    fn write_wav(path: &Path) {
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend(356u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(16000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(320u32.to_le_bytes());
        bytes.extend([0u8; 320]);
        fs::write(path, bytes).unwrap();
    }
    #[test]
    fn an_unemitted_progress_callback_still_enforces_expired_authorization() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        fs::create_dir_all(&media).unwrap();
        let paths = (0..2)
            .map(|index| {
                let path = media.join(format!("{index}.wav"));
                write_wav(&path);
                path
            })
            .collect::<Vec<_>>();
        let before = paths
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>();
        let app = App::open(&media, &dir.path().join("journal.sqlite")).unwrap();
        let service =
            Service::new(Arc::new(Mutex::new(Some(app))), dir.path().join("service")).unwrap();
        service.set_enabled(true).unwrap();
        let pairing = service
            .pair_client(
                "Progress expiry fixture".into(),
                Grant {
                    roots: vec![media],
                    actions: ["read", "stage", "write"]
                        .into_iter()
                        .map(String::from)
                        .collect(),
                    ..Grant::default()
                },
            )
            .unwrap();
        let principal = service.authenticate(&pairing.credential).unwrap();
        let (started, ready) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let wait = Mutex::new(wait);
        let blocked = AtomicBool::new(false);
        service
            .set_event_sink(Arc::new(move |event, detail| {
                if event == "operation_changed"
                    && detail.get("processed") == Some(&json!(0))
                    && !blocked.swap(true, Ordering::AcqRel)
                {
                    started.send(()).unwrap();
                    wait.lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                }
            }))
            .unwrap();
        let operation=service.call(&principal,"files.write",json!({"paths":paths,"values":{"title":"Never write"},"idempotency_key":"progress-expiry"})).unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        {
            let mut state = service.0.state.lock().unwrap();
            let client = state
                .security
                .clients
                .iter_mut()
                .find(|c| c.id == principal.id)
                .unwrap();
            client.grant.expires_at = Some(now().saturating_sub(1));
        }
        release.send(()).unwrap();
        let id = operation["operation"].as_str().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = service.operations().unwrap();
            let record = status["operations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == id)
                .unwrap();
            if record["status"] == "cancelled_and_restored" {
                assert_eq!(record["processed"], 0);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Operation did not cancel: {record}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        for (path, before) in paths.iter().zip(before) {
            assert_eq!(fs::read(path).unwrap(), before);
        }
    }
    #[test]
    fn service_record_and_terminal_events_never_exceed_the_two_real_files() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        fs::create_dir_all(&media).unwrap();
        let paths = (0..2)
            .map(|index| {
                let path = media.join(format!("{index}.wav"));
                write_wav(&path);
                path
            })
            .collect::<Vec<_>>();
        let app = App::open(&media, &dir.path().join("journal.sqlite")).unwrap();
        let service =
            Service::new(Arc::new(Mutex::new(Some(app))), dir.path().join("service")).unwrap();
        service.set_enabled(true).unwrap();
        let pairing = service
            .pair_client(
                "Native progress fixture".into(),
                Grant {
                    roots: vec![media],
                    actions: ["read", "stage", "write"]
                        .into_iter()
                        .map(String::from)
                        .collect(),
                    ..Grant::default()
                },
            )
            .unwrap();
        let principal = service.authenticate(&pairing.credential).unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let observed_events = observed.clone();
        service
            .set_event_sink(Arc::new(move |event, detail| {
                if event == "operation_changed" {
                    observed_events.lock().unwrap().push(detail);
                }
            }))
            .unwrap();
        let operation=service.call(&principal,"files.write",json!({"paths":paths,"values":{"title":"Native counted write"},"idempotency_key":"two-file-counts"})).unwrap();
        let id = operation["operation"].as_str().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let values = service.operations().unwrap();
            let record = values["operations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|record| record["id"] == id)
                .unwrap();
            assert!(record["processed"].as_u64().unwrap() <= 2);
            if record["status"] == "succeeded"
                && observed
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e.get("completed") == Some(&json!(true)))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Native write did not complete: {record}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let events = observed.lock().unwrap();
        let counts = events
            .iter()
            .filter_map(|value| value.get("processed").and_then(Value::as_u64))
            .collect::<Vec<_>>();
        assert!(counts.iter().all(|count| *count <= 2));
        assert!(counts.contains(&0));
        assert!(counts.contains(&1));
        assert_eq!(counts.last(), Some(&2));
        assert_eq!(events.first().unwrap()["status"], "queued");
        let last = events.last().unwrap();
        assert_eq!(last["completed"], true);
        assert_eq!(last["status"], "succeeded");
        assert_eq!(last["processed"], 2);
        assert_eq!(last["total"], 2);
    }
}

#[cfg(test)]
mod composition_tests {
    use super::*;
    use base64::Engine;
    use std::time::{Duration, Instant};
    struct Fixture {
        service: Service,
        principal: Principal,
        paths: Vec<PathBuf>,
        _dir: tempfile::TempDir,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let media = dir.path().join("media");
            fs::create_dir_all(&media).unwrap();
            let paths = (0..2)
                .map(|index| {
                    let path = media.join(format!("{index}.wav"));
                    let mut bytes = Vec::new();
                    bytes.extend(b"RIFF");
                    bytes.extend(356u32.to_le_bytes());
                    bytes.extend(b"WAVEfmt ");
                    bytes.extend(16u32.to_le_bytes());
                    bytes.extend(1u16.to_le_bytes());
                    bytes.extend(1u16.to_le_bytes());
                    bytes.extend(8000u32.to_le_bytes());
                    bytes.extend(16000u32.to_le_bytes());
                    bytes.extend(2u16.to_le_bytes());
                    bytes.extend(16u16.to_le_bytes());
                    bytes.extend(b"data");
                    bytes.extend(320u32.to_le_bytes());
                    bytes.extend([0u8; 320]);
                    fs::write(&path, bytes).unwrap();
                    path
                })
                .collect::<Vec<_>>();
            let mut app = App::open(&media, &dir.path().join("journal.sqlite")).unwrap();
            let initial = paths
                .iter()
                .flat_map(|path| {
                    [("title", "Saved"), ("track", "1"), ("tracktotal", "12")].map(
                        |(field, value)| TagEditDto {
                            path: path.to_string_lossy().into_owned(),
                            field: field.into(),
                            value: Some(value.into()),
                        },
                    )
                })
                .collect::<Vec<_>>();
            let plan = app.preview_tag_edits(&initial).unwrap();
            app.apply(&plan).unwrap();
            let service =
                Service::new(Arc::new(Mutex::new(Some(app))), dir.path().join("service")).unwrap();
            service.set_enabled(true).unwrap();
            let paired = service
                .pair_client(
                    "Composition fixture".into(),
                    Grant {
                        roots: vec![media],
                        actions: ["read", "stage", "write"]
                            .into_iter()
                            .map(String::from)
                            .collect(),
                        ..Grant::default()
                    },
                )
                .unwrap();
            let principal = service.authenticate(&paired.credential).unwrap();
            Self {
                service,
                principal,
                paths,
                _dir: dir,
            }
        }
        fn call(&self, tool: &str, args: Value) -> Value {
            self.service.call(&self.principal, tool, args).unwrap()
        }
        fn draft(&self, values: Value) -> Value {
            self.call(
                "draft.create",
                json!({"paths":[self.paths[0]],"values":values}),
            )
        }
        fn release(&self, first: usize, second: usize, separator: &str) -> String {
            let tracks=(1..=first).map(|n|json!({"position":format!("1{separator}{n:02}"),"disc":1,"title":format!("Disc1 track{n}")})).chain((1..=second).map(|n|json!({"position":format!("2{separator}{n:02}"),"disc":2,"title":format!("Disc2 track{n}")}))).collect::<Vec<_>>();
            let release:ReleaseDto=serde_json::from_value(json!({"id":"fixture-release","artist":"Various","title":"Fixture Album","year":2004,"genres":[],"styles":[],"tracks":tracks,"disc_total":2})).unwrap();
            let id = opaque("release").unwrap();
            self.service.0.state.lock().unwrap().releases.insert(
                id.clone(),
                ReleaseReceipt {
                    owner: self.principal.id.clone(),
                    source: "discogs".into(),
                    release,
                },
            );
            id
        }
        fn asset(&self, color: [u8; 3]) -> Value {
            let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                2,
                2,
                image::Rgb(color),
            ));
            let mut cursor = std::io::Cursor::new(Vec::new());
            image
                .write_to(&mut cursor, image::ImageFormat::Png)
                .unwrap();
            self.call("artwork.upload",json!({"data_base64":base64::engine::general_purpose::STANDARD.encode(cursor.into_inner())}))
        }
    }
    fn field<'a>(draft: &'a Value, path: &Path, key: &str) -> Option<&'a Value> {
        draft["changes"]
            .as_array()?
            .iter()
            .find(|change| change["path"].as_str() == path.to_str())?["tags"]
            .as_array()?
            .iter()
            .find(|field| field["field"] == key)
    }
    #[test]
    fn explicit_edit_equal_to_saved_removes_previous_desired_field_and_keeps_other_intent() {
        let f = Fixture::new();
        let draft = f.draft(json!({"tracktotal":"25","custom:KEEP":"Pending"}));
        let restored=f.call("draft.create",json!({"paths":[f.paths[0]],"draft":draft["draft"],"edits":[{"path":f.paths[0],"field":"tracktotal","value":"12"}]}));
        assert!(field(&restored, &f.paths[0], "tracktotal").is_none());
        assert_eq!(
            field(&restored, &f.paths[0], "custom:KEEP").unwrap()["new"],
            "Pending"
        );
    }
    #[test]
    fn clear_tags_discards_newly_staged_custom_fields_that_are_not_on_disk() {
        let f = Fixture::new();
        let draft = f.draft(json!({"custom:NEW":"Remove this"}));
        let cleared = f.call(
            "draft.clear_tags",
            json!({"paths":[f.paths[0]],"draft":draft["draft"]}),
        );
        assert!(field(&cleared, &f.paths[0], "custom:NEW").is_none());
        assert_eq!(
            field(&cleared, &f.paths[0], "title").unwrap()["new"],
            Value::Null
        );
    }
    #[test]
    fn fetched_multidisc_totals_override_saved_equal_previous_wrong_totals() {
        let f = Fixture::new();
        let release = f.release(13, 12, ".");
        let wrong = f.draft(json!({"tracktotal":"25"}));
        let imported=f.call("draft.import",json!({"paths":[f.paths[0]],"draft":wrong["draft"],"release":release,"track_indices":[13]}));
        assert!(field(&imported, &f.paths[0], "tracktotal").is_none());
        assert_eq!(field(&imported, &f.paths[0], "disc").unwrap()["new"], "2");
        assert_eq!(
            field(&imported, &f.paths[0], "title").unwrap()["new"],
            "Disc2 track1"
        );
    }
    #[test]
    fn selection_spanning_discs_receives_each_discs_own_track_total() {
        let f = Fixture::new();
        let release = f.release(13, 12, "-");
        let wrong = f.call(
            "draft.create",
            json!({"paths":f.paths,"values":{"tracktotal":"28"}}),
        );
        let imported=f.call("draft.import",json!({"paths":f.paths,"draft":wrong["draft"],"release":release,"track_indices":[0,13]}));
        assert_eq!(
            field(&imported, &f.paths[0], "tracktotal").unwrap()["new"],
            "13"
        );
        assert!(field(&imported, &f.paths[1], "tracktotal").is_none());
    }
    #[test]
    fn actual_second_release_shape_has_fourteen_tracks_per_disc_not_twenty_eight() {
        let f = Fixture::new();
        let receipt = f.release(14, 14, "-");
        let imported = f.call(
            "draft.import",
            json!({"paths":[f.paths[0]],"release":receipt,"track_indices":[15]}),
        );
        assert_eq!(
            field(&imported, &f.paths[0], "tracktotal").unwrap()["new"],
            "14"
        );
        assert_eq!(field(&imported, &f.paths[0], "track").unwrap()["new"], "2");
    }
    #[test]
    fn artwork_revert_to_saved_drops_only_cover_intent_and_keeps_pending_tags() {
        let f = Fixture::new();
        let saved = f.asset([10, 20, 30]);
        let changed = f.asset([30, 20, 10]);
        {
            let state = f.service.0.state.lock().unwrap();
            let cover = state
                .assets
                .get(saved["asset"].as_str().unwrap())
                .unwrap()
                .cover
                .clone();
            drop(state);
            let mut guard = f.service.0.app.lock().unwrap();
            let app = guard.as_mut().unwrap();
            let plan = app
                .preview_cover_set(&[f.paths[0].clone()], &[cover])
                .unwrap();
            app.apply(&plan).unwrap();
        }
        let tags = f.draft(json!({"comment":"Keep my comment"}));
        let staged=f.call("draft.artwork",json!({"paths":[f.paths[0]],"draft":tags["draft"],"asset":changed["asset"],"mode":"replace_all"}));
        let reverted=f.call("draft.artwork",json!({"paths":[f.paths[0]],"draft":staged["draft"],"asset":saved["asset"],"mode":"replace_all"}));
        assert_eq!(reverted["files"], 1);
        assert_eq!(reverted["changes"][0]["artwork"], Value::Null);
        assert_eq!(
            field(&reverted, &f.paths[0], "comment").unwrap()["new"],
            "Keep my comment"
        );
    }
    #[test]
    fn transform_description_uses_latest_named_operation_without_old_draft_lineage() {
        let f = Fixture::new();
        let pending = f.draft(json!({"title":"Title  Feat. Guest"}));
        let first=f.call("draft.transform",json!({"paths":[f.paths[0]],"draft":pending["draft"],"groups":[{"name":"Normalize feat","scope":"title","rules":[{"kind":"replace","from":"Feat.","to":"feat."}]}]}));
        let second=f.call("draft.transform",json!({"paths":[f.paths[0]],"draft":first["draft"],"groups":[{"name":"Tidy spacing","scope":"title","rules":[{"kind":"replace","from":"  ","to":" "}]}]}));
        assert!(second["description"]
            .as_str()
            .unwrap()
            .contains("Tidy spacing"));
        assert!(!second["description"]
            .as_str()
            .unwrap()
            .contains("Normalize feat"));
        assert_eq!(
            field(&second, &f.paths[0], "title").unwrap()["new"],
            "Title feat. Guest"
        );
    }
    #[test]
    fn a_lock_added_after_preparation_refuses_commit_and_a_queued_write() {
        let f = Fixture::new();
        let draft = f.draft(json!({"title":"Not allowed"}));
        {
            let guard = f.service.0.app.lock().unwrap();
            guard.as_ref().unwrap().set_locked_fields(&["title".into()]);
        }
        let request =
            json!({"draft":draft["draft"],"digest":draft["digest"],"idempotency_key":"new-lock"});
        assert_eq!(
            f.service
                .call(&f.principal, "plan.commit", request)
                .unwrap_err()
                .code,
            "field_locked"
        );
        {
            let guard = f.service.0.app.lock().unwrap();
            guard.as_ref().unwrap().set_locked_fields(&[]);
        }
        let mutation = f.service.0.mutation.lock().unwrap();
        let accepted=f.call("plan.commit",json!({"draft":draft["draft"],"digest":draft["digest"],"idempotency_key":"queued-lock"}));
        {
            let guard = f.service.0.app.lock().unwrap();
            guard.as_ref().unwrap().set_locked_fields(&["title".into()]);
        }
        drop(mutation);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let result = f.call(
                "operations.status",
                json!({"operation":accepted["operation"]}),
            );
            let record = &result["operations"][0];
            if record["status"] == "failed" {
                assert_eq!(record["error"]["code"], "field_locked");
                break;
            }
            assert!(Instant::now() < deadline, "{record}");
            std::thread::sleep(Duration::from_millis(2));
        }
        let guard = f.service.0.app.lock().unwrap();
        assert_eq!(
            guard
                .as_ref()
                .unwrap()
                .read_track(&f.paths[0])
                .unwrap()
                .tags["title"],
            "Saved"
        );
    }
    struct ImportSettings;
    impl SettingsBackend for ImportSettings {
        fn read(&self) -> ServiceResult<Value> {
            Ok(json!({"import_skip_fields":["artist"]}))
        }
        fn compare_and_write(&self, _: &str, _: Value) -> ServiceResult<()> {
            Err(ServiceError::invalid("Unused test writer"))
        }
    }
    #[test]
    fn import_none_and_skipped_fields_preserve_previous_intent_but_saved_equal_provided_fields_replace_it(
    ) {
        let f = Fixture::new();
        {
            let guard = f.service.0.app.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .apply_settings(&tagrex_commands::SettingsDto {
                    import_skip_fields: vec!["artist".into()],
                    ..Default::default()
                });
        }
        f.service
            .set_settings_backend(Arc::new(ImportSettings))
            .unwrap();
        let draft=f.draft(json!({"artist":"Keep pending artist","title":"Replace pending title","isrc":"GBAYE0601477","bpm":"174","key":"Am","custom:RELEASECOUNTRY":"Keep pending country"}));
        let imported=f.call("draft.import",json!({"paths":[f.paths[0]],"draft":draft["draft"],"import":{"source":"discogs","country":null,"tracks":[{"position":"1","disc":1,"artist":"Skipped source artist","title":"Saved","isrc":null,"bpm":null,"key":null}]}}));
        assert!(field(&imported, &f.paths[0], "title").is_none());
        for (key, value) in [
            ("artist", "Keep pending artist"),
            ("isrc", "GBAYE0601477"),
            ("bpm", "174"),
            ("key", "Am"),
            ("custom:RELEASECOUNTRY", "Keep pending country"),
        ] {
            assert_eq!(
                field(&imported, &f.paths[0], key).unwrap()["new"],
                value,
                "Import unexpectedly erased {key}"
            );
        }
    }
    #[test]
    fn restoring_the_saved_filename_removes_previous_rename_only() {
        let f = Fixture::new();
        let tags = f.draft(json!({"comment":"Keep"}));
        let renamed = f.call(
            "draft.rename",
            json!({"paths":[f.paths[0]],"draft":tags["draft"],"mask":"Renamed"}),
        );
        let reverted = f.call(
            "draft.rename",
            json!({"paths":[f.paths[0]],"draft":renamed["draft"],"mask":"0"}),
        );
        assert_eq!(reverted["changes"][0]["rename_to"], Value::Null);
        assert_eq!(
            field(&reverted, &f.paths[0], "comment").unwrap()["new"],
            "Keep"
        );
    }
    #[test]
    fn the_catalog_defines_edit_objects_scope_artwork_modes_and_filter_wrappers() {
        let tools = Service::tool_catalog();
        let edit = tools
            .iter()
            .find(|tool| tool.name == "draft.create")
            .unwrap();
        assert_eq!(
            edit.input_schema["properties"]["edits"]["items"]["required"],
            json!(["path", "field", "value"])
        );
        assert!(edit.input_schema["anyOf"].is_array());
        let art = tools
            .iter()
            .find(|tool| tool.name == "draft.artwork")
            .unwrap();
        assert!(art.input_schema["properties"].get("mode").is_some());
        let preset = tools
            .iter()
            .find(|tool| tool.name == "presets.put")
            .unwrap();
        assert!(preset.description.contains("preset={query:"));
        assert!(preset.description.contains("SHARED"));
    }
}
