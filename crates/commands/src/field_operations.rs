//! Compiled field recipes. Patterns and selectors are compiled once per batch;
//! each file is read once and steps see preceding desired values, never disk
//! values from before the recipe. Only the ordinary plan gate can publish writes.
use super::*;

/// Native field operations, serializable for callers and saved recipes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldOperation {
    Format {
        field: String,
        pattern: String,
    },
    /// Render the source using current staged tags, then extract named fields.
    Extract {
        source: String,
        pattern: String,
    },
    /// Split an explicitly chosen list field; punctuation in other fields stays literal.
    SplitValues {
        field: String,
        separator: String,
    },
    /// Join a supported list into one value; refuse an unrepresentable result.
    MergeValues {
        field: String,
        separator: String,
    },
    /// Keep the first occurrence of each exact value, in its original order.
    DeduplicateValues {
        field: String,
    },
    Remove {
        fields: Vec<String>,
    },
    Keep {
        fields: Vec<String>,
    },
}

pub(crate) enum Step {
    Format(TagField, Mask),
    Extract(Mask, Mask),
    Assign(HashMap<PathBuf, TagMap>),
    Values(TagField, ValueStep),
    Select {
        fields: HashSet<String>,
        keep: bool,
        pictures: bool,
    },
    Transform(String, TransformChain),
}
pub(crate) fn invalid(message: impl Into<String>) -> AppError {
    MaskError::BadArgument(message.into()).into()
}
pub(crate) enum ValueStep {
    Split(String),
    Merge(String),
    Deduplicate,
}

fn assign_value(
    tags: &mut TagMap,
    field: &TagField,
    value: Option<String>,
    locked: &HashSet<String>,
    blocked: &mut BTreeMap<TagField, Option<String>>,
) -> Result<(), AppError> {
    let mut matches = tags.keys().filter(|key| key.same_name(field));
    let key = matches.next().cloned().unwrap_or_else(|| field.clone());
    if matches.next().is_some() {
        return Err(invalid(
            "Multiple stored fields differ only in letter case; resolve them before editing.",
        ));
    }
    let value = value.filter(|value| !value.is_empty());
    if locked.contains(&key.to_storage_key()) {
        blocked.insert(key, value);
    } else if let Some(value) = value {
        tags.insert(key, value);
    } else {
        tags.remove(&key);
    }
    Ok(())
}
impl FieldOperation {
    fn compile(&self) -> Result<Step, AppError> {
        match self {
            Self::Format { field, pattern } => {
                if ["tags", "filename", "fileext", "cover", "picture"]
                    .iter()
                    .any(|name| field.trim().eq_ignore_ascii_case(name))
                {
                    return Err(invalid("Expected a text tag destination; use custom: to explicitly name a custom field."));
                }
                if pattern.len() > 8192 {
                    return Err(invalid("Field patterns are limited to 8192 bytes."));
                }
                let field = TagField::from_name(field).ok_or_else(|| invalid("Expected a writable tag field; use rename tools for file or directory targets."))?;
                let mask = Mask::parse(pattern)?;
                // Reject extract-only masks before reading even the first file.
                if mask.is_extract_only() {
                    return Err(MaskError::ExtractOnly.into());
                }
                Ok(Step::Format(field, mask))
            }
            Self::Extract { source, pattern } => {
                let source = Mask::parse(source)?;
                if source.is_extract_only() {
                    return Err(MaskError::ExtractOnly.into());
                }
                let pattern = Mask::parse(pattern)?;
                pattern.validate_unique_extraction()?;
                if pattern.fields().is_empty() {
                    return Err(invalid(
                        "An extraction pattern must name at least one destination field.",
                    ));
                }
                Ok(Step::Extract(source, pattern))
            }
            Self::SplitValues { field, separator } | Self::MergeValues { field, separator } => {
                if (matches!(self, Self::SplitValues { .. }) && separator.is_empty())
                    || separator.len() > 1024
                    || separator.contains('\0')
                {
                    return Err(invalid(
                        "Value separators are limited to 1024 bytes without NUL; a split separator cannot be empty.",
                    ));
                }
                let field = list_field(field)?;
                Ok(Step::Values(
                    field,
                    match self {
                        Self::SplitValues { .. } => ValueStep::Split(separator.clone()),
                        _ => ValueStep::Merge(separator.clone()),
                    },
                ))
            }
            Self::DeduplicateValues { field } => {
                Ok(Step::Values(list_field(field)?, ValueStep::Deduplicate))
            }
            Self::Remove { fields } | Self::Keep { fields } => {
                if fields.len() > 256 {
                    return Err(invalid("A field selector is limited to 256 names."));
                }
                let mut tags = HashSet::new();
                let mut pictures = false;
                for name in fields {
                    let name = name.trim();
                    if name.eq_ignore_ascii_case("cover") || name.eq_ignore_ascii_case("picture") {
                        pictures = true;
                    } else {
                        let field = TagField::from_name(name)
                            .ok_or_else(|| invalid(format!("Invalid field selector: {name}")))?;
                        tags.insert(field.to_storage_key().to_ascii_lowercase());
                    }
                }
                Ok(Step::Select {
                    fields: tags,
                    keep: matches!(self, Self::Keep { .. }),
                    pictures,
                })
            }
        }
    }
}

fn list_field(name: &str) -> Result<TagField, AppError> {
    let field = TagField::from_name(name).ok_or_else(|| invalid("Expected a tag field."))?;
    if !field.is_multi_value() {
        return Err(invalid("This field does not yet support lossless multiple-value editing. Choose artist, albumartist, genre or composer."));
    }
    Ok(field)
}

pub fn has_field_rules(groups: &[ActionGroupDto]) -> bool {
    groups.iter().flat_map(|group| &group.rules).any(|rule| {
        rule.enabled
            && matches!(
                rule.kind.as_str(),
                "format"
                    | "extract"
                    | "split_values"
                    | "merge_values"
                    | "deduplicate_values"
                    | "remove_fields"
                    | "keep_fields"
            )
    })
}
/// Validate all recipes even for empty selections. Disabled rules remain inert.
pub fn validate_field_groups(groups: &[ActionGroupDto]) -> Result<(), AppError> {
    compile_groups(groups).map(|_| ())
}
fn compile_groups(groups: &[ActionGroupDto]) -> Result<Vec<Step>, AppError> {
    if has_field_rules(groups)
        && groups
            .iter()
            .flat_map(|group| group.rules.iter().map(move |rule| (group, rule)))
            .any(|(group, rule)| {
                rule.enabled
                    && matches!(
                        rule.scope.as_deref().unwrap_or(&group.scope),
                        "filename" | "fileext"
                    )
            })
    {
        return Err(invalid("Field recipes operate on text tags. Use the existing rename tools for filename and directory changes."));
    }

    let mut steps = Vec::new();
    for group in groups {
        for rule in group.rules.iter().filter(|rule| rule.enabled) {
            if steps.len() >= 256 {
                return Err(invalid("Recipes are limited to 256 enabled steps."));
            }
            let fields = || {
                rule.from
                    .split(';')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect()
            };
            steps.push(match rule.kind.as_str() {
                "format" => FieldOperation::Format {
                    field: rule.scope.clone().unwrap_or_else(|| group.scope.clone()),
                    pattern: rule.to.clone(),
                }
                .compile()?,
                "remove_fields" => FieldOperation::Remove { fields: fields() }.compile()?,
                "keep_fields" => FieldOperation::Keep { fields: fields() }.compile()?,
                "extract" => FieldOperation::Extract {
                    source: rule.from.clone(),
                    pattern: rule.to.clone(),
                }
                .compile()?,
                "split_values" => FieldOperation::SplitValues {
                    field: rule.scope.clone().unwrap_or_else(|| group.scope.clone()),
                    separator: rule.from.clone(),
                }
                .compile()?,
                "merge_values" => FieldOperation::MergeValues {
                    field: rule.scope.clone().unwrap_or_else(|| group.scope.clone()),
                    separator: rule.to.clone(),
                }
                .compile()?,
                "deduplicate_values" => FieldOperation::DeduplicateValues {
                    field: rule.scope.clone().unwrap_or_else(|| group.scope.clone()),
                }
                .compile()?,
                _ => Step::Transform(
                    rule.scope.clone().unwrap_or_else(|| group.scope.clone()),
                    build_chain(std::slice::from_ref(rule))?,
                ),
            });
        }
    }
    Ok(steps)
}
impl App {
    /// Compile a sequence of explicit operations and preview its cumulative diff.
    /// `working` is the trusted current plan: unrelated staged rows and structural
    /// changes survive. No audio is written here. Cancellation publishes no plan.
    pub fn preview_field_operations(
        &self,
        paths: &[PathBuf],
        operations: &[FieldOperation],
        working: Option<&PlanDto>,
    ) -> Result<PlanDto, AppError> {
        self.preview_field_operations_with_cancel(paths, operations, working, || false)
    }
    pub fn preview_field_operations_with_cancel(
        &self,
        paths: &[PathBuf],
        operations: &[FieldOperation],
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<PlanDto, AppError> {
        if operations.len() > 256 {
            return Err(invalid("Recipes are limited to 256 steps."));
        }
        let steps = operations
            .iter()
            .map(FieldOperation::compile)
            .collect::<Result<Vec<_>, _>>()?;
        self.run_field_steps(paths, &steps, working, cancelled)
    }
    pub fn preview_field_groups(
        &self,
        paths: &[PathBuf],
        groups: &[ActionGroupDto],
        working: Option<&PlanDto>,
    ) -> Result<PlanDto, AppError> {
        if !has_field_rules(groups) {
            return match working {
                Some(plan) => self.preview_transform_over_plan(plan, groups),
                None => self.preview_transform_groups(paths, groups),
            };
        }
        self.run_field_steps(paths, &compile_groups(groups)?, working, || false)
    }
    pub(crate) fn run_field_steps(
        &self,
        paths: &[PathBuf],
        steps: &[Step],
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<PlanDto, AppError> {
        let _policy = self.model_policy.enter();
        let _encoding_scope = cover_encoding::Scope::new();
        let policy_revision = tagrex_core::model::read_policy_revision();
        let check_policy = || {
            if policy_revision == u64::MAX
                || tagrex_core::model::read_policy_revision() != policy_revision
            {
                Err(invalid(
                    "Metadata settings changed during the recipe; preview it again.",
                ))
            } else {
                Ok(())
            }
        };
        if paths.len() > 100_000 {
            return Err(invalid(
                "Field batches are limited to 100000 selected files.",
            ));
        }
        if paths.iter().any(|path| path.to_str().is_none()) {
            return Err(invalid(
                "Field batches require Unicode paths so preview and Apply address the same files.",
            ));
        }
        if let Some(plan) = working {
            plan.validate_cover_assets()?;
        }
        let base = working_changes(working);
        let locked: HashSet<_> = self.locked_fields().into_iter().collect();
        let selected: HashSet<_> = paths.iter().map(PathBuf::as_path).collect();
        let mut changes: Vec<_> = working
            .into_iter()
            .flat_map(|plan| &plan.changes)
            .filter(|change| !selected.contains(Path::new(&change.path)))
            .cloned()
            .collect();
        let mut seen = HashSet::new();
        let mut unmatched = Vec::new();
        let mut batch_bytes = 0usize;
        for path in paths {
            if cancelled() {
                return Err(tagrex_core::plan::PlanError::Cancelled.into());
            }
            check_policy()?;
            if !seen.insert(path) {
                continue;
            }
            let track = self.read_cached(path)?;
            let previous = base.get(path.as_path()).copied();
            if previous.is_some_and(|change| {
                change.tag_changes.iter().any(|edit| {
                    let field = TagField::from_storage_key(&edit.field);
                    track.tags.get(&field).map(String::as_str).unwrap_or("")
                        != edit.old.as_deref().unwrap_or("")
                })
            }) {
                return Err(tagrex_core::plan::PlanError::Stale(path.clone()).into());
            }

            let mut tags = track.tags.clone();
            overlay_working_tags(&mut tags, previous);
            let mut blocked = BTreeMap::new();
            for key in &locked {
                let field = TagField::from_storage_key(key);
                if tags.get(&field) != track.tags.get(&field) {
                    blocked.insert(field.clone(), tags.get(&field).cloned());
                    match track.tags.get(&field) {
                        Some(value) => {
                            tags.insert(field, value.clone());
                        }
                        None => {
                            tags.remove(&field);
                        }
                    }
                }
            }

            let mut change = previous.cloned().unwrap_or_else(|| FileChangeDto {
                path: path.to_string_lossy().into_owned(),
                rename_to: None,
                tag_changes: Vec::new(),
                cover_change: None,
                sidecar_renames: Vec::new(),
                block_changes: Vec::new(),
                copy: false,
                rename_root: None,
                creates_folder: false,
                folder_extras: Vec::new(),
            });
            let mut remove_covers = false;
            for step in steps {
                if cancelled() {
                    return Err(tagrex_core::plan::PlanError::Cancelled.into());
                }
                check_policy()?;
                match step {
                    Step::Format(field, mask) => {
                        let file = self.file_context(mask, &track);
                        let value = mask.render_value_with(&tags, &file)?;
                        assign_value(&mut tags, field, Some(value), &locked, &mut blocked)?;
                    }
                    Step::Extract(source, pattern) => {
                        let file = self.file_context(source, &track);
                        let value = source.render_value_with(&tags, &file)?;
                        match pattern.extract(&value) {
                            Ok(extracted) => {
                                // Extract before assigning: source and destinations may overlap.
                                for (field, value) in extracted {
                                    let value = normalize_extracted(&field, &value);
                                    if !value.is_empty() {
                                        assign_value(
                                            &mut tags,
                                            &field,
                                            Some(value),
                                            &locked,
                                            &mut blocked,
                                        )?;
                                    }
                                }
                            }
                            Err(MaskError::NoMatch) => {
                                tally_unrendered(&mut unmatched, "extraction pattern".into(), path)
                            }
                            Err(err) => return Err(err.into()),
                        }
                    }
                    Step::Assign(assignments) => {
                        if let Some(values) = assignments.get(path) {
                            for (field, value) in values {
                                assign_value(
                                    &mut tags,
                                    field,
                                    Some(value.clone()),
                                    &locked,
                                    &mut blocked,
                                )?;
                            }
                        }
                    }
                    Step::Values(field, operation) => {
                        let Some(value) = tags.get(field) else {
                            continue;
                        };
                        let separator = tagrex_core::model::multi_value_separator();
                        let values =
                            tagrex_core::values::FieldValues::from_legacy_list(value, &separator)?;
                        let next = match operation {
                            ValueStep::Split(delimiter) => values.split(delimiter)?,
                            ValueStep::Merge(delimiter) => values.merge(delimiter)?,
                            ValueStep::Deduplicate => values.deduplicated(),
                        };
                        let value = next.to_legacy_list(&separator)?;
                        assign_value(&mut tags, field, Some(value), &locked, &mut blocked)?;
                    }
                    Step::Select {
                        fields,
                        keep,
                        pictures,
                    } => {
                        tags.retain(|key, _| {
                            let retain = fields
                                .contains(&key.to_storage_key().to_ascii_lowercase())
                                == *keep;
                            if !retain && locked.contains(&key.to_storage_key()) {
                                blocked.insert(key.clone(), None);
                                true
                            } else {
                                retain
                            }
                        });
                        // Keep-only applies to text tags. Artwork removal is always
                        // explicit in Tagnetic, including in imported recipes.
                        remove_covers |= *pictures && !keep;
                    }
                    Step::Transform(scope, chain) => {
                        for (field, value) in &mut tags {
                            if scope == "tags" || scope == &field.to_storage_key() {
                                let next = chain.apply(value);
                                if locked.contains(&field.to_storage_key()) {
                                    blocked.insert(field.clone(), Some(next));
                                } else {
                                    *value = next;
                                }
                            }
                        }
                    }
                }
                if tags.values().map(String::len).sum::<usize>() > 8 * 1024 * 1024 {
                    return Err(invalid(
                        "A field recipe is limited to 8 MiB of text per file.",
                    ));
                }
            }
            let original_fields: HashMap<_, _> = previous
                .into_iter()
                .flat_map(|change| &change.tag_changes)
                .map(|edit| (edit.field.as_str(), &edit.old))
                .collect();
            let keys: std::collections::BTreeSet<_> =
                track.tags.keys().chain(tags.keys()).cloned().collect();
            change.tag_changes = keys
                .into_iter()
                .filter_map(|field| {
                    let key = field.to_storage_key();
                    let old = original_fields
                        .get(key.as_str())
                        .map(|old| (*old).clone())
                        .unwrap_or_else(|| track.tags.get(&field).cloned());
                    let new = tags.get(&field).cloned();
                    (old != new).then(|| FieldChangeDto::new(key, old, new))
                })
                .collect();
            // Keep rejected attempts for the existing gate's lock feedback,
            // without letting a blocked intermediate value feed later steps.
            change
                .tag_changes
                .extend(blocked.into_iter().filter_map(|(field, new)| {
                    let old = track.tags.get(&field).cloned();
                    (old != new).then(|| FieldChangeDto::new(field.to_storage_key(), old, new))
                }));
            for edit in &change.tag_changes {
                batch_bytes = batch_bytes
                    .saturating_add(edit.old.as_ref().map_or(0, String::len))
                    .saturating_add(edit.new.as_ref().map_or(0, String::len));
            }
            if batch_bytes > 64 * 1024 * 1024 {
                return Err(invalid(
                    "A field batch is limited to 64 MiB of changed text, including Undo values.",
                ));
            }
            if remove_covers {
                let old = match &change.cover_change {
                    Some(cover) => cover.old.clone(),
                    None => cover_arts_to_dto(&TagEngine::read_covers(path)?),
                };
                change.cover_change = (!old.is_empty()).then_some(CoverChangeDto {
                    old,
                    new: Vec::new(),
                });
            }
            changes.push(change);
        }
        if cancelled() {
            return Err(tagrex_core::plan::PlanError::Cancelled.into());
        }
        check_policy()?;
        let mut plan = match working {
            Some(plan) => self.cleaned_up_plan(plan, changes),
            None => self.plan(
                PlanMessage::Transform {
                    scope: "Field operations".into(),
                },
                Vec::new(),
                changes,
                false,
            ),
        };
        plan.unrendered.extend(unmatched);
        check_policy()?;
        Ok(plan)
    }
}
