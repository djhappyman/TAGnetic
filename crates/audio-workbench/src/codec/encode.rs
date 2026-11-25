use super::{check_cancelled, MediaError, OutputFormat, SourceInfo};
use flacenc::bitsink::ByteSink;
use flacenc::component::{BitRepr, Stream, StreamInfo};
use flacenc::error::{Verified, Verify};
use flacenc::source::{Context, Fill, FrameBuf};
use rusty_mp3::{Mp3Encoder, Mp3EncoderConfig};
use std::fs::File;
use std::io::{BufWriter, Seek, Write};

const FLAC_BLOCK_FRAMES: usize = 4096;
const FLAC_HEADER_BYTES: usize = 42;
// Every MP3 push is drained immediately. Bound upstream working queues to
// a few MP3 frames even when the input decoder returns a large block.
const MP3_BLOCK_FRAMES: usize = 1152;
const MPEG_RATES: [u32; 9] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];

pub(super) fn check_output(source: &SourceInfo, format: OutputFormat) -> Result<(), MediaError> {
    match format {
        OutputFormat::Flac => {
            if !(8..=24).contains(&source.bits_per_sample.unwrap_or(24)) {
                return Err(MediaError::Unsupported(
                    "FLAC export preserves integer precision from 8 to 24 bits",
                ));
            }
            if !(1..=8).contains(&source.channels)
                || !(1..=96_000).contains(&source.sample_rate)
                || (source.sample_rate > u32::from(u16::MAX)
                    && !source.sample_rate.is_multiple_of(10))
            {
                return Err(MediaError::Unsupported(
                    "FLAC export requires one to eight channels and a supported sample rate up to 96 kHz",
                ));
            }
        }
        OutputFormat::Mp3 { bitrate_kbps } => {
            if !(1..=2).contains(&source.channels) || !MPEG_RATES.contains(&source.sample_rate) {
                return Err(MediaError::Unsupported(
                    "MP3 export requires mono or stereo at a standard MP3 sample rate",
                ));
            }
            if !(32..=320).contains(&bitrate_kbps) {
                return Err(MediaError::Unsupported(
                    "Choose an MP3 target bitrate from 32 to 320 kbps",
                ));
            }
            // MPEG-2/2.5 cannot encode targets above 160 kbps. A standard
            // 128/192/256/320 preset remains visible, with a clear error.
            if source.sample_rate < 32_000 && bitrate_kbps > 160 {
                return Err(MediaError::Unsupported(
                    "MP3 recordings below 32 kHz support up to 160 kbps. Choose a lower bitrate.",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn flac_bits(source: &SourceInfo) -> u8 {
    match source.bits_per_sample.unwrap_or(24) {
        0..=8 => 8,
        9..=12 => 12,
        13..=16 => 16,
        17..=20 => 20,
        _ => 24,
    }
}

pub(super) enum SegmentWriter {
    Flac(Box<FlacWriter>),
    Mp3(Box<Mp3Writer>),
}

impl SegmentWriter {
    pub(super) fn new(
        file: File,
        source: &SourceInfo,
        format: OutputFormat,
    ) -> Result<Self, MediaError> {
        let file = BufWriter::new(file);
        match format {
            OutputFormat::Flac => Ok(Self::Flac(Box::new(FlacWriter::new(file, source)?))),
            OutputFormat::Mp3 { bitrate_kbps } => Ok(Self::Mp3(Box::new(Mp3Writer {
                file,
                encoder: Mp3Encoder::new(Mp3EncoderConfig {
                    bitrate_kbps: u32::from(bitrate_kbps),
                    // Upstream's MPEG-1 CBR mode banks all frames to manage its
                    // reservoir. VBR exposes each packet incrementally.
                    vbr_quality: Some(f32::from(bitrate_kbps)),
                }),
                channels: source.channels,
                sample_rate: source.sample_rate,
                samples: Vec::with_capacity(MP3_BLOCK_FRAMES * usize::from(source.channels)),
            }))),
        }
    }

    pub(super) fn push(
        &mut self,
        samples: &[i32],
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<(), MediaError> {
        match self {
            Self::Flac(writer) => writer.push(samples, cancelled),
            Self::Mp3(writer) => writer.push(samples, cancelled),
        }
    }

    pub(super) fn finish(self, cancelled: &mut impl FnMut() -> bool) -> Result<(), MediaError> {
        check_cancelled(cancelled)?;
        match self {
            Self::Flac(writer) => writer.finish(),
            Self::Mp3(writer) => writer.finish(cancelled),
        }
    }
}

pub(super) struct FlacWriter {
    file: BufWriter<File>,
    config: Verified<flacenc::config::Encoder>,
    info: StreamInfo,
    context: Context,
    frame_buf: FrameBuf,
    sink: ByteSink,
    frame_number: usize,
    block_samples: usize,
    shift: usize,
    pending: Vec<i32>,
}

impl FlacWriter {
    fn new(mut file: BufWriter<File>, source: &SourceInfo) -> Result<Self, MediaError> {
        let channels = usize::from(source.channels);
        let bits = usize::from(flac_bits(source));
        let config = flacenc::config::Encoder::default()
            .into_verified()
            .map_err(|(_, error)| encode_error(error))?;
        let info =
            StreamInfo::new(source.sample_rate as usize, channels, bits).map_err(encode_error)?;
        let frame_buf = FrameBuf::with_size(channels, FLAC_BLOCK_FRAMES).map_err(encode_error)?;
        let block_samples = FLAC_BLOCK_FRAMES * channels;
        file.write_all(&[0; FLAC_HEADER_BYTES])?;
        Ok(Self {
            file,
            config,
            info,
            context: Context::new(bits, channels),
            frame_buf,
            sink: ByteSink::new(),
            frame_number: 0,
            block_samples,
            shift: 32 - bits,
            pending: Vec::with_capacity(block_samples),
        })
    }

    fn push(
        &mut self,
        mut samples: &[i32],
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<(), MediaError> {
        while !samples.is_empty() {
            let count = (self.block_samples - self.pending.len()).min(samples.len());
            self.pending
                .extend(samples[..count].iter().map(|sample| sample >> self.shift));
            samples = &samples[count..];
            if self.pending.len() == self.block_samples {
                check_cancelled(cancelled)?;
                self.write_block()?;
            }
        }
        Ok(())
    }

    fn write_block(&mut self) -> Result<(), MediaError> {
        self.frame_buf
            .fill_interleaved(&self.pending)
            .map_err(encode_error)?;
        self.context
            .fill_interleaved(&self.pending)
            .map_err(encode_error)?;
        self.pending.clear();
        let frame = flacenc::encode_fixed_size_frame(
            &self.config,
            &self.frame_buf,
            self.frame_number,
            &self.info,
        )
        .map_err(encode_error)?;
        self.info.update_frame_info(&frame);
        self.sink.clear();
        frame.write(&mut self.sink).map_err(encode_error)?;
        self.file.write_all(self.sink.as_slice())?;
        self.frame_number += 1;
        Ok(())
    }

    fn finish(mut self) -> Result<(), MediaError> {
        if !self.pending.is_empty() {
            self.write_block()?;
        }
        // FLAC STREAMINFO excludes the final partial block from the fixed
        // block size. The final frame can legitimately be one sample long.
        self.info
            .set_block_sizes(FLAC_BLOCK_FRAMES, FLAC_BLOCK_FRAMES)
            .map_err(encode_error)?;
        self.info.set_md5_digest(&self.context.md5_digest());
        self.info.set_total_samples(self.context.total_samples());
        let header = Stream::with_stream_info(self.info);
        self.sink.clear();
        header.write(&mut self.sink).map_err(encode_error)?;
        if self.sink.as_slice().len() != FLAC_HEADER_BYTES {
            return Err(MediaError::Encode(
                "The FLAC header has an unexpected size".into(),
            ));
        }
        self.file.rewind()?;
        self.file.write_all(self.sink.as_slice())?;
        self.file.flush()?;
        Ok(())
    }
}

pub(super) struct Mp3Writer {
    file: BufWriter<File>,
    encoder: Mp3Encoder,
    channels: u16,
    sample_rate: u32,
    samples: Vec<f32>,
}

impl Mp3Writer {
    fn push(
        &mut self,
        samples: &[i32],
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<(), MediaError> {
        for chunk in samples.chunks(MP3_BLOCK_FRAMES * usize::from(self.channels)) {
            check_cancelled(cancelled)?;
            self.samples.clear();
            self.samples.extend(
                chunk
                    .iter()
                    .map(|&sample| sample as f32 * (1.0 / 2_147_483_648.0)),
            );
            self.encoder
                .push_pcm_f32(&self.samples, self.channels, self.sample_rate)
                .map_err(encode_error)?;
            self.drain(cancelled)?;
        }
        Ok(())
    }

    fn drain(&mut self, cancelled: &mut impl FnMut() -> bool) -> Result<(), MediaError> {
        loop {
            check_cancelled(cancelled)?;
            match self.encoder.next_packet() {
                Ok(packet) => self.file.write_all(&packet)?,
                Err(rusty_mp3::Error::Again | rusty_mp3::Error::Eof) => return Ok(()),
                Err(error) => return Err(encode_error(error)),
            }
        }
    }

    fn finish(mut self, cancelled: &mut impl FnMut() -> bool) -> Result<(), MediaError> {
        self.encoder.finish();
        self.drain(cancelled)?;
        let tag = self.encoder.lame_tag_frame().ok_or_else(|| {
            MediaError::Encode("The MP3 delay and padding header is missing".into())
        })?;
        self.file.rewind()?;
        self.file.write_all(tag)?;
        self.file.flush()?;
        Ok(())
    }
}

fn encode_error(error: impl std::fmt::Display) -> MediaError {
    MediaError::Encode(error.to_string())
}
