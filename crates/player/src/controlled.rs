//! One bounded seek slot per decoded source. The actor submits requests and
//! polls acknowledgements; it never waits for the audio device to pull samples.
//! A request belongs to this source, never the sink's next queued track.
use rodio::Source;
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, Weak,
};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(super) struct Ticket {
    state: Arc<Mutex<State>>,
    owner: Weak<Ownership>,
}
struct State {
    target: Duration,
    cancelled: bool,
    superseded: bool,
    result: Option<Result<(Duration, Instant), String>>,
}
/// The source owns the lifetime of every decoder seek it has entered, even
/// after the actor replaces its visible ticket. There is one latest weak ticket,
/// one pending slot and at most one decoder call on the existing consumer.
struct Ownership {
    entered_seek: AtomicBool,
    moved_since_accept: AtomicBool,
    abort: AtomicBool,
    latest: Mutex<Option<Weak<Mutex<State>>>>,
}
impl Ticket {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new(target: Duration) -> Self {
        Self::owned(target, Weak::new())
    }
    fn owned(target: Duration, owner: Weak<Ownership>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                target,
                cancelled: false,
                superseded: false,
                result: None,
            })),
            owner,
        }
    }
    /// Terminal cancellation invalidates decoder work already entered by any
    /// earlier request on this source. Ordinary supersession is different.
    pub fn cancel(&self) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cancelled = true;
        if let Some(owner) = self.owner.upgrade() {
            if owner.entered_seek.load(Ordering::Acquire) {
                owner.abort.store(true, Ordering::Release);
            }
        }
    }
    #[cfg_attr(test, allow(dead_code))]
    pub fn supersede(&self) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .superseded = true;
    }
    /// Only the actor accepts a completed current request. Until then terminal
    /// cancellation must still invalidate a move whose clock was not adopted.
    #[cfg_attr(test, allow(dead_code))]
    pub fn accept(&self) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let latest = owner.latest.lock().unwrap_or_else(|p| p.into_inner());
        let is_latest = latest
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|state| Arc::ptr_eq(&state, &self.state));
        if is_latest {
            let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            if !state.cancelled && !state.superseded && state.result.is_some() {
                owner.entered_seek.store(false, Ordering::Release);
                owner.moved_since_accept.store(false, Ordering::Release);
            }
        }
    }
    pub fn poll(&self) -> Option<Result<(Duration, Instant), String>> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.cancelled || state.superseded {
            None
        } else {
            state.result.clone()
        }
    }
    pub fn finish(&self, result: Result<(), String>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let target = state.target;
        if !state.cancelled && !state.superseded {
            state.result = Some(result.map(|()| (target, Instant::now())));
            true
        } else {
            false
        }
    }
    fn begin(&self) -> Option<Duration> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.cancelled || state.superseded {
            return None;
        }
        if let Some(owner) = self.owner.upgrade() {
            owner.entered_seek.store(true, Ordering::Release);
        }
        Some(state.target)
    }
    fn cancelled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cancelled
    }
    fn superseded(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .superseded
    }
}

#[derive(Clone)]
pub(super) struct Control {
    pending: Arc<Mutex<Option<Ticket>>>,
    ended: Arc<AtomicBool>,
    requested: Arc<AtomicBool>,
    ownership: Arc<Ownership>,
}
impl Control {
    pub fn seek(&self, target: Duration) -> Ticket {
        let ticket = Ticket::owned(target, Arc::downgrade(&self.ownership));
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        if self.ended.load(Ordering::Acquire) || self.ownership.abort.load(Ordering::Acquire) {
            ticket.finish(Err("The requested track has ended".into()));
            return ticket;
        }
        let mut latest = self
            .ownership
            .latest
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(previous) = latest
            .replace(Arc::downgrade(&ticket.state))
            .and_then(|ticket| ticket.upgrade())
        {
            previous
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .superseded = true;
        }
        *pending = Some(ticket.clone());
        self.requested.store(true, Ordering::Release);
        ticket
    }
}

/// Metadata exposed to a downstream converter belongs to one coherent native
/// span. Pause/seek requests cannot change that snapshot between getter calls.
struct Playback<S> {
    source: S,
    metadata: Option<Span>,
    advertised: bool,
    frame_remaining: usize,
    frame_paused: bool,
    ended: bool,
    uncommitted_move: bool,
}
#[derive(Clone, Copy)]
struct Span {
    channels: rodio::ChannelCount,
    rate: rodio::SampleRate,
    remaining: Option<usize>,
}
/// Implemented only by the consumer-owned conversion adapter whose seek
/// rebuilds its buffers and preserves its declared physical output format.
#[cfg_attr(test, allow(dead_code))]
pub(super) trait FixedFormat: Source {}

pub(super) struct Controlled<S, const FIXED: bool = true> {
    state: RefCell<Playback<S>>,
    control: Control,
    paused: Arc<AtomicBool>,
}
impl<S: Source> Controlled<S, false> {
    #[cfg(test)]
    pub fn new(source: S, paused: Arc<AtomicBool>) -> (Self, Control) {
        Self::with_format(source, paused)
    }
}
impl<S: Source, const FIXED: bool> Controlled<S, FIXED> {
    fn with_format(source: S, paused: Arc<AtomicBool>) -> (Self, Control) {
        let control = Control {
            pending: Arc::new(Mutex::new(None)),
            ended: Arc::new(AtomicBool::new(false)),
            requested: Arc::new(AtomicBool::new(false)),
            ownership: Arc::new(Ownership {
                entered_seek: AtomicBool::new(false),
                moved_since_accept: AtomicBool::new(false),
                abort: AtomicBool::new(false),
                latest: Mutex::new(None),
            }),
        };
        (
            Self {
                state: RefCell::new(Playback {
                    source,
                    metadata: None,
                    advertised: false,
                    frame_remaining: 0,
                    frame_paused: false,
                    ended: false,
                    uncommitted_move: false,
                }),
                control: control.clone(),
                paused,
            },
            control,
        )
    }
    fn take_request(control: &Control) -> Option<Ticket> {
        if !control.requested.load(Ordering::Acquire)
            || !control.requested.swap(false, Ordering::AcqRel)
        {
            return None;
        }
        control
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }
    #[inline]
    fn prepare(control: &Control, state: &mut Playback<S>, advertise: bool) {
        state.advertised |= advertise;
        if control.ownership.abort.load(Ordering::Acquire) {
            state.ended = true;
            control.ended.store(true, Ordering::Release);
        }
        if state.ended {
            return;
        }
        let boundary = state.metadata.is_none_or(|span| span.remaining == Some(0));
        if state.frame_remaining != 0 && !boundary {
            return;
        }
        if FIXED
            && !advertise
            && !boundary
            && !state.uncommitted_move
            && !control.requested.load(Ordering::Acquire)
            && state.source.current_span_len().is_none()
        {
            // Fixed output keeps its advertised rate and layout across native
            // spans. The source still reports EOF even if output is paused.
            return;
        }
        Self::refresh_span(control, state, boundary);
    }

    fn refresh_span(control: &Control, state: &mut Playback<S>, boundary: bool) {
        if !FIXED && state.advertised && !boundary {
            // There is no safe reconfiguration point inside an advertised
            // unknown-length span. Refuse explicitly rather than seeking under
            // a converter that will never refresh its old format metadata.
            if state.metadata.is_some_and(|span| span.remaining.is_none()) {
                if let Some(ticket) = Self::take_request(control) {
                    ticket.finish(Err(
                        "The source exposes no finite format boundary for safe seeking".into(),
                    ));
                }
            }
            return;
        }
        // Process at most two decoder calls per turn. Continuous supersession
        // can keep one newer request pending, but never builds a work backlog or
        // emits a superseded target. Between turns an unresolved move is silent.
        for _ in 0..2 {
            let Some(ticket) = Self::take_request(control) else {
                break;
            };
            let Some(target) = ticket.begin() else {
                if ticket.cancelled() && state.uncommitted_move {
                    state.ended = true;
                    control.ended.store(true, Ordering::Release);
                }
                continue;
            };
            if ticket.cancelled() || control.ownership.abort.load(Ordering::Acquire) {
                state.ended = true;
                control.ended.store(true, Ordering::Release);
                break;
            }
            // No request/control lock is held across decoder I/O.
            let result = state.source.try_seek(target).map_err(|e| e.to_string());
            if ticket.cancelled() || control.ownership.abort.load(Ordering::Acquire) {
                state.ended = true;
                control.ended.store(true, Ordering::Release);
                break;
            }
            let moved = result.is_ok();
            if moved {
                control
                    .ownership
                    .moved_since_accept
                    .store(true, Ordering::Release);
            }
            if ticket.superseded() {
                state.uncommitted_move |= moved;
                continue;
            }
            let refused_after_move = result.is_err()
                && (state.uncommitted_move
                    || control.ownership.moved_since_accept.load(Ordering::Acquire));
            if moved {
                state.uncommitted_move = false;
            }
            if !ticket.finish(result) {
                state.uncommitted_move |= moved;
                continue;
            }
            if refused_after_move {
                // The latest decoder refused after an earlier superseded seek
                // moved it. There is no verified original position to restore.
                state.ended = true;
                control.ended.store(true, Ordering::Release);
                break;
            }
        }
        if state.ended {
            return;
        }
        let span = Span {
            channels: state.source.channels(),
            rate: state.source.sample_rate(),
            remaining: state.source.current_span_len(),
        };
        if span.remaining == Some(0) {
            state.ended = true;
            control.ended.store(true, Ordering::Release);
        }
        state.metadata = Some(span);
        state.frame_remaining = 0;
    }
}
impl<S: Source + FixedFormat> Controlled<S, true> {
    #[cfg_attr(test, allow(dead_code))]
    pub fn new_fixed(source: S, paused: Arc<AtomicBool>) -> (Self, Control) {
        Self::with_format(source, paused)
    }
}
impl<S, const FIXED: bool> Drop for Controlled<S, FIXED> {
    fn drop(&mut self) {
        self.control.ended.store(true, Ordering::Release);
        if let Some(ticket) = self
            .control
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            ticket.finish(Err("The requested track has ended".into()));
        }
    }
}
impl<S: Source, const FIXED: bool> Iterator for Controlled<S, FIXED> {
    type Item = rodio::Sample;
    fn next(&mut self) -> Option<Self::Item> {
        let state = self.state.get_mut();
        Self::prepare(&self.control, state, false);
        if state.ended {
            return None;
        }
        let span = state.metadata.expect("prepare installs source metadata");
        if state.frame_remaining == 0 {
            state.frame_paused = self.paused.load(Ordering::Acquire);
            state.frame_remaining = usize::from(span.channels.get());
        }
        state.frame_remaining -= 1;
        // Count emitted silence against the exposed span too. It can only
        // shorten a converter's cached span; it cannot let resumed real samples
        // overrun a native boundary using the previous format.
        if let Some(remaining) = state
            .metadata
            .as_mut()
            .and_then(|span| span.remaining.as_mut())
        {
            *remaining = remaining.saturating_sub(1);
        }
        let sample = if state.frame_paused || state.uncommitted_move {
            Some(0.0)
        } else {
            state.source.next()
        };
        if sample.is_none() {
            state.ended = true;
            self.control.ended.store(true, Ordering::Release);
        }
        sample
    }
}
impl<S: Source, const FIXED: bool> Source for Controlled<S, FIXED> {
    fn current_span_len(&self) -> Option<usize> {
        let mut state = self.state.borrow_mut();
        Self::prepare(&self.control, &mut state, true);
        if state.ended {
            Some(0)
        } else {
            state.metadata.and_then(|span| span.remaining)
        }
    }
    fn channels(&self) -> rodio::ChannelCount {
        let mut state = self.state.borrow_mut();
        Self::prepare(&self.control, &mut state, true);
        state
            .metadata
            .map_or_else(|| state.source.channels(), |span| span.channels)
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        let mut state = self.state.borrow_mut();
        Self::prepare(&self.control, &mut state, true);
        state
            .metadata
            .map_or_else(|| state.source.sample_rate(), |span| span.rate)
    }
    fn total_duration(&self) -> Option<Duration> {
        self.state.borrow().source.total_duration()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::source::SeekError;
    use std::sync::mpsc;

    struct Counting {
        sample: usize,
        seeks: Arc<Mutex<Vec<Duration>>>,
        blocked: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
        refuse: bool,
    }
    impl Iterator for Counting {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            let value = self.sample as f32;
            self.sample += 1;
            Some(value)
        }
    }
    impl Source for Counting {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> rodio::ChannelCount {
            rodio::ChannelCount::new(2).unwrap()
        }
        fn sample_rate(&self) -> rodio::SampleRate {
            rodio::SampleRate::new(2).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
        fn try_seek(&mut self, target: Duration) -> Result<(), SeekError> {
            self.seeks.lock().unwrap().push(target);
            if let Some((entered, release)) = self.blocked.take() {
                entered.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            if self.refuse {
                return Err(SeekError::NotSupported {
                    underlying_source: "test counting source",
                });
            }
            self.sample = (target.as_secs_f64() * 4.0) as usize;
            Ok(())
        }
    }
    type TestSource = (
        Controlled<Counting, false>,
        Control,
        Arc<AtomicBool>,
        Arc<Mutex<Vec<Duration>>>,
    );
    fn source(paused: bool) -> TestSource {
        let seeks = Arc::new(Mutex::new(Vec::new()));
        let pause = Arc::new(AtomicBool::new(paused));
        let (source, control) = Controlled::new(
            Counting {
                sample: 0,
                seeks: seeks.clone(),
                blocked: None,
                refuse: false,
            },
            pause.clone(),
        );
        (source, control, pause, seeks)
    }
    #[test]
    fn stalled_consumer_requests_are_bounded_and_only_latest_request_is_executed() {
        let (mut source, control, _, seeks) = source(false);
        let mut last = None;
        for i in 0..100_000 {
            let ticket = control.seek(Duration::from_millis(i));
            assert!(ticket.poll().is_none());
            last = Some(ticket);
        }
        assert!(seeks.lock().unwrap().is_empty());
        let sample = source.next().unwrap();
        assert_eq!(
            seeks.lock().unwrap().as_slice(),
            &[Duration::from_millis(99_999)]
        );
        assert_eq!(sample, 399.0);
        assert!(last.unwrap().poll().unwrap().is_ok());
    }
    #[test]
    fn paused_seek_acknowledges_without_consuming_or_emitting_audio_and_keeps_stereo_alignment() {
        let (mut source, control, paused, _) = source(false);
        assert_eq!(source.next(), Some(0.0));
        paused.store(true, Ordering::Release);
        // Pause at a half frame takes effect at the next complete frame.
        assert_eq!(source.next(), Some(1.0));
        let ticket = control.seek(Duration::from_secs(2));
        assert_eq!(source.next(), Some(0.0));
        assert!(ticket.poll().unwrap().is_ok());
        for _ in 0..999 {
            assert_eq!(source.next(), Some(0.0));
        }
        paused.store(false, Ordering::Release);
        assert_eq!(source.next(), Some(8.0));
        assert_eq!(source.next(), Some(9.0));
    }
    #[test]
    fn cancelling_an_unconsumed_seek_keeps_original_position_and_never_seeks_another_source() {
        let (mut first, first_control, _, first_seeks) = source(false);
        let (mut second, _, _, second_seeks) = source(false);
        let ticket = first_control.seek(Duration::from_secs(2));
        ticket.cancel();
        assert_eq!(first.next(), Some(0.0));
        assert_eq!(second.next(), Some(0.0));
        assert!(first_seeks.lock().unwrap().is_empty());
        assert!(second_seeks.lock().unwrap().is_empty());
        drop(first);
        assert!(first_control.seek(Duration::ZERO).poll().unwrap().is_err());
    }
    #[test]
    fn cancelling_a_decoder_seek_in_progress_does_not_wait_and_prevents_late_audio() {
        let (mut source, control, _, _) = source(false);
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        source.state.get_mut().source.blocked = Some((entered_tx, release_rx));
        let ticket = control.seek(Duration::from_secs(2));
        let consumer = std::thread::spawn(move || source.next());
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let start = Instant::now();
        ticket.cancel();
        assert!(start.elapsed() < Duration::from_millis(100));
        assert!(ticket.poll().is_none());
        release.send(()).unwrap();
        assert_eq!(consumer.join().unwrap(), None);
        assert!(ticket.poll().is_none());
    }
    #[test]
    fn refused_decoder_seek_is_not_a_success_and_does_not_move_audio() {
        let (mut source, control, _, _) = source(false);
        source.state.get_mut().source.refuse = true;
        let ticket = control.seek(Duration::from_secs(2));
        assert_eq!(source.next(), Some(0.0));
        assert!(ticket.poll().unwrap().is_err());
        assert_eq!(source.next(), Some(1.0));
    }
    #[test]
    fn pause_resume_stress_preserves_every_real_interleaved_sample() {
        let (mut source, _, paused, _) = source(false);
        let mut state = 0x7a91u32;
        let mut expected = 0usize;
        let mut frame_paused = false;
        for sample in 0..200_000 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let pause = state & 8 != 0;
            paused.store(pause, Ordering::Release);
            if sample % 2 == 0 {
                frame_paused = pause;
            }
            let actual = source.next().unwrap();
            if frame_paused {
                assert_eq!(actual, 0.0);
            } else {
                assert_eq!(actual, expected as f32);
                expected += 1;
            }
        }
        assert!(expected > 90_000);
    }
}
