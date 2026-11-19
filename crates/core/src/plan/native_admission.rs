//! Bounded FIFO admission for the executor's private synchronous I/O phase.
//!
//! A queued request gets 500 ms without advancing toward the front. Only the
//! departure of a request ahead of it renews that allowance; later arrivals and
//! unrelated activity cannot extend its wait. With at most `max_waiters`
//! requests queued, admission remains bounded even when holders stop progressing.
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
    rank_idle: Option<Duration>,
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
            rank_idle: rank.map(|rank| now.saturating_duration_since(self.waiting[rank].advanced)),
        }
    }
    fn failure(&self, cause: Refusal, context: RefusalContext, now: Instant) -> Failure {
        use std::fmt::Write;
        let mut diagnostic = format!(
            "reason={}; rank={:?}; queued={}; active={}; rank_idle_us={:?}; holders=[",
            context.reason,
            context.rank,
            context.queued,
            context.active,
            context.rank_idle.map(|age| age.as_micros()),
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
        if let Some(holder) = state.holders.iter_mut().rev().find(|h| h.thread == thread) {
            holder.stage = stage;
            holder.stage_started = Instant::now();
        }
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
            assert!(failure.diagnostic.contains("rank_idle_us=Some(500000)"));
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
}
