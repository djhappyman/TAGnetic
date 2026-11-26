use serde::{Deserialize, Serialize};

use super::{db_gain, deserialize_slots, Biquad, MediaError};

pub const MAX_EQ_BANDS: usize = 14;
pub(super) const MAX_EQ_GAIN_DB: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EqBandKind {
    Low,
    Bass,
    #[default]
    Bell,
    Notch,
    Treble,
    High,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EqChannel {
    #[default]
    Stereo,
    Left,
    Right,
}

impl EqChannel {
    pub(super) fn index(self) -> Option<usize> {
        match self {
            Self::Stereo => None,
            Self::Left => Some(0),
            Self::Right => Some(1),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicEq {
    pub threshold_db: f32,
    /// Positive reduces loud input. Negative lifts quiet input below the threshold.
    pub amount_db: f32,
    #[serde(default = "default_dynamic_ratio")]
    pub ratio: f32,
}

fn default_dynamic_ratio() -> f32 {
    2.0
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EqBand {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub kind: EqBandKind,
    pub hz: u16,
    pub gain_db: f32,
    pub q: u16,
    #[serde(default)]
    pub bypassed: bool,
    #[serde(default)]
    pub channel: EqChannel,
    #[serde(default)]
    pub slope: u8,
    #[serde(default)]
    pub adaptive_q: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<DynamicEq>,
}

impl EqBand {
    pub(super) fn active(self) -> bool {
        !self.bypassed
            && (matches!(
                self.kind,
                EqBandKind::Low | EqBandKind::High | EqBandKind::Notch
            ) || self.gain_db != 0.0
                || self.dynamic.is_some())
    }

    pub(super) fn validate(self, sample_rate: u32) -> Result<(), MediaError> {
        let quality = if self.kind == EqBandKind::Notch {
            500..=3000
        } else {
            20..=1200
        };
        if !(20..=20_000).contains(&self.hz)
            || !(-MAX_EQ_GAIN_DB..=MAX_EQ_GAIN_DB).contains(&self.gain_db)
            || !quality.contains(&self.q)
            || !matches!(self.slope, 0 | 12 | 24 | 36 | 48)
            || (self.active() && f64::from(self.hz) > f64::from(sample_rate) * 0.45)
            || self.dynamic.is_some_and(|dynamic| {
                !(-72.0..=-6.0).contains(&dynamic.threshold_db)
                    || !(1.0..=12.0).contains(&dynamic.amount_db.abs())
                    || !(1.0..=20.0).contains(&dynamic.ratio)
            })
        {
            return Err(MediaError::Unsupported(
                "Choose a valid frequency, gain, width and shape for this EQ band",
            ));
        }
        Ok(())
    }

    pub(super) fn filters(self, rate: f64) -> Vec<Biquad> {
        let q = self.effective_quality();
        match self.kind {
            EqBandKind::Low | EqBandKind::High => {
                Biquad::cut(rate, self.hz, self.slope, self.kind == EqBandKind::Low).collect()
            }
            EqBandKind::Bass | EqBandKind::Treble => {
                vec![Biquad::shelf(
                    rate,
                    self.hz,
                    self.gain_db,
                    self.kind == EqBandKind::Treble,
                )]
            }
            EqBandKind::Bell if self.gain_db == 0.0 => Vec::new(),
            EqBandKind::Bell => vec![Biquad::peak(rate, self.hz, self.gain_db, q)],
            EqBandKind::Notch => vec![Biquad::notch(rate, self.hz, q)],
        }
    }

    pub(super) fn effective_quality(self) -> f64 {
        let quality = f64::from(self.q) / 100.0;
        if self.kind == EqBandKind::Bell && self.adaptive_q {
            (quality * (1.0 + f64::from(self.gain_db.abs()) / 12.0)).min(12.0)
        } else {
            quality
        }
    }
}

pub(super) struct DynamicBand {
    pub(super) index: usize,
    channels: Vec<([Biquad; 2], Biquad)>,
    envelope: f64,
    threshold: f64,
    upward: bool,
    exponent: f64,
    range_scale: f64,
    blend: f64,
    blend_step: f64,
    samples_left: u8,
    attack: f64,
    release: f64,
}

impl DynamicBand {
    pub(super) fn new(index: usize, band: EqBand, rate: f64, channels: usize) -> Option<Self> {
        let dynamic = band.dynamic?;
        let q = band.effective_quality();
        let (detector, filter) = match band.kind {
            EqBandKind::Low | EqBandKind::Bass => (
                Biquad::low_pass(rate, band.hz),
                Biquad::shelf(rate, band.hz, -dynamic.amount_db, false),
            ),
            EqBandKind::High | EqBandKind::Treble => (
                Biquad::high_pass(rate, band.hz),
                Biquad::shelf(rate, band.hz, -dynamic.amount_db, true),
            ),
            // Keep the notch in the static chain and treat its nearby frequencies.
            EqBandKind::Bell | EqBandKind::Notch => (
                Biquad::band_detector(rate, band.hz, q),
                Biquad::peak(rate, band.hz, -dynamic.amount_db, q),
            ),
        };
        let threshold = db_gain(f64::from(dynamic.threshold_db));
        let upward = dynamic.amount_db < 0.0;
        Some(Self {
            index,
            channels: vec![([detector.clone(), detector], filter); channels],
            envelope: if upward { threshold } else { 0.0 },
            threshold,
            upward,
            exponent: 1.0 - f64::from(dynamic.ratio).recip(),
            range_scale: (db_gain(-f64::from(dynamic.amount_db)) - 1.0).recip(),
            blend: 0.0,
            blend_step: 0.0,
            samples_left: 0,
            attack: 1.0 - (-1.0 / (rate * 0.012)).exp(),
            release: 1.0 - (-1.0 / (rate * 0.15)).exp(),
        })
    }

    pub(super) fn mix(&self) -> f64 {
        self.blend
    }

    fn target_mix(&self) -> f64 {
        let level = self.envelope / self.threshold;
        let triggered = if self.upward {
            self.envelope > 1e-7 && level < 1.0
        } else {
            level > 1.0
        };
        if !triggered || self.exponent == 0.0 {
            return 0.0;
        }
        // Convert the compressor's centre gain to a blend of dry and full-range EQ.
        ((level.powf(-self.exponent) - 1.0) * self.range_scale).clamp(0.0, 1.0)
    }

    pub(super) fn apply(&mut self, frame: &mut [f64], sidechain: &[f64]) {
        const GAIN_INTERVAL: u8 = 16;
        let peak = sidechain.iter().zip(&mut self.channels).fold(
            0.0_f64,
            |peak, (sample, ([first, second], _))| {
                peak.max(second.filter(first.filter(*sample)).abs())
            },
        );
        let speed = if peak > self.envelope {
            self.attack
        } else {
            self.release
        };
        self.envelope += (peak - self.envelope) * speed;
        if self.envelope < 1e-24 {
            self.envelope = 0.0;
        }
        // Interpolate the gain between control updates to avoid audio-rate powers.
        if self.samples_left == 0 {
            self.blend_step = (self.target_mix() - self.blend) / f64::from(GAIN_INTERVAL);
            self.samples_left = GAIN_INTERVAL;
        }
        self.samples_left -= 1;
        self.blend = (self.blend + self.blend_step).clamp(0.0, 1.0);
        let blend = self.blend;
        for (sample, (_, filter)) in frame.iter_mut().zip(&mut self.channels) {
            *sample += (filter.filter(*sample) - *sample) * blend;
        }
    }
}

pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<[Option<EqBand>; MAX_EQ_BANDS]>, D::Error> {
    deserialize_slots(deserializer)
}
