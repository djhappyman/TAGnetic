//! Bounded FIFO admission for the executor's private synchronous I/O phase.
//!
//! A queued request gets 500 ms without completed work by an admitted
//! predecessor or advancement toward the front. Completed writes, copies,
//! durable states and acknowledgements renew that allowance; starting work,
//! later arrivals and unrelated activity do not. Queue size and each admitted
//! operation's size/target-bounded work remain finite. Total queue latency is
//! not fixed: slow operations may complete many steps before releasing a slot.
use crate::resources::AdmissionError;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Condvar, Mutex,
    },
    time::{Duration, Instant},
};

const STALL: Duration = Duration::from_millis(500);
const CANCEL_POLL: Duration = Duration::from_millis(2);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    Cancelled,
    Resource(AdmissionError),
}

#[derive(Debug)]
pub(super) struct Failure {
    pub(super) cause: Refusal,
    pub(super) diagnostic: String,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Stage {
    Work(&'static str),
    Media(crate::media_mutation::Event),
    Native(crate::model::native_patch::Event),
    Checkpoint(crate::media_mutation::Phase, bool),
}
impl Stage {
    fn completed_work(self) -> bool {
        use crate::{media_mutation::Event as Media, model::native_patch::Event as Native};
        matches!(
            self,
            Self::Media(Media::Synced(_) | Media::StateDurable(_) | Media::CopyFinished(_))
                | Self::Native(Native::Verified)
                | Self::Checkpoint(_, true)
        ) || matches!(
            self,
            Self::Media(Media::Copied(_, bytes)) | Self::Native(Native::Written(bytes)) if bytes > 0
        )
    }
}

struct Holder {
    slot: usize,
    thread: std::thread::ThreadId,
    started: Instant,
    stage_started: Instant,
    stage: Stage,
}

struct RefusalContext {
    reason: &'static str,
    rank: Option<usize>,
    queued: usize,
    active: usize,
    progress_idle: Option<Duration>,
}

struct Waiter {
    ticket: u64,
    advanced: Instant,
}

struct State {
    active: usize,
    next: u64,
    waiting: VecDeque<Waiter>,
    holders: Vec<Holder>,
}
impl State {
    fn context(&self, reason: &'static str, ticket: Option<u64>, now: Instant) -> RefusalContext {
        let rank = ticket.and_then(|ticket| self.waiting.iter().position(|w| w.ticket == ticket));
        RefusalContext {
            reason,
            rank,
            queued: self.waiting.len(),
            active: self.active,
            progress_idle: rank
                .map(|rank| now.saturating_duration_since(self.waiting[rank].advanced)),
        }
    }
    fn failure(&self, cause: Refusal, context: RefusalContext, now: Instant) -> Failure {
        use std::fmt::Write;
        let mut diagnostic = format!(
            "reason={}; rank={:?}; queued={}; active={}; progress_idle_us={:?}; holders=[",
            context.reason,
            context.rank,
            context.queued,
            context.active,
            context.progress_idle.map(|age| age.as_micros()),
        );
        for (index, holder) in self.holders.iter().enumerate() {
            if index != 0 {
                diagnostic.push_str(", ");
            }
            let _ = write!(
                diagnostic,
                "slot={}, age_us={}, stage_age_us={}, stage=",
                holder.slot,
                now.saturating_duration_since(holder.started).as_micros(),
                now.saturating_duration_since(holder.stage_started)
                    .as_micros(),
            );
            // Read the enum fields explicitly, retaining bounded source-defined
            // labels and no media paths or contents in these diagnostics.
            let _ = match holder.stage {
                Stage::Work(name) => write!(diagnostic, "Work({name})"),
                Stage::Media(event) => write!(diagnostic, "Media({event:?})"),
                Stage::Native(event) => write!(diagnostic, "Native({event:?})"),
                Stage::Checkpoint(phase, done) => {
                    write!(diagnostic, "Checkpoint({phase:?}, {done})")
                }
            };
        }
        diagnostic.push(']');
        Failure { cause, diagnostic }
    }
    fn holder(&mut self, now: Instant) -> usize {
        // Active slots are reused only after their permit is dropped. Queue
        // ticket resets therefore cannot confuse diagnostic ownership.
        let mut slot = 0;
        while self.holders.iter().any(|holder| holder.slot == slot) {
            slot += 1;
        }
        self.holders.push(Holder {
            slot,
            thread: std::thread::current().id(),
            started: now,
            stage_started: now,
            stage: Stage::Work("admitted"),
        });
        slot
    }
    fn observe(&mut self, thread: std::thread::ThreadId, stage: Stage, now: Instant) -> bool {
        if let Some(holder) = self.holders.iter_mut().rev().find(|h| h.thread == thread) {
            holder.stage = stage;
            holder.stage_started = now;
            if stage.completed_work() {
                // FIFO ensures every active holder precedes every queued
                // request. Only actual completed work renews their allowance.
                for waiter in &mut self.waiting {
                    waiter.advanced = now;
                }
                return true;
            }
        }
        false
    }
    fn enqueue(&mut self, maximum: usize, now: Instant) -> Result<u64, Refusal> {
        if self.waiting.len() >= maximum {
            return Err(Refusal::Resource(AdmissionError::Busy));
        }
        let ticket = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or(Refusal::Resource(AdmissionError::TooLarge))?;
        self.waiting.push_back(Waiter {
            ticket,
            advanced: now,
        });
        Ok(ticket)
    }
    fn remove(&mut self, position: usize, now: Instant) {
        self.waiting.remove(position);
        for waiter in self.waiting.iter_mut().skip(position) {
            waiter.advanced = now;
        }
        // There are no outstanding tickets once the queue empties. Active
        // permits do not carry tickets, so resetting avoids eventual overflow.
        if self.waiting.is_empty() {
            self.next = 0;
        }
    }
    fn poll(
        &mut self,
        ticket: u64,
        capacity: usize,
        cancelled: bool,
        now: Instant,
    ) -> Result<bool, Refusal> {
        let position = self
            .waiting
            .iter()
            .position(|w| w.ticket == ticket)
            .ok_or(Refusal::Resource(AdmissionError::Busy))?;
        if cancelled {
            self.remove(position, now);
            return Err(Refusal::Cancelled);
        }
        if position == 0 && self.active < capacity {
            self.remove(position, now);
            self.active += 1;
            return Ok(true);
        }
        if now.saturating_duration_since(self.waiting[position].advanced) >= STALL {
            self.remove(position, now);
            return Err(Refusal::Resource(AdmissionError::Busy));
        }
        Ok(false)
    }
}

pub(super) struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}
impl Gate {
    pub(super) const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                active: 0,
                next: 0,
                waiting: VecDeque::new(),
                holders: Vec::new(),
            }),
            changed: Condvar::new(),
        }
    }
    pub(super) fn enter(
        &self,
        capacity: usize,
        maximum: usize,
        cancel: &AtomicBool,
    ) -> Result<Permit<'_>, Failure> {
        if cancel.load(Ordering::SeqCst) {
            return Err(Failure {
                cause: Refusal::Cancelled,
                diagnostic: "reason=cancelled before enqueue".into(),
            });
        }
        if capacity == 0 || maximum == 0 {
            return Err(Failure {
                cause: Refusal::Resource(AdmissionError::TooLarge),
                diagnostic: "reason=zero admission capacity or queue bound".into(),
            });
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        let enqueue_context = state.context(
            if state.waiting.len() >= maximum {
                "enqueue capacity full"
            } else {
                "enqueue ticket exhausted"
            },
            None,
            now,
        );
        let ticket = state
            .enqueue(maximum, now)
            .map_err(|cause| state.failure(cause, enqueue_context, now))?;
        self.changed.notify_all();
        loop {
            let cancelled = cancel.load(Ordering::SeqCst);
            let now = Instant::now();
            // Capture position and its age before poll removes a refused
            // waiter and advances the successors' observations.
            let mut context = state.context("stalled", Some(ticket), now);
            if context.rank.is_none() {
                context.reason = "queued ticket missing";
            } else if cancelled {
                context.reason = "queued request cancelled";
            }
            match state.poll(ticket, capacity, cancelled, now) {
                Ok(true) => {
                    let slot = state.holder(now);
                    self.changed.notify_all();
                    return Ok(Permit(self, slot));
                }
                Err(error) => {
                    self.changed.notify_all();
                    return Err(state.failure(error, context, now));
                }
                Ok(false) => {
                    let (next, _) = self
                        .changed
                        .wait_timeout(state, CANCEL_POLL)
                        .unwrap_or_else(|p| p.into_inner());
                    state = next;
                }
            }
        }
    }
    pub(super) fn stage(&self, stage: Stage) {
        let thread = std::thread::current().id();
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.observe(thread, stage, Instant::now());
        // Progress cannot free a slot. Avoid waking every queued request on
        // each copied buffer; admission/departure already notifies waiters.
    }
}
pub(super) struct Permit<'a>(&'a Gate, usize);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(at) = state
            .holders
            .iter()
            .position(|holder| holder.slot == self.1)
        {
            state.holders.remove(at);
        }
        state.active -= 1;
        self.0.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        State {
            active: 1,
            next: 0,
            waiting: VecDeque::new(),
            holders: Vec::new(),
        }
    }
    #[test]
    fn queued_requests_cannot_be_overtaken_by_new_arrivals() {
        let now = Instant::now();
        let mut q = state();
        let old = q.enqueue(8, now).unwrap();
        q.active = 0;
        let new = q.enqueue(8, now).unwrap();
        assert_eq!(q.poll(new, 1, false, now), Ok(false));
        assert_eq!(q.poll(old, 1, false, now), Ok(true));
        assert_eq!(q.poll(new, 1, false, now), Ok(false));
        q.active = 0;
        assert_eq!(q.poll(new, 1, false, now), Ok(true));
    }
    #[test]
    fn advancing_queue_gets_a_turn_beyond_the_original_arrival_deadline() {
        let now = Instant::now();
        let mut q = state();
        let tickets: Vec<_> = (0..3).map(|_| q.enqueue(8, now).unwrap()).collect();
        for (i, ticket) in tickets.into_iter().enumerate() {
            let turn = now + Duration::from_millis(400 * (i as u64 + 1));
            assert_eq!(
                q.poll(ticket, 1, false, turn - Duration::from_millis(50)),
                Ok(false),
                "progress ahead must preserve a queued request beyond 500 ms total"
            );
            q.active = 0;
            assert_eq!(q.poll(ticket, 1, false, turn), Ok(true));
        }
        assert!(q.waiting.is_empty());
    }
    #[test]
    fn arrivals_and_departures_behind_a_waiter_cannot_postpone_its_stall_deadline() {
        let now = Instant::now();
        let mut q = state();
        let first = q.enqueue(8, now).unwrap();
        let late = q.enqueue(8, now + Duration::from_millis(450)).unwrap();
        assert_eq!(
            q.poll(late, 1, true, now + Duration::from_millis(490)),
            Err(Refusal::Cancelled)
        );
        assert_eq!(
            q.poll(first, 1, false, now + STALL),
            Err(Refusal::Resource(AdmissionError::Busy))
        );
        assert!(q.waiting.is_empty());
    }
    #[test]
    fn cancelled_or_expired_heads_release_successors_and_queue_memory_is_bounded() {
        for cancelled in [false, true] {
            let now = Instant::now();
            let mut q = state();
            let first = q.enqueue(2, now).unwrap();
            let next = q.enqueue(2, now).unwrap();
            assert_eq!(
                q.enqueue(2, now),
                Err(Refusal::Resource(AdmissionError::Busy))
            );
            let expected = if cancelled {
                Refusal::Cancelled
            } else {
                Refusal::Resource(AdmissionError::Busy)
            };
            assert_eq!(q.poll(first, 1, cancelled, now + STALL), Err(expected));
            assert_eq!(q.poll(next, 1, false, now + STALL), Ok(false));
            q.active = 0;
            assert_eq!(q.poll(next, 1, false, now + STALL), Ok(true));
            assert!(q.waiting.is_empty());
        }
    }
    #[test]
    fn waiting_cancellation_and_unwind_release_queue_and_permit() {
        let gate = Gate::new();
        let cancel = AtomicBool::new(false);
        let holder = gate.enter(1, 8, &cancel).unwrap();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                matches!(
                    gate.enter(1, 8, &cancel),
                    Err(Failure {
                        cause: Refusal::Cancelled,
                        ..
                    })
                )
            });
            let end = Instant::now() + Duration::from_secs(2);
            let mut state = gate.state.lock().unwrap();
            while state.waiting.is_empty() && Instant::now() < end {
                state = gate.changed.wait_timeout(state, CANCEL_POLL).unwrap().0;
            }
            assert_eq!(state.waiting.len(), 1);
            cancel.store(true, Ordering::SeqCst);
            gate.changed.notify_all();
            drop(state);
            assert!(worker.join().unwrap());
        });
        drop(holder);
        cancel.store(false, Ordering::SeqCst);
        let result = std::panic::catch_unwind(|| {
            let _held = gate.enter(1, 8, &cancel).unwrap();
            panic!("controlled permit unwind");
        });
        assert!(result.is_err());
        assert!(gate.enter(1, 8, &cancel).is_ok());
        assert!(gate.state.lock().unwrap().waiting.is_empty());
    }
    #[test]
    fn refusal_diagnostics_capture_pre_removal_rank_and_active_stage_without_extending_wait() {
        let gate = Gate::new();
        let cancel = AtomicBool::new(false);
        let holder = gate.enter(1, 1, &cancel).unwrap();
        gate.stage(Stage::Work("controlled sync"));
        {
            let now = Instant::now();
            let mut state = gate.state.lock().unwrap();
            let ticket = state.enqueue(1, now).unwrap();
            let at = now + STALL;
            let context = state.context("stalled", Some(ticket), at);
            let cause = state.poll(ticket, 1, false, at).unwrap_err();
            let failure = state.failure(cause, context, at);
            assert_eq!(failure.cause, Refusal::Resource(AdmissionError::Busy));
            assert!(failure
                .diagnostic
                .contains("reason=stalled; rank=Some(0); queued=1; active=1"));
            assert!(failure.diagnostic.contains("progress_idle_us=Some(500000)"));
            assert!(failure.diagnostic.contains("stage=Work(controlled sync)"));
            assert!(state.waiting.is_empty());
            assert_eq!(state.holders.len(), 1);
            state.enqueue(1, now).unwrap();
        }
        let failure = match gate.enter(1, 1, &cancel) {
            Err(failure) => failure,
            Ok(_) => panic!("a full diagnostic queue must refuse"),
        };
        assert_eq!(failure.cause, Refusal::Resource(AdmissionError::Busy));
        assert!(failure.diagnostic.contains("reason=enqueue capacity full"));
        {
            let mut state = gate.state.lock().unwrap();
            let ticket = state.waiting[0].ticket;
            assert_eq!(
                state.poll(ticket, 1, true, Instant::now()),
                Err(Refusal::Cancelled)
            );
        }
        drop(holder);
        let state = gate.state.lock().unwrap();
        assert_eq!(state.active, 0);
        assert!(state.holders.is_empty());
        assert!(state.waiting.is_empty());
    }
    #[test]
    fn completed_predecessor_work_renews_stall_beyond_arrival_but_not_after_progress_stops() {
        let now = Instant::now();
        let mut state = state();
        state.holder(now);
        let ticket = state.enqueue(8, now).unwrap();
        let thread = std::thread::current().id();
        for millis in [400, 800] {
            assert!(state.observe(
                thread,
                Stage::Media(crate::media_mutation::Event::Synced(
                    crate::media_mutation::SyncKind::Directory,
                )),
                now + Duration::from_millis(millis),
            ));
            assert_eq!(
                state.poll(ticket, 1, false, now + Duration::from_millis(millis + 100)),
                Ok(false)
            );
        }
        assert_eq!(
            state.poll(ticket, 1, false, now + Duration::from_millis(1300)),
            Err(Refusal::Resource(AdmissionError::Busy))
        );
        assert!(state.waiting.is_empty());
    }
    #[test]
    fn only_positive_completed_events_from_an_active_holder_renew_waiters() {
        use crate::media_mutation::{CopyKind, Event, Phase, SyncKind};
        use crate::model::native_patch::Event as Native;
        let now = Instant::now();
        let thread = std::thread::current().id();
        let mut state = state();
        let ticket = state.enqueue(8, now).unwrap();
        let completed = [
            Stage::Media(Event::Synced(SyncKind::Directory)),
            Stage::Media(Event::StateDurable(Phase::Prepared)),
            Stage::Media(Event::Copied(CopyKind::Preimage, 1)),
            Stage::Media(Event::CopyFinished(CopyKind::Preimage)),
            Stage::Native(Native::Written(1)),
            Stage::Native(Native::Verified),
            Stage::Checkpoint(Phase::Prepared, true),
        ];
        for stage in completed {
            assert!(stage.completed_work());
            assert!(!state.observe(thread, stage, now + Duration::from_millis(450)));
            assert_eq!(state.waiting[0].advanced, now);
        }
        state.holder(now);
        for stage in [
            Stage::Work("starting work"),
            Stage::Media(Event::BeforeSync(SyncKind::Directory)),
            Stage::Media(Event::CopyStarted(CopyKind::Preimage)),
            Stage::Media(Event::Copied(CopyKind::Preimage, 0)),
            Stage::Media(Event::BeforeCallback(Phase::Prepared)),
            Stage::Native(Native::Written(0)),
            Stage::Native(Native::ReadChunk),
            Stage::Native(Native::Moved),
            Stage::Native(Native::Scan),
            Stage::Checkpoint(Phase::Prepared, false),
        ] {
            assert!(!stage.completed_work());
            assert!(!state.observe(thread, stage, now + Duration::from_millis(450)));
            assert_eq!(state.waiting[0].advanced, now);
        }
        assert_eq!(
            state.poll(ticket, 1, false, now + STALL),
            Err(Refusal::Resource(AdmissionError::Busy))
        );
    }
    #[test]
    fn cancellation_still_wins_after_completed_work_and_at_stall_deadline() {
        let now = Instant::now();
        let mut state = state();
        state.holder(now);
        let ticket = state.enqueue(8, now).unwrap();
        assert!(state.observe(
            std::thread::current().id(),
            Stage::Checkpoint(crate::media_mutation::Phase::Prepared, true),
            now + Duration::from_millis(400),
        ));
        assert_eq!(
            state.poll(ticket, 1, true, now + Duration::from_millis(900)),
            Err(Refusal::Cancelled)
        );
        assert!(state.waiting.is_empty());
    }
}

// BEGIN completed-work regression
#[cfg(test)]
mod progress_regression {
    use super::*;

    #[test]
    fn completed_work_keeps_a_queued_request_until_its_turn() {
        let gate = Gate::new();
        let cancel = AtomicBool::new(false);
        std::thread::scope(|scope| {
            // This permit is owned inside the scope closure so unwinding drops
            // it before the scoped worker is joined, even if an assertion fails.
            let holder = gate.enter(1, 8, &cancel).unwrap();
            let (finished, result) = std::sync::mpsc::channel();
            let worker_gate = &gate;
            let worker_cancel = &cancel;
            let worker = scope.spawn(move || {
                let outcome = worker_gate
                    .enter(1, 8, worker_cancel)
                    .map(drop)
                    .map_err(|failure| format!("{:?}: {}", failure.cause, failure.diagnostic));
                let _ = finished.send(outcome);
            });
            let end = Instant::now() + Duration::from_secs(2);
            let mut state = gate.state.lock().unwrap();
            while state.waiting.is_empty() && Instant::now() < end {
                state = gate.changed.wait_timeout(state, CANCEL_POLL).unwrap().0;
            }
            let arrived = state.waiting.len() == 1;
            drop(state);
            let mut early = None;
            if arrived {
                for _ in 0..5 {
                    std::thread::sleep(Duration::from_millis(150));
                    gate.stage(Stage::Media(crate::media_mutation::Event::Synced(
                        crate::media_mutation::SyncKind::Directory,
                    )));
                    if let Ok(outcome) = result.try_recv() {
                        early = Some(outcome);
                        break;
                    }
                }
            }
            drop(holder);
            let returned_early = early.is_some();
            let outcome = early.unwrap_or_else(|| {
                result
                    .recv_timeout(Duration::from_secs(2))
                    .expect("queued worker must finish after holder release")
            });
            cancel.store(true, Ordering::SeqCst);
            gate.changed.notify_all();
            worker.join().unwrap();
            assert!(
                arrived,
                "queued request must reach the controlled rendezvous"
            );
            assert!(
                outcome.is_ok(),
                "completed work did not preserve the queued request: {outcome:?}"
            );
            assert!(!returned_early, "queued work entered before holder release");
            let state = gate.state.lock().unwrap();
            assert_eq!(state.active, 0);
            assert!(state.waiting.is_empty());
            assert!(state.holders.is_empty());
        });
    }
}
