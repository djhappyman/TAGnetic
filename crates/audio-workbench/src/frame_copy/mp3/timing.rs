//! A decoder-facing view of LAME's CRC-protected Xing/Info frame.
use super::{gapless_extension, media_start, xing_at, MAX_PACKET_BYTES};
use crate::MediaError;
use rusty_mp3::encode::lametag::crc16;
use rusty_mp3::header::FrameHeader;
use std::io::{self, Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;

pub(super) fn checked_crc_offset(bytes: &[u8], header: &FrameHeader) -> Option<usize> {
    let xing = 4 + header.side_info_len();
    if !header.crc_protected
        || !xing_at(bytes, xing)
        || bytes[6..xing].iter().any(|byte| *byte != 0)
    {
        return None;
    }
    let extension = gapless_extension(bytes, xing)?;
    let offset = extension + 34;
    let stored = bytes.get(offset..offset + 2)?.try_into().ok()?;
    if crc16(0, &bytes[..offset]) != u16::from_be_bytes(stored) {
        return None;
    }
    Some(offset)
}

/// LAME's protected timing frame reserves the MPEG CRC bytes before otherwise
/// all-zero side information. The decoder expects all zero there to recognize
/// the Xing/Info frame. Change its protection flag and preserve its valid LAME
/// checksum in this read view. Audio frames and source bytes stay intact.
pub(crate) fn source(
    mut original: Box<dyn MediaSource>,
) -> Result<Box<dyn MediaSource>, MediaError> {
    let Some(len) = original.byte_len() else {
        return Ok(original);
    };
    let patch = read_patch(&mut original, len)?;
    original.rewind()?;
    match patch {
        Some((offset, data)) => Ok(Box::new(TimingView {
            original,
            offset,
            data,
            pos: 0,
        })),
        None => Ok(original),
    }
}

fn read_patch(
    original: &mut Box<dyn MediaSource>,
    len: u64,
) -> Result<Option<(u64, Vec<u8>)>, MediaError> {
    let offset = match media_start(original, len) {
        Ok(offset) => offset,
        Err(MediaError::Unsupported(_)) => return Ok(None),
        Err(MediaError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    let mut first = [0; 4];
    original.read_exact(&mut first)?;
    let Ok(header) = FrameHeader::parse(first) else {
        return Ok(None);
    };
    let size = header.frame_size();
    if !header.crc_protected
        || !(4..=MAX_PACKET_BYTES).contains(&size)
        || size as u64 > len - offset
    {
        return Ok(None);
    }
    let mut bytes = vec![0; size];
    bytes[..4].copy_from_slice(&first);
    original.read_exact(&mut bytes[4..])?;
    let Some(checksum) = checked_crc_offset(&bytes, &header) else {
        return Ok(None);
    };
    bytes[1] |= 1;
    bytes[4..6].fill(0);
    let crc = crc16(0, &bytes[..checksum]);
    bytes[checksum..checksum + 2].copy_from_slice(&crc.to_be_bytes());
    Ok(Some((offset, bytes)))
}

struct TimingView {
    original: Box<dyn MediaSource>,
    offset: u64,
    data: Vec<u8>,
    pos: u64,
}

impl Read for TimingView {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = self.original.read(buf)?;
        let start = self.pos.max(self.offset);
        let end = (self.pos + count as u64).min(self.offset + self.data.len() as u64);
        if end > start {
            let count = (end - start) as usize;
            let dst = (start - self.pos) as usize;
            let src = (start - self.offset) as usize;
            buf[dst..dst + count].copy_from_slice(&self.data[src..src + count]);
        }
        self.pos += count as u64;
        Ok(count)
    }
}

impl Seek for TimingView {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.pos = self.original.seek(pos)?;
        Ok(self.pos)
    }
}

impl MediaSource for TimingView {
    fn is_seekable(&self) -> bool {
        self.original.is_seekable()
    }

    fn byte_len(&self) -> Option<u64> {
        self.original.byte_len()
    }
}
