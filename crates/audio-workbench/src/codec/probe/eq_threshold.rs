use super::{check_cancelled, MediaError, PreparedTreatment, SeekIndex, SourceInfo};
use crate::{FrameRange, RepairSource};
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::time::{Duration, Instant};

/// Suggest a threshold using the same sidechain and envelope as the selected band.
pub fn learn_eq_threshold(
    mut source: File,
    info: SourceInfo,
    range: FrameRange,
    index: Option<&SeekIndex>,
    prepared: PreparedTreatment,
    band_index: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<f32, MediaError> {
    let band = prepared
        .settings()
        .bands()
        .nth(band_index)
        .filter(|band| !band.bypassed)
        .and_then(|band| band.dynamic)
        .ok_or(MediaError::Range("Select an active dynamic EQ band"))?;
    if range.start >= range.end
        || range.end > info.frames
        || info.sample_rate == 0
        || info.channels == 0
    {
        return Err(MediaError::Range("Choose a loop inside the recording"));
    }
    let rate = u64::from(info.sample_rate);
    let channels = usize::from(info.channels);
    let frames = range.end - range.start;
    if frames < rate / 10 {
        return Err(MediaError::Range(
            "Use a loop at least a tenth of a second long",
        ));
    }
    // Sample long loops across their whole duration without decoding the entire recording.
    let windows = if frames > rate * 8 { 4 } else { 1 };
    let width = frames.min(rate * 8) / windows;
    let interval = (rate / 50).max(1);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stop = || cancelled() || Instant::now() >= deadline;
    let mut readings = Vec::new();
    let mut processed = Vec::new();

    for window in 0..windows {
        check_cancelled(&mut stop)?;
        let start = range.start + (frames - width) * window / (windows - 1).max(1);
        let end = start + width;
        let warmup = prepared.preview_start(start, info.sample_rate);
        source.seek(SeekFrom::Start(0))?;
        let mut decoder = RepairSource::open(
            source.try_clone()?,
            warmup,
            index,
            prepared.settings(),
            &mut stop,
        )?;
        if decoder.source_info().channels != info.channels
            || decoder.source_info().sample_rate != info.sample_rate
        {
            return Err(MediaError::Range("Audio parameters changed"));
        }
        let mut processor = prepared.processor_at(info, warmup)?;
        let mut cursor = warmup;
        let mut measure_at = (start + interval).min(end);
        while cursor < end {
            check_cancelled(&mut stop)?;
            let Some(samples) = decoder.next_block(&mut stop)? else {
                break;
            };
            let samples = samples.samples();
            let block_end = (cursor + (samples.len() / channels) as u64).min(end);
            let mut next = cursor;
            while next < block_end {
                let last = block_end.min(measure_at);
                processor.process(
                    &samples[((next - cursor) as usize * channels)
                        ..((last - cursor) as usize * channels)],
                    &mut processed,
                );
                next = last;
                if next == measure_at {
                    if let Some(db) = processor
                        .dynamic_eq_detector(band_index)
                        .filter(|db| *db > -90.0)
                    {
                        readings.push(db);
                    }
                    measure_at = (measure_at + interval).min(end);
                }
            }
            cursor = block_end;
        }
        if cursor < end {
            return Err(MediaError::Range("The recording ended inside the loop"));
        }
    }

    if readings.is_empty() {
        return Err(MediaError::Range(
            "There is no measurable signal in this band",
        ));
    }
    readings.sort_unstable_by(f32::total_cmp);
    // Cut the loudest part or lift the quietest part of the measured loop.
    let quartile = if band.amount_db > 0.0 { 3 } else { 1 };
    let measured = readings[(readings.len() - 1) * quartile / 4];
    Ok((measured * 10.0).round().clamp(-720.0, -60.0) / 10.0)
}
