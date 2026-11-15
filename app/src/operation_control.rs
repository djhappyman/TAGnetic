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
