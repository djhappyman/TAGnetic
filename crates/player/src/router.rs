//! One permanent mixer source with one replaceable pending sink. Rodio's mixer
//! channel is unbounded, so connecting a new source for every Play/Stop would
//! retain all old sinks while an output device is not pulling samples.
use rodio::{queue::SourcesQueueOutput, source::UniformSourceIterator, Source};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering},
    Arc, Mutex, TryLockError,
};
use std::time::Duration;
type Output = UniformSourceIterator<SourcesQueueOutput>;
struct Handoff {
    align: Box<dyn FnOnce() -> bool + Send>,
    finish: Box<dyn FnOnce() + Send>,
}

struct Replacement {
    output: Output,
    handoff: Option<Handoff>,
}

struct Shared {
    pending: Mutex<Option<Replacement>>,
    changed: AtomicBool,
    channels: rodio::ChannelCount,
    rate: rodio::SampleRate,
    studio: Option<StudioOutput>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum StudioMonitorChannel {
    #[default]
    Stereo,
    Left,
    Right,
}

#[derive(Default)]
struct StudioOutput {
    monitor: AtomicU8,
    peaks: [AtomicU32; 2],
}
#[derive(Clone)]
pub(super) struct Handle(Arc<Shared>);
impl Handle {
    pub fn format(&self) -> (rodio::ChannelCount, rodio::SampleRate) {
        (self.0.channels, self.0.rate)
    }

    pub fn studio_monitor(&self, channel: StudioMonitorChannel) {
        if let Some(studio) = &self.0.studio {
            studio.monitor.store(channel as u8, Ordering::Relaxed);
        }
    }

    pub fn studio_peaks(&self) -> Option<[f32; 2]> {
        let output = self.0.studio.as_ref()?;
        Some(std::array::from_fn(|index| {
            f32::from_bits(output.peaks[index].swap(0, Ordering::Relaxed))
        }))
    }
    pub fn replace(&self, output: SourcesQueueOutput) {
        self.queue(output, None);
    }

    /// Let a prepared replacement align to the outgoing source's final
    /// position before installing it. A stale candidate leaves it playing.
    pub fn replace_if(
        &self,
        output: SourcesQueueOutput,
        align: impl FnOnce() -> bool + Send + 'static,
        finish: impl FnOnce() + Send + 'static,
    ) {
        self.queue(
            output,
            Some(Handoff {
                align: Box::new(align),
                finish: Box::new(finish),
            }),
        );
    }

    fn queue(&self, output: SourcesQueueOutput, handoff: Option<Handoff>) {
        let output = UniformSourceIterator::new(output, self.0.channels, self.0.rate);
        let previous = self
            .0
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .replace(Replacement { output, handoff });
        self.0.changed.store(true, Ordering::Release);
        // Source/decoder destruction is outside the control lock.
        drop(previous);
    }
}
pub(super) struct Router {
    shared: Arc<Shared>,
    current: Option<Output>,
    fade: Option<Fade>,
    frame_remaining: usize,
    monitor_gains: [f32; 2],
    meter_peaks: [f32; 2],
    meter_frames: u32,
}

struct Fade {
    outgoing: Output,
    remaining: u32,
    length: u32,
    finish: Box<dyn FnOnce() + Send>,
}
impl Router {
    pub fn new(channels: rodio::ChannelCount, rate: rodio::SampleRate) -> (Handle, Self) {
        Self::with_studio(channels, rate, None)
    }

    pub fn new_studio(rate: rodio::SampleRate) -> (Handle, Self) {
        Self::with_studio(
            rodio::ChannelCount::new(2).unwrap(),
            rate,
            Some(StudioOutput::default()),
        )
    }

    fn with_studio(
        channels: rodio::ChannelCount,
        rate: rodio::SampleRate,
        studio: Option<StudioOutput>,
    ) -> (Handle, Self) {
        let shared = Arc::new(Shared {
            pending: Mutex::new(None),
            changed: AtomicBool::new(false),
            channels,
            rate,
            studio,
        });
        (
            Handle(shared.clone()),
            Self {
                shared,
                current: None,
                fade: None,
                frame_remaining: 0,
                monitor_gains: [1.0; 2],
                meter_peaks: [0.0; 2],
                meter_frames: 0,
            },
        )
    }
}
impl Iterator for Router {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        // A new sink starts on a complete output frame, preserving channel order.
        if self.frame_remaining == 0 && self.shared.changed.load(Ordering::Acquire) {
            let next = match self.shared.pending.try_lock() {
                Ok(mut pending) => {
                    self.shared.changed.store(false, Ordering::Release);
                    pending.take()
                }
                Err(TryLockError::Poisoned(error)) => {
                    self.shared.changed.store(false, Ordering::Release);
                    error.into_inner().take()
                }
                Err(TryLockError::WouldBlock) => None,
            };
            if let Some(Replacement { output, handoff }) = next {
                if let Some(Handoff { align, finish }) = handoff {
                    if align() {
                        if let Some(outgoing) = self.current.replace(output) {
                            let length = (self.shared.rate.get() / 250).max(1);
                            self.fade = Some(Fade {
                                outgoing,
                                length,
                                remaining: length,
                                finish,
                            });
                        } else {
                            finish();
                        }
                    }
                } else {
                    self.current = Some(output);
                    self.fade = None;
                    self.meter_peaks = [0.0; 2];
                    self.meter_frames = 0;
                    if let Some(studio) = &self.shared.studio {
                        for peak in &studio.peaks {
                            peak.store(0, Ordering::Relaxed);
                        }
                        self.monitor_gains = monitor_gains(studio);
                    }
                }
            }
        }
        if self.frame_remaining == 0 {
            self.frame_remaining = usize::from(self.shared.channels.get());
        }
        self.frame_remaining -= 1;
        let mut sample = self
            .current
            .as_mut()
            .and_then(Iterator::next)
            .unwrap_or(0.0);
        if let Some(fade) = &mut self.fade {
            let mix = (fade.length - fade.remaining + 1) as f32 / fade.length as f32;
            let outgoing = fade.outgoing.next().unwrap_or(0.0);
            sample = outgoing * (1.0 - mix) + sample * mix;
            if self.frame_remaining == 0 {
                fade.remaining -= 1;
                if fade.remaining == 0 {
                    let fade = self.fade.take().expect("completed fade");
                    (fade.finish)();
                }
            }
        }
        if let Some(studio) = &self.shared.studio {
            let channel = 1 - self.frame_remaining;
            if channel == 0 {
                let step = 1.0 / ((self.shared.rate.get() / 250).max(1) as f32);
                for (gain, target) in self.monitor_gains.iter_mut().zip(monitor_gains(studio)) {
                    *gain += (target - *gain).clamp(-step, step);
                }
            }
            sample *= self.monitor_gains[channel];
            self.meter_peaks[channel] = self.meter_peaks[channel].max(sample.abs());
            if channel == 1 {
                self.meter_frames += 1;
                if self.meter_frames >= (self.shared.rate.get() / 250).max(1) {
                    for (peak, measured) in studio.peaks.iter().zip(self.meter_peaks) {
                        peak.fetch_max(measured.to_bits(), Ordering::Relaxed);
                    }
                    self.meter_peaks = [0.0; 2];
                    self.meter_frames = 0;
                }
            }
        }
        Some(sample)
    }
}

fn monitor_gains(studio: &StudioOutput) -> [f32; 2] {
    match studio.monitor.load(Ordering::Relaxed) {
        value if value == StudioMonitorChannel::Left as u8 => [1.0, 0.0],
        value if value == StudioMonitorChannel::Right as u8 => [0.0, 1.0],
        _ => [1.0, 1.0],
    }
}
impl Source for Router {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.shared.channels
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.shared.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    struct Tracked(Arc<AtomicUsize>);
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl Iterator for Tracked {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            Some(0.25)
        }
    }
    impl Source for Tracked {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> rodio::ChannelCount {
            rodio::ChannelCount::new(2).unwrap()
        }
        fn sample_rate(&self) -> rodio::SampleRate {
            rodio::SampleRate::new(44_100).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    #[test]
    fn studio_meter_reads_output_with_volume_and_monitor_gain_applied() {
        let rate = rodio::SampleRate::new(8_000).unwrap();
        let (handle, mut router) = Router::new_studio(rate);
        let (channels, _) = handle.format();
        let (player, output) = rodio::Player::new();
        player.set_volume(0.5);
        player.append(rodio::buffer::SamplesBuffer::new(
            channels,
            rate,
            [0.75, -0.5].repeat(256),
        ));
        handle.replace(output);
        for _ in 0..128 {
            router.next();
        }
        assert_eq!(handle.studio_peaks(), Some([0.375, 0.25]));
        assert_eq!(handle.studio_peaks(), Some([0.0, 0.0]));

        handle.studio_monitor(StudioMonitorChannel::Right);
        for _ in 0..128 {
            router.next();
        }
        handle.studio_peaks();
        assert_eq!(router.next(), Some(0.0));
        assert_eq!(router.next(), Some(-0.25));
        for _ in 0..126 {
            router.next();
        }
        assert_eq!(handle.studio_peaks(), Some([0.0, 0.25]));
    }

    #[test]
    fn stale_replacement_leaves_the_current_output_at_the_frame_boundary() {
        let channels = rodio::ChannelCount::new(2).unwrap();
        let rate = rodio::SampleRate::new(44_100).unwrap();
        let (handle, mut router) = Router::new(channels, rate);
        let source = |sample| {
            let (player, output) = rodio::Player::new();
            player.append(rodio::buffer::SamplesBuffer::new(
                channels,
                rate,
                vec![sample; 8],
            ));
            (player, output)
        };
        let (_current, output) = source(0.25);
        handle.replace(output);
        assert_eq!(router.next(), Some(0.25));

        let (_stale, output) = source(0.75);
        let checks = Arc::new(AtomicUsize::new(0));
        let checks_at_swap = checks.clone();
        handle.replace_if(
            output,
            move || {
                checks_at_swap.fetch_add(1, Ordering::SeqCst);
                false
            },
            || panic!("a stale replacement must never finish"),
        );
        assert_eq!(router.next(), Some(0.25));
        assert_eq!(checks.load(Ordering::SeqCst), 0);
        assert_eq!(router.next(), Some(0.25));
        assert_eq!(checks.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unconsumed_dependency_mixer_retains_every_fresh_sink_positive_control() {
        let live = Arc::new(AtomicUsize::new(0));
        let (mixer, output) = rodio::mixer::mixer(
            rodio::ChannelCount::new(2).unwrap(),
            rodio::SampleRate::new(44_100).unwrap(),
        );
        for _ in 0..128 {
            let player = rodio::Player::connect_new(&mixer);
            live.fetch_add(1, Ordering::SeqCst);
            player.append(Tracked(live.clone()));
            drop(player);
        }
        // This is the old production shape, with no device consumption. It
        // deliberately demonstrates the dependency retains all queued sinks.
        assert_eq!(live.load(Ordering::SeqCst), 128);
        drop(mixer);
        drop(output);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stalled_output_retains_only_one_pending_sink_and_one_active_sink() {
        let live = Arc::new(AtomicUsize::new(0));
        let channels = rodio::ChannelCount::new(2).unwrap();
        let rate = rodio::SampleRate::new(44_100).unwrap();
        let (handle, mut router) = Router::new(channels, rate);
        for i in 0..20_000 {
            let (player, output) = rodio::Player::new();
            live.fetch_add(1, Ordering::SeqCst);
            player.append(Tracked(live.clone()));
            handle.replace(output);
            assert!(live.load(Ordering::SeqCst) <= if i == 0 { 1 } else { 2 });
            if i == 0 {
                // Bootstrap the first sink; later replacements happen while the
                // device is not pulling anything, with this source still live.
                for _ in 0..2_000 {
                    router.next();
                }
            }
            drop(player);
        }
        assert_eq!(live.load(Ordering::SeqCst), 2);
        drop(router);
        drop(handle);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }
}
