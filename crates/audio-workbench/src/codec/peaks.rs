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

impl Peak {
    const EMPTY: Self = Self {
        min: 1.0,
        max: -1.0,
    };

    fn merge(&mut self, other: Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    fn push(&mut self, samples: &[i32]) {
        let mut min = i32::MAX;
        let mut max = i32::MIN;
        for &sample in samples {
            min = min.min(sample);
            max = max.max(sample);
        }
        const SCALE: f32 = 1.0 / 2_147_483_648.0;
        self.merge(Self {
            min: min as f32 * SCALE,
            max: max as f32 * SCALE,
        });
    }
}

/// A bounded min/max overview. Buckets are merged if the recording outgrows
/// the cache, retaining all extrema without allocating a PCM-sized buffer.
#[derive(Clone, Debug)]
pub struct WaveformIndex {
    buckets: Vec<Peak>,
    frames_per_bucket: u64,
    frames: u64,
}

impl Default for WaveformIndex {
    fn default() -> Self {
        Self {
            buckets: Vec::new(),
            frames_per_bucket: INITIAL_CACHE_FRAMES_PER_BIN,
            frames: 0,
        }
    }
}

impl WaveformIndex {
    pub(super) fn push(&mut self, mut samples: &[i32], channels: usize) {
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
                self.buckets.push(Peak::EMPTY);
            }
            let count = (self.frames_per_bucket - offset).min((samples.len() / channels) as u64);
            let (bucket_samples, remaining) = samples.split_at(count as usize * channels);
            let bucket = (self.frames / self.frames_per_bucket) as usize;
            self.buckets[bucket].push(bucket_samples);
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
        let count = bins_in_range(range, bins)?;
        if range.end > self.frames {
            return Err(MediaError::Range(
                "The waveform range extends past the recording",
            ));
        }
        let mut peaks = Vec::with_capacity(count);
        for index in 0..count {
            let first = bin_boundary(range, index, count) / self.frames_per_bucket;
            let last = (bin_boundary(range, index + 1, count) - 1) / self.frames_per_bucket;
            let mut peak = Peak::EMPTY;
            for &bucket in &self.buckets[first as usize..=last as usize] {
                peak.merge(bucket);
            }
            peaks.push(peak);
        }
        Ok(peaks)
    }
}

pub(super) struct PeakBins {
    range: FrameRange,
    peaks: Vec<Peak>,
    index: usize,
}

impl PeakBins {
    pub(super) fn new(range: FrameRange, bins: usize) -> Result<Self, MediaError> {
        let count = bins_in_range(range, bins)?;
        Ok(Self {
            range,
            peaks: vec![Peak::EMPTY; count],
            index: 0,
        })
    }

    pub(super) fn push(&mut self, mut first: u64, mut samples: &[i32], channels: usize) {
        while !samples.is_empty() {
            let boundary = bin_boundary(self.range, self.index + 1, self.peaks.len());
            let count = (boundary - first).min((samples.len() / channels) as u64);
            let (bin_samples, remaining) = samples.split_at(count as usize * channels);
            self.peaks[self.index].push(bin_samples);
            samples = remaining;
            first += count;
            if first == boundary {
                self.index += 1;
            }
        }
    }

    pub(super) fn finish(self) -> Vec<Peak> {
        self.peaks
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
