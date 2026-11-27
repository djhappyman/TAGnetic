//! Detection and staged experiments for tightly scoped source repair.

use super::RepairChannel;
#[cfg(test)]
use std::ops::Range;

fn source_frames(samples: &[i32], channels: usize) -> Option<usize> {
    if (1..=2).contains(&channels) && samples.len().is_multiple_of(channels) {
        Some(samples.len() / channels)
    } else {
        None
    }
}

#[cfg(test)]
fn limit(rate: u32, millis: u32) -> usize {
    (u64::from(rate) * u64::from(millis) / 1_000) as usize
}

fn smooth(position: f64) -> f64 {
    position * position * (3.0 - 2.0 * position)
}

/// Replace a selected isolated impulse using clean boundary samples. The two
/// sample slopes preserve a sustained note over very short selections.
#[cfg(test)]
fn interpolate_click(
    samples: &mut [i32],
    channels: usize,
    channel: Option<usize>,
    edit: Range<usize>,
    rate: u32,
) -> bool {
    let Some(frames) = source_frames(samples, channels) else {
        return false;
    };
    if edit.is_empty()
        || edit.len() > limit(rate, 2)
        || edit.start < 2
        || edit.end > frames.saturating_sub(2)
        || channel.is_some_and(|index| index >= channels)
    {
        return false;
    }

    for lane in 0..channels {
        if channel.is_some_and(|channel| lane != channel) {
            continue;
        }
        let pcm = |frame: usize| samples[frame * channels + lane];
        let anchors = [
            pcm(edit.start - 2),
            pcm(edit.start - 1),
            pcm(edit.end),
            pcm(edit.end + 1),
        ];
        for (offset, frame) in edit.clone().enumerate() {
            samples[frame * channels + lane] = interpolated_sample(anchors, offset, edit.len());
        }
    }
    true
}

pub(super) fn interpolated_sample(anchors: [i32; 4], offset: usize, length: usize) -> i32 {
    let [before, left, right, after] = anchors.map(f64::from);
    let span = (length + 1) as f64;
    let outgoing = left - before;
    let incoming = after - right;
    // Guard against a boundary slope spanning an implausibly large peak.
    let headroom = (right - left).abs().max(outgoing.abs()).max(incoming.abs());
    let min = left.min(right) - headroom;
    let max = left.max(right) + headroom;
    let t = (offset + 1) as f64 / span;
    let cross = smooth(t);
    let estimate = (1.0 - cross) * left
        + cross * right
        + t * (1.0 - t) * ((1.0 - t) * outgoing - t * incoming) * span;
    estimate.clamp(min, max).round() as i32
}

/// Detect isolated single-sample impulses using nearby slope as a noise
/// reference. This intentionally abstains on groups of impulses and convincing
/// percussive attacks. A caller must audition each suggestion before applying.
fn isolated_clicks(samples: &[i32], channel: usize, channels: usize) -> Vec<ClickSuggestion> {
    let Some(frames) = source_frames(samples, channels) else {
        return Vec::new();
    };
    if channel >= channels {
        return Vec::new();
    }
    let pcm = |frame: usize| f64::from(samples[frame * channels + channel]);
    let slope = |frame: usize| pcm(frame) - pcm(frame - 1);
    let minimum_spike = f64::from(i32::MAX) * 0.015;
    let mut found = Vec::new();
    for frame in 17..frames.saturating_sub(17) {
        let incoming = slope(frame);
        let outgoing = slope(frame + 1);
        if incoming.abs().min(outgoing.abs()) < minimum_spike
            || incoming.signum() == outgoing.signum()
            || (incoming + outgoing).abs() > incoming.abs().min(outgoing.abs()) * 0.3
        {
            continue;
        }
        let mut neighbors = [0.0_f64; 24];
        for (i, reference) in (frame - 15..frame - 3)
            .chain(frame + 4..frame + 16)
            .enumerate()
        {
            neighbors[i] = slope(reference).abs();
        }
        neighbors.sort_unstable_by(f64::total_cmp);
        let local = neighbors[18];
        let peak = incoming.abs().min(outgoing.abs());
        if [frame - 2, frame - 1, frame + 2, frame + 3]
            .iter()
            .any(|&near| slope(near).abs() >= peak * 0.25)
        {
            continue;
        }
        let sensitivity = (1..=5).find(|&level| {
            let level = f64::from(level);
            peak > local * (16.0 - 2.0 * level)
                && peak >= f64::from(i32::MAX) * (0.04 - 0.005 * level)
        });
        if let Some(sensitivity) = sensitivity {
            found.push(ClickSuggestion {
                frame: frame as u64,
                sensitivity,
                channel: None,
            });
            if found.len() == 256 {
                break;
            }
        }
    }
    found
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ClickSuggestion {
    pub frame: u64,
    /// The lowest slider setting at which the PCM stands out.
    pub sensitivity: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<RepairChannel>,
}

/// Suggest source positions for review without changing the recipe or PCM.
pub(super) fn suggest_clicks(
    samples: &[i32],
    channels: usize,
    channel: Option<usize>,
    first_frame: u64,
) -> Vec<ClickSuggestion> {
    let lanes = channel.map_or(0..channels, |index| index..index.saturating_add(1));
    let mut frames: Vec<_> = lanes
        .flat_map(|lane| {
            let channel = (channels == 2).then_some(if lane == 0 {
                RepairChannel::Left
            } else {
                RepairChannel::Right
            });
            isolated_clicks(samples, lane, channels)
                .into_iter()
                .filter_map(move |click| {
                    Some(ClickSuggestion {
                        frame: first_frame.checked_add(click.frame)?,
                        channel,
                        ..click
                    })
                })
        })
        .collect();
    frames.sort_by_key(|click| click.frame);
    frames.truncate(256);
    frames
}

/// Use one common donor position for all edited channels. Target boundary
/// anchors blend with the donor entirely inside the selected gap.
#[cfg(test)]
fn replace_from_donor(
    samples: &mut [i32],
    channels: usize,
    channel: Option<usize>,
    edit: Range<usize>,
    donor_start: usize,
    rate: u32,
) -> bool {
    let Some(frames) = source_frames(samples, channels) else {
        return false;
    };
    let Some(donor_end) = donor_start.checked_add(edit.len()) else {
        return false;
    };
    if edit.is_empty()
        || edit.len() > limit(rate, 80)
        || edit.start == 0
        || edit.end >= frames
        || donor_end > frames
        || !(donor_end <= edit.start || donor_start >= edit.end)
        || channel.is_some_and(|index| index >= channels)
    {
        return false;
    }
    let blend = limit(rate, 3).max(1).min(edit.len().div_ceil(4)) as f64;
    for lane in 0..channels {
        if channel.is_some_and(|channel| lane != channel) {
            continue;
        }
        let left = f64::from(samples[(edit.start - 1) * channels + lane]);
        let right = f64::from(samples[edit.end * channels + lane]);
        for (offset, frame) in edit.clone().enumerate() {
            let take = f64::from(samples[(donor_start + offset) * channels + lane]);
            let mix_in = smooth(((offset + 1) as f64 / blend).min(1.0));
            let mix_out = smooth(((edit.len() - offset) as f64 / blend).min(1.0));
            let joined = left + (take - left) * mix_in;
            samples[frame * channels + lane] = (right + (joined - right) * mix_out).round() as i32;
        }
    }
    true
}

/// Rank nearby repeats by both clean shoulders. Reject weak matches and
/// silence. Search only a short preroll and keep every stereo frame aligned.
#[cfg(test)]
fn find_gap_donor(
    samples: &[i32],
    channels: usize,
    edit: Range<usize>,
    rate: u32,
) -> Option<usize> {
    let frames = source_frames(samples, channels)?;
    let context = limit(rate, 4);
    if context == 0
        || edit.is_empty()
        || edit.len() > limit(rate, 20)
        || edit.start < context
        || edit.end.checked_add(context)? > frames
    {
        return None;
    }
    let compare = |candidate: usize| {
        let mut energy = 0.0;
        let mut error = 0.0;
        for (target, donor) in (edit.start - context..edit.start)
            .zip(candidate - context..candidate)
            .chain(
                (edit.end..edit.end + context)
                    .zip(candidate + edit.len()..candidate + edit.len() + context),
            )
        {
            for lane in 0..channels {
                let a = f64::from(samples[target * channels + lane]);
                let b = f64::from(samples[donor * channels + lane]);
                energy += a * a + b * b;
                error += (a - b).powi(2);
            }
        }
        if energy > f64::from(i32::MAX).powi(2) * 1e-8 * context as f64 {
            error / energy
        } else {
            f64::INFINITY
        }
    };
    let last = edit.start.checked_sub(edit.len() + context * 2)?;
    let first = context.max(edit.start.saturating_sub(limit(rate, 160)));
    // The search is bounded independently of the source recording length.
    let (donor, mismatch) = (first..=last)
        .map(|candidate| (candidate, compare(candidate)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    (mismatch < 0.025).then_some(donor)
}

/// Locate high digital plateaus for reconstruction experiments. This detector
/// is deliberately specific to flat, integer peaks.
#[cfg(test)]
fn hard_clip_plateaus(samples: &[i32]) -> Vec<Range<usize>> {
    let mut output = Vec::new();
    let threshold = (i32::MAX as u32 / 4) * 3;
    let mut start = 1;
    while start + 2 < samples.len() {
        let peak = samples[start];
        if peak.unsigned_abs() < threshold
            || samples[start - 1].unsigned_abs() >= peak.unsigned_abs()
        {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while samples.get(end) == Some(&peak) {
            end += 1;
        }
        if end - start >= 3
            && samples
                .get(end)
                .is_some_and(|sample| sample.unsigned_abs() < peak.unsigned_abs())
        {
            output.push(start..end);
        }
        start = end;
    }
    output
}

/// Bounded reduction toward a learned floor, for an isolated future
/// reconstruction test. The cap and power ratio protect strong components.
#[cfg(test)]
fn floor_gain(signal_power: f64, noise_power: f64, maximum_db: f64) -> f64 {
    if !signal_power.is_finite()
        || signal_power <= 0.0
        || !noise_power.is_finite()
        || noise_power <= 0.0
        || !maximum_db.is_finite()
    {
        return 1.0;
    }
    let minimum = 10.0_f64.powf(-maximum_db.clamp(0.0, 12.0) / 20.0);
    (1.0 - noise_power / signal_power)
        .max(minimum * minimum)
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    const RATE: u32 = 48_000;

    fn stereo(frames: usize) -> Vec<i32> {
        (0..frames)
            .flat_map(|frame| {
                let phase = frame as f64 * TAU / 120.0;
                [
                    (phase.sin() * 100_000_000.0) as i32,
                    ((phase + 0.8).sin() * 60_000_000.0) as i32,
                ]
            })
            .collect()
    }

    #[test]
    fn interpolation_removes_a_tick_and_preserves_every_other_sample() {
        let clean = stereo(1_000);
        let mut input = clean.clone();
        let event = 405..410;
        input[event.start * 2] = i32::MAX;
        input[(event.start + 2) * 2] = i32::MIN;
        assert!(interpolate_click(
            &mut input,
            2,
            Some(0),
            event.clone(),
            RATE
        ));
        for (index, (&after, &source)) in input.iter().zip(&clean).enumerate() {
            if index % 2 == 0 && event.contains(&(index / 2)) {
                assert!((i64::from(after) - i64::from(source)).abs() < 500_000);
            } else {
                assert_eq!(after, source);
            }
        }
    }

    #[test]
    fn an_overlong_or_boundary_selection_is_rejected_before_touching_a_transient() {
        let clean = stereo(2_000);
        let mut input = clean.clone();
        assert!(!interpolate_click(&mut input, 2, None, 200..400, RATE));
        assert!(!interpolate_click(&mut input, 2, None, 0..3, RATE));
        assert_eq!(input, clean);
    }

    #[test]
    fn click_suggestions_keep_stereo_channels_and_percussion_separate() {
        let mut input = stereo(7_000);
        for offset in 0..160 {
            let envelope = 1.0 - offset as f64 / 160.0;
            let onset = (offset as f64 * TAU / 19.0).sin() * envelope;
            input[(3_000 + offset) * 2] += (onset * 400_000_000.0) as i32;
        }
        input[1_000 * 2] += 900_000_000;
        input[6_000 * 2 + 1] -= 600_000_000;
        let positions = |clicks: Vec<ClickSuggestion>| {
            clicks
                .into_iter()
                .map(|click| click.frame)
                .collect::<Vec<_>>()
        };
        assert_eq!(positions(isolated_clicks(&input, 0, 2)), vec![1_000]);
        assert_eq!(positions(isolated_clicks(&input, 1, 2)), vec![6_000]);
        assert_eq!(
            positions(suggest_clicks(&input, 2, None, 50_000)),
            vec![51_000, 56_000]
        );
        assert!(interpolate_click(
            &mut input,
            2,
            Some(0),
            1_000..1_001,
            RATE
        ));
        assert!(isolated_clicks(&input, 0, 2).is_empty());
    }

    #[test]
    fn sensitivity_ranks_clicks_per_source_channel() {
        let mut input = vec![0; 1_000 * 2];
        input[150 * 2] = 40_000_000;
        input[150 * 2 + 1] = 100_000_000;
        input[300 * 2] = 40_000_000;
        assert_eq!(
            suggest_clicks(&input, 2, None, 0),
            [
                ClickSuggestion {
                    frame: 150,
                    sensitivity: 5,
                    channel: Some(RepairChannel::Left)
                },
                ClickSuggestion {
                    frame: 150,
                    sensitivity: 1,
                    channel: Some(RepairChannel::Right)
                },
                ClickSuggestion {
                    frame: 300,
                    sensitivity: 5,
                    channel: Some(RepairChannel::Left)
                },
            ]
        );
    }

    #[test]
    fn matching_uses_both_shoulders_and_a_common_stereo_donor() {
        let clean = stereo(7_000);
        let mut input = clean.clone();
        let gap = 5_401..5_713;
        input[gap.start * 2..gap.end * 2].fill(0);
        let donor = find_gap_donor(&input, 2, gap.clone(), RATE).expect("periodic stereo material");
        assert_eq!(donor % 120, gap.start % 120);
        assert!(replace_from_donor(
            &mut input,
            2,
            None,
            gap.clone(),
            donor,
            RATE
        ));
        let mid = gap.start + gap.len() / 2;
        assert_eq!(input[mid * 2..mid * 2 + 2], clean[mid * 2..mid * 2 + 2]);
        assert_eq!(input[..gap.start * 2], clean[..gap.start * 2]);
        assert_eq!(input[gap.end * 2..], clean[gap.end * 2..]);
        assert!(find_gap_donor(&vec![0; 14_000], 2, gap, RATE).is_none());
    }

    #[test]
    fn an_explicit_donor_leaves_the_other_channel_and_donor_unchanged() {
        let clean = stereo(7_000);
        let mut input = clean.clone();
        let gap = 3_600..4_200;
        assert!(replace_from_donor(
            &mut input,
            2,
            Some(1),
            gap.clone(),
            1_000,
            RATE
        ));
        assert!(!replace_from_donor(
            &mut input,
            2,
            None,
            gap.clone(),
            gap.start + 1,
            RATE
        ));
        for (index, sample) in input.iter().enumerate() {
            if index % 2 == 0 || !gap.contains(&(index / 2)) {
                assert_eq!(*sample, clean[index]);
            }
        }
        assert_eq!(input[3_900 * 2 + 1], clean[1_300 * 2 + 1]);
    }

    #[test]
    fn clipping_detection_separates_digital_plateaus_from_a_clean_crest() {
        let dry: Vec<_> = (0..240)
            .map(|frame| (2_000_000_000.0 * (frame as f64 * TAU / 120.0).sin()).round() as i32)
            .collect();
        let clip = dry
            .iter()
            .map(|sample| (*sample).clamp(-1_850_000_000, 1_850_000_000))
            .collect::<Vec<_>>();
        let plateaus = hard_clip_plateaus(&clip);
        assert_eq!(plateaus.len(), 4);
        assert!(plateaus.iter().all(|range| range.len() >= 3));
        assert!(hard_clip_plateaus(&dry).is_empty());
    }

    #[test]
    fn learned_floor_model_limits_reduction_of_nearby_strong_components() {
        let noise_only = floor_gain(2.0, 2.0, 9.0);
        assert!((20.0 * noise_only.log10() + 9.0).abs() < 1e-6);
        assert!(floor_gain(200.0, 2.0, 9.0) > 0.99);
        assert_eq!(floor_gain(0.0, 2.0, 9.0), 1.0);
    }
}
