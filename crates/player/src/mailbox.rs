//! Bounded, nonblocking producer mailbox. Only trailing seek/volume updates
//! commute; accepted transport controls keep their FIFO position.
use super::Cmd;
use std::sync::mpsc::RecvTimeoutError;
use std::{
    collections::VecDeque,
    fmt,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
const CAPACITY: usize = 64;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerError {
    Busy,
    Closed,
    InvalidValue,
}
impl fmt::Display for PlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "Playback command queue is busy; try again",
            Self::Closed => "Playback worker is unavailable",
            Self::InvalidValue => "Playback command has an invalid value",
        })
    }
}
impl std::error::Error for PlayerError {}
struct State {
    queue: VecDeque<Cmd>,
    closed: bool,
}
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}
pub(super) struct Sender(Arc<Shared>);
pub(super) struct Receiver(Arc<Shared>);
pub(super) fn channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            queue: VecDeque::new(),
            closed: false,
        }),
        ready: Condvar::new(),
    });
    (Sender(shared.clone()), Receiver(shared))
}
impl Sender {
    pub fn send(&self, mut cmd: Cmd) -> Result<(), PlayerError> {
        match &mut cmd {
            Cmd::Seek(value) => {
                if !value.is_finite() || Duration::try_from_secs_f64(value.max(0.0)).is_err() {
                    return Err(PlayerError::InvalidValue);
                }
                *value = value.max(0.0);
            }
            Cmd::SetVolume(value) => {
                if !value.is_finite() {
                    return Err(PlayerError::InvalidValue);
                }
                *value = value.clamp(0.0, 1.0);
            }
            Cmd::Play(path) | Cmd::SetNext(path) if path.as_os_str().len() > 65_536 => {
                return Err(PlayerError::InvalidValue)
            }
            _ => {}
        }
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.closed {
            return Err(PlayerError::Closed);
        }
        // Replacing a seek/volume is safe only after the last transport barrier.
        // Moving across Play/Stop/Pause/Resume/SetNext could target another track.
        let mut replace = None;
        if matches!(cmd, Cmd::Seek(_) | Cmd::SetVolume(_)) {
            for (i, old) in state.queue.iter().enumerate().rev() {
                if !matches!(old, Cmd::Seek(_) | Cmd::SetVolume(_)) {
                    break;
                }
                if matches!(
                    (&cmd, old),
                    (Cmd::Seek(_), Cmd::Seek(_)) | (Cmd::SetVolume(_), Cmd::SetVolume(_))
                ) {
                    replace = Some(i);
                    break;
                }
            }
        } else if matches!(cmd, Cmd::SetNext(_))
            && matches!(state.queue.back(), Some(Cmd::SetNext(_)))
        {
            replace = Some(state.queue.len() - 1);
        }
        if let Some(i) = replace {
            state.queue.remove(i);
        }
        if state.queue.len() >= CAPACITY {
            return Err(PlayerError::Busy);
        }
        state.queue.push_back(cmd);
        drop(state);
        self.0.ready.notify_one();
        Ok(())
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .closed = true;
        self.0.ready.notify_all();
    }
}
impl Receiver {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Cmd, RecvTimeoutError> {
        let state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        let (mut state, _) = self
            .0
            .ready
            .wait_timeout_while(state, timeout, |s| s.queue.is_empty() && !s.closed)
            .unwrap_or_else(|p| p.into_inner());
        if let Some(cmd) = state.queue.pop_front() {
            return Ok(cmd);
        }
        Err(if state.closed {
            RecvTimeoutError::Disconnected
        } else {
            RecvTimeoutError::Timeout
        })
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .closed = true;
        self.0.ready.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn next(rx: &Receiver) -> Cmd {
        rx.recv_timeout(Duration::ZERO).unwrap()
    }
    #[test]
    fn seek_and_volume_bursts_keep_only_the_latest_values() {
        let (tx, rx) = channel();
        for i in 0..100_000 {
            tx.send(Cmd::Seek(i as f64)).unwrap();
            tx.send(Cmd::SetVolume((i % 100) as f32 / 100.0)).unwrap();
        }
        assert_eq!(tx.0.state.lock().unwrap().queue.len(), 2);
        assert!(matches!(next(&rx), Cmd::Seek(99_999.0)));
        assert!(matches!(next(&rx),Cmd::SetVolume(v) if (v-0.99).abs()<0.001));
    }
    #[test]
    fn transport_barriers_preserve_track_and_control_order() {
        let (tx, rx) = channel();
        tx.send(Cmd::Play("one.wav".into())).unwrap();
        tx.send(Cmd::Seek(1.0)).unwrap();
        tx.send(Cmd::Seek(2.0)).unwrap();
        tx.send(Cmd::Stop).unwrap();
        tx.send(Cmd::Play("two.wav".into())).unwrap();
        tx.send(Cmd::Seek(3.0)).unwrap();
        tx.send(Cmd::Pause).unwrap();
        tx.send(Cmd::Seek(4.0)).unwrap();
        assert!(matches!(next(&rx),Cmd::Play(p) if p==std::path::Path::new("one.wav")));
        assert!(matches!(next(&rx), Cmd::Seek(2.0)));
        assert!(matches!(next(&rx), Cmd::Stop));
        assert!(matches!(next(&rx),Cmd::Play(p) if p==std::path::Path::new("two.wav")));
        assert!(matches!(next(&rx), Cmd::Seek(3.0)));
        assert!(matches!(next(&rx), Cmd::Pause));
        assert!(matches!(next(&rx), Cmd::Seek(4.0)));
    }
    #[test]
    fn full_transport_queue_fails_explicitly_without_losing_accepted_controls() {
        let (tx, rx) = channel();
        for _ in 0..CAPACITY {
            tx.send(Cmd::Stop).unwrap();
        }
        assert_eq!(
            tx.send(Cmd::Play("later.wav".into())),
            Err(PlayerError::Busy)
        );
        for _ in 0..CAPACITY {
            assert!(matches!(next(&rx), Cmd::Stop));
        }
        assert!(tx.send(Cmd::Play("later.wav".into())).is_ok());
    }
    #[test]
    fn invalid_numbers_and_closed_workers_are_reported_and_shutdown_drains_commands() {
        let (tx, rx) = channel();
        assert_eq!(tx.send(Cmd::Seek(f64::NAN)), Err(PlayerError::InvalidValue));
        assert_eq!(tx.send(Cmd::Seek(f64::MAX)), Err(PlayerError::InvalidValue));
        assert_eq!(
            tx.send(Cmd::SetVolume(f32::INFINITY)),
            Err(PlayerError::InvalidValue)
        );
        tx.send(Cmd::Stop).unwrap();
        drop(tx);
        assert!(matches!(next(&rx), Cmd::Stop));
        assert!(matches!(
            rx.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Disconnected)
        ));
        let (tx, rx) = channel();
        drop(rx);
        assert_eq!(tx.send(Cmd::Stop), Err(PlayerError::Closed));
    }
}
