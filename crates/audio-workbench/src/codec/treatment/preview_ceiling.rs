//! Linked peak protection for immediate audition before the recording is measured.

const RELEASE_SECONDS: f64 = 0.1;
const UNITY_TOLERANCE: f64 = 1e-6;

pub(super) struct PreviewCeiling {
    ceiling: f64,
    release_step: f64,
    attenuation: f64,
}

impl PreviewCeiling {
    pub(super) fn history_frames(sample_rate: u32) -> u64 {
        // After eight release constants any unprimed level difference is below 0.003 dB.
        (f64::from(sample_rate) * RELEASE_SECONDS * 8.0).ceil() as u64
    }

    pub(super) fn new(sample_rate: u32, ceiling: f64) -> Self {
        Self {
            ceiling,
            release_step: -(-1.0 / (f64::from(sample_rate) * RELEASE_SECONDS)).exp_m1(),
            attenuation: 1.0,
        }
    }

    /// One gain for the entire frame preserves balance while keeping peaks below the ceiling.
    pub(super) fn gain(&mut self, peak: f64, base_gain: f64) -> f64 {
        let boosted = peak * base_gain;
        if self.attenuation == 1.0 && boosted <= self.ceiling {
            return base_gain;
        }

        let maximum = if boosted > self.ceiling {
            self.ceiling / boosted
        } else {
            1.0
        };
        let remaining = 1.0 - self.attenuation;
        let recovered = if remaining <= UNITY_TOLERANCE {
            1.0
        } else {
            self.attenuation + remaining * self.release_step
        };
        // Reduce gain immediately on a new peak and release it gradually
        // between peaks to avoid reshaping every cycle of an overloaded tone.
        self.attenuation = recovered.min(maximum);
        base_gain * self.attenuation
    }

    pub(super) fn carry_state(&mut self, previous: &Self) {
        if self.ceiling == previous.ceiling && self.release_step == previous.release_step {
            self.attenuation = previous.attenuation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        AudioTreatment, EqBand, EqBandKind, PreparedTreatment, SourceInfo, MAX_EQ_BANDS, PCM_SCALE,
    };
    use super::*;
    use std::f64::consts::TAU;

    const RATE: u32 = 48_000;
    const CEILING: f64 = 0.891_250_938_133_745_6;

    #[test]
    fn seeking_past_an_overloading_trim_transient_keeps_preview_gain() {
        use super::super::{AudioTreatment, PreparedTreatment, SourceInfo, PCM_SCALE};
        let source = SourceInfo {
            sample_rate: RATE,
            channels: 1,
            frames: u64::from(RATE),
            bits_per_sample: Some(24),
        };
        let prepared = PreparedTreatment::preview(
            AudioTreatment {
                eq_output_db: 12.0,
                ..AudioTreatment::default()
            },
            source,
        )
        .unwrap();
        let mut signal = vec![(0.05 * PCM_SCALE) as i32; RATE as usize];
        let from = RATE as usize / 2;
        signal[from] = (0.9 * PCM_SCALE) as i32;
        let seek = from + RATE as usize / 50;
        let end = seek + 1024;
        let start = prepared.preview_start(seek as u64, RATE);
        let mut continuous = Vec::new();
        prepared
            .processor(source)
            .unwrap()
            .process(&signal[..end], &mut continuous);
        let mut sought = Vec::new();
        prepared
            .processor_at(source, start)
            .unwrap()
            .process(&signal[start as usize..end], &mut sought);
        assert_eq!(&continuous[seek..end], &sought[seek - start as usize..]);
    }

    #[test]
    fn protects_both_channels_with_one_gain_and_recovers_smoothly() {
        let mut guard = PreviewCeiling::new(RATE, CEILING);
        let base = 2.0;
        assert_eq!(guard.gain(0.4, base), base);
        assert_eq!(guard.gain(CEILING / base, base), base);

        let left = 0.9;
        let right = 0.27;
        let gain = guard.gain(left, base);
        assert!((left * gain - CEILING).abs() < 1e-12);
        assert!((right * gain / (left * gain) - right / left).abs() < 1e-12);

        let first = guard.gain(0.4, base);
        assert!(first > gain && first - gain < 0.001);
        for _ in 1..RATE / 10 {
            guard.gain(0.4, base);
        }
        let expected = base - (base - gain) / std::f64::consts::E;
        assert!((guard.gain(0.4, base) - expected).abs() < 0.001);
        for _ in 0..RATE * 2 {
            guard.gain(0.0, base);
        }
        assert_eq!(guard.gain(0.4, base), base);
    }

    #[test]
    fn reduces_steady_sine_distortion_compared_to_a_sample_ceiling() {
        const FRAMES: usize = RATE as usize / 10;
        let mut guard = PreviewCeiling::new(RATE, CEILING);
        let angle = |frame| TAU * 1_000.0 * frame as f64 / f64::from(RATE);
        let boost = 10.0_f64.powf(12.0 / 20.0);
        let mut limited = Vec::with_capacity(FRAMES);
        let mut hard_clipped = Vec::with_capacity(FRAMES);
        for frame in 0..FRAMES * 2 {
            let input = 0.5 * angle(frame).sin();
            let output = input * guard.gain(input.abs(), boost);
            assert!(output.abs() <= CEILING + 1e-12);
            if frame >= FRAMES {
                limited.push(output);
                hard_clipped.push((input * boost).clamp(-CEILING, CEILING));
            }
        }
        let thd = |samples: &[f64]| {
            let power_at = |harmonic| {
                let (sin, cos) = samples
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |sum, (frame, y)| {
                        let (sin, cos) = (angle(frame) * harmonic as f64).sin_cos();
                        (sum.0 + y * sin, sum.1 + y * cos)
                    });
                sin * sin + cos * cos
            };
            ((2..24).map(power_at).sum::<f64>() / power_at(1)).sqrt()
        };
        let limited_thd = thd(&limited);
        let clipped_thd = thd(&hard_clipped);
        assert!(
            limited_thd < 0.003 && limited_thd < clipped_thd / 20.0,
            "linked ceiling THD {limited_thd}, hard ceiling THD {clipped_thd}"
        );
    }

    #[test]
    fn carries_gain_recovery_between_compatible_processors() {
        let mut previous = PreviewCeiling::new(RATE, CEILING);
        previous.gain(1.0, 2.0);
        let mut next = PreviewCeiling::new(RATE, CEILING);
        next.carry_state(&previous);
        let mut other_rate = PreviewCeiling::new(44_100, CEILING);
        other_rate.carry_state(&previous);

        assert_eq!(next.gain(0.1, 2.0), previous.gain(0.1, 2.0));
        assert_eq!(other_rate.gain(0.1, 2.0), 2.0);
    }

    #[test]
    fn comparison_attenuation_matches_an_overloaded_eq_after_protection() {
        let source = SourceInfo {
            sample_rate: RATE,
            channels: 1,
            frames: u64::from(RATE),
            bits_per_sample: Some(24),
        };
        let dry = PreparedTreatment::preview(AudioTreatment::default(), source).unwrap();
        let mut bands = [None; MAX_EQ_BANDS];
        bands[0] = Some(EqBand {
            kind: EqBandKind::Bell,
            hz: 1_000,
            q: 80,
            gain_db: 12.0,
            ..EqBand::default()
        });
        let wet = PreparedTreatment::preview(
            AudioTreatment {
                eq_bands: Some(bands),
                ..AudioTreatment::default()
            },
            source,
        )
        .unwrap();
        let input: Vec<_> = (0..RATE)
            .map(|frame| {
                let phase = TAU * 1_000.0 * f64::from(frame) / f64::from(RATE);
                (phase.sin() * 0.5 * PCM_SCALE).round() as i32
            })
            .collect();
        let rms_db = |samples: &[i32]| {
            let steady = &samples[samples.len() / 2..];
            let mean_square = steady
                .iter()
                .map(|&sample| (f64::from(sample) / PCM_SCALE).powi(2))
                .sum::<f64>()
                / steady.len() as f64;
            10.0 * mean_square.log10()
        };

        let mut output = Vec::new();
        dry.processor(source).unwrap().process(&input, &mut output);
        let dry_level = rms_db(&output);
        let readings = wet
            .processor(source)
            .unwrap()
            .process_for_playback(&input, &mut output);
        assert!(readings.overload);
        let attenuation = dry_level - rms_db(&output);
        assert!(attenuation < -4.0);

        wet.with_preview_monitor(None, Some(attenuation as f32))
            .processor(source)
            .unwrap()
            .process(&input, &mut output);
        let difference = rms_db(&output) - dry_level;
        assert!(difference.abs() < 0.1, "level difference {difference} dB");
    }
}
