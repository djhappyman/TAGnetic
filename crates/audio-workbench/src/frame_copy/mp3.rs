//! Layer III frame copying with a bounded reservoir and synthesis lead-in.
use crate::{FrameRange, MediaError};
use rusty_mp3::encode::lametag::{tag_frame, tag_header, TagInfo, TagStats, DECODER_DELAY};
use rusty_mp3::header::{FrameHeader, MpegVersion};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use symphonia::core::codecs::CODEC_TYPE_MP3;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::{Limit, MetadataOptions};
use symphonia::core::probe::Hint;

pub(crate) mod timing;

const MAX_PACKET_BYTES: usize = 1_441;
const MAX_PACKETS: usize = 2_000_000;
const MAX_BUFFERED_METADATA: usize = 8 * 1024 * 1024;
const MAX_TRIM: u64 = 0xfff;
const SEEK_PREROLL_PACKETS: usize = 2;

#[derive(Debug)]
pub(super) struct Index {
    packets: Vec<Packet>,
    stream: FrameHeader,
    boundaries: Vec<u64>,
    delay: u64,
}

#[derive(Clone, Copy, Debug)]
struct Packet {
    offset: u64,
    len: u16,
    data_offset: u16,
    back: u16,
}

impl Packet {
    fn data_len(self) -> usize {
        usize::from(self.len - self.data_offset)
    }
}

fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), MediaError> {
    if cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

fn rejected() -> MediaError {
    MediaError::Unsupported("This MP3 layout cannot be split safely with frame copying")
}

fn layout(source: &mut File, frames: u64) -> Result<(u32, usize, u64, u64), MediaError> {
    source.rewind()?;
    let stream = MediaSourceStream::new(
        timing::source(Box::new(source.try_clone()?))?,
        MediaSourceStreamOptions::default(),
    );
    let format = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            stream,
            &FormatOptions::default(),
            &MetadataOptions {
                limit_metadata_bytes: Limit::Maximum(MAX_BUFFERED_METADATA),
                limit_visual_bytes: Limit::Maximum(MAX_BUFFERED_METADATA),
            },
        )
        .map_err(|error| MediaError::Decode(error.to_string()))?;
    let params = &format
        .format
        .default_track()
        .ok_or_else(rejected)?
        .codec_params;
    if params.codec != CODEC_TYPE_MP3 {
        return Err(rejected());
    }
    let sample_rate = params.sample_rate.ok_or_else(rejected)?;
    let channels = params.channels.ok_or_else(rejected)?.count();
    let (delay, padding) = match (params.delay, params.padding, params.n_frames) {
        (Some(delay), Some(padding), Some(raw)) => {
            let delay = u64::from(delay);
            let padding = u64::from(padding);
            if raw.checked_sub(delay + padding) != Some(frames) {
                return Err(rejected());
            }
            (delay, padding)
        }
        _ => (0, 0),
    };
    Ok((sample_rate, channels, delay, padding))
}

fn media_start(reader: &mut (impl Read + Seek), len: u64) -> Result<u64, MediaError> {
    // Consecutive leading ID3v2 tags are unusual but legal in real collections.
    let mut offset = 0;
    for _ in 0..32 {
        let mut header = [0; 10];
        reader.seek(SeekFrom::Start(offset))?;
        reader.read_exact(&mut header)?;
        if &header[..3] != b"ID3" {
            reader.seek(SeekFrom::Start(offset))?;
            return Ok(offset);
        }
        if !(2..=4).contains(&header[3]) || header[6..].iter().any(|byte| byte & 0x80 != 0) {
            return Err(rejected());
        }
        let size = header[6..]
            .iter()
            .fold(0_u64, |sum, byte| (sum << 7) | u64::from(*byte));
        let footer = if header[3] == 4 && header[5] & 0x10 != 0 {
            10
        } else {
            0
        };
        offset = offset
            .checked_add(10 + size + footer)
            .filter(|next| *next < len)
            .ok_or_else(rejected)?;
    }
    Err(rejected())
}

fn xing_at(bytes: &[u8], offset: usize) -> bool {
    bytes
        .get(offset..offset + 4)
        .is_some_and(|id| id == b"Xing" || id == b"Info")
}

fn gapless_extension(bytes: &[u8], xing: usize) -> Option<usize> {
    let flags = bytes.get(xing + 4..xing + 8)?;
    let flags = u32::from_be_bytes(flags.try_into().expect("four-byte Xing flags"));
    let fields = 4 * u32::from(flags & 0x1 != 0)
        + 4 * u32::from(flags & 0x2 != 0)
        + 100 * u32::from(flags & 0x4 != 0)
        + 4 * u32::from(flags & 0x8 != 0);
    let encoder = xing + 8 + fields as usize;
    matches!(
        bytes.get(encoder..encoder + 4),
        Some(b"LAME" | b"Lavc" | b"Lavf")
    )
    .then_some(encoder)
}

fn is_timing_packet(bytes: &[u8], header: &FrameHeader, delay: u64) -> Result<bool, MediaError> {
    // Match the timing-frame positions used by the playback demuxer.
    let xing = 4 + header.side_info_len();
    if header.crc_protected
        && [xing, xing + 2]
            .into_iter()
            .any(|offset| xing_at(bytes, offset) && bytes[6..offset].iter().all(|byte| *byte == 0))
    {
        if delay != 0 && timing::checked_crc_offset(bytes, header).is_some() {
            return Ok(true);
        }
        return Err(MediaError::Unsupported(
            "This MP3 has a CRC-protected timing header that does not match the playback timeline",
        ));
    }
    let xing_tag = xing_at(bytes, xing) && bytes[4..xing].iter().all(|byte| *byte == 0);
    if xing_tag && delay == 0 && gapless_extension(bytes, xing).is_some() {
        return Err(MediaError::Unsupported(
            "This MP3 has gapless timing that could not be verified for frame copying",
        ));
    }
    let vbri_tag = bytes.len() >= 62
        && bytes.get(36..42) == Some(b"VBRI\0\x01")
        && bytes[6..36].iter().all(|byte| *byte == 0);
    Ok(xing_tag || vbri_tag)
}

pub(super) fn scan(
    source: &mut File,
    frames: u64,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Index, MediaError> {
    stop(cancelled)?;
    let (sample_rate, channels, delay, padding) = layout(source, frames)?;
    let len = source.metadata()?.len();
    let mut reader = BufReader::new(source);
    let mut offset = media_start(&mut reader, len)?;
    let mut stream: Option<FrameHeader> = None;
    let mut bytes = [0; MAX_PACKET_BYTES];
    let mut packets = Vec::new();

    while len - offset >= 4 {
        stop(cancelled)?;
        reader.read_exact(&mut bytes[..4])?;
        let Ok(header) = FrameHeader::parse(bytes[..4].try_into().expect("four-byte MPEG header"))
        else {
            // The full decoded timeline below distinguishes tags or trailing
            // junk from a gap that hides audio we would otherwise omit.
            break;
        };
        let size = header.frame_size();
        let side = 4 + 2 * usize::from(header.crc_protected);
        let data = side + header.side_info_len();
        if header.sample_rate != sample_rate
            || header.channel_mode.channels() != channels
            || data >= size
            || size > bytes.len()
            || size as u64 > len - offset
        {
            return Err(rejected());
        }
        if stream
            .as_ref()
            .is_some_and(|stream| stream.version != header.version)
        {
            return Err(rejected());
        }
        reader.read_exact(&mut bytes[4..size])?;
        if is_timing_packet(&bytes[..size], &header, delay)? {
            // A middle timing frame could carry reservoir data. Refuse
            // concatenated streams rather than silently discarding context.
            if !packets.is_empty() || stream.is_some() {
                return Err(rejected());
            }
        } else {
            let back = if header.version == MpegVersion::V1 {
                u16::from_be_bytes([bytes[side], bytes[side + 1]]) >> 7
            } else {
                u16::from(bytes[side])
            };
            if packets.len() >= MAX_PACKETS {
                return Err(MediaError::Unsupported(
                    "This MP3 is too long to index for frame copying",
                ));
            }
            packets.push(Packet {
                offset,
                len: size as u16,
                data_offset: data as u16,
                back,
            });
        }
        stream.get_or_insert(header);
        offset += size as u64;
    }

    let stream = stream.ok_or_else(rejected)?;
    let samples = stream.version.samples_per_frame() as u64;
    let raw = (packets.len() as u64) * samples;
    if frames == 0
        || raw.checked_sub(delay + padding) != Some(frames)
        || packets.first().is_none_or(|packet| packet.back != 0)
    {
        return Err(rejected());
    }
    let mut boundaries = Vec::with_capacity(packets.len() + 1);
    boundaries.push(0);
    for packet in 1..packets.len() as u64 {
        let position = (packet * samples).saturating_sub(delay);
        if position > 0 && position < frames {
            boundaries.push(position);
        }
    }
    boundaries.push(frames);
    Ok(Index {
        packets,
        stream,
        boundaries,
        delay,
    })
}

impl Index {
    pub(super) fn boundaries(&self) -> &[u64] {
        &self.boundaries
    }

    pub(super) fn write(
        &self,
        source: &mut File,
        range: FrameRange,
        output: File,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<(), MediaError> {
        stop(cancelled)?;
        if range.start >= range.end
            || self.boundaries.binary_search(&range.start).is_err()
            || self.boundaries.binary_search(&range.end).is_err()
        {
            return Err(MediaError::Range(
                "MP3 copy requires resolved frame boundaries",
            ));
        }
        let samples = self.stream.version.samples_per_frame() as u64;
        let raw_start = self.delay + range.start;
        let raw_end = self.delay + range.end;
        let first = (raw_start as usize / samples as usize).saturating_sub(SEEK_PREROLL_PACKETS);
        let mut last = raw_end.div_ceil(samples) as usize;
        let packet = self.packets[first];
        let original_preroll = raw_start - first as u64 * samples;
        let mut reader = BufReader::new(source);
        reader.seek(SeekFrom::Start(packet.offset))?;
        let mut starting_bytes = [0; 4];
        reader.read_exact(&mut starting_bytes)?;
        let starting_header = FrameHeader::parse(starting_bytes).map_err(|_| rejected())?;
        // Some players synchronize only when the first two audio headers have
        // matching channel mode and other MPEG flags. Copy those from the actual
        // first audio frame. The original file's timing header may differ.
        let reservoir =
            read_reservoir(&mut reader, &self.packets[..first], packet.back, cancelled)?;
        let prefix = silent_packet(&starting_header, &reservoir)?;
        // Give very short files enough frames for common independent probes.
        // Prefer the original next packet where available and trim it entirely.
        if last - first < 2 && last < self.packets.len() {
            last += 1;
        }
        let trailing = last - first < 2;
        let suffix = trailing
            .then(|| silent_packet(&starting_header, &[]))
            .transpose()?;

        let delay = original_preroll + samples - DECODER_DELAY as u64;
        let padding =
            last as u64 * samples - raw_end + u64::from(trailing) * samples + DECODER_DELAY as u64;
        if delay > MAX_TRIM || padding > MAX_TRIM {
            return Err(rejected());
        }

        let mut header = tag_header(&starting_header);
        header.channel_mode = starting_header.channel_mode;
        let mut stats = TagStats::new(header.frame_size());
        let mut output = BufWriter::new(output);
        output.write_all(&vec![0; header.frame_size()])?;
        output.write_all(&prefix)?;
        stats.add_frame(&prefix);
        reader.seek(SeekFrom::Start(packet.offset))?;
        let mut bytes = [0; MAX_PACKET_BYTES];
        let mut bytes_written = (header.frame_size() + prefix.len()) as u64;
        for packet in &self.packets[first..last] {
            stop(cancelled)?;
            let bytes = &mut bytes[..usize::from(packet.len)];
            reader.read_exact(bytes)?;
            let frame_header =
                FrameHeader::parse(bytes[..4].try_into().expect("four-byte MPEG header"))
                    .map_err(|_| rejected())?;
            if frame_header.frame_size() != bytes.len() {
                return Err(rejected());
            }
            output.write_all(bytes)?;
            stats.add_frame(bytes);
            bytes_written += bytes.len() as u64;
        }
        if let Some(suffix) = &suffix {
            output.write_all(suffix)?;
            stats.add_frame(suffix);
            bytes_written += suffix.len() as u64;
        }
        if bytes_written > u64::from(u32::MAX) {
            return Err(MediaError::Unsupported(
                "An MP3 frame-copy output cannot exceed 4 GiB",
            ));
        }
        let tag = tag_frame(
            &header,
            &stats,
            &TagInfo {
                vbr: true,
                delay: delay as usize,
                padding: padding as usize,
            },
        );
        output.rewind()?;
        output.write_all(&tag)?;
        output.flush()?;
        Ok(())
    }
}

fn read_reservoir(
    reader: &mut BufReader<&mut File>,
    previous: &[Packet],
    back: u16,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Vec<u8>, MediaError> {
    let mut remaining = usize::from(back);
    let mut content = vec![0; remaining];
    for packet in previous.iter().rev().take(usize::from(back)) {
        stop(cancelled)?;
        let count = remaining.min(packet.data_len());
        let position = packet.offset + u64::from(packet.len) - count as u64;
        reader.seek(SeekFrom::Start(position))?;
        reader.read_exact(&mut content[remaining - count..remaining])?;
        remaining -= count;
        if remaining == 0 {
            break;
        }
    }
    if remaining != 0 {
        return Err(rejected());
    }
    Ok(content)
}

fn silent_packet(stream: &FrameHeader, reservoir: &[u8]) -> Result<Vec<u8>, MediaError> {
    use rusty_mp3::tables::{BITRATE_V1_L3, BITRATE_V2_L3};
    let mut header = stream.clone();
    header.crc_protected = false;
    header.padding = false;
    let bitrate_table = match header.version {
        MpegVersion::V1 => BITRATE_V1_L3,
        _ => BITRATE_V2_L3,
    };
    let required = 4 + header.side_info_len() + reservoir.len();
    for bitrate in bitrate_table[1..15].iter().copied() {
        header.bitrate_kbps = bitrate;
        let size = header.frame_size();
        if size >= required {
            let mut packet = vec![0; size];
            packet[..4].copy_from_slice(&header.to_bytes());
            packet[size - reservoir.len()..].copy_from_slice(reservoir);
            return Ok(packet);
        }
    }
    Err(rejected())
}

#[cfg(test)]
mod tests;
