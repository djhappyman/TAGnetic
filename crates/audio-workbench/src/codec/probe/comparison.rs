use super::{
    centered, check_cancelled, interleaved, level_db, overlap, probe_fft_size, MediaError,
    PreparedTreatment, PreviewDecoder, SeekIndex, SourceInfo,
};
use crate::{FrameRange, RepairSource, TreatmentProcessor};
use serde::Serialize;
use std::{
    fs::File,
    time::{Duration, Instant},
};

// This job measures two chains through their warmup and remains cancellable.
const COMPARISON_BUDGET: Duration = Duration::from_secs(3);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewComparison {
    pub input: f32,
    pub output: f32,
    pub alternate_output: f32,
}

struct Variant {
    start: u64,
    processor: TreatmentProcessor,
    pcm: Vec<i32>,
}

impl Variant {
    fn new(
        settings: &PreparedTreatment,
        info: SourceInfo,
        frame: u64,
        span: FrameRange,
    ) -> Result<Self, MediaError> {
        let recipe = settings.settings();
        let start = if recipe.compressor_db != 0 || recipe.gate_db != 0 {
            span.start
                .min(frame.saturating_sub(u64::from(info.sample_rate) * 3 / 4))
        } else {
            span.start
        };
        let start = settings
            .preview_start(start, info.sample_rate)
            .max(start.saturating_sub(u64::from(info.sample_rate) * 5));
        Ok(Self {
            start,
            processor: settings.processor_at(info, start)?,
            pcm: Vec::new(),
        })
    }

    fn measure_source_repair(
        &mut self,
        mut decoder: RepairSource,
        info: SourceInfo,
        span: FrameRange,
        mut dry: Option<&mut Vec<i32>>,
        stop: &mut impl FnMut() -> bool,
    ) -> Result<(), MediaError> {
        let mut cursor = self.start;
        if decoder.source_info().channels != info.channels
            || decoder.source_info().sample_rate != info.sample_rate
        {
            return Err(MediaError::Range("Audio parameters changed"));
        }
        let channels = usize::from(info.channels);
        let output_channels = usize::from(self.processor.output_info().channels);
        let mut processed = Vec::new();
        while cursor < span.end {
            let Some(block) = decoder.next_block(&mut *stop)? else {
                break;
            };
            let end = (cursor + (block.dry.len() / channels) as u64).min(span.end);
            let length = (end - cursor) as usize * channels;
            self.processor
                .process(&block.samples()[..length], &mut processed);
            if let Some(measurement) = overlap(cursor, end, span) {
                if let Some(dry) = &mut dry {
                    dry.extend_from_slice(&block.dry[interleaved(measurement, cursor, channels)]);
                }
                self.pcm.extend_from_slice(
                    &processed[interleaved(measurement, cursor, output_channels)],
                );
            }
            cursor = end;
        }
        Ok(())
    }
}

/// Measure two treatments over the same dry excerpt. Ordinary treatment
/// shares one decode. Each source repair needs its own bounded PCM context.
pub fn compare_preview_levels(
    source: File,
    info: SourceInfo,
    frame: u64,
    index: Option<&SeekIndex>,
    settings: [PreparedTreatment; 2],
    mut cancelled: impl FnMut() -> bool,
) -> Result<PreviewComparison, MediaError> {
    if frame >= info.frames || info.sample_rate == 0 || info.channels == 0 {
        return Err(MediaError::Range(
            "Audio comparison starts outside the recording",
        ));
    }
    let span = centered(
        frame,
        u64::from(probe_fft_size(info.sample_rate)),
        info.frames,
    );
    let mut variants = [
        Variant::new(&settings[0], info, frame, span)?,
        Variant::new(&settings[1], info, frame, span)?,
    ];
    let mut cursor = variants[0].start.min(variants[1].start);
    let channels = usize::from(info.channels);
    let deadline = Instant::now() + COMPARISON_BUDGET;
    let mut stop = || cancelled() || Instant::now() >= deadline;
    let mut dry = Vec::with_capacity((span.end - span.start) as usize * channels);
    if settings.iter().any(|treatment| {
        let recipe = treatment.settings();
        recipe.mask_repairs().next().is_some() || recipe.click_repairs().next().is_some()
    }) {
        for (number, (variant, treatment)) in variants.iter_mut().zip(&settings).enumerate() {
            let decoder = RepairSource::open(
                source.try_clone()?,
                variant.start,
                index,
                treatment.settings(),
                &mut stop,
            )?;
            variant.measure_source_repair(
                decoder,
                info,
                span,
                (number == 0).then_some(&mut dry),
                &mut stop,
            )?;
        }
        return levels(&dry, &variants);
    }

    let mut decoder = PreviewDecoder::open_indexed(source, cursor, index, &mut stop)?;
    if decoder.source_info().channels != info.channels
        || decoder.source_info().sample_rate != info.sample_rate
    {
        return Err(MediaError::Range("Audio parameters changed"));
    }
    let mut processed = Vec::new();
    while cursor < span.end {
        check_cancelled(&mut stop)?;
        let Some(block) = decoder.next_block(&mut stop)? else {
            break;
        };
        let end = (cursor + (block.len() / channels) as u64).min(span.end);
        let measurement = overlap(cursor, end, span);
        if let Some(measurement) = measurement {
            dry.extend_from_slice(&block[interleaved(measurement, cursor, channels)]);
        }
        for variant in &mut variants {
            let start = cursor.max(variant.start);
            if start >= end {
                continue;
            }
            variant.processor.process(
                &block[interleaved(FrameRange { start, end }, cursor, channels)],
                &mut processed,
            );
            if let Some(measurement) = measurement {
                let output_channels = usize::from(variant.processor.output_info().channels);
                variant.pcm.extend_from_slice(
                    &processed[interleaved(measurement, start, output_channels)],
                );
            }
        }
        cursor = end;
    }
    levels(&dry, &variants)
}

fn levels(dry: &[i32], variants: &[Variant; 2]) -> Result<PreviewComparison, MediaError> {
    if dry.is_empty() {
        return Err(MediaError::Range(
            "No decoded samples at the comparison position",
        ));
    }
    Ok(PreviewComparison {
        input: level_db(dry).0,
        output: level_db(&variants[0].pcm).0,
        alternate_output: level_db(&variants[1].pcm).0,
    })
}
