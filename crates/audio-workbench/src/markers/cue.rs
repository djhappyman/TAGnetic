use std::fmt;

use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};
use serde::{Deserialize, Serialize};

use super::{check_timeline, scale_time, validate_ranges, FrameRange, MarkerError};

const CUE_TICKS: u32 = 75;
const MAX_CUE_TRACKS: usize = 99;
const MAX_CUE_BYTES: usize = 1024 * 1024;
const GENERATED_GAPS: &str =
    "Generated CUE gaps describe silence. They do not add samples to split audio.";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CueEncoding {
    /// Prefer UTF-8 when the sheet has no byte order mark.
    #[default]
    Utf8,
    /// Interpret a sheet without a byte order mark as Windows-1252.
    Windows1252,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CueTags {
    pub title: Option<String>,
    pub performer: Option<String>,
    pub songwriter: Option<String>,
    pub isrc: Option<String>,
    pub genre: Option<String>,
    pub date: Option<String>,
    pub catalog: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CueTrack {
    pub number: u8,
    /// Position in CUE ticks, relative to the recording.
    pub index01: u64,
    pub index00: Option<u64>,
    /// Lengths of generated gaps in CUE ticks, outside the source audio.
    pub pregap: Option<u64>,
    pub postgap: Option<u64>,
    pub tags: CueTags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CueSheet {
    /// The FILE argument is descriptive. It cannot grant filesystem access.
    pub source_name: String,
    pub file_type: String,
    pub tags: CueTags,
    pub tracks: Vec<CueTrack>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CueMarker {
    pub number: u8,
    pub frame: u64,
    pub pregap_frame: Option<u64>,
    pub index01: u64,
    pub index00: Option<u64>,
    pub pregap: Option<u64>,
    pub postgap: Option<u64>,
    pub tags: CueTags,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CueExportTrack {
    pub range: FrameRange,
    pub tags: CueTags,
    pub pregap_frame: Option<u64>,
    /// The imported ticks are reused if they still agree with the edited frame.
    pub index01: Option<u64>,
    pub index00: Option<u64>,
    pub pregap: Option<u64>,
    pub postgap: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CueExport {
    pub text: String,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct ParsedTrack {
    number: u8,
    index00: Option<u64>,
    index01: Option<u64>,
    pregap: Option<u64>,
    postgap: Option<u64>,
    tags: CueTags,
}

impl ParsedTrack {
    fn finish(self) -> Result<CueTrack, MarkerError> {
        let index01 = self.index01.ok_or(MarkerError::CueBoundary {
            track: usize::from(self.number),
        })?;
        if self.index00.is_some_and(|index00| index00 >= index01) {
            return Err(MarkerError::CueBoundary {
                track: usize::from(self.number),
            });
        }
        Ok(CueTrack {
            number: self.number,
            index01,
            index00: self.index00,
            pregap: self.pregap,
            postgap: self.postgap,
            tags: self.tags,
        })
    }
}

/// Parse a single-file AUDIO cue sheet without opening its FILE reference.
pub fn parse_cue(bytes: &[u8], encoding: CueEncoding) -> Result<CueSheet, MarkerError> {
    if bytes.len() > MAX_CUE_BYTES {
        return Err(MarkerError::CueInputLimit);
    }
    let (detected, offset) = Encoding::for_bom(bytes).unwrap_or((
        match encoding {
            CueEncoding::Utf8 => UTF_8,
            CueEncoding::Windows1252 => WINDOWS_1252,
        },
        0,
    ));
    let decoded = detected
        .decode_without_bom_handling_and_without_replacement(&bytes[offset..])
        .ok_or(MarkerError::CueEncoding)?;
    let mut file = None;
    let mut tags = CueTags::default();
    let mut parsed: Vec<ParsedTrack> = Vec::new();
    let mut warnings = Vec::new();
    for (index, line) in decoded.lines().enumerate() {
        let line_error = |reason| MarkerError::CueLine {
            line: index + 1,
            reason,
        };
        if line
            .chars()
            .any(|value| value.is_control() && value != '\t')
        {
            return Err(line_error("unexpected control character"));
        }
        let (command, rest) = next_word(line.trim());
        match command.to_ascii_uppercase().as_str() {
            "" => continue,
            "FILE" => {
                if file.is_some() {
                    return Err(line_error("only a single FILE declaration is supported"));
                }
                let (name, kind) =
                    next_value(rest).ok_or_else(|| line_error("invalid recording name"))?;
                let kind = kind.to_ascii_uppercase();
                if name.is_empty()
                    || name.chars().any(|c| c.is_control())
                    || !matches!(kind.as_str(), "WAVE" | "AIFF" | "MP3" | "FLAC")
                {
                    return Err(line_error("expected a WAV, AIFF, MP3 or FLAC audio FILE"));
                }
                file = Some((name.to_owned(), kind));
            }
            "TRACK" => {
                if file.is_none() {
                    return Err(line_error("TRACK must follow FILE"));
                }
                if parsed.len() >= MAX_CUE_TRACKS {
                    return Err(MarkerError::CueTrackLimit);
                }
                let (number, kind) = next_word(rest);
                let number = number
                    .parse::<u8>()
                    .ok()
                    .filter(|number| (1..=99).contains(number))
                    .ok_or_else(|| line_error("track number must be between 1 and 99"))?;
                if !kind.eq_ignore_ascii_case("AUDIO") {
                    return Err(line_error("only AUDIO tracks are supported"));
                }
                if parsed.last().is_some_and(|last| last.number >= number) {
                    return Err(line_error("track numbers must increase"));
                }
                parsed.push(ParsedTrack {
                    number,
                    ..ParsedTrack::default()
                });
            }
            "INDEX" => {
                let track = parsed
                    .last_mut()
                    .ok_or_else(|| line_error("INDEX must follow TRACK"))?;
                let (number, time) = next_word(rest);
                let time =
                    parse_cue_time(time).ok_or_else(|| line_error("invalid MM:SS:FF time"))?;
                match number {
                    "00" | "0" if track.index00.is_none() && track.index01.is_none() => {
                        track.index00 = Some(time)
                    }
                    "01" | "1" if track.index01.is_none() => track.index01 = Some(time),
                    "00" | "0" | "01" | "1" => {
                        return Err(line_error("duplicate or out-of-order INDEX"))
                    }
                    _ => return Err(line_error("only INDEX 00 and INDEX 01 are supported")),
                }
            }
            "PREGAP" | "POSTGAP" => {
                let track = parsed
                    .last_mut()
                    .ok_or_else(|| line_error("gap must follow TRACK"))?;
                let gap = if command.eq_ignore_ascii_case("PREGAP") {
                    &mut track.pregap
                } else {
                    &mut track.postgap
                };
                if gap.is_some() {
                    return Err(line_error("duplicate generated gap"));
                }
                *gap = Some(parse_cue_time(rest).ok_or_else(|| line_error("invalid gap time"))?);
                warn(&mut warnings, GENERATED_GAPS);
            }
            "CDTEXTFILE" | "FLAGS" => warn(
                &mut warnings,
                "Auxiliary CUE instructions were not applied.",
            ),
            "REM" => {
                let (name, value) = next_word(rest);
                let current_tags = parsed.last_mut().map_or(&mut tags, |track| &mut track.tags);
                if let Some(field) = rem_tag(current_tags, name) {
                    *field =
                        Some(read_tag(value).ok_or_else(|| line_error("invalid quoted metadata"))?);
                }
            }
            _ => {
                let current_tags = parsed.last_mut().map_or(&mut tags, |track| &mut track.tags);
                if let Some(field) = normal_tag(current_tags, command) {
                    *field =
                        Some(read_tag(rest).ok_or_else(|| line_error("invalid quoted metadata"))?);
                } else {
                    warn(
                        &mut warnings,
                        "Unrecognized CUE instructions were not applied.",
                    );
                }
            }
        }
    }
    let (source_name, file_type) = file.ok_or(MarkerError::CueStructure)?;
    if parsed.is_empty() {
        return Err(MarkerError::CueStructure);
    }
    let tracks = parsed
        .into_iter()
        .map(ParsedTrack::finish)
        .collect::<Result<Vec<_>, _>>()?;
    for pair in tracks.windows(2) {
        let left = &pair[0];
        let right = &pair[1];
        if right.index01 <= left.index01 || right.index00.is_some_and(|gap| gap < left.index01) {
            return Err(MarkerError::CueBoundary {
                track: usize::from(right.number),
            });
        }
    }
    Ok(CueSheet {
        source_name,
        file_type,
        tags,
        tracks,
        warnings,
    })
}

impl CueSheet {
    /// Convert whole-sheet offsets to exact absolute frames on a verified source.
    /// Callers retain the first nonzero start as a possible lead-in.
    pub fn markers(
        &self,
        sample_rate: u32,
        total_frames: u64,
    ) -> Result<Vec<CueMarker>, MarkerError> {
        check_timeline(sample_rate, total_frames)?;
        let mut previous = None;
        self.tracks
            .iter()
            .map(|track| {
                let frame = scale_time(track.index01, sample_rate, CUE_TICKS)?;
                let pregap_frame = track
                    .index00
                    .map(|gap| scale_time(gap, sample_rate, CUE_TICKS))
                    .transpose()?;
                if frame >= total_frames
                    || previous.is_some_and(|prior| prior >= frame)
                    || pregap_frame.is_some_and(|gap| {
                        gap >= frame || previous.is_some_and(|prior| gap < prior)
                    })
                {
                    return Err(MarkerError::CueBoundary {
                        track: usize::from(track.number),
                    });
                }
                previous = Some(frame);
                Ok(CueMarker {
                    number: track.number,
                    frame,
                    pregap_frame,
                    index01: track.index01,
                    index00: track.index00,
                    pregap: track.pregap,
                    postgap: track.postgap,
                    tags: track.tags.clone(),
                })
            })
            .collect()
    }
}

/// Write track starts for a single master. The first INDEX 01 may follow
/// unassigned opening audio. From there, the tracks must cover the source.
pub fn write_cue(
    source_name: &str,
    sample_rate: u32,
    total_frames: u64,
    tags: &CueTags,
    tracks: &[CueExportTrack],
) -> Result<CueExport, MarkerError> {
    check_timeline(sample_rate, total_frames)?;
    if tracks.len() > MAX_CUE_TRACKS {
        return Err(MarkerError::CueTrackLimit);
    }
    let ranges: Vec<_> = tracks.iter().map(|track| track.range).collect();
    validate_ranges(&ranges, total_frames)?;
    if !ranges.last().is_some_and(|last| last.end == total_frames)
        || ranges.windows(2).any(|pair| pair[0].end != pair[1].start)
    {
        return Err(MarkerError::CueExclusions);
    }
    let kind = file_type(source_name).ok_or(MarkerError::CueSourceName)?;
    if source_name.len() > MAX_CUE_BYTES
        || source_name.contains('"')
        || source_name.chars().any(char::is_control)
    {
        return Err(MarkerError::CueSourceName);
    }
    let mut writer = CueWriter::default();
    writer.tags(tags, "")?;
    writer.emit(format_args!("FILE \"{source_name}\" {kind}\n"))?;
    let mut previous_tick = None;
    for (index, entry) in tracks.iter().enumerate() {
        let track = index + 1;
        let tick = export_tick(entry.range.start, entry.index01, sample_rate)?;
        let previous_start = index
            .checked_sub(1)
            .and_then(|prior| tracks.get(prior))
            .map_or(0, |prior| prior.range.start);
        if scale_time(tick, sample_rate, CUE_TICKS)? >= total_frames
            || previous_tick.is_some_and(|previous| previous >= tick)
        {
            return Err(MarkerError::CueRounding { track });
        }
        writer.rounding(entry.range.start, tick, sample_rate)?;
        writer.emit(format_args!("  TRACK {track:02} AUDIO\n"))?;
        writer.tags(&entry.tags, "    ")?;
        if let Some(duration) = entry.pregap {
            warn(&mut writer.warnings, GENERATED_GAPS);
            writer.emit(format_args!("    PREGAP {}\n", format_time(duration)))?;
        }
        if let Some(frame) = entry.pregap_frame {
            if frame < previous_start || frame >= entry.range.start {
                return Err(MarkerError::CueBoundary { track });
            }
            let pregap_tick = export_tick(frame, entry.index00, sample_rate)?;
            if pregap_tick >= tick || previous_tick.is_some_and(|previous| pregap_tick < previous) {
                return Err(MarkerError::CueRounding { track });
            }
            writer.rounding(frame, pregap_tick, sample_rate)?;
            writer.emit(format_args!("    INDEX 00 {}\n", format_time(pregap_tick)))?;
        }
        writer.emit(format_args!("    INDEX 01 {}\n", format_time(tick)))?;
        if let Some(duration) = entry.postgap {
            warn(&mut writer.warnings, GENERATED_GAPS);
            writer.emit(format_args!("    POSTGAP {}\n", format_time(duration)))?;
        }
        previous_tick = Some(tick);
    }
    Ok(CueExport {
        text: writer.text,
        warnings: writer.warnings,
    })
}

fn next_word(value: &str) -> (&str, &str) {
    match value.find(char::is_whitespace) {
        Some(index) => (&value[..index], value[index..].trim_start()),
        None => (value, ""),
    }
}

fn next_value(value: &str) -> Option<(&str, &str)> {
    match value.strip_prefix('"') {
        Some(quoted) => {
            let (name, tail) = quoted.split_once('"')?;
            if !tail.is_empty() && !tail.starts_with(char::is_whitespace) {
                return None;
            }
            Some((name, tail.trim_start()))
        }
        None => Some(next_word(value)),
    }
}

fn read_tag(value: &str) -> Option<String> {
    if value.starts_with('"') {
        let (value, tail) = next_value(value)?;
        tail.is_empty().then(|| value.to_owned())
    } else {
        Some(value.to_owned())
    }
}

fn normal_tag<'a>(tags: &'a mut CueTags, name: &str) -> Option<&'a mut Option<String>> {
    match name.to_ascii_uppercase().as_str() {
        "TITLE" => Some(&mut tags.title),
        "PERFORMER" => Some(&mut tags.performer),
        "SONGWRITER" => Some(&mut tags.songwriter),
        "ISRC" => Some(&mut tags.isrc),
        "CATALOG" => Some(&mut tags.catalog),
        _ => None,
    }
}

fn rem_tag<'a>(tags: &'a mut CueTags, name: &str) -> Option<&'a mut Option<String>> {
    match name.to_ascii_uppercase().as_str() {
        "GENRE" => Some(&mut tags.genre),
        "DATE" => Some(&mut tags.date),
        _ => None,
    }
}

fn parse_cue_time(value: &str) -> Option<u64> {
    let (minutes, rest) = value.split_once(':')?;
    let (seconds, frames) = rest.split_once(':')?;
    if [minutes, seconds, frames]
        .iter()
        .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let minutes = minutes.parse::<u64>().ok()?;
    let seconds = seconds.parse::<u64>().ok()?;
    let frames = frames.parse::<u64>().ok()?;
    if seconds >= 60 || frames >= u64::from(CUE_TICKS) {
        return None;
    }
    minutes
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(u64::from(CUE_TICKS))?
        .checked_add(frames)
}

fn file_type(source_name: &str) -> Option<&'static str> {
    let (_, extension) = source_name.rsplit_once('.')?;
    match extension.to_ascii_lowercase().as_str() {
        "wav" | "wave" | "flac" => Some("WAVE"),
        "aif" | "aiff" => Some("AIFF"),
        "mp3" => Some("MP3"),
        _ => None,
    }
}

fn export_tick(frame: u64, original: Option<u64>, sample_rate: u32) -> Result<u64, MarkerError> {
    if let Some(original) = original {
        if scale_time(original, sample_rate, CUE_TICKS).ok() == Some(frame) {
            return Ok(original);
        }
    }
    scale_time(frame, CUE_TICKS, sample_rate)
}

fn format_time(ticks: u64) -> String {
    let ticks_per_second = u64::from(CUE_TICKS);
    let seconds = ticks / ticks_per_second;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 60,
        seconds % 60,
        ticks % ticks_per_second
    )
}

fn warn(warnings: &mut Vec<String>, warning: &'static str) {
    if !warnings.iter().any(|message| message == warning) {
        warnings.push(warning.into());
    }
}

#[derive(Default)]
struct CueWriter {
    text: String,
    warnings: Vec<String>,
}

impl CueWriter {
    fn emit(&mut self, value: fmt::Arguments<'_>) -> Result<(), MarkerError> {
        use fmt::Write as _;
        self.text
            .write_fmt(value)
            .map_err(|_| MarkerError::CueMetadata)?;
        if self.text.len() > MAX_CUE_BYTES {
            return Err(MarkerError::CueMetadata);
        }
        Ok(())
    }

    fn tags(&mut self, tags: &CueTags, indent: &str) -> Result<(), MarkerError> {
        for (field, value) in [
            ("TITLE", &tags.title),
            ("PERFORMER", &tags.performer),
            ("SONGWRITER", &tags.songwriter),
            ("ISRC", &tags.isrc),
            ("CATALOG", &tags.catalog),
            ("REM GENRE", &tags.genre),
            ("REM DATE", &tags.date),
        ] {
            let Some(value) = value.as_deref() else {
                continue;
            };
            if value.len() > MAX_CUE_BYTES || value.chars().any(char::is_control) {
                return Err(MarkerError::CueMetadata);
            }
            if matches!(field, "ISRC" | "CATALOG") {
                // These fields are unquoted in CDRWIN-style CUE sheets.
                if value.is_empty() {
                    continue;
                }
                let valid = if field == "ISRC" {
                    !indent.is_empty()
                        && value.len() == 12
                        && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
                } else {
                    indent.is_empty()
                        && value.len() == 13
                        && value.bytes().all(|byte| byte.is_ascii_digit())
                };
                if !valid {
                    warn(
                        &mut self.warnings,
                        "An invalid ISRC or catalog number was omitted.",
                    );
                    continue;
                }
                self.emit(format_args!("{indent}{field} {value}\n"))?;
                continue;
            }
            if value.contains('"') {
                warn(
                    &mut self.warnings,
                    "Double quotes in metadata are written as apostrophes in the CUE sheet.",
                );
            }
            self.emit(format_args!(
                "{indent}{field} \"{}\"\n",
                value.replace('"', "'")
            ))?;
        }
        Ok(())
    }

    fn rounding(&mut self, frame: u64, tick: u64, sample_rate: u32) -> Result<(), MarkerError> {
        if scale_time(tick, sample_rate, CUE_TICKS)? != frame {
            warn(
                &mut self.warnings,
                "Some cut positions were rounded to CUE's 1/75-second grid. Keep the split draft for exact positions.",
            );
        }
        Ok(())
    }
}
