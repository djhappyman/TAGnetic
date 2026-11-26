//! Audio EQ Cookbook filters in transposed direct form II.
//! https://www.w3.org/TR/audio-eq-cookbook/

use std::f64::consts::{FRAC_1_SQRT_2, TAU};

#[derive(Clone)]
pub(super) struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    delay: [f64; 2],
}

impl Biquad {
    pub(super) fn high_pass(rate: f64, frequency: u16) -> Self {
        let (sin, cos) = angle(rate, frequency);
        let b = (1.0 + cos) * 0.5;
        Self::symmetric([b, -2.0 * b, b], cos, sin * FRAC_1_SQRT_2)
    }

    pub(super) fn low_pass(rate: f64, frequency: u16) -> Self {
        let (sin, cos) = angle(rate, frequency);
        let b = (1.0 - cos) * 0.5;
        Self::symmetric([b, 2.0 * b, b], cos, sin * FRAC_1_SQRT_2)
    }

    pub(super) fn notch(rate: f64, frequency: u16, quality: u8) -> Self {
        let (sin, cos) = angle(rate, frequency);
        Self::symmetric(
            [1.0, -2.0 * cos, 1.0],
            cos,
            sin / (2.0 * f64::from(quality)),
        )
    }

    pub(super) fn peak(rate: f64, frequency: u16, db: i8) -> Self {
        let (sin, cos) = angle(rate, frequency);
        let amplitude = 10.0_f64.powf(f64::from(db) / 40.0);
        let alpha = sin / (2.0 * 0.8);
        Self::new(
            [1.0 + alpha * amplitude, -2.0 * cos, 1.0 - alpha * amplitude],
            [1.0 + alpha / amplitude, -2.0 * cos, 1.0 - alpha / amplitude],
        )
    }

    pub(super) fn shelf(rate: f64, frequency: u16, db: i8, high: bool) -> Self {
        let (sin, cos) = angle(rate, frequency);
        let amplitude = 10.0_f64.powf(f64::from(db) / 40.0);
        let slope = 2.0 * amplitude.sqrt() * sin * FRAC_1_SQRT_2;
        let plus = amplitude + 1.0;
        let minus = amplitude - 1.0;
        if high {
            Self::new(
                [
                    amplitude * (plus + minus * cos + slope),
                    -2.0 * amplitude * (minus + plus * cos),
                    amplitude * (plus + minus * cos - slope),
                ],
                [
                    plus - minus * cos + slope,
                    2.0 * (minus - plus * cos),
                    plus - minus * cos - slope,
                ],
            )
        } else {
            Self::new(
                [
                    amplitude * (plus - minus * cos + slope),
                    2.0 * amplitude * (minus - plus * cos),
                    amplitude * (plus - minus * cos - slope),
                ],
                [
                    plus + minus * cos + slope,
                    -2.0 * (minus + plus * cos),
                    plus + minus * cos - slope,
                ],
            )
        }
    }

    fn symmetric(b: [f64; 3], cos: f64, alpha: f64) -> Self {
        Self::new(b, [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
    }

    fn new(b: [f64; 3], [a0, a1, a2]: [f64; 3]) -> Self {
        Self {
            b: b.map(|value| value / a0),
            a: [a1 / a0, a2 / a0],
            delay: [0.0; 2],
        }
    }

    pub(super) fn filter(&mut self, input: f64) -> f64 {
        let output = self.b[0] * input + self.delay[0];
        self.delay[0] = self.b[1] * input - self.a[0] * output + self.delay[1];
        self.delay[1] = self.b[2] * input - self.a[1] * output;
        // In long runs of silence, stop decaying below any PCM precision to
        // keep denormal values out of the processing loop.
        for delay in &mut self.delay {
            if delay.abs() < 1e-24 {
                *delay = 0.0;
            }
        }
        output
    }
}

fn angle(rate: f64, frequency: u16) -> (f64, f64) {
    (TAU * f64::from(frequency) / rate).sin_cos()
}
