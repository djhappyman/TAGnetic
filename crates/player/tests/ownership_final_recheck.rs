//! Independent ownership/replacement schedules over the actual adapters.
#[path = "../src/controlled.rs"]
mod controlled;
#[path = "../src/fixed.rs"]
mod fixed;
#[path = "../src/router.rs"]
mod router;

use rodio::{ChannelCount, SampleRate, Source};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

struct SeekGate {
    position: usize,
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Iterator for SeekGate {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.position as f32;
        self.position += 1;
        Some(sample)
    }
}
impl Source for SeekGate {
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
        if target == Duration::from_secs(4) {
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        self.position = target.as_secs() as usize * 16000;
        Ok(())
    }
}

#[test]
fn stale_acknowledgement_cannot_disown_a_newer_seek_already_in_decoder_io() {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let source = fixed::Fixed::new(
        SeekGate {
            position: 0,
            entered: entered_tx,
            release: release_rx,
        },
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8000).unwrap(),
    );
    let (mut source, control) =
        controlled::Controlled::new_fixed(source, Arc::new(AtomicBool::new(false)));
    let completed_old = control.seek(Duration::from_secs(2));
    assert_eq!(source.next(), Some(32000.0));
    assert!(completed_old.poll().unwrap().is_ok());
    assert_eq!(source.next(), Some(32001.0));
    let latest = control.seek(Duration::from_secs(4));
    let consumer = std::thread::spawn(move || source.next());
    entered.recv_timeout(Duration::from_secs(1)).unwrap();
    // A late acknowledgement from a superseded ticket must not clear the
    // source's ownership of a different decoder seek which is now in flight.
    completed_old.accept();
    latest.cancel();
    release.send(()).unwrap();
    let sample = consumer.join().unwrap();
    assert_eq!(sample, None);
    assert!(latest.poll().is_none());
    assert!(control
        .seek(Duration::from_secs(5))
        .poll()
        .unwrap()
        .is_err());
}

struct BlockingDrop {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Iterator for BlockingDrop {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        Some(0.125)
    }
}
impl Source for BlockingDrop {
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
}
impl Drop for BlockingDrop {
    fn drop(&mut self) {
        self.entered.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
struct CountedConstant {
    sample: f32,
    live: Arc<AtomicUsize>,
}
impl Iterator for CountedConstant {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        Some(self.sample)
    }
}
impl Source for CountedConstant {
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
}
impl Drop for CountedConstant {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

#[test]
fn replacement_burst_stays_bounded_and_latest_wins_while_old_active_drop_is_blocked() {
    let (handle, mut output) = router::Router::new(
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8000).unwrap(),
    );
    assert_eq!(
        handle.format(),
        (
            ChannelCount::new(2).unwrap(),
            SampleRate::new(8000).unwrap()
        )
    );
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let (first, first_output) = rodio::Player::new();
    first.append(BlockingDrop {
        entered: entered_tx,
        release: release_rx,
    });
    handle.replace(first_output);
    for _ in 0..4000 {
        output.next();
    }
    let (second, second_output) = rodio::Player::new();
    handle.replace(second_output);
    let consumer = std::thread::spawn(move || {
        output.next();
        output
    });
    entered.recv_timeout(Duration::from_secs(1)).unwrap();
    let live = Arc::new(AtomicUsize::new(0));
    let counter = live.clone();
    let control = handle.clone();
    let (done_tx, done) = mpsc::channel();
    let replacer = std::thread::spawn(move || {
        let mut last = None;
        for index in 0..10_000 {
            let (player, pending) = rodio::Player::new();
            counter.fetch_add(1, Ordering::SeqCst);
            player.append(CountedConstant {
                sample: index as f32,
                live: counter.clone(),
            });
            control.replace(pending);
            assert!(
                counter.load(Ordering::SeqCst) <= 1,
                "obsolete pending sources accumulated"
            );
            // Do not retain an additional external owner of every retired sink:
            // the boundedness claim concerns the router's own slots. Keep only
            // the final handle alive so its stream can be checked after release.
            if index == 9999 {
                last = Some(player);
            } else {
                drop(player);
            }
        }
        done_tx.send(()).unwrap();
        last.unwrap()
    });
    let responsive = done.recv_timeout(Duration::from_secs(2)).is_ok();
    // Always release and join owned threads, including the failing-control path.
    release.send(()).unwrap();
    let mut output = consumer.join().unwrap();
    let last = replacer.join().unwrap();
    assert!(
        responsive,
        "replacement controls waited for unrelated old-source Drop"
    );
    assert_eq!(live.load(Ordering::SeqCst), 1);
    let samples: Vec<_> = output.by_ref().take(4000).collect();
    assert!(samples.contains(&9999.0));
    assert!(samples
        .iter()
        .all(|sample| *sample == 0.0 || *sample == 9999.0));
    drop((first, second, last, output, handle));
    assert_eq!(live.load(Ordering::SeqCst), 0);
}
