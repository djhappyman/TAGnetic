use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{check_timeline, MarkerError};

const MAX_TRACKS: usize = 1_000;
const MAX_DEPTH: usize = 12;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderTrackKind {
    #[default]
    Track,
    Index,
    Heading,
    #[serde(other)]
    Unrecognized,
}

/// A provider row before normalization. Keep duration text and subtracks intact
/// so malformed times and parent totals can be distinguished from track times.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderTrack {
    pub kind: ProviderTrackKind,
    pub position: String,
    pub title: String,
    pub performer: String,
    pub duration: String,
    pub children: Vec<ProviderTrack>,
}

/// Place a zero-based, half-open range of flattened track rows at a recording
/// position. Fetch proposals without anchors to show the row indices first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAnchor {
    pub start_index: usize,
    pub end_index: usize,
    pub frame: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMarker {
    pub position: String,
    pub title: String,
    pub performer: String,
    pub heading: Option<String>,
    pub side: Option<String>,
    pub duration_seconds: Option<u64>,
    /// Approximate starting position. Unknown timing stays unset.
    pub frame: Option<u64>,
}

/// Compare the estimated end of a selected range with its available extent.
/// Actual audio can include applause, lead-in, or a gap while flipping a tape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSpan {
    pub start_index: usize,
    pub end_index: usize,
    pub expected_end_frame: Option<u64>,
    pub available_end_frame: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProposal {
    pub tracks: Vec<ProviderMarker>,
    pub spans: Vec<ProviderSpan>,
    pub warnings: Vec<String>,
}

/// Propose editable starts using the original release order and explicit
/// anchors. Durations are estimates and are never scaled to fit the recording.
pub fn provider_markers(
    tracks: &[ProviderTrack],
    anchors: &[ProviderAnchor],
    sample_rate: u32,
    total_frames: u64,
) -> Result<ProviderProposal, MarkerError> {
    check_timeline(sample_rate, total_frames)?;
    let mut result = ProviderProposal::default();
    let mut limit = TrackListLimit {
        rows: MAX_TRACKS,
        text_bytes: MAX_TEXT_BYTES,
    };
    flatten(tracks, None, "", 0, &mut limit, &mut result)?;
    if result.tracks.is_empty() {
        warn(
            &mut result.warnings,
            "The release contains no usable audio tracks.",
        );
    }

    let ambiguities = ambiguous_positions(&result.tracks);
    if ambiguities.iter().any(|ambiguous| *ambiguous) {
        warn(
            &mut result.warnings,
            "Some track positions are repeated or out of order. Add an anchor after the uncertain tracks.",
        );
    }
    validate_anchors(anchors, result.tracks.len(), total_frames)?;
    for (index, anchor) in anchors.iter().enumerate() {
        let limit = anchors
            .get(index + 1)
            .map_or(total_frames, |next| next.frame);
        let mut next = Some(anchor.frame);
        let side = result.tracks[anchor.start_index].side.clone();
        let start = anchor.start_index;
        let end = anchor.end_index;
        for (offset, candidate) in result.tracks[start..end].iter_mut().enumerate() {
            if offset > 0 && candidate.side != side {
                warn(
                    &mut result.warnings,
                    "A track range crossed a side or disc boundary. Anchor each side or disc separately.",
                );
                next = None;
            }
            if offset > 0 && ambiguities[start + offset] {
                next = None;
            }
            let Some(frame) = next.filter(|frame| *frame < limit) else {
                next = None;
                break;
            };
            candidate.frame = Some(frame);
            next = candidate
                .duration_seconds
                .map(|duration| {
                    duration
                        .checked_mul(u64::from(sample_rate))
                        .and_then(|duration| frame.checked_add(duration))
                        .ok_or(MarkerError::TimeOverflow)
                })
                .transpose()?;
            if next.is_none() && offset + 1 < end - start {
                warn(
                    &mut result.warnings,
                    "A duration is missing or invalid. Anchor a later track to continue the estimates.",
                );
            }
            let has_later_row = offset + 1 < end - start;
            if next.is_some_and(|estimated_end| {
                estimated_end > limit || (estimated_end == limit && has_later_row)
            }) {
                warn(
                    &mut result.warnings,
                    "The listed durations extend beyond the next anchor or the recording.",
                );
                // Only expose a full-range duration when all rows contributed.
                if has_later_row {
                    next = None;
                }
                break;
            }
        }
        result.spans.push(ProviderSpan {
            start_index: start,
            end_index: end,
            expected_end_frame: next,
            available_end_frame: limit,
        });
    }
    Ok(result)
}

fn validate_anchors(
    anchors: &[ProviderAnchor],
    count: usize,
    total_frames: u64,
) -> Result<(), MarkerError> {
    let mut previous = None;
    for (index, anchor) in anchors.iter().enumerate() {
        if anchor.start_index >= anchor.end_index
            || anchor.end_index > count
            || anchor.frame >= total_frames
            || previous.is_some_and(|prior: &ProviderAnchor| {
                prior.end_index > anchor.start_index || prior.frame >= anchor.frame
            })
        {
            return Err(MarkerError::ProviderAnchor { anchor: index + 1 });
        }
        previous = Some(anchor);
    }
    Ok(())
}

struct TrackListLimit {
    rows: usize,
    text_bytes: usize,
}

fn flatten<'a>(
    tracks: &'a [ProviderTrack],
    mut heading: Option<&'a str>,
    inherited_performer: &str,
    depth: usize,
    limit: &mut TrackListLimit,
    result: &mut ProviderProposal,
) -> Result<(), MarkerError> {
    if depth > MAX_DEPTH {
        return Err(MarkerError::ProviderLimit);
    }
    for track in tracks {
        limit.rows = limit
            .rows
            .checked_sub(1)
            .ok_or(MarkerError::ProviderLimit)?;
        for bytes in [
            track.position.len(),
            track.title.len(),
            track.performer.len(),
            track.duration.len(),
        ] {
            limit.text_bytes = limit
                .text_bytes
                .checked_sub(bytes)
                .ok_or(MarkerError::ProviderLimit)?;
        }
        let performer = if track.performer.trim().is_empty() {
            inherited_performer
        } else {
            &track.performer
        };
        match track.kind {
            ProviderTrackKind::Heading if track.children.is_empty() => {
                let title = track.title.trim();
                heading = (!title.is_empty() && title != "-").then_some(title);
            }
            ProviderTrackKind::Unrecognized => warn(
                &mut result.warnings,
                "Unrecognized provider rows were skipped.",
            ),
            _ if !track.children.is_empty() => {
                if !track.duration.trim().is_empty() {
                    warn(
                        &mut result.warnings,
                        "A group duration was ignored so subtrack durations are only counted once.",
                    );
                }
                let group = (!track.title.trim().is_empty())
                    .then_some(track.title.as_str())
                    .or(heading);
                flatten(&track.children, group, performer, depth + 1, limit, result)?;
            }
            ProviderTrackKind::Index => warn(
                &mut result.warnings,
                "An index row with no playable subtracks was skipped.",
            ),
            ProviderTrackKind::Track => {
                // A heading or performer can be inherited by many tracks.
                let inherited_bytes = heading.map_or(0, str::len)
                    + performer.len().saturating_sub(track.performer.len());
                limit.text_bytes = limit
                    .text_bytes
                    .checked_sub(inherited_bytes)
                    .ok_or(MarkerError::ProviderLimit)?;
                let duration_seconds = parse_duration(&track.duration);
                if duration_seconds.is_none() && !track.duration.trim().is_empty() {
                    warn(
                        &mut result.warnings,
                        "Invalid listed durations were treated as unknown.",
                    );
                }
                result.tracks.push(ProviderMarker {
                    position: track.position.clone(),
                    title: track.title.clone(),
                    performer: performer.to_owned(),
                    heading: heading.map(str::to_owned),
                    side: position(&track.position).0,
                    duration_seconds,
                    frame: None,
                });
            }
            ProviderTrackKind::Heading => {}
        }
    }
    Ok(())
}

fn ambiguous_positions(tracks: &[ProviderMarker]) -> Vec<bool> {
    let mut seen = HashSet::new();
    let mut previous = None;
    tracks
        .iter()
        .map(|track| {
            let label = track.position.trim();
            let duplicate = !label.is_empty() && !seen.insert(label.to_ascii_uppercase());
            let ordinal = position(label).1;
            let backwards = previous.is_some_and(|(side, prior)| {
                side == track.side.as_deref() && ordinal.is_some_and(|number| number <= prior)
            });
            previous = ordinal.map(|number| (track.side.as_deref(), number));
            duplicate || backwards
        })
        .collect()
}

fn position(value: &str) -> (Option<String>, Option<u64>) {
    let value = value.trim();
    if value.is_empty() {
        return (None, None);
    }
    // Multi-disc positions such as 1-01 or CD2-4 must not accumulate across media.
    if let Some((prefix, rest)) = value.split_once('-') {
        let disc = prefix
            .strip_prefix("CD")
            .or_else(|| prefix.strip_prefix("cd"))
            .unwrap_or(prefix);
        if let (Some(disc), Some(track)) = (integer(disc), integer(rest)) {
            return (Some(format!("Disc {disc}")), Some(track));
        }
    }
    let prefix = value.bytes().take_while(u8::is_ascii_alphabetic).count();
    if (1..=3).contains(&prefix) {
        let side = &value[..prefix];
        let track = &value[prefix..];
        let digits = track.bytes().take_while(u8::is_ascii_digit).count();
        let suffix = &track[digits..];
        if (digits > 0
            && (suffix.is_empty()
                || suffix.starts_with('.')
                || suffix.bytes().all(|byte| byte.is_ascii_alphabetic())))
            || (track.is_empty() && side.bytes().all(|byte| byte.is_ascii_uppercase()))
        {
            let ordinal = suffix.is_empty().then(|| integer(track)).flatten();
            return (Some(side.to_ascii_uppercase()), ordinal);
        }
    }
    // 1A is a tape style. A lowercase 1a can be a medley sub-track.
    if let Some((digits, side)) = value.split_at_checked(value.len().saturating_sub(1)) {
        if side.as_bytes().first().is_some_and(u8::is_ascii_uppercase) {
            if let Some(track) = integer(digits) {
                return (Some(side.to_owned()), Some(track));
            }
        }
    }
    (None, integer(value))
}

fn integer(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn parse_duration(value: &str) -> Option<u64> {
    // Discogs records mm:ss or hh:mm:ss. Its submission guide also permits
    // periods in place of colons, never fractional seconds.
    let value = value.trim();
    let separator = if value.contains(':') { ':' } else { '.' };
    let (first, rest) = value.split_once(separator)?;
    let (hours, minutes, seconds) = match rest.split_once(separator) {
        Some((minutes, seconds)) => {
            let minutes = integer(minutes)?;
            if minutes >= 60 {
                return None;
            }
            (integer(first)?, minutes, integer(seconds)?)
        }
        None => (0, integer(first)?, integer(rest)?),
    };
    if seconds >= 60 {
        return None;
    }
    let duration = hours
        .checked_mul(3600)?
        .checked_add(minutes.checked_mul(60)?)?
        .checked_add(seconds)?;
    (duration != 0).then_some(duration)
}

fn warn(warnings: &mut Vec<String>, warning: &'static str) {
    if !warnings.iter().any(|message| message == warning) {
        warnings.push(warning.into());
    }
}
