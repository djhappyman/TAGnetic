//! Audio EQ Cookbook filters in transposed direct form II.
//! https://www.w3.org/TR/audio-eq-cookbook/

use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};

#[derive(Clone)]
pub(in crate::codec) struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    delay: [f64; 2],
}

impl Biquad {
    pub(super) fn glide_toward(&mut self, target: &Self, weight: f64) {
        if weight == 1.0 {
            self.a = target.a;
            self.b = target.b;
            return;
        }
        // The stable region for normalized real second-order denominators
        // is convex. Keep filter history as the coefficients move within it.
        for (current, target) in self
            .a
            .iter_mut()
            .zip(target.a)
            .chain(self.b.iter_mut().zip(target.b))
        {
            *current += (target - *current) * weight;
        }
    }

    pub(super) fn carry_delay(&mut self, previous: &Self) -> bool {
        if self.a != previous.a || self.b != previous.b {
            return false;
        }
        self.delay = previous.delay;
        true
    }

    pub(super) fn high_pass(rate: f64, frequency: u16) -> Self {
        let (sin, cos) = angle(rate, frequency);
        Self::pass(cos, sin * FRAC_1_SQRT_2, true)
    }

    pub(super) fn low_pass(rate: f64, frequency: u16) -> Self {
        let (sin, cos) = angle(rate, frequency);
        Self::pass(cos, sin * FRAC_1_SQRT_2, false)
    }

    pub(super) fn cut(
        rate: f64,
        frequency: u16,
        slope: u8,
        high_pass: bool,
    ) -> impl Iterator<Item = Self> {
        let (sin, cos) = angle(rate, frequency);
        let sections = slope.max(12) / 12;
        (0..sections).map(move |index| {
            // Butterworth sections use 1/(2Q) = cos((2k+1)π/(2*order)).
            // Run the lower-Q sections first to limit intermediate peaks.
            let damping = if sections == 1 {
                FRAC_1_SQRT_2
            } else {
                (PI * f64::from(2 * index + 1) / f64::from(4 * sections)).cos()
            };
            Self::pass(cos, sin * damping, high_pass)
        })
    }

    fn pass(cos: f64, alpha: f64, high_pass: bool) -> Self {
        let b = if high_pass {
            let edge = (1.0 + cos) * 0.5;
            [edge, -2.0 * edge, edge]
        } else {
            let edge = (1.0 - cos) * 0.5;
            [edge, 2.0 * edge, edge]
        };
        Self::symmetric(b, cos, alpha)
    }

    pub(super) fn notch(rate: f64, frequency: u16, quality: impl Into<f64>) -> Self {
        let (sin, cos) = angle(rate, frequency);
        Self::symmetric([1.0, -2.0 * cos, 1.0], cos, sin / (2.0 * quality.into()))
    }

    pub(in crate::codec) fn peak(rate: f64, frequency: u16, db: f32, quality: f64) -> Self {
        let (sin, cos) = angle(rate, frequency);
        let amplitude = 10.0_f64.powf(f64::from(db) / 40.0);
        let alpha = sin / (2.0 * quality);
        Self::new(
            [1.0 + alpha * amplitude, -2.0 * cos, 1.0 - alpha * amplitude],
            [1.0 + alpha / amplitude, -2.0 * cos, 1.0 - alpha / amplitude],
        )
    }

    pub(super) fn band_pass(rate: f64, low_hz: u16, high_hz: u16) -> Self {
        // Prewarp both edges. This also keeps narrow audition bands centred
        // when they are close to the top of the editable frequency range.
        let edge = |hz| (PI * f64::from(hz) / rate).tan();
        let (low, high) = (edge(low_hz), edge(high_hz));
        let centre = (low * high).sqrt();
        let width = high - low;
        let centre_sq = centre * centre;
        Self::new(
            [width, 0.0, -width],
            [
                1.0 + width + centre_sq,
                2.0 * (centre_sq - 1.0),
                1.0 - width + centre_sq,
            ],
        )
    }

    pub(super) fn band_detector(rate: f64, frequency: u16, quality: f64) -> Self {
        let (sin, cos) = angle(rate, frequency);
        let alpha = sin / (2.0 * quality);
        Self::symmetric([alpha, 0.0, -alpha], cos, alpha)
    }

    pub(super) fn shelf(rate: f64, frequency: u16, db: f32, high: bool) -> Self {
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

    pub(super) fn new(b: [f64; 3], [a0, a1, a2]: [f64; 3]) -> Self {
        Self {
            b: b.map(|value| value / a0),
            a: [a1 / a0, a2 / a0],
            delay: [0.0; 2],
        }
    }

    pub(in crate::codec) fn response_db(&self, rate: f64, hz: f64) -> f64 {
        let (sin, cos) = (TAU * hz / rate).sin_cos();
        let power = |[b0, b1, b2]: [f64; 3]| {
            let real = b0 + b1 * cos + b2 * (2.0 * cos * cos - 1.0);
            let imag = b1 * sin + b2 * (2.0 * cos * sin);
            real * real + imag * imag
        };
        10.0 * (power(self.b) / power([1.0, self.a[0], self.a[1]]))
            .max(1e-24)
            .log10()
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
