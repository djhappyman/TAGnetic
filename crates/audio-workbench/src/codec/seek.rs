use super::{check_cancelled, MediaError, ProbedSource, SourceInfo};
use rusty_mp3::header::FrameHeader;
use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::Hasher;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::time::SystemTime;

const MAX_POINTS: usize = 8192;
const FIRST_INTERVAL: u64 = 65_536;
const MAX_PACKET_BYTES: usize = 1_441;

/// Sparse MP3 packet locations learned from a verified recording.
///
/// Pass this index back only with a file for the same recording. An index is
/// optional, and callers can use the ordinary decoder when no index is available.
#[derive(Debug)]
pub struct SeekIndex {
    pub(super) source: ProbedSource,
    pub(super) delay: u32,
    len: u64,
    modified: SystemTime,
    points: Vec<Point>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Point {
    pub(super) raw_frame: u64,
    pub(super) byte: u64,
}

impl SeekIndex {
    pub(super) fn point(&self, file: &File, frame: u64) -> Option<Point> {
        let index = self
            .points
            .partition_point(|point| point.raw_frame - u64::from(self.delay) <= frame);
        let point = *self.points.get(index.checked_sub(1)?)?;
        let metadata = file.metadata().ok()?;
        (metadata.len() == self.len && metadata.modified().ok()? == self.modified).then_some(point)
    }
}

pub(super) struct Builder {
    file: File,
    first_byte: u64,
    byte: u64,
    raw_frame: u64,
    delay: u32,
    interval: u64,
    next: u64,
    len: u64,
    modified: SystemTime,
    packets: u64,
    hash: DefaultHasher,
    points: Vec<Point>,
}

impl Builder {
    pub(super) fn new(file: &File, first_byte: u64, delay: u32) -> Option<Self> {
        let metadata = file.metadata().ok()?;
        let len = metadata.len();
        if first_byte >= len {
            return None;
        }
        Some(Self {
            file: file.try_clone().ok()?,
            first_byte,
            byte: first_byte,
            raw_frame: 0,
            delay,
            interval: FIRST_INTERVAL,
            next: FIRST_INTERVAL,
            len,
            modified: metadata.modified().ok()?,
            packets: 0,
            hash: DefaultHasher::new(),
            points: Vec::new(),
        })
    }

    pub(super) fn push(&mut self, bytes: &[u8], raw_frames: u64) {
        if self.raw_frame >= self.next {
            if self.points.len() == MAX_POINTS {
                let mut index = 0;
                self.points.retain(|_| {
                    index += 1;
                    index % 2 == 1
                });
                self.interval *= 2;
            }
            self.points.push(Point {
                raw_frame: self.raw_frame,
                byte: self.byte,
            });
            self.next = self.raw_frame + self.interval;
        }
        self.hash.write_usize(bytes.len());
        self.hash.write(bytes);
        self.packets += 1;
        self.byte += bytes.len() as u64;
        self.raw_frame += raw_frames;
    }

    pub(super) fn finish(
        self,
        source: ProbedSource,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<SeekIndex>, MediaError> {
        if self.points.is_empty() || self.byte > self.len {
            return Ok(None);
        }
        let mut reader = BufReader::with_capacity(128 * 1024, &self.file);
        // Compare the contiguous frame scan with the packets consumed by the
        // trusted demuxer. Gaps, skipped timing frames and resyncs retain the
        // normal seek path rather than using a potentially shifted index.
        match self.verify(&mut reader, source.source, cancelled) {
            Err(MediaError::Cancelled) => return Err(MediaError::Cancelled),
            Ok(true) => {}
            _ => return Ok(None),
        }
        Ok(Some(SeekIndex {
            source,
            delay: self.delay,
            len: self.len,
            modified: self.modified,
            points: self.points,
        }))
    }

    fn verify(
        &self,
        reader: &mut (impl Read + Seek),
        source: SourceInfo,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<bool, MediaError> {
        reader.seek(SeekFrom::Start(self.first_byte))?;
        let mut bytes = [0; MAX_PACKET_BYTES];
        let mut hash = DefaultHasher::new();
        for _ in 0..self.packets {
            check_cancelled(cancelled)?;
            reader.read_exact(&mut bytes[..4])?;
            let Ok(header) = FrameHeader::parse(bytes[..4].try_into().expect("four byte header"))
            else {
                return Ok(false);
            };
            let len = header.frame_size();
            if !(4..=bytes.len()).contains(&len)
                || header.sample_rate != source.sample_rate
                || header.channel_mode.channels() != usize::from(source.channels)
            {
                return Ok(false);
            }
            reader.read_exact(&mut bytes[4..len])?;
            hash.write_usize(len);
            hash.write(&bytes[..len]);
        }
        Ok(self.hash.finish() == hash.finish())
    }
}
