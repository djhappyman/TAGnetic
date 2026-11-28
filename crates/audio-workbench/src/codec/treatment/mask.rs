//! Source-coordinate shapes shared by the saved recipe and the repair reader.

use super::{MediaError, SourceInfo, SpectralRepair};
use rustfft::{num_complex::Complex64, Fft, FftPlanner};
use serde::{
    de::{Error, IgnoredAny, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use std::{f64::consts::PI, fmt, marker::PhantomData, ops::Range, sync::Arc};

const MAX_REGIONS: usize = 16;
const MAX_POINTS: usize = 256;
const MAX_TOTAL_POINTS: usize = 512;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairMask {
    pub version: u8,
    pub feather_frames: u32,
    pub feather_hz: u32,
    #[serde(deserialize_with = "bounded::<_, _, MAX_REGIONS>")]
    pub regions: Vec<RepairMaskRegion>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepairMaskPoint {
    pub frame: u64,
    pub hz: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskOperation {
    Add,
    Subtract,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum RepairMaskRegion {
    Rectangle {
        operation: MaskOperation,
        start_frame: u64,
        end_frame: u64,
        low_hz: u32,
        high_hz: u32,
    },
    Brush {
        operation: MaskOperation,
        radius_frames: u32,
        radius_hz: u32,
        #[serde(deserialize_with = "bounded::<_, _, MAX_POINTS>")]
        points: Vec<RepairMaskPoint>,
    },
    Lasso {
        operation: MaskOperation,
        #[serde(deserialize_with = "bounded::<_, _, MAX_POINTS>")]
        points: Vec<RepairMaskPoint>,
    },
}

impl RepairMaskRegion {
    pub(super) fn operation(&self) -> MaskOperation {
        match self {
            Self::Rectangle { operation, .. }
            | Self::Brush { operation, .. }
            | Self::Lasso { operation, .. } => *operation,
        }
    }
}

impl RepairMask {
    pub(in crate::codec) fn validate(
        &self,
        repair: &SpectralRepair,
        source: SourceInfo,
        end_frame: u64,
    ) -> Result<(), MediaError> {
        let invalid = MediaError::Range("Choose a valid spectral mask inside the repair bounds");
        if self.version != 1
            || self.feather_frames > source.sample_rate
            || self.feather_hz > 10_000
            || !(1..=MAX_REGIONS).contains(&self.regions.len())
            || self.regions[0].operation() != MaskOperation::Add
        {
            return Err(invalid);
        }
        let mut total_points = 0;
        for region in &self.regions {
            let contains = |frame: u64, hz: u32| {
                if region.operation() == MaskOperation::Add {
                    (repair.start_frame..=repair.end_frame).contains(&frame)
                        && (repair.low_hz..=repair.high_hz).contains(&hz)
                } else {
                    frame <= end_frame && hz <= source.sample_rate.div_ceil(2)
                }
            };
            let (points, min) = match region {
                RepairMaskRegion::Rectangle {
                    start_frame,
                    end_frame,
                    low_hz,
                    high_hz,
                    ..
                } => {
                    if start_frame >= end_frame
                        || low_hz >= high_hz
                        || !contains(*start_frame, *low_hz)
                        || !contains(*end_frame, *high_hz)
                    {
                        return Err(invalid);
                    }
                    continue;
                }
                RepairMaskRegion::Brush {
                    radius_frames,
                    radius_hz,
                    points,
                    ..
                } => {
                    if !(1..=source.sample_rate.saturating_mul(2)).contains(radius_frames)
                        || !(1..=20_000).contains(radius_hz)
                    {
                        return Err(invalid);
                    }
                    (points, 1)
                }
                RepairMaskRegion::Lasso { points, .. } => (points, 3),
            };
            total_points += points.len();
            if !(min..=MAX_POINTS).contains(&points.len())
                || total_points > MAX_TOTAL_POINTS
                || !points.iter().all(|point| contains(point.frame, point.hz))
            {
                return Err(invalid);
            }
        }
        Ok(())
    }
}

fn bounded<'de, D, T, const N: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T, const N: usize>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Bounded<T, N> {
        type Value = Vec<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "at most {N} spectral mask items")
        }

        fn visit_seq<S: SeqAccess<'de>>(self, mut values: S) -> Result<Self::Value, S::Error> {
            let mut items = Vec::with_capacity(values.size_hint().unwrap_or_default().min(N));
            for _ in 0..N {
                let Some(item) = values.next_element()? else {
                    return Ok(items);
                };
                items.push(item);
            }
            if values.next_element::<IgnoredAny>()?.is_some() {
                return Err(S::Error::invalid_length(N + 1, &self));
            }
            Ok(items)
        }
    }

    deserializer.deserialize_seq(Bounded::<T, N>(PhantomData))
}

pub(in crate::codec) struct RepairSpectra {
    audition: bool,
    channels: usize,
    step_hz: f64,
    fft: Arc<dyn Fft<f64>>,
    inverse: Arc<dyn Fft<f64>>,
    scratch: Vec<Complex64>,
    spectrum: Vec<Complex64>,
    inverse_spectrum: Vec<Complex64>,
    differences: Vec<f64>,
    window: Vec<f64>,
    gain: Vec<f64>,
    weights: Vec<f64>,
    region: Vec<f64>,
    edits: Vec<MaskEdit>,
}

struct MaskEdit {
    mask: Arc<RepairMask>,
    channel: Option<usize>,
    broadband: bool,
    amount: f64,
    full_gain: f64,
    time_ranges: Vec<Range<f64>>,
}

impl MaskEdit {
    fn new(repair: &SpectralRepair, mask: Arc<RepairMask>, nyquist: u32) -> Self {
        let broadband = mask.regions.iter().all(|region| {
            matches!(region,
            RepairMaskRegion::Rectangle { low_hz: 0, high_hz, .. } if *high_hz >= nyquist)
        });
        let bounds = repair.start_frame as f64..repair.end_frame as f64;
        let time_ranges = mask
            .regions
            .iter()
            .filter(|region| region.operation() == MaskOperation::Add)
            .map(|region| {
                let (start, end) = match region {
                    RepairMaskRegion::Rectangle {
                        start_frame,
                        end_frame,
                        ..
                    } => (*start_frame as f64, *end_frame as f64),
                    RepairMaskRegion::Brush {
                        radius_frames,
                        points,
                        ..
                    } => {
                        let (lo, hi) = time_extent(points);
                        (
                            lo - f64::from(*radius_frames),
                            hi + f64::from(*radius_frames),
                        )
                    }
                    RepairMaskRegion::Lasso { points, .. } => time_extent(points),
                };
                start.max(bounds.start)..end.min(bounds.end)
            })
            .collect();
        let amount = -f64::from(repair.amount_db) / 20.0;
        Self {
            mask,
            channel: repair
                .channel
                .map(|ch| usize::from(ch == super::RepairChannel::Right)),
            amount,
            full_gain: 10_f64.powf(amount),
            broadband,
            time_ranges,
        }
    }

    fn targets(&self, channel: usize) -> bool {
        self.channel.is_none_or(|only| only == channel)
    }

    fn time_support(&self, range: Range<f64>) -> bool {
        let feather = f64::from(self.mask.feather_frames);
        self.time_ranges
            .iter()
            .any(|r| r.start - feather < range.end && range.start < r.end + feather)
    }

    fn envelope(&self, position: f64) -> f64 {
        let feather = f64::from(self.mask.feather_frames);
        self.time_ranges
            .iter()
            .map(|range| {
                // A very short internal join prevents windowed changes from
                // introducing a new discontinuity at an unfeathered edge.
                let fade = ((range.end - range.start) / 2.0).min(64.0);
                let edge = (position - range.start + feather).min(range.end + feather - position);
                smooth(edge / fade.max(1.0))
            })
            .fold(0.0, f64::max)
    }

    fn broadband_weight(&self, position: f64) -> f64 {
        let mut weight = 0.0_f64;
        for region in &self.mask.regions {
            if let RepairMaskRegion::Rectangle {
                operation,
                start_frame,
                end_frame,
                ..
            } = region
            {
                let next = axis_weight(
                    position,
                    *start_frame as f64,
                    *end_frame as f64,
                    f64::from(self.mask.feather_frames),
                );
                combine(&mut weight, next, *operation);
            }
        }
        weight
    }
}

impl RepairSpectra {
    pub(in crate::codec) fn new(
        source: SourceInfo,
        repairs: impl Iterator<Item = SpectralRepair>,
    ) -> Self {
        let size = (source.sample_rate as usize / 12)
            .next_power_of_two()
            .clamp(1024, 4096);
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(size);
        let inverse = planner.plan_fft_inverse(size);
        let scratch = vec![
            Complex64::ZERO;
            fft.get_inplace_scratch_len()
                .max(inverse.get_inplace_scratch_len())
        ];
        let bins = size / 2 + 1;
        let channels = usize::from(source.channels);
        let edits = repairs
            .filter_map(|repair| {
                let mask = repair.mask.clone()?;
                Some(MaskEdit::new(&repair, mask, source.sample_rate.div_ceil(2)))
            })
            .collect();
        Self {
            audition: false,
            channels,
            step_hz: f64::from(source.sample_rate) / size as f64,
            fft,
            inverse,
            scratch,
            spectrum: vec![Complex64::ZERO; size],
            inverse_spectrum: vec![Complex64::ZERO; size],
            differences: vec![0.0; bins],
            // The sine window's squared weights sum to two at 75% overlap.
            window: (0..size)
                .map(|index| (PI * index as f64 / size as f64).sin())
                .collect(),
            gain: vec![1.0; bins],
            weights: vec![0.0; bins],
            region: vec![0.0; bins],
            edits,
        }
    }

    pub(in crate::codec) fn audition(source: SourceInfo, edit: SpectralRepair) -> Self {
        let mut spectra = Self::new(source, std::iter::once(edit));
        spectra.audition = true;
        // Even a full-band stripe can contain subtracted regions or exclude a channel.
        spectra.edits[0].broadband = false;
        spectra
    }

    pub(in crate::codec) fn size(&self) -> usize {
        self.window.len()
    }

    pub(in crate::codec) fn has_spectral(&self) -> bool {
        self.edits.iter().any(|edit| !edit.broadband)
    }

    pub(in crate::codec) fn has_broadband(&self, range: Range<u64>) -> bool {
        let range = range.start as f64..range.end as f64;
        self.edits
            .iter()
            .any(|edit| edit.broadband && edit.time_support(range.clone()))
    }

    /// Calculate weighted spectral deltas at an absolute, hop-aligned center.
    /// The caller adds overlapping deltas to aligned dry samples.
    pub(in crate::codec) fn window_delta(
        &mut self,
        center: i64,
        stock_start: u64,
        stock: &[i32],
        output: &mut Vec<f64>,
    ) {
        let size = self.size();
        let start = center - size as i64 / 2;
        let range = (center - size as i64 / 8) as f64..(center + size as i64 / 8) as f64;
        output.clear();
        if !self
            .edits
            .iter()
            .any(|edit| !edit.broadband && edit.time_support(range.clone()))
        {
            return;
        }
        for channel in 0..self.channels {
            self.gain.fill(1.0);
            let mut transformed = false;
            for edit in self.edits.iter().filter(|edit| {
                !edit.broadband && edit.targets(channel) && edit.time_support(range.clone())
            }) {
                self.weights.fill(0.0);
                // A narrow time span can fall between FFT centers. Clamp a
                // nearby center to that span, and gate synthesis to its
                // source support. Broader gestures follow the hop grid.
                let frame = center as f64;
                let frame = if !edit.time_ranges.iter().any(|r| r.contains(&frame)) {
                    edit.time_ranges
                        .iter()
                        .filter(|r| {
                            r.end - r.start < size as f64 / 4.0
                                && r.start < range.end
                                && range.start < r.end
                        })
                        .map(|r| r.start + (r.end - r.start) * 0.5)
                        .min_by(|a, b| (frame - a).abs().total_cmp(&(frame - b).abs()))
                        .unwrap_or(frame)
                } else {
                    frame
                };
                for region in &edit.mask.regions {
                    let bins =
                        raster_region(region, frame, self.step_hz, &edit.mask, &mut self.region);
                    let operation = region.operation();
                    for (weight, next) in self.weights[bins.clone()]
                        .iter_mut()
                        .zip(&mut self.region[bins])
                    {
                        combine(weight, *next, operation);
                        *next = 0.0;
                    }
                }
                if self.weights.iter().all(|&weight| weight == 0.0) {
                    continue;
                }
                if !transformed {
                    for (index, (bin, &window)) in
                        self.spectrum.iter_mut().zip(&self.window).enumerate()
                    {
                        let position = start + index as i64 - stock_start as i64;
                        let pcm = usize::try_from(position)
                            .ok()
                            .and_then(|position| stock.get(position * self.channels + channel))
                            .copied()
                            .unwrap_or_default();
                        *bin = Complex64::new(f64::from(pcm) * window, 0.0);
                    }
                    self.fft
                        .process_with_scratch(&mut self.spectrum, &mut self.scratch);
                    transformed = true;
                }
                for ((gain, difference), &weight) in self
                    .gain
                    .iter_mut()
                    .zip(&mut self.differences)
                    .zip(&self.weights)
                {
                    if self.audition {
                        *difference = weight;
                        continue;
                    }
                    let change = if weight == 0.0 {
                        1.0
                    } else if weight == 1.0 {
                        edit.full_gain
                    } else {
                        10_f64.powf(edit.amount * weight)
                    };
                    *difference = *gain * (change - 1.0);
                    *gain *= change;
                }
                // Synthesize only this edit's increment. Gating each increment
                // before summation keeps its tail out of an abutting edit.
                for (index, (next, source)) in self
                    .inverse_spectrum
                    .iter_mut()
                    .zip(&self.spectrum)
                    .enumerate()
                {
                    *next = source * self.differences[index.min(size - index)];
                }
                self.inverse
                    .process_with_scratch(&mut self.inverse_spectrum, &mut self.scratch);
                output.resize(size * self.channels, 0.0);
                for (index, (bin, &window)) in
                    self.inverse_spectrum.iter().zip(&self.window).enumerate()
                {
                    let envelope = edit.envelope(start as f64 + index as f64 + 0.5);
                    output[index * self.channels + channel] +=
                        envelope * bin.re * window / (2 * size) as f64;
                }
            }
        }
    }

    pub(in crate::codec) fn render_sample(
        &self,
        frame: u64,
        channel: usize,
        dry: i32,
        delta: f64,
    ) -> i32 {
        let position = frame as f64 + 0.5;
        let gain = self
            .edits
            .iter()
            .filter(|edit| edit.broadband && edit.targets(channel))
            .map(|edit| edit.broadband_weight(position) * edit.amount)
            .sum::<f64>();
        let wet = f64::from(dry) + delta;
        (if gain == 0.0 {
            wet
        } else {
            wet * 10_f64.powf(gain)
        })
        .round() as i32
    }
}

fn smooth(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn axis_weight(x: f64, lo: f64, hi: f64, feather: f64) -> f64 {
    if x >= lo && x < hi {
        1.0
    } else if feather > 0.0 {
        smooth(1.0 - (lo - x).max(x - hi) / feather)
    } else {
        0.0
    }
}

fn band_area(x: f64, lo: f64, hi: f64, feather: f64) -> f64 {
    let solid = (x - lo).clamp(0.0, hi - lo);
    if feather == 0.0 {
        return solid;
    }
    // Integral of smoothstep, measured from the start of each shoulder.
    let rise = ((x - lo) / feather + 1.0).clamp(0.0, 1.0);
    let fall = ((x - hi) / feather).clamp(0.0, 1.0);
    let integral = |t: f64| t * t * t * (1.0 - t / 2.0);
    solid + feather * (integral(rise) + fall - integral(fall))
}

fn combine(previous: &mut f64, next: f64, operation: MaskOperation) {
    match operation {
        MaskOperation::Add => *previous = previous.max(next),
        MaskOperation::Subtract => *previous *= 1.0 - next,
    }
}

fn time_extent(points: &[RepairMaskPoint]) -> (f64, f64) {
    let first = points.first().map_or(0.0, |point| point.frame as f64);
    points.iter().fold((first, first), |(lo, hi), point| {
        (lo.min(point.frame as f64), hi.max(point.frame as f64))
    })
}

fn bins_between(lo: f64, hi: f64, bin_hz: f64, len: usize) -> Range<usize> {
    ((lo / bin_hz).ceil() as usize).min(len)
        ..(((hi / bin_hz).floor() as usize).saturating_add(1)).min(len)
}

fn include_bins(support: &mut Range<usize>, bins: &Range<usize>) {
    if !bins.is_empty() {
        support.start = support.start.min(bins.start);
        support.end = support.end.max(bins.end);
    }
}

/// Rasterize one region into zeroed scratch and return the bins to combine and clear.
fn raster_region(
    shape: &RepairMaskRegion,
    frame: f64,
    bin_hz: f64,
    mask: &RepairMask,
    weights: &mut [f64],
) -> Range<usize> {
    let time_feather = f64::from(mask.feather_frames);
    let freq_feather = f64::from(mask.feather_hz);
    let mut support = weights.len()..0;
    match shape {
        RepairMaskRegion::Rectangle {
            start_frame,
            end_frame,
            low_hz,
            high_hz,
            ..
        } => {
            let temporal = axis_weight(frame, *start_frame as f64, *end_frame as f64, time_feather);
            if temporal > 0.0 {
                let (lo, hi) = (f64::from(*low_hz), f64::from(*high_hz));
                let half = bin_hz / 2.0;
                let nyquist = (weights.len() - 1) as f64 * bin_hz;
                let bins = bins_between(
                    lo - freq_feather - half,
                    hi + freq_feather + half,
                    bin_hz,
                    weights.len(),
                );
                support = bins.clone();
                for (index, weight) in bins.clone().zip(&mut weights[bins]) {
                    // Include narrow rectangles that fall between FFT centers.
                    let hz = index as f64 * bin_hz;
                    let cell_start = (hz - half).max(0.0);
                    let cell_end = (hz + half).min(nyquist);
                    let area = band_area(cell_end, lo, hi, freq_feather)
                        - band_area(cell_start, lo, hi, freq_feather);
                    *weight = temporal * (area / (cell_end - cell_start)).clamp(0.0, 1.0);
                }
            }
        }
        RepairMaskRegion::Brush {
            radius_frames,
            radius_hz,
            points,
            ..
        } => {
            let radii = [f64::from(*radius_frames), f64::from(*radius_hz)];
            let outer = [radii[0] + time_feather, radii[1] + freq_feather];
            let starts = points.iter().take(points.len().saturating_sub(1).max(1));
            for (first, second) in starts.zip(points.iter().skip(1).chain(points.last())) {
                let a = [first.frame as f64, f64::from(first.hz)];
                let b = [second.frame as f64, f64::from(second.hz)];
                if frame < a[0].min(b[0]) - outer[0] || frame > a[0].max(b[0]) + outer[0] {
                    continue;
                }
                let bins = bins_between(
                    a[1].min(b[1]) - outer[1],
                    a[1].max(b[1]) + outer[1],
                    bin_hz,
                    weights.len(),
                );
                include_bins(&mut support, &bins);
                for (index, weight) in bins.clone().zip(&mut weights[bins]) {
                    let point = [frame, index as f64 * bin_hz];
                    let inner = segment_distance(point, a, b, radii);
                    let next = if inner <= 1.0 {
                        1.0
                    } else {
                        let edge = segment_distance(point, a, b, outer);
                        if edge >= 1.0 {
                            0.0
                        } else {
                            smooth((1.0 - edge) / (inner - edge))
                        }
                    };
                    *weight = weight.max(next);
                }
            }
        }
        RepairMaskRegion::Lasso { points, .. } => {
            let edges = points.iter().zip(points.iter().cycle().skip(1));
            let mut crossings = Vec::new();
            let mut nearby = Vec::new();
            for (a, b) in edges {
                let (x0, x1) = (a.frame as f64, b.frame as f64);
                let (y0, y1) = (f64::from(a.hz), f64::from(b.hz));
                if (x0 <= frame && frame < x1) || (x1 <= frame && frame < x0) {
                    crossings.push(y0 + (y1 - y0) * (frame - x0) / (x1 - x0));
                }
                if (time_feather > 0.0 || freq_feather > 0.0)
                    && x0.min(x1) - time_feather <= frame
                    && frame <= x0.max(x1) + time_feather
                {
                    nearby.push(([x0, y0], [x1, y1]));
                }
            }
            crossings.sort_unstable_by(f64::total_cmp);
            for &[low, high] in crossings.as_chunks::<2>().0 {
                let bins = bins_between(low, high, bin_hz, weights.len());
                include_bins(&mut support, &bins);
                weights[bins].fill(1.0);
            }
            let radii = [time_feather.max(1e-6), freq_feather.max(1e-6)];
            for (a, b) in nearby {
                let bins = bins_between(
                    a[1].min(b[1]) - freq_feather,
                    a[1].max(b[1]) + freq_feather,
                    bin_hz,
                    weights.len(),
                );
                include_bins(&mut support, &bins);
                for (index, weight) in bins.clone().zip(&mut weights[bins]) {
                    let distance = segment_distance([frame, index as f64 * bin_hz], a, b, radii);
                    *weight = weight.max(smooth(1.0 - distance));
                }
            }
        }
    }
    support.start.min(support.end)..support.end
}

// Coordinates normalized to the two axes of the ellipse. This preserves
// the brush footprint while crossing several bins between pointer points.
fn segment_distance(point: [f64; 2], a: [f64; 2], b: [f64; 2], radii: [f64; 2]) -> f64 {
    let dx = (b[0] - a[0]) / radii[0];
    let dy = (b[1] - a[1]) / radii[1];
    let x = (point[0] - a[0]) / radii[0];
    let y = (point[1] - a[1]) / radii[1];
    let length = dx * dx + dy * dy;
    let factor = if length > 0.0 {
        ((x * dx + y * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (x - dx * factor).hypot(y - dy * factor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{hint::black_box, time::Instant};

    #[test]
    fn rectangle_feathers_time_and_frequency_independently() {
        let region = RepairMaskRegion::Rectangle {
            operation: MaskOperation::Add,
            start_frame: 100,
            end_frame: 500,
            low_hz: 1000,
            high_hz: 1500,
        };
        let mask = RepairMask {
            version: 1,
            feather_frames: 40,
            feather_hz: 100,
            regions: Vec::new(),
        };
        let mut weights = [0.0; 40];
        let bin_hz = 50.0;
        raster_region(&region, 80.0, bin_hz, &mask, &mut weights);
        assert_eq!(weights[24], 0.5);
        assert_eq!(weights[19], 0.25);
        assert_eq!(weights[17], 0.0);
        weights.fill(0.0);
        raster_region(&region, 300.0, bin_hz, &mask, &mut weights);
        assert_eq!(weights[24], 1.0);
        assert_eq!(weights[19], 0.5);
        assert_eq!(weights[17], 0.0);
    }

    #[test]
    fn narrow_rectangle_weights_fft_cells_between_centers_and_at_nyquist() {
        let mask = RepairMask {
            version: 1,
            feather_frames: 0,
            feather_hz: 0,
            regions: Vec::new(),
        };
        let rectangle = |low_hz, high_hz| RepairMaskRegion::Rectangle {
            operation: MaskOperation::Add,
            start_frame: 400,
            end_frame: 800,
            low_hz,
            high_hz,
        };
        let mut weights = vec![0.0; 2049];
        let bin_hz = 96_000.0 / 4096.0;
        raster_region(&rectangle(986, 1006), 600.0, bin_hz, &mask, &mut weights);
        assert!(weights[42] > 0.0 && weights[43] > 0.0);
        assert_eq!(weights.iter().filter(|&&weight| weight > 0.0).count(), 2);
        assert!((weights[42] + weights[43] - 20.0 / bin_hz).abs() < 1e-12);

        weights.fill(0.0);
        raster_region(
            &rectangle(47_996, 48_000),
            600.0,
            bin_hz,
            &mask,
            &mut weights,
        );
        assert_eq!(weights[2047], 0.0);
        assert!((weights[2048] - 8.0 / bin_hz).abs() < 1e-12);
    }

    #[test]
    fn dense_stereo_masks_keep_one_window_of_kernel_output() {
        let source = SourceInfo {
            sample_rate: 48_000,
            channels: 2,
            frames: 48_000,
            bits_per_sample: Some(24),
        };
        let brush = |hz| RepairMaskRegion::Brush {
            operation: MaskOperation::Add,
            radius_frames: 720,
            radius_hz: 240,
            points: (0..MAX_POINTS)
                .map(|index| RepairMaskPoint {
                    frame: (index as u64 + 1) * 180,
                    hz: hz + 60 * (index % 9) as u32,
                })
                .collect(),
        };
        let mut regions = vec![brush(1_000), brush(3_000)];
        regions.extend(
            (0..MAX_REGIONS - 2).map(|index| RepairMaskRegion::Rectangle {
                operation: MaskOperation::Add,
                start_frame: 4000 + index as u64 * 2000,
                end_frame: 6500 + index as u64 * 2000,
                low_hz: 5000 + index as u32 * 100,
                high_hz: 5600 + index as u32 * 100,
            }),
        );
        let repair = SpectralRepair {
            start_frame: 0,
            end_frame: source.frames,
            low_hz: 100,
            high_hz: 10_000,
            amount_db: 18,
            mask: Some(Arc::new(RepairMask {
                version: 1,
                feather_frames: 240,
                feather_hz: 50,
                regions,
            })),
            ..SpectralRepair::default()
        };
        let mut processor = RepairSpectra::new(
            source,
            std::iter::repeat_n(repair, super::super::MAX_SPECTRAL_REPAIRS),
        );
        let size = processor.size();
        let samples = vec![1_000_000_i32; source.frames as usize * 2];
        let mut output = Vec::new();
        let mut touched = 0;
        let started = Instant::now();
        for center in (0..source.frames as i64).step_by(size / 4) {
            processor.window_delta(center, 0, &samples, &mut output);
            assert!(output.capacity() <= size * 2);
            touched += usize::from(!black_box(&output).is_empty());
        }
        assert!(touched >= 40);
        eprintln!(
            "one second of stereo audio with eight 16-region masks and 512 points each: {:?}",
            started.elapsed()
        );
    }
}
