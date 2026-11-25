use super::decode::MAX_FRAMES;
use super::{FrameRange, MediaError};
use serde::{Deserialize, Serialize};

const MAX_REQUEST_BINS: usize = 8192;
const MAX_CACHE_BINS: usize = 262_144;
const INITIAL_CACHE_FRAMES_PER_BIN: u64 = 256;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Peak {
    pub min: f32,
    pub max: f32,
}

const PCM_SCALE: f32 = 1.0 / 2_147_483_648.0;

#[derive(Clone, Copy, Debug)]
struct PeakGroup {
    min: [i32; 2],
    max: [i32; 2],
    // Keep accumulated energy and sample count so merging buckets weights
    // short or partial buckets correctly. Values are squared PCM units.
    energy: [f32; 2],
    samples_per_lane: u64,
}

impl PeakGroup {
    const EMPTY: Self = Self {
        min: [i32::MAX; 2],
        max: [i32::MIN; 2],
        energy: [0.0; 2],
        samples_per_lane: 0,
    };

    fn push(&mut self, samples: &[i32], channels: usize) {
        let mut chunk = Self::EMPTY;
        if channels == 2 {
            let frames = samples.as_chunks::<2>().0;
            let (mut left_energy, mut right_energy) = (0.0, 0.0);
            for &[left, right] in frames {
                chunk.min[0] = chunk.min[0].min(left);
                chunk.max[0] = chunk.max[0].max(left);
                chunk.min[1] = chunk.min[1].min(right);
                chunk.max[1] = chunk.max[1].max(right);
                let (left, right) = (left as f32, right as f32);
                left_energy += left * left;
                right_energy += right * right;
            }
            chunk.energy = [left_energy, right_energy];
            chunk.samples_per_lane = frames.len() as u64;
        } else {
            let mut energy = 0.0;
            for &sample in samples {
                chunk.min[0] = chunk.min[0].min(sample);
                chunk.max[0] = chunk.max[0].max(sample);
                let sample = sample as f32;
                energy += sample * sample;
            }
            chunk.energy[0] = energy;
            chunk.samples_per_lane = samples.len() as u64;
        }
        self.merge(chunk);
    }

    fn merge(&mut self, other: Self) {
        for channel in 0..2 {
            self.min[channel] = self.min[channel].min(other.min[channel]);
            self.max[channel] = self.max[channel].max(other.max[channel]);
            self.energy[channel] += other.energy[channel];
        }
        self.samples_per_lane += other.samples_per_lane;
    }

    fn combined(self) -> Peak {
        Peak {
            min: self.min[0].min(self.min[1]) as f32 * PCM_SCALE,
            max: self.max[0].max(self.max[1]) as f32 * PCM_SCALE,
        }
    }

    fn stereo(self) -> [Peak; 2] {
        std::array::from_fn(|channel| Peak {
            min: self.min[channel] as f32 * PCM_SCALE,
            max: self.max[channel] as f32 * PCM_SCALE,
        })
    }

    fn rms(self, sum: f32, channels: u64) -> f32 {
        let count = (self.samples_per_lane * channels).max(1) as f32;
        (sum / count).sqrt() * PCM_SCALE
    }
}

#[derive(Clone, Debug)]
pub struct WaveformPeaks {
    pub combined: Vec<Peak>,
    pub rms: Vec<f32>,
    /// Independent left/right extrema and RMS for each bin, in source order.
    pub stereo: Option<Vec<[Peak; 2]>>,
    pub stereo_rms: Option<Vec<[f32; 2]>>,
}

impl WaveformPeaks {
    fn from_groups(groups: impl Iterator<Item = PeakGroup>, stereo: bool) -> Self {
        let (count, _) = groups.size_hint();
        let mut combined = Vec::with_capacity(count);
        let mut rms = Vec::with_capacity(count);
        let mut channels = stereo.then(|| Vec::with_capacity(count));
        let mut stereo_rms = stereo.then(|| Vec::with_capacity(count));
        for group in groups {
            combined.push(group.combined());
            rms.push(group.rms(
                group.energy[0] + group.energy[1],
                if stereo { 2 } else { 1 },
            ));
            if let (Some(channels), Some(stereo_rms)) = (&mut channels, &mut stereo_rms) {
                channels.push(group.stereo());
                stereo_rms.push(group.energy.map(|energy| group.rms(energy, 1)));
            }
        }
        Self {
            combined,
            rms,
            stereo: channels,
            stereo_rms,
        }
    }
}

/// A bounded min/max overview. Buckets are merged if the recording outgrows
/// the cache, retaining all extrema without allocating a PCM-sized buffer.
#[derive(Clone, Debug)]
pub struct WaveformIndex {
    buckets: Vec<PeakGroup>,
    stereo: bool,
    frames_per_bucket: u64,
    frames: u64,
}

impl Default for WaveformIndex {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            stereo: false,
            frames_per_bucket: INITIAL_CACHE_FRAMES_PER_BIN,
            frames: 0,
        }
    }
}

impl WaveformIndex {
    pub(super) fn push(&mut self, mut samples: &[i32], channels: usize) {
        self.stereo = channels == 2;
        while !samples.is_empty() {
            if self.frames == self.frames_per_bucket * MAX_CACHE_BINS as u64 {
                for index in 0..MAX_CACHE_BINS / 2 {
                    let mut merged = self.buckets[index * 2];
                    merged.merge(self.buckets[index * 2 + 1]);
                    self.buckets[index] = merged;
                }
                self.buckets.truncate(MAX_CACHE_BINS / 2);
                self.frames_per_bucket *= 2;
            }

            let offset = self.frames % self.frames_per_bucket;
            if offset == 0 {
                self.buckets.push(PeakGroup::EMPTY);
            }
            let count = (self.frames_per_bucket - offset).min((samples.len() / channels) as u64);
            let (bucket_samples, remaining) = samples.split_at(count as usize * channels);
            let bucket = (self.frames / self.frames_per_bucket) as usize;
            self.buckets[bucket].push(bucket_samples, channels);
            self.frames += count;
            samples = remaining;
        }
    }

    /// The approximate number of input frames covered by each cached bucket.
    pub fn frames_per_bucket(&self) -> u64 {
        self.frames_per_bucket
    }

    /// Summarize cached buckets overlapping the range. Boundary buckets may
    /// include neighboring frames. Use [super::waveform] for exact boundaries.
    pub fn peaks(&self, range: FrameRange, bins: usize) -> Result<Vec<Peak>, MediaError> {
        Ok(self.channel_peaks(range, bins)?.combined)
    }

    pub fn channel_peaks(
        &self,
        range: FrameRange,
        bins: usize,
    ) -> Result<WaveformPeaks, MediaError> {
        let count = bins_in_range(range, bins)?;
        if range.end > self.frames {
            return Err(MediaError::Range(
                "The waveform range extends past the recording",
            ));
        }
        let groups = (0..count).map(|index| {
            let first = bin_boundary(range, index, count) / self.frames_per_bucket;
            let last = (bin_boundary(range, index + 1, count) - 1) / self.frames_per_bucket;
            let mut peak = PeakGroup::EMPTY;
            for &bucket in &self.buckets[first as usize..=last as usize] {
                peak.merge(bucket);
            }
            peak
        });
        Ok(WaveformPeaks::from_groups(groups, self.stereo))
    }
}

pub(super) struct PeakBins {
    range: FrameRange,
    peaks: Vec<PeakGroup>,
    index: usize,
}

impl PeakBins {
    pub(super) fn new(range: FrameRange, bins: usize) -> Result<Self, MediaError> {
        let count = bins_in_range(range, bins)?;
        Ok(Self {
            range,
            peaks: vec![PeakGroup::EMPTY; count],
            index: 0,
        })
    }

    pub(super) fn push(&mut self, mut first: u64, mut samples: &[i32], channels: usize) {
        while !samples.is_empty() {
            let boundary = bin_boundary(self.range, self.index + 1, self.peaks.len());
            let count = (boundary - first).min((samples.len() / channels) as u64);
            let (bin_samples, remaining) = samples.split_at(count as usize * channels);
            self.peaks[self.index].push(bin_samples, channels);
            samples = remaining;
            first += count;
            if first == boundary {
                self.index += 1;
            }
        }
    }

    pub(super) fn finish(self, channels: usize) -> WaveformPeaks {
        WaveformPeaks::from_groups(self.peaks.into_iter(), channels == 2)
    }
}

pub(super) fn validate_bin_count(bins: usize) -> Result<(), MediaError> {
    if (1..=MAX_REQUEST_BINS).contains(&bins) {
        Ok(())
    } else {
        Err(MediaError::Range("Choose from 1 to 8,192 waveform bins"))
    }
}

fn bins_in_range(range: FrameRange, bins: usize) -> Result<usize, MediaError> {
    validate_bin_count(bins)?;
    if range.start >= range.end || range.end > MAX_FRAMES {
        return Err(MediaError::Range(
            "Choose a nonempty waveform range inside the recording",
        ));
    }
    Ok((bins as u64).min(range.end - range.start) as usize)
}

fn bin_boundary(range: FrameRange, index: usize, count: usize) -> u64 {
    range.start + (range.end - range.start) * index as u64 / count as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_peaks_can_have_different_energy_in_each_channel() {
        let peak = 1 << 30;
        let mut samples = Vec::new();
        for i in 0..512 {
            let left = match i {
                0 => peak,
                1 => -peak,
                2..256 => 0,
                _ if i % 2 == 0 => peak,
                _ => -peak,
            };
            samples.extend([left, 0]);
        }
        let range = FrameRange { start: 0, end: 512 };
        let mut overview = WaveformIndex::default();
        overview.push(&samples[..100], 2);
        overview.push(&samples[100..], 2);
        let overview = overview.channel_peaks(range, 2).unwrap();
        let mut tile = PeakBins::new(range, 2).unwrap();
        tile.push(0, &samples, 2);
        let tile = tile.finish(2);

        assert_eq!(overview.combined, tile.combined);
        assert_eq!(overview.combined[0], overview.combined[1]);
        assert_eq!(overview.rms, tile.rms);
        assert_eq!(overview.stereo_rms, tile.stereo_rms);
        let stereo = overview.stereo_rms.unwrap();
        assert_eq!(stereo[0], [0.5 / 128.0_f32.sqrt(), 0.0]);
        assert_eq!(stereo[1], [0.5, 0.0]);
    }
}
