//! Sample-accurate split plans and editable estimates from external track lists.
//! Neither importer opens a path or changes a recording.

mod cue;
mod provider;

pub use cue::{
    parse_cue, write_cue, CueEncoding, CueExport, CueExportTrack, CueMarker, CueSheet, CueTags,
    CueTrack,
};
pub use provider::{
    provider_markers, ProviderAnchor, ProviderMarker, ProviderProposal, ProviderSpan,
    ProviderTrack, ProviderTrackKind,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A half-open range of frames in the decoded source, before any resampling.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRange {
    pub start: u64,
    pub end: u64,
}

/// Marker input failures, with CUE line numbers where available.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MarkerError {
    #[error("The recording must have a nonzero sample rate and decoded duration.")]
    InvalidTimeline,
    #[error("At least one output range is required.")]
    NoRanges,
    #[error("Track {track} has an empty or backwards range.")]
    EmptyRange { track: usize },
    #[error("Track {track} extends beyond the recording.")]
    RangeOutsideSource { track: usize },
    #[error("Track {track} overlaps or starts before an earlier track.")]
    OverlappingRange { track: usize },
    #[error("A single-file CUE cannot represent the excluded audio. Save the split draft or export tracks instead.")]
    CueExclusions,
    #[error("This CUE contains more than 99 tracks.")]
    CueTrackLimit,
    #[error("CUE is larger than the 1 MiB import limit.")]
    CueInputLimit,
    #[error("Could not decode the CUE sheet. Use UTF-8, UTF-16 with a byte order mark, or explicitly select Windows-1252.")]
    CueEncoding,
    #[error("CUE line {line}: {reason}")]
    CueLine { line: usize, reason: &'static str },
    #[error("CUE must contain one FILE declaration and at least one AUDIO track.")]
    CueStructure,
    #[error("Track {track} needs a valid, ordered CUE start and pregap within the recording.")]
    CueBoundary { track: usize },
    #[error(
        "Track {track} overlaps another cut or the end after rounding to the nearest CUE frame."
    )]
    CueRounding { track: usize },
    #[error("The recording name must be WAV, AIFF, MP3 or FLAC and contain no quotes or control characters.")]
    CueSourceName,
    #[error("CUE metadata contains a control character or exceeds the output size limit.")]
    CueMetadata,
    #[error("The track list exceeds the supported size or nesting depth.")]
    ProviderLimit,
    #[error("Track list anchor {anchor} must cover a nonempty, ordered range of tracks and start inside the recording after any earlier anchor.")]
    ProviderAnchor { anchor: usize },
    #[error("The timestamp is too large.")]
    TimeOverflow,
}

/// Check that all output ranges are positive, ordered, disjoint and bounded.
/// Gaps are permitted for audio export. CUE export checks contiguity separately.
///
/// ```
/// use tagrex_audio_workbench::markers::{validate_ranges, FrameRange};
/// assert!(validate_ranges(&[FrameRange { start: 0, end: 8 }], 8).is_ok());
/// assert!(validate_ranges(&[FrameRange { start: 0, end: 9 }], 8).is_err());
/// ```
pub fn validate_ranges(ranges: &[FrameRange], total_frames: u64) -> Result<(), MarkerError> {
    if ranges.is_empty() {
        return Err(MarkerError::NoRanges);
    }
    let mut previous_end = 0;
    for (index, range) in ranges.iter().enumerate() {
        let track = index + 1;
        if range.start >= range.end {
            return Err(MarkerError::EmptyRange { track });
        }
        if range.end > total_frames {
            return Err(MarkerError::RangeOutsideSource { track });
        }
        if range.start < previous_end {
            return Err(MarkerError::OverlappingRange { track });
        }
        previous_end = range.end;
    }
    Ok(())
}

fn check_timeline(sample_rate: u32, total_frames: u64) -> Result<(), MarkerError> {
    if sample_rate == 0 || total_frames == 0 {
        return Err(MarkerError::InvalidTimeline);
    }
    Ok(())
}

// Round absolute positions to the nearest tick, with halfway ties forward.
// A wide intermediate lets callers detect overflow instead of truncating.
fn scale_time(value: u64, numerator: u32, denominator: u32) -> Result<u64, MarkerError> {
    let denominator = u128::from(denominator);
    let result = (u128::from(value) * u128::from(numerator) + denominator / 2) / denominator;
    u64::try_from(result).map_err(|_| MarkerError::TimeOverflow)
}
