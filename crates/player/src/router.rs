//! One permanent mixer source with one replaceable pending sink. Rodio's mixer
//! channel is unbounded, so connecting a new source for every Play/Stop would
//! retain all old sinks while an output device is not pulling samples.
use rodio::{queue::SourcesQueueOutput, source::UniformSourceIterator, Source};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, TryLockError,
};
use std::time::Duration;
type Output = UniformSourceIterator<SourcesQueueOutput>;
type Handoff = Box<dyn FnOnce() -> bool + Send>;

struct Replacement {
    output: Output,
    handoff: Option<Handoff>,
}

struct Shared {
    pending: Mutex<Option<Replacement>>,
    changed: AtomicBool,
    channels: rodio::ChannelCount,
    rate: rodio::SampleRate,
}
#[derive(Clone)]
pub(super) struct Handle(Arc<Shared>);
impl Handle {
    pub fn format(&self) -> (rodio::ChannelCount, rodio::SampleRate) {
        (self.0.channels, self.0.rate)
    }
    pub fn replace(&self, output: SourcesQueueOutput) {
        self.queue(output, None);
    }

    /// Let a prepared replacement align to the outgoing source's final
    /// position before installing it. A stale candidate leaves it playing.
    pub fn replace_if(
        &self,
        output: SourcesQueueOutput,
        handoff: impl FnOnce() -> bool + Send + 'static,
    ) {
        self.queue(output, Some(Box::new(handoff)));
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
    frame_remaining: usize,
}
impl Router {
    pub fn new(channels: rodio::ChannelCount, rate: rodio::SampleRate) -> (Handle, Self) {
        let shared = Arc::new(Shared {
            pending: Mutex::new(None),
            changed: AtomicBool::new(false),
            channels,
            rate,
        });
        (
            Handle(shared.clone()),
            Self {
                shared,
                current: None,
                frame_remaining: 0,
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
                if handoff.is_none_or(|handoff| handoff()) {
                    let previous = self.current.replace(output);
                    drop(previous);
                }
            }
        }
        if self.frame_remaining == 0 {
            self.frame_remaining = usize::from(self.shared.channels.get());
        }
        self.frame_remaining -= 1;
        Some(
            self.current
                .as_mut()
                .and_then(Iterator::next)
                .unwrap_or(0.0),
        )
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
        handle.replace_if(output, move || {
            checks_at_swap.fetch_add(1, Ordering::SeqCst);
            false
        });
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
