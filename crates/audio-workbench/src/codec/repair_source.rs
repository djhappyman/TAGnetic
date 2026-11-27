//! Bounded, aligned source PCM for previews, analysis and renders.

use super::{
    check_cancelled, restoration::interpolated_sample, treatment::mask::RepairSpectra,
    AudioTreatment, MediaError, PreviewDecoder, Recording, RepairChannel, SeekIndex, SourceInfo,
    SpectralRepair,
};
use std::{collections::VecDeque, fs::File};

const MAX_MASK_WINDOW: u64 = 4096;
const COMPACT_AT: usize = 4096;

/// A source decoder whose optional spectral stage preserves the input timeline.
/// Seeking or changing decoder packet size does not change a mask's FFT grid.
pub struct RepairSource {
    decoder: PreviewDecoder,
    repairs: Option<BufferedRepairs>,
    position: u64,
}

/// Dry input and optional, sample-aligned mask-treated audio.
#[derive(Clone, Copy)]
pub struct RepairBlock<'a> {
    pub dry: &'a [i32],
    pub repaired: Option<&'a [i32]>,
}

impl<'a> RepairBlock<'a> {
    pub fn samples(&self) -> &'a [i32] {
        self.repaired.unwrap_or(self.dry)
    }
}

impl RepairSource {
    /// Open a source at the requested first frame. Source-coordinate repairs need
    /// a bounded lookbehind and lookahead around that point.
    pub fn open(
        source: File,
        start: u64,
        index: Option<&SeekIndex>,
        settings: &AudioTreatment,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Self, MediaError> {
        if start >= super::decode::MAX_FRAMES {
            return Err(MediaError::Range(
                "Preview starts beyond the supported timeline",
            ));
        }
        let decode_start = if settings.mask_repairs().next().is_some()
            || settings.click_repairs().next().is_some()
        {
            start.saturating_sub(MAX_MASK_WINDOW)
        } else {
            start
        };
        let decoder = PreviewDecoder::open_indexed(source, decode_start, index, cancelled)?;
        Self::new(decoder, start, decode_start, settings)
    }

    pub(super) fn from_recording(
        recording: Recording,
        settings: &AudioTreatment,
    ) -> Result<Self, MediaError> {
        Self::new(
            PreviewDecoder {
                recording,
                start: 0,
            },
            0,
            0,
            settings,
        )
    }

    fn new(
        decoder: PreviewDecoder,
        start: u64,
        decode_start: u64,
        settings: &AudioTreatment,
    ) -> Result<Self, MediaError> {
        let clicks: Vec<_> = settings.click_repairs().cloned().collect();
        let repairs = if settings.mask_repairs().next().is_none() && clicks.is_empty() {
            None
        } else {
            let info = decoder.source_info();
            settings.output_info_with_frames(info, None)?;
            let processor = RepairSpectra::new(info, settings.mask_repairs().cloned());
            Some(BufferedRepairs::new(
                processor,
                clicks,
                usize::from(info.channels),
                start,
                decode_start,
            ))
        };
        Ok(Self {
            decoder,
            repairs,
            position: start,
        })
    }

    pub fn source_info(&self) -> SourceInfo {
        self.decoder.source_info()
    }

    pub(super) fn position(&self) -> u64 {
        self.position
    }

    pub fn next_block(
        &mut self,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<RepairBlock<'_>>, MediaError> {
        let channels = usize::from(self.decoder.source_info().channels);
        let block = if let Some(repairs) = &mut self.repairs {
            repairs.next_block(&mut self.decoder, self.position, &mut cancelled)?
        } else {
            self.decoder
                .next_block(&mut cancelled)?
                .map(|dry| RepairBlock {
                    dry,
                    repaired: None,
                })
        };
        if let Some(block) = &block {
            self.position += (block.dry.len() / channels) as u64;
        }
        Ok(block)
    }
}

struct WindowDelta {
    start: i64,
    samples: Vec<f64>,
}

struct BufferedRepairs {
    spectra: RepairSpectra,
    clicks: Vec<SpectralRepair>,
    stock: Vec<i32>,
    stock_start: u64,
    channels: usize,
    eof: bool,
    next_center: i64,
    windows: VecDeque<WindowDelta>,
    recycled_windows: Vec<Vec<f64>>,
    repaired: Vec<i32>,
}

impl BufferedRepairs {
    fn new(
        spectra: RepairSpectra,
        clicks: Vec<SpectralRepair>,
        channels: usize,
        start: u64,
        decode_start: u64,
    ) -> Self {
        let hop = spectra.size() as u64 / 4;
        let first_hop = start / hop * hop;
        Self {
            spectra,
            clicks,
            stock: Vec::new(),
            stock_start: decode_start,
            channels,
            eof: false,
            next_center: first_hop as i64 - hop as i64,
            windows: VecDeque::with_capacity(4),
            recycled_windows: Vec::with_capacity(4),
            repaired: Vec::new(),
        }
    }

    fn stock_end(&self) -> u64 {
        self.stock_start + (self.stock.len() / self.channels) as u64
    }

    fn next_block(
        &mut self,
        decoder: &mut PreviewDecoder,
        position: u64,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Option<RepairBlock<'_>>, MediaError> {
        check_cancelled(cancelled)?;
        let size = self.spectra.size() as u64;
        let hop = size / 4;
        let boundary = (position / hop + 1) * hop;
        let spectral = self.spectra.has_spectral();
        let click_context_end = self
            .clicks
            .iter()
            .filter(|edit| edit.start_frame < boundary && edit.end_frame > position)
            .map(|edit| edit.end_frame + 2)
            .max();
        let next_stock_frame = if spectral {
            boundary + 3 * hop
        } else {
            boundary
        }
        .max(click_context_end.unwrap_or_default());
        let retain_from = position.saturating_sub(size);
        let removable = (retain_from.saturating_sub(self.stock_start) as usize * self.channels)
            .min(self.stock.len());
        if removable >= COMPACT_AT {
            self.stock.drain(..removable);
            self.stock_start += (removable / self.channels) as u64;
        }
        while !self.eof && self.stock_end() < next_stock_frame {
            match decoder.next_block(&mut *cancelled)? {
                Some(pcm) => self.stock.extend_from_slice(pcm),
                None => self.eof = true,
            }
        }
        let end = boundary.min(self.stock_end());
        if end <= position {
            return Ok(None);
        }
        if spectral {
            while self
                .windows
                .front()
                .is_some_and(|window| window.start + size as i64 <= position as i64)
            {
                if let Some(window) = self.windows.pop_front() {
                    self.recycled_windows.push(window.samples);
                }
            }
            let half = size as i64 / 2;
            while self.next_center - half < boundary as i64 {
                let mut samples = self.recycled_windows.pop().unwrap_or_default();
                self.spectra.window_delta(
                    self.next_center,
                    self.stock_start,
                    &self.stock,
                    &mut samples,
                );
                if samples.is_empty() {
                    self.recycled_windows.push(samples);
                } else {
                    self.windows.push_back(WindowDelta {
                        start: self.next_center - half,
                        samples,
                    });
                }
                self.next_center += hop as i64;
            }
        }
        let lo = (position - self.stock_start) as usize * self.channels;
        let hi = (end - self.stock_start) as usize * self.channels;
        let dry = &self.stock[lo..hi];
        if click_context_end.is_none()
            && self.windows.is_empty()
            && !self.spectra.has_broadband(position..end)
        {
            return Ok(Some(RepairBlock {
                dry,
                repaired: None,
            }));
        }
        self.repaired.clear();
        for (index, &sample) in dry.iter().enumerate() {
            let frame = position + (index / self.channels) as u64;
            let channel = index % self.channels;
            if click_context_end.is_some() {
                if let Some(sample) = self.interpolate(frame, channel) {
                    self.repaired.push(sample);
                    continue;
                }
            }
            let delta = self
                .windows
                .iter()
                .filter_map(|window| {
                    let offset = usize::try_from(frame as i64 - window.start).ok()?;
                    window.samples.get(offset * self.channels + channel)
                })
                .sum();
            self.repaired
                .push(self.spectra.render_sample(frame, channel, sample, delta));
        }
        Ok(Some(RepairBlock {
            dry,
            repaired: Some(&self.repaired),
        }))
    }
    fn interpolate(&self, frame: u64, channel: usize) -> Option<i32> {
        let edit = self.clicks.iter().rev().find(|edit| {
            (edit.start_frame..edit.end_frame).contains(&frame)
                && edit
                    .channel
                    .is_none_or(|target| usize::from(target == RepairChannel::Right) == channel)
        })?;
        let pcm = |at: u64| {
            let index = at.checked_sub(self.stock_start)? as usize * self.channels + channel;
            self.stock.get(index).copied()
        };
        let anchors = [
            pcm(edit.start_frame - 2)?,
            pcm(edit.start_frame - 1)?,
            pcm(edit.end_frame)?,
            pcm(edit.end_frame + 1)?,
        ];
        // Interpolate from original boundary samples. A simultaneous spectral
        // edit still applies around the click, while the interpolation owns
        // its exact samples and keeps the original impulse out of the result.
        Some(interpolated_sample(
            anchors,
            (frame - edit.start_frame) as usize,
            (edit.end_frame - edit.start_frame) as usize,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        render_treated_file, FrameRange, MaskOperation, OutputFormat, PreparedTreatment,
        RenderSegment, RepairChannel, RepairMask, RepairMaskPoint, RepairMaskRegion,
        SpectralRepair, MAX_SPECTRAL_REPAIRS,
    };
    use std::{
        f64::consts::TAU,
        fs::OpenOptions,
        io::{Seek, Write},
        ops::Range,
        sync::Arc,
    };

    const RATE: u32 = 48_000;
    const FRAMES: u64 = RATE as u64;
    const SOURCE: SourceInfo = SourceInfo {
        sample_rate: RATE,
        channels: 2,
        frames: FRAMES,
        bits_per_sample: Some(32),
    };

    fn signal_file() -> File {
        let length = FRAMES as u32 * 8;
        let mut file = tempfile::tempfile().unwrap();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(length + 36).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 8).to_le_bytes());
        wav.extend_from_slice(&8_u16.to_le_bytes());
        wav.extend_from_slice(&32_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&length.to_le_bytes());
        for frame in 0..FRAMES {
            let tone = |frequency: f64| {
                (TAU * frequency * frame as f64 / f64::from(RATE)).sin() * 400_000_000.0
            };
            let left = (tone(1000.0) + tone(3000.0)).round() as i32;
            let right = (tone(1800.0) + tone(1000.0)).round() as i32;
            wav.extend_from_slice(&left.to_le_bytes());
            wav.extend_from_slice(&right.to_le_bytes());
        }
        file.write_all(&wav).unwrap();
        file.rewind().unwrap();
        file
    }

    fn frame(ms: u64) -> u64 {
        ms * u64::from(RATE) / 1000
    }

    fn point(ms: u64, hz: u32) -> RepairMaskPoint {
        RepairMaskPoint {
            frame: frame(ms),
            hz,
        }
    }

    fn recipe(
        regions: Vec<RepairMaskRegion>,
        time: FrameRange,
        band: Range<u32>,
        feather: u32,
    ) -> AudioTreatment {
        let mut repairs = [const { None }; MAX_SPECTRAL_REPAIRS];
        repairs[0] = Some(SpectralRepair {
            start_frame: time.start,
            end_frame: time.end,
            low_hz: band.start,
            high_hz: band.end,
            amount_db: 23,
            channel: Some(RepairChannel::Left),
            mask: Some(Arc::new(RepairMask {
                version: 1,
                feather_frames: feather,
                feather_hz: 0,
                regions,
            })),
            ..SpectralRepair::default()
        });
        AudioTreatment {
            spectral_repairs: Some(repairs),
            ..AudioTreatment::default()
        }
    }

    #[test]
    fn click_interpolation_crosses_block_boundaries_and_seeks_without_touching_other_pcm() {
        let start = 19_455;
        let end = start + 3;
        let mut settings = recipe(Vec::new(), FrameRange { start, end }, 20..20_000, 0);
        let edit = settings.spectral_repairs.as_mut().unwrap()[0]
            .as_mut()
            .unwrap();
        edit.mask = None;
        edit.interpolate = true;

        let read = |at| {
            let mut file = signal_file();
            file.seek(std::io::SeekFrom::Start(44 + start * 8)).unwrap();
            file.write_all(&i32::MAX.to_le_bytes()).unwrap();
            file.rewind().unwrap();
            let mut source = RepairSource::open(file, at, None, &settings, || false).unwrap();
            let mut dry = Vec::new();
            let mut repaired = Vec::new();
            while let Some(block) = source.next_block(|| false).unwrap() {
                dry.extend_from_slice(block.dry);
                repaired.extend_from_slice(block.samples());
            }
            (dry, repaired)
        };
        let (dry, repaired) = read(0);
        for (index, (&original, &sample)) in dry.iter().zip(&repaired).enumerate() {
            if index % 2 != 0 || !(start..end).contains(&((index / 2) as u64)) {
                assert_eq!(sample, original);
            }
        }
        assert!(repaired[start as usize * 2] < 800_000_000);
        let seek = start + 1;
        let (_, sought) = read(seek);
        assert_eq!(&repaired[seek as usize * 2..], sought);

        let mut invalid = settings.clone();
        invalid.spectral_repairs.as_mut().unwrap()[0]
            .as_mut()
            .unwrap()
            .end_frame = start + 97;
        assert!(invalid.output_info(SOURCE).is_err());
    }

    fn read_pcm(settings: &AudioTreatment, start: u64) -> (Vec<i32>, Vec<i32>) {
        let mut source =
            RepairSource::open(signal_file(), start, None, settings, || false).unwrap();
        let mut dry = Vec::new();
        let mut repaired = Vec::new();
        while let Some(block) = source.next_block(|| false).unwrap() {
            dry.extend(block.dry);
            repaired.extend(block.samples());
        }
        assert_eq!(source.position(), FRAMES);
        (dry, repaired)
    }

    fn magnitude(samples: &[i32], hz: f64, start_ms: u64, end_ms: u64) -> f64 {
        let (re, im) = (frame(start_ms)..frame(end_ms)).fold((0.0, 0.0), |(re, im), frame| {
            let phase = TAU * hz * frame as f64 / f64::from(RATE);
            let value = f64::from(samples[frame as usize * 2]);
            (re + value * phase.cos(), im + value * phase.sin())
        });
        re.hypot(im)
    }

    fn change_db(dry: &[i32], wet: &[i32], hz: f64, start: u64, end: u64) -> f64 {
        20.0 * (magnitude(wet, hz, start, end) / magnitude(dry, hz, start, end)).log10()
    }

    fn rectangle(
        operation: MaskOperation,
        start: u64,
        end: u64,
        low_hz: u32,
        high_hz: u32,
    ) -> RepairMaskRegion {
        RepairMaskRegion::Rectangle {
            operation,
            start_frame: frame(start),
            end_frame: frame(end),
            low_hz,
            high_hz,
        }
    }

    #[test]
    fn brush_tracks_a_curved_band_and_preserves_other_frequency_time_and_channel() {
        let start = frame(130);
        let end = frame(870);
        let settings = recipe(
            vec![RepairMaskRegion::Brush {
                operation: MaskOperation::Add,
                radius_frames: frame(20) as u32,
                radius_hz: 170,
                points: vec![
                    point(150, 1000),
                    point(410, 1000),
                    point(530, 3000),
                    point(850, 3000),
                ],
            }],
            FrameRange { start, end },
            830..3170,
            0,
        );
        let (dry, wet) = read_pcm(&settings, 0);
        let db = |hz, from, to| change_db(&dry, &wet, hz, from, to);
        assert!(db(1000.0, 250, 350) < -20.0);
        assert!(db(3000.0, 250, 350).abs() < 0.1);
        assert!(db(3000.0, 640, 740) < -20.0);
        assert!(db(1000.0, 640, 740).abs() < 0.1);
        assert!(dry
            .iter()
            .skip(1)
            .step_by(2)
            .eq(wet.iter().skip(1).step_by(2)));
        assert_eq!(wet[..start as usize * 2], dry[..start as usize * 2]);
        assert_eq!(wet[end as usize * 2..], dry[end as usize * 2..]);
    }

    #[test]
    fn lasso_preserves_subtracted_content_and_unpainted_time() {
        let settings = recipe(
            vec![
                RepairMaskRegion::Lasso {
                    operation: MaskOperation::Add,
                    points: vec![point(100, 700), point(100, 3500), point(640, 800)],
                },
                rectangle(MaskOperation::Subtract, 0, 350, 550, 1350),
                rectangle(MaskOperation::Add, 760, 870, 2750, 3250),
            ],
            FrameRange {
                start: frame(100),
                end: frame(870),
            },
            700..3500,
            0,
        );
        let (dry, wet) = read_pcm(&settings, 0);
        let db = |hz, from, to| change_db(&dry, &wet, hz, from, to);
        assert!(db(1000.0, 150, 220).abs() < 0.1);
        assert!(db(3000.0, 145, 155) < -12.0);
        assert!(db(1000.0, 405, 495) < -20.0);
        assert!(db(3000.0, 405, 495).abs() < 0.1);
        assert_eq!(
            wet[frame(640) as usize * 2..frame(760) as usize * 2],
            dry[frame(640) as usize * 2..frame(760) as usize * 2]
        );
        assert!(db(3000.0, 795, 835) < -20.0);
    }

    #[test]
    fn spectral_edits_do_not_leak_into_an_abutting_repair() {
        let first = recipe(
            vec![rectangle(MaskOperation::Add, 170, 440, 800, 1200)],
            FrameRange {
                start: frame(170),
                end: frame(440),
            },
            800..1200,
            0,
        );
        let second = recipe(
            vec![rectangle(MaskOperation::Add, 440, 510, 2750, 3250)],
            FrameRange {
                start: frame(440),
                end: frame(510),
            },
            2750..3250,
            0,
        );
        let mut combined = first.clone();
        combined.spectral_repairs.as_mut().unwrap()[1] =
            second.spectral_repairs.as_ref().unwrap()[0].clone();
        let (_, first_pcm) = read_pcm(&first, 0);
        let (_, second_pcm) = read_pcm(&second, 0);
        let (_, both_pcm) = read_pcm(&combined, 0);
        let boundary = frame(440) as usize * 2;
        assert_eq!(both_pcm[..boundary], first_pcm[..boundary]);
        assert_eq!(both_pcm[boundary..], second_pcm[boundary..]);
    }

    #[test]
    fn broadband_mask_works_at_sample_resolution_and_has_a_feather() {
        let start = frame(320) + 7;
        let end = start + 1;
        let settings = recipe(
            vec![RepairMaskRegion::Rectangle {
                operation: MaskOperation::Add,
                start_frame: start,
                end_frame: end,
                low_hz: 0,
                high_hz: RATE / 2,
            }],
            FrameRange { start, end },
            0..RATE / 2,
            2,
        );
        let (dry, wet) = read_pcm(&settings, 0);
        let offset = start as usize * 2;
        let gain = |index| f64::from(wet[index]) / f64::from(dry[index]);
        assert!((gain(offset) - 10_f64.powf(-23.0 / 20.0)).abs() < 0.00001);
        assert!(gain(offset - 2) > gain(offset));
        assert!(gain(offset - 2) < 1.0);
        assert_eq!(wet[..offset - 4], dry[..offset - 4]);
        assert_eq!(wet[offset + 6..], dry[offset + 6..]);
    }

    #[test]
    fn seek_and_export_match_at_non_hop_aligned_frames() {
        let settings = recipe(
            vec![rectangle(MaskOperation::Add, 170, 820, 800, 1200)],
            FrameRange {
                start: frame(170),
                end: frame(820),
            },
            800..1200,
            frame(12) as u32,
        );
        let start = frame(325) + 17;
        let end = frame(723) + 13;
        let (_, whole) = read_pcm(&settings, 0);
        let (dry, wet) = read_pcm(&settings, start);
        assert_eq!(&whole[start as usize * 2..], wet);
        assert!(dry.iter().zip(&wet).any(|(a, b)| a != b));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("selected-repair.flac");
        let prepared = PreparedTreatment::preview(settings, SOURCE).unwrap();
        render_treated_file(
            signal_file(),
            &[RenderSegment {
                range: FrameRange { start, end },
                output: path.clone(),
            }],
            OutputFormat::Flac,
            prepared,
            || false,
            |_| {},
            |path| OpenOptions::new().write(true).create_new(true).open(path),
        )
        .unwrap();
        let mut export = PreviewDecoder::open(File::open(path).unwrap(), 0, || false).unwrap();
        let mut encoded = Vec::new();
        while let Some(block) = export.next_block(|| false).unwrap() {
            encoded.extend_from_slice(block);
        }
        assert_eq!(encoded.len(), (end - start) as usize * 2);
        assert!(encoded
            .iter()
            .zip(&wet)
            .all(|(encoded, wet)| encoded.abs_diff(*wet) <= 255));
    }

    #[test]
    fn bypass_preserves_entire_source_without_a_mask_reader() {
        let mut settings = recipe(
            vec![rectangle(MaskOperation::Add, 170, 820, 800, 1200)],
            FrameRange {
                start: frame(170),
                end: frame(820),
            },
            800..1200,
            0,
        );
        let repair = settings.spectral_repairs.as_mut().unwrap()[0]
            .as_mut()
            .unwrap();
        repair.bypassed = true;
        repair.name = Some("Quiet whistle".into());
        assert!(!settings.is_active());
        let mut source = RepairSource::open(signal_file(), 0, None, &settings, || false).unwrap();
        assert!(source
            .next_block(|| false)
            .unwrap()
            .unwrap()
            .repaired
            .is_none());
        assert_eq!(settings.output_info(SOURCE).unwrap(), SOURCE);
    }
}
