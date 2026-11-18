//! Shared, nonblocking admission for explicitly accounted backend resources.
//!
//! Owners reserve before allocating work buffers and hold a [`Lease`] as long
//! as their charged data lives. This is accounting for participating owners,
//! not an allocator or a promise about whole-process resident memory.

use std::sync::{Arc, LazyLock, Mutex};

/// Concurrent retained bytes, transient bytes and expensive jobs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Charge {
    pub retained: usize,
    pub transient: usize,
    pub jobs: usize,
}

/// Per-class ceilings and a combined byte ceiling.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub retained: usize,
    pub transient: usize,
    pub total: usize,
    pub jobs: usize,
}

/// Admission refusal is either permanent for this request or retryable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    #[error("The operation exceeds its shared resource budget")]
    TooLarge,
    #[error("Backend resources are busy; retry after current work completes")]
    Busy,
}

#[derive(Debug)]
struct State {
    limits: Limits,
    used: Charge,
    high_water: Charge,
}

/// A clonable coordinator. Clones share the same accounting state.
#[derive(Clone, Debug)]
pub struct Budget(Arc<Mutex<State>>);

/// Production participants share this coordinator across sessions and crates.
/// Defaults leave room for the existing 256 MiB maximum artwork work estimate;
/// they are conservative policy limits, not a measured optimal configuration.
static PROCESS: LazyLock<Budget> = LazyLock::new(|| {
    Budget::new(Limits {
        retained: 256 * 1024 * 1024,
        transient: 384 * 1024 * 1024,
        total: 512 * 1024 * 1024,
        jobs: 8,
    })
});

impl Budget {
    pub fn new(limits: Limits) -> Self {
        Self(Arc::new(Mutex::new(State {
            limits,
            used: Charge::default(),
            high_water: Charge::default(),
        })))
    }

    /// The process-wide coordinator for participating native operations.
    pub fn process() -> Self {
        PROCESS.clone()
    }

    /// Read-only ceilings for bounded compound-operation admission.
    #[cfg(target_os = "linux")]
    pub(crate) fn limits(&self) -> Limits {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).limits
    }

    /// Reserve atomically without waiting or creating a worker thread.
    pub fn try_acquire(&self, charge: Charge) -> Result<Lease, AdmissionError> {
        let mut lease = Lease {
            budget: self.clone(),
            charge: Charge::default(),
        };
        lease.try_resize(charge)?;
        Ok(lease)
    }

    /// Current and per-class peak charges. This contains no paths or tag data.
    pub fn usage(&self) -> (Charge, Charge) {
        let state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        (state.used, state.high_water)
    }
}

/// Exclusive ownership of one charge. Moving a lease transfers ownership;
/// dropping it releases accounting, including during unwinding.
#[derive(Debug)]
pub struct Lease {
    budget: Budget,
    charge: Charge,
}

impl Lease {
    /// Replace this owner's reservation atomically. Refusal leaves the old
    /// reservation intact, so callers can retain already allocated buffers.
    pub fn try_resize(&mut self, charge: Charge) -> Result<(), AdmissionError> {
        let mut state = self
            .budget
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let limits = state.limits;
        let fits = |c: Charge| {
            c.retained <= limits.retained
                && c.transient <= limits.transient
                && c.jobs <= limits.jobs
                && c.retained
                    .checked_add(c.transient)
                    .is_some_and(|total| total <= limits.total)
        };
        if !fits(charge) {
            return Err(AdmissionError::TooLarge);
        }
        let add = |used: usize, old: usize, new: usize| (used - old).checked_add(new);
        let candidate = add(state.used.retained, self.charge.retained, charge.retained)
            .zip(add(
                state.used.transient,
                self.charge.transient,
                charge.transient,
            ))
            .zip(add(state.used.jobs, self.charge.jobs, charge.jobs))
            .map(|((retained, transient), jobs)| Charge {
                retained,
                transient,
                jobs,
            })
            .filter(|candidate| fits(*candidate))
            .ok_or(AdmissionError::Busy)?;
        state.used = candidate;
        state.high_water.retained = state.high_water.retained.max(candidate.retained);
        state.high_water.transient = state.high_water.transient.max(candidate.transient);
        state.high_water.jobs = state.high_water.jobs.max(candidate.jobs);
        self.charge = charge;
        Ok(())
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self
            .budget
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.used.retained -= self.charge.retained;
        state.used.transient -= self.charge.transient;
        state.used.jobs -= self.charge.jobs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> Budget {
        Budget::new(Limits {
            retained: 100,
            transient: 100,
            total: 120,
            jobs: 4,
        })
    }

    #[test]
    fn mixed_owners_enforce_total_and_resize_failure_keeps_original_charge() {
        let budget = budget();
        let retained = budget
            .try_acquire(Charge {
                retained: 70,
                ..Charge::default()
            })
            .unwrap();
        let mut work = budget
            .try_acquire(Charge {
                transient: 50,
                jobs: 1,
                ..Charge::default()
            })
            .unwrap();
        assert_eq!(
            work.try_resize(Charge {
                transient: 51,
                jobs: 1,
                ..Charge::default()
            }),
            Err(AdmissionError::Busy)
        );
        assert_eq!(
            budget.usage().0,
            Charge {
                retained: 70,
                transient: 50,
                jobs: 1
            }
        );
        drop(retained);
        work.try_resize(Charge {
            transient: 100,
            jobs: 4,
            ..Charge::default()
        })
        .unwrap();
        assert_eq!(
            budget
                .try_acquire(Charge {
                    jobs: 1,
                    ..Charge::default()
                })
                .unwrap_err(),
            AdmissionError::Busy
        );
        drop(work);
        assert_eq!(budget.usage().0, Charge::default());
    }

    #[test]
    fn overflow_and_oversized_requests_never_modify_accounting() {
        let budget = Budget::new(Limits {
            retained: usize::MAX,
            transient: usize::MAX,
            total: usize::MAX,
            jobs: usize::MAX,
        });
        assert_eq!(
            budget
                .try_acquire(Charge {
                    retained: usize::MAX,
                    transient: 1,
                    jobs: 0
                })
                .unwrap_err(),
            AdmissionError::TooLarge
        );
        let owner = budget
            .try_acquire(Charge {
                retained: usize::MAX,
                ..Charge::default()
            })
            .unwrap();
        assert_eq!(
            budget
                .try_acquire(Charge {
                    retained: 1,
                    ..Charge::default()
                })
                .unwrap_err(),
            AdmissionError::Busy
        );
        drop(owner);
        assert_eq!(budget.usage().0, Charge::default());
    }

    #[test]
    fn error_and_panic_release_all_charges() {
        let budget = budget();
        let operation = || -> Result<(), ()> {
            let _owner = budget
                .try_acquire(Charge {
                    retained: 20,
                    transient: 20,
                    jobs: 1,
                })
                .unwrap();
            Err(())
        };
        assert!(operation().is_err());
        assert_eq!(budget.usage().0, Charge::default());
        assert!(std::panic::catch_unwind(|| {
            let _owner = budget
                .try_acquire(Charge {
                    retained: 20,
                    transient: 20,
                    jobs: 1,
                })
                .unwrap();
            panic!("simulated parser panic");
        })
        .is_err());
        assert_eq!(budget.usage().0, Charge::default());
    }

    #[test]
    fn concurrent_pressure_never_overadmits_and_eventually_releases() {
        let budget = budget();
        let barrier = Arc::new(std::sync::Barrier::new(16));
        std::thread::scope(|threads| {
            for id in 0..16 {
                let budget = budget.clone();
                let barrier = barrier.clone();
                threads.spawn(move || {
                    barrier.wait();
                    for step in 0..2000 {
                        let request = Charge {
                            retained: (id + step) % 21,
                            transient: 20,
                            jobs: 1,
                        };
                        if let Ok(mut owner) = budget.try_acquire(request) {
                            std::thread::yield_now();
                            let used = budget.usage().0;
                            assert!(used.retained <= 100 && used.transient <= 100);
                            assert!(used.retained + used.transient <= 120 && used.jobs <= 4);
                            let _ = owner.try_resize(Charge {
                                transient: 30,
                                ..request
                            });
                        }
                    }
                });
            }
        });
        assert_eq!(budget.usage().0, Charge::default());
    }
}
