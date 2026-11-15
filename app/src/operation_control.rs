//! Admission and cancellation owned by one Apply/BPM run, including its workers.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tagrex_commands::ErrorDto;

#[derive(Default)]
pub struct OperationControl(Arc<Mutex<Option<Arc<AtomicBool>>>>);

#[derive(Clone)]
pub struct OperationGuard(Arc<ActiveOperation>);

struct ActiveOperation {
    owner: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    cancelled: Arc<AtomicBool>,
}

impl OperationControl {
    /// Reject overlapping requests before touching the active cancellation flag.
    pub fn admit(&self, name: &str) -> Result<OperationGuard, ErrorDto> {
        let mut active = self.0.lock().map_err(super::shell_error)?;
        if active.is_some() {
            return Err(super::shell_error(format!("{name} is already running")));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        *active = Some(cancelled.clone());
        Ok(OperationGuard(Arc::new(ActiveOperation {
            owner: self.0.clone(),
            cancelled,
        })))
    }

    pub fn cancel(&self) {
        if let Ok(active) = self.0.lock() {
            if let Some(cancelled) = active.as_ref() {
                cancelled.store(true, Ordering::SeqCst);
            }
        }
    }
}

impl OperationGuard {
    pub fn cancelled(&self) -> &AtomicBool {
        &self.0.cancelled
    }
}

impl Drop for ActiveOperation {
    fn drop(&mut self) {
        if let Ok(mut active) = self.owner.lock() {
            *active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn rejected_request_does_not_reset_cancellation_and_next_run_is_fresh() {
        let control = OperationControl::default();
        control.cancel(); // No pending cancellation for a later operation.
        let first = control.admit("Apply").unwrap();
        control.cancel();
        assert!(control
            .admit("Apply")
            .err()
            .unwrap()
            .text
            .contains("already running"));
        assert!(first.cancelled().load(Ordering::SeqCst));
        drop(first);
        let next = control.admit("Apply").unwrap();
        assert!(!next.cancelled().load(Ordering::SeqCst));
    }

    #[test]
    fn dropped_caller_keeps_admission_until_queued_blocking_work_finishes() {
        let control = OperationControl::default();
        let operation = control.admit("BPM detection").unwrap();
        let worker = operation.clone();
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let job = tauri::async_runtime::spawn_blocking(move || {
            started.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(worker.cancelled().load(Ordering::SeqCst));
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(operation);
        control.cancel();
        assert!(control.admit("BPM detection").is_err());
        release.send(()).unwrap();
        tauri::async_runtime::block_on(job).unwrap();
        assert!(control.admit("BPM detection").is_ok());
    }

    #[test]
    fn queued_request_keeps_its_cancellation_and_dropping_it_releases_admission() {
        let queue = Arc::new(super::super::blocking::Queue::new());
        let control = OperationControl::default();
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let blocker = tauri::async_runtime::spawn({
            let queue = queue.clone();
            async move {
                queue
                    .run(move || {
                        started.send(()).unwrap();
                        wait.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok(())
                    })
                    .await
            }
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let operation = control.admit("Apply").unwrap();
        let worker = operation.clone();
        let mut queued = Box::pin(queue.run(move || {
            assert!(worker.cancelled().load(Ordering::SeqCst));
            Ok(())
        }));
        tauri::async_runtime::block_on(std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(queued.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        }));
        control.cancel();
        assert!(control.admit("Apply").is_err());
        assert!(operation.cancelled().load(Ordering::SeqCst));
        drop(operation);
        drop(queued);
        assert!(control.admit("Apply").is_ok());
        release.send(()).unwrap();
        tauri::async_runtime::block_on(blocker).unwrap().unwrap();
    }

    #[test]
    fn error_and_panic_release_admission() {
        let control = OperationControl::default();
        let failure = || -> Result<(), ErrorDto> {
            let _operation = control.admit("Apply")?;
            Err(super::super::shell_error("failed"))
        };
        assert!(failure().is_err());
        let operation = control.admit("Apply").unwrap();
        let worker = tauri::async_runtime::spawn_blocking(move || {
            let _operation = operation;
            panic!("injected worker panic");
        });
        assert!(tauri::async_runtime::block_on(worker).is_err());
        assert!(control.admit("Apply").is_ok());
    }
}

/// Async admission for IPC image downloads. The command crate retains its
/// synchronous cap for callers that do not go through the Tauri dispatcher.
pub struct ImageAdmission {
    available: tauri::async_runtime::Mutex<tauri::async_runtime::Receiver<()>>,
    returned: tauri::async_runtime::Sender<()>,
}

struct ImagePermit(tauri::async_runtime::Sender<()>);

impl Drop for ImagePermit {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

impl ImageAdmission {
    pub fn new(limit: usize) -> Self {
        let (returned, available) = tauri::async_runtime::channel(limit);
        for _ in 0..limit {
            returned
                .try_send(())
                .expect("new image permit channel has capacity");
        }
        Self {
            available: tauri::async_runtime::Mutex::new(available),
            returned,
        }
    }

    pub async fn run<T: Send + 'static>(
        &self,
        download: impl FnOnce() -> Result<T, ErrorDto> + Send + 'static,
    ) -> Result<T, ErrorDto> {
        self.available
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| super::shell_error("Image admission closed"))?;
        let permit = ImagePermit(self.returned.clone());
        tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            download()
        })
        .await
        .map_err(super::shell_error)?
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use std::future::Future;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn saturated_images_wait_before_spawning_and_leave_other_work_responsive() {
        let admission = Arc::new(ImageAdmission::new(6));
        let (started, ready) = mpsc::channel();
        let mut releases = Vec::new();
        let mut jobs = Vec::new();
        for _ in 0..6 {
            let (release, wait) = mpsc::channel();
            releases.push(release);
            jobs.push(tauri::async_runtime::spawn({
                let admission = admission.clone();
                let started = started.clone();
                async move {
                    admission
                        .run(move || {
                            started.send(()).unwrap();
                            wait.recv_timeout(Duration::from_secs(5)).unwrap();
                            Ok(())
                        })
                        .await
                }
            }));
        }
        for _ in 0..6 {
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        let mut waiters = Vec::new();
        for _ in 0..12 {
            let admission = admission.clone();
            let mut waiting = Box::pin(async move {
                admission
                    .run::<()>(|| panic!("a saturated image entered blocking work"))
                    .await
            });
            tauri::async_runtime::block_on(std::future::poll_fn(|cx| {
                assert!(waiting.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            }));
            waiters.push(waiting);
        }
        // The futures above are suspended at admission, with no blocking job.
        let (done, observed) = mpsc::channel();
        let unrelated = tauri::async_runtime::spawn_blocking(move || done.send(()).unwrap());
        let responsive = observed.recv_timeout(Duration::from_secs(2)).is_ok();
        drop(waiters);
        for release in releases {
            release.send(()).unwrap();
        }
        for job in jobs {
            tauri::async_runtime::block_on(job).unwrap().unwrap();
        }
        tauri::async_runtime::block_on(unrelated).unwrap();
        assert!(responsive);
        assert_eq!(
            tauri::async_runtime::block_on(admission.run(|| Ok(7))).unwrap(),
            7
        );
    }

    #[test]
    fn image_permit_survives_caller_abort_and_releases_on_error_or_panic() {
        let admission = Arc::new(ImageAdmission::new(1));
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let first = tauri::async_runtime::spawn({
            let admission = admission.clone();
            async move {
                admission
                    .run(move || {
                        started.send(()).unwrap();
                        wait.recv_timeout(Duration::from_secs(5)).unwrap();
                        Err::<(), _>(super::super::shell_error("fake download error"))
                    })
                    .await
            }
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        first.abort();
        let mut next = Box::pin(admission.run(|| Ok(8)));
        tauri::async_runtime::block_on(std::future::poll_fn(|cx| {
            assert!(next.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        }));
        release.send(()).unwrap();
        assert_eq!(tauri::async_runtime::block_on(next).unwrap(), 8);
        assert!(tauri::async_runtime::block_on(
            admission.run(|| -> Result<(), ErrorDto> { panic!("fake download panic") })
        )
        .is_err());
        assert!(tauri::async_runtime::block_on(admission.run(|| Ok(()))).is_ok());
    }
}
