//! Overlapping FFT detectors drive a causal, minimum-phase correction filter.
//! The input past the next hop is never needed, so this adds no transport delay.
use super::{EqBand, EqBandKind, EqChannel};
use crate::codec::probe::SpectralEqResponse;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::{cell::RefCell, f32::consts::TAU, ops::Range, sync::Arc};

thread_local! {
    static PLANNER: RefCell<FftPlanner<f32>> = RefCell::new(FftPlanner::new());
}

const SILENCE: f32 = 0.000_015_849;
const FLOOR_DB: f32 = -96.0;
const LOG_AMPLITUDE_PER_DB: f32 = std::f32::consts::LN_10 / 20.0;

pub(super) struct SpectralEq {
    bands: Vec<Band>,
    channels: Vec<Channel>,
    size: usize,
    hop: usize,
    phase: usize,
    cursor: usize,
    window: Vec<f32>,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    transform: Vec<Complex32>,
    audio_spectrum: Vec<Complex32>,
    levels: Vec<f32>,
    peaks: Vec<f32>,
    gain: Vec<f32>,
    previous: Correction,
    current: Correction,
    attack: f32,
    release: f32,
}

struct Channel {
    // Mirroring the rings makes the convolution head contiguous, including wraps.
    dry: Vec<f32>,
    wet: Vec<f32>,
    previous_tail: Vec<f32>,
    current_tail: Vec<f32>,
}

struct Correction {
    head: Vec<f32>,
    tail: Vec<Complex32>,
    active: bool,
}

struct Band {
    index: usize,
    bins: Range<usize>,
    weights: Vec<f32>,
    threshold: f32,
    amount: f32,
    exponent: f32,
    mix: f32,
    detector: f32,
}

impl SpectralEq {
    pub(super) fn new(
        bands: &[(usize, EqBand)],
        rate: u32,
        channels: usize,
        first_frame: u64,
    ) -> Option<Self> {
        if bands.is_empty() {
            return None;
        }
        // 2048 at 44.1/48 kHz, 4096 at 88.2/96 kHz, bounded at unusual rates.
        let size = (rate as usize / 32).next_power_of_two().clamp(256, 4096);
        let hop = size / 8;
        let (forward, inverse) = PLANNER.with_borrow_mut(|planner| {
            (
                planner.plan_fft_forward(size),
                planner.plan_fft_inverse(size),
            )
        });
        let scratch_len = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        let spectrum = || vec![Complex32::default(); size];
        let correction = || Correction {
            head: vec![0.0; hop],
            tail: spectrum(),
            active: false,
        };
        let speed = |seconds: f32| 1.0 - (-(hop as f32) / (rate as f32 * seconds)).exp();
        // A Hann window sums to half its size, with half the tone's amplitude in
        // each conjugate bin. Store the normalisation in the coefficients.
        let window = (0..size)
            .map(|i| (1.0 - (TAU * i as f32 / size as f32).cos()) * (2.0 / size as f32))
            .collect();
        Some(Self {
            bands: bands
                .iter()
                .map(|&(index, band)| Band::new(index, band, rate, size))
                .collect(),
            channels: (0..channels)
                .map(|_| Channel {
                    dry: vec![0.0; size * 2],
                    wet: vec![0.0; size * 2],
                    previous_tail: vec![0.0; hop],
                    current_tail: vec![0.0; hop],
                })
                .collect(),
            size,
            hop,
            phase: (first_frame % hop as u64) as usize,
            cursor: 0,
            window,
            forward,
            inverse,
            scratch: vec![Complex32::default(); scratch_len],
            transform: spectrum(),
            audio_spectrum: spectrum(),
            levels: vec![0.0; size / 2 + 1],
            peaks: vec![0.0; size / 2 + 1],
            gain: vec![0.0; size / 2 + 1],
            previous: correction(),
            current: correction(),
            attack: speed(0.012),
            release: speed(0.15),
        })
    }

    pub(super) fn readings(&self) -> impl Iterator<Item = (usize, f32, f32)> + '_ {
        self.bands
            .iter()
            .map(|band| (band.index, band.mix, band.detector))
    }

    pub(super) fn response(&self, channel: EqChannel, rate: u32) -> Option<SpectralEqResponse> {
        const DISPLAY_FLOOR_DB: f32 = 0.02;
        if !self.current.active {
            return None;
        }
        let audible = |gain: &f32| gain.abs() >= DISPLAY_FLOOR_DB;
        let first_bin = self.gain.iter().position(audible)?.saturating_sub(1);
        let last_bin = (self.gain.iter().rposition(audible)? + 1).min(self.size / 2);
        Some(SpectralEqResponse {
            channel,
            bin_hz: rate as f32 / self.size as f32,
            first_bin,
            gains_db: (first_bin..=last_bin)
                .map(|bin| Self::smoothed_gain(&self.gain, bin))
                .collect(),
        })
    }

    fn smoothed_gain(gains: &[f32], bin: usize) -> f32 {
        let half = gains.len() - 1;
        let lower = if bin == 0 { 1 } else { bin - 1 };
        let upper = if bin == half { half - 1 } else { bin + 1 };
        let gain = gains[bin];
        ((gains[lower] + 2.0 * gain + gains[upper]) * 0.25).clamp(gain.min(0.0), gain.max(0.0))
    }

    pub(super) fn carry_detector(&mut self, previous: &Self) {
        if self.levels.len() == previous.levels.len()
            && self.channels.len() == previous.channels.len()
        {
            self.levels.copy_from_slice(&previous.levels);
        }
    }

    pub(super) fn retune_from(&mut self, target: &mut Self) {
        // Compatible edits keep the analysis rings, linked envelopes, and the
        // running convolution. The next hop fades to the revised correction.
        self.bands = std::mem::take(&mut target.bands);
    }

    pub(super) fn apply(&mut self, frame: &mut [f64], sidechain: &[f64]) {
        if self.phase == 0 {
            self.update_filter();
        }
        let overlap = (self.phase + 1) as f32 / self.hop as f32;
        let end = self.size + self.cursor + 1;
        for ((output, &detector), channel) in
            frame.iter_mut().zip(sidechain).zip(&mut self.channels)
        {
            let input = *output as f32;
            channel.wet[self.cursor] = input;
            channel.wet[self.size + self.cursor] = input;
            channel.dry[self.cursor] = detector as f32;
            channel.dry[self.size + self.cursor] = detector as f32;
            if self.current.active || self.previous.active {
                // The FFT already supplied the contribution of all taps beyond
                // the hop. Only this short head can depend on current input.
                let history = &channel.wet[end - self.hop..end];
                let old = self
                    .previous
                    .sample(history, channel.previous_tail[self.phase]);
                let new = self
                    .current
                    .sample(history, channel.current_tail[self.phase]);
                *output += f64::from(old + (new - old) * overlap);
            }
        }
        self.phase = (self.phase + 1) % self.hop;
        self.cursor = (self.cursor + 1) % self.size;
    }

    fn update_filter(&mut self) {
        self.peaks.fill(0.0);
        for channel in &self.channels {
            let history = &channel.dry[self.cursor..self.cursor + self.size];
            for ((point, &sample), &window) in
                self.transform.iter_mut().zip(history).zip(&self.window)
            {
                *point = Complex32::new(sample * window, 0.0);
            }
            self.forward
                .process_with_scratch(&mut self.transform, &mut self.scratch);
            for (peak, point) in self.peaks.iter_mut().zip(&self.transform) {
                *peak = peak.max(point.norm_sqr());
            }
        }
        // DC and Nyquist have no conjugate copy.
        self.peaks[0] *= 0.25;
        self.peaks[self.size / 2] *= 0.25;
        for (level, peak) in self.levels.iter_mut().zip(&mut self.peaks) {
            let amplitude = peak.sqrt();
            let speed = if amplitude > *level {
                self.attack
            } else {
                self.release
            };
            *level += (amplitude - *level) * speed;
            *peak = if *level > SILENCE {
                level.ln() / LOG_AMPLITUDE_PER_DB
            } else {
                FLOOR_DB
            };
        }

        self.gain.fill(0.0);
        for band in &mut self.bands {
            band.measure(&self.peaks, &mut self.gain);
        }
        std::mem::swap(&mut self.previous, &mut self.current);
        self.current.active = self.gain.iter().any(|gain| gain.abs() >= 0.005);
        if self.current.active {
            self.minimum_phase_correction();
        }
        if !self.current.active && !self.previous.active {
            return;
        }
        for channel in &mut self.channels {
            // Overlap-save. The last hop is zero-filled because the long
            // tail has no taps within that hop. Its output already is final.
            let valid = self.size - self.hop;
            let past = &channel.wet[self.cursor + self.hop..self.cursor + self.size];
            for (point, &sample) in self.audio_spectrum.iter_mut().zip(past) {
                *point = Complex32::new(sample, 0.0);
            }
            self.audio_spectrum[valid..].fill(Complex32::default());
            self.forward
                .process_with_scratch(&mut self.audio_spectrum, &mut self.scratch);
            for (kernel, output) in [
                (&self.previous, &mut channel.previous_tail),
                (&self.current, &mut channel.current_tail),
            ] {
                if kernel.active {
                    for ((point, audio), kernel) in self
                        .transform
                        .iter_mut()
                        .zip(&self.audio_spectrum)
                        .zip(&kernel.tail)
                    {
                        *point = audio * kernel;
                    }
                    self.inverse
                        .process_with_scratch(&mut self.transform, &mut self.scratch);
                    for (output, point) in output.iter_mut().zip(&self.transform[valid..]) {
                        *output = point.re / self.size as f32;
                    }
                }
            }
        }
    }

    fn minimum_phase_correction(&mut self) {
        // Convert the real log spectrum to its cepstrum, keep causal quefrencies
        // and double their weight. Exponentiating the resulting analytic log
        // spectrum yields the corresponding minimum-phase transfer function.
        let scale = LOG_AMPLITUDE_PER_DB / self.size as f32;
        let half = self.size / 2;
        // The detector resolves finite-width frequency neighborhoods. Smooth
        // across adjacent bins so a sub-bin bandwidth cannot create a steep
        // transition that rings when the causal impulse response is truncated.
        for (bin, point) in self.transform.iter_mut().enumerate() {
            let bin = bin.min(self.size - bin);
            *point = Complex32::new(scale * Self::smoothed_gain(&self.gain, bin), 0.0);
        }
        self.inverse
            .process_with_scratch(&mut self.transform, &mut self.scratch);
        for point in &mut self.transform[1..half] {
            *point *= 2.0;
        }
        self.transform[half + 1..].fill(Complex32::default());
        self.forward
            .process_with_scratch(&mut self.transform, &mut self.scratch);
        for point in &mut self.transform {
            *point = point.exp();
            // A delta is the dry pass-through. Convolve only its correction.
            point.re -= 1.0;
            *point /= self.size as f32;
        }
        self.inverse
            .process_with_scratch(&mut self.transform, &mut self.scratch);
        for (tap, point) in self.current.head.iter_mut().rev().zip(&self.transform) {
            *tap = point.re;
        }
        self.transform[..self.hop].fill(Complex32::default());
        // Truncate with a short taper to discard the circular tail.
        let taper = self.size / 16;
        for (i, point) in self.transform[half - taper..half].iter_mut().enumerate() {
            *point *= 1.0 - i as f32 / taper as f32;
        }
        self.transform[half..].fill(Complex32::default());
        self.current.tail.copy_from_slice(&self.transform);
        self.forward
            .process_with_scratch(&mut self.current.tail, &mut self.scratch);
    }
}

impl Correction {
    fn sample(&self, history: &[f32], tail: f32) -> f32 {
        if !self.active {
            return 0.0;
        }
        // Independent accumulators let the hot dot product vectorise without
        // requiring relaxed floating-point compilation.
        let mut sum = [0.0; 4];
        let (input, _) = history.as_chunks::<4>();
        let (coefficients, _) = self.head.as_chunks::<4>();
        for (input, coeff) in input.iter().zip(coefficients) {
            for i in 0..4 {
                sum[i] += input[i] * coeff[i];
            }
        }
        tail + sum.into_iter().sum::<f32>()
    }
}

impl Band {
    fn new(index: usize, band: EqBand, rate: u32, size: usize) -> Self {
        let dynamic = band.dynamic.expect("spectral bands have dynamic settings");
        let warp = |hz: f64| (std::f64::consts::PI * hz / f64::from(rate)).tan();
        let center = warp(f64::from(band.hz));
        let spacing = f64::from(rate) / size as f64;
        let center_bin = (f64::from(band.hz) / spacing).max(1.0);
        let quality = band.effective_quality();
        let transition = |x: f64| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        let mut weights: Vec<_> = (0..=size / 2)
            .map(|bin| {
                let frequency = warp(bin as f64 * spacing) / center;
                (match band.kind {
                    EqBandKind::Bass => transition((1.5 - frequency) / 0.8),
                    EqBandKind::Treble => transition((frequency - 0.67) / 0.83),
                    EqBandKind::Bell => {
                        let x = quality * (frequency - frequency.recip()).abs();
                        let resolved = transition(3.0 - x) / (1.0 + x * x);
                        let fractional_bin = transition(1.0 - (bin as f64 - center_bin).abs());
                        resolved.max(fractional_bin)
                    }
                    _ => 0.0,
                }) as f32
            })
            .collect();
        let first = weights.iter().position(|&weight| weight > 0.0).unwrap_or(0);
        let last = weights
            .iter()
            .rposition(|&weight| weight > 0.0)
            .map_or(0, |bin| bin + 1);
        weights.truncate(last);
        weights.drain(..first);
        Self {
            index,
            bins: first..last,
            weights,
            threshold: dynamic.threshold_db,
            amount: dynamic.amount_db,
            exponent: 1.0 - dynamic.ratio.recip(),
            mix: 0.0,
            detector: FLOOR_DB,
        }
    }

    fn measure(&mut self, levels: &[f32], gain: &mut [f32]) {
        let upward = self.amount < 0.0;
        let limit = self.amount.abs();
        self.detector = FLOOR_DB;
        self.mix = 0.0;
        for ((bin, &weight), gain) in self
            .bins
            .clone()
            .zip(&self.weights)
            .zip(&mut gain[self.bins.clone()])
        {
            let db = levels[bin];
            if db <= FLOOR_DB {
                continue;
            }
            if weight >= 0.1 {
                self.detector = self.detector.max(db);
            }
            let difference = if upward {
                // A Hann-window tone occupies several bins. Avoid lifting the
                // shoulders of a nearby component which is already loud.
                let neighbors = bin.saturating_sub(2)..(bin + 3).min(levels.len());
                let nearby_peak = levels[neighbors].iter().copied().fold(db, f32::max);
                self.threshold - nearby_peak
            } else {
                db - self.threshold
            };
            let change = (difference * self.exponent).clamp(0.0, limit) * weight;
            self.mix = self.mix.max(change / limit);
            *gain += if upward { change } else { -change };
        }
    }
}
