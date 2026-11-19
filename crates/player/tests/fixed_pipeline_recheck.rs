//! Fresh independent production-pipeline and actor-handoff canaries.
#[path = "../src/controlled.rs"]
mod controlled;
#[path = "../src/fixed.rs"]
mod fixed;
#[path = "../src/router.rs"]
mod router;

use rodio::{source::UniformSourceIterator, ChannelCount, SampleRate, Source};
use std::{
    sync::{atomic::AtomicBool, mpsc, Arc},
    time::Duration,
};

#[derive(Clone)]
struct Changing {
    offset: usize,
}
impl Iterator for Changing {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.offset == 128 {
            return None;
        }
        let value = ((self.offset * 13 % 97) as f32 - 48.0) / 64.0;
        self.offset += 1;
        Some(value)
    }
}
impl Source for Changing {
    fn current_span_len(&self) -> Option<usize> {
        Some(if self.offset < 32 {
            32 - self.offset
        } else if self.offset < 96 {
            96 - self.offset
        } else {
            128 - self.offset
        })
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(if self.offset < 32 || self.offset >= 96 {
            1
        } else {
            2
        })
        .unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(if self.offset < 32 {
            8000
        } else if self.offset < 96 {
            44100
        } else {
            11025
        })
        .unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[test]
fn independent_fixed_pipeline_matches_real_converter_for_rate_channel_matrix() {
    for channels in [1, 2, 4, 6] {
        for rate in [8000, 11025, 22050, 44100, 48000, 96000] {
            let channels = ChannelCount::new(channels).unwrap();
            let rate = SampleRate::new(rate).unwrap();
            let expected: Vec<_> =
                UniformSourceIterator::new(Changing { offset: 0 }, channels, rate).collect();
            let source = fixed::Fixed::new(Changing { offset: 0 }, channels, rate);
            let (source, _) =
                controlled::Controlled::new_fixed(source, Arc::new(AtomicBool::new(false)));
            let actual: Vec<_> = UniformSourceIterator::new(source, channels, rate).collect();
            assert_eq!(
                actual,
                expected,
                "physical {} channels / {} Hz",
                channels.get(),
                rate.get()
            );
        }
    }
}

struct BlockingSeek {
    position: usize,
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Iterator for BlockingSeek {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let value = self.position as f32;
        self.position += 1;
        Some(value)
    }
}
impl Source for BlockingSeek {
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
        self.entered.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        self.position = target.as_secs() as usize * 16000;
        Ok(())
    }
}

#[test]
fn independent_superseded_inflight_seek_does_not_emit_an_unowned_old_target() {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let fixed = fixed::Fixed::new(
        BlockingSeek {
            position: 0,
            entered: entered_tx,
            release: release_rx,
        },
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8000).unwrap(),
    );
    let (mut source, control) =
        controlled::Controlled::new_fixed(fixed, Arc::new(AtomicBool::new(false)));
    let old = control.seek(Duration::from_secs(2));
    let consumer = std::thread::spawn(move || source.next());
    entered.recv_timeout(Duration::from_secs(1)).unwrap();
    // This is the current actor's Cmd::Seek handoff: pending_seek.take() drops
    // the old actor ticket, while Control::seek can only replace its pending
    // slot (the consumer has already removed the old request from that slot).
    drop(old);
    let latest = control.seek(Duration::from_secs(4));
    // Model the latest actor-owned request hitting its deadline before the
    // older decoder call returns. No actor owner remains for the old request.
    latest.cancel();
    release.send(()).unwrap();
    let sample = consumer.join().unwrap();
    assert_eq!(
        sample, None,
        "superseded in-flight seek emitted old-target audio after the latest request was refused"
    );
}

struct SlowDrop {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Iterator for SlowDrop {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        Some(0.25)
    }
}
impl Source for SlowDrop {
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
impl Drop for SlowDrop {
    fn drop(&mut self) {
        self.entered.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}

#[test]
fn independent_router_drops_replaced_active_source_outside_pending_control_lock() {
    let (handle, mut router) = router::Router::new(
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
    let (old, old_output) = rodio::Player::new();
    old.append(SlowDrop {
        entered: entered_tx,
        release: release_rx,
    });
    handle.replace(old_output);
    // SourceQueue initially advertises its empty-silence bootstrap; consume
    // enough to put the old source in the router's active sink.
    for _ in 0..4000 {
        router.next();
    }
    let (next, next_output) = rodio::Player::new();
    handle.replace(next_output);
    let consumer = std::thread::spawn(move || {
        router.next();
        router
    });
    entered.recv_timeout(Duration::from_secs(1)).unwrap();
    let (done_tx, done) = mpsc::channel();
    let clone = handle.clone();
    let replacer = std::thread::spawn(move || {
        let (third, output) = rodio::Player::new();
        clone.replace(output);
        done_tx.send(()).unwrap();
        third
    });
    let responsive = done.recv_timeout(Duration::from_millis(200)).is_ok();
    release.send(()).unwrap();
    let router = consumer.join().unwrap();
    let third = replacer.join().unwrap();
    drop((old, next, third, router, handle));
    assert!(
        responsive,
        "active-source destruction held the router pending lock and blocked control replacement"
    );
}
