//! Analysis uses decoded playback packets on the worker, never on the device callback.
use super::{
    centered, interleaved, level_db, overlap, probe_fft_size, spectrum_views, with_spectrum,
    MediaError, SourceInfo, SpectralEqResponse, TreatmentProbe, TreatmentProbeOptions,
    REDUCTION_POINTS,
};
use crate::AudioTreatment;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const MAX_HISTORY_SAMPLES: usize = 2_097_152;

#[derive(Clone, Default)]
pub struct PlaybackReadings {
    pub reduction: f32,
    pub detector: Option<f32>,
    pub gate_reduction: f32,
    pub mix: Vec<f32>,
    pub eq_detectors: Vec<Option<f32>>,
    pub overload: bool,
    pub eq_input_peak: Option<f32>,
    pub spectral_response: Vec<SpectralEqResponse>,
}

struct Packet {
    start: u64,
    end: u64,
    dry: Vec<i32>,
    wet: Vec<i32>,
    readings: PlaybackReadings,
    revision: u64,
}

#[derive(Default)]
pub struct PlaybackHistory {
    requested: Option<Instant>,
    layout: Option<(SourceInfo, SourceInfo)>,
    treatment: AudioTreatment,
    revision: u64,
    producing: bool,
    cached_loop: Option<crate::FrameRange>,
    packets: VecDeque<Packet>,
    sample_count: usize,
    last_read: Option<u64>,
}

pub struct PlaybackProbeWindow {
    rate: u32,
    dry_channels: usize,
    wet_channels: usize,
    dry: Vec<i32>,
    wet: Vec<i32>,
    readings: PlaybackReadings,
    output_peak: f32,
    reduction: Vec<f32>,
    gate: Option<Vec<f32>>,
}

impl PlaybackHistory {
    fn footprint(packet: &Packet) -> usize {
        packet.dry.len()
            + packet.wet.len()
            + packet
                .readings
                .spectral_response
                .iter()
                .map(|trace| trace.gains_db.len())
                .sum::<usize>()
    }

    pub fn set_producing(&mut self, producing: bool) {
        self.producing = producing;
    }

    pub fn producing(&self) -> bool {
        self.producing
    }

    pub fn retain_cached_loop(&mut self, start: u64, end: u64) {
        self.cached_loop = Some(crate::FrameRange { start, end });
    }

    pub fn wanted(&self) -> bool {
        self.cached_loop.is_some()
            || self
                .requested
                .is_some_and(|time| time.elapsed() < Duration::from_secs(2))
    }

    pub fn push(
        &mut self,
        start: u64,
        layout: (SourceInfo, SourceInfo),
        treatment: AudioTreatment,
        dry: &[i32],
        wet: &[i32],
        readings: PlaybackReadings,
    ) {
        if !self.wanted() {
            return;
        }
        let channels = usize::from(layout.0.channels);
        let wet_channels = usize::from(layout.1.channels);
        let frames = (dry.len() / channels).min(wet.len() / wet_channels);
        let range = self.cached_loop.map_or(
            (frames != 0).then_some(crate::FrameRange {
                start,
                end: start + frames as u64,
            }),
            |loop_range| overlap(start, start + frames as u64, loop_range),
        );
        let Some(range) = range else { return };
        let dry = &dry[interleaved(range, start, channels)];
        let wet = &wet[interleaved(range, start, wet_channels)];
        let start = range.start;
        if self.layout != Some(layout) || self.packets.back().is_some_and(|last| last.end != start)
        {
            self.packets.clear();
            self.sample_count = 0;
            self.last_read = None;
        }
        self.layout = Some(layout);
        if self.treatment != treatment {
            self.revision = self.revision.wrapping_add(1);
            self.treatment = treatment;
        }
        let end = range.end;
        let packet = Packet {
            start,
            end,
            dry: dry.to_vec(),
            wet: wet.to_vec(),
            readings,
            revision: self.revision,
        };
        self.sample_count += Self::footprint(&packet);
        self.packets.push_back(packet);
        let oldest = self.cached_loop.map_or_else(
            || end.saturating_sub(u64::from(layout.0.sample_rate) * 2),
            |range| range.start,
        );
        while self
            .packets
            .front()
            .is_some_and(|first| first.end <= oldest || self.sample_count > MAX_HISTORY_SAMPLES)
        {
            let packet = self
                .packets
                .pop_front()
                .expect("first playback packet exists");
            self.sample_count -= Self::footprint(&packet);
        }
    }

    pub fn take(&mut self, frame: u64, treatment: AudioTreatment) -> Option<PlaybackProbeWindow> {
        self.requested = Some(Instant::now());
        let (input, output) = self.layout?;
        if frame >= input.frames {
            return None;
        }
        let first = self.packets.front()?.start;
        let last = self.packets.back()?.end;
        let width = u64::from(live_fft_size(input.sample_rate));
        let desired = centered(frame, width, input.frames);
        let audible = self
            .packets
            .iter()
            .find(|packet| (packet.start..packet.end).contains(&frame))?;
        let mut readings = audible.readings.clone();
        if self.treatment != treatment || audible.revision != self.revision {
            // PCM and peaks describe the sound at the playhead during a retune.
            // Per-band readings belong only to the matching recipe.
            readings.mix.clear();
            readings.eq_detectors.clear();
            readings.spectral_response.clear();
        }
        // A playback worker can be slightly behind the right side of a centred
        // FFT window. Use the available adjacent frames rather than seeking.
        let span = overlap(first, last, desired)?;
        let available = self
            .cached_loop
            .map_or(input.frames, |range| range.end - range.start);
        if span.end - span.start < width.min(available) / 2 {
            return None;
        }
        let dry_channels = usize::from(input.channels);
        let wet_channels = usize::from(output.channels);
        let frames = (span.end - span.start) as usize;
        let mut dry = Vec::with_capacity(frames * dry_channels);
        let mut wet = Vec::with_capacity(frames * wet_channels);
        let history = frame.saturating_sub(u64::from(input.sample_rate) * 3 / 4);
        let step = (frame + 1 - history).div_ceil(REDUCTION_POINTS as u64);
        let mut reduction = Vec::with_capacity(REDUCTION_POINTS);
        let mut gate = (treatment.gate_db != 0).then(|| Vec::with_capacity(REDUCTION_POINTS));
        let mut at = history.max(first);
        let meter_start = frame
            .saturating_sub(u64::from(input.sample_rate) / 30)
            .max(span.start);
        let meter_range = crate::FrameRange {
            start: meter_start,
            end: frame + 1,
        };
        let since = self
            .last_read
            .filter(|previous| *previous <= frame)
            .unwrap_or(meter_start);
        self.last_read = Some(frame);
        let mut overload = false;
        let mut eq_input_peak = None::<f32>;
        let mut output_peak = 0_u32;
        for packet in &self.packets {
            if let Some(part) = overlap(packet.start, packet.end, span) {
                dry.extend_from_slice(&packet.dry[interleaved(part, packet.start, dry_channels)]);
                wet.extend_from_slice(&packet.wet[interleaved(part, packet.start, wet_channels)]);
            }
            if let Some(part) = overlap(packet.start, packet.end, meter_range) {
                if packet.revision == audible.revision {
                    if let Some(peak) = packet.readings.eq_input_peak {
                        eq_input_peak =
                            Some(eq_input_peak.map_or(peak, |previous| previous.max(peak)));
                    }
                }
                for sample in &packet.wet[interleaved(part, packet.start, wet_channels)] {
                    output_peak = output_peak.max(sample.unsigned_abs());
                }
            }
            if packet.end > since && packet.start <= frame {
                overload |= packet.readings.overload;
            }
            while at.min(frame) >= packet.start && at < packet.end && at <= frame {
                if treatment.compressor_db != 0 {
                    reduction.push(packet.readings.reduction);
                }
                if let Some(gate) = &mut gate {
                    gate.push(packet.readings.gate_reduction);
                }
                at += step;
            }
        }
        readings.overload |= overload;
        readings.eq_input_peak = eq_input_peak;
        Some(PlaybackProbeWindow {
            rate: input.sample_rate,
            dry_channels,
            wet_channels,
            dry,
            wet,
            readings,
            output_peak: super::amplitude_db(f64::from(output_peak) / f64::from(super::PCM_SCALE)),
            reduction,
            gate,
        })
    }
}

fn live_fft_size(rate: u32) -> u32 {
    probe_fft_size(rate) / 2
}

fn smooth_display_trace(levels: &mut [f32]) {
    let Some(&first) = levels.first() else {
        return;
    };
    let mut left = first;
    for index in 1..levels.len().saturating_sub(1) {
        let center = levels[index];
        levels[index] = 0.76 * center + 0.12 * (left + levels[index + 1]);
        left = center;
    }
}

impl PlaybackProbeWindow {
    pub fn analyze(
        self,
        options: TreatmentProbeOptions,
        mut stopped: impl FnMut() -> bool,
    ) -> Result<TreatmentProbe, MediaError> {
        let (input, input_peak) = level_db(&self.dry);
        let (output, _) = level_db(&self.wet);
        let (mut original, mut treated, mut frequencies) = (Vec::new(), Vec::new(), Vec::new());
        if options.spectrum {
            with_spectrum(live_fft_size(self.rate) as usize, self.rate, |spectrum| {
                (original, treated) = spectrum_views(
                    spectrum,
                    (&self.dry, self.dry_channels),
                    (&self.wet, self.wet_channels),
                    true,
                    options.spectrum_view,
                    &mut stopped,
                )?;
                smooth_display_trace(&mut original);
                smooth_display_trace(&mut treated);
                frequencies = spectrum.frequencies.clone();
                Ok::<(), MediaError>(())
            })?;
        }
        Ok(TreatmentProbe {
            frequencies,
            original,
            treated,
            reduction: self.reduction,
            input,
            output,
            input_peak,
            eq_input_peak: self.readings.eq_input_peak.unwrap_or(input_peak),
            output_peak: self.output_peak,
            output_overload: self.readings.overload,
            from_playback: true,
            detector: self.readings.detector,
            gate_reduction: self.gate,
            dynamic_eq_mix: self.readings.mix,
            spectral_response: self.readings.spectral_response,
            dynamic_eq_detectors: self.readings.eq_detectors,
            eq_threshold_learning: true,
            upward_eq: true,
            dynamic_all_shapes: true,
            dynamic_ratio: true,
            spectral_eq: true,
            spectrogram: None,
            spectrogram_start: None,
            spectrogram_end: None,
            spectrogram_channel: None,
            spectrogram_treated: None,
            spectrogram_waveform: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meters_follow_the_trimmed_signal_and_cached_loop() {
        let source = SourceInfo {
            sample_rate: 48_000,
            channels: 1,
            frames: 150_000,
            bits_per_sample: Some(24),
        };
        let settings = AudioTreatment {
            eq_input_db: -18.0,
            ..AudioTreatment::default()
        };
        let mut processor = crate::PreparedTreatment::preview(settings, source)
            .unwrap()
            .processor(source)
            .unwrap();
        let mut history = PlaybackHistory::default();
        history.retain_cached_loop(1_000, 145_000);
        let input = vec![1_000_000_000; 2_000];
        let mut output = Vec::new();
        for frame in (0..146_000).step_by(2_000) {
            let readings = processor.process_for_playback(&input, &mut output);
            history.push(frame, (source, source), settings, &input, &output, readings);
        }
        for frame in [144_000, 1_000] {
            let probe = history
                .take(frame, settings)
                .unwrap()
                .analyze(TreatmentProbeOptions::default(), || false)
                .unwrap();
            assert!((probe.eq_input_peak - probe.input_peak + 18.0).abs() < 0.01);
            assert!((probe.output_peak - probe.eq_input_peak).abs() < 0.01);
        }
    }

    #[test]
    fn live_spectrum_uses_audible_packets_during_a_treatment_change() {
        let input = SourceInfo {
            sample_rate: 48_000,
            channels: 1,
            frames: 48_000,
            bits_per_sample: Some(16),
        };
        let mut history = PlaybackHistory::default();
        let dry = vec![1_000_000_000; 4096];
        let wet = vec![500_000_000; dry.len()];
        let recipe = AudioTreatment::default();
        let changed = AudioTreatment {
            eq_output_db: -6.0,
            ..recipe
        };
        assert!(history.take(0, recipe).is_none());
        history.push(
            0,
            (input, input),
            recipe,
            &dry,
            &wet,
            PlaybackReadings {
                mix: vec![0.3],
                eq_detectors: vec![Some(-12.0)],
                ..PlaybackReadings::default()
            },
        );
        history.push(
            4096,
            (input, input),
            changed,
            &dry,
            &wet,
            PlaybackReadings {
                overload: true,
                mix: vec![0.7],
                eq_detectors: vec![Some(-18.0)],
                ..PlaybackReadings::default()
            },
        );
        for (frame, settings) in [(3000, changed), (5000, recipe)] {
            let older = history
                .take(frame, settings)
                .unwrap()
                .analyze(
                    TreatmentProbeOptions {
                        spectrum: true,
                        ..TreatmentProbeOptions::default()
                    },
                    || false,
                )
                .unwrap();
            assert!(older.from_playback && !older.frequencies.is_empty());
            assert!(older.dynamic_eq_mix.is_empty() && older.dynamic_eq_detectors.is_empty());
            assert!((older.input - older.output - 6.0206).abs() < 0.001);
        }
        let result = history
            .take(5000, changed)
            .unwrap()
            .analyze(
                TreatmentProbeOptions {
                    spectrum: true,
                    spectrum_view: super::super::SpectrumView::After,
                    ..TreatmentProbeOptions::default()
                },
                || false,
            )
            .unwrap();
        assert!(result.from_playback);
        assert!(result.output_overload);
        assert!(result.original.is_empty() && !result.treated.is_empty());
        assert_eq!(result.dynamic_eq_mix, [0.7]);
        assert_eq!(result.dynamic_eq_detectors, [Some(-18.0)]);
        assert!((result.input - result.output - 6.0206).abs() < 0.001);
        history.push(
            12_288,
            (input, input),
            changed,
            &dry,
            &wet,
            PlaybackReadings::default(),
        );
        assert!(history.take(5000, changed).is_none());
    }
}
