//! Causal, channel-linked compression and gentle downward expansion.

use super::{db_gain, AudioTreatment};

struct Envelope {
    attack: f64,
    release: f64,
    level: f64,
}

impl Envelope {
    fn new(rate: f64, attack_ms: u16, release_ms: u16) -> Self {
        let coefficient = |ms| 1.0 - (-1000.0 / (rate * f64::from(ms))).exp();
        Self {
            attack: coefficient(attack_ms),
            release: coefficient(release_ms),
            level: 0.0,
        }
    }

    fn advance(&mut self, peak: f64) {
        let speed = if peak > self.level {
            self.attack
        } else {
            self.release
        };
        self.level += (peak - self.level) * speed;
        if self.level < 1e-24 {
            self.level = 0.0;
        }
    }
}

struct Knee {
    gain_begins: f64,
    gain_settles: f64,
    half_width: f64,
    curve: f64,
}

struct Compressor {
    detector: Envelope,
    input_gain: f64,
    threshold: f64,
    strength: f64,
    knee: Option<Knee>,
}

impl Compressor {
    fn gain(&self) -> f64 {
        let level = self.detector.level;
        if let Some(knee) = &self.knee {
            if level <= knee.gain_begins {
                return 1.0;
            }
            if level < knee.gain_settles {
                let position = 20.0 * (level / self.threshold).log10() + knee.half_width;
                return db_gain(knee.curve * position * position);
            }
        }
        if level <= self.threshold {
            return 1.0;
        }
        let attenuation = self.threshold / level;
        if self.strength == 0.5 {
            attenuation.sqrt()
        } else {
            attenuation.powf(self.strength)
        }
    }
}

struct Gate {
    detector: Envelope,
    threshold: f64,
    floor: f64,
}

impl Gate {
    fn gain(&self) -> f64 {
        (self.detector.level / self.threshold).clamp(self.floor, 1.0)
    }
}

pub(super) struct Dynamics {
    compressor: Option<Compressor>,
    gate: Option<Gate>,
}

impl Dynamics {
    pub(super) fn new(settings: AudioTreatment, rate: f64) -> Option<Self> {
        const DEFAULT_ATTACK: u16 = 8;
        const DEFAULT_RELEASE: u16 = 160;
        settings.has_dynamics().then(|| Self {
            compressor: (settings.compressor_db != 0).then(|| {
                let ratio = f64::from(settings.compressor_ratio.max(2));
                let threshold = db_gain(f64::from(settings.compressor_db));
                let strength = 1.0 - 1.0 / ratio;
                let attack = match settings.compressor_attack_ms {
                    0 => DEFAULT_ATTACK,
                    value => value,
                };
                let release = match settings.compressor_release_ms {
                    0 => DEFAULT_RELEASE,
                    value => value,
                };
                let knee = (settings.compressor_knee_db != 0).then(|| {
                    let width = f64::from(settings.compressor_knee_db);
                    Knee {
                        gain_begins: threshold * db_gain(-width / 2.0),
                        gain_settles: threshold * db_gain(width / 2.0),
                        half_width: width / 2.0,
                        curve: -strength / (2.0 * width),
                    }
                });
                Compressor {
                    detector: Envelope::new(rate, attack, release),
                    input_gain: db_gain(f64::from(settings.compressor_input_db)),
                    threshold,
                    strength,
                    knee,
                }
            }),
            gate: (settings.gate_db != 0).then(|| {
                let reduction = if settings.gate_reduction_db == 0 {
                    12
                } else {
                    settings.gate_reduction_db
                };
                Gate {
                    // Gate timing is independent of the compressor controls.
                    detector: Envelope::new(rate, DEFAULT_ATTACK, DEFAULT_RELEASE),
                    threshold: db_gain(f64::from(settings.gate_db)),
                    floor: db_gain(-f64::from(reduction)),
                }
            }),
        })
    }

    fn advance(&mut self, peak: f64) -> (f64, f64) {
        let compressor = self.compressor.as_mut().map_or(1.0, |compressor| {
            compressor.detector.advance(peak * compressor.input_gain);
            compressor.gain()
        });
        let gate = self.gate.as_mut().map_or(1.0, |gate| {
            gate.detector.advance(peak);
            gate.gain()
        });
        (compressor, gate)
    }

    pub(super) fn gain(&mut self, peak: f64) -> f64 {
        let (compressor, gate) = self.advance(peak);
        compressor * gate
    }

    /// Gain of the signal removed by the compressor, with the same gate.
    pub(super) fn difference_gain(&mut self, peak: f64) -> f64 {
        let (compressor, gate) = self.advance(peak);
        (1.0 - compressor) * gate
    }

    pub(super) fn compression_gain_db(&self) -> f32 {
        self.compressor
            .as_ref()
            .map_or(0.0, |compressor| gain_db(compressor.gain()))
    }

    pub(super) fn compressor_detector_db(&self) -> Option<f32> {
        self.compressor
            .as_ref()
            .map(|compressor| gain_db(compressor.detector.level.max(db_gain(-96.0))))
    }

    pub(super) fn gate_gain_db(&self) -> f32 {
        self.gate.as_ref().map_or(0.0, |gate| gain_db(gate.gain()))
    }
}

fn gain_db(value: f64) -> f32 {
    (20.0 * value.log10()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn recipe() -> AudioTreatment {
        AudioTreatment {
            compressor_db: -20,
            compressor_ratio: 4,
            gate_db: -42,
            ..AudioTreatment::default()
        }
    }

    #[test]
    fn compressor_controls_leave_gate_timing_unchanged() {
        let mut original = Dynamics::new(recipe(), RATE).unwrap();
        let mut custom = Dynamics::new(
            AudioTreatment {
                compressor_attack_ms: 100,
                compressor_release_ms: 300,
                ..recipe()
            },
            RATE,
        )
        .unwrap();
        for index in 0..4096 {
            let peak = if index < 1024 { 0.3 } else { 0.001 };
            original.gain(peak);
            custom.gain(peak);
            assert_eq!(original.gate_gain_db(), custom.gate_gain_db());
        }
        assert!(
            original.compressor_detector_db().unwrap() > custom.compressor_detector_db().unwrap()
        );
    }

    #[test]
    fn soft_knee_blends_into_the_hard_knee_response() {
        let mut hard = Dynamics::new(recipe(), RATE).unwrap();
        let mut soft = Dynamics::new(
            AudioTreatment {
                compressor_knee_db: 12,
                ..recipe()
            },
            RATE,
        )
        .unwrap();
        for db in [-40.0, -26.0, -20.0, -14.0, -6.0] {
            hard.compressor.as_mut().unwrap().detector.level = db_gain(db);
            soft.compressor.as_mut().unwrap().detector.level = db_gain(db);
            let hard_db = hard.compression_gain_db();
            let soft_db = soft.compression_gain_db();
            if db == -20.0 {
                assert!((soft_db + 1.125).abs() < 0.001);
                assert_eq!(hard_db, 0.0);
            } else {
                assert!((soft_db - hard_db).abs() < 0.001);
            }
        }
    }

    #[test]
    fn difference_plus_treated_matches_compressor_bypassed_signal() {
        let configured = AudioTreatment {
            compressor_knee_db: 8,
            ..recipe()
        };
        let mut treated = Dynamics::new(configured, RATE).unwrap();
        let mut difference = Dynamics::new(configured, RATE).unwrap();
        let mut bypassed = Dynamics::new(
            AudioTreatment {
                compressor_db: 0,
                ..configured
            },
            RATE,
        )
        .unwrap();
        for frame in 0..4096 {
            let peak = if frame < 2048 { 0.5 } else { 0.002 };
            let reconstructed = treated.gain(peak) + difference.difference_gain(peak);
            assert!((reconstructed - bypassed.gain(peak)).abs() < 1e-12);
        }
    }
}
