//! A causal, channel-linked compressor and gentle downward expander.

use super::{db_gain, AudioTreatment};

pub(super) struct Dynamics {
    compressor: Option<(f64, f64)>,
    gate: Option<(f64, f64)>,
    attack: f64,
    release: f64,
    envelope: f64,
}

impl Dynamics {
    pub(super) fn new(settings: AudioTreatment, rate: f64) -> Option<Self> {
        settings.has_dynamics().then(|| Self {
            compressor: (settings.compressor_db != 0).then(|| {
                let ratio = f64::from(settings.compressor_ratio.max(2));
                (
                    db_gain(f64::from(settings.compressor_db)),
                    1.0 - 1.0 / ratio,
                )
            }),
            gate: (settings.gate_db != 0).then(|| {
                let reduction = if settings.gate_reduction_db == 0 {
                    12
                } else {
                    settings.gate_reduction_db
                };
                (
                    db_gain(f64::from(settings.gate_db)),
                    db_gain(-f64::from(reduction)),
                )
            }),
            attack: 1.0 - (-1.0 / (rate * 0.008)).exp(),
            release: 1.0 - (-1.0 / (rate * 0.160)).exp(),
            envelope: 0.0,
        })
    }

    pub(super) fn gain(&mut self, peak: f64) -> f64 {
        let speed = if peak > self.envelope {
            self.attack
        } else {
            self.release
        };
        self.envelope += (peak - self.envelope) * speed;
        if self.envelope < 1e-24 {
            self.envelope = 0.0;
        }
        let mut gain = 1.0;
        if let Some((threshold, exponent)) = self.compressor {
            if self.envelope > threshold {
                let attenuation = threshold / self.envelope;
                gain *= if exponent == 0.5 {
                    attenuation.sqrt()
                } else {
                    attenuation.powf(exponent)
                };
            }
        }
        if let Some((threshold, floor)) = self.gate {
            gain *= (self.envelope / threshold).clamp(floor, 1.0);
        }
        gain
    }

    pub(super) fn compression_gain_db(&self) -> f32 {
        let Some((threshold, exponent)) = self.compressor else {
            return 0.0;
        };
        if self.envelope <= threshold {
            return 0.0;
        }
        (20.0 * exponent * (threshold / self.envelope).log10()) as f32
    }
}
