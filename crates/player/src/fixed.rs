//! Consumer-owned conversion to the physical output format. Uses Rodio's real
//! rate/channel converters and native span boundaries. A successful seek rebuilds
//! their interpolation/repeat buffers before another output sample is emitted.
use rodio::{
    conversions::{ChannelCountConverter, SampleRateConverter},
    Source,
};
use std::time::Duration;
struct NativeSpan<S> {
    source: S,
    remaining: Option<usize>,
}
impl<S: Source> Iterator for NativeSpan<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.remaining == Some(0) {
            return None;
        }
        if let Some(remaining) = &mut self.remaining {
            *remaining -= 1;
        }
        self.source.next()
    }
}
type Converter<S> = ChannelCountConverter<SampleRateConverter<NativeSpan<S>>>;
pub(super) struct Fixed<S: Source> {
    converter: Option<Converter<S>>,
    channels: rodio::ChannelCount,
    rate: rodio::SampleRate,
    duration: Option<Duration>,
    next: Option<f32>,
}
impl<S: Source> Fixed<S> {
    pub fn new(source: S, channels: rodio::ChannelCount, rate: rodio::SampleRate) -> Self {
        let duration = source.total_duration();
        let converter = Self::convert(source, channels, rate);
        let mut fixed = Self {
            converter: Some(converter),
            channels,
            rate,
            duration,
            next: None,
        };
        fixed.next = fixed.pull();
        fixed
    }
    fn convert(source: S, channels: rodio::ChannelCount, rate: rodio::SampleRate) -> Converter<S> {
        let remaining = source.current_span_len().map(|n| n.min(32_768));
        let native_channels = source.channels();
        let native_rate = source.sample_rate();
        ChannelCountConverter::new(
            SampleRateConverter::new(
                NativeSpan { source, remaining },
                native_rate,
                rate,
                native_channels,
            ),
            native_channels,
            channels,
        )
    }
    fn pull(&mut self) -> Option<f32> {
        if let Some(sample) = self.converter.as_mut()?.next() {
            return Some(sample);
        }
        let source = self.converter.take()?.into_inner().into_inner().source;
        let mut next = Self::convert(source, self.channels, self.rate);
        let sample = next.next();
        self.converter = Some(next);
        sample
    }
}
impl<S: Source> Iterator for Fixed<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = self.next.take()?;
        self.next = self.pull();
        Some(sample)
    }
}
impl<S: Source> Source for Fixed<S> {
    fn current_span_len(&self) -> Option<usize> {
        if self.next.is_none() {
            Some(0)
        } else {
            None
        }
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }
    fn try_seek(&mut self, target: Duration) -> Result<(), rodio::source::SeekError> {
        let Some(converter) = self.converter.as_mut() else {
            return Err(rodio::source::SeekError::Other(std::sync::Arc::new(
                std::io::Error::other("No converter remains"),
            )));
        };
        // On refusal, retain all conversion buffers and the prefetched output
        // sample. Discarding these on an unchanged source would skip audio.
        converter.inner_mut().inner_mut().source.try_seek(target)?;
        let Some(converter) = self.converter.take() else {
            return Err(rodio::source::SeekError::Other(std::sync::Arc::new(
                std::io::Error::other("No converter remains"),
            )));
        };
        let source = converter.into_inner().into_inner().source;
        self.converter = Some(Self::convert(source, self.channels, self.rate));
        self.next = self.pull();
        Ok(())
    }
}
impl<S: Source> super::controlled::FixedFormat for Fixed<S> {}
