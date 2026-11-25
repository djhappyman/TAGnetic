//! Split encoded audio at reviewed codec frame boundaries.

mod flac;
mod mp3;
pub(crate) use mp3::timing::source as mp3_timing_view;

use crate::{FrameRange, MediaError, RenderProgress, RenderSegment};
use std::fs::File;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyCodec {
    Flac,
    Mp3,
}

enum Frames {
    Flac(flac::Index),
    Mp3(mp3::Index),
}

/// Index of safe positions and their encoded data in one verified recording.
/// Offsets are valid only for the source used to build this index.
pub struct FrameCopyIndex(Frames);

impl FrameCopyIndex {
    pub fn codec(&self) -> CopyCodec {
        match &self.0 {
            Frames::Flac(_) => CopyCodec::Flac,
            Frames::Mp3(_) => CopyCodec::Mp3,
        }
    }

    pub fn scan(
        source: &mut File,
        codec: CopyCodec,
        expected_frames: u64,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, MediaError> {
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        let frames = match codec {
            CopyCodec::Flac => Frames::Flac(flac::scan(source, expected_frames, &mut cancelled)?),
            CopyCodec::Mp3 => Frames::Mp3(mp3::scan(source, expected_frames, &mut cancelled)?),
        };
        let index = Self(frames);
        let cut_points = index.boundaries();
        if expected_frames == 0
            || cut_points.first() != Some(&0)
            || cut_points.last() != Some(&expected_frames)
        {
            return Err(MediaError::Range(
                "The encoded frames disagree with the recording timeline",
            ));
        }
        Ok(index)
    }

    fn boundaries(&self) -> &[u64] {
        match &self.0 {
            Frames::Flac(index) => index.boundaries(),
            Frames::Mp3(index) => index.boundaries(),
        }
    }

    pub fn nearest(&self, requested: u64) -> u64 {
        nearest_boundary(self.boundaries(), requested)
    }

    pub fn write(
        &self,
        mut source: File,
        segments: &[RenderSegment],
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(RenderProgress),
        mut open: impl FnMut(&Path) -> std::io::Result<File>,
    ) -> Result<(), MediaError> {
        let cuts = self.boundaries();
        let mut last = 0;
        let mut total_frames = 0;
        for segment in segments {
            let FrameRange { start, end } = segment.range;
            if start < last
                || start >= end
                || cuts.binary_search(&start).is_err()
                || cuts.binary_search(&end).is_err()
            {
                return Err(MediaError::Range(
                    "Copy ranges need distinct, ordered frame boundaries",
                ));
            }
            total_frames += end - start;
            last = end;
        }
        let mut completed_frames = 0;
        for (i, segment) in segments.iter().enumerate() {
            if cancelled() {
                return Err(MediaError::Cancelled);
            }
            let output = open(&segment.output)?;
            match &self.0 {
                Frames::Flac(index) => {
                    index.write(&mut source, segment.range, output, &mut cancelled)?
                }
                Frames::Mp3(index) => {
                    index.write(&mut source, segment.range, output, &mut cancelled)?
                }
            }
            completed_frames += segment.range.end - segment.range.start;
            progress(RenderProgress {
                source_frame: segment.range.end,
                completed_frames,
                total_frames,
                completed_segments: i + 1,
                total_segments: segments.len(),
            });
        }
        if cancelled() {
            return Err(MediaError::Cancelled);
        }
        Ok(())
    }
}

fn nearest_boundary(cuts: &[u64], requested: u64) -> u64 {
    let next = cuts.partition_point(|frame| *frame < requested);
    let before = cuts[next.saturating_sub(1)];
    let after = cuts.get(next).copied().unwrap_or(before);
    if requested.saturating_sub(before) <= after.saturating_sub(requested) {
        before
    } else {
        after
    }
}

#[cfg(test)]
mod tests {
    use super::nearest_boundary;

    #[test]
    fn snaps_consistently_and_picks_the_earlier_frame_on_ties() {
        let cuts = [0, 20, 40, 58];
        assert_eq!(nearest_boundary(&cuts, 0), 0);
        assert_eq!(nearest_boundary(&cuts, 19), 20);
        assert_eq!(nearest_boundary(&cuts, 30), 20);
        assert_eq!(nearest_boundary(&cuts, 31), 40);
        assert_eq!(nearest_boundary(&cuts, 58), 58);
    }
}
