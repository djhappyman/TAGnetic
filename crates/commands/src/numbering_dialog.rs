//! Desktop numbering intent. Rust owns counters, vinyl parsing and totals;
//! the interface supplies only selection order and displayed group identities.
use super::*;
use crate::field_operations::invalid;
use crate::native_batch::{check_cancel, restore_locked_sources, unique_paths};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NumberingDialogOptions {
    Tracks {
        start: String,
        #[serde(default, rename = "trackTotal")]
        track_total: bool,
        #[serde(default, rename = "restartGroup")]
        restart_group: bool,
    },
    Disc {
        start: String,
        #[serde(default, rename = "discTotal")]
        disc_total: bool,
        #[serde(default, rename = "advanceGroup")]
        advance_group: bool,
        #[serde(default, rename = "resetAlbum")]
        reset_album: bool,
    },
    Vinyl {
        source: String,
        #[serde(default, rename = "keepPosition")]
        keep_position: bool,
        #[serde(default, rename = "trackTotal")]
        track_total: bool,
        #[serde(default, rename = "discTotal")]
        disc_total: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberingDialogPreview {
    /// Operation-only plan: unrelated staged edits must never enter quick Write.
    pub plan: PlanDto,
    /// Includes unchanged-on-disk assignments, which can replace a staged value.
    pub edits: Vec<TagEditDto>,
    pub matched: usize,
    pub skipped: usize,
}

fn starting_number(value: &str) -> Result<u32, AppError> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid("Enter a whole starting number of 1 or more."));
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| invalid("The starting number is outside the supported range."))
}

fn vinyl_numbers(value: &str) -> Result<Option<(u32, u32)>, AppError> {
    let value = value.trim();
    let Some(disc) = side_disc_from_position(value) else {
        return Ok(None);
    };
    // The shared native side parser established ASCII shape before slicing.
    let digits = if value.as_bytes()[0].is_ascii_alphabetic() {
        &value[1..]
    } else {
        &value[..value.len() - 1]
    };
    let track = if digits.is_empty() {
        1
    } else {
        digits
            .parse::<u32>()
            .map_err(|_| invalid("A vinyl track number is too large."))?
    };
    if track == 0 {
        return Err(invalid("Vinyl track numbers must start at 1."));
    }
    Ok(Some((disc, track)))
}

impl App {
    /// Preview one of the independent desktop numbering actions. `groups` must
    /// align exactly with `paths`; identities describe the currently displayed
    /// grouping, including drop/release groups not expressible as a single tag.
    pub fn preview_numbering_dialog_with_cancel(
        &self,
        paths: &[PathBuf],
        groups: &[String],
        options: &NumberingDialogOptions,
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<NumberingDialogPreview, AppError> {
        let _policy = self.model_policy.enter();
        check_cancel(&cancelled)?;
        if paths.len() != groups.len() {
            return Err(invalid("Numbering groups must match the selected files."));
        }
        let selected = unique_paths(paths)?;
        if selected.len() != paths.len() {
            return Err(invalid("Numbering selection contains duplicate files."));
        }
        if groups
            .iter()
            .try_fold(0usize, |n, value| n.checked_add(value.len()))
            .is_none_or(|n| n > 16 * 1024 * 1024)
        {
            return Err(invalid(
                "Numbering group identities exceed the 16 MiB limit.",
            ));
        }
        self.validate_numbering_inputs(&selected, working, &cancelled)?;
        let fields: &[&str] = match options {
            NumberingDialogOptions::Tracks {
                track_total: true, ..
            } => &["track", "tracktotal"],
            NumberingDialogOptions::Tracks { .. } => &["track"],
            NumberingDialogOptions::Disc {
                disc_total: true, ..
            } => &["disc", "disctotal"],
            NumberingDialogOptions::Disc { .. } => &["disc"],
            NumberingDialogOptions::Vinyl { .. } => &["track", "disc"],
        };
        let locked = self.locked_fields();
        let mut targets = fields.to_vec();
        if let NumberingDialogOptions::Vinyl {
            keep_position,
            track_total,
            disc_total,
            ..
        } = options
        {
            if *keep_position {
                targets.push("custom:POSITION");
            }
            if *track_total {
                targets.push("tracktotal");
            }
            if *disc_total {
                targets.push("disctotal");
            }
        }
        if targets
            .iter()
            .any(|field| locked.iter().any(|key| key.eq_ignore_ascii_case(field)))
        {
            return Err(invalid("Unlock the numbering fields before continuing."));
        }
        if let NumberingDialogOptions::Vinyl {
            source,
            keep_position,
            track_total,
            disc_total,
        } = options
        {
            let preview = self.preview_vinyl_numbers(
                &selected,
                source,
                (*keep_position, *track_total, *disc_total),
                working,
                &cancelled,
            )?;
            self.validate_numbering_inputs(&selected, working, &cancelled)?;
            return Ok(preview);
        }
        let group_keys = paths.iter().cloned().zip(groups.iter().cloned()).collect();
        let mut native = NumberingOptions {
            keep_vinyl: false,
            ..Default::default()
        };
        let number_tracks = match options {
            NumberingDialogOptions::Tracks {
                start,
                track_total,
                restart_group,
            } => {
                native.start = starting_number(start)?;
                native.write_total = *track_total;
                native.reset_track_on = restart_group.then_some(NumberBoundary::Folder);
                true
            }
            NumberingDialogOptions::Disc {
                start,
                disc_total,
                advance_group,
                reset_album,
            } => {
                native.disc = Some(starting_number(start)?);
                native.write_disc_total = *disc_total;
                native.next_disc_on = advance_group.then_some(NumberBoundary::Folder);
                native.reset_disc_on = reset_album.then(|| NumberBoundary::Field {
                    field: "album".into(),
                });
                false
            }
            NumberingDialogOptions::Vinyl { .. } => unreachable!(),
        };
        let preview = self.numbering_preview(
            &selected,
            &native,
            working,
            Some(&group_keys),
            number_tracks,
            &cancelled,
        )?;
        self.validate_numbering_inputs(&selected, working, &cancelled)?;
        Ok(preview)
    }

    // Source validation remains separate from the output plan: quick Write must
    // refuse stale source metadata without inheriting unrelated pending edits.
    fn validate_numbering_inputs(
        &self,
        paths: &[PathBuf],
        working: Option<&PlanDto>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), AppError> {
        let base = working_changes(working);
        let mut bytes = 0usize;
        for path in paths {
            check_cancel(cancelled)?;
            let Some(change) = base.get(path.as_path()) else {
                continue;
            };
            let track = self.read_cached(path)?;
            for edit in &change.tag_changes {
                bytes = bytes
                    .checked_add(edit.old.as_ref().map_or(0, String::len))
                    .and_then(|n| n.checked_add(edit.new.as_ref().map_or(0, String::len)))
                    .ok_or_else(|| invalid("Numbering source values are too large."))?;
                if bytes > 64 * 1024 * 1024 {
                    return Err(invalid("Numbering source values exceed the 64 MiB limit."));
                }
                let field = TagField::from_storage_key(&edit.field);
                if track.tags.get(&field).map(String::as_str).unwrap_or("")
                    != edit.old.as_deref().unwrap_or("")
                {
                    return Err(tagrex_core::plan::PlanError::Stale(path.clone()).into());
                }
            }
        }
        check_cancel(cancelled)
    }

    fn preview_vinyl_numbers(
        &self,
        paths: &[PathBuf],
        source: &str,
        totals: (bool, bool, bool),
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<NumberingDialogPreview, AppError> {
        let source = match source {
            "track" => TagField::TrackNumber,
            "disc" => TagField::DiscNumber,
            _ => return Err(invalid("Choose the Track or Disc field containing A1/B2.")),
        };
        let (keep_position, track_total, disc_total) = totals;
        let base = working_changes(working);
        let locked = self.locked_fields();
        let mut previous_album = None;
        let mut release = 0usize;
        let mut rows = Vec::new();
        let mut tracks = HashMap::<(usize, u32), u32>::new();
        let mut discs = HashMap::<usize, u32>::new();
        for path in paths {
            check_cancel(&cancelled)?;
            let track = self.read_cached(path)?;
            let mut tags = track.tags.clone();
            overlay_working_tags(&mut tags, base.get(path.as_path()).copied());
            restore_locked_sources(&mut tags, &track.tags, &locked);
            let album = tags.get(&TagField::Album).cloned().unwrap_or_default();
            if previous_album
                .as_ref()
                .is_some_and(|previous| previous != &album)
            {
                release += 1;
            }
            previous_album = Some(album);
            let raw = tags
                .get(&source)
                .map(|value| value.trim())
                .unwrap_or_default();
            let Some((disc, number)) = vinyl_numbers(raw)? else {
                continue;
            };
            tracks
                .entry((release, disc))
                .and_modify(|n| *n = (*n).max(number))
                .or_insert(number);
            discs
                .entry(release)
                .and_modify(|n| *n = (*n).max(disc))
                .or_insert(disc);
            rows.push((path.clone(), release, disc, number, raw.to_owned()));
        }
        let mut values = HashMap::new();
        for (path, release, disc, number, raw) in rows {
            check_cancel(&cancelled)?;
            let mut tags: TagMap = [
                (TagField::TrackNumber, number.to_string()),
                (TagField::DiscNumber, disc.to_string()),
            ]
            .into_iter()
            .collect();
            if keep_position {
                tags.insert(TagField::from_storage_key("custom:POSITION"), raw);
            }
            if track_total {
                tags.insert(TagField::TrackTotal, tracks[&(release, disc)].to_string());
            }
            if disc_total {
                tags.insert(TagField::DiscTotal, discs[&release].to_string());
            }
            values.insert(path, tags);
        }
        self.finish_numbering(paths, values, None, cancelled)
    }

    pub(super) fn finish_numbering(
        &self,
        paths: &[PathBuf],
        values: HashMap<PathBuf, TagMap>,
        working: Option<&PlanDto>,
        cancelled: impl Fn() -> bool,
    ) -> Result<NumberingDialogPreview, AppError> {
        let mut edits = Vec::new();
        let mut matched = 0;
        for path in paths {
            check_cancel(&cancelled)?;
            if let Some(tags) = values.get(path) {
                matched += 1;
                for (field, value) in tags {
                    edits.push(TagEditDto {
                        path: path.to_string_lossy().into_owned(),
                        field: field.to_storage_key(),
                        value: Some(value.clone()),
                    });
                }
            }
        }
        let plan = self.preview_assignments(paths, values, working, &cancelled)?;
        check_cancel(&cancelled)?;
        Ok(NumberingDialogPreview {
            plan,
            edits,
            matched,
            skipped: paths.len() - matched,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vinyl_shape_and_numeric_limits_are_exact() {
        for (raw, expected) in [
            (" B3 ", Some((2, 3))),
            ("3b", Some((2, 3))),
            ("B", Some((2, 1))),
            ("A0002", Some((1, 2))),
            ("12", None),
            ("AB1", None),
            ("É2", None),
            ("B-3", None),
        ] {
            assert_eq!(vinyl_numbers(raw).unwrap(), expected);
        }
        assert!(vinyl_numbers("A0").is_err());
        assert!(vinyl_numbers("A4294967296").is_err());
        assert_eq!(vinyl_numbers("Z4294967295").unwrap(), Some((26, u32::MAX)));
    }
    #[test]
    fn starts_reject_fraction_exponent_negative_and_overflow() {
        for raw in ["", "0", "-1", "1.5", "1e2", "4294967296"] {
            assert!(starting_number(raw).is_err());
        }
        assert_eq!(starting_number(" 0007 ").unwrap(), 7);
    }
}
