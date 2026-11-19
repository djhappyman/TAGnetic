//! Native import and counter operations. All results re-enter the field-plan
//! gate; none of these helpers writes audio or publishes a partial result.
use super::*;
use crate::field_operations::{invalid, Step};

const MAX_FILES: usize = 100_000;
const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;
const MATCH_FIELD: &str = "TAGREX_IMPORT_MATCH";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextImportMode {
    #[default]
    Sequential,
    Filename,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextImportOptions {
    pub pattern: String,
    #[serde(default)]
    pub mode: TextImportMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextImportStatus {
    Matched,
    PatternMismatch,
    FileNotSelected,
    NoRemainingFile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextImportRow {
    /// One-based physical line number, including blank lines in the source.
    pub line: usize,
    pub path: Option<String>,
    pub status: TextImportStatus,
    /// Fields this row explicitly supplies, including values already on disk.
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextImportPreview {
    pub plan: PlanDto,
    pub rows: Vec<TextImportRow>,
    pub unmatched_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextSidecarOptions {
    pub field: String,
    /// Relative to each audio file's current folder, with tag/file placeholders.
    pub filename: String,
    /// Missing files are reported; unreadable or invalid files always fail.
    #[serde(default)]
    pub skip_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NumberBoundary {
    Folder,
    Field { field: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NumberingOptions {
    pub start: u32,
    pub step: u32,
    pub width: u8,
    pub write_total: bool,
    pub total: Option<u32>,
    /// Fixed disc number, or the starting disc when boundaries are configured.
    pub disc: Option<u32>,
    pub disc_width: u8,
    pub write_disc_total: bool,
    pub disc_total: Option<u32>,
    pub next_disc_on: Option<NumberBoundary>,
    pub reset_disc_on: Option<NumberBoundary>,
    pub reset_track_on: Option<NumberBoundary>,
    pub reset_track_on_disc: bool,
    pub keep_vinyl: bool,
}
impl Default for NumberingOptions {
    fn default() -> Self {
        Self {
            start: 1,
            step: 1,
            width: 0,
            write_total: false,
            total: None,
            disc: None,
            disc_width: 0,
            write_disc_total: false,
            disc_total: None,
            next_disc_on: None,
            reset_disc_on: None,
            reset_track_on: None,
            reset_track_on_disc: true,
            keep_vinyl: true,
        }
    }
}

fn unique_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, AppError> {
    if paths.len() > MAX_FILES {
        return Err(invalid(
            "Native batches are limited to 100000 selected files.",
        ));
    }
    if paths.iter().any(|path| path.to_str().is_none()) {
        return Err(invalid(
            "Native batches require Unicode paths so preview and Apply address the same files.",
        ));
    }
    let mut seen = HashSet::new();
    Ok(paths
        .iter()
        .filter(|path| seen.insert(path.as_path()))
        .cloned()
        .collect())
}
fn check_cancel(cancelled: &impl Fn() -> bool) -> Result<(), AppError> {
    if cancelled() {
        return Err(tagrex_core::plan::PlanError::Cancelled.into());
    }
    Ok(())
}

impl App {
    pub(crate) fn preview_assignments(
        &self,
        paths: &[PathBuf],
        values: HashMap<PathBuf, TagMap>,
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<PlanDto, AppError> {
        let bytes = values
            .values()
            .flat_map(|tags| tags.values())
            .try_fold(0usize, |total, value| total.checked_add(value.len()));
        if bytes.is_none_or(|size| size > MAX_BATCH_BYTES) {
            return Err(invalid("Imported values exceed the 64 MiB batch limit."));
        }
        self.run_field_steps(paths, &[Step::Assign(values)], working, cancelled)
    }

    pub fn preview_text_import(
        &self,
        paths: &[PathBuf],
        text: &str,
        options: &TextImportOptions,
        working: Option<&PlanDto>,
    ) -> Result<TextImportPreview, AppError> {
        self.preview_text_import_with_cancel(paths, text, options, working, || false)
    }

    /// Index the selection once, then match each parsed row once. Identical
    /// filenames or two rows targeting one file fail rather than choose a winner.
    pub fn preview_text_import_with_cancel(
        &self,
        paths: &[PathBuf],
        text: &str,
        options: &TextImportOptions,
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<TextImportPreview, AppError> {
        check_cancel(&cancelled)?;
        if text.len() > TEXT_LIST_MAX_BYTES as usize || text.contains('\0') {
            return Err(invalid("Text imports are limited to 1 MiB without NUL."));
        }
        let paths = unique_paths(paths)?;
        let mut pattern = text_list_mask(&options.pattern);
        if pattern
            .to_ascii_lowercase()
            .contains(&MATCH_FIELD.to_ascii_lowercase())
        {
            return Err(invalid("The text import matching field name is reserved."));
        }
        let tokens: &[&str] = match options.mode {
            TextImportMode::Sequential => &[],
            TextImportMode::Filename => &["%filenameext%", "%_filename_ext%"],
            TextImportMode::Path => &["%filepath%", "%_path%"],
        };
        if !tokens.is_empty() {
            let lower = pattern.to_ascii_lowercase();
            if tokens
                .iter()
                .map(|token| lower.matches(token).count())
                .sum::<usize>()
                != 1
            {
                return Err(invalid(format!(
                    "The matching pattern must contain exactly one matching placeholder: {}.",
                    tokens.join(" or ")
                )));
            }
            let token = tokens
                .iter()
                .find(|token| lower.contains(**token))
                .ok_or_else(|| invalid("Missing matching placeholder."))?;
            let index = lower
                .find(token)
                .ok_or_else(|| invalid("Missing matching placeholder."))?;
            pattern.replace_range(
                index..index + token.len(),
                &format!("%custom:{MATCH_FIELD}%"),
            );
        }
        let mask = Mask::parse(&pattern)?;
        mask.validate_unique_extraction()?;
        if !mask
            .fields()
            .iter()
            .any(|field| *field != TagField::Custom(MATCH_FIELD.into()))
        {
            return Err(invalid(
                "A text import pattern must name at least one destination tag.",
            ));
        }
        let mut index = HashMap::new();
        if options.mode != TextImportMode::Sequential {
            for path in &paths {
                let key = match options.mode {
                    TextImportMode::Filename => path.file_name().and_then(|name| name.to_str()),
                    _ => path.to_str(),
                }
                .ok_or_else(|| invalid("Text matching requires Unicode paths."))?;
                if index.insert(key.to_string(), path).is_some() {
                    return Err(invalid(format!(
                        "Ambiguous selected match key: {key}. Use full-path matching."
                    )));
                }
            }
        }
        let mut values = HashMap::new();
        let mut matched = HashSet::new();
        let mut rows = Vec::new();
        let mut next = 0;
        for (line_index, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            check_cancel(&cancelled)?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if rows.len() >= MAX_FILES {
                return Err(invalid("Text imports are limited to 100000 nonblank rows."));
            }
            let mut extracted = match mask.extract(line) {
                Ok(tags) => tags,
                Err(MaskError::NoMatch) => {
                    rows.push(TextImportRow {
                        line: line_index + 1,
                        path: None,
                        status: TextImportStatus::PatternMismatch,
                        fields: Vec::new(),
                    });
                    continue;
                }
                Err(err) => return Err(err.into()),
            };
            let path = if options.mode == TextImportMode::Sequential {
                let path = paths.get(next);
                next += 1;
                path
            } else {
                let key = extracted
                    .remove(&TagField::Custom(MATCH_FIELD.into()))
                    .unwrap_or_default();
                index.get(key.as_str()).copied()
            };
            let Some(path) = path else {
                rows.push(TextImportRow {
                    line: line_index + 1,
                    path: None,
                    fields: Vec::new(),
                    status: if options.mode == TextImportMode::Sequential {
                        TextImportStatus::NoRemainingFile
                    } else {
                        TextImportStatus::FileNotSelected
                    },
                });
                continue;
            };
            if !matched.insert(path.clone()) {
                return Err(invalid(format!(
                    "More than one text row matches {}.",
                    path.display()
                )));
            }
            let tags: TagMap = extracted
                .into_iter()
                .filter_map(|(field, value)| {
                    let value = normalize_extracted(&field, &value);
                    (!value.is_empty()).then_some((field, value))
                })
                .collect();
            let fields = tags.keys().map(TagField::to_storage_key).collect();
            values.insert(path.clone(), tags);
            rows.push(TextImportRow {
                line: line_index + 1,
                path: Some(path.to_string_lossy().into()),
                status: TextImportStatus::Matched,
                fields,
            });
        }
        let unmatched_files = paths
            .iter()
            .filter(|path| !matched.contains(*path))
            .map(|path| path.to_string_lossy().into())
            .collect();
        let plan = self.preview_assignments(&paths, values, working, cancelled)?;
        Ok(TextImportPreview {
            plan,
            rows,
            unmatched_files,
        })
    }

    pub fn preview_text_sidecars(
        &self,
        paths: &[PathBuf],
        options: &TextSidecarOptions,
        working: Option<&PlanDto>,
    ) -> Result<PlanDto, AppError> {
        self.preview_text_sidecars_with_cancel(paths, options, working, || false)
    }

    /// Read only regular files beneath each selected audio file's own folder.
    /// Absolute filenames, parent traversal and redirected final inputs fail.
    pub fn preview_text_sidecars_with_cancel(
        &self,
        paths: &[PathBuf],
        options: &TextSidecarOptions,
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<PlanDto, AppError> {
        check_cancel(&cancelled)?;
        // Reuse destination validation, including reserved file/artwork names.
        let destination = FieldOperation::Format {
            field: options.field.clone(),
            pattern: String::new(),
        };
        self.preview_field_operations(&[], &[destination], None)?;
        let field = TagField::from_name(&options.field)
            .ok_or_else(|| invalid("Expected a text tag field."))?;
        let mask = Mask::parse(&options.filename)?;
        if mask.is_extract_only() {
            return Err(MaskError::ExtractOnly.into());
        }
        let paths = unique_paths(paths)?;
        let base = working_changes(working);
        let locked = self.locked_fields();
        let mut values = HashMap::new();
        let mut missing = Vec::new();
        let mut bytes = 0usize;
        for path in &paths {
            check_cancel(&cancelled)?;
            let track = self.read_cached(path)?;
            let mut tags = track.tags.clone();
            overlay_working_tags(&mut tags, base.get(path.as_path()).copied());
            restore_locked_sources(&mut tags, &track.tags, &locked);
            let filename = mask.render_value_with(&tags, &self.file_context(&mask, &track))?;
            // Portable patterns may contain either separator. Refuse Windows
            // drive/ADS syntax on every host as well as native parent traversal.
            if filename.contains(':')
                || filename.contains('\0')
                || filename.starts_with(['/', '\\'])
            {
                return Err(invalid(
                    "Text sidecar filenames must stay beneath the audio file's folder.",
                ));
            }
            let relative = PathBuf::from(filename.replace('\\', "/"));
            let root = path
                .parent()
                .ok_or_else(|| invalid("Audio path has no parent folder."))?;
            let file = match output::read_under(root, &relative) {
                Ok(file) => file,
                Err(err) if options.skip_missing && err.kind() == std::io::ErrorKind::NotFound => {
                    tally_unrendered(&mut missing, "text sidecar".into(), path);
                    continue;
                }
                Err(err) => return Err(err.into()),
            };
            let text = decode_text_file(file, &root.join(&relative))?;
            bytes = bytes
                .checked_add(text.len())
                .ok_or_else(|| invalid("Text sidecar batch is too large."))?;
            if bytes > MAX_BATCH_BYTES {
                return Err(invalid("Text sidecars exceed the 64 MiB batch limit."));
            }
            values.insert(path.clone(), [(field.clone(), text)].into_iter().collect());
        }
        let mut plan = self.preview_assignments(&paths, values, working, cancelled)?;
        plan.unrendered.extend(missing);
        Ok(plan)
    }

    pub fn preview_numbering(
        &self,
        paths: &[PathBuf],
        options: &NumberingOptions,
        working: Option<&PlanDto>,
    ) -> Result<PlanDto, AppError> {
        self.preview_numbering_with_cancel(paths, options, working, || false)
    }

    /// Counters follow explicit selection order. Boundaries are changes between
    /// consecutive eligible files, never a hidden sort or regrouping.
    pub fn preview_numbering_with_cancel(
        &self,
        paths: &[PathBuf],
        options: &NumberingOptions,
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<PlanDto, AppError> {
        check_cancel(&cancelled)?;
        if options.start == 0
            || options.step == 0
            || options.disc == Some(0)
            || options.width > 12
            || options.disc_width > 12
        {
            return Err(invalid(
                "Numbering needs positive counters and padding widths no greater than 12.",
            ));
        }
        for boundary in [
            &options.next_disc_on,
            &options.reset_disc_on,
            &options.reset_track_on,
        ]
        .into_iter()
        .flatten()
        {
            if let NumberBoundary::Field { field } = boundary {
                boundary_field(field)?;
            }
        }
        if options.disc.is_none()
            && (options.next_disc_on.is_some()
                || options.reset_disc_on.is_some()
                || options.write_disc_total
                || options.disc_total.is_some())
        {
            return Err(invalid(
                "Disc numbering options require a starting disc number.",
            ));
        }
        let paths = unique_paths(paths)?;
        let base = working_changes(working);
        let locked = self.locked_fields();
        let mut counters = Vec::new();
        let mut previous: Option<[String; 3]> = None;
        let mut track_number = options.start;
        let mut disc = options.disc.unwrap_or(1);
        let mut track_group = 0usize;
        let mut release_group = 0usize;
        let mut skipped_vinyl = Vec::new();
        for path in &paths {
            check_cancel(&cancelled)?;
            let track = self.read_cached(path)?;
            let mut tags = track.tags.clone();
            overlay_working_tags(&mut tags, base.get(path.as_path()).copied());
            restore_locked_sources(&mut tags, &track.tags, &locked);
            if options.keep_vinyl
                && tags
                    .get(&TagField::TrackNumber)
                    .is_some_and(|value| is_vinyl_position(value))
            {
                tally_unrendered(&mut skipped_vinyl, "preserved vinyl position".into(), path);
                continue;
            }
            let keys = [
                boundary_key(options.next_disc_on.as_ref(), path, &tags)?,
                boundary_key(options.reset_disc_on.as_ref(), path, &tags)?,
                boundary_key(options.reset_track_on.as_ref(), path, &tags)?,
            ];
            if let Some(previous) = &previous {
                let new_release = options.reset_disc_on.is_some() && keys[1] != previous[1];
                let new_disc = options.next_disc_on.is_some() && keys[0] != previous[0];
                let new_track_group = options.reset_track_on.is_some() && keys[2] != previous[2];
                if new_release {
                    disc = options.disc.unwrap_or(1);
                    release_group += 1;
                } else if new_disc {
                    disc = disc
                        .checked_add(1)
                        .ok_or_else(|| invalid("Disc numbering overflow."))?;
                }
                if new_release || new_track_group || (new_disc && options.reset_track_on_disc) {
                    track_number = options.start;
                    track_group += 1;
                } else {
                    track_number = track_number
                        .checked_add(options.step)
                        .ok_or_else(|| invalid("Track numbering overflow."))?;
                }
            }
            previous = Some(keys);
            counters.push((path.clone(), track_number, disc, track_group, release_group));
        }
        let mut track_totals = HashMap::<usize, u32>::new();
        let mut disc_totals = HashMap::<usize, u32>::new();
        for (_, number, disc, group, release) in &counters {
            track_totals
                .entry(*group)
                .and_modify(|value| *value = (*value).max(*number))
                .or_insert(*number);
            disc_totals
                .entry(*release)
                .and_modify(|value| *value = (*value).max(*disc))
                .or_insert(*disc);
        }
        let mut values = HashMap::new();
        for (path, number, disc, group, release) in counters {
            check_cancel(&cancelled)?;
            let mut tags = TagMap::new();
            tags.insert(
                TagField::TrackNumber,
                format!("{number:0width$}", width = usize::from(options.width)),
            );
            if options.write_total {
                let total = options.total.unwrap_or(track_totals[&group]);
                if total < track_totals[&group] {
                    return Err(invalid(
                        "Track total is smaller than a generated track number.",
                    ));
                }
                tags.insert(TagField::TrackTotal, total.to_string());
            }
            if options.disc.is_some() {
                tags.insert(
                    TagField::DiscNumber,
                    format!("{disc:0width$}", width = usize::from(options.disc_width)),
                );
                if options.write_disc_total || options.disc_total.is_some() {
                    let total = options.disc_total.unwrap_or(disc_totals[&release]);
                    if total < disc_totals[&release] {
                        return Err(invalid(
                            "Disc total is smaller than a generated disc number.",
                        ));
                    }
                    tags.insert(TagField::DiscTotal, total.to_string());
                }
            }
            values.insert(path, tags);
        }
        let mut plan = self.preview_assignments(&paths, values, working, cancelled)?;
        plan.unrendered.extend(skipped_vinyl);
        Ok(plan)
    }
}

fn restore_locked_sources(tags: &mut TagMap, saved: &TagMap, locked: &[String]) {
    for key in locked {
        let field = TagField::from_storage_key(key);
        if let Some(value) = saved.get(&field) {
            tags.insert(field, value.clone());
        } else {
            tags.remove(&field);
        }
    }
}
fn boundary_field(name: &str) -> Result<TagField, AppError> {
    if ["filename", "fileext", "tags", "cover", "picture"]
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return Err(invalid("A numbering field boundary must name a text tag."));
    }
    TagField::from_name(name).ok_or_else(|| invalid("A numbering boundary must name a text tag."))
}
fn boundary_key(
    boundary: Option<&NumberBoundary>,
    path: &Path,
    tags: &TagMap,
) -> Result<String, AppError> {
    match boundary {
        None => Ok(String::new()),
        Some(NumberBoundary::Folder) => Ok(path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_string_lossy()
            .into()),
        Some(NumberBoundary::Field { field }) => {
            let field = boundary_field(field)?;
            let mut matches = tags.iter().filter(|(key, _)| key.same_name(&field));
            let value = matches
                .next()
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            if matches.next().is_some() {
                return Err(invalid("Ambiguous field spelling in numbering boundary."));
            }
            Ok(value)
        }
    }
}
