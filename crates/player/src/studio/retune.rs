//! An EQ parameter edit can reuse the decoder and its dry history. It is
//! applied by the producer before the next packet reaches the speaker.
use std::{collections::VecDeque, sync::Mutex};
use tagrex_audio_workbench::{
    PlaybackReadings, PreparedTreatment, RepairBlock, SourceInfo, TreatmentProcessor,
};

const MAX_PREROLL_SAMPLES: usize = 96_000;
const FADE_MILLISECONDS: u32 = 12;

#[derive(Default)]
pub(super) struct RetuneSlot(Mutex<Target>);

#[derive(Default)]
struct Target {
    accepting: bool,
    next: Option<PreparedTreatment>,
}

impl RetuneSlot {
    pub(super) fn enable(&self) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).accepting = true;
    }

    pub(super) fn submit(&self, target: PreparedTreatment) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if !state.accepting {
            return false;
        }
        state.next = Some(target);
        true
    }

    fn take(&self, last: bool, transitioning: bool) -> Option<PreparedTreatment> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.accepting &= !last;
        if transitioning && !last {
            return None;
        }
        state.next.take()
    }

    pub(super) fn close(&self) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.accepting = false;
        state.next = None;
    }
}

pub(super) struct LiveProcessor {
    active: TreatmentProcessor,
    prepared: PreparedTreatment,
    previous: Option<TreatmentProcessor>,
    dry: VecDeque<i32>,
    scratch: Vec<i32>,
    fade_left: usize,
    input_channels: usize,
    fade_frames: usize,
    history_limit: usize,
}

impl LiveProcessor {
    pub(super) fn new(
        active: TreatmentProcessor,
        prepared: PreparedTreatment,
        source: SourceInfo,
        enabled: bool,
    ) -> Self {
        let input_channels = usize::from(source.channels).max(1);
        let history_limit = if enabled {
            (source.sample_rate as usize / 4).min(MAX_PREROLL_SAMPLES / input_channels)
                * input_channels
        } else {
            0
        };
        Self {
            active,
            prepared,
            previous: None,
            dry: VecDeque::with_capacity(history_limit),
            scratch: Vec::new(),
            fade_left: 0,
            input_channels,
            fade_frames: (source.sample_rate * FADE_MILLISECONDS / 1000).max(1) as usize,
            history_limit,
        }
    }

    pub(super) fn output_info(&self) -> SourceInfo {
        self.active.output_info()
    }

    pub(super) fn prepared(&self) -> &PreparedTreatment {
        &self.prepared
    }

    pub(super) fn retune(
        &mut self,
        slot: &RetuneSlot,
        source: SourceInfo,
        next_frame: u64,
        last: bool,
    ) {
        if self.history_limit == 0 {
            return;
        }
        let Some(next) = slot.take(last, self.previous.is_some()) else {
            return;
        };
        let history_start =
            next_frame.saturating_sub((self.dry.len() / self.input_channels) as u64);
        let Ok(mut processor) = next.processor_at(source, history_start) else {
            return;
        };
        if self.active.glide_eq(&mut processor, self.fade_frames) {
            self.prepared = next;
            return;
        }
        processor.process(self.dry.make_contiguous(), &mut self.scratch);
        processor.carry_eq_state(&self.active);
        self.previous = Some(std::mem::replace(&mut self.active, processor));
        self.prepared = next;
        self.fade_left = self.fade_frames;
    }

    pub(super) fn process_source(
        &mut self,
        block: RepairBlock<'_>,
        output: &mut Vec<i32>,
        meter: bool,
    ) -> Option<PlaybackReadings> {
        let Some(repaired) = block.repaired else {
            return self.process(block.dry, output, meter);
        };
        if meter {
            Some(
                self.active
                    .process_repaired_for_playback(block.dry, repaired, output),
            )
        } else {
            self.active.process_aligned(block.dry, repaired, output);
            None
        }
    }

    fn process(
        &mut self,
        input: &[i32],
        output: &mut Vec<i32>,
        meter: bool,
    ) -> Option<PlaybackReadings> {
        let readings = if meter {
            Some(self.active.process_for_playback(input, output))
        } else {
            self.active.process(input, output);
            None
        };
        if let Some(previous) = &mut self.previous {
            let channels = usize::from(self.active.output_info().channels);
            let frames = self.fade_left.min(output.len() / channels);
            previous.process(&input[..frames * self.input_channels], &mut self.scratch);
            let (elapsed, length) = (self.fade_frames - self.fade_left, self.fade_frames);
            for (offset, (wet, old)) in output
                .chunks_exact_mut(channels)
                .zip(self.scratch.chunks_exact(channels))
                .enumerate()
            {
                let mix = (elapsed + offset + 1) as f64 / length as f64;
                for (wet, old) in wet.iter_mut().zip(old) {
                    *wet = (f64::from(*old) * (1.0 - mix) + f64::from(*wet) * mix).round() as i32;
                }
            }
            self.fade_left -= frames;
            if self.fade_left == 0 {
                self.previous = None;
            }
        }
        if self.history_limit != 0 {
            let retained = &input[input.len().saturating_sub(self.history_limit)..];
            let discard = (self.dry.len() + retained.len()).saturating_sub(self.history_limit);
            self.dry.drain(..discard);
            self.dry.extend(retained);
        }
        readings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tagrex_audio_workbench::{
        AudioTreatment, DynamicEq, EqBand, EqBandKind, EqChannel, MAX_EQ_BANDS,
    };

    #[test]
    fn a_parameter_edit_reuses_the_stream_and_fades_to_the_target() {
        let source = SourceInfo {
            sample_rate: 48_000,
            channels: 2,
            frames: 96_000,
            bits_per_sample: Some(16),
        };
        let before = PreparedTreatment::preview(AudioTreatment::default(), source).unwrap();
        let after = PreparedTreatment::preview(
            AudioTreatment {
                eq_output_db: -6.0,
                ..AudioTreatment::default()
            },
            source,
        )
        .unwrap();
        assert!(before.can_retune_eq(&after, source.sample_rate));
        let slot = RetuneSlot::default();
        slot.enable();
        let mut live = LiveProcessor::new(
            before.processor(source).unwrap(),
            before.clone(),
            source,
            true,
        );
        let input = vec![120_000_000; 4096];
        let mut output = Vec::new();
        live.process(&input, &mut output, false);
        assert_eq!(input, output);
        assert!(slot.submit(after.clone()));
        live.retune(&slot, source, (input.len() / 2) as u64, false);
        live.process(&input, &mut output, false);
        assert_eq!(output.len(), input.len());
        assert!(output[0] > 119_000_000);
        assert!((output[output.len() - 1] - 60_142_468).abs() < 10);
        assert!(output
            .as_chunks::<2>()
            .0
            .iter()
            .all(|frame| frame[0] == frame[1]));
        assert!(live.prepared().same_preview(&after));
        assert!(live.previous.is_none());
        live.retune(&slot, source, input.len() as u64, true);
        assert!(!slot.submit(before));
    }

    #[test]
    fn moving_a_steep_cut_does_not_cancel_the_sound_in_its_passband() {
        let source = SourceInfo {
            sample_rate: 48_000,
            channels: 1,
            frames: 96_000,
            bits_per_sample: Some(24),
        };
        let cut = |hz| {
            let mut bands = [None; MAX_EQ_BANDS];
            bands[0] = Some(EqBand {
                id: Some(1),
                kind: EqBandKind::High,
                hz,
                q: 100,
                slope: 48,
                ..EqBand::default()
            });
            PreparedTreatment::preview(
                AudioTreatment {
                    eq_bands: Some(bands),
                    ..AudioTreatment::default()
                },
                source,
            )
            .unwrap()
        };
        let before = cut(1_200);
        let after = cut(2_200);
        let slot = RetuneSlot::default();
        slot.enable();
        let mut live = LiveProcessor::new(before.processor(source).unwrap(), before, source, true);
        let input: Vec<_> = (0..12_000)
            .map(|sample| {
                (100_000_000.0 * (f64::from(sample) * std::f64::consts::TAU / 48.0).sin()) as i32
            })
            .collect();
        let mut output = Vec::new();
        live.process(&input, &mut output, false);
        assert!(slot.submit(after.clone()));
        live.retune(&slot, source, input.len() as u64, false);
        live.process(&input, &mut output, false);
        let energy = |samples: &[i32]| {
            samples
                .iter()
                .map(|&sample| f64::from(sample).powi(2))
                .sum::<f64>()
        };
        let dry_energy = energy(&input[..96]);
        for window in output[..960].as_chunks::<96>().0 {
            let gain_db = 10.0 * (energy(window) / dry_energy).log10();
            assert!(gain_db > -2.0, "unexpected passband dip of {gain_db:.2} dB");
        }
    }

    #[test]
    fn a_spectral_drag_reuses_the_producer_history_without_a_second_voice() {
        let source = SourceInfo {
            sample_rate: 192_000,
            channels: 2,
            frames: 384_000,
            bits_per_sample: Some(24),
        };
        let settings = |gain_db, amount_db| AudioTreatment {
            eq_bands: Some(std::array::from_fn(|index| {
                let spectral = index != MAX_EQ_BANDS - 1;
                Some(EqBand {
                    id: Some(index as u32 + 1),
                    kind: EqBandKind::Bell,
                    hz: 560 + index as u16 * 125,
                    gain_db: if spectral { gain_db } else { 6.0 },
                    q: 90,
                    channel: [
                        EqChannel::Stereo,
                        EqChannel::Left,
                        EqChannel::Right,
                        EqChannel::Mid,
                        EqChannel::Side,
                    ][index % 5],
                    dynamic: Some(DynamicEq {
                        threshold_db: -45.0,
                        amount_db: if spectral { amount_db } else { 6.0 },
                        ratio: 3.0,
                        spectral,
                    }),
                    ..EqBand::default()
                })
            })),
            ..AudioTreatment::default()
        };
        let before = PreparedTreatment::preview(settings(0.0, 6.0), source).unwrap();
        let after = PreparedTreatment::preview(settings(1.0, 7.0), source).unwrap();
        let slot = RetuneSlot::default();
        slot.enable();
        let mut live = LiveProcessor::new(
            before.processor(source).unwrap(),
            before.clone(),
            source,
            true,
        );
        let input = vec![40_000_000; 4096];
        let mut output = Vec::new();
        live.process(&input, &mut output, false);
        assert!(before.can_retune_eq(&after, source.sample_rate));
        assert!(slot.submit(after.clone()));
        live.retune(&slot, source, (input.len() / 2) as u64, false);
        assert!(live.previous.is_none());
        assert!(live.prepared().same_preview(&after));
        live.process(&input, &mut output, false);
        assert_eq!(output.len(), input.len());
        let zero_strength = PreparedTreatment::preview(
            AudioTreatment {
                eq_strength: Some(0.0),
                ..after.settings().clone()
            },
            source,
        )
        .unwrap();
        assert!(after.can_retune_eq(&zero_strength, source.sample_rate));
        assert!(slot.submit(zero_strength.clone()));
        live.retune(&slot, source, input.len() as u64, false);
        assert!(live.previous.is_none());
        assert!(live.prepared().same_preview(&zero_strength));
    }
}
