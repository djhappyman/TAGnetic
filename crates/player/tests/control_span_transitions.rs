//! Additional control/format boundary cases using the actual Rodio converter.
//! The independent review canaries remain unchanged in adversarial_controls.rs.
#[path = "../src/controlled.rs"]
mod controlled;
#[path = "../src/fixed.rs"]
mod fixed;
use rodio::{source::UniformSourceIterator, ChannelCount, SampleRate, Source};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

#[derive(Clone)]
struct Formats {
    offset: usize,
}
impl Iterator for Formats {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.offset == 24 {
            return None;
        }
        let sample = (self.offset + 1) as f32 * 0.01;
        self.offset += 1;
        Some(sample)
    }
}
impl Source for Formats {
    fn current_span_len(&self) -> Option<usize> {
        Some(if self.offset < 8 {
            8 - self.offset
        } else {
            24 - self.offset
        })
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(if self.offset < 8 { 1 } else { 2 }).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(if self.offset < 8 { 8_000 } else { 16_000 }).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
    fn try_seek(&mut self, to: Duration) -> Result<(), rodio::source::SeekError> {
        self.offset = if to.is_zero() { 0 } else { 12 };
        Ok(())
    }
}
fn converted(source: impl Source) -> Vec<f32> {
    UniformSourceIterator::new(
        source,
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8_000).unwrap(),
    )
    .collect()
}
#[test]
fn pause_resume_within_native_span_preserves_later_channel_and_rate_conversion() {
    let expected = converted(Formats { offset: 0 });
    let paused = Arc::new(AtomicBool::new(false));
    let (source, _) = controlled::Controlled::new(Formats { offset: 0 }, paused.clone());
    let mut actual = UniformSourceIterator::new(
        source,
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8_000).unwrap(),
    );
    let prefix: Vec<_> = actual.by_ref().take(4).collect();
    paused.store(true, Ordering::Release);
    let silence: Vec<_> = actual.by_ref().take(8).collect();
    assert_eq!(silence, vec![0.0; 8]);
    paused.store(false, Ordering::Release);
    let mut output = prefix;
    output.extend(actual);
    assert_eq!(
        output, expected,
        "resume changed native-boundary conversion after removing explicit paused silence"
    );
}
#[test]
fn seek_between_metadata_getters_waits_for_a_coherent_boundary_and_resets_format() {
    let (source, control) =
        controlled::Controlled::new(Formats { offset: 0 }, Arc::new(AtomicBool::new(false)));
    assert_eq!(source.current_span_len(), Some(8));
    let ticket = control.seek(Duration::from_secs(1));
    // A request arrives after the converter read span length, before channels/rate.
    assert_eq!(source.channels().get(), 1);
    assert_eq!(source.sample_rate().get(), 8_000);
    assert!(ticket.poll().is_none());
    let actual = converted(source);
    let mut expected = converted(Formats { offset: 0 })[..16].to_vec();
    expected.extend(converted(Formats { offset: 12 }));
    assert_eq!(actual, expected);
    assert!(ticket.poll().unwrap().is_ok());
}
#[test]
fn a_new_span_seek_changes_format_before_the_converter_reads_any_metadata() {
    let (source, control) =
        controlled::Controlled::new(Formats { offset: 0 }, Arc::new(AtomicBool::new(false)));
    let ticket = control.seek(Duration::from_secs(1));
    let actual = converted(source);
    assert_eq!(actual, converted(Formats { offset: 12 }));
    assert!(ticket.poll().unwrap().is_ok());
}
#[test]
fn exhausted_source_does_not_turn_into_infinite_silence_when_paused() {
    let paused = Arc::new(AtomicBool::new(true));
    let (source, control) = controlled::Controlled::new(Formats { offset: 24 }, paused);
    assert_eq!(source.current_span_len(), Some(0));
    assert!(converted(source).is_empty());
    assert!(control.seek(Duration::ZERO).poll().unwrap().is_err());
}

#[test]
fn production_fixed_conversion_matches_real_converter_across_native_formats() {
    let expected = converted(Formats { offset: 0 });
    let normalized = fixed::Fixed::new(
        Formats { offset: 0 },
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8_000).unwrap(),
    );
    let (source, _) =
        controlled::Controlled::new_fixed(normalized, Arc::new(AtomicBool::new(false)));
    assert_eq!(converted(source), expected);
}
#[test]
fn production_fixed_seek_resets_resampler_buffers_and_uses_new_native_format() {
    let normalized = fixed::Fixed::new(
        Formats { offset: 0 },
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8_000).unwrap(),
    );
    let (source, control) =
        controlled::Controlled::new_fixed(normalized, Arc::new(AtomicBool::new(false)));
    let mut pipeline = UniformSourceIterator::new(
        source,
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8_000).unwrap(),
    );
    let prefix: Vec<_> = pipeline.by_ref().take(4).collect();
    assert_eq!(prefix, converted(Formats { offset: 0 })[..4]);
    let ticket = control.seek(Duration::from_secs(1));
    let actual: Vec<_> = pipeline.collect();
    assert_eq!(actual, converted(Formats { offset: 12 }));
    assert!(ticket.poll().unwrap().is_ok());
}
#[derive(Clone)]
struct LargeSpan {
    offset: usize,
    unknown_span: bool,
}
impl Iterator for LargeSpan {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.offset == 80_000 {
            return None;
        }
        let sample = self.offset as f32 * 0.000_001;
        self.offset += 1;
        Some(sample)
    }
}
impl Source for LargeSpan {
    fn current_span_len(&self) -> Option<usize> {
        if self.unknown_span {
            None
        } else {
            Some(80_000 - self.offset)
        }
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(1).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(8_000).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }
    fn try_seek(&mut self, target: Duration) -> Result<(), rodio::source::SeekError> {
        self.offset = (target.as_secs() * 8_000).min(80_000) as usize;
        Ok(())
    }
}
#[test]
fn production_fixed_output_seeks_long_and_unknown_native_spans_within_an_output_frame() {
    for unknown_span in [false, true] {
        let native = LargeSpan {
            offset: 0,
            unknown_span,
        };
        let fixed = fixed::Fixed::new(
            native,
            ChannelCount::new(2).unwrap(),
            SampleRate::new(48_000).unwrap(),
        );
        let pause = Arc::new(AtomicBool::new(false));
        let (source, control) = controlled::Controlled::new_fixed(fixed, pause.clone());
        let mut pipeline = UniformSourceIterator::new(
            source,
            ChannelCount::new(2).unwrap(),
            SampleRate::new(48_000).unwrap(),
        );
        for _ in 0..138 {
            assert!(pipeline.next().is_some());
        }
        let ticket = control.seek(Duration::from_secs(2));
        let mut consumed = Vec::new();
        while ticket.poll().is_none() && consumed.len() < 3 {
            consumed.push(pipeline.next().unwrap());
        }
        assert!(ticket.poll().unwrap().is_ok());
        assert!(
            consumed.len() <= 2,
            "seek deferred to a native span instead of an output frame"
        );
        let expected: Vec<_> = UniformSourceIterator::new(
            LargeSpan {
                offset: 16_000,
                unknown_span,
            },
            ChannelCount::new(2).unwrap(),
            SampleRate::new(48_000).unwrap(),
        )
        .take(32)
        .collect();
        let mut actual = vec![*consumed.last().unwrap()];
        actual.extend(pipeline.by_ref().take(31));
        assert_eq!(actual, expected, "seek retained old interpolation samples");
        pause.store(true, Ordering::Release);
        let ticket = control.seek(Duration::from_secs(1));
        let mut consumed = 0;
        while ticket.poll().is_none() && consumed < 3 {
            assert_eq!(pipeline.next(), Some(0.0));
            consumed += 1;
        }
        assert!(ticket.poll().unwrap().is_ok());
        assert!(consumed <= 2);
    }
}
#[test]
fn production_fixed_output_ends_when_paused_after_its_last_real_sample() {
    let fixed = fixed::Fixed::new(
        Formats { offset: 22 },
        ChannelCount::new(2).unwrap(),
        SampleRate::new(16_000).unwrap(),
    );
    let pause = Arc::new(AtomicBool::new(false));
    let (mut source, control) = controlled::Controlled::new_fixed(fixed, pause.clone());
    assert!(source.next().is_some());
    assert!(source.next().is_some());
    pause.store(true, Ordering::Release);
    assert_eq!(source.current_span_len(), Some(0));
    assert_eq!(source.next(), None);
    assert!(control.seek(Duration::ZERO).poll().unwrap().is_err());
}

#[test]
fn production_fixed_pause_resume_keeps_real_converter_samples_across_both_format_changes() {
    let expected = converted(Formats { offset: 0 });
    let fixed = fixed::Fixed::new(
        Formats { offset: 0 },
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8_000).unwrap(),
    );
    let paused = Arc::new(AtomicBool::new(false));
    let (mut source, _) = controlled::Controlled::new_fixed(fixed, paused.clone());
    let mut actual: Vec<_> = source.by_ref().take(4).collect();
    paused.store(true, Ordering::Release);
    assert_eq!(source.by_ref().take(8).collect::<Vec<_>>(), vec![0.0; 8]);
    paused.store(false, Ordering::Release);
    actual.extend(source);
    assert_eq!(actual, expected);
}
#[test]
fn production_refused_seek_preserves_prefetched_resampling_samples() {
    #[derive(Clone)]
    struct Refuses(Formats);
    impl Iterator for Refuses {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            self.0.next()
        }
    }
    impl Source for Refuses {
        fn current_span_len(&self) -> Option<usize> {
            self.0.current_span_len()
        }
        fn channels(&self) -> ChannelCount {
            self.0.channels()
        }
        fn sample_rate(&self) -> SampleRate {
            self.0.sample_rate()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
        fn try_seek(&mut self, _: Duration) -> Result<(), rodio::source::SeekError> {
            Err(rodio::source::SeekError::NotSupported {
                underlying_source: "refusal fixture",
            })
        }
    }
    let expected = converted(Refuses(Formats { offset: 8 }));
    let fixed = fixed::Fixed::new(
        Refuses(Formats { offset: 8 }),
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8_000).unwrap(),
    );
    let (mut source, control) =
        controlled::Controlled::new_fixed(fixed, Arc::new(AtomicBool::new(false)));
    let mut actual: Vec<_> = source.by_ref().take(2).collect();
    let ticket = control.seek(Duration::ZERO);
    actual.extend(source);
    assert!(ticket.poll().unwrap().is_err());
    assert_eq!(actual, expected);
}
