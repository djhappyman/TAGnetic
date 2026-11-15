//! Beatport publication generations and single-flight token requests.
//! All persistence closures run on blocking workers; HTTP runs outside the lock.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tagrex_commands::ErrorDto;
use tauri::async_runtime::{channel, Sender};

type TokenResult = Result<String, ErrorDto>;
type Flight = Arc<Mutex<Vec<Sender<TokenResult>>>>;

#[derive(Default)]
pub struct SessionControl {
    persistence: Mutex<()>,
    generation: AtomicU64,
    // Enrollment never waits on filesystem I/O or HTTP.
    refresh: Mutex<Option<(u64, Flight)>>,
}

impl SessionControl {
    pub fn read<T>(
        &self,
        read: impl FnOnce() -> Result<T, ErrorDto>,
    ) -> Result<(u64, T), ErrorDto> {
        let _persistence = self.persistence.lock().map_err(super::shell_error)?;
        Ok((self.generation.load(Ordering::SeqCst), read()?))
    }

    pub fn read_at<T>(
        &self,
        generation: u64,
        read: impl FnOnce() -> Result<T, ErrorDto>,
    ) -> Result<T, ErrorDto> {
        let _persistence = self.persistence.lock().map_err(super::shell_error)?;
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err(super::shell_error("Beatport session changed"));
        }
        read()
    }

    pub fn publish(
        &self,
        generation: u64,
        write: impl FnOnce() -> Result<(), ErrorDto>,
    ) -> Result<(), ErrorDto> {
        let _persistence = self.persistence.lock().map_err(super::shell_error)?;
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err(super::shell_error(
                "Beatport session changed; sign-in result discarded",
            ));
        }
        write()?;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub fn logout(&self, remove: impl FnOnce() -> Result<(), ErrorDto>) -> Result<(), ErrorDto> {
        let _persistence = self.persistence.lock().map_err(super::shell_error)?;
        // Invalidate even if removing the file fails. Old work must never
        // publish after a logout request, including a failed one.
        self.generation.fetch_add(1, Ordering::SeqCst);
        remove()
    }

    /// All callers in a flight receive the same result, including failure.
    /// A detached coordinator owns completion if an IPC caller disappears.
    pub async fn token(
        self: &Arc<Self>,
        work: impl FnOnce(Arc<Self>, u64) -> TokenResult + Send + 'static,
    ) -> TokenResult {
        let (tx, mut rx) = channel(1);
        let launch = {
            let mut refresh = self.refresh.lock().map_err(super::shell_error)?;
            let generation = self.generation.load(Ordering::SeqCst);
            if let Some((_, flight)) = refresh.as_ref().filter(|(epoch, _)| *epoch == generation) {
                flight.lock().map_err(super::shell_error)?.push(tx);
                None
            } else {
                let flight = Arc::new(Mutex::new(vec![tx]));
                *refresh = Some((generation, flight.clone()));
                Some((generation, flight))
            }
        };
        if let Some((generation, flight)) = launch {
            let control = self.clone();
            tauri::async_runtime::spawn(async move {
                let worker = control.clone();
                let result = tauri::async_runtime::spawn_blocking(move || work(worker, generation))
                    .await
                    .map_err(super::shell_error)
                    .and_then(|result| result);
                if let Ok(mut refresh) = control.refresh.lock() {
                    if refresh
                        .as_ref()
                        .is_some_and(|(_, active)| Arc::ptr_eq(active, &flight))
                    {
                        *refresh = None;
                    }
                    if let Ok(mut waiters) = flight.lock() {
                        for waiter in waiters.drain(..) {
                            let _ = waiter.try_send(result.clone());
                        }
                    }
                }
            });
        }
        rx.recv()
            .await
            .ok_or_else(|| super::shell_error("Beatport token task ended without a result"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    struct SessionFile(PathBuf);
    impl SessionFile {
        fn new() -> Arc<Self> {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "tagrex-fake-session-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::write(&path, "fake-expired-token").unwrap();
            Arc::new(Self(path))
        }
        fn remove(&self) -> Result<(), ErrorDto> {
            std::fs::remove_file(&self.0).map_err(super::super::shell_error)
        }
        fn write(&self) -> Result<(), ErrorDto> {
            std::fs::write(&self.0, "fake-new-token").map_err(super::super::shell_error)
        }
    }
    impl Drop for SessionFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn logout_rejects_delayed_login_publication() {
        let control = Arc::new(SessionControl::default());
        let file = SessionFile::new();
        let generation = control.read(|| Ok(())).unwrap().0;
        let (release, wait) = mpsc::channel();
        let worker = std::thread::spawn({
            let control = control.clone();
            let file = file.clone();
            move || {
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
                control.publish(generation, || file.write())
            }
        });
        control.logout(|| file.remove()).unwrap();
        release.send(()).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(!file.0.exists());
    }

    #[test]
    fn logout_rejects_delayed_refresh_while_settings_remain_responsive() {
        let control = Arc::new(SessionControl::default());
        let file = SessionFile::new();
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let request = tauri::async_runtime::spawn({
            let control = control.clone();
            let file = file.clone();
            async move {
                control
                    .token(move |control, generation| {
                        control.read_at(generation, || {
                            Ok(std::fs::read_to_string(&file.0).unwrap())
                        })?;
                        started.send(()).unwrap();
                        wait.recv_timeout(Duration::from_secs(5)).unwrap();
                        control.publish(generation, || file.write())?;
                        Ok("fake-new-token".into())
                    })
                    .await
            }
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        // This is the same persistence lock logout uses in production.
        let responsiveness = std::time::Instant::now();
        control.logout(|| file.remove()).unwrap();
        let unrelated =
            tauri::async_runtime::block_on(super::super::SETTINGS_QUEUE.run(|| Ok(42))).unwrap();
        assert_eq!(unrelated, 42);
        assert!(responsiveness.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        assert!(tauri::async_runtime::block_on(request).unwrap().is_err());
        assert!(!file.0.exists());
    }

    #[test]
    fn concurrent_requests_share_one_refresh_and_the_same_result() {
        let control = Arc::new(SessionControl::default());
        let count = Arc::new(AtomicUsize::new(0));
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let first = tauri::async_runtime::spawn({
            let control = control.clone();
            let count = count.clone();
            async move {
                control
                    .token(move |_, _| {
                        count.fetch_add(1, Ordering::SeqCst);
                        started.send(()).unwrap();
                        wait.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok("one-token".into())
                    })
                    .await
            }
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut followers = Vec::new();
        // Poll each future once to deterministically register its waiter.
        for _ in 0..8 {
            let control = control.clone();
            let mut request = Box::pin(async move {
                control
                    .token(|_, _| panic!("a follower started another refresh"))
                    .await
            });
            tauri::async_runtime::block_on(std::future::poll_fn(|cx| {
                assert!(request.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            }));
            followers.push(request);
        }
        release.send(()).unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(first).unwrap().unwrap(),
            "one-token"
        );
        for request in followers {
            assert_eq!(
                tauri::async_runtime::block_on(request).unwrap(),
                "one-token"
            );
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failure_panic_and_dropped_caller_allow_later_retry() {
        let control = Arc::new(SessionControl::default());
        assert!(tauri::async_runtime::block_on(
            control.token(|_, _| Err(super::super::shell_error("fake HTTP failure")))
        )
        .is_err());
        assert!(
            tauri::async_runtime::block_on(control.token(|_, _| panic!("fake worker panic")))
                .is_err()
        );
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let first = tauri::async_runtime::spawn({
            let control = control.clone();
            async move {
                control
                    .token(move |_, _| {
                        started.send(()).unwrap();
                        wait.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok("still-owned".into())
                    })
                    .await
            }
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        first.abort();
        let mut follower = Box::pin(control.token(|_, _| panic!("lost the active flight")));
        tauri::async_runtime::block_on(std::future::poll_fn(|cx| {
            assert!(follower.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        }));
        release.send(()).unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(follower).unwrap(),
            "still-owned"
        );
        assert_eq!(
            tauri::async_runtime::block_on(control.token(|_, _| Ok("retry".into()))).unwrap(),
            "retry"
        );
    }

    #[test]
    fn stale_refresh_completion_cannot_clear_a_new_sessions_flight() {
        let control = Arc::new(SessionControl::default());
        let (old_started, old_ready) = mpsc::channel();
        let (old_release, old_wait) = mpsc::channel();
        let old = tauri::async_runtime::spawn({
            let control = control.clone();
            async move {
                control
                    .token(move |control, generation| {
                        control.read_at(generation, || Ok(()))?;
                        old_started.send(()).unwrap();
                        old_wait.recv_timeout(Duration::from_secs(5)).unwrap();
                        control.publish(generation, || Ok(()))?;
                        Ok("old".into())
                    })
                    .await
            }
        });
        old_ready.recv_timeout(Duration::from_secs(5)).unwrap();
        control.logout(|| Ok(())).unwrap();
        let (new_started, new_ready) = mpsc::channel();
        let (new_release, new_wait) = mpsc::channel();
        let new = tauri::async_runtime::spawn({
            let control = control.clone();
            async move {
                control
                    .token(move |control, generation| {
                        control.read_at(generation, || Ok(()))?;
                        new_started.send(()).unwrap();
                        new_wait.recv_timeout(Duration::from_secs(5)).unwrap();
                        control.publish(generation, || Ok(()))?;
                        Ok("new".into())
                    })
                    .await
            }
        });
        new_ready.recv_timeout(Duration::from_secs(5)).unwrap();
        old_release.send(()).unwrap();
        assert!(tauri::async_runtime::block_on(old).unwrap().is_err());
        let mut follower =
            Box::pin(control.token(|_, _| panic!("old completion cleared the new flight")));
        tauri::async_runtime::block_on(std::future::poll_fn(|cx| {
            assert!(follower.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        }));
        new_release.send(()).unwrap();
        assert_eq!(tauri::async_runtime::block_on(new).unwrap().unwrap(), "new");
        assert_eq!(tauri::async_runtime::block_on(follower).unwrap(), "new");
    }

    #[test]
    fn newer_publication_and_failed_logout_invalidate_old_snapshots() {
        let control = SessionControl::default();
        let old = control.read(|| Ok(())).unwrap().0;
        control.publish(old, || Ok(())).unwrap();
        assert!(control
            .publish(old, || panic!("stale publication ran"))
            .is_err());
        let current = control.read(|| Ok(())).unwrap().0;
        assert!(control
            .logout(|| Err(super::super::shell_error("fake I/O failure")))
            .is_err());
        assert!(control
            .read_at::<()>(current, || panic!("stale snapshot ran"))
            .is_err());
    }
}
