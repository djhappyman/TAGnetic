use super::{check_cancelled, seek, FrameCount, MediaError, ProbedSource, SourceInfo};
use std::fs::File;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::*;
use symphonia::core::errors::Error;
use symphonia::core::formats::{
    util::trim_packet, FormatOptions, FormatReader, Packet, SeekMode, SeekTo,
};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions, ReadBytes};
use symphonia::core::meta::{Limit, MetadataOptions};
use symphonia::core::probe::Hint;

pub(super) const MAX_FRAMES: u64 = (1 << 36) - 1;
const MAX_PACKET_FRAMES: u64 = 65_536;
const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;
// More than enough MP3 granules for the bit reservoir and synthesis
// filters to settle before the first frame displayed in the tile.
const MP3_PREROLL_FRAMES: u64 = 16_384;

pub(super) struct Recording {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    source: ProbedSource,
    expected_frames: Option<u64>,
    position: u64,
    samples: Option<SampleBuffer<i32>>,
    can_seek_lossless: bool,
    is_mp3: bool,
    seek_builder: Option<seek::Builder>,
    mp3_seek: Option<Mp3Seek>,
}

struct Mp3Seek {
    raw_start: u64,
    delay: u32,
}

impl Recording {
    pub(super) fn open(file: File) -> Result<Self, MediaError> {
        Self::open_file(file, false)
    }

    pub(super) fn open_for_analysis(file: File, build_seek: bool) -> Result<Self, MediaError> {
        Self::open_file(file, build_seek)
    }

    pub(super) fn take_seek_builder(&mut self) -> Option<seek::Builder> {
        self.seek_builder.take()
    }

    fn open_file(file: File, build_seek: bool) -> Result<Self, MediaError> {
        let mut reader = probe(file.try_clone()?, false)?;
        let mut gapless = false;
        // Symphonia treats an estimated MP3 duration as a trim boundary when
        // gapless is enabled. Enable it only for streams with valid delay and
        // padding metadata, never for old MP3s without a Xing/LAME header.
        if let Some(track) = reader
            .default_track()
            .filter(|track| track.codec_params.codec == CODEC_TYPE_MP3)
        {
            let params = &track.codec_params;
            if let (Some(delay), Some(padding), Some(frames)) =
                (params.delay, params.padding, params.n_frames)
            {
                if u64::from(delay) + u64::from(padding) > frames {
                    return Err(MediaError::Decode(
                        "The MP3 delay and padding exceed the recording length".into(),
                    ));
                }
                drop(reader);
                reader = probe(file.try_clone()?, true)?;
                gapless = true;
            }
        }
        let mut seek_builder = None;
        if build_seek {
            if let Some(track) = reader
                .default_track()
                .filter(|track| track.codec_params.codec == CODEC_TYPE_MP3)
            {
                let delay = if gapless {
                    track.codec_params.delay.unwrap_or(0)
                } else {
                    0
                };
                // A probed MP3 reader is positioned at its first audio
                // packet, after any metadata or normalized Xing/Info frame.
                // The buffered stream exposes the exact consumed byte offset.
                let first_byte = reader.into_inner().pos();
                seek_builder = seek::Builder::new(&file, first_byte, delay);
                reader = probe(file, gapless)?;
            }
        }
        let track = reader
            .default_track()
            .ok_or_else(|| MediaError::Decode("No audio stream found".into()))?;
        let params = &track.codec_params;
        let integer = match params.codec {
            CODEC_TYPE_MP3 | CODEC_TYPE_PCM_F32LE | CODEC_TYPE_PCM_F32BE | CODEC_TYPE_PCM_F64LE
            | CODEC_TYPE_PCM_F64BE | CODEC_TYPE_PCM_ALAW | CODEC_TYPE_PCM_MULAW => false,
            CODEC_TYPE_FLAC | CODEC_TYPE_PCM_U8 | CODEC_TYPE_PCM_S8 | CODEC_TYPE_PCM_S16LE
            | CODEC_TYPE_PCM_S16BE | CODEC_TYPE_PCM_S24LE | CODEC_TYPE_PCM_S24BE
            | CODEC_TYPE_PCM_S32LE | CODEC_TYPE_PCM_S32BE => true,
            _ => {
                return Err(MediaError::Unsupported(
                    "Choose MP3, FLAC, PCM WAV or PCM AIFF audio",
                ))
            }
        };
        let sample_rate = params
            .sample_rate
            .filter(|rate| (1..=655_350).contains(rate))
            .ok_or(MediaError::Unsupported(
                "The recording has an invalid sample rate",
            ))?;
        let channels = params
            .channels
            .map(|layout| layout.count())
            .filter(|channels| (1..=8).contains(channels))
            .ok_or(MediaError::Unsupported(
                "The recording has an unsupported channel layout",
            ))?;
        let bits_per_sample = if integer {
            Some(
                params
                    .bits_per_sample
                    .and_then(|bits| u8::try_from(bits).ok())
                    .filter(|bits| (1..=32).contains(bits))
                    .ok_or(MediaError::Unsupported(
                        "The recording has an invalid sample precision",
                    ))?,
            )
        } else {
            None
        };
        let is_mp3 = params.codec == CODEC_TYPE_MP3;
        let expected_frames = params
            .n_frames
            .filter(|frames| (!is_mp3 || gapless) && *frames != 0);
        if expected_frames.is_some_and(|frames| frames > MAX_FRAMES) {
            return Err(MediaError::Unsupported(
                "The recording has too many audio frames",
            ));
        }
        let can_seek_lossless = !is_mp3
            && params.start_ts == 0
            && params
                .time_base
                .is_some_and(|base| base.numer == 1 && base.denom == sample_rate);
        let decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions::default())
            .map_err(decode_error)?;
        let info = SourceInfo {
            sample_rate,
            channels: channels as u16,
            frames: params
                .n_frames
                .filter(|frames| *frames <= MAX_FRAMES)
                .unwrap_or(0),
            bits_per_sample,
        };
        let frame_count = if expected_frames.is_some() {
            FrameCount::Declared
        } else if info.frames != 0 {
            FrameCount::Estimated
        } else {
            FrameCount::Unknown
        };
        let source = ProbedSource {
            source: info,
            frame_count,
        };
        let track = track.id;
        Ok(Self {
            reader,
            decoder,
            track,
            source,
            expected_frames,
            position: 0,
            samples: None,
            can_seek_lossless,
            is_mp3,
            seek_builder,
            mp3_seek: None,
        })
    }

    pub(super) fn open_at(
        file: File,
        frame: u64,
        index: Option<&seek::SeekIndex>,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Self, MediaError> {
        if frame == 0 {
            return Self::open(file);
        }
        let preroll_start = frame.saturating_sub(MP3_PREROLL_FRAMES);
        let indexed = index.and_then(|index| {
            index
                .point(&file, preroll_start)
                .map(|point| (index, point))
        });
        let mut recording = match indexed {
            Some((index, point)) => match Self::open_mp3_suffix(file.try_clone()?, index, point) {
                Ok(recording) => recording,
                Err(_) => Self::open(file.try_clone()?)?,
            },
            None => Self::open(file.try_clone()?)?,
        };
        if recording.is_mp3 {
            // Read only compressed packets up to preroll, whether the
            // recording started from a verified point or from its beginning.
            while recording.position < preroll_start {
                let packet = recording
                    .read_packet(cancelled)?
                    .ok_or(MediaError::Range("The tile starts after the recording"))?;
                recording.position += packet.dur();
            }
            return Ok(recording);
        }
        if !recording.can_seek_lossless {
            return Ok(recording);
        }
        if let Ok(seeked) = recording.reader.seek(
            SeekMode::Accurate,
            SeekTo::TimeStamp {
                ts: frame,
                track_id: recording.track,
            },
        ) {
            if seeked.actual_ts <= frame {
                recording.decoder.reset();
                recording.position = seeked.actual_ts;
                return Ok(recording);
            }
        }
        drop(recording);
        Self::open(file)
    }

    fn open_mp3_suffix(
        mut file: File,
        index: &seek::SeekIndex,
        point: seek::Point,
    ) -> Result<Self, MediaError> {
        file.seek(SeekFrom::Start(point.byte))?;
        let reader = symphonia::default::formats::MpaReader::try_new(
            MediaSourceStream::new(
                Box::new(Mp3Suffix(file)),
                MediaSourceStreamOptions::default(),
            ),
            &FormatOptions::default(),
        )
        .map_err(decode_error)?;
        let track = reader
            .default_track()
            .ok_or_else(|| MediaError::Decode("No audio stream found".into()))?;
        let params = &track.codec_params;
        let source = index.source;
        if params.codec != CODEC_TYPE_MP3
            || params.sample_rate != Some(source.source.sample_rate)
            || params.channels.map(|channels| channels.count())
                != Some(usize::from(source.source.channels))
        {
            return Err(MediaError::Decode("The MP3 seek point changed".into()));
        }
        let track = track.id;
        let decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions::default())
            .map_err(decode_error)?;
        Ok(Self {
            reader: Box::new(reader),
            decoder,
            track,
            source,
            expected_frames: Some(source.source.frames),
            position: point.raw_frame - u64::from(index.delay),
            samples: None,
            can_seek_lossless: false,
            is_mp3: true,
            seek_builder: None,
            mp3_seek: Some(Mp3Seek {
                raw_start: point.raw_frame,
                delay: index.delay,
            }),
        })
    }

    pub(super) fn position(&self) -> u64 {
        self.position
    }

    pub(super) fn info(&self) -> SourceInfo {
        self.source.source
    }

    pub(super) fn probed_source(&self) -> ProbedSource {
        self.source
    }

    fn read_packet(
        &mut self,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<Packet>, MediaError> {
        loop {
            check_cancelled(cancelled)?;
            let mut packet = match self.reader.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                    if self
                        .expected_frames
                        .is_some_and(|expected| expected != self.position)
                    {
                        return Err(MediaError::Decode(
                            "The audio ended before its declared length".into(),
                        ));
                    }
                    return Ok(None);
                }
                Err(error) => return Err(decode_error(error)),
            };
            if packet.track_id() != self.track {
                continue;
            }
            if let Some(seek) = &self.mp3_seek {
                packet.ts += seek.raw_start;
                trim_packet(&mut packet, seek.delay, self.expected_frames);
            }
            if packet.dur() > 0 && packet.ts() != self.position {
                return Err(MediaError::Decode(
                    "Audio timestamps are discontinuous".into(),
                ));
            }
            if packet.dur() > MAX_PACKET_FRAMES {
                return Err(MediaError::Unsupported(
                    "The recording contains an oversized audio block",
                ));
            }
            if let Some(builder) = &mut self.seek_builder {
                builder.push(&packet.data, packet.block_dur());
            }
            return Ok(Some(packet));
        }
    }

    pub(super) fn next(
        &mut self,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<&[i32]>, MediaError> {
        while let Some(packet) = self.read_packet(cancelled)? {
            let channels = usize::from(self.info().channels);
            let sample_rate = self.info().sample_rate;
            let decoded = self.decoder.decode(&packet).map_err(decode_error)?;
            if decoded.frames() as u64 != packet.dur() {
                return Err(MediaError::Decode(
                    "The decoded audio block has an unexpected length".into(),
                ));
            }
            if decoded.frames() == 0 {
                continue;
            }
            if decoded.spec().rate != sample_rate || decoded.spec().channels.count() != channels {
                return Err(MediaError::Unsupported(
                    "The sample rate or channel layout changes inside the recording",
                ));
            }
            if decoded.capacity() as u64 > MAX_PACKET_FRAMES {
                return Err(MediaError::Unsupported(
                    "The recording contains an oversized audio block",
                ));
            }
            let frame_count = decoded.frames() as u64;
            if frame_count == 0 {
                continue;
            }
            self.position = self
                .position
                .checked_add(frame_count)
                .filter(|end| *end <= MAX_FRAMES)
                .ok_or(MediaError::Unsupported(
                    "The recording has too many audio frames",
                ))?;
            let capacity = decoded.capacity() * channels;
            if self
                .samples
                .as_ref()
                .is_none_or(|samples| samples.capacity() < capacity)
            {
                self.samples = Some(SampleBuffer::new(
                    decoded.capacity() as u64,
                    *decoded.spec(),
                ));
            }
            let buffer = self
                .samples
                .as_mut()
                .ok_or_else(|| MediaError::Decode("Audio buffer is unavailable".into()))?;
            buffer.copy_interleaved_ref(decoded);
            return Ok(Some(buffer.samples()));
        }
        Ok(None)
    }

    pub(super) fn discard_before(
        &mut self,
        start: u64,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<usize>, MediaError> {
        let first = self.position;
        if self.next(cancelled)?.is_none() || self.position <= start {
            return Ok(None);
        }
        Ok(Some((start - first) as usize))
    }

    pub(super) fn current_samples(&self) -> &[i32] {
        self.samples.as_ref().map_or(&[], SampleBuffer::samples)
    }
}

// The demuxer only needs forward reads after a known boundary. Reporting
// no seek capability also skips the arbitrary-suffix bitrate estimate.
struct Mp3Suffix(File);

impl Read for Mp3Suffix {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Seek for Mp3Suffix {
    fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
        Err(io::Error::from(ErrorKind::Unsupported))
    }
}

impl MediaSource for Mp3Suffix {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

fn probe(mut file: File, enable_gapless: bool) -> Result<Box<dyn FormatReader>, MediaError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(MediaError::Unsupported("Choose a regular audio file"));
    }
    file.rewind()?;
    let stream = MediaSourceStream::new(
        crate::frame_copy::mp3_timing_view(super::aiff::source(file, metadata.len())?)?,
        MediaSourceStreamOptions::default(),
    );
    let format_options = FormatOptions {
        enable_gapless,
        ..FormatOptions::default()
    };
    let metadata_options = MetadataOptions {
        limit_metadata_bytes: Limit::Maximum(MAX_METADATA_BYTES),
        limit_visual_bytes: Limit::Maximum(MAX_METADATA_BYTES),
    };
    symphonia::default::get_probe()
        .format(&Hint::new(), stream, &format_options, &metadata_options)
        .map(|probed| probed.format)
        .map_err(decode_error)
}

fn decode_error(error: Error) -> MediaError {
    MediaError::Decode(error.to_string())
}
