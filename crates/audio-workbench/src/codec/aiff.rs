use super::MediaError;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use symphonia::core::io::MediaSource;

const MAX_CHUNKS: usize = 65_536;

/// Expose AIFF sample data to Symphonia 0.5.5 without copying the recording.
/// Its reader expects zero offset and block size, and counts the SSND header
/// itself as audio. Strip the padding from the read view and give the reader
/// the payload length so decoding and seeking end on the actual last sample.
pub(super) fn source(mut file: File, file_len: u64) -> Result<Box<dyn MediaSource>, MediaError> {
    if file_len < 12 {
        return Ok(Box::new(file));
    }
    let mut form = [0; 12];
    file.read_exact(&mut form)?;
    file.rewind()?;
    if &form[..4] != b"FORM" || !matches!(&form[8..], b"AIFF" | b"AIFC") {
        return Ok(Box::new(file));
    }
    let form_size = u32::from_be_bytes([form[4], form[5], form[6], form[7]]);
    let end = 8 + u64::from(form_size);
    if end > file_len {
        return Err(MediaError::Decode(
            "The AIFF file is shorter than its FORM chunk".into(),
        ));
    }
    let mut position = 12;
    for _ in 0..MAX_CHUNKS {
        if position + 8 > end {
            break;
        }
        file.seek(SeekFrom::Start(position))?;
        let mut chunk = [0; 8];
        file.read_exact(&mut chunk)?;
        let size = u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        let next = position + 8 + u64::from(size);
        if next > end {
            return Err(MediaError::Decode(
                "An AIFF chunk extends past its FORM".into(),
            ));
        }
        if &chunk[..4] == b"SSND" {
            let payload_size = size
                .checked_sub(8)
                .ok_or_else(|| MediaError::Decode("The AIFF sound header is incomplete".into()))?;
            let mut sound_header = [0; 8];
            file.read_exact(&mut sound_header)?;
            let padding = u32::from_be_bytes([
                sound_header[0],
                sound_header[1],
                sound_header[2],
                sound_header[3],
            ]);
            let audio_size = payload_size.checked_sub(padding).ok_or_else(|| {
                MediaError::Decode("The AIFF sound offset extends past its chunk".into())
            })?;
            file.rewind()?;
            return Ok(Box::new(AiffSource {
                file,
                file_len: file_len - u64::from(padding),
                position: 0,
                audio_start: position + 16,
                padding: u64::from(padding),
                patches: [
                    (4, (form_size - padding).to_be_bytes()),
                    (position + 4, audio_size.to_be_bytes()),
                    (position + 8, [0; 4]),
                    (position + 12, [0; 4]),
                ],
            }));
        }
        position = next + (u64::from(size) & 1);
    }
    Err(MediaError::Decode(
        "The AIFF sound chunk could not be found".into(),
    ))
}

struct AiffSource {
    file: File,
    file_len: u64,
    position: u64,
    audio_start: u64,
    padding: u64,
    patches: [(u64, [u8; 4]); 4],
}

impl Read for AiffSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.file_len || buffer.is_empty() {
            return Ok(0);
        }
        let header = self.position < self.audio_start;
        let boundary = if header {
            self.audio_start
        } else {
            self.file_len
        };
        let count = buffer
            .len()
            .min(usize::try_from(boundary - self.position).unwrap_or(usize::MAX));
        if self.position == self.audio_start && self.padding != 0 {
            self.file
                .seek(SeekFrom::Start(self.audio_start + self.padding))?;
        }
        let count = self.file.read(&mut buffer[..count])?;
        if header {
            let new_position = self.position + count as u64;
            for (offset, bytes) in &self.patches {
                let start = self.position.max(*offset);
                let end = new_position.min(offset + 4);
                if start < end {
                    let read_offset = (start - self.position) as usize;
                    let patch_offset = (start - offset) as usize;
                    let length = (end - start) as usize;
                    buffer[read_offset..read_offset + length]
                        .copy_from_slice(&bytes[patch_offset..patch_offset + length]);
                }
            }
        }
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for AiffSource {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::End(delta) => self.file_len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        };
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "Invalid AIFF seek position");
        let target = target.ok_or_else(invalid)?;
        let physical = if target < self.audio_start {
            target
        } else {
            target.checked_add(self.padding).ok_or_else(invalid)?
        };
        self.file.seek(SeekFrom::Start(physical))?;
        self.position = target;
        Ok(target)
    }
}

impl MediaSource for AiffSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.file_len)
    }
}
