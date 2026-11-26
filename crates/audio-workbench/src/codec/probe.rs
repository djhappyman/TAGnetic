use super::{
    check_cancelled, MediaError, PreparedTreatment, PreviewDecoder, SeekIndex, SourceInfo,
};
use crate::FrameRange;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

const FLOOR_DB: f32 = -96.0;
const PCM_SCALE: f32 = 2_147_483_648.0;
const BAND_COUNT: usize = 112;
const REDUCTION_POINTS: usize = 48;
const SPECTROGRAM_COLUMNS: u64 = 32;
const MAX_VISUAL_SAMPLES: u64 = 4_000_000;
const DECODE_BUDGET: Duration = Duration::from_millis(650);

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SpectrogramChannel {
    #[default]
    Combined,
    Left,
    Right,
}

impl SpectrogramChannel {
    fn index(self, channels: usize) -> Option<usize> {
        match self {
            Self::Combined => None,
            Self::Left => Some(0),
            Self::Right => Some(1.min(channels - 1)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TreatmentProbeOptions {
    pub spectrum: bool,
    pub spectrogram: bool,
    pub viewport: Option<FrameRange>,
    pub spectrogram_channel: SpectrogramChannel,
    pub spectrogram_treated: bool,
}

impl TreatmentProbeOptions {
    pub fn spectrogram_only(self) -> bool {
        self.spectrogram && !self.spectrum
    }

    pub fn dry_spectrogram(self) -> bool {
        self.spectrogram_only() && !self.spectrogram_treated
    }
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
    pub output_peak: f32,
    /// Linked compressor detector level at the requested frame, in dBFS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detector: Option<f32>,
    /// Quiet-gap gain in dB, oldest to newest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_reduction: Option<Vec<f32>>,
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
    let channels = usize::from(info.channels);
    let base_fft_size = (info.sample_rate / 6)
        .next_power_of_two()
        .clamp(2_048, 32_768);
    let viewport = options.viewport.filter(|_| options.spectrogram_only());
    if viewport.is_some_and(|range| {
        range.start >= range.end
            || range.end > info.frames
            || range.end - range.start
                > (rate * 12).min(MAX_VISUAL_SAMPLES / u64::from(info.channels))
    }) {
        return Err(MediaError::Range(
            "Spectrogram view is outside the supported range",
        ));
    }
    let fft_size = viewport.map_or(base_fft_size, |range| {
        (((range.end - range.start) / 8).max(2_048) as u32)
            .next_power_of_two()
            .min(base_fft_size)
    });
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
    let keep_wet = if options.spectrogram && options.spectrogram_treated {
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
    let deadline = Instant::now() + DECODE_BUDGET;
    let mut stop_decoding = || cancelled() || Instant::now() >= deadline;
    let mut decoder = PreviewDecoder::open_indexed(source, cursor, index, &mut stop_decoding)?;
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
    let reduction_step = (history.end - history.start).div_ceil(REDUCTION_POINTS as u64);

    while cursor < end {
        check_cancelled(&mut stop_decoding)?;
        let Some(block) = decoder.next_block(&mut stop_decoding)? else {
            break;
        };
        let next = (cursor + (block.len() / channels) as u64).min(end);
        if let Some(span) = overlap(cursor, next, keep_raw) {
            original_pcm.extend_from_slice(&block[interleaved(span, cursor, channels)]);
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
            let span = FrameRange {
                start: first,
                end: last,
            };
            processor.process(&block[interleaved(span, cursor, channels)], &mut processed);
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
    if options.spectrum || options.spectrogram {
        let mut analyzer = Spectrum::new(fft_size as usize, info.sample_rate);
        if options.spectrum {
            original = analyzer.measure(original_slice, channels, &mut cancelled)?;
            treated = if active {
                analyzer.measure(treated_slice, treated_channels, &mut cancelled)?
            } else {
                original.clone()
            };
        }
        if options.spectrogram {
            let (visual_pcm, visual_channels) = if active && options.spectrogram_treated {
                (&treated_pcm[..], treated_channels)
            } else {
                (&original_pcm[..], channels)
            };
            let span = (visual_pcm.len() / visual_channels) as u64;
            let mut channel = options.spectrogram_channel.index(visual_channels);
            if options.spectrogram_treated
                && visual_channels == 2
                && prepared.settings().swap_channels
            {
                // Repair selections refer to a source channel, even when the output swaps it.
                channel = channel.map(|index| 1 - index);
            }
            let max_columns = if viewport.is_some() {
                64
            } else {
                SPECTROGRAM_COLUMNS
            };
            let stride = if viewport.is_some() {
                256
            } else {
                u64::from(fft_size) / 2
            };
            let columns = max_columns.min(span.div_ceil(stride));
            let mut values = Vec::with_capacity(columns as usize);
            for column in 0..columns {
                let at = (2 * column + 1) * span / (2 * columns);
                let relative = centered(at, u64::from(fft_size), span);
                let region = interleaved(relative, 0, visual_channels);
                values.push(analyzer.measure_channel(
                    &visual_pcm[region],
                    visual_channels,
                    channel,
                    &mut cancelled,
                )?);
            }
            spectrogram = Some(values);
            spectrogram_waveform = Some(waveform_samples(visual_pcm, visual_channels, channel));
        }
        frequencies = analyzer.frequencies;
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
    Ok(TreatmentProbe {
        frequencies,
        original,
        treated,
        reduction,
        input,
        output,
        input_peak,
        output_peak,
        detector,
        gate_reduction,
        spectrogram,
        spectrogram_start: options.spectrogram.then_some(keep_raw.start),
        spectrogram_end: options.spectrogram.then_some(visual_end),
        spectrogram_channel: options.spectrogram.then_some(options.spectrogram_channel),
        spectrogram_treated: options.spectrogram.then_some(options.spectrogram_treated),
        spectrogram_waveform,
    })
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
    frequencies: Vec<f32>,
    bands: Vec<Range<usize>>,
}

impl Spectrum {
    fn new(size: usize, rate: u32) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(size);
        let high = (rate as f32 * 0.45).min(20_000.0);
        let low = (high * 0.5).min(20.0);
        let frequencies: Vec<_> = (0..BAND_COUNT)
            .map(|band| low * (high / low).powf(band as f32 / (BAND_COUNT - 1) as f32))
            .collect();
        let bin_spacing = rate as f32 / size as f32;
        let band_radius = (high / low).powf(0.5 / (BAND_COUNT - 1) as f32);
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
        self.measure_channel(samples, channels, None, cancelled)
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
        let mut file = tempfile::tempfile().unwrap();
        let bytes = INFO.frames as u32 * 4;
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36 + bytes).to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&[16, 0, 0, 0, 1, 0, 2, 0]).unwrap();
        file.write_all(&RATE.to_le_bytes()).unwrap();
        file.write_all(&(RATE * 4).to_le_bytes()).unwrap();
        file.write_all(&[4, 0, 16, 0]).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&bytes.to_le_bytes()).unwrap();
        let pcm: Vec<_> = (0..INFO.frames)
            .flat_map(|frame| {
                let frequency = if frame < INFO.frames / 2 {
                    92.0
                } else {
                    10_500.0
                };
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
        let combined =
            probe_treatment(recording(), INFO, frame, None, prepared, true, || false).unwrap();
        let meter_only = probe_treatment_with_options(
            recording(),
            INFO,
            frame,
            None,
            prepared,
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
            prepared,
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
                prepared,
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
