//! Ownership and supersession stress over the production conversion/control path.
#[path = "../src/controlled.rs"]
mod controlled;
#[path = "../src/fixed.rs"]
mod fixed;
use rodio::{ChannelCount, SampleRate, Source};
use std::collections::VecDeque;
use std::sync::{atomic::AtomicBool, mpsc, Arc, Mutex};
use std::time::Duration;
struct Gate {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
struct Decoder {
    position: usize,
    gates: VecDeque<Gate>,
    refuse: Option<Duration>,
    calls: Arc<Mutex<Vec<Duration>>>,
}
impl Iterator for Decoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.position as f32;
        self.position += 1;
        Some(sample)
    }
}
impl Source for Decoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(8000).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
    fn try_seek(&mut self, target: Duration) -> Result<(), rodio::source::SeekError> {
        self.calls.lock().unwrap().push(target);
        if let Some(gate) = self.gates.pop_front() {
            gate.entered.send(()).unwrap();
            gate.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        if self.refuse == Some(target) {
            return Err(rodio::source::SeekError::NotSupported {
                underlying_source: "refusing ownership fixture",
            });
        }
        self.position = target.as_secs() as usize * 16000;
        Ok(())
    }
}
type Stream = controlled::Controlled<fixed::Fixed<Decoder>>;
struct Fixture {
    source: Stream,
    control: controlled::Control,
    gates: Vec<(mpsc::Receiver<()>, mpsc::Sender<()>)>,
    calls: Arc<Mutex<Vec<Duration>>>,
}
fn fixture(gates: usize, refuse: Option<Duration>) -> Fixture {
    let mut waiting = Vec::new();
    let mut decoder_gates = VecDeque::new();
    for _ in 0..gates {
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        waiting.push((entered, release));
        decoder_gates.push_back(Gate {
            entered: entered_tx,
            release: release_rx,
        });
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let fixed = fixed::Fixed::new(
        Decoder {
            position: 0,
            gates: decoder_gates,
            refuse,
            calls: calls.clone(),
        },
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8000).unwrap(),
    );
    let (source, control) =
        controlled::Controlled::new_fixed(fixed, Arc::new(AtomicBool::new(false)));
    Fixture {
        source,
        control,
        gates: waiting,
        calls,
    }
}
#[test]
fn a_hundred_thousand_supersessions_during_blocked_io_execute_only_the_latest_and_keep_playback_usable(
) {
    let Fixture {
        mut source,
        control,
        gates,
        calls,
    } = fixture(1, None);
    let old = control.seek(Duration::from_secs(2));
    let consumer = std::thread::spawn(move || {
        let sample = source.next();
        (sample, source)
    });
    gates[0].0.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut latest = None;
    for n in 0..100_000 {
        latest = Some(control.seek(Duration::from_secs(3 + n % 4)));
    }
    gates[0].1.send(()).unwrap();
    let (sample, mut source) = consumer.join().unwrap();
    assert_eq!(sample, Some(96_000.0));
    assert!(old.poll().is_none());
    assert!(latest.unwrap().poll().unwrap().is_ok());
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[Duration::from_secs(2), Duration::from_secs(6)]
    );
    assert_eq!(source.next(), Some(96_000.0 + 1.0));
    let again = control.seek(Duration::from_secs(5));
    assert_eq!(source.next(), Some(80_000.0));
    assert!(again.poll().unwrap().is_ok());
}
#[test]
fn latest_refusal_after_a_superseded_success_never_releases_old_target_audio() {
    let Fixture {
        mut source,
        control,
        gates,
        calls,
    } = fixture(1, Some(Duration::from_secs(4)));
    let old = control.seek(Duration::from_secs(2));
    let consumer = std::thread::spawn(move || {
        let sample = source.next();
        (sample, source)
    });
    gates[0].0.recv_timeout(Duration::from_secs(1)).unwrap();
    control.seek(Duration::from_secs(3));
    let latest = control.seek(Duration::from_secs(4));
    gates[0].1.send(()).unwrap();
    let (sample, mut source) = consumer.join().unwrap();
    assert_eq!(sample, None);
    assert_eq!(source.next(), None);
    assert!(old.poll().is_none());
    assert!(latest.poll().unwrap().is_err());
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[Duration::from_secs(2), Duration::from_secs(4)]
    );
}
#[test]
fn latest_terminal_cancellation_invalidates_all_earlier_entered_seeks_and_cannot_be_overwritten() {
    let Fixture {
        mut source,
        control,
        gates,
        calls,
    } = fixture(1, None);
    let old = control.seek(Duration::from_secs(2));
    let consumer = std::thread::spawn(move || source.next());
    gates[0].0.recv_timeout(Duration::from_secs(1)).unwrap();
    for target in 3..128 {
        control.seek(Duration::from_secs(target));
    }
    let latest = control.seek(Duration::from_secs(128));
    latest.cancel();
    let after_timeout = control.seek(Duration::from_secs(5));
    assert!(after_timeout.poll().unwrap().is_err());
    gates[0].1.send(()).unwrap();
    assert_eq!(consumer.join().unwrap(), None);
    assert!(latest.poll().is_none());
    assert!(old.poll().is_none());
    assert_eq!(calls.lock().unwrap().as_slice(), &[Duration::from_secs(2)]);
}
#[test]
fn repeated_inflight_supersessions_mute_uncommitted_audio_without_ending_a_usable_track() {
    let Fixture {
        mut source,
        control,
        gates,
        calls,
    } = fixture(2, None);
    let old = control.seek(Duration::from_secs(2));
    let consumer = std::thread::spawn(move || {
        let sample = source.next();
        (sample, source)
    });
    gates[0].0.recv_timeout(Duration::from_secs(1)).unwrap();
    let middle = control.seek(Duration::from_secs(3));
    gates[0].1.send(()).unwrap();
    gates[1].0.recv_timeout(Duration::from_secs(1)).unwrap();
    control.seek(Duration::from_secs(4));
    let latest = control.seek(Duration::from_secs(5));
    gates[1].1.send(()).unwrap();
    let (sample, mut source) = consumer.join().unwrap();
    // Two decoder calls is the per-turn cap; pending latest work waits for the
    // next output frame. No sample from either old target may escape meanwhile.
    assert_eq!(sample, Some(0.0));
    assert_eq!(source.next(), Some(0.0));
    assert!(latest.poll().is_none());
    assert_eq!(source.next(), Some(80_000.0));
    assert!(latest.poll().unwrap().is_ok());
    assert!(old.poll().is_none());
    assert!(middle.poll().is_none());
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[
            Duration::from_secs(2),
            Duration::from_secs(3),
            Duration::from_secs(5)
        ]
    );
}

#[test]
fn actor_acceptance_releases_old_request_ownership_before_a_later_unconsumed_cancel() {
    let Fixture {
        mut source,
        control,
        ..
    } = fixture(0, None);
    let accepted = control.seek(Duration::from_secs(2));
    assert_eq!(source.next(), Some(32_000.0));
    assert!(accepted.poll().unwrap().is_ok());
    accepted.accept();
    assert_eq!(source.next(), Some(32_001.0));
    let unconsumed = control.seek(Duration::from_secs(4));
    unconsumed.cancel();
    assert_eq!(source.next(), Some(32_002.0));
    assert!(unconsumed.poll().is_none());
    let later = control.seek(Duration::from_secs(5));
    // Complete the existing stereo frame before the new seek is processed.
    assert_eq!(source.next(), Some(32_003.0));
    assert_eq!(source.next(), Some(80_000.0));
    assert!(later.poll().unwrap().is_ok());
}

#[test]
fn completed_but_unaccepted_old_seek_cannot_become_the_fallback_after_latest_refusal() {
    let Fixture {
        mut source,
        control,
        ..
    } = fixture(0, Some(Duration::from_secs(4)));
    let old = control.seek(Duration::from_secs(2));
    assert_eq!(source.next(), Some(32_000.0));
    assert!(old.poll().unwrap().is_ok());
    // Completion precedes actor adoption. A newer request supersedes that
    // response before the actor used it to update its clock.
    old.supersede();
    let latest = control.seek(Duration::from_secs(4));
    assert_eq!(source.next(), Some(32_001.0));
    assert_eq!(source.next(), None);
    assert!(latest.poll().unwrap().is_err());
    assert!(old.poll().is_none());
}
