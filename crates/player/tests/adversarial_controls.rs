//! Independent format-span and routing probes. Production modules are included
//! unchanged so these tests can exercise their private source adapters.
#[path = "../src/controlled.rs"]
mod controlled;

use rodio::{source::UniformSourceIterator, ChannelCount, SampleRate, Source};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

#[derive(Clone)]
struct ChangingChannels {
    offset: usize,
}
impl Iterator for ChangingChannels {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let samples = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let sample = samples.get(self.offset).copied();
        self.offset += usize::from(sample.is_some());
        sample
    }
}
impl Source for ChangingChannels {
    fn current_span_len(&self) -> Option<usize> {
        Some(if self.offset < 4 {
            4 - self.offset
        } else {
            8 - self.offset
        })
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(if self.offset < 4 { 1 } else { 2 }).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(8_000).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[test]
fn independent_controlled_source_preserves_dynamic_channel_spans() {
    let original = ChangingChannels { offset: 0 };
    let expected: Vec<_> = UniformSourceIterator::new(
        original.clone(),
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8_000).unwrap(),
    )
    .collect();
    let (controlled, _) = controlled::Controlled::new(original, Arc::new(AtomicBool::new(false)));
    let actual: Vec<_> = UniformSourceIterator::new(
        controlled,
        ChannelCount::new(2).unwrap(),
        SampleRate::new(8_000).unwrap(),
    )
    .collect();
    assert_eq!(
        actual, expected,
        "source wrapper must preserve Rodio's channel span boundaries"
    );
}

#[derive(Clone)]
struct ChangingRate {
    offset: usize,
}
impl Iterator for ChangingRate {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let value = (self.offset < 20).then_some(self.offset as f32 * 0.01);
        self.offset += usize::from(value.is_some());
        value
    }
}
impl Source for ChangingRate {
    fn current_span_len(&self) -> Option<usize> {
        Some(if self.offset < 4 {
            4 - self.offset
        } else {
            20 - self.offset
        })
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(1).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(if self.offset < 4 { 8_000 } else { 16_000 }).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[test]
fn independent_controlled_source_preserves_dynamic_sample_rate_spans() {
    let original = ChangingRate { offset: 0 };
    let expected: Vec<_> = UniformSourceIterator::new(
        original.clone(),
        ChannelCount::new(1).unwrap(),
        SampleRate::new(8_000).unwrap(),
    )
    .collect();
    let (controlled, _) = controlled::Controlled::new(original, Arc::new(AtomicBool::new(false)));
    let actual: Vec<_> = UniformSourceIterator::new(
        controlled,
        ChannelCount::new(1).unwrap(),
        SampleRate::new(8_000).unwrap(),
    )
    .collect();
    assert_eq!(
        actual, expected,
        "source wrapper must preserve resampling boundaries"
    );
}
