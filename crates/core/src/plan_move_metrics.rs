//! Test-only exclusive phase accounting. This is not production telemetry or a
//! source of timing pass targets; nested phase durations are subtracted.
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug)]
pub(crate) enum Phase {
    Reservation,
    Capture,
    Publication,
    Proof,
    Cleanup,
}
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Timing {
    pub calls: usize,
    pub elapsed: Duration,
}
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Metrics {
    pub reservation: Timing,
    pub capture: Timing,
    pub publication: Timing,
    pub proof: Timing,
    pub cleanup: Timing,
}
#[cfg(target_os = "linux")]
impl Metrics {
    pub fn total(self) -> Duration {
        self.reservation.elapsed
            + self.capture.elapsed
            + self.publication.elapsed
            + self.proof.elapsed
            + self.cleanup.elapsed
    }
}
struct Frame {
    phase: Phase,
    start: Instant,
    children: Duration,
}
#[derive(Default)]
struct State {
    frames: Vec<Frame>,
    metrics: Metrics,
}
thread_local! {static STATE:RefCell<State>=RefCell::new(State::default());}
pub(crate) struct Timer;
impl Timer {
    pub fn new(phase: Phase) -> Self {
        STATE.with(|state| {
            state.borrow_mut().frames.push(Frame {
                phase,
                start: Instant::now(),
                children: Duration::ZERO,
            })
        });
        Self
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let frame = state.frames.pop().expect("balanced test timing scopes");
            let elapsed = frame.start.elapsed();
            let exclusive = elapsed.saturating_sub(frame.children);
            let timing = match frame.phase {
                Phase::Reservation => &mut state.metrics.reservation,
                Phase::Capture => &mut state.metrics.capture,
                Phase::Publication => &mut state.metrics.publication,
                Phase::Proof => &mut state.metrics.proof,
                Phase::Cleanup => &mut state.metrics.cleanup,
            };
            timing.calls += 1;
            timing.elapsed += exclusive;
            if let Some(parent) = state.frames.last_mut() {
                parent.children += elapsed;
            }
        });
    }
}
#[cfg(target_os = "linux")]
pub(crate) fn take() -> Metrics {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        assert!(state.frames.is_empty());
        std::mem::take(&mut state.metrics)
    })
}
