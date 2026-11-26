//! Source-rate processing shared by analysis, preview and export.
use super::{
    check_cancelled,
    decode::{Recording, MAX_FRAMES},
    FrameCount, MediaError, SourceInfo,
};
use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};
use std::fs::File;

mod band_repair;
mod biquad;
mod dynamics;

use band_repair::BandRepair;
pub use band_repair::{SpectralRepair, MAX_SPECTRAL_REPAIRS};
use biquad::Biquad;
use dynamics::Dynamics;

const TARGET_LUFS: i8 = -18;
const CEILING_DBTP: f64 = -1.0;
const MAX_BOOST_DB: f64 = 18.0;
const MAX_PREVIEW_BOOST_DB: f64 = 12.0;
const PCM_SCALE: f64 = 2_147_483_648.0;
const BASS_HZ: u16 = 120;
const PRESENCE_HZ: u16 = 1_500;
const TREBLE_HZ: u16 = 4_000;

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
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    #[default]
    Off,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AudioTreatment {
    pub normalize: bool,
    pub normalize_target_db: i8,
    pub mono: MonoMode,
    pub rumble: bool,
    pub rumble_hz: u16,
    pub rumble_slope: u8,
    pub hum: HumFilter,
    pub hum_harmonics: u8,
    pub dc_hz: u16,
    pub high_cut_hz: u16,
    pub high_cut_slope: u8,
    pub notch_hz: u16,
    pub notch_q: u8,
    pub bass_db: i8,
    pub bass_hz: u16,
    pub presence_db: i8,
    pub presence_hz: u16,
    /// Peaking filter Q multiplied by 100.
    pub presence_q: u16,
    pub treble_db: i8,
    pub treble_hz: u16,
    pub gain_db: i8,
    pub compressor_db: i8,
    pub compressor_ratio: u8,
    pub gate_db: i8,
    pub gate_reduction_db: u8,
    pub balance: i8,
    pub stereo_width: i8,
    pub swap_channels: bool,
    pub polarity: Polarity,
    #[serde(
        deserialize_with = "band_repair::deserialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub spectral_repairs: Option<[Option<SpectralRepair>; MAX_SPECTRAL_REPAIRS]>,
}

impl AudioTreatment {
    pub fn is_active(self) -> bool {
        self.normalize
            || self.mono != MonoMode::Off
            || self.has_filter()
            || self.repairs().next().is_some()
            || self.has_dynamics()
            || self.has_stereo_repair()
            || self.gain_db != 0
    }

    fn has_filter(self) -> bool {
        self.rumble
            || self.hum != HumFilter::Off
            || self.dc_hz != 0
            || self.high_cut_hz != 0
            || self.notch_hz != 0
            || self.bass_db != 0
            || self.presence_db != 0
            || self.treble_db != 0
    }

    fn repairs(&self) -> impl Iterator<Item = SpectralRepair> + '_ {
        self.spectral_repairs.iter().flatten().copied().flatten()
    }

    fn has_dynamics(self) -> bool {
        self.compressor_db != 0 || self.gate_db != 0
    }

    fn has_stereo_repair(self) -> bool {
        self.swap_channels
            || self.polarity != Polarity::Off
            || self.balance != 0
            || self.stereo_width != 0
    }

    /// Whole-file loudness or peak protection is only required for certain tools.
    pub fn needs_measurement(self) -> bool {
        self.normalize
            || self.has_filter()
            || self.has_dynamics()
            || self.gain_db > 0
            || self.stereo_width > 0
    }

    fn rumble_cutoff(self) -> u16 {
        if self.rumble_hz == 0 {
            35
        } else {
            self.rumble_hz
        }
    }

    fn tone_frequencies(self) -> [u16; 3] {
        [
            (self.bass_hz, BASS_HZ),
            (self.presence_hz, PRESENCE_HZ),
            (self.treble_hz, TREBLE_HZ),
        ]
        .map(|(chosen, default)| if chosen == 0 { default } else { chosen })
    }

    fn hum_frequency(self) -> u16 {
        match self.hum {
            HumFilter::Off => 0,
            HumFilter::Hz50 => 50,
            HumFilter::Hz60 => 60,
        }
    }

    fn normalize_target(self) -> f64 {
        f64::from(if self.normalize_target_db == 0 {
            TARGET_LUFS
        } else {
            self.normalize_target_db
        })
    }

    fn validate(self, sample_rate: u32) -> Result<(), MediaError> {
        let in_range = |value, min, max| value == 0 || (min..=max).contains(&value);
        for (value, min, max, error) in [
            (
                i32::from(self.normalize_target_db),
                -30,
                -8,
                "Choose a normalisation target between −30 and −8 LUFS",
            ),
            (
                i32::from(self.rumble_hz),
                15,
                100,
                "Choose a rumble cutoff between 15 and 100 Hz",
            ),
            (
                i32::from(self.hum_harmonics),
                1,
                5,
                "Choose one to five hum bands",
            ),
            (
                i32::from(self.dc_hz),
                2,
                15,
                "Choose a DC cutoff between 2 and 15 Hz",
            ),
            (
                i32::from(self.high_cut_hz),
                4000,
                18000,
                "Choose a high cut between 4 and 18 kHz",
            ),
            (
                i32::from(self.notch_hz),
                200,
                12000,
                "Choose a whistle frequency between 200 Hz and 12 kHz",
            ),
            (
                i32::from(self.notch_q),
                5,
                30,
                "Choose a notch sharpness between 5 and 30",
            ),
            (
                i32::from(self.bass_db),
                -6,
                6,
                "Choose a bass adjustment between −6 and +6 dB",
            ),
            (
                i32::from(self.presence_db),
                -6,
                6,
                "Choose a presence adjustment between −6 and +6 dB",
            ),
            (
                i32::from(self.treble_db),
                -6,
                6,
                "Choose a treble adjustment between −6 and +6 dB",
            ),
            (
                i32::from(self.presence_q),
                20,
                1200,
                "Choose a presence Q between 0.20 and 12.00",
            ),
            (
                i32::from(self.gain_db),
                -18,
                12,
                "Choose a gain adjustment between −18 and +12 dB",
            ),
            (
                i32::from(self.compressor_db),
                -36,
                -6,
                "Choose a compression threshold between −36 and −6 dBFS",
            ),
            (
                i32::from(self.compressor_ratio),
                2,
                6,
                "Choose a compression ratio between 2:1 and 6:1",
            ),
            (
                i32::from(self.gate_db),
                -72,
                -30,
                "Choose a quiet-gap threshold between −72 and −30 dBFS",
            ),
            (
                i32::from(self.gate_reduction_db),
                3,
                24,
                "Choose a quiet-gap reduction between 3 and 24 dB",
            ),
            (
                i32::from(self.balance),
                -100,
                100,
                "Choose a stereo balance between −100 and +100",
            ),
            (
                i32::from(self.stereo_width),
                -100,
                50,
                "Choose a stereo width between 0% and 150%",
            ),
        ] {
            if !in_range(value, min, max) {
                return Err(MediaError::Unsupported(error));
            }
        }
        for frequency in [self.bass_hz, self.presence_hz, self.treble_hz] {
            if !in_range(i32::from(frequency), 20, 18_000) {
                return Err(MediaError::Unsupported(
                    "Choose an EQ frequency between 20 Hz and 18 kHz",
                ));
            }
        }
        for slope in [self.rumble_slope, self.high_cut_slope] {
            if !matches!(slope, 0 | 12 | 24 | 36 | 48) {
                return Err(MediaError::Unsupported(
                    "Choose a cutoff slope of 12, 24, 36, or 48 dB/octave",
                ));
            }
        }
        let [bass_hz, presence_hz, treble_hz] = self.tone_frequencies();
        let highest_frequency = [
            self.dc_hz,
            if self.rumble { self.rumble_cutoff() } else { 0 },
            self.hum_frequency() * u16::from(self.hum_harmonics.max(1)),
            self.high_cut_hz,
            self.notch_hz,
            if self.bass_db != 0 { bass_hz } else { 0 },
            if self.presence_db != 0 {
                presence_hz
            } else {
                0
            },
            if self.treble_db != 0 { treble_hz } else { 0 },
        ]
        .into_iter()
        .max()
        .unwrap_or_default();
        // Stay clear of Nyquist so a narrow notch can settle within the seek preroll.
        if f64::from(highest_frequency) > f64::from(sample_rate) * 0.45 {
            return Err(MediaError::Unsupported(
                "Lower the filter frequency for this recording's sample rate",
            ));
        }
        Ok(())
    }

    pub fn output_info(self, source: SourceInfo) -> Result<SourceInfo, MediaError> {
        self.output_info_with_frames(source, Some(source.frames))
    }

    fn output_info_with_frames(
        self,
        mut source: SourceInfo,
        verified_frames: Option<u64>,
    ) -> Result<SourceInfo, MediaError> {
        if source.sample_rate == 0 || source.channels == 0 {
            return Err(MediaError::Unsupported(
                "The recording has no usable audio format",
            ));
        }
        self.validate(source.sample_rate)?;
        let frequency_limit = (u64::from(source.sample_rate) * 45 / 100).min(20_000);
        for repair in self.repairs() {
            if repair.start_frame >= repair.end_frame
                || repair.end_frame > verified_frames.unwrap_or(MAX_FRAMES).min(MAX_FRAMES)
            {
                return Err(MediaError::Range("Choose a repair inside the recording"));
            }
            if repair.low_hz < 20
                || repair.high_hz.saturating_sub(repair.low_hz) < 20
                || u64::from(repair.high_hz) > frequency_limit
            {
                return Err(MediaError::Unsupported(
                    "Choose a repair band at least 20 Hz wide within the recording’s frequency range",
                ));
            }
            if !(1..=36).contains(&repair.amount_db) {
                return Err(MediaError::Unsupported(
                    "Choose a repair attenuation between 1 and 36 dB",
                ));
            }
        }
        if self.has_stereo_repair() && source.channels != 2 {
            return Err(MediaError::Unsupported(
                "Stereo repair requires a stereo source",
            ));
        }
        if self.mono != MonoMode::Off {
            if source.channels != 2 {
                return Err(MediaError::Unsupported(
                    "Mono controls require a stereo source",
                ));
            }
            source.channels = 1;
        }
        if self.normalize
            || self.has_filter()
            || self.repairs().next().is_some()
            || self.has_dynamics()
            || self.mono == MonoMode::Mix
            || self.gain_db != 0
            || self.balance != 0
            || self.stereo_width != 0
        {
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
    preview_ceiling: bool,
}

impl Default for PreparedTreatment {
    fn default() -> Self {
        Self {
            settings: AudioTreatment::default(),
            gain: 1.0,
            loudness: None,
            preview_ceiling: false,
        }
    }
}

impl PreparedTreatment {
    /// Prepare an immediate audition without scanning the recording. Until
    /// measurement completes, only a linked sample ceiling protects peaks.
    pub fn preview(settings: AudioTreatment, source: SourceInfo) -> Result<Self, MediaError> {
        settings.output_info(source)?;
        Ok(Self {
            settings,
            preview_ceiling: settings.needs_measurement(),
            ..Self::default()
        })
    }

    /// Use the dry overview to estimate level and leave constant headroom
    /// before the treated recording's loudness and peak are measured.
    pub fn with_preview_level(mut self, mean_square: f64, peak: f64) -> Self {
        if !self.preview_ceiling || !(0.0..=1.0).contains(&peak) {
            return self;
        }
        let manual_db = f64::from(self.settings.gain_db);
        let estimated_gain = if self.settings.normalize && (1e-7..=1.0).contains(&mean_square) {
            let approximate_db = 10.0 * mean_square.log10() + manual_db;
            db_gain((self.settings.normalize_target() - approximate_db).min(MAX_PREVIEW_BOOST_DB))
        } else {
            1.0
        };
        let tone_boost: f64 = [
            self.settings.bass_db,
            self.settings.presence_db,
            self.settings.treble_db,
        ]
        .into_iter()
        .map(|value| f64::from(value.max(0)))
        .sum();
        let width = 1.0 + f64::from(self.settings.stereo_width.max(0)) * 0.01;
        let estimated_peak = peak * db_gain(manual_db + tone_boost) * width;
        // Constant gain keeps ordinary peaks out of the instantaneous guard.
        // The guard remains a fallback for unmeasured filter transients.
        self.gain = if estimated_peak > 0.0 {
            estimated_gain.min(db_gain(CEILING_DBTP) / estimated_peak)
        } else {
            estimated_gain
        };
        self
    }

    pub fn settings(self) -> AudioTreatment {
        self.settings
    }

    pub fn gain_db(self) -> f64 {
        20.0 * self.gain.log10()
    }

    pub fn loudness_lufs(self) -> Option<f64> {
        self.loudness
    }

    /// Prime stateful effects before a seek so they approach their settled
    /// continuous-playback state. Stateless changes start on the exact frame.
    pub fn preview_start(self, start: u64, rate: u32) -> u64 {
        let settings = self.settings;
        let mut decode_start = start;
        for repair in settings.repairs().filter(|repair| start < repair.end_frame) {
            let history = band_repair::history_frames(rate, repair);
            let repair_warmup = repair.start_frame.saturating_sub(history);
            if start > repair_warmup {
                decode_start =
                    decode_start.min(start.max(repair.start_frame).saturating_sub(history));
            }
        }
        if !settings.has_filter() && !settings.has_dynamics() {
            return decode_start;
        }
        let rate = u64::from(rate);
        let mut frames = rate * 2;
        if settings.notch_hz != 0 {
            // A notch very close to Nyquist settles more slowly. Budget for
            // the narrower side of the available band at unusual sample rates.
            let frequency = u64::from(settings.notch_hz);
            let edge = frequency.min((rate / 2).saturating_sub(frequency)).max(1);
            let quality = if settings.notch_q == 0 {
                15
            } else {
                settings.notch_q
            };
            frames = frames.max((rate * u64::from(quality) * 6).div_ceil(edge));
        }
        decode_start.min(start.saturating_sub(frames))
    }

    pub fn processor(self, source: SourceInfo) -> Result<TreatmentProcessor, MediaError> {
        self.processor_at(source, 0)
    }

    /// Begin a processor at the absolute source frame of its first PCM input.
    pub fn processor_at(
        self,
        source: SourceInfo,
        first_frame: u64,
    ) -> Result<TreatmentProcessor, MediaError> {
        if first_frame > MAX_FRAMES {
            return Err(MediaError::Range(
                "Audio processing starts outside the recording",
            ));
        }
        TreatmentProcessor::new(self, source, first_frame)
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
    let declared = recording.probed_source().frame_count == FrameCount::Declared;
    settings.output_info_with_frames(info, declared.then_some(info.frames))?;
    let repair_end = if declared {
        None
    } else {
        settings.repairs().map(|repair| repair.end_frame).max()
    };
    if !settings.needs_measurement() {
        if let Some(end) = repair_end {
            while recording.position() < end {
                if recording.next(&mut cancelled)?.is_none() {
                    return Err(MediaError::Range("Choose a repair inside the recording"));
                }
            }
        }
        return Ok(prepared);
    }
    let mut processor = prepared.processor(info)?;

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
        processor.visit(block, |frame| samples.extend_from_slice(frame));
        meter.add_frames_f64(&samples).map_err(measurement_error)?;
    }
    check_cancelled(&mut cancelled)?;
    if repair_end.is_some_and(|end| end > recording.position()) {
        return Err(MediaError::Range("Choose a repair inside the recording"));
    }
    let loudness = meter.loudness_global().map_err(measurement_error)?;
    let peak = (0..channels)
        .map(|channel| meter.true_peak(channel))
        .try_fold(0.0_f64, |peak, channel| {
            channel.map(|value| peak.max(value))
        })
        .map_err(measurement_error)?;
    prepared.loudness = loudness.is_finite().then_some(loudness);
    let gain_db = match prepared.loudness {
        Some(loudness) if settings.normalize => {
            (settings.normalize_target() - loudness).min(MAX_BOOST_DB)
        }
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
    band_repairs: Vec<BandRepair>,
    next_frame: u64,
    dynamics: Option<Dynamics>,
    frame: Vec<f64>,
    width: f64,
    balance: [f64; 2],
    manual_gain: f64,
}

impl TreatmentProcessor {
    fn new(
        prepared: PreparedTreatment,
        source: SourceInfo,
        first_frame: u64,
    ) -> Result<Self, MediaError> {
        let settings = prepared.settings;
        // Preparation validates against the recording's length. A decoder
        // reopened for a seek may only know the file header's estimate.
        let output = settings.output_info_with_frames(source, None)?;
        let mut filters = Vec::new();
        let rate = f64::from(source.sample_rate);
        if settings.dc_hz != 0 {
            filters.push(Biquad::high_pass(rate, settings.dc_hz));
        }
        if settings.rumble {
            filters.extend(Biquad::cut(
                rate,
                settings.rumble_cutoff(),
                settings.rumble_slope,
                true,
            ));
        }
        let hum_frequency = settings.hum_frequency();
        if hum_frequency != 0 {
            for harmonic in 1..=settings.hum_harmonics.max(1) {
                filters.push(Biquad::notch(rate, hum_frequency * u16::from(harmonic), 10));
            }
        }
        if settings.notch_hz != 0 {
            let quality = if settings.notch_q == 0 {
                15
            } else {
                settings.notch_q
            };
            filters.push(Biquad::notch(rate, settings.notch_hz, quality));
        }
        if settings.high_cut_hz != 0 {
            filters.extend(Biquad::cut(
                rate,
                settings.high_cut_hz,
                settings.high_cut_slope,
                false,
            ));
        }
        let [bass_hz, presence_hz, treble_hz] = settings.tone_frequencies();
        if settings.bass_db != 0 {
            filters.push(Biquad::shelf(rate, bass_hz, settings.bass_db, false));
        }
        if settings.presence_db != 0 {
            let quality = if settings.presence_q == 0 {
                0.8
            } else {
                f64::from(settings.presence_q) / 100.0
            };
            filters.push(Biquad::peak(
                rate,
                presence_hz,
                settings.presence_db,
                quality,
            ));
        }
        if settings.treble_db != 0 {
            filters.push(Biquad::shelf(rate, treble_hz, settings.treble_db, true));
        }
        let balance = f64::from(settings.balance) * 0.01;
        let channels = usize::from(output.channels);
        let band_repairs = settings
            .repairs()
            .map(|repair| BandRepair::new(repair, source.sample_rate, channels))
            .collect();
        Ok(Self {
            prepared,
            source_channels: usize::from(source.channels),
            output,
            filters: vec![filters; channels],
            band_repairs,
            next_frame: first_frame,
            dynamics: Dynamics::new(settings, rate),
            frame: vec![0.0; channels],
            width: 1.0 + f64::from(settings.stereo_width) * 0.01,
            balance: [1.0 - balance.max(0.0), 1.0 + balance.min(0.0)],
            manual_gain: db_gain(f64::from(settings.gain_db)),
        })
    }

    pub fn output_info(&self) -> SourceInfo {
        self.output
    }

    /// The compressor's signed gain in dB on the last processed frame.
    pub fn compression_gain_db(&self) -> f32 {
        self.dynamics
            .as_ref()
            .map_or(0.0, Dynamics::compression_gain_db)
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
        let preview_ceiling = self.prepared.preview_ceiling.then(|| db_gain(CEILING_DBTP));
        self.visit(input, |frame| {
            let gain = preview_ceiling.map_or(gain, |ceiling| {
                let peak = frame_peak(frame);
                if peak > 0.0 {
                    gain.min(ceiling / peak)
                } else {
                    gain
                }
            });
            let scale = gain * PCM_SCALE;
            output.extend(frame.iter().map(|sample| (sample * scale).round() as i32));
        });
    }

    fn visit(&mut self, input: &[i32], mut produced: impl FnMut(&[f64])) {
        let settings = self.prepared.settings;
        let pcm = |sample: i32| f64::from(sample) / PCM_SCALE;
        for input_frame in input.chunks_exact(self.source_channels) {
            if let &[left, right] = input_frame {
                let mut stereo = [pcm(left), pcm(right)];
                if settings.swap_channels {
                    stereo.swap(0, 1);
                }
                match settings.polarity {
                    Polarity::Off => {}
                    Polarity::Left => stereo[0] = -stereo[0],
                    Polarity::Right => stereo[1] = -stereo[1],
                }
                if settings.stereo_width != 0 {
                    let mid = (stereo[0] + stereo[1]) * 0.5;
                    let side = (stereo[0] - stereo[1]) * (0.5 * self.width);
                    stereo = [mid + side, mid - side];
                }
                stereo[0] *= self.balance[0];
                stereo[1] *= self.balance[1];
                match settings.mono {
                    MonoMode::Off => self.frame.copy_from_slice(&stereo),
                    MonoMode::Mix => self.frame[0] = (stereo[0] + stereo[1]) * 0.5,
                    MonoMode::Left => self.frame[0] = stereo[0],
                    MonoMode::Right => self.frame[0] = stereo[1],
                }
            } else {
                for (output, &sample) in self.frame.iter_mut().zip(input_frame) {
                    *output = pcm(sample);
                }
            }
            if !self.band_repairs.is_empty() {
                for repair in &mut self.band_repairs {
                    repair.apply(self.next_frame, &mut self.frame);
                }
                self.next_frame += 1;
            }
            for (sample, filters) in self.frame.iter_mut().zip(&mut self.filters) {
                for filter in filters {
                    *sample = filter.filter(*sample);
                }
            }
            let gain = self.manual_gain
                * self
                    .dynamics
                    .as_mut()
                    .map_or(1.0, |dynamics| dynamics.gain(frame_peak(&self.frame)));
            for sample in &mut self.frame {
                *sample *= gain;
            }
            produced(&self.frame);
        }
    }
}

fn frame_peak(frame: &[f64]) -> f64 {
    frame
        .iter()
        .fold(0.0, |peak, sample| peak.max(sample.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, Write};

    const RATE: u32 = 48_000;

    fn info(channels: u16) -> SourceInfo {
        SourceInfo {
            sample_rate: RATE,
            channels,
            frames: u64::from(RATE),
            bits_per_sample: Some(16),
        }
    }

    fn pcm(amplitude: f64) -> i32 {
        (amplitude * PCM_SCALE).round() as i32
    }

    fn tone(frequency: f64, amplitude: f64) -> Vec<i32> {
        (0..RATE)
            .map(|frame| {
                let angle = f64::from(frame) * std::f64::consts::TAU * frequency / f64::from(RATE);
                pcm(amplitude * angle.sin())
            })
            .collect()
    }

    fn rms(samples: &[i32]) -> f64 {
        let power = samples
            .iter()
            .map(|sample| (f64::from(*sample) / PCM_SCALE).powi(2))
            .sum::<f64>();
        (power / samples.len() as f64).sqrt()
    }

    fn audition(input: &[i32], settings: AudioTreatment, channels: u16) -> Vec<i32> {
        let source = info(channels);
        let mut processor = PreparedTreatment::preview(settings, source)
            .unwrap()
            .processor(source)
            .unwrap();
        let mut output = Vec::new();
        processor.process(input, &mut output);
        output
    }

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
        assert!(
            (tone.loudness_lufs().unwrap() + tone.gain_db() - f64::from(TARGET_LUFS)).abs() < 0.1
        );
        let silent = prepare_treatment(test_audio(0.0, 1000.0), settings, || false).unwrap();
        assert_eq!(silent.gain_db(), 0.0);
        assert_eq!(silent.loudness_lufs(), None);
    }

    #[test]
    fn preview_normalization_uses_overview_without_boosting_silence() {
        let settings = AudioTreatment {
            normalize: true,
            normalize_target_db: -21,
            ..AudioTreatment::default()
        };
        let preview = PreparedTreatment::preview(settings, info(1)).unwrap();
        let quiet = preview.with_preview_level(0.00045, 0.03);
        assert!((quiet.gain_db() - MAX_PREVIEW_BOOST_DB).abs() < 0.01);
        let mut output = Vec::new();
        quiet
            .processor(info(1))
            .unwrap()
            .process(&[pcm(0.03)], &mut output);
        assert!(output[0] > pcm(0.11));
        assert_eq!(preview.with_preview_level(0.0, 0.0).gain_db(), 0.0);
        assert_eq!(preview.with_preview_level(f64::NAN, 0.03).gain_db(), 0.0);
        quiet
            .processor(info(1))
            .unwrap()
            .process(&[pcm(0.5)], &mut output);
        assert_eq!(output[0], pcm(db_gain(CEILING_DBTP)));
    }

    #[test]
    fn preview_gain_uses_constant_headroom_before_the_peak_guard() {
        let settings = AudioTreatment {
            gain_db: 12,
            ..AudioTreatment::default()
        };
        let prepared = PreparedTreatment::preview(settings, info(1))
            .unwrap()
            .with_preview_level(0.18, 0.6);
        let input: Vec<_> = (0..48)
            .map(|frame| {
                let phase = f64::from(frame) * std::f64::consts::TAU / 48.0;
                (phase.sin() * 0.6 * PCM_SCALE).round() as i32
            })
            .collect();
        let mut output = Vec::new();
        prepared
            .processor(info(1))
            .unwrap()
            .process(&input, &mut output);
        let constant_gain = db_gain(CEILING_DBTP) / 0.6;
        for (actual, input) in output.into_iter().zip(input) {
            assert!((f64::from(actual) - f64::from(input) * constant_gain).abs() <= 2.0);
        }
    }

    #[test]
    fn legacy_draft_and_dormant_parameters_leave_dry_audio_inactive() {
        let draft = serde::de::value::MapDeserializer::<_, serde::de::value::Error>::new(
            [("rumble", false), ("normalize", false)].into_iter(),
        );
        let settings = AudioTreatment::deserialize(draft).unwrap();
        assert_eq!(settings, AudioTreatment::default());
        let settings = AudioTreatment {
            normalize_target_db: -23,
            rumble_hz: 50,
            rumble_slope: 24,
            high_cut_slope: 48,
            hum_harmonics: 3,
            notch_q: 25,
            bass_hz: 100,
            presence_hz: 2500,
            presence_q: 125,
            treble_hz: 5000,
            compressor_ratio: 4,
            gate_reduction_db: 18,
            ..settings
        };
        assert!(!settings.is_active());
        assert!(!settings.needs_measurement());
        let input = [i32::MIN, 77, 1, i32::MAX];
        assert_eq!(audition(&input, settings, 1), input);
    }

    #[test]
    fn settings_validate_channel_layout_frequency_and_parameter_bounds() {
        let settings = AudioTreatment {
            swap_channels: true,
            ..AudioTreatment::default()
        };
        assert!(PreparedTreatment::preview(settings, info(1)).is_err());
        assert!(!settings.needs_measurement());
        assert_eq!(
            settings.output_info(info(2)).unwrap().bits_per_sample,
            Some(16)
        );
        for settings in [
            AudioTreatment {
                high_cut_hz: 8000,
                ..AudioTreatment::default()
            },
            AudioTreatment {
                notch_hz: 700,
                notch_q: 31,
                ..AudioTreatment::default()
            },
            AudioTreatment {
                compressor_db: -20,
                compressor_ratio: 1,
                ..AudioTreatment::default()
            },
            AudioTreatment {
                gate_db: -55,
                gate_reduction_db: 2,
                ..AudioTreatment::default()
            },
            AudioTreatment {
                rumble_slope: 13,
                ..AudioTreatment::default()
            },
            AudioTreatment {
                presence_q: 1201,
                ..AudioTreatment::default()
            },
            AudioTreatment {
                treble_hz: 19,
                ..AudioTreatment::default()
            },
        ] {
            assert!(PreparedTreatment::preview(
                settings,
                SourceInfo {
                    sample_rate: 8000,
                    ..info(2)
                }
            )
            .is_err());
        }
    }

    #[test]
    fn sample_rate_limits_only_active_eq_bands() {
        let source = SourceInfo {
            sample_rate: 8000,
            ..info(2)
        };
        let settings = AudioTreatment {
            bass_hz: 3600,
            presence_hz: 3601,
            treble_hz: 10_000,
            ..AudioTreatment::default()
        };
        assert!(settings.output_info(source).is_ok());
        assert!(AudioTreatment {
            bass_db: 1,
            ..settings
        }
        .output_info(source)
        .is_ok());
        assert!(AudioTreatment {
            presence_db: 1,
            ..settings
        }
        .output_info(source)
        .is_err());
        assert!(AudioTreatment {
            treble_db: 1,
            ..settings
        }
        .output_info(source)
        .is_err());
    }

    #[test]
    fn cleanup_filters_reject_dc_harmonics_and_hiss_without_removing_the_passband() {
        let fixtures = [
            (
                AudioTreatment {
                    dc_hz: 5,
                    ..AudioTreatment::default()
                },
                0.0,
                40.0,
                0.001,
            ),
            (
                AudioTreatment {
                    hum: HumFilter::Hz60,
                    hum_harmonics: 3,
                    ..AudioTreatment::default()
                },
                180.0,
                550.0,
                0.001,
            ),
            (
                AudioTreatment {
                    notch_hz: 1000,
                    notch_q: 20,
                    ..AudioTreatment::default()
                },
                1000.0,
                750.0,
                0.001,
            ),
            (
                AudioTreatment {
                    high_cut_hz: 4000,
                    ..AudioTreatment::default()
                },
                16000.0,
                1000.0,
                0.05,
            ),
        ];
        let settled = RATE as usize / 2;
        for (settings, stop, pass, max_ratio) in fixtures {
            let stopband = if stop == 0.0 {
                vec![pcm(0.2); RATE as usize]
            } else {
                tone(stop, 0.2)
            };
            let rejected = audition(&stopband, settings, 1);
            assert!(
                rms(&rejected[settled..]) / rms(&stopband[settled..]) < max_ratio,
                "{settings:?}"
            );
            let passband = tone(pass, 0.2);
            let allowed = audition(&passband, settings, 1);
            assert!(
                rms(&allowed[settled..]) / rms(&passband[settled..]) > 0.97,
                "{settings:?}"
            );
        }
    }

    #[test]
    fn tone_controls_shape_three_distinct_bands_and_report_processed_precision() {
        for (settings, in_band, out_of_band, db) in [
            (
                AudioTreatment {
                    bass_db: 6,
                    ..AudioTreatment::default()
                },
                30.0,
                9000.0,
                6.0,
            ),
            (
                AudioTreatment {
                    presence_db: -6,
                    ..AudioTreatment::default()
                },
                1500.0,
                90.0,
                -6.0,
            ),
            (
                AudioTreatment {
                    treble_db: 6,
                    ..AudioTreatment::default()
                },
                13000.0,
                300.0,
                6.0,
            ),
        ] {
            assert!((tone_gain(settings, in_band) - db).abs() < 0.25);
            assert!(tone_gain(settings, out_of_band).abs() < 0.1);
            assert_eq!(
                settings.output_info(info(1)).unwrap().bits_per_sample,
                Some(24)
            );
        }
    }

    fn tone_gain(settings: AudioTreatment, frequency: f64) -> f64 {
        let input = tone(frequency, 0.1);
        let output = audition(&input, settings, 1);
        let settled = RATE as usize / 2;
        20.0 * (rms(&output[settled..]) / rms(&input[settled..])).log10()
    }

    #[test]
    fn presence_center_and_q_change_the_native_response() {
        let legacy = AudioTreatment {
            presence_db: 6,
            ..AudioTreatment::default()
        };
        let shifted = AudioTreatment {
            presence_hz: 3000,
            ..legacy
        };
        let narrow = AudioTreatment {
            presence_q: 400,
            ..shifted
        };
        assert!((tone_gain(legacy, 1500.0) - 6.0).abs() < 0.01);
        assert!((tone_gain(shifted, 3000.0) - 6.0).abs() < 0.01);
        assert!((tone_gain(narrow, 3000.0) - 6.0).abs() < 0.01);
        assert!(tone_gain(shifted, 1500.0) > tone_gain(narrow, 1500.0) + 2.0);
    }

    #[test]
    fn shelf_centers_follow_the_configured_frequencies() {
        for (settings, frequency) in [
            (
                AudioTreatment {
                    bass_db: 6,
                    bass_hz: 480,
                    ..AudioTreatment::default()
                },
                480.0,
            ),
            (
                AudioTreatment {
                    treble_db: 6,
                    treble_hz: 7500,
                    ..AudioTreatment::default()
                },
                7500.0,
            ),
        ] {
            assert!((tone_gain(settings, frequency) - 3.0).abs() < 0.01);
        }
    }

    #[test]
    fn steeper_cutoffs_keep_the_same_corner_and_reject_more_outside_it() {
        let rumble = |slope| AudioTreatment {
            rumble: true,
            rumble_hz: 50,
            rumble_slope: slope,
            ..AudioTreatment::default()
        };
        let high_cut = |slope| AudioTreatment {
            high_cut_hz: 6000,
            high_cut_slope: slope,
            ..AudioTreatment::default()
        };
        let fixture = tone(110.0, 0.1);
        assert_eq!(
            audition(&fixture, rumble(0), 1),
            audition(&fixture, rumble(12), 1)
        );
        assert_eq!(
            audition(&fixture, high_cut(0), 1),
            audition(&fixture, high_cut(12), 1)
        );
        for slope in [24, 36, 48] {
            assert!((tone_gain(rumble(slope), 50.0) + 3.0103).abs() < 0.01);
            assert!((tone_gain(high_cut(slope), 6000.0) + 3.0103).abs() < 0.01);
        }
        assert!(tone_gain(rumble(48), 25.0) < tone_gain(rumble(12), 25.0) - 30.0);
        assert!(tone_gain(high_cut(48), 12000.0) < tone_gain(high_cut(12), 12000.0) - 30.0);
    }

    #[test]
    fn compressor_meter_reports_the_latest_applied_reduction() {
        let settings = AudioTreatment {
            compressor_db: -20,
            gate_db: -30,
            ..AudioTreatment::default()
        };
        let source = info(1);
        let mut compressor = PreparedTreatment::preview(settings, source)
            .unwrap()
            .processor(source)
            .unwrap();
        let mut output = Vec::new();
        assert_eq!(compressor.compression_gain_db(), 0.0);
        compressor.process(&[pcm(0.5); 4096], &mut output);
        assert!((compressor.compression_gain_db() + 6.99).abs() < 0.01);
        compressor.process(&[pcm(0.001); 24_000], &mut output);
        assert_eq!(compressor.compression_gain_db(), 0.0);
        assert!(output.last().unwrap() < &pcm(0.001));
    }

    fn spectral_fixture() -> SpectralRepair {
        SpectralRepair {
            start_frame: u64::from(RATE) / 4,
            end_frame: u64::from(RATE) * 3 / 4,
            low_hz: 800,
            high_hz: 1250,
            amount_db: 24,
        }
    }

    fn with_spectral_repair(repair: SpectralRepair) -> AudioTreatment {
        let mut repairs = [None; MAX_SPECTRAL_REPAIRS];
        repairs[0] = Some(repair);
        AudioTreatment {
            spectral_repairs: Some(repairs),
            ..AudioTreatment::default()
        }
    }

    #[test]
    fn spectral_repair_attenuates_its_band_in_its_time_span_only() {
        let repair = spectral_fixture();
        let settings = with_spectral_repair(repair);
        let input: Vec<_> = tone(1000.0, 0.2)
            .into_iter()
            .zip(tone(6000.0, 0.2))
            .flat_map(|(inside, outside)| [inside, outside])
            .collect();
        assert!(settings.is_active());
        assert!(!settings.needs_measurement());
        let output = audition(&input, settings, 2);
        let start = repair.start_frame as usize * 2;
        let end = repair.end_frame as usize * 2;
        assert_eq!(output[..start], input[..start]);
        assert_eq!(output[end..], input[end..]);
        assert!((f64::from(output[start + 1]) - f64::from(input[start + 1])).abs() < 10_000.0);

        let settled = start + RATE as usize / 4..end - RATE as usize / 4;
        for (channel, expected_db) in [(0, -24.0), (1, 0.0)] {
            let power = |samples: &[i32]| {
                samples[settled.clone()]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|frame| (f64::from(frame[channel]) / PCM_SCALE).powi(2))
                    .sum::<f64>()
            };
            let gain_db = 10.0 * (power(&output) / power(&input)).log10();
            assert!((gain_db - expected_db).abs() < 0.04, "{gain_db}");
        }
    }

    #[test]
    fn spectral_repair_validates_source_coordinates_and_keeps_legacy_drafts() {
        let repair = spectral_fixture();
        for invalid in [
            SpectralRepair {
                start_frame: repair.end_frame,
                ..repair
            },
            SpectralRepair {
                end_frame: u64::from(RATE) + 1,
                ..repair
            },
            SpectralRepair {
                low_hz: 19,
                ..repair
            },
            SpectralRepair {
                high_hz: repair.low_hz + 19,
                ..repair
            },
            SpectralRepair {
                high_hz: 18_000,
                ..repair
            },
            SpectralRepair {
                amount_db: 37,
                ..repair
            },
        ] {
            assert!(with_spectral_repair(invalid)
                .output_info(SourceInfo {
                    sample_rate: 32_000,
                    ..info(1)
                })
                .is_err());
        }
        assert_eq!(AudioTreatment::default().spectral_repairs, None);
        assert_eq!(
            with_spectral_repair(repair)
                .output_info(info(1))
                .unwrap()
                .bits_per_sample,
            Some(24)
        );
    }

    #[test]
    fn spectral_repair_matches_flac_export_when_preview_starts_mid_edit() {
        use crate::{render_treated_file, FrameRange, OutputFormat, PreviewDecoder, RenderSegment};

        let rate = u64::from(RATE);
        let settings = with_spectral_repair(SpectralRepair {
            start_frame: rate / 2,
            end_frame: rate * 3 / 2,
            ..spectral_fixture()
        });
        let source = SourceInfo {
            frames: rate * 2,
            ..info(1)
        };
        let prepared = PreparedTreatment::preview(settings, source).unwrap();
        let start = rate * 3 / 4;
        let end = rate * 7 / 4;
        let warmup = prepared.preview_start(start, RATE);
        assert!(warmup > 0 && warmup < start);
        let mut decoder = PreviewDecoder::open(test_audio(0.2, 1000.0), warmup, || false).unwrap();
        let mut processor = prepared.processor_at(source, warmup).unwrap();
        let mut preview = Vec::new();
        let mut chunk = Vec::new();
        while let Some(block) = decoder.next_block(|| false).unwrap() {
            processor.process(block, &mut chunk);
            preview.extend_from_slice(&chunk);
        }

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("repaired.flac");
        render_treated_file(
            test_audio(0.2, 1000.0),
            &[RenderSegment {
                range: FrameRange { start, end },
                output: path.clone(),
            }],
            OutputFormat::Flac,
            prepared,
            || false,
            |_| {},
            |output| {
                File::options()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(output)
            },
        )
        .unwrap();
        let mut export = Recording::open(File::open(path).unwrap()).unwrap();
        let mut rendered = Vec::new();
        while let Some(block) = export.next(&mut || false).unwrap() {
            rendered.extend_from_slice(block);
        }
        assert_eq!(rendered.len(), (end - start) as usize);
        for (actual, preview) in rendered.iter().zip(&preview[(start - warmup) as usize..]) {
            assert!((i64::from(*actual) - i64::from(*preview)).abs() < 256);
        }
        assert!(
            rms(&rendered[..RATE as usize / 4]) < rms(&rendered[7 * RATE as usize / 8..]) * 0.07
        );
    }

    #[test]
    fn stereo_mapping_precedes_mono_selection_and_preserves_frame_count() {
        let stereo = [pcm(0.25), pcm(0.5), pcm(-0.5), pcm(0.0)];
        let swap = AudioTreatment {
            swap_channels: true,
            polarity: Polarity::Left,
            ..AudioTreatment::default()
        };
        assert_eq!(
            audition(&stereo, swap, 2),
            [pcm(-0.5), pcm(0.25), 0, pcm(-0.5)]
        );
        assert_eq!(
            audition(
                &stereo,
                AudioTreatment {
                    mono: MonoMode::Left,
                    ..swap
                },
                2
            ),
            [pcm(-0.5), 0]
        );
        let field = AudioTreatment {
            stereo_width: -100,
            balance: 100,
            ..AudioTreatment::default()
        };
        assert_eq!(audition(&stereo, field, 2), [0, pcm(0.375), 0, pcm(-0.25)]);
    }

    #[test]
    fn dynamics_attenuate_both_channels_and_preview_ceiling_preserves_balance() {
        let compressor = AudioTreatment {
            compressor_db: -18,
            compressor_ratio: 4,
            ..AudioTreatment::default()
        };
        let loud = [pcm(0.6), pcm(0.3)].repeat(RATE as usize);
        let compressed = audition(&loud, compressor, 2);
        let pair = &compressed[compressed.len() - 2..];
        assert!((0.17..0.20).contains(&(f64::from(pair[0]) / PCM_SCALE)));
        assert!((pair[0] - pair[1] * 2).abs() <= 1);

        let expander = AudioTreatment {
            gate_db: -40,
            gate_reduction_db: 18,
            ..AudioTreatment::default()
        };
        let quiet = [pcm(0.001), pcm(0.0005)].repeat(RATE as usize);
        let expanded = audition(&quiet, expander, 2);
        let pair = &expanded[expanded.len() - 2..];
        assert!((f64::from(pair[0]) / f64::from(quiet[0]) - db_gain(-18.0)).abs() < 0.001);
        assert!((pair[0] - pair[1] * 2).abs() <= 1);

        let boost = AudioTreatment {
            gain_db: 12,
            ..AudioTreatment::default()
        };
        let limited = audition(&loud[..2], boost, 2);
        assert_eq!(limited[0], pcm(db_gain(CEILING_DBTP)));
        assert!((limited[0] - limited[1] * 2).abs() <= 4);
    }

    #[test]
    fn stateful_effects_are_packet_independent_and_seek_preroll_settles() {
        let settings = AudioTreatment {
            dc_hz: 2,
            high_cut_hz: 8000,
            compressor_db: -24,
            gate_db: -48,
            ..AudioTreatment::default()
        };
        let source = info(1);
        let prepared = PreparedTreatment::preview(settings, source).unwrap();
        let input = tone(275.0, 0.4).repeat(4);
        let mut continuous = Vec::new();
        prepared
            .processor(source)
            .unwrap()
            .process(&input, &mut continuous);
        let mut split = prepared.processor(source).unwrap();
        let mut block = Vec::new();
        let mut joined = Vec::new();
        for samples in input.chunks(711) {
            split.process(samples, &mut block);
            joined.extend_from_slice(&block);
        }
        assert_eq!(joined, continuous);
        let start = 3 * u64::from(RATE);
        let head = prepared.preview_start(start, RATE) as usize;
        let skip = start as usize - head;
        let mut seek = prepared.processor(source).unwrap();
        seek.process(&input[head..], &mut block);
        assert_eq!(block.len() - skip, continuous.len() - start as usize);
        assert!(block[skip..]
            .iter()
            .zip(&continuous[start as usize..])
            .all(|(a, b)| a.abs_diff(*b) <= 64));
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
