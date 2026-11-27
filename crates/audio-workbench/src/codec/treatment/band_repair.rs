//! Temporal band attenuation with four minimum-phase second-order sections.

use super::{deserialize_slots, Biquad, RepairMask};
use rustfft::num_complex::Complex64;
use serde::{Deserialize, Deserializer, Serialize};
use std::f64::consts::PI;
use std::ops::Range;
use std::sync::Arc;

pub const MAX_SPECTRAL_REPAIRS: usize = 8;
const SECTIONS: usize = 4;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairChannel {
    Left,
    Right,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectralRepair {
    pub start_frame: u64,
    pub end_frame: u64,
    pub low_hz: u32,
    pub high_hz: u32,
    pub amount_db: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<RepairChannel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<Arc<str>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bypassed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interpolate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<Arc<RepairMask>>,
}

type RepairSlots = [Option<SpectralRepair>; MAX_SPECTRAL_REPAIRS];

pub(super) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<RepairSlots>, D::Error> {
    deserialize_slots(deserializer)
}

pub(super) struct BandRepair {
    frames: Range<u64>,
    warmup: u64,
    fade_frames: u64,
    channels: Vec<[Biquad; SECTIONS]>,
    channel: Option<usize>,
}

impl BandRepair {
    pub(super) fn new(repair: &SpectralRepair, rate: u32, channels: usize) -> Self {
        let length = repair.end_frame - repair.start_frame;
        let design = band_design(rate, repair);
        Self {
            frames: repair.start_frame..repair.end_frame,
            warmup: repair
                .start_frame
                .saturating_sub(settle_frames(rate, &design)),
            fade_frames: (u64::from(rate) * 8 / 1000).clamp(1, (length / 2).max(1)),
            channels: vec![design.map(|(pole, zero)| section(pole, zero)); channels],
            channel: repair
                .channel
                .map(|channel| usize::from(channel == RepairChannel::Right)),
        }
    }

    pub(super) fn apply(&mut self, position: u64, frame: &mut [f64]) {
        if position < self.warmup || position >= self.frames.end {
            return;
        }
        let edge = position
            .saturating_add(1)
            .saturating_sub(self.frames.start)
            .min(self.frames.end - position)
            .min(self.fade_frames);
        let ramp = edge as f64 / self.fade_frames as f64;
        let mix = ramp * ramp * (3.0 - 2.0 * ramp);
        for (channel, (dry, sections)) in frame.iter_mut().zip(&mut self.channels).enumerate() {
            if self.channel.is_some_and(|target| target != channel) {
                continue;
            }
            let wet = sections
                .iter_mut()
                .fold(*dry, |sample, section| section.filter(sample));
            *dry += mix * (wet - *dry);
        }
    }
}

pub(super) fn history_frames(rate: u32, repair: &SpectralRepair) -> u64 {
    settle_frames(rate, &band_design(rate, repair))
}

fn settle_frames(rate: u32, design: &[(Complex64, Complex64); SECTIONS]) -> u64 {
    let radius = design
        .iter()
        .map(|(pole, _)| pole.norm())
        .fold(0.0, f64::max);
    // Prime until the slowest pole decays below roughly −156 dB. Ordinary
    // midrange repairs thus need only a small lookback when seeking.
    ((-18.0 / radius.ln()).ceil() as u64).min(u64::from(rate) * 4)
}

fn band_design(rate: u32, repair: &SpectralRepair) -> [(Complex64, Complex64); SECTIONS] {
    let warp = |hz| (PI * f64::from(hz) / f64::from(rate)).tan();
    let low = warp(repair.low_hz);
    let high = warp(repair.high_hz);
    let center_squared = low * high;
    // The fourth-order shelf prototype has squared amplitude
    // (gain² + Ω^8)/(1 + Ω^8). Scale Ω so both selected edges land at
    // half the requested attenuation in dB.
    let zero_radius = 10.0_f64.powf(-f64::from(repair.amount_db) / (20.0 * SECTIONS as f64));
    let bandwidth = (high - low) / zero_radius.sqrt();
    let pairs = [7.0 * PI / 8.0, 5.0 * PI / 8.0].map(|angle| {
        let prototype = Complex64::from_polar(1.0, angle);
        let poles = band_roots(prototype, bandwidth, center_squared).map(to_digital);
        let zeros = band_roots(prototype * zero_radius, bandwidth, center_squared).map(to_digital);
        [(poles[0], zeros[0]), (poles[1], zeros[1])]
    });
    let [[first, second], [third, fourth]] = pairs;
    [first, second, third, fourth]
}

fn band_roots(root: Complex64, bandwidth: f64, center_squared: f64) -> [Complex64; 2] {
    // Wideband substitution s -> (s² + center²) / (bandwidth * s).
    let half = root * (bandwidth / 2.0);
    let discriminant = (half * half - center_squared).sqrt();
    let plus = half + discriminant;
    let minus = half - discriminant;
    let outer = if plus.norm_sqr() > minus.norm_sqr() {
        plus
    } else {
        minus
    };
    // The root product gives the other root without subtractive cancellation
    // when a wide selection has very different lower and upper edges.
    [center_squared / outer, outer]
}

fn to_digital(root: Complex64) -> Complex64 {
    (1.0 + root) / (1.0 - root)
}

fn section(pole: Complex64, zero: Complex64) -> Biquad {
    let gain = (1.0 - pole).norm_sqr() / (1.0 - zero).norm_sqr();
    Biquad::new(
        [gain, -2.0 * zero.re * gain, zero.norm_sqr() * gain],
        [1.0, -2.0 * pole.re, pole.norm_sqr()],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::value::{Error, SeqDeserializer, UnitDeserializer};

    #[test]
    fn spectral_repair_list_accepts_short_sequences_and_rejects_excess_entries() {
        let list =
            |len| SeqDeserializer::new(std::iter::repeat_n(UnitDeserializer::<Error>::new(), len));
        assert_eq!(
            deserialize(list(1)).unwrap(),
            Some([const { None }; MAX_SPECTRAL_REPAIRS])
        );
        assert!(deserialize(list(MAX_SPECTRAL_REPAIRS + 1)).is_err());
    }
}
