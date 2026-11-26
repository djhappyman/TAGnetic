use crate::FrameRange;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod aiff;
mod decode;
mod encode;
mod peaks;
mod probe;
mod seek;
mod treatment;

use decode::Recording;
use peaks::PeakBins;
pub use peaks::{Peak, WaveformIndex, WaveformPeaks};
pub use probe::{
    compare_preview_levels, learn_eq_threshold, match_reference, probe_treatment,
    probe_treatment_with_options, PlaybackHistory, PlaybackProbeWindow, PlaybackReadings,
    PreviewComparison, SpectralEqResponse, SpectrogramChannel, SpectrumView, TreatmentProbe,
    TreatmentProbeOptions,
};
pub use seek::SeekIndex;
pub use treatment::{
    prepare_treatment, AudioTreatment, DynamicEq, EqBand, EqBandKind, EqChannel, HumFilter,
    MonoMode, Polarity, PreparedTreatment, PreviewMonitor, RepairChannel, SpectralRepair,
    TreatmentProcessor, MAX_EQ_BANDS, MAX_SPECTRAL_REPAIRS,
};

const MAX_SEGMENTS: usize = 1_000;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("Audio operation stopped")]
    Cancelled,
    #[error("Unsupported audio: {0}")]
    Unsupported(&'static str),
    #[error("Invalid audio range: {0}")]
    Range(&'static str),
    #[error("Could not decode audio: {0}")]
    Decode(String),
    #[error("Could not encode audio: {0}")]
    Encode(String),
    #[error("Audio file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// None denotes a source without an integer sample precision, such as MP3.
    pub bits_per_sample: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameCount {
    Declared,
    Estimated,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbedSource {
    pub source: SourceInfo,
    pub frame_count: FrameCount,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisProgress {
    pub source: ProbedSource,
    pub scanned_frames: u64,
    /// Peaks cover the decoded prefix, from frame zero to scanned_frames.
    pub peaks: Vec<Peak>,
    pub rms: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereo_peaks: Option<Vec<[Peak; 2]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereo_rms: Option<Vec<[f32; 2]>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub source: SourceInfo,
    pub peaks: Vec<Peak>,
    pub rms: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereo_peaks: Option<Vec<[Peak; 2]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereo_rms: Option<Vec<[f32; 2]>>,
    #[serde(skip)]
    pub waveform: WaveformIndex,
    #[serde(skip)]
    pub seek_index: Option<Arc<SeekIndex>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum OutputFormat {
    Flac,
    /// Variable bitrate, targeting this average in kbps.
    Mp3 {
        bitrate_kbps: u16,
    },
}

#[derive(Clone, Debug)]
pub struct RenderSegment {
    pub range: FrameRange,
    /// Path in caller-owned staging storage. Existing files are refused.
    pub output: PathBuf,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderProgress {
    pub source_frame: u64,
    pub completed_frames: u64,
    pub total_frames: u64,
    pub completed_segments: usize,
    pub total_segments: usize,
}

fn check_cancelled(cancelled: &mut impl FnMut() -> bool) -> Result<(), MediaError> {
    if cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

/// Read the sample rate, channel count and any container duration quickly.
pub fn probe_file(source: File) -> Result<ProbedSource, MediaError> {
    Ok(Recording::open(source)?.probed_source())
}

/// A sequential preview opened on the same sample timeline as export.
/// Construct and read it off the real-time audio callback.
pub struct PreviewDecoder {
    recording: Recording,
    start: u64,
}

impl PreviewDecoder {
    pub fn open(
        source: File,
        start_frame: u64,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Self, MediaError> {
        Self::open_indexed(source, start_frame, None, cancelled)
    }

    /// Open a preview with the optional sparse index from this recording's analysis.
    pub fn open_indexed(
        source: File,
        start_frame: u64,
        index: Option<&SeekIndex>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, MediaError> {
        check_cancelled(&mut cancelled)?;
        if start_frame >= decode::MAX_FRAMES {
            return Err(MediaError::Range(
                "Preview starts beyond the supported timeline",
            ));
        }
        let recording = Recording::open_at(source, start_frame, index, &mut cancelled)?;
        Ok(Self {
            recording,
            start: start_frame,
        })
    }

    pub fn source_info(&self) -> SourceInfo {
        self.recording.info()
    }

    pub fn next_block(
        &mut self,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<&[i32]>, MediaError> {
        let channels = usize::from(self.source_info().channels);
        while self.recording.position() < self.start {
            let first = self.recording.position();
            // Check the next packet before returning it through the borrowed
            // decoder buffer. Keep only the first packet's leading discard.
            match self.recording.discard_before(self.start, &mut cancelled)? {
                Some(skip) => {
                    return Ok(Some(&self.recording.current_samples()[skip * channels..]));
                }
                None if self.recording.position() == first => {
                    return Err(MediaError::Range("Preview starts after the recording"));
                }
                None => {}
            }
        }
        self.recording.next(&mut cancelled)
    }
}

/// Inspect a recording and build a bounded overview cache in one decoding pass.
pub fn inspect(
    source: &Path,
    bins: usize,
    cancelled: impl FnMut() -> bool,
) -> Result<Analysis, MediaError> {
    inspect_file(File::open(source)?, bins, cancelled)
}

/// As [inspect], using an already held recording. The file is read from its start.
pub fn inspect_file(
    source: File,
    bins: usize,
    cancelled: impl FnMut() -> bool,
) -> Result<Analysis, MediaError> {
    analyze(source, bins, cancelled, None::<fn(AnalysisProgress)>)
}

/// Publish a header immediately and a waveform of the decoded prefix while
/// analysis runs. The returned [Analysis] is the verified, complete result.
pub fn inspect_file_with_progress(
    source: File,
    bins: usize,
    cancelled: impl FnMut() -> bool,
    progress: impl FnMut(AnalysisProgress),
) -> Result<Analysis, MediaError> {
    analyze(source, bins, cancelled, Some(progress))
}

fn analyze(
    input: File,
    bins: usize,
    mut cancelled: impl FnMut() -> bool,
    mut progress: Option<impl FnMut(AnalysisProgress)>,
) -> Result<Analysis, MediaError> {
    peaks::validate_bin_count(bins)?;
    check_cancelled(&mut cancelled)?;
    let mut recording = Recording::open_for_analysis(input, bins > 1)?;
    let probed = recording.probed_source();
    let mut source = probed.source;
    let channels = usize::from(source.channels);
    let mut waveform = WaveformIndex::default();
    let mut last_reported = Instant::now();
    if let Some(report) = &mut progress {
        report(AnalysisProgress {
            source: probed,
            scanned_frames: 0,
            peaks: Vec::new(),
            rms: Vec::new(),
            stereo_peaks: None,
            stereo_rms: None,
        });
    }
    while let Some(packet) = recording.next(&mut cancelled)? {
        waveform.push(packet, channels);
        if let Some(report) = &mut progress {
            if last_reported.elapsed() >= Duration::from_millis(250) {
                let scanned_frames = recording.position();
                let range = FrameRange {
                    start: 0,
                    end: scanned_frames,
                };
                let peaks = waveform.channel_peaks(range, bins)?;
                report(AnalysisProgress {
                    source: probed,
                    scanned_frames,
                    peaks: peaks.combined,
                    rms: peaks.rms,
                    stereo_peaks: peaks.stereo,
                    stereo_rms: peaks.stereo_rms,
                });
                last_reported = Instant::now();
            }
        }
    }
    source.frames = recording.position();
    if source.frames == 0 {
        return Err(MediaError::Decode(
            "The recording contains no audio frames".into(),
        ));
    }
    let peaks = waveform.channel_peaks(
        FrameRange {
            start: 0,
            end: source.frames,
        },
        bins,
    )?;
    let seek_index = recording
        .take_seek_builder()
        .map(|builder| {
            builder.finish(
                ProbedSource {
                    source,
                    frame_count: FrameCount::Declared,
                },
                &mut cancelled,
            )
        })
        .transpose()?
        .flatten()
        .map(Arc::new);
    Ok(Analysis {
        source,
        peaks: peaks.combined,
        rms: peaks.rms,
        stereo_peaks: peaks.stereo,
        stereo_rms: peaks.stereo_rms,
        waveform,
        seek_index,
    })
}

/// Compute a visible waveform tile over an exact range of decoded audio frames.
pub fn waveform(
    source: &Path,
    range: FrameRange,
    bins: usize,
    cancelled: impl FnMut() -> bool,
) -> Result<Vec<Peak>, MediaError> {
    waveform_file(File::open(source)?, range, bins, cancelled)
}

/// As [waveform], using an already held recording.
pub fn waveform_file(
    source: File,
    range: FrameRange,
    bins: usize,
    cancelled: impl FnMut() -> bool,
) -> Result<Vec<Peak>, MediaError> {
    waveform_file_indexed(source, range, bins, None, cancelled)
}

/// Compute a visible waveform tile with the optional index from this recording's analysis.
pub fn waveform_file_indexed(
    source: File,
    range: FrameRange,
    bins: usize,
    index: Option<&SeekIndex>,
    cancelled: impl FnMut() -> bool,
) -> Result<Vec<Peak>, MediaError> {
    Ok(waveform_channels_file_indexed(source, range, bins, index, cancelled)?.combined)
}

/// Compute a tile with independent channel peaks for stereo sources.
pub fn waveform_channels_file_indexed(
    source: File,
    range: FrameRange,
    bins: usize,
    index: Option<&SeekIndex>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<WaveformPeaks, MediaError> {
    let mut peaks = PeakBins::new(range, bins)?;
    check_cancelled(&mut cancelled)?;
    let mut recording = Recording::open_at(source, range.start, index, &mut cancelled)?;
    let channels = recording.info().channels as usize;
    while recording.position() < range.end {
        let start = recording.position();
        let packet = recording
            .next(&mut cancelled)?
            .ok_or(MediaError::Range("The tile extends past the recording"))?;
        let end = start + (packet.len() / channels) as u64;
        let first = start.max(range.start);
        let last = end.min(range.end);
        if first < last {
            let skip = (first - start) as usize * channels;
            let take = (last - first) as usize * channels;
            peaks.push(first, &packet[skip..skip + take], channels);
        }
    }
    Ok(peaks.finish(channels))
}

/// Check the requested codec can retain the source's rate and channel layout.
pub fn check_output(source: &SourceInfo, format: OutputFormat) -> Result<(), MediaError> {
    encode::check_output(source, format)
}

/// Integer depth written by the lossless encoder. Intermediate depths widen
/// without altering any integer samples. Decoded float and MP3 use 24 bits.
pub fn flac_output_bits(source: &SourceInfo) -> u8 {
    encode::flac_bits(source)
}

/// Render ordered, disjoint segments through one streaming pass over the source.
pub fn render(
    source: &Path,
    segments: &[RenderSegment],
    format: OutputFormat,
    cancelled: impl FnMut() -> bool,
    progress: impl FnMut(RenderProgress),
) -> Result<(), MediaError> {
    render_file(File::open(source)?, segments, format, cancelled, progress)
}

/// As [render], using a held source. All output artifacts remain in staging,
/// for the caller to publish or clean up. This operation never replaces files.
pub fn render_file(
    source: File,
    segments: &[RenderSegment],
    format: OutputFormat,
    cancelled: impl FnMut() -> bool,
    progress: impl FnMut(RenderProgress),
) -> Result<(), MediaError> {
    render_file_with_outputs(source, segments, format, cancelled, progress, |path| {
        OpenOptions::new().write(true).create_new(true).open(path)
    })
}

/// Render through caller-owned staging. The opener must create a new, writable,
/// seekable, empty file at each output path and refuse existing files.
pub fn render_file_with_outputs(
    source: File,
    segments: &[RenderSegment],
    format: OutputFormat,
    cancelled: impl FnMut() -> bool,
    progress: impl FnMut(RenderProgress),
    open: impl FnMut(&Path) -> std::io::Result<File>,
) -> Result<(), MediaError> {
    render_treated_file(
        source,
        segments,
        format,
        PreparedTreatment::default(),
        cancelled,
        progress,
        open,
    )
}

/// Render prepared audio through the same streaming processor used by preview.
pub fn render_treated_file(
    source: File,
    segments: &[RenderSegment],
    format: OutputFormat,
    treatment: PreparedTreatment,
    mut cancelled: impl FnMut() -> bool,
    mut progress: impl FnMut(RenderProgress),
    mut open: impl FnMut(&Path) -> std::io::Result<File>,
) -> Result<(), MediaError> {
    if segments.is_empty() || segments.len() > MAX_SEGMENTS {
        return Err(MediaError::Range("Choose from 1 to 1,000 output tracks"));
    }
    let mut last = 0;
    let mut total_frames = 0;
    let mut paths = HashSet::with_capacity(segments.len());
    for segment in segments {
        if segment.range.start < last
            || segment.range.start >= segment.range.end
            || segment.range.end > decode::MAX_FRAMES
        {
            return Err(MediaError::Range(
                "Output ranges must be ordered, disjoint and nonempty",
            ));
        }
        if !paths.insert(&segment.output) {
            return Err(MediaError::Range("Output paths must be distinct"));
        }
        last = segment.range.end;
        total_frames += segment.range.end - segment.range.start;
    }

    check_cancelled(&mut cancelled)?;
    let mut recording = Recording::open(source)?;
    let mut processor = treatment.processor(recording.info())?;
    let info = processor.output_info();
    check_output(&info, format)?;
    let channels = info.channels as usize;
    let mut processed = Vec::new();
    let mut next = 0;
    let mut completed_frames = 0;
    let mut reported_frames = 0;
    let mut encoder = None;

    while next < segments.len() {
        let start = recording.position();
        let packet = recording
            .next(&mut cancelled)?
            .ok_or(MediaError::Range("An output extends past the recording"))?;
        let packet = if treatment.settings().is_active() {
            processor.process(packet, &mut processed);
            &processed
        } else {
            packet
        };
        let end = start + (packet.len() / channels) as u64;
        while let Some(segment) = segments
            .get(next)
            .filter(|segment| segment.range.start < end)
        {
            let first = start.max(segment.range.start);
            let last = end.min(segment.range.end);
            if encoder.is_none() {
                check_cancelled(&mut cancelled)?;
                encoder = Some(encode::SegmentWriter::new(
                    open(&segment.output)?,
                    &info,
                    format,
                )?);
            }
            let writer = encoder
                .as_mut()
                .ok_or_else(|| MediaError::Encode("Output writer is unavailable".into()))?;
            let skip = (first - start) as usize * channels;
            let count = (last - first) as usize * channels;
            writer.push(&packet[skip..skip + count], &mut cancelled)?;
            completed_frames += last - first;
            let completed_segment = last == segment.range.end;
            if completed_segment {
                let writer = encoder
                    .take()
                    .ok_or_else(|| MediaError::Encode("Output writer is unavailable".into()))?;
                writer.finish(&mut cancelled)?;
                next += 1;
            }
            if completed_segment
                || completed_frames - reported_frames >= (u64::from(info.sample_rate) / 10).max(1)
            {
                progress(RenderProgress {
                    source_frame: last,
                    completed_frames,
                    total_frames,
                    completed_segments: next,
                    total_segments: segments.len(),
                });
                reported_frames = completed_frames;
            }
            if !completed_segment {
                break;
            }
        }
    }
    check_cancelled(&mut cancelled)
}
