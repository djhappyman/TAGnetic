//! Source-rate processing shared by analysis, preview and export.
use super::{check_cancelled, decode::Recording, MediaError, SourceInfo};
use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};
use std::fs::File;

const TARGET_LUFS: f64 = -18.0;
const CEILING_DBTP: f64 = -1.0;
const MAX_BOOST_DB: f64 = 18.0;
const PCM_SCALE: f64 = 2_147_483_648.0;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonoMode {
    #[default]
    Off,
    Mix,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HumFilter {
    #[default]
    Off,
    Hz50,
    Hz60,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AudioTreatment {
    pub normalize: bool,
    pub mono: MonoMode,
    pub rumble: bool,
    pub hum: HumFilter,
}

impl AudioTreatment {
    pub fn is_active(self) -> bool {
        self.normalize || self.mono != MonoMode::Off || self.rumble || self.hum != HumFilter::Off
    }

    fn has_filter(self) -> bool {
        self.rumble || self.hum != HumFilter::Off
    }

    pub fn output_info(self, mut source: SourceInfo) -> Result<SourceInfo, MediaError> {
        if self.mono != MonoMode::Off {
            if source.channels != 2 {
                return Err(MediaError::Unsupported(
                    "Mono controls require a stereo source",
                ));
            }
            source.channels = 1;
        }
        if self.has_filter() && source.sample_rate <= 120 {
            return Err(MediaError::Unsupported(
                "This sample rate is too low for audio cleanup",
            ));
        }
        if self.normalize || self.has_filter() || self.mono == MonoMode::Mix {
            source.bits_per_sample = Some(24);
        }
        Ok(source)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PreparedTreatment {
    settings: AudioTreatment,
    gain: f64,
    loudness: Option<f64>,
}

impl Default for PreparedTreatment {
    fn default() -> Self {
        Self {
            settings: AudioTreatment::default(),
            gain: 1.0,
            loudness: None,
        }
    }
}

impl PreparedTreatment {
    pub fn settings(self) -> AudioTreatment {
        self.settings
    }

    pub fn gain_db(self) -> f64 {
        20.0 * self.gain.log10()
    }

    pub fn loudness_lufs(self) -> Option<f64> {
        self.loudness
    }

    /// Start filters before a seek so a cut or an audition has the same settling
    /// history as continuous playback. Stateless changes start on the exact frame.
    pub fn preview_start(self, start: u64, rate: u32) -> u64 {
        if self.settings.has_filter() {
            start.saturating_sub(u64::from(rate) * 2)
        } else {
            start
        }
    }

    pub fn processor(self, source: SourceInfo) -> Result<TreatmentProcessor, MediaError> {
        TreatmentProcessor::new(self, source)
    }
}

/// Measure the entire recording after channel and filter changes. One gain
/// covers all splits, retaining their relative volume. The meter is bounded.
pub fn prepare_treatment(
    source: File,
    settings: AudioTreatment,
    mut cancelled: impl FnMut() -> bool,
) -> Result<PreparedTreatment, MediaError> {
    check_cancelled(&mut cancelled)?;
    let mut recording = Recording::open(source)?;
    let info = recording.info();
    let mut prepared = PreparedTreatment {
        settings,
        ..PreparedTreatment::default()
    };
    let mut processor = prepared.processor(info)?;
    if !settings.normalize && !settings.has_filter() {
        return Ok(prepared);
    }

    let channels = u32::from(processor.output_info().channels);
    let mut meter = EbuR128::new(
        channels,
        info.sample_rate,
        Mode::I | Mode::TRUE_PEAK | Mode::HISTOGRAM,
    )
    .map_err(measurement_error)?;
    let mut samples = Vec::new();
    while let Some(block) = recording.next(&mut cancelled)? {
        samples.clear();
        processor.visit(block, |sample| samples.push(sample));
        meter.add_frames_f64(&samples).map_err(measurement_error)?;
    }
    check_cancelled(&mut cancelled)?;
    let loudness = meter.loudness_global().map_err(measurement_error)?;
    let peak = (0..channels)
        .map(|channel| meter.true_peak(channel))
        .try_fold(0.0_f64, |peak, channel| {
            channel.map(|value| peak.max(value))
        })
        .map_err(measurement_error)?;
    prepared.loudness = loudness.is_finite().then_some(loudness);
    let gain_db = match prepared.loudness {
        Some(loudness) if settings.normalize => (TARGET_LUFS - loudness).min(MAX_BOOST_DB),
        _ => 0.0,
    };
    let ceiling_gain = if peak > 0.0 {
        db_gain(CEILING_DBTP) / peak
    } else {
        1.0
    };
    prepared.gain = db_gain(gain_db).min(ceiling_gain);
    Ok(prepared)
}

fn measurement_error(error: ebur128::Error) -> MediaError {
    MediaError::Decode(format!("Could not measure audio loudness: {error}"))
}

fn db_gain(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

pub struct TreatmentProcessor {
    prepared: PreparedTreatment,
    source_channels: usize,
    output: SourceInfo,
    filters: Vec<Vec<Biquad>>,
}

impl TreatmentProcessor {
    fn new(prepared: PreparedTreatment, source: SourceInfo) -> Result<Self, MediaError> {
        let output = prepared.settings.output_info(source)?;
        let mut filters = Vec::new();
        let rate = f64::from(source.sample_rate);
        if prepared.settings.rumble {
            filters.push(Biquad::high_pass(rate, 35.0));
        }
        if prepared.settings.hum != HumFilter::Off {
            let frequency = if prepared.settings.hum == HumFilter::Hz50 {
                50.0
            } else {
                60.0
            };
            filters.push(Biquad::notch(rate, frequency));
        }
        Ok(Self {
            prepared,
            source_channels: usize::from(source.channels),
            output,
            filters: vec![filters; usize::from(output.channels)],
        })
    }

    pub fn output_info(&self) -> SourceInfo {
        self.output
    }

    /// Process complete, interleaved frames, retaining history between blocks.
    /// The output uses the same signed full-scale representation as the decoder.
    pub fn process(&mut self, input: &[i32], output: &mut Vec<i32>) {
        output.clear();
        if !self.prepared.settings.is_active() {
            output.extend_from_slice(input);
            return;
        }
        let gain = self.prepared.gain;
        self.visit(input, |sample| {
            output.push((sample * gain * PCM_SCALE).round() as i32)
        });
    }

    fn visit(&mut self, input: &[i32], mut produced: impl FnMut(f64)) {
        let mut emit = |channel: usize, sample: f64| {
            let mut sample = sample / PCM_SCALE;
            for filter in &mut self.filters[channel] {
                sample = filter.filter(sample);
            }
            produced(sample);
        };
        let float = f64::from;
        for frame in input.chunks_exact(self.source_channels) {
            match self.prepared.settings.mono {
                MonoMode::Off => {
                    for (channel, &sample) in frame.iter().enumerate() {
                        emit(channel, float(sample));
                    }
                }
                MonoMode::Mix => emit(0, (float(frame[0]) + float(frame[1])) * 0.5),
                MonoMode::Left => emit(0, float(frame[0])),
                MonoMode::Right => emit(0, float(frame[1])),
            }
        }
    }
}

#[derive(Clone)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    delay: [f64; 2],
}

impl Biquad {
    // Robert Bristow-Johnson's Audio EQ Cookbook coefficients.
    fn high_pass(rate: f64, frequency: f64) -> Self {
        let (sin, cos) = (std::f64::consts::TAU * frequency / rate).sin_cos();
        let b = (1.0 + cos) * 0.5;
        Self::new([b, -2.0 * b, b], cos, sin * std::f64::consts::FRAC_1_SQRT_2)
    }

    fn notch(rate: f64, frequency: f64) -> Self {
        let (sin, cos) = (std::f64::consts::TAU * frequency / rate).sin_cos();
        Self::new([1.0, -2.0 * cos, 1.0], cos, sin / 20.0)
    }

    fn new(b: [f64; 3], cos: f64, alpha: f64) -> Self {
        let a0 = 1.0 + alpha;
        Self {
            b: b.map(|value| value / a0),
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            delay: [0.0; 2],
        }
    }

    fn filter(&mut self, input: f64) -> f64 {
        let output = self.b[0] * input + self.delay[0];
        self.delay[0] = self.b[1] * input - self.a[0] * output + self.delay[1];
        self.delay[1] = self.b[2] * input - self.a[1] * output;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    const RATE: u32 = 48_000;

    fn test_audio(amplitude: f64, frequency: f64) -> File {
        let mut source = tempfile::tempfile().unwrap();
        let frames = RATE * 2;
        let bytes = frames * 2;
        source.write_all(b"RIFF").unwrap();
        source.write_all(&(36 + bytes).to_le_bytes()).unwrap();
        source.write_all(b"WAVEfmt ").unwrap();
        source.write_all(&[16, 0, 0, 0, 1, 0, 1, 0]).unwrap();
        source.write_all(&RATE.to_le_bytes()).unwrap();
        source.write_all(&(RATE * 2).to_le_bytes()).unwrap();
        source.write_all(&[2, 0, 16, 0]).unwrap();
        source.write_all(b"data").unwrap();
        source.write_all(&bytes.to_le_bytes()).unwrap();
        let pcm: Vec<_> = (0..frames)
            .flat_map(|frame| {
                let angle = f64::from(frame) * std::f64::consts::TAU * frequency / f64::from(RATE);
                ((angle.sin() * amplitude * 32767.0) as i16).to_le_bytes()
            })
            .collect();
        source.write_all(&pcm).unwrap();
        source.rewind().unwrap();
        source
    }

    #[test]
    fn normalization_targets_loudness_without_boosting_silence() {
        let settings = AudioTreatment {
            normalize: true,
            ..AudioTreatment::default()
        };
        let tone = prepare_treatment(test_audio(0.03, 1000.0), settings, || false).unwrap();
        assert!(tone.gain_db() > 6.0);
        assert!((tone.loudness_lufs().unwrap() + tone.gain_db() - TARGET_LUFS).abs() < 0.1);
        let silent = prepare_treatment(test_audio(0.0, 1000.0), settings, || false).unwrap();
        assert_eq!(silent.gain_db(), 0.0);
        assert_eq!(silent.loudness_lufs(), None);
    }

    #[test]
    fn hum_filter_reduces_its_frequency_and_retains_history_across_blocks() {
        let settings = AudioTreatment {
            hum: HumFilter::Hz50,
            ..AudioTreatment::default()
        };
        let mut recording = Recording::open(test_audio(0.4, 50.0)).unwrap();
        let prepared = prepare_treatment(test_audio(0.4, 50.0), settings, || false).unwrap();
        let mut processor = prepared.processor(recording.info()).unwrap();
        let mut position = 0;
        let mut processed = Vec::new();
        while let Some(block) = recording.next(&mut || false).unwrap() {
            processor.process(block, &mut processed);
            if position > u64::from(RATE) {
                assert!(processed
                    .iter()
                    .all(|sample| sample.unsigned_abs() < 250_000));
            }
            position += block.len() as u64;
        }
        assert_eq!(position, u64::from(RATE) * 2);
    }
}
