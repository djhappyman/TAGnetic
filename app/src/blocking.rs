//! Bounded admission precedes spawning. The worker owns its permit even when
//! the invoking future is dropped; mutations keep FIFO order. Preview lanes
//! additionally cancel superseded queued/running read-only work.
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    task::{Poll, Waker},
};
use tagrex_commands::ErrorDto;

const MAX_PENDING: usize = 64;
struct Resource {
    gate: Arc<tauri::async_runtime::Mutex<()>>,
    pending: AtomicUsize,
}
struct Admission(Arc<Resource>);
impl Drop for Admission {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::AcqRel);
    }
}
pub struct Queue(OnceLock<Arc<Resource>>);
impl Queue {
    pub const fn new() -> Self {
        Self(OnceLock::new())
    }
    fn resource(&self) -> Arc<Resource> {
        self.0
            .get_or_init(|| {
                Arc::new(Resource {
                    gate: Arc::new(tauri::async_runtime::Mutex::new(())),
                    pending: AtomicUsize::new(0),
                })
            })
            .clone()
    }
    pub async fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce() -> Result<T, ErrorDto> + Send + 'static,
    ) -> Result<T, ErrorDto> {
        self.run_inner(None, move |_| job()).await
    }
    pub async fn run_latest<T: Send + 'static>(
        &self,
        lane: &Latest,
        job: impl FnOnce(Cancellation) -> Result<T, ErrorDto> + Send + 'static,
    ) -> Result<T, ErrorDto> {
        self.run_inner(Some(lane), move |cancel| {
            job(cancel.expect("latest lane supplies cancellation"))
        })
        .await
    }
    async fn run_inner<T: Send + 'static>(
        &self,
        lane: Option<&Latest>,
        job: impl FnOnce(Option<Cancellation>) -> Result<T, ErrorDto> + Send + 'static,
    ) -> Result<T, ErrorDto> {
        let resource = self.resource();
        // Keep admission compatible with the MSRV and newer Rust versions,
        // which deprecate fetch_update in favor of the newer try_update API.
        let mut pending = resource.pending.load(Ordering::Acquire);
        loop {
            if pending >= MAX_PENDING {
                return Err(ErrorDto::plain("Too many pending operations; wait for the current work to finish and try again"));
            }
            match resource.pending.compare_exchange_weak(
                pending,
                pending + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => pending = current,
            }
        }
        let admission = Admission(resource.clone());
        // A rejected request must not cancel an already admitted preview.
        let cancel = lane.map(Latest::begin);
        let waiting = cancel.clone().map(Waiting);
        let mut lock = std::pin::pin!(resource.gate.clone().lock_owned());
        let permit = std::future::poll_fn(|cx| {
            if let Some(cancel) = &cancel {
                // Register then check: a cancellation cannot race past the
                // waker and leave this obsolete waiter parked behind slow I/O.
                *cancel.0.waiter.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(cx.waker().clone());
                if cancel.cancelled() {
                    return Poll::Ready(Err(superseded()));
                }
            }
            lock.as_mut().poll(cx).map(Ok)
        })
        .await;
        drop(waiting);
        let permit = permit?;
        tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            let _admission = admission;
            if cancel.as_ref().is_some_and(Cancellation::cancelled) {
                return Err(superseded());
            }
            let result = job(cancel.clone());
            if cancel.as_ref().is_some_and(Cancellation::cancelled) {
                return Err(superseded());
            }
            result
        })
        .await
        .map_err(|error| ErrorDto::plain(format!("Background task failed: {error}")))?
    }
}
fn superseded() -> ErrorDto {
    ErrorDto::plain("Preview was superseded by newer input")
}

pub struct Latest(Mutex<Option<Cancellation>>);
impl Latest {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }
    fn begin(&self) -> Cancellation {
        let next = Cancellation(Arc::new(CancelFlag {
            cancelled: AtomicBool::new(false),
            waiter: Mutex::new(None),
        }));
        if let Some(old) = self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .replace(next.clone())
        {
            old.0.cancelled.store(true, Ordering::Release);
            let waiter = old
                .0
                .waiter
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            if let Some(waiter) = waiter {
                waiter.wake();
            }
        }
        next
    }
}
struct Waiting(Cancellation);
impl Drop for Waiting {
    fn drop(&mut self) {
        self.0.clear_waiter();
    }
}
struct CancelFlag {
    cancelled: AtomicBool,
    waiter: Mutex<Option<Waker>>,
}
#[derive(Clone)]
pub struct Cancellation(Arc<CancelFlag>);
impl Cancellation {
    pub fn cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    fn clear_waiter(&self) {
        self.0
            .waiter
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
    }
}

#[cfg(test)]
mod tests {
    use super::Queue;
    use std::future::Future;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[test]
    fn jobs_leave_the_calling_thread() {
        let caller = std::thread::current().id();
        let worker =
            tauri::async_runtime::block_on(Queue::new().run(|| Ok(std::thread::current().id())))
                .unwrap();
        assert_ne!(caller, worker);
    }

    #[test]
    fn a_busy_resource_does_not_block_an_unrelated_queue() {
        let busy = Arc::new(Queue::new());
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let first = tauri::async_runtime::spawn({
            let busy = busy.clone();
            async move {
                busy.run(move || {
                    started.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(())
                })
                .await
            }
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let (finished, observed) = mpsc::channel();
        let independent = tauri::async_runtime::spawn(async move {
            Queue::new()
                .run(move || {
                    finished.send(()).unwrap();
                    Ok(())
                })
                .await
        });
        let responsive = observed.recv_timeout(Duration::from_secs(2)).is_ok();
        release.send(()).unwrap();
        tauri::async_runtime::block_on(first).unwrap().unwrap();
        tauri::async_runtime::block_on(independent)
            .unwrap()
            .unwrap();
        assert!(responsive, "unrelated work waited for the busy resource");
    }
    fn prime<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
        tauri::async_runtime::block_on(std::future::poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        }));
    }
    #[test]
    fn admission_is_bounded_and_dropped_waiters_release_their_slots() {
        let queue = Queue::new();
        let resource = queue.resource();
        let held = tauri::async_runtime::block_on(resource.gate.clone().lock_owned());
        let mut queued = Vec::new();
        for _ in 0..super::MAX_PENDING {
            let mut future = Box::pin(queue.run(|| Ok(())));
            prime(future.as_mut());
            queued.push(future);
        }
        assert_eq!(
            resource.pending.load(std::sync::atomic::Ordering::Acquire),
            super::MAX_PENDING
        );
        assert!(tauri::async_runtime::block_on(queue.run(|| Ok(())))
            .unwrap_err()
            .text
            .contains("Too many pending"));
        drop(queued);
        assert_eq!(
            resource.pending.load(std::sync::atomic::Ordering::Acquire),
            0
        );
        drop(held);
        tauri::async_runtime::block_on(queue.run(|| Ok(()))).unwrap();
    }
    #[test]
    fn a_dropped_invoker_does_not_release_a_running_workers_permit() {
        let queue = Queue::new();
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut first = Box::pin(queue.run(move || {
            started.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        }));
        prime(first.as_mut());
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(first);
        let (started, ready) = mpsc::channel();
        let mut next = Box::pin(queue.run(move || {
            started.send(()).unwrap();
            Ok(())
        }));
        prime(next.as_mut());
        assert!(ready.try_recv().is_err());
        assert_eq!(
            queue
                .resource()
                .pending
                .load(std::sync::atomic::Ordering::Acquire),
            2
        );
        release.send(()).unwrap();
        tauri::async_runtime::block_on(next).unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    #[test]
    fn a_thousand_obsolete_previews_cancel_while_waiting_and_only_the_last_executes() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let queue = Queue::new();
        let lane = super::Latest::new();
        let resource = queue.resource();
        let held = tauri::async_runtime::block_on(resource.gate.clone().lock_owned());
        let executed = Arc::new(AtomicUsize::new(0));
        let make = || {
            let executed = executed.clone();
            Box::pin(queue.run_latest(&lane, move |_| {
                executed.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }))
        };
        let mut latest = make();
        prime(latest.as_mut());
        for _ in 0..1000 {
            let mut next = make();
            prime(next.as_mut());
            assert!(tauri::async_runtime::block_on(latest)
                .unwrap_err()
                .text
                .contains("superseded"));
            latest = next;
        }
        assert_eq!(executed.load(Ordering::Relaxed), 0);
        assert_eq!(resource.pending.load(Ordering::Acquire), 1);
        drop(held);
        tauri::async_runtime::block_on(latest).unwrap();
        assert_eq!(executed.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn supersession_reaches_running_workers_and_never_returns_an_obsolete_result() {
        let queue = Queue::new();
        let lane = super::Latest::new();
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut first = Box::pin(queue.run_latest(&lane, move |cancel| {
            started.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(cancel.cancelled());
            Ok(1)
        }));
        prime(first.as_mut());
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut next = Box::pin(queue.run_latest(&lane, move |_| Ok(2)));
        prime(next.as_mut());
        release.send(()).unwrap();
        assert!(tauri::async_runtime::block_on(first).is_err());
        assert_eq!(tauri::async_runtime::block_on(next).unwrap(), 2);
    }
    #[test]
    fn preview_lanes_are_independent_and_regular_jobs_are_never_superseded() {
        let queue = Queue::new();
        let a = super::Latest::new();
        let b = super::Latest::new();
        let held = tauri::async_runtime::block_on(queue.resource().gate.clone().lock_owned());
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut first = Box::pin(queue.run_latest(&a, {
            let order = order.clone();
            move |cancel| {
                assert!(!cancel.cancelled());
                order.lock().unwrap().push(1);
                Ok(())
            }
        }));
        prime(first.as_mut());
        let mut second = Box::pin(queue.run({
            let order = order.clone();
            move || {
                order.lock().unwrap().push(2);
                Ok(())
            }
        }));
        prime(second.as_mut());
        let mut third = Box::pin(queue.run_latest(&b, {
            let order = order.clone();
            move |cancel| {
                assert!(!cancel.cancelled());
                order.lock().unwrap().push(3);
                Ok(())
            }
        }));
        prime(third.as_mut());
        drop(held);
        tauri::async_runtime::block_on(first).unwrap();
        tauri::async_runtime::block_on(second).unwrap();
        tauri::async_runtime::block_on(third).unwrap();
        assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
    }
    /// A waker that only counts how often it was woken.
    struct CountingWaker(std::sync::atomic::AtomicUsize);

    impl std::task::Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn supersession_wakes_a_parked_waiter_and_frees_its_slot() {
        use std::sync::atomic::Ordering;
        let queue = Queue::new();
        let lane = super::Latest::new();
        let held = tauri::async_runtime::block_on(queue.resource().gate.clone().lock_owned());
        let wakes = Arc::new(CountingWaker(std::sync::atomic::AtomicUsize::new(0)));
        let waker = std::task::Waker::from(wakes.clone());
        let mut cx = std::task::Context::from_waker(&waker);
        let mut parked = Box::pin(queue.run_latest(&lane, |_| Ok(())));

        // Polled once: the waiter registers its waker and waits for the gate.
        assert!(parked.as_mut().poll(&mut cx).is_pending());
        assert_eq!(wakes.0.load(Ordering::SeqCst), 0);

        // A newer request on the lane supersedes it. Waking the parked waiter is
        // what stops it sitting behind slow I/O until the gate happens to free.
        let newest = lane.begin();
        assert_eq!(
            wakes.0.load(Ordering::SeqCst),
            1,
            "a superseded waiter was left parked"
        );

        // Re-polled, it reports supersession instead of queueing for the gate.
        assert!(matches!(
            parked.as_mut().poll(&mut cx),
            std::task::Poll::Ready(Err(_))
        ));
        assert_eq!(queue.resource().pending.load(Ordering::Acquire), 0);
        assert!(!newest.cancelled(), "the newest request was cancelled");
        drop(held);
    }

    #[test]
    fn a_superseded_preview_never_executes_and_does_not_hold_up_a_regular_job() {
        let queue = Queue::new();
        let lane = super::Latest::new();
        let held = tauri::async_runtime::block_on(queue.resource().gate.clone().lock_owned());
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        let mut preview = Box::pin(queue.run_latest(&lane, {
            let order = order.clone();
            move |_| {
                order.lock().unwrap().push("preview");
                Ok(())
            }
        }));
        prime(preview.as_mut());
        let mut regular = Box::pin(queue.run({
            let order = order.clone();
            move || {
                order.lock().unwrap().push("regular");
                Ok(())
            }
        }));
        prime(regular.as_mut());

        // The newer preview cancels the queued one, which leaves the gate queue
        // instead of waiting its turn — a regular job behind it runs.
        lane.begin();
        assert!(tauri::async_runtime::block_on(preview).is_err());
        drop(held);
        tauri::async_runtime::block_on(regular).unwrap();
        assert_eq!(*order.lock().unwrap(), vec!["regular"]);
        assert_eq!(
            queue
                .resource()
                .pending
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
    }

    #[test]
    fn worker_errors_and_panics_release_both_admission_and_serialization() {
        let queue = Queue::new();
        assert!(tauri::async_runtime::block_on(
            queue.run(|| Err::<(), _>(tagrex_commands::ErrorDto::plain("injected")))
        )
        .is_err());
        assert!(tauri::async_runtime::block_on(
            queue.run(|| -> Result<(), tagrex_commands::ErrorDto> { panic!("injected") })
        )
        .is_err());
        tauri::async_runtime::block_on(queue.run(|| Ok(()))).unwrap();
        assert_eq!(
            queue
                .resource()
                .pending
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
    }
}
