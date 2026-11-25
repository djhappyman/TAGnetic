use crate::{FrameRange, MediaError};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use symphonia::core::checksum::{Crc16Ansi, Crc8Ccitt};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions, Monitor};
use symphonia::default::formats::FlacReader;

const STREAM_INFO_LEN: usize = 34;
const PREFIX_LEN: usize = 8 + STREAM_INFO_LEN;
const SAMPLE_COUNT_MASK: u64 = (1 << 36) - 1;
const MAX_PACKETS: usize = 2_000_000;
const MAX_PACKET_BYTES: usize = 0x00ff_ffff;
const MAX_METADATA_BLOCKS: usize = 65_536;

pub(super) struct Index {
    stream_info: StreamInfo,
    boundaries: Vec<u64>,
    offsets: Vec<u64>,
    variable_blocks: bool,
}

#[derive(Clone, Copy)]
struct StreamInfo([u8; STREAM_INFO_LEN]);

impl StreamInfo {
    fn prefix(self) -> [u8; PREFIX_LEN] {
        let mut bytes = [0; PREFIX_LEN];
        bytes[..8].copy_from_slice(b"fLaC\x80\x00\x00\x22");
        bytes[8..].copy_from_slice(&self.0);
        bytes
    }

    fn output_prefix(mut self, samples: u64, full_stream: bool) -> [u8; PREFIX_LEN] {
        // STREAMINFO's audio settings and block bounds still apply. Clear the
        // frame byte bounds because length can change with the new numbering.
        self.0[4..10].fill(0);
        let fields = self.audio_fields() & !SAMPLE_COUNT_MASK;
        self.0[10..18].copy_from_slice(&(fields | samples).to_be_bytes());
        if !full_stream {
            // RFC 9639 defines zero as unknown. The master hash describes
            // decoded audio for the whole master, never a selected portion.
            self.0[18..].fill(0);
        }
        self.prefix()
    }

    fn audio_fields(self) -> u64 {
        u64::from_be_bytes(self.0[10..18].try_into().expect("fixed STREAMINFO width"))
    }

    fn min_block(self) -> u64 {
        u64::from(u16::from_be_bytes([self.0[0], self.0[1]]))
    }

    fn max_block(self) -> u64 {
        u64::from(u16::from_be_bytes([self.0[2], self.0[3]]))
    }

    fn sample_rate(self) -> u64 {
        self.audio_fields() >> 44
    }

    fn channels(self) -> u64 {
        ((self.audio_fields() >> 41) & 7) + 1
    }

    fn bits_per_sample(self) -> u64 {
        ((self.audio_fields() >> 36) & 31) + 1
    }
}

/// Give the packet demuxer only STREAMINFO and the audio bytes. Existing tags
/// and seek tables describe the master and are handled separately by Studio.
/// The sequential packet reader only seeks within its own buffered bytes.
struct AudioPackets {
    prefix: Cursor<[u8; PREFIX_LEN]>,
    source: File,
    length: u64,
}

impl Read for AudioPackets {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.prefix.position() < PREFIX_LEN as u64 {
            self.prefix.read(bytes)
        } else {
            self.source.read(bytes)
        }
    }
}

impl Seek for AudioPackets {
    fn seek(&mut self, _position: SeekFrom) -> io::Result<u64> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

impl MediaSource for AudioPackets {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.length)
    }
}

pub(super) fn scan(
    source: &mut File,
    frames: u64,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Index, MediaError> {
    check_cancelled(cancelled)?;
    let length = source.metadata()?.len();
    let (info, audio_start) = stream_info(source, length, cancelled)?;
    let declared_frames = info.audio_fields() & SAMPLE_COUNT_MASK;
    if !(1..=SAMPLE_COUNT_MASK).contains(&frames)
        || (declared_frames != 0 && declared_frames != frames)
        || !(16..=info.max_block()).contains(&info.min_block())
        || info.sample_rate() == 0
        || info.bits_per_sample() < 4
    {
        return Err(bad_flac("FLAC STREAMINFO disagrees with the recording"));
    }

    let packets = AudioPackets {
        prefix: Cursor::new(info.prefix()),
        source: source.try_clone()?,
        length: PREFIX_LEN as u64 + length - audio_start,
    };
    let stream = MediaSourceStream::new(Box::new(packets), MediaSourceStreamOptions::default());
    let mut reader = FlacReader::try_new(stream, &FormatOptions::default())
        .map_err(|error| MediaError::Decode(error.to_string()))?;
    let mut position = 0;
    let mut offset = audio_start;
    let mut boundaries = vec![0];
    let mut offsets = vec![offset];
    let mut previous_strategy = None;
    while offset < length {
        check_cancelled(cancelled)?;
        if offsets.len() > MAX_PACKETS {
            return Err(MediaError::Unsupported(
                "This FLAC has too many compressed frames for frame copying",
            ));
        }
        let packet = match reader.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(bad_flac("FLAC has an incomplete or invalid frame"));
            }
            Err(error) => return Err(MediaError::Decode(error.to_string())),
        };
        let header = frame_header(&packet.data, info)?;
        let expected_number = if header.variable {
            position
        } else {
            (offsets.len() - 1) as u64
        };
        let next = position + header.samples;
        if header.number != expected_number
            || previous_strategy.is_some_and(|previous| previous != header.variable)
            || packet.ts != position
            || packet.dur != header.samples
            || header.samples > info.max_block()
            || (next < frames && header.samples < info.min_block())
            || (!header.variable
                && (info.min_block() != info.max_block()
                    || (next < frames && header.samples != info.max_block())))
            || next > frames
        {
            return Err(bad_flac("FLAC frames have inconsistent sample positions"));
        }
        position = next;
        offset = offset
            .checked_add(packet.data.len() as u64)
            .filter(|end| *end <= length)
            .ok_or_else(|| bad_flac("FLAC frames extend beyond the recording"))?;
        boundaries.push(position);
        offsets.push(offset);
        previous_strategy = Some(header.variable);
    }
    if position != frames {
        return Err(bad_flac(
            "FLAC frame count disagrees with the decoded length",
        ));
    }
    Ok(Index {
        stream_info: info,
        boundaries,
        offsets,
        variable_blocks: previous_strategy.unwrap_or(false),
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
        check_cancelled(cancelled)?;
        let first = self.boundary(range.start)?;
        let last = self.boundary(range.end)?;
        if first >= last {
            return Err(MediaError::Range("FLAC copy needs a nonempty range"));
        }
        source.seek(SeekFrom::Start(self.offsets[first]))?;
        let mut source = BufReader::new(source);
        let mut output = BufWriter::new(output);
        let samples = range.end - range.start;
        let full_stream = first == 0 && last + 1 == self.offsets.len();
        output.write_all(&self.stream_info.output_prefix(samples, full_stream))?;

        let mut packet = Vec::new();
        for frame in first..last {
            check_cancelled(cancelled)?;
            let length = (self.offsets[frame + 1] - self.offsets[frame]) as usize;
            packet.resize(length, 0);
            source.read_exact(&mut packet)?;

            let header = frame_header(&packet, self.stream_info)?;
            let expected_number = if self.variable_blocks {
                self.boundaries[frame]
            } else {
                frame as u64
            };
            if header.variable != self.variable_blocks
                || header.number != expected_number
                || header.samples != self.boundaries[frame + 1] - self.boundaries[frame]
            {
                return Err(bad_flac("FLAC frame layout changed after review"));
            }

            let number = if self.variable_blocks {
                self.boundaries[frame] - range.start
            } else {
                (frame - first) as u64
            };
            write_packet(&mut output, &packet, header, number)?;
        }
        check_cancelled(cancelled)?;
        output.flush()?;
        Ok(())
    }

    fn boundary(&self, sample: u64) -> Result<usize, MediaError> {
        self.boundaries
            .binary_search(&sample)
            .map_err(|_| MediaError::Range("FLAC copy needs a compressed frame boundary"))
    }
}

fn bad_flac(message: &str) -> MediaError {
    MediaError::Decode(message.into())
}

fn check_cancelled(cancelled: &mut impl FnMut() -> bool) -> Result<(), MediaError> {
    if cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}

fn stream_info(
    source: &mut File,
    length: u64,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(StreamInfo, u64), MediaError> {
    source.rewind()?;
    let mut marker = [0; 4];
    source.read_exact(&mut marker)?;
    if &marker[..3] == b"ID3" {
        // Some otherwise native FLAC files carry a leading ID3v2 tag.
        source.rewind()?;
        let mut tag = [0; 10];
        source.read_exact(&mut tag)?;
        if !(2..=4).contains(&tag[3]) || tag[6..].iter().any(|byte| *byte & 0x80 != 0) {
            return Err(bad_flac("FLAC has an invalid leading ID3 tag"));
        }
        let tag_size = tag[6..]
            .iter()
            .fold(0, |size, byte| size * 128 + u64::from(*byte));
        let footer_size = if tag[3] == 4 && tag[5] & 0x10 != 0 {
            10
        } else {
            0
        };
        source.seek(SeekFrom::Start(10 + tag_size + footer_size))?;
        source.read_exact(&mut marker)?;
    }
    if &marker != b"fLaC" {
        return Err(MediaError::Unsupported(
            "Frame copying requires a native FLAC stream",
        ));
    }
    let mut header = [0; 4];
    source.read_exact(&mut header)?;
    if header[0] & 0x7f != 0 || header[1..] != [0, 0, STREAM_INFO_LEN as u8] {
        return Err(bad_flac("FLAC must start with one STREAMINFO block"));
    }
    let mut info = [0; STREAM_INFO_LEN];
    source.read_exact(&mut info)?;
    let mut position = source.stream_position()?;
    for _ in 0..MAX_METADATA_BLOCKS {
        if header[0] & 0x80 != 0 {
            return if position < length {
                Ok((StreamInfo(info), position))
            } else {
                Err(bad_flac("FLAC has no audio frames"))
            };
        }
        check_cancelled(cancelled)?;
        source.read_exact(&mut header)?;
        if matches!(header[0] & 0x7f, 0 | 127) {
            return Err(bad_flac("FLAC contains an invalid metadata block"));
        }
        position += 4 + u64::from(u32::from_be_bytes([0, header[1], header[2], header[3]]));
        if position >= length {
            return Err(bad_flac("FLAC metadata exceeds the recording length"));
        }
        source.seek(SeekFrom::Start(position))?;
    }
    Err(bad_flac("FLAC has too many metadata blocks"))
}

#[derive(Clone, Copy)]
struct FrameHeader {
    variable: bool,
    number: u64,
    number_end: usize,
    crc_position: usize,
    samples: u64,
}

fn frame_header(packet: &[u8], info: StreamInfo) -> Result<FrameHeader, MediaError> {
    if !(9..=MAX_PACKET_BYTES).contains(&packet.len())
        || packet[0] != 0xff
        || !matches!(packet[1], 0xf8 | 0xf9)
        || packet[3] & 1 != 0
    {
        return Err(bad_flac("Invalid FLAC frame header"));
    }
    let variable = packet[1] == 0xf9;
    let (number, number_len) = coded_number(&packet[4..])
        .filter(|(number, _)| {
            *number
                <= if variable {
                    SAMPLE_COUNT_MASK
                } else {
                    0x7fff_ffff
                }
        })
        .ok_or_else(|| bad_flac("Invalid FLAC frame number"))?;
    let number_end = 4 + number_len;
    let block = packet[2] >> 4;
    let rate = packet[2] & 0x0f;
    let block_extra = match block {
        6 => 1,
        7 => 2,
        _ => 0,
    };
    let rate_extra = match rate {
        12 => 1,
        13 | 14 => 2,
        _ => 0,
    };
    let crc_position = number_end + block_extra + rate_extra;
    if packet.len() < crc_position + 4
        || crc8(&packet[..crc_position]) != packet[crc_position]
        || crc16(&packet[..packet.len() - 2]) != packet[packet.len() - 2..]
    {
        return Err(bad_flac("FLAC frame checksum is invalid"));
    }
    let samples = match block {
        1 => 192,
        2..=5 => 576 << (block - 2),
        6 => u64::from(packet[number_end]) + 1,
        7 => {
            u64::from(u16::from_be_bytes([
                packet[number_end],
                packet[number_end + 1],
            ])) + 1
        }
        8..=15 => 256 << (block - 8),
        _ => return Err(bad_flac("FLAC has an invalid block size")),
    };
    let rate_start = number_end + block_extra;
    let sample_rate = match rate {
        0 => info.sample_rate(),
        1..=11 => [
            88_200, 176_400, 192_000, 8_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 96_000,
        ][(rate - 1) as usize],
        12 => u64::from(packet[rate_start]) * 1000,
        13 | 14 => {
            let value = u64::from(u16::from_be_bytes([
                packet[rate_start],
                packet[rate_start + 1],
            ]));
            if rate == 14 {
                value * 10
            } else {
                value
            }
        }
        _ => return Err(bad_flac("FLAC has an invalid sample rate")),
    };
    let channels = match packet[3] >> 4 {
        count @ 0..=7 => u64::from(count) + 1,
        8..=10 => 2,
        _ => return Err(bad_flac("FLAC has an invalid channel assignment")),
    };
    let bits = match (packet[3] >> 1) & 7 {
        0 => info.bits_per_sample(),
        1 => 8,
        2 => 12,
        4 => 16,
        5 => 20,
        6 => 24,
        7 => 32,
        _ => return Err(bad_flac("FLAC has an invalid sample precision")),
    };
    if sample_rate != info.sample_rate()
        || channels != info.channels()
        || bits != info.bits_per_sample()
    {
        return Err(bad_flac("FLAC frame audio settings differ from STREAMINFO"));
    }
    Ok(FrameHeader {
        variable,
        number,
        number_end,
        crc_position,
        samples,
    })
}

fn coded_number(bytes: &[u8]) -> Option<(u64, usize)> {
    let first = *bytes.first()?;
    if first < 0x80 {
        return Some((u64::from(first), 1));
    }
    let count = first.leading_ones() as usize;
    if !(2..=7).contains(&count) {
        return None;
    }
    let mut number = u64::from(first & (0x7f >> count));
    for tail in bytes.get(1..count)? {
        if *tail & 0xc0 != 0x80 {
            return None;
        }
        number = (number << 6) | u64::from(*tail & 0x3f);
    }
    (number_size(number) == count).then_some((number, count))
}

fn number_size(number: u64) -> usize {
    match number {
        0..=0x7f => 1,
        0x80..=0x7ff => 2,
        0x800..=0xffff => 3,
        0x1_0000..=0x1f_ffff => 4,
        0x20_0000..=0x3ff_ffff => 5,
        0x400_0000..=0x7fff_ffff => 6,
        _ => 7,
    }
}

fn write_number(mut number: u64, target: &mut [u8]) -> usize {
    let count = number_size(number);
    for tail in target[1..count].iter_mut().rev() {
        *tail = 0x80 | (number as u8 & 0x3f);
        number >>= 6;
    }
    let prefix = if count == 1 { 0 } else { 0xff << (8 - count) };
    target[0] = prefix | number as u8;
    count
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = Crc8Ccitt::new(0);
    crc.process_buf_bytes(bytes);
    crc.crc()
}

fn crc16(bytes: &[u8]) -> [u8; 2] {
    let mut crc = Crc16Ansi::new(0);
    crc.process_buf_bytes(bytes);
    crc.crc().to_be_bytes()
}

fn write_packet(
    output: &mut impl Write,
    source: &[u8],
    header: FrameHeader,
    number: u64,
) -> io::Result<()> {
    if number == header.number {
        return output.write_all(source);
    }
    let mut patched = [0; 16];
    patched[..4].copy_from_slice(&source[..4]);
    let extra = &source[header.number_end..header.crc_position];
    let after_number = 4 + write_number(number, &mut patched[4..]);
    let new_crc = after_number + extra.len();
    patched[after_number..new_crc].copy_from_slice(extra);
    patched[new_crc] = crc8(&patched[..new_crc]);
    let new_header = &patched[..=new_crc];
    let payload = &source[header.crc_position + 1..source.len() - 2];

    let mut checksum = Crc16Ansi::new(0);
    checksum.process_buf_bytes(new_header);
    checksum.process_buf_bytes(payload);
    output.write_all(new_header)?;
    output.write_all(payload)?;
    output.write_all(&checksum.crc().to_be_bytes())
}

#[cfg(test)]
mod tests;
