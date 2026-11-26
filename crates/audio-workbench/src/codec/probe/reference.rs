//! Estimate a bounded static EQ from short excerpts of two recordings.
use std::{
    fs::File,
    time::{Duration, Instant},
};

use super::{
    check_cancelled, FrameRange, MediaError, PreviewDecoder, SeekIndex, Spectrum, FLOOR_DB,
};
use crate::{codec::treatment::biquad::Biquad, EqBand, EqBandKind};

const SECONDS: u64 = 8;
const CENTRES: [u16; 8] = [80, 160, 315, 630, 1_250, 2_500, 5_000, 10_000];

pub fn match_reference(
    source: File,
    range: FrameRange,
    index: Option<&SeekIndex>,
    reference: File,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Vec<EqBand>, MediaError> {
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut stop = || cancelled() || Instant::now() >= deadline;
    let mut input = PreviewDecoder::open_indexed(source, range.start, index, &mut stop)?;
    let mut target = PreviewDecoder::open(reference, 0, &mut stop)?;
    let rate = input.source_info().sample_rate;
    let limit = f64::from(rate.min(target.source_info().sample_rate)) * 0.45;
    let a = average(&mut input, range.end.saturating_sub(range.start), &mut stop)?;
    let b = average(&mut target, u64::MAX, &mut stop)?;
    let frequencies: Vec<_> = CENTRES
        .into_iter()
        .filter(|hz| f64::from(*hz) <= limit)
        .collect();
    let levels_a = centre_levels(&a, &frequencies);
    let levels_b = centre_levels(&b, &frequencies);
    fit(&frequencies, &levels_a, &levels_b, rate)
}

struct Average {
    frequencies: Vec<f32>,
    powers: Vec<f64>,
}

fn average(
    decoder: &mut PreviewDecoder,
    frames: u64,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Average, MediaError> {
    let info = decoder.source_info();
    let channels = usize::from(info.channels);
    if channels == 0 || info.sample_rate == 0 {
        return Err(MediaError::Unsupported("The reference has no usable audio"));
    }
    let mut remaining = frames.min(u64::from(info.sample_rate) * SECONDS) as usize * channels;
    let size = (info.sample_rate / 4)
        .max(1)
        .next_power_of_two()
        .clamp(4096, 16_384) as usize;
    let mut spectrum = Spectrum::new(size, info.sample_rate);
    let mut powers = vec![0.0; spectrum.frequencies.len()];
    let mut block = Vec::with_capacity(size * channels);
    let mut collect = |block: &[i32], spectrum: &mut Spectrum, stop: &mut _| {
        if block.len() < 128 * channels {
            return Ok(());
        }
        for (sum, db) in powers
            .iter_mut()
            .zip(spectrum.measure(block, channels, stop)?)
        {
            if db > FLOOR_DB {
                *sum += 10.0_f64.powf(f64::from(db) / 10.0);
            }
        }
        Ok::<(), MediaError>(())
    };
    while remaining != 0 {
        check_cancelled(cancelled)?;
        let Some(packet) = decoder.next_block(&mut *cancelled)? else {
            break;
        };
        let available = remaining.min(packet.len());
        remaining -= available;
        let mut samples = &packet[..available];
        while !samples.is_empty() {
            let count = samples.len().min(size * channels - block.len());
            block.extend_from_slice(&samples[..count]);
            samples = &samples[count..];
            if block.len() == size * channels {
                collect(&block, &mut spectrum, cancelled)?;
                block.clear();
            }
        }
    }
    collect(&block, &mut spectrum, cancelled)?;
    Ok(Average {
        frequencies: spectrum.frequencies,
        powers,
    })
}

fn centre_levels(average: &Average, centres: &[u16]) -> Vec<f64> {
    centres
        .iter()
        .map(|&centre| {
            let centre = f32::from(centre);
            let width = 2.0_f32.sqrt();
            let power: f64 = average
                .frequencies
                .iter()
                .zip(&average.powers)
                .filter(|(hz, _)| **hz >= centre / width && **hz < centre * width)
                .map(|(_, power)| *power)
                .sum();
            10.0 * power.log10()
        })
        .collect()
}

fn fit(
    centres: &[u16],
    input: &[f64],
    target: &[f64],
    rate: u32,
) -> Result<Vec<EqBand>, MediaError> {
    let peak = |levels: &[f64]| levels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (peak_a, peak_b) = (peak(input), peak(target));
    let mut differences: Vec<Option<f64>> = input
        .iter()
        .zip(target)
        .map(|(&a, &b)| {
            (a.is_finite() && b.is_finite() && a > peak_a - 42.0 && b > peak_b - 42.0)
                .then_some(b - a)
        })
        .collect();
    let mut reliable: Vec<_> = differences.iter().flatten().copied().collect();
    if reliable.len() < 2 {
        return Err(MediaError::Range(
            "Choose excerpts with more audible, overlapping frequencies",
        ));
    }
    reliable.sort_by(f64::total_cmp);
    let midpoint = (reliable[(reliable.len() - 1) / 2] + reliable[reliable.len() / 2]) * 0.5;
    for difference in differences.iter_mut().flatten() {
        *difference = (*difference - midpoint).clamp(-8.0, 8.0);
    }
    // Fit strong differences first and subtract the neighbouring effect before
    // choosing the next band. This limits accidental stacking between centres.
    let mut bands = Vec::new();
    while let Some((index, difference)) = differences
        .iter()
        .enumerate()
        .filter_map(|(index, difference)| difference.map(|db| (index, db)))
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
    {
        if difference.abs() < 1.0 {
            break;
        }
        differences[index] = None;
        let hz = centres[index];
        let gain_db = (difference.clamp(-6.0, 6.0) * 10.0).round() as f32 / 10.0;
        let band = EqBand {
            id: Some(index as u32 + 1),
            kind: EqBandKind::Bell,
            hz,
            gain_db,
            q: 140,
            ..EqBand::default()
        };
        let filter = Biquad::peak(f64::from(rate), hz, gain_db, 1.4);
        for (&hz, residual) in centres.iter().zip(&mut differences) {
            if let Some(residual) = residual {
                *residual -= filter.response_db(f64::from(rate), f64::from(hz));
            }
        }
        bands.push(band);
    }
    bands.sort_by_key(|band| band.hz);
    Ok(bands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_ignores_uniform_loudness_and_fits_tonal_difference() {
        let input = [-40.0, -31.0, -31.0, -30.0, -21.0, -25.0, -30.0, -40.0];
        let mut target = input.map(|db| db + 9.0);
        assert!(fit(&CENTRES, &input, &target, 48000).unwrap().is_empty());
        target[1] -= 5.0;
        target[6] += 4.0;
        let bands = fit(&CENTRES, &input, &target, 48000).unwrap();
        assert!(bands
            .iter()
            .any(|band| band.hz == CENTRES[1] && band.gain_db < -3.0));
        assert!(bands
            .iter()
            .any(|band| band.hz == CENTRES[6] && band.gain_db > 2.0));
    }
}
