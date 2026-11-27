mod comparison;
pub use comparison::{compare_preview_levels, PreviewComparison};
mod eq_threshold;
mod playback;
mod reference;
pub use eq_threshold::learn_eq_threshold;
pub use playback::{PlaybackHistory, PlaybackProbeWindow, PlaybackReadings};
pub use reference::match_reference;

use super::restoration::suggest_clicks;
use super::treatment::dry_monitor_sample;
use super::{
    check_cancelled, MediaError, PreparedTreatment, PreviewDecoder, SeekIndex, SourceInfo,
};
use crate::FrameRange;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fs::File;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

const FLOOR_DB: f32 = -96.0;
const PCM_SCALE: f32 = 2_147_483_648.0;
const BAND_COUNT: usize = 112;
const FOCUSED_BAND_COUNT: usize = 256;
const REDUCTION_POINTS: usize = 48;
const SPECTROGRAM_COLUMNS: u64 = 32;
const FOCUSED_FFT_SAMPLE_BUDGET: u64 = 3_000_000;
const MAX_VISUAL_SAMPLES: u64 = 4_000_000;
const DECODE_BUDGET: Duration = Duration::from_millis(650);

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SpectrogramChannel {
    #[default]
    Combined,
    Left,
    Right,
    Stereo,
}

impl SpectrogramChannel {
    fn index(self, channels: usize) -> Option<usize> {
        match self {
            Self::Combined | Self::Stereo => None,
            Self::Left => Some(0),
            Self::Right => Some(1.min(channels - 1)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SpectrogramDetail {
    #[default]
    Balanced,
    Transients,
    Tones,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpectrogramFocus {
    pub low_hz: u32,
    pub high_hz: u32,
}

impl SpectrogramFocus {
    fn narrow(self) -> bool {
        self.high_hz <= self.low_hz.saturating_mul(4)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SpectrumView {
    Before,
    After,
    #[default]
    Both,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TreatmentProbeOptions {
    pub spectrum: bool,
    pub spectrum_view: SpectrumView,
    pub spectrogram: bool,
    pub viewport: Option<FrameRange>,
    pub spectrogram_channel: SpectrogramChannel,
    pub spectrogram_treated: bool,
    pub spectrogram_difference: bool,
    pub spectrogram_detail: SpectrogramDetail,
    pub spectrogram_focus: Option<SpectrogramFocus>,
}

impl TreatmentProbeOptions {
    pub fn spectrogram_only(self) -> bool {
        self.spectrogram && !self.spectrum
    }

    pub fn dry_spectrogram(self) -> bool {
        self.spectrogram_only() && !self.spectrogram_treated && !self.spectrogram_difference
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectralEqResponse {
    pub channel: crate::EqChannel,
    pub bin_hz: f32,
    pub first_bin: usize,
    pub gains_db: Vec<f32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreatmentProbe {
    pub frequencies: Vec<f32>,
    pub original: Vec<f32>,
    pub treated: Vec<f32>,
    /// Compressor gain in dB, oldest to newest.
    pub reduction: Vec<f32>,
    pub input: f32,
    pub output: f32,
    pub input_peak: f32,
    pub eq_input_peak: f32,
    pub output_peak: f32,
    pub output_overload: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_playback: bool,
    /// Linked compressor detector level at the requested frame, in dBFS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detector: Option<f32>,
    /// Quiet-gap gain in dB, oldest to newest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_reduction: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dynamic_eq_mix: Vec<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spectral_response: Vec<SpectralEqResponse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dynamic_eq_detectors: Vec<Option<f32>>,
    pub eq_threshold_learning: bool,
    pub upward_eq: bool,
    pub dynamic_all_shapes: bool,
    pub dynamic_ratio: bool,
    pub spectral_eq: bool,
    /// Original audio, with one frequency-band array per column in time order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram: Option<Vec<Vec<f32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_start: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_end: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_channel: Option<SpectrogramChannel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_treated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_waveform: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_difference: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_detail: Option<SpectrogramDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_focus: Option<SpectrogramFocus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_window_frames: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_left: Option<Vec<Vec<f32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_right: Option<Vec<Vec<f32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_left_waveform: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_right_waveform: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectrogram_clicks: Option<Vec<super::restoration::ClickSuggestion>>,
}

/// Read a bounded local window. The treatment uses the same recipe and
/// interleaved PCM as native audition.
pub fn probe_treatment(
    source: File,
    info: SourceInfo,
    frame: u64,
    index: Option<&SeekIndex>,
    prepared: PreparedTreatment,
    spectrogram: bool,
    cancelled: impl FnMut() -> bool,
) -> Result<TreatmentProbe, MediaError> {
    probe_treatment_with_options(
        source,
        info,
        frame,
        index,
        prepared,
        TreatmentProbeOptions {
            spectrum: true,
            spectrogram,
            ..TreatmentProbeOptions::default()
        },
        cancelled,
    )
}

pub fn probe_treatment_with_options(
    source: File,
    info: SourceInfo,
    frame: u64,
    index: Option<&SeekIndex>,
    prepared: PreparedTreatment,
    options: TreatmentProbeOptions,
    mut cancelled: impl FnMut() -> bool,
) -> Result<TreatmentProbe, MediaError> {
    if frame >= info.frames || info.sample_rate == 0 || info.channels == 0 {
        return Err(MediaError::Range(
            "Audio analysis starts outside the recording",
        ));
    }
    let rate = u64::from(info.sample_rate);
    let viewport = options.viewport.filter(|_| options.spectrogram_only());
    let max_hz = (rate * 45 / 100).min(20_000) as u32;
    let options = TreatmentProbeOptions {
        spectrogram_focus: options.spectrogram_focus.filter(|focus| {
            viewport.is_some()
                && focus.low_hz >= 20
                && focus.high_hz > focus.low_hz
                && focus.high_hz <= max_hz
        }),
        ..options
    };
    if let Some(range) = viewport {
        if range.start >= range.end || range.end > info.frames {
            return Err(MediaError::Range(
                "Spectrogram view is outside the recording",
            ));
        }
        if range.end - range.start > (rate * 12).min(MAX_VISUAL_SAMPLES / u64::from(info.channels))
        {
            return probe_overview(
                source,
                info,
                index,
                prepared,
                options,
                range,
                &mut cancelled,
            );
        }
    }
    probe_window(
        source, info, frame, index, prepared, options, false, cancelled,
    )
}

fn probe_overview(
    source: File,
    info: SourceInfo,
    index: Option<&SeekIndex>,
    prepared: PreparedTreatment,
    options: TreatmentProbeOptions,
    range: FrameRange,
    mut cancelled: impl FnMut() -> bool,
) -> Result<TreatmentProbe, MediaError> {
    // Uniform short windows keep decode and DSP work proportional to display
    // resolution. They use the same seek, priming and channel mapping as detail.
    let detailed = prepared.settings().is_active() && !options.dry_spectrogram();
    let count = if detailed { 96 } else { 192 };
    let width = u64::from(match options.spectrogram_detail {
        SpectrogramDetail::Balanced => {
            balanced_fft_size(info.sample_rate, options.spectrogram_focus)
        }
        SpectrogramDetail::Tones => probe_fft_size(info.sample_rate)
            .saturating_mul(4)
            .min(32_768),
        SpectrogramDetail::Transients => {
            (info.sample_rate / 48).next_power_of_two().clamp(256, 4096)
        }
    });
    let deadline = Instant::now() + Duration::from_secs(12);
    let span = range.end - range.start;
    let mut combined = Vec::with_capacity(count as usize);
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut waveform = Vec::with_capacity(count as usize * 2);
    let mut left_waveform = Vec::new();
    let mut right_waveform = Vec::new();
    let mut result = None;
    for column in 0..count {
        check_cancelled(&mut cancelled)?;
        if Instant::now() >= deadline {
            return Err(MediaError::Range(
                "Spectrogram overview exceeded its analysis budget",
            ));
        }
        let frame = range.start + (2 * column + 1) * span / (2 * count);
        let sample = centered(frame, width, info.frames);
        let mut reading = probe_window(
            source.try_clone()?,
            info,
            frame,
            index,
            prepared.clone(),
            TreatmentProbeOptions {
                viewport: Some(sample),
                ..options
            },
            true,
            || cancelled() || Instant::now() >= deadline,
        )?;
        combined.extend(reading.spectrogram.take().into_iter().flatten());
        left.extend(reading.spectrogram_left.take().into_iter().flatten());
        right.extend(reading.spectrogram_right.take().into_iter().flatten());
        for (output, peaks) in [
            (&mut waveform, reading.spectrogram_waveform.take()),
            (&mut left_waveform, reading.spectrogram_left_waveform.take()),
            (
                &mut right_waveform,
                reading.spectrogram_right_waveform.take(),
            ),
        ] {
            if let Some(peaks) = peaks {
                output.extend([
                    peaks.iter().step_by(2).copied().fold(0.0_f32, f32::min),
                    peaks
                        .iter()
                        .skip(1)
                        .step_by(2)
                        .copied()
                        .fold(0.0_f32, f32::max),
                ]);
            }
        }
        result = Some(reading);
    }
    let mut result = result.ok_or(MediaError::Range("No spectrogram overview data"))?;
    result.spectrogram = Some(combined);
    result.spectrogram_left = (!left.is_empty()).then_some(left);
    result.spectrogram_right = (!right.is_empty()).then_some(right);
    result.spectrogram_waveform = Some(waveform);
    result.spectrogram_left_waveform = (!left_waveform.is_empty()).then_some(left_waveform);
    result.spectrogram_right_waveform = (!right_waveform.is_empty()).then_some(right_waveform);
    result.spectrogram_start = Some(range.start);
    result.spectrogram_end = Some(range.end);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn probe_window(
    source: File,
    info: SourceInfo,
    frame: u64,
    index: Option<&SeekIndex>,
    prepared: PreparedTreatment,
    options: TreatmentProbeOptions,
    overview_column: bool,
    mut cancelled: impl FnMut() -> bool,
) -> Result<TreatmentProbe, MediaError> {
    let rate = u64::from(info.sample_rate);
    let channels = usize::from(info.channels);
    let base_fft_size = probe_fft_size(info.sample_rate);
    let viewport = options.viewport.filter(|_| options.spectrogram_only());
    let focus = options.spectrogram_focus;
    let balanced_max = balanced_fft_size(info.sample_rate, focus);
    let window_divisor = if focus.is_some_and(SpectrogramFocus::narrow) {
        4
    } else {
        8
    };
    let balanced_size = viewport.map_or(base_fft_size, |range| {
        (((range.end - range.start) / window_divisor).max(2_048) as u32)
            .next_power_of_two()
            .min(balanced_max)
    });
    let fft_size = if options.spectrogram_only() {
        match options.spectrogram_detail {
            SpectrogramDetail::Balanced if overview_column => balanced_max,
            SpectrogramDetail::Balanced => balanced_size,
            SpectrogramDetail::Transients => {
                (info.sample_rate / 48).next_power_of_two().clamp(256, 4096)
            }
            SpectrogramDetail::Tones => base_fft_size.saturating_mul(4).min(32_768),
        }
    } else {
        balanced_size
    };
    let center = viewport.map_or(frame, |range| range.start + (range.end - range.start) / 2);
    let spectrum = centered(center, u64::from(fft_size), info.frames);
    let history = FrameRange {
        start: frame.saturating_sub(rate * 3 / 4),
        end: frame + 1,
    };
    let prepared = if options.dry_spectrogram() {
        PreparedTreatment::default()
    } else {
        prepared
    };
    let active = prepared.settings().is_active();
    let track_compressor = !options.spectrogram_only() && prepared.settings().compressor_db != 0;
    let track_gate = !options.spectrogram_only() && prepared.settings().gate_db != 0;
    let track_dynamics = track_compressor || track_gate;
    let track_eq = !options.spectrogram_only()
        && prepared
            .settings()
            .bands()
            .any(|band| !band.bypassed && band.dynamic.is_some());
    let keep_raw = if let Some(range) = viewport {
        range
    } else if options.spectrogram {
        let width = (rate * 7 / 2)
            .min(MAX_VISUAL_SAMPLES / u64::from(info.channels))
            .max(u64::from(fft_size));
        centered(frame, width, info.frames)
    } else {
        spectrum
    };
    let show_wet = options.spectrogram_treated || options.spectrogram_difference;
    let keep_wet = if options.spectrogram && show_wet {
        keep_raw
    } else {
        spectrum
    };
    let treatment_start = if track_dynamics {
        keep_wet.start.min(history.start)
    } else {
        keep_wet.start
    };
    let treatment_end = if track_dynamics {
        keep_wet.end.max(history.end)
    } else {
        keep_wet.end
    };
    let warmup = prepared
        .preview_start(treatment_start, info.sample_rate)
        .max(treatment_start.saturating_sub(rate * 5));
    let (mut cursor, end) = if options.dry_spectrogram() {
        (keep_raw.start, keep_raw.end)
    } else {
        (keep_raw.start.min(warmup), keep_raw.end.max(treatment_end))
    };
    // Off-transport probes replay DSP history. A high-rate spectral chain
    // may need longer than the ordinary interactive decode budget.
    let budget = if prepared.settings().has_spectral_eq() {
        Duration::from_secs(if show_wet { 8 } else { 3 })
    } else {
        DECODE_BUDGET
    };
    let deadline = Instant::now() + budget;
    let mut stop_decoding = || cancelled() || Instant::now() >= deadline;
    let dry_settings = super::AudioTreatment::default();
    let mask_settings = if options.dry_spectrogram() {
        &dry_settings
    } else {
        prepared.settings()
    };
    let mut decoder =
        super::RepairSource::open(source, cursor, index, mask_settings, &mut stop_decoding)?;
    if decoder.source_info().sample_rate != info.sample_rate
        || decoder.source_info().channels != info.channels
    {
        return Err(MediaError::Range("Audio parameters changed"));
    }
    let mut processor = prepared.processor_at(info, warmup)?;
    let treated_channels = usize::from(processor.output_info().channels);
    let mut original_pcm = Vec::with_capacity((keep_raw.end - keep_raw.start) as usize * channels);
    let mut treated_pcm = Vec::with_capacity(if active {
        (keep_wet.end - keep_wet.start) as usize * treated_channels
    } else {
        0
    });
    let mut processed = Vec::new();
    let mut reduction = vec![0.0; REDUCTION_POINTS];
    let mut gate_reduction = track_gate.then(|| vec![0.0; REDUCTION_POINTS]);
    let mut detector = None;
    let mut dynamic_eq_mix = Vec::new();
    let mut spectral_response = Vec::new();
    let mut dynamic_eq_detectors = Vec::new();
    let mut unclipped_peak = 0.0_f64;
    let track_peaks = !options.spectrogram_only();
    let reduction_step = (history.end - history.start).div_ceil(REDUCTION_POINTS as u64);

    while cursor < end {
        check_cancelled(&mut stop_decoding)?;
        let Some(block) = decoder.next_block(&mut stop_decoding)? else {
            break;
        };
        let next = (cursor + (block.dry.len() / channels) as u64).min(end);
        if let Some(span) = overlap(cursor, next, keep_raw) {
            original_pcm.extend_from_slice(&block.dry[interleaved(span, cursor, channels)]);
        }
        // A dry spectrogram can begin before the treatment's priming window.
        let mut first = cursor.max(warmup);
        let process_end = next.min(treatment_end);
        while active && first < process_end {
            let last = if track_dynamics && first < history.end {
                let boundary = if first < history.start {
                    history.start
                } else {
                    history.start + ((first - history.start) / reduction_step + 1) * reduction_step
                };
                process_end.min(boundary)
            } else {
                process_end
            };
            let last = if track_eq && first <= frame {
                last.min(frame + 1)
            } else {
                last
            };
            let last = if track_peaks && first < spectrum.start {
                last.min(spectrum.start)
            } else if track_peaks && first < spectrum.end {
                last.min(spectrum.end)
            } else {
                last
            };
            let span = FrameRange {
                start: first,
                end: last,
            };
            let input = &block.samples()[interleaved(span, cursor, channels)];
            if track_peaks && (spectrum.start..spectrum.end).contains(&first) {
                unclipped_peak =
                    unclipped_peak.max(processor.process_with_peak(input, &mut processed));
            } else {
                processor.process(input, &mut processed);
            }
            if let Some(samples) = overlap(first, last, keep_wet) {
                treated_pcm
                    .extend_from_slice(&processed[interleaved(samples, first, treated_channels)]);
            }
            if track_dynamics && first >= history.start && first < history.end {
                let bucket = ((first - history.start) / reduction_step) as usize;
                if track_compressor {
                    reduction[bucket] = processor.compression_gain_db();
                    detector = processor.dynamics_detector_db();
                }
                if let Some(gains) = &mut gate_reduction {
                    gains[bucket] = processor.gate_gain_db();
                }
            }
            if track_eq && last == frame + 1 {
                (dynamic_eq_mix, dynamic_eq_detectors) = processor.dynamic_eq_readings();
                spectral_response = processor.spectral_eq_response();
            }
            first = last;
        }
        cursor = next;
    }
    let visual_end = keep_raw.start + (original_pcm.len() / channels) as u64;
    let seen = overlap(keep_raw.start, visual_end, spectrum).ok_or(MediaError::Range(
        "No decoded samples at the analysis position",
    ))?;
    let original_slice = &original_pcm[interleaved(seen, keep_raw.start, channels)];
    let treated_end = keep_wet.start + (treated_pcm.len() / treated_channels) as u64;
    let treated_slice = overlap(keep_wet.start, treated_end, spectrum).map_or(&[][..], |span| {
        &treated_pcm[interleaved(span, keep_wet.start, treated_channels)]
    });
    let mut frequencies = Vec::new();
    let mut original = Vec::new();
    let mut treated = Vec::new();
    let mut spectrogram = None;
    let mut spectrogram_waveform = None;
    let mut spectrogram_left = None;
    let mut spectrogram_right = None;
    let mut spectrogram_left_waveform = None;
    let mut spectrogram_right_waveform = None;
    let difference = options.spectrogram_difference.then(|| {
        if active {
            let mono = prepared.settings().mono;
            let swapped = prepared.settings().swap_channels;
            original_pcm
                .chunks_exact(channels)
                .zip(treated_pcm.chunks_exact(treated_channels))
                .flat_map(|(dry, wet)| {
                    wet.iter().enumerate().map(move |(lane, sample)| {
                        dry_monitor_sample(dry, lane, mono, swapped).saturating_sub(*sample)
                    })
                })
                .collect::<Vec<_>>()
        } else {
            vec![0; original_pcm.len()]
        }
    });
    if options.spectrum || options.spectrogram {
        with_spectrum_focus(fft_size as usize, info.sample_rate, focus, |analyzer| {
            if options.spectrum {
                (original, treated) = spectrum_views(
                    analyzer,
                    (original_slice, channels),
                    (treated_slice, treated_channels),
                    active,
                    options.spectrum_view,
                    &mut cancelled,
                )?;
            }
            if options.spectrogram {
                let (visual_pcm, visual_channels) = if let Some(pcm) = &difference {
                    (&pcm[..], treated_channels)
                } else if active && show_wet {
                    (&treated_pcm[..], treated_channels)
                } else {
                    (&original_pcm[..], channels)
                };
                let span = (visual_pcm.len() / visual_channels) as u64;
                let mut channel = options.spectrogram_channel.index(visual_channels);
                let swapped = show_wet && visual_channels == 2 && prepared.settings().swap_channels;
                if swapped {
                    // Repair selections refer to a source channel, even when the output swaps it.
                    channel = channel.map(|index| 1 - index);
                }
                let max_columns = if overview_column {
                    1
                } else if focus.is_some() {
                    (FOCUSED_FFT_SAMPLE_BUDGET / u64::from(fft_size)).min(512)
                } else if options.spectrogram_detail == SpectrogramDetail::Transients {
                    128
                } else if viewport.is_some() {
                    64
                } else {
                    SPECTROGRAM_COLUMNS
                };
                let stride = if focus.is_some() {
                    u64::from((fft_size / 16).clamp(32, 256))
                } else if options.spectrogram_detail == SpectrogramDetail::Transients {
                    u64::from(fft_size / 4)
                } else if viewport.is_some() {
                    256
                } else {
                    u64::from(fft_size) / 2
                };
                let columns = max_columns.min(span.div_ceil(stride));
                let mut values = Vec::with_capacity(columns as usize);
                let stereo = (options.spectrogram_channel == SpectrogramChannel::Stereo
                    && visual_channels == 2)
                    .then(|| {
                        [
                            Vec::with_capacity(columns as usize),
                            Vec::with_capacity(columns as usize),
                        ]
                    });
                let mut lanes = stereo;
                for column in 0..columns {
                    let at = (2 * column + 1) * span / (2 * columns);
                    let relative = centered(at, u64::from(fft_size), span);
                    let region = interleaved(relative, 0, visual_channels);
                    values.push(analyzer.measure_channel(
                        &visual_pcm[region.clone()],
                        visual_channels,
                        channel,
                        &mut cancelled,
                    )?);
                    if let Some(lanes) = &mut lanes {
                        for (lane, values) in lanes.iter_mut().enumerate() {
                            values.push(analyzer.measure_channel(
                                &visual_pcm[region.clone()],
                                visual_channels,
                                Some(lane ^ usize::from(swapped)),
                                &mut cancelled,
                            )?);
                        }
                    }
                }
                spectrogram = Some(values);
                spectrogram_waveform = Some(waveform_samples(visual_pcm, visual_channels, channel));
                if let Some([left, right]) = lanes {
                    spectrogram_left = Some(left);
                    spectrogram_right = Some(right);
                    spectrogram_left_waveform = Some(waveform_samples(
                        visual_pcm,
                        visual_channels,
                        Some(usize::from(swapped)),
                    ));
                    spectrogram_right_waveform = Some(waveform_samples(
                        visual_pcm,
                        visual_channels,
                        Some(usize::from(!swapped)),
                    ));
                }
            }
            frequencies = analyzer.frequencies.clone();
            Ok::<(), MediaError>(())
        })?;
    }
    check_cancelled(&mut cancelled)?;
    let populated =
        (cursor.min(history.end).saturating_sub(history.start)).div_ceil(reduction_step) as usize;
    reduction.truncate(populated);
    if let Some(gains) = &mut gate_reduction {
        gains.truncate(populated);
    }
    let (input, input_peak) = level_db(original_slice);
    let (output, output_peak) = if active {
        level_db(treated_slice)
    } else {
        (input, input_peak)
    };
    let spectrogram_clicks = (options.spectrogram
        && !overview_column
        && !show_wet
        && options.spectrogram_detail == SpectrogramDetail::Transients)
        .then(|| {
            suggest_clicks(
                &original_pcm,
                channels,
                options.spectrogram_channel.index(channels),
                keep_raw.start,
            )
        });
    check_cancelled(&mut cancelled)?;
    Ok(TreatmentProbe {
        frequencies,
        original,
        treated,
        reduction,
        input,
        output,
        input_peak,
        eq_input_peak: amplitude_db(
            f64::from(
                original_slice
                    .iter()
                    .map(|sample| sample.unsigned_abs())
                    .max()
                    .unwrap_or(0),
            ) / f64::from(PCM_SCALE)
                * 10.0_f64.powf(f64::from(prepared.settings().eq_input_db) / 20.0),
        ),
        output_peak,
        output_overload: unclipped_peak > 1.0,
        from_playback: false,
        detector,
        gate_reduction,
        dynamic_eq_mix,
        spectral_response,
        dynamic_eq_detectors,
        eq_threshold_learning: true,
        upward_eq: true,
        dynamic_all_shapes: true,
        dynamic_ratio: true,
        spectral_eq: true,
        spectrogram,
        spectrogram_start: options.spectrogram.then_some(keep_raw.start),
        spectrogram_end: options.spectrogram.then_some(visual_end),
        spectrogram_channel: options.spectrogram.then_some(options.spectrogram_channel),
        spectrogram_treated: options.spectrogram.then_some(options.spectrogram_treated),
        spectrogram_waveform,
        spectrogram_difference: options
            .spectrogram
            .then_some(options.spectrogram_difference),
        spectrogram_detail: options.spectrogram.then_some(options.spectrogram_detail),
        spectrogram_focus: focus,
        spectrogram_window_frames: options
            .spectrogram
            .then_some(fft_size.min((visual_end - keep_raw.start) as u32)),
        spectrogram_left,
        spectrogram_right,
        spectrogram_left_waveform,
        spectrogram_right_waveform,
        spectrogram_clicks,
    })
}

fn probe_fft_size(rate: u32) -> u32 {
    (rate / 6).next_power_of_two().clamp(2_048, 32_768)
}

fn balanced_fft_size(rate: u32, focus: Option<SpectrogramFocus>) -> u32 {
    let size = probe_fft_size(rate);
    if focus.is_some_and(SpectrogramFocus::narrow) {
        (size * 2).min(32_768)
    } else {
        size
    }
}

type SpectrumKey = (usize, u32, Option<SpectrogramFocus>);

thread_local! {
    static SPECTRUM_CACHE: RefCell<Option<(SpectrumKey, Spectrum)>> = const { RefCell::new(None) };
}

fn with_spectrum<R>(size: usize, rate: u32, run: impl FnOnce(&mut Spectrum) -> R) -> R {
    with_spectrum_focus(size, rate, None, run)
}

fn with_spectrum_focus<R>(
    size: usize,
    rate: u32,
    focus: Option<SpectrogramFocus>,
    run: impl FnOnce(&mut Spectrum) -> R,
) -> R {
    SPECTRUM_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (size, rate, focus);
        if cache.as_ref().is_none_or(|(cached, _)| *cached != key) {
            *cache = Some((key, Spectrum::focused(size, rate, focus)));
        }
        run(&mut cache.as_mut().expect("initialized spectrum cache").1)
    })
}

fn spectrum_views(
    analyzer: &mut Spectrum,
    dry: (&[i32], usize),
    wet: (&[i32], usize),
    active: bool,
    view: SpectrumView,
    stopped: &mut impl FnMut() -> bool,
) -> Result<(Vec<f32>, Vec<f32>), MediaError> {
    let before = if view != SpectrumView::After {
        analyzer.measure(dry.0, dry.1, stopped)?
    } else {
        Vec::new()
    };
    let after = if view == SpectrumView::Before {
        Vec::new()
    } else if active {
        analyzer.measure(wet.0, wet.1, stopped)?
    } else if before.is_empty() {
        analyzer.measure(dry.0, dry.1, stopped)?
    } else {
        before.clone()
    };
    Ok((before, after))
}

fn centered(frame: u64, width: u64, total: u64) -> FrameRange {
    let width = width.min(total);
    let start = frame.saturating_sub(width / 2).min(total - width);
    FrameRange {
        start,
        end: start + width,
    }
}

fn overlap(start: u64, end: u64, range: FrameRange) -> Option<FrameRange> {
    let start = start.max(range.start);
    let end = end.min(range.end);
    (start < end).then_some(FrameRange { start, end })
}

fn interleaved(span: FrameRange, offset: u64, channels: usize) -> Range<usize> {
    (span.start - offset) as usize * channels..(span.end - offset) as usize * channels
}

fn power_db(power: f32) -> f32 {
    (10.0 * power.max(1e-10).log10()).clamp(FLOOR_DB, 0.0)
}

pub(super) fn amplitude_db(amplitude: f64) -> f32 {
    (20.0 * amplitude.log10()).max(f64::from(FLOOR_DB)) as f32
}

fn level_db(samples: &[i32]) -> (f32, f32) {
    let mut power = 0.0;
    let mut peak = 0;
    for &sample in samples {
        power += (f64::from(sample) / f64::from(PCM_SCALE)).powi(2);
        peak = peak.max(sample.unsigned_abs());
    }
    let peak_power = (peak as f32 / PCM_SCALE).powi(2);
    (
        power_db((power / samples.len().max(1) as f64) as f32),
        power_db(peak_power),
    )
}

fn waveform_samples(samples: &[i32], channels: usize, channel: Option<usize>) -> Vec<f32> {
    const BINS: usize = 256;
    let frames = samples.len() / channels;
    let count = BINS.min(frames);
    let mut output = Vec::with_capacity(count * 2);
    for bin in 0..count {
        let start = bin * frames / count;
        let end = (bin + 1) * frames / count;
        let mut low = i32::MAX;
        let mut high = i32::MIN;
        for frame in samples[start * channels..end * channels].chunks_exact(channels) {
            if let Some(index) = channel {
                low = low.min(frame[index]);
                high = high.max(frame[index]);
            } else {
                for &sample in frame {
                    low = low.min(sample);
                    high = high.max(sample);
                }
            }
        }
        output.extend([low as f32 / PCM_SCALE, high as f32 / PCM_SCALE]);
    }
    output
}

struct Spectrum {
    fft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    buffer: Vec<Complex32>,
    window: Vec<f32>,
    power: Vec<f32>,
    bin_spacing: f32,
    frequencies: Vec<f32>,
    bands: Vec<Range<usize>>,
}

impl Spectrum {
    fn new(size: usize, rate: u32) -> Self {
        Self::focused(size, rate, None)
    }

    fn focused(size: usize, rate: u32, focus: Option<SpectrogramFocus>) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(size);
        let high = (rate as f32 * 0.45).min(20_000.0);
        let low = (high * 0.5).min(20.0);
        let (low, high, band_count) = focus.map_or((low, high, BAND_COUNT), |focus| {
            (
                focus.low_hz as f32,
                focus.high_hz as f32,
                FOCUSED_BAND_COUNT,
            )
        });
        let frequencies: Vec<_> = (0..band_count)
            .map(|band| low * (high / low).powf(band as f32 / (band_count - 1) as f32))
            .collect();
        let bin_spacing = rate as f32 / size as f32;
        let band_radius = (high / low).powf(0.5 / (band_count - 1) as f32);
        let bands = frequencies
            .iter()
            .map(|&frequency| {
                let nearest = (frequency / bin_spacing).round().max(1.0) as usize;
                let start = (frequency / band_radius / bin_spacing).ceil() as usize;
                let end = (frequency * band_radius / bin_spacing).floor() as usize + 1;
                start.min(nearest)..end.max(nearest + 1)
            })
            .collect();
        Self {
            scratch: vec![Complex32::default(); fft.get_inplace_scratch_len()],
            fft,
            buffer: vec![Complex32::default(); size],
            window: hann(size),
            power: vec![0.0; size / 2 + 1],
            bin_spacing,
            frequencies,
            bands,
        }
    }

    fn measure(
        &mut self,
        samples: &[i32],
        channels: usize,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Vec<f32>, MediaError> {
        let mut levels = self.measure_channel(samples, channels, None, cancelled)?;
        // Narrow display bands often share an FFT bin in the bass. Interpolate
        // their positions instead of repeating the same reading across a step.
        for (index, level) in levels.iter_mut().enumerate() {
            if self.bands[index].len() == 1 {
                let bin = (self.frequencies[index] / self.bin_spacing).max(1.0);
                let left = bin as usize;
                let right = (left + 1).min(self.power.len() - 1);
                let power = self.power[left] + (self.power[right] - self.power[left]) * bin.fract();
                *level = power_db(power);
            }
        }
        Ok(levels)
    }

    fn measure_channel(
        &mut self,
        samples: &[i32],
        channels: usize,
        channel: Option<usize>,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Vec<f32>, MediaError> {
        let frames = (samples.len() / channels).min(self.window.len());
        let short_window;
        let window = if frames < self.window.len() {
            short_window = hann(frames);
            &short_window
        } else {
            &self.window
        };
        let selected = channel.map_or(0..channels, |index| index..index + 1);
        let scale = (2.0 / window.iter().sum::<f32>().max(1.0)).powi(2) / selected.len() as f32;
        self.power.fill(0.0);
        for channel in selected {
            check_cancelled(cancelled)?;
            for ((point, &amplitude), frame) in self
                .buffer
                .iter_mut()
                .zip(window)
                .zip(samples.chunks_exact(channels))
            {
                *point = Complex32::new(frame[channel] as f32 / PCM_SCALE * amplitude, 0.0);
            }
            self.buffer[frames..].fill(Complex32::default());
            self.fft
                .process_with_scratch(&mut self.buffer, &mut self.scratch);
            for (power, point) in self.power.iter_mut().zip(&self.buffer) {
                *power += point.norm_sqr() * scale;
            }
        }
        Ok(self
            .bands
            .iter()
            .map(|band| power_db(self.power[band.clone()].iter().copied().fold(0.0, f32::max)))
            .collect())
    }
}

fn hann(size: usize) -> Vec<f32> {
    (0..size)
        .map(|index| {
            if size <= 2 {
                1.0
            } else {
                let phase = std::f32::consts::TAU * index as f32 / (size - 1) as f32;
                0.5 - 0.5 * phase.cos()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioTreatment;
    use std::io::{Seek, Write};

    const RATE: u32 = 48_000;
    const INFO: SourceInfo = SourceInfo {
        sample_rate: RATE,
        channels: 2,
        frames: RATE as u64 * 4,
        bits_per_sample: Some(16),
    };

    fn recording() -> File {
        stereo_recording(-1.0)
    }

    fn stereo_recording(right_gain: f64) -> File {
        stereo_recording_with_frames(right_gain, INFO.frames)
    }

    fn stereo_recording_with_frames(right_gain: f64, frames: u64) -> File {
        let mut file = tempfile::tempfile().unwrap();
        let bytes = frames as u32 * 4;
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + bytes).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&[16, 0, 0, 0, 1, 0, 2, 0]).unwrap();
        file.write_all(&RATE.to_le_bytes()).unwrap();
        file.write_all(&(RATE * 4).to_le_bytes()).unwrap();
        file.write_all(&[4, 0, 16, 0]).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&bytes.to_le_bytes()).unwrap();
        let pcm: Vec<_> = (0..frames)
            .flat_map(|frame| {
                let frequency = if frame < frames / 2 { 92.0 } else { 10_500.0 };
                let phase = frame as f64 * std::f64::consts::TAU * frequency / f64::from(RATE);
                let sample = (phase.sin() * 24_576.0) as i16;
                [sample, (f64::from(sample) * right_gain) as i16]
                    .into_iter()
                    .flat_map(i16::to_le_bytes)
            })
            .collect();
        file.write_all(&pcm).unwrap();
        file.rewind().unwrap();
        file
    }

    #[test]
    fn full_recording_spectrogram_tracks_time_and_processed_channels() {
        let info = SourceInfo {
            frames: u64::from(RATE) * 14,
            ..INFO
        };
        let source = stereo_recording_with_frames(0.5, info.frames);
        let range = FrameRange {
            start: 0,
            end: info.frames,
        };
        let options = TreatmentProbeOptions {
            spectrogram: true,
            viewport: Some(range),
            spectrogram_channel: SpectrogramChannel::Stereo,
            ..TreatmentProbeOptions::default()
        };
        let read = |settings, options| {
            probe_treatment_with_options(
                source.try_clone().unwrap(),
                info,
                0,
                None,
                PreparedTreatment::preview(settings, info).unwrap(),
                options,
                || false,
            )
            .unwrap()
        };
        let dry = read(AudioTreatment::default(), options);
        let wet = read(
            AudioTreatment {
                gain_db: -12,
                ..AudioTreatment::default()
            },
            TreatmentProbeOptions {
                spectrogram_treated: true,
                ..options
            },
        );
        for probe in [&dry, &wet] {
            assert_eq!(probe.spectrogram_start, Some(range.start));
            assert_eq!(probe.spectrogram_end, Some(range.end));
            let left = probe.spectrogram_left.as_ref().unwrap();
            let right = probe.spectrogram_right.as_ref().unwrap();
            assert_eq!(left.len(), right.len());
            assert!((64..=192).contains(&left.len()));
            assert!((70.0..110.0).contains(&strongest_band(&probe.frequencies, &left[0])));
            assert!((10_000.0..11_000.0)
                .contains(&strongest_band(&probe.frequencies, left.last().unwrap())));
            let peak = |column: &[f32]| column.iter().copied().fold(FLOOR_DB, f32::max);
            assert!((peak(&left[0]) - peak(&right[0]) - 6.0).abs() < 0.3);
        }
        let peak = |probe: &TreatmentProbe| {
            probe.spectrogram_left.as_ref().unwrap()[0]
                .iter()
                .copied()
                .fold(FLOOR_DB, f32::max)
        };
        assert!((peak(&dry) - peak(&wet) - 12.0).abs() < 0.5);
    }

    fn strongest_band(frequencies: &[f32], values: &[f32]) -> f32 {
        let index = values
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .unwrap()
            .0;
        frequencies[index]
    }

    #[test]
    fn probe_measures_the_real_filtered_and_compressed_audio() {
        let settings = AudioTreatment {
            high_cut_hz: 6_000,
            compressor_db: -24,
            compressor_ratio: 4,
            ..AudioTreatment::default()
        };
        let prepared = PreparedTreatment::preview(settings, INFO).unwrap();
        let probe = probe_treatment(
            recording(),
            INFO,
            INFO.frames - 1,
            None,
            prepared,
            false,
            || false,
        )
        .unwrap();
        assert!((10_000.0..11_000.0).contains(&strongest_band(&probe.frequencies, &probe.original)));
        assert!((-6.2..-4.8).contains(&probe.input));
        assert!(probe.output < probe.input - 15.0);
        let strongest = |bands: &[f32]| bands.iter().copied().fold(FLOOR_DB, f32::max);
        assert!(strongest(&probe.treated) < strongest(&probe.original) - 15.0);
        assert!(probe.reduction.len() > 30);
        assert!(probe
            .reduction
            .iter()
            .all(|db| db.is_finite() && *db <= 0.0));
        assert!(probe.reduction.iter().any(|db| *db < -2.0));
    }

    #[test]
    fn probe_reports_dynamic_eq_mix_for_the_band_that_is_cutting() {
        use crate::{DynamicEq, EqBand, EqChannel, MAX_EQ_BANDS};
        let mut bands = [None; MAX_EQ_BANDS];
        bands[0] = Some(EqBand {
            hz: 10_500,
            q: 500,
            channel: EqChannel::Right,
            dynamic: Some(DynamicEq {
                spectral: false,
                threshold_db: -24.0,
                ratio: 2.0,
                amount_db: 9.0,
            }),
            ..EqBand::default()
        });
        bands[1] = Some(EqBand {
            hz: 92,
            ..bands[0].unwrap()
        });
        let prepared = PreparedTreatment::preview(
            AudioTreatment {
                eq_bands: Some(bands),
                ..AudioTreatment::default()
            },
            INFO,
        )
        .unwrap();
        let low = probe_treatment(
            recording(),
            INFO,
            INFO.frames / 4,
            None,
            prepared.clone(),
            false,
            || false,
        )
        .unwrap();
        let high = probe_treatment(
            recording(),
            INFO,
            INFO.frames * 3 / 4,
            None,
            prepared,
            false,
            || false,
        )
        .unwrap();
        assert_eq!(low.dynamic_eq_mix, vec![0.0, 1.0]);
        assert_eq!(high.dynamic_eq_mix, vec![1.0, 0.0]);
    }

    #[test]
    fn levels_from_shared_decode_match_independent_probes() {
        let a = PreparedTreatment::preview(
            AudioTreatment {
                presence_hz: 92,
                presence_db: 4.0,
                ..AudioTreatment::default()
            },
            INFO,
        )
        .unwrap();
        let b = PreparedTreatment::preview(
            AudioTreatment {
                eq_output_db: -6.0,
                ..AudioTreatment::default()
            },
            INFO,
        )
        .unwrap();
        let frame = INFO.frames / 3;
        let shared = compare_preview_levels(
            recording(),
            INFO,
            frame,
            None,
            [a.clone(), b.clone()],
            || false,
        )
        .unwrap();
        let first = probe_treatment(recording(), INFO, frame, None, a, false, || false).unwrap();
        let second = probe_treatment(recording(), INFO, frame, None, b, false, || false).unwrap();
        assert!((shared.input - first.input).abs() < 0.0001);
        assert!((shared.output - first.output).abs() < 0.0001);
        assert!((shared.alternate_output - second.output).abs() < 0.0001);
    }

    #[test]
    fn output_trim_reports_overloads_and_leaves_the_input_peak_alone() {
        let measure = |trim| {
            let settings = AudioTreatment {
                eq_output_db: trim,
                ..AudioTreatment::default()
            };
            let prepared = PreparedTreatment::preview(settings, INFO)
                .unwrap()
                .with_preview_level(0.28, 0.75);
            probe_treatment(
                recording(),
                INFO,
                INFO.frames / 4,
                None,
                prepared,
                false,
                || false,
            )
            .unwrap()
        };
        let loud = measure(6.0);
        let quiet = measure(-6.0);
        assert!(loud.output_overload);
        assert!(!quiet.output_overload);
        assert_eq!(loud.input_peak, quiet.input_peak);
        assert!((quiet.output_peak - quiet.input_peak + 6.0).abs() < 0.1);
    }

    #[test]
    fn threshold_learning_and_live_readings_use_the_selected_sidechain() {
        use crate::{DynamicEq, EqBand, EqChannel, MAX_EQ_BANDS};
        let prepare = |channel| {
            let mut bands = [None; MAX_EQ_BANDS];
            bands[0] = Some(EqBand {
                hz: 92,
                q: 300,
                channel,
                dynamic: Some(DynamicEq {
                    spectral: false,
                    threshold_db: -24.0,
                    amount_db: 9.0,
                    ratio: 2.0,
                }),
                ..EqBand::default()
            });
            PreparedTreatment::preview(
                AudioTreatment {
                    eq_bands: Some(bands),
                    ..AudioTreatment::default()
                },
                INFO,
            )
            .unwrap()
        };
        let range = FrameRange {
            start: INFO.frames / 8,
            end: INFO.frames / 3,
        };
        let right = prepare(EqChannel::Right);
        let louder_threshold = learn_eq_threshold(
            stereo_recording(0.4),
            INFO,
            range,
            None,
            right.clone(),
            0,
            || false,
        )
        .unwrap();
        let right_threshold = learn_eq_threshold(
            stereo_recording(0.08),
            INFO,
            range,
            None,
            right.clone(),
            0,
            || false,
        )
        .unwrap();
        assert!((louder_threshold - right_threshold - 14.0).abs() < 0.3);
        let probe = probe_treatment(
            stereo_recording(0.08),
            INFO,
            INFO.frames / 4,
            None,
            right.clone(),
            false,
            || false,
        )
        .unwrap();
        assert!((probe.dynamic_eq_detectors[0].unwrap() - right_threshold).abs() < 1.5);
        assert!(
            learn_eq_threshold(stereo_recording(0.0), INFO, range, None, right, 0, || false)
                .is_err()
        );
    }

    #[test]
    fn spectrogram_preserves_the_time_order_and_frequency_of_the_source() {
        let prepared = PreparedTreatment::preview(AudioTreatment::default(), INFO).unwrap();
        let probe = probe_treatment(
            recording(),
            INFO,
            INFO.frames / 2,
            None,
            prepared,
            true,
            || false,
        )
        .unwrap();
        let columns = probe.spectrogram.unwrap();
        assert!(probe.spectrogram_start.unwrap() < INFO.frames / 2);
        assert!(probe.spectrogram_end.unwrap() > INFO.frames / 2);
        assert!(columns.len() > 20);
        assert!((80.0..105.0).contains(&strongest_band(&probe.frequencies, &columns[0])));
        assert!((10_000.0..11_000.0)
            .contains(&strongest_band(&probe.frequencies, columns.last().unwrap())));
        assert_eq!(probe.original, probe.treated);
    }

    #[test]
    fn meter_only_preserves_levels_and_dry_spectrogram_is_independent_of_treatment() {
        let settings = AudioTreatment {
            compressor_db: -24,
            compressor_ratio: 4,
            ..AudioTreatment::default()
        };
        let prepared = PreparedTreatment::preview(settings, INFO).unwrap();
        let frame = INFO.frames / 2;
        let combined = probe_treatment(
            recording(),
            INFO,
            frame,
            None,
            prepared.clone(),
            true,
            || false,
        )
        .unwrap();
        let meter_only = probe_treatment_with_options(
            recording(),
            INFO,
            frame,
            None,
            prepared.clone(),
            TreatmentProbeOptions {
                spectrum: false,
                spectrogram: false,
                ..TreatmentProbeOptions::default()
            },
            || false,
        )
        .unwrap();
        assert!(meter_only.frequencies.is_empty());
        assert!(meter_only.original.is_empty());
        assert!(meter_only.treated.is_empty());
        assert!(meter_only.spectrogram.is_none());
        assert_eq!(meter_only.input, combined.input);
        assert_eq!(meter_only.output, combined.output);
        assert_eq!(meter_only.reduction, combined.reduction);

        let dry = probe_treatment_with_options(
            recording(),
            INFO,
            frame,
            None,
            prepared,
            TreatmentProbeOptions {
                spectrum: false,
                spectrogram: true,
                ..TreatmentProbeOptions::default()
            },
            || false,
        )
        .unwrap();
        assert_eq!(dry.frequencies, combined.frequencies);
        assert_eq!(dry.spectrogram, combined.spectrogram);
        assert_eq!(dry.spectrogram_start, combined.spectrogram_start);
        assert_eq!(dry.spectrogram_end, combined.spectrogram_end);
        assert!(dry.original.is_empty());
        assert!(dry.treated.is_empty());
        assert_eq!(dry.input, dry.output);
        assert!(dry.reduction.iter().all(|&db| db == 0.0));
    }

    #[test]
    fn explicit_spectrogram_view_keeps_the_requested_source_coordinates() {
        let viewport = FrameRange {
            start: u64::from(INFO.sample_rate) / 2,
            end: u64::from(INFO.sample_rate),
        };
        let options = TreatmentProbeOptions {
            spectrum: false,
            spectrogram: true,
            viewport: Some(viewport),
            ..TreatmentProbeOptions::default()
        };
        let probe = probe_treatment_with_options(
            recording(),
            INFO,
            INFO.frames - 1,
            None,
            PreparedTreatment::default(),
            options,
            || false,
        )
        .unwrap();
        assert_eq!(probe.spectrogram_start, Some(viewport.start));
        assert_eq!(probe.spectrogram_end, Some(viewport.end));
        assert!(probe.spectrogram.unwrap().len() >= 24);
        assert!(probe.treated.is_empty());
        let outside = TreatmentProbeOptions {
            viewport: Some(FrameRange {
                start: 0,
                end: INFO.frames + 1,
            }),
            ..options
        };
        assert!(probe_treatment_with_options(
            recording(),
            INFO,
            0,
            None,
            PreparedTreatment::default(),
            outside,
            || false,
        )
        .is_err());
    }

    #[test]
    fn zoomed_spectrogram_measures_denser_time_columns_and_focused_frequency_bins() {
        let viewport = FrameRange {
            start: u64::from(INFO.sample_rate) * 3 / 2,
            end: INFO.frames,
        };
        let display = |focus| {
            probe_treatment_with_options(
                recording(),
                INFO,
                viewport.start,
                None,
                PreparedTreatment::default(),
                TreatmentProbeOptions {
                    spectrogram: true,
                    viewport: Some(viewport),
                    spectrogram_focus: focus,
                    ..TreatmentProbeOptions::default()
                },
                || false,
            )
            .unwrap()
        };
        let broad = display(None);
        let focus = SpectrogramFocus {
            low_hz: 9_800,
            high_hz: 11_200,
        };
        let zoomed = display(Some(focus));
        let columns = zoomed.spectrogram.as_ref().unwrap();
        assert_eq!(zoomed.spectrogram_focus, Some(focus));
        assert_eq!(zoomed.spectrogram_start, broad.spectrogram_start);
        assert_eq!(zoomed.spectrogram_end, broad.spectrogram_end);
        assert!(columns.len() > broad.spectrogram.unwrap().len());
        assert!(zoomed.frequencies.len() > broad.frequencies.len());
        assert!((10_480.0..10_520.0).contains(&strongest_band(
            &zoomed.frequencies,
            &columns[columns.len() / 2]
        )));
    }

    #[test]
    fn difference_and_linked_stereo_probes_keep_coordinates_and_detail() {
        let viewport = FrameRange {
            start: 24_000,
            end: 48_000,
        };
        let detail = SpectrogramDetail::Transients;
        let options = TreatmentProbeOptions {
            spectrogram: true,
            viewport: Some(viewport),
            spectrogram_channel: SpectrogramChannel::Stereo,
            spectrogram_detail: detail,
            ..TreatmentProbeOptions::default()
        };
        let probe = |treatment, options| {
            probe_treatment_with_options(
                stereo_recording(0.0),
                INFO,
                0,
                None,
                PreparedTreatment::preview(treatment, INFO).unwrap(),
                options,
                || false,
            )
            .unwrap()
        };
        let original = probe(AudioTreatment::default(), options);
        let difference = TreatmentProbeOptions {
            spectrogram_treated: true,
            spectrogram_difference: true,
            ..options
        };
        let silent = probe(AudioTreatment::default(), difference);
        let removed = probe(
            AudioTreatment {
                gain_db: -6,
                ..AudioTreatment::default()
            },
            difference,
        );
        let swapped = probe(
            AudioTreatment {
                gain_db: -6,
                swap_channels: true,
                ..AudioTreatment::default()
            },
            difference,
        );
        for reading in [&original, &removed, &silent, &swapped] {
            assert_eq!(reading.spectrogram_start, Some(viewport.start));
            assert_eq!(reading.spectrogram_end, Some(viewport.end));
            assert_eq!(reading.spectrogram_detail, Some(detail));
            assert_eq!(reading.spectrogram_window_frames, Some(1024));
            assert_eq!(reading.frequencies, original.frequencies);
            let left = reading.spectrogram_left.as_ref().unwrap();
            let right = reading.spectrogram_right.as_ref().unwrap();
            assert_eq!(left.len(), right.len());
            assert!(left.len() > 64);
            assert!(right.iter().flatten().all(|value| *value == FLOOR_DB));
        }
        let peak_db = |reading: &TreatmentProbe| {
            reading
                .spectrogram_left
                .as_ref()
                .unwrap()
                .iter()
                .flatten()
                .copied()
                .fold(FLOOR_DB, f32::max)
        };
        assert_eq!(peak_db(&silent), FLOOR_DB);
        assert!((peak_db(&original) - peak_db(&removed) - 6.04).abs() < 0.2);
        assert_eq!(removed.spectrogram_left, swapped.spectrogram_left);
        assert_eq!(
            removed.spectrogram_left_waveform,
            swapped.spectrogram_left_waveform
        );
    }

    #[test]
    fn treated_spectrogram_and_waveform_use_the_same_processed_channel() {
        let options = TreatmentProbeOptions {
            spectrogram: true,
            viewport: Some(FrameRange {
                start: 12_000,
                end: 30_000,
            }),
            spectrogram_channel: SpectrogramChannel::Right,
            ..TreatmentProbeOptions::default()
        };
        let recipe = AudioTreatment {
            gain_db: -12,
            swap_channels: true,
            ..AudioTreatment::default()
        };
        let prepared = PreparedTreatment::preview(recipe, INFO).unwrap();
        let source = probe_treatment_with_options(
            stereo_recording(0.5),
            INFO,
            24_000,
            None,
            prepared.clone(),
            options,
            || false,
        )
        .unwrap();
        let treated = probe_treatment_with_options(
            stereo_recording(0.5),
            INFO,
            24_000,
            None,
            prepared,
            TreatmentProbeOptions {
                spectrogram_treated: true,
                ..options
            },
            || false,
        )
        .unwrap();
        assert_eq!(source.spectrogram_channel, Some(SpectrogramChannel::Right));
        assert_eq!(treated.spectrogram_channel, source.spectrogram_channel);
        assert_eq!(treated.spectrogram_start, source.spectrogram_start);
        assert_eq!(treated.spectrogram_end, source.spectrogram_end);
        assert_eq!(treated.spectrogram_treated, Some(true));
        let max = |values: Vec<Vec<f32>>| values.into_iter().flatten().fold(f32::MIN, f32::max);
        assert!(
            (max(source.spectrogram.unwrap()) - max(treated.spectrogram.unwrap()) - 12.0).abs()
                < 0.1
        );
        let peak = |values: Vec<f32>| values.into_iter().map(f32::abs).fold(0.0, f32::max);
        let ratio = peak(treated.spectrogram_waveform.unwrap())
            / peak(source.spectrogram_waveform.unwrap());
        assert!((ratio - 0.2512).abs() < 0.001);
    }

    #[test]
    fn probe_rejects_invalid_frames_and_respects_cancellation() {
        let prepared = PreparedTreatment::preview(AudioTreatment::default(), INFO).unwrap();
        assert!(matches!(
            probe_treatment(
                recording(),
                INFO,
                INFO.frames,
                None,
                prepared.clone(),
                false,
                || false
            ),
            Err(MediaError::Range(_))
        ));
        assert!(matches!(
            probe_treatment(recording(), INFO, 0, None, prepared, false, || true),
            Err(MediaError::Cancelled)
        ));
    }
}
