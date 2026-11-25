//! Bounded, nonblocking producer mailbox. Trailing seek/volume updates
//! coalesce without moving across playback ownership or transport controls.
use super::{studio, Cmd};
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
                return Err(PlayerError::InvalidValue);
            }
            Cmd::Studio(command) if !command.valid() => return Err(PlayerError::InvalidValue),
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
                let volume_crosses_studio_seek = matches!(cmd, Cmd::SetVolume(_))
                    && matches!(old, Cmd::Studio(studio::Command::Seek { .. }));
                if !volume_crosses_studio_seek && !matches!(old, Cmd::Seek(_) | Cmd::SetVolume(_)) {
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
        } else if let Cmd::Studio(studio::Command::Seek {
            owner, sequence, ..
        }) = &cmd
        {
            for (i, old) in state.queue.iter().enumerate().rev() {
                match old {
                    Cmd::Studio(studio::Command::Seek {
                        owner: previous,
                        sequence: previous_sequence,
                        ..
                    }) if previous == owner => {
                        if *sequence != 0 && previous_sequence > sequence {
                            return Ok(());
                        }
                        replace = Some(i);
                        break;
                    }
                    Cmd::SetVolume(_) => {}
                    _ => break,
                }
            }
        } else if let Cmd::Studio(studio::Command::Close { owner }) = &cmd {
            if matches!(state.queue.back(), Some(Cmd::Studio(studio::Command::Close { owner: previous })) if previous == owner)
            {
                return Ok(());
            }
        } else if matches!(cmd, Cmd::SetNext(_))
            && matches!(state.queue.back(), Some(Cmd::SetNext(_)))
        {
            replace = Some(state.queue.len() - 1);
        }
        if let Some(i) = replace {
            state.queue.remove(i);
        }
        // Keep one extra slot to relinquish the preview when ordinary playback
        // controls have filled the queue. Losing that close could suspend the
        // main player even after the editor has been dismissed.
        let reserve = usize::from(matches!(cmd, Cmd::Studio(studio::Command::Close { .. })));
        if state.queue.len() >= CAPACITY + reserve {
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
    fn a_late_studio_seek_cannot_replace_a_newer_queued_seek() {
        let (sender, receiver) = channel();
        let seek = |sequence| {
            Cmd::Studio(studio::Command::Seek {
                owner: "editor".into(),
                sequence,
                position: Duration::from_secs(sequence),
            })
        };
        sender.send(seek(2)).unwrap();
        sender.send(Cmd::SetVolume(0.5)).unwrap();
        sender.send(seek(1)).unwrap();
        assert!(matches!(
            next(&receiver),
            Cmd::Studio(studio::Command::Seek { sequence: 2, .. })
        ));
        assert!(matches!(next(&receiver), Cmd::SetVolume(_)));
    }

    #[test]
    fn studio_seek_bursts_coalesce_by_owner_and_close_has_reserved_capacity() {
        let (tx, rx) = channel();
        let seek = |owner: &str, ms| {
            Cmd::Studio(studio::Command::Seek {
                owner: owner.into(),
                sequence: 0,
                position: Duration::from_millis(ms),
            })
        };
        for i in 0..10_000 {
            tx.send(seek("first", i)).unwrap();
            tx.send(Cmd::SetVolume(0.5)).unwrap();
        }
        tx.send(seek("other", 15)).unwrap();
        tx.send(seek("first", 123)).unwrap();
        assert!(
            matches!(next(&rx), Cmd::Studio(studio::Command::Seek {owner, position, ..}) if owner == "first" && position.as_millis() == 9_999)
        );
        assert!(matches!(next(&rx), Cmd::SetVolume(_)));
        assert!(
            matches!(next(&rx), Cmd::Studio(studio::Command::Seek {owner, ..}) if owner == "other")
        );
        assert!(
            matches!(next(&rx), Cmd::Studio(studio::Command::Seek {owner, position, ..}) if owner == "first" && position.as_millis() == 123)
        );
        for _ in 0..CAPACITY {
            tx.send(Cmd::Pause).unwrap();
        }
        tx.send(Cmd::Studio(studio::Command::Close {
            owner: "first".into(),
        }))
        .unwrap();
        tx.send(Cmd::Studio(studio::Command::Close {
            owner: "first".into(),
        }))
        .unwrap();
        assert_eq!(tx.send(seek("first", 123)), Err(PlayerError::Busy));
        for _ in 0..CAPACITY {
            assert!(matches!(next(&rx), Cmd::Pause));
        }
        assert!(
            matches!(next(&rx), Cmd::Studio(studio::Command::Close { owner }) if owner == "first")
        );
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
