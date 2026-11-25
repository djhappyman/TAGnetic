use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tagrex_audio_workbench::{
    check_output, inspect, inspect_file, inspect_file_with_progress, probe_file, render,
    render_file, waveform, waveform_file_indexed, FrameCount, FrameRange, MediaError, OutputFormat,
    Peak, PreviewDecoder, RenderSegment, SeekIndex, SourceInfo,
};

fn ffmpeg(args: &[&str], source: &Path, output: &Path) {
    let mut command = Command::new("ffmpeg");
    command.args(["-v", "error", "-nostdin", "-i"]).arg(source);
    command.args(args).arg(output);
    assert_success(command.output().unwrap());
}

fn read_pcm(path: &Path, format: &str) -> Vec<u8> {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", format, "pipe:1"])
        .output()
        .unwrap();
    assert_success(output)
}

fn assert_success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn references_available() -> bool {
    let available = ["ffmpeg", "flac"]
        .iter()
        .all(|tool| Command::new(tool).arg("-version").output().is_ok());
    if !available {
        assert!(
            std::env::var_os("TAGREX_REQUIRE_REFERENCE_CODECS").is_none(),
            "Install FFmpeg and reference FLAC for independent readback"
        );
        eprintln!("Independent FFmpeg and FLAC readback skipped");
    }
    available
}

fn signal(frame: u64, channel: u16) -> f64 {
    let x = frame as f64;
    let tone = if channel == 0 { 0.089 } else { 0.054 };
    let envelope = 0.3 + ((x * 0.0008).sin() + 1.0) * 0.24;
    (x * tone).sin() * envelope + (x * 0.179).sin() * 0.07
}

fn pcm_wav(path: &Path, frames: u64, channels: u16, bits: u16, rate: u32) -> Vec<u8> {
    let sample_bytes = bits / 8;
    let block = channels * sample_bytes;
    let data_len = u32::try_from(frames * u64::from(block)).unwrap();
    let mut file = File::create(path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + data_len + data_len % 2).to_le_bytes())
        .unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&rate.to_le_bytes()).unwrap();
    file.write_all(&(rate * u32::from(block)).to_le_bytes())
        .unwrap();
    file.write_all(&block.to_le_bytes()).unwrap();
    file.write_all(&bits.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_len.to_le_bytes()).unwrap();
    let mut pcm = Vec::with_capacity(data_len as usize);
    for frame in 0..frames {
        for channel in 0..channels {
            let sample = (signal(frame, channel) * f64::from(1_i32 << (bits - 1))) as i32;
            pcm.extend_from_slice(&sample.to_le_bytes()[..usize::from(sample_bytes)]);
        }
    }
    file.write_all(&pcm).unwrap();
    if data_len % 2 != 0 {
        file.write_all(&[0]).unwrap();
    }
    pcm
}

fn output_ranges(
    directory: &Path,
    prefix: &str,
    extension: &str,
    cuts: &[u64],
) -> Vec<RenderSegment> {
    cuts.windows(2)
        .enumerate()
        .map(|(i, pair)| RenderSegment {
            range: FrameRange {
                start: pair[0],
                end: pair[1],
            },
            output: directory.join(format!("{prefix}-{i}.{extension}")),
        })
        .collect()
}

fn reference_flac(path: &Path) -> Vec<u8> {
    assert_success(
        Command::new("flac")
            .args(["--test", "--silent"])
            .arg(path)
            .output()
            .unwrap(),
    );
    assert_success(
        Command::new("flac")
            .args([
                "--decode",
                "--silent",
                "--stdout",
                "--force-raw-format",
                "--endian=little",
                "--sign=signed",
            ])
            .arg(path)
            .output()
            .unwrap(),
    )
}

#[test]
fn wav_aiff_and_flac_preserve_pcm_at_nonround_boundaries() {
    if !references_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let frames = 176_419;
    let cuts = [0, 1, 37, 58_801, 120_007, frames - 11, frames];
    for (bits, channels, pcm_big) in [(16, 2, "pcm_s16be"), (24, 1, "pcm_s24be")] {
        let wav = dir.path().join(format!("input-{bits}.wav"));
        let aiff = dir.path().join(format!("input-{bits}.aiff"));
        let flac = dir.path().join(format!("input-{bits}.flac"));
        let pcm = pcm_wav(&wav, frames, channels, bits, 44_100);
        ffmpeg(&["-c:a", pcm_big], &wav, &aiff);
        ffmpeg(&["-c:a", "flac"], &wav, &flac);
        let baseline = waveform(
            &wav,
            FrameRange {
                start: 82_237,
                end: 85_013,
            },
            513,
            || false,
        )
        .unwrap();
        for input in [wav, aiff, flac] {
            let analysis = inspect(&input, 800, || false)
                .unwrap_or_else(|error| panic!("{}: {error}", input.display()));
            assert_eq!(
                analysis.source,
                SourceInfo {
                    sample_rate: 44_100,
                    channels,
                    frames,
                    bits_per_sample: Some(bits as u8),
                },
                "{}",
                input.display()
            );
            assert_eq!(analysis.peaks.len(), 800);
            let range = FrameRange {
                start: 82_237,
                end: 85_013,
            };
            assert_eq!(
                waveform(&input, range, 513, || false).unwrap(),
                baseline,
                "{}",
                input.display()
            );
            let approximate = analysis.waveform.peaks(range, 513).unwrap();
            for (coarse, exact) in approximate.iter().zip(&baseline) {
                assert!(coarse.min <= exact.min && coarse.max >= exact.max);
            }

            let stem = input.extension().unwrap().to_str().unwrap();
            let segments = output_ranges(dir.path(), &format!("{bits}-{stem}"), "flac", &cuts);
            let mut latest = None;
            // Exercise the held handle even if someone already read from it.
            let mut source = File::open(&input).unwrap();
            source.seek(SeekFrom::Start(101)).unwrap();
            render_file(
                source,
                &segments,
                OutputFormat::Flac,
                || false,
                |progress| latest = Some(progress),
            )
            .unwrap();
            let progress = latest.unwrap();
            assert_eq!(progress.completed_frames, frames);
            assert_eq!(progress.completed_segments, segments.len());

            let block = usize::from(channels * bits / 8);
            let mut joined = Vec::with_capacity(pcm.len());
            for segment in segments {
                let decoded = reference_flac(&segment.output);
                let start = segment.range.start as usize * block;
                let end = segment.range.end as usize * block;
                assert_eq!(decoded, pcm[start..end], "{}", segment.output.display());
                joined.extend(decoded);
            }
            assert_eq!(joined, pcm);
        }
    }
}

#[test]
fn block_aligned_aiff_keeps_exact_tail_and_seeks_through_the_offset() {
    if !references_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let frames = 16_419;
    for (bits, channels, offset) in [(16, 2, 256), (24, 1, 257)] {
        let wav = dir.path().join(format!("source-{bits}.wav"));
        let aiff = dir.path().join(format!("padded-{bits}.aiff"));
        let pcm = pcm_wav(&wav, frames, channels, bits, 44_100);
        let sample_bytes = usize::from(bits / 8);
        let mut common = Vec::new();
        common.extend_from_slice(&channels.to_be_bytes());
        common.extend_from_slice(&(frames as u32).to_be_bytes());
        common.extend_from_slice(&bits.to_be_bytes());
        common.extend_from_slice(&[0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]); // 44,100 Hz
        let mut sound = (offset as u32).to_be_bytes().to_vec();
        sound.extend_from_slice(&512_u32.to_be_bytes());
        sound.extend(std::iter::repeat_n(0xa5, offset));
        for sample in pcm.chunks_exact(sample_bytes) {
            sound.extend(sample.iter().rev());
        }
        let mut body = b"AIFF".to_vec();
        for (id, data) in [
            (b"COMM", common.as_slice()),
            (b"SSND", sound.as_slice()),
            (b"NAME", b"Tail sentinel".as_slice()),
        ] {
            body.extend_from_slice(id);
            body.extend_from_slice(&(data.len() as u32).to_be_bytes());
            body.extend_from_slice(data);
            if data.len() % 2 != 0 {
                body.push(0);
            }
        }
        let mut original = b"FORM".to_vec();
        original.extend_from_slice(&(body.len() as u32).to_be_bytes());
        original.extend(body);
        fs::write(&aiff, &original).unwrap();

        let reference = read_pcm(&aiff, "f32le");
        assert_eq!(reference, read_pcm(&wav, "f32le"));
        let analysis = inspect(&aiff, 512, || false).unwrap();
        assert_eq!(analysis.source.frames, frames);
        let last_second = FrameRange {
            start: frames - 199,
            end: frames,
        };
        assert_eq!(
            waveform(&aiff, last_second, 127, || false).unwrap(),
            ffmpeg_peaks(&reference, last_second, 127, usize::from(channels))
        );
        let segments = output_ranges(
            dir.path(),
            &format!("padded-{bits}"),
            "flac",
            &[0, 21, frames - 1, frames],
        );
        render(&aiff, &segments, OutputFormat::Flac, || false, |_| {}).unwrap();
        let joined = segments
            .iter()
            .flat_map(|segment| reference_flac(&segment.output))
            .collect::<Vec<_>>();
        assert_eq!(joined, pcm);
        assert_eq!(fs::read(&aiff).unwrap(), original);
    }
}

#[test]
fn cbr_and_vbr_mp3_input_has_gapless_frame_timing() {
    if !references_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("source.wav");
    let frames = 240_111;
    pcm_wav(&wav, frames, 2, 16, 44_100);
    for (name, setting) in [("cbr", ["-b:a", "192k"]), ("vbr", ["-q:a", "2"])] {
        let input = dir.path().join(format!("input-{name}.mp3"));
        ffmpeg(
            &["-c:a", "libmp3lame", setting[0], setting[1]],
            &wav,
            &input,
        );
        let from_ffmpeg = read_pcm(&input, "f32le");
        let analysis = inspect(&input, 1024, || false).unwrap();
        assert_eq!(analysis.source.frames, frames);
        assert_eq!(analysis.source.frames as usize, from_ffmpeg.len() / 8);
        assert_eq!(analysis.source.bits_per_sample, None);
        let range = FrameRange {
            start: frames - 6319,
            end: frames - 91,
        };
        let exact = waveform(&input, range, 119, || false).unwrap();
        let actual = ffmpeg_peaks(&from_ffmpeg, range, 119, 2);
        for (native, reference) in exact.iter().zip(&actual) {
            // Independent decoders vary by a tiny amount due to rounding.
            assert!(
                (native.min - reference.min).abs() < 0.0001,
                "{native:?} vs {reference:?}"
            );
            assert!(
                (native.max - reference.max).abs() < 0.0001,
                "{native:?} vs {reference:?}"
            );
        }
    }
}

fn preview_samples(
    path: &Path,
    start: u64,
    count: usize,
    index: Option<&SeekIndex>,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Vec<i32>, MediaError> {
    let mut preview =
        PreviewDecoder::open_indexed(File::open(path)?, start, index, &mut cancelled)?;
    let mut output = Vec::with_capacity(count);
    while output.len() < count {
        let Some(block) = preview.next_block(&mut cancelled)? else {
            break;
        };
        output.extend_from_slice(&block[..block.len().min(count - output.len())]);
    }
    Ok(output)
}

#[test]
fn sparse_mp3_seeks_match_sequential_pcm_and_skip_the_compressed_prefix() {
    if !references_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("sparse-seek.wav");
    pcm_wav(&wav, 7 * 44_100 + 123, 2, 16, 44_100);
    let formats: &[(&str, &[&str])] = &[
        ("cbr-gapless", &["-b:a", "160k"]),
        ("vbr-gapless", &["-q:a", "3"]),
        (
            "cbr-headerless",
            &["-b:a", "160k", "-write_xing", "0", "-id3v2_version", "0"],
        ),
        ("vbr-headerless", &["-q:a", "3", "-write_xing", "0"]),
        ("mpeg2-gapless", &["-ar", "22050", "-ac", "1", "-q:a", "3"]),
    ];
    let mut sources = Vec::new();
    for (label, settings) in formats {
        let path = dir.path().join(format!("{label}.mp3"));
        let mut args = vec!["-c:a", "libmp3lame"];
        args.extend_from_slice(settings);
        ffmpeg(&args, &wav, &path);
        sources.push((*label, path));
    }
    let protected = dir.path().join("protected-gapless.mp3");
    match Command::new("lame")
        .args(["--silent", "-p", "-b", "160"])
        .arg(&wav)
        .arg(&protected)
        .output()
    {
        Ok(output) => {
            assert_success(output);
            sources.push(("protected-gapless", protected));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("could not run LAME: {error}"),
    }
    for (label, path) in sources {
        let analysis = inspect(&path, 1024, || false).unwrap();
        let index = analysis.seek_index.as_deref().expect(label);
        let channels = usize::from(analysis.source.channels);
        let total = analysis.source.frames;
        let pcm = preview_samples(&path, 0, total as usize * channels + 1, None, || false).unwrap();
        assert_eq!(pcm.len(), total as usize * channels, "{label}");
        for (start, end) in [
            (66_739, 67_052),
            (total - 7_001, total - 181),
            (total - 17, total),
        ] {
            let count = (end - start) as usize * channels;
            let actual = preview_samples(&path, start, count, Some(index), || false).unwrap();
            assert!(
                actual == pcm[start as usize * channels..end as usize * channels],
                "{label}: different PCM at frame {start}"
            );
            let range = FrameRange { start, end };
            let peaks =
                waveform_file_indexed(File::open(&path).unwrap(), range, 31, Some(index), || false)
                    .unwrap();
            assert_eq!(
                peaks,
                waveform(&path, range, 31, || false).unwrap(),
                "{label}"
            );
        }
        let start = total - 7_001;
        let mut checks = 0;
        let indexed = preview_samples(&path, start, channels * 100, Some(index), || {
            checks += 1;
            checks > 180
        });
        assert!(indexed.is_ok(), "{label}: index scanned the old prefix");
        checks = 0;
        assert!(
            matches!(
                preview_samples(&path, start, channels * 100, None, || {
                    checks += 1;
                    checks > 180
                }),
                Err(MediaError::Cancelled)
            ),
            "{label}: fixture no longer exceeds the unindexed packet budget"
        );
    }

    // The ordinary demuxer can resynchronize after gaps. Reject sparse
    // offsets unless the original packet stream was physically contiguous.
    let mut gapped = fs::read(dir.path().join("cbr-headerless.mp3")).unwrap();
    let mut gap = 0;
    for _ in 0..80 {
        let header =
            rusty_mp3::header::FrameHeader::parse(gapped[gap..gap + 4].try_into().unwrap())
                .unwrap();
        gap += header.frame_size();
    }
    gapped.splice(gap..gap, b"interrupted stream gap".iter().copied());
    let path = dir.path().join("gapped.mp3");
    fs::write(&path, gapped).unwrap();
    let clean = inspect(&dir.path().join("cbr-headerless.mp3"), 1, || false).unwrap();
    let recovered = inspect(&path, 1024, || false).unwrap();
    assert_eq!(recovered.source.frames, clean.source.frames);
    assert!(recovered.seek_index.is_none());
}

#[test]
fn a_vbr_mp3_without_a_xing_header_keeps_all_frames() {
    if !references_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("source.wav");
    let input = dir.path().join("legacy-vbr.mp3");
    pcm_wav(&wav, 305_111, 2, 16, 44_100);
    ffmpeg(
        &["-c:a", "libmp3lame", "-q:a", "3", "-write_xing", "0"],
        &wav,
        &input,
    );
    let probe = probe_file(File::open(&input).unwrap()).unwrap();
    assert!(matches!(
        probe.frame_count,
        FrameCount::Estimated | FrameCount::Unknown
    ));
    let reference = read_pcm(&input, "f32le");
    let analysis = inspect(&input, 512, || false).unwrap();
    assert_eq!(analysis.source.frames, reference.len() as u64 / 8);

    let range = FrameRange {
        start: analysis.source.frames - 8199,
        end: analysis.source.frames - 37,
    };
    let peaks = waveform(&input, range, 129, || false).unwrap();
    for (native, reference) in peaks.iter().zip(ffmpeg_peaks(&reference, range, 129, 2)) {
        assert!(
            (native.min - reference.min).abs() < 0.0001,
            "{native:?} vs {reference:?}"
        );
        assert!(
            (native.max - reference.max).abs() < 0.0001,
            "{native:?} vs {reference:?}"
        );
    }
}

#[test]
fn analysis_provides_an_immediate_header_and_verifies_the_frame_count() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("header.wav");
    pcm_wav(&path, 4097, 2, 16, 44_100);
    let probed = probe_file(File::open(&path).unwrap()).unwrap();
    assert_eq!(probed.frame_count, FrameCount::Declared);
    assert_eq!(probed.source.frames, 4097);
    let mut updates = Vec::new();
    let result = inspect_file_with_progress(
        File::open(&path).unwrap(),
        512,
        || false,
        |update| updates.push(update),
    )
    .unwrap();
    assert_eq!(result.source, probed.source);
    assert_eq!(updates[0].source, probed);
    assert_eq!(updates[0].scanned_frames, 0);
    assert!(updates[0].peaks.is_empty());
    assert_eq!(result.waveform.frames_per_bucket(), 256);
}

fn ffmpeg_peaks(pcm: &[u8], range: FrameRange, bins: usize, channels: usize) -> Vec<Peak> {
    let block = channels * 4;
    let span = range.end - range.start;
    (0..bins)
        .map(|index| {
            let start = (range.start + span * index as u64 / bins as u64) as usize * block;
            let end = (range.start + span * (index + 1) as u64 / bins as u64) as usize * block;
            pcm[start..end].as_chunks::<4>().0.iter().fold(
                Peak {
                    min: 1.0,
                    max: -1.0,
                },
                |peak, bytes| {
                    let sample = f32::from_le_bytes(*bytes);
                    Peak {
                        min: peak.min.min(sample),
                        max: peak.max.max(sample),
                    }
                },
            )
        })
        .collect()
}

#[test]
fn mp3_exports_decode_to_the_exact_requested_length() {
    if !references_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for (rate, channels) in [(44_100, 2), (22_050, 1), (8_000, 1)] {
        let wav = dir.path().join(format!("source-{rate}.wav"));
        let frames = u64::from(rate) * 2 + 23;
        pcm_wav(&wav, frames, channels, 16, rate);
        let cuts = [0, 1, 37, 697, 4097, frames - 19, frames];
        let segments = output_ranges(dir.path(), &format!("at-{rate}"), "mp3", &cuts);
        render(
            &wav,
            &segments,
            OutputFormat::Mp3 { bitrate_kbps: 128 },
            || false,
            |_| {},
        )
        .unwrap();
        for segment in &segments {
            let expected = segment.range.end - segment.range.start;
            let ffmpeg_pcm = read_pcm(&segment.output, "f32le");
            assert_eq!(
                ffmpeg_pcm.len() as u64 / (4 * u64::from(channels)),
                expected,
                "{}",
                segment.output.display()
            );
            let decoded = inspect(&segment.output, 128, || false).unwrap().source;
            assert_eq!(
                (decoded.frames, decoded.sample_rate, decoded.channels),
                (expected, rate, channels),
                "{}",
                segment.output.display()
            );
        }
        assert_audio_matches_source(&segments[4], rate, channels);
    }
}

fn assert_audio_matches_source(segment: &RenderSegment, rate: u32, channels: u16) {
    let pcm = read_pcm(&segment.output, "f32le");
    let actual = pcm
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f64::from(f32::from_le_bytes(*bytes)));
    let mut energy = 0.0;
    let mut difference = 0.0;
    let edge = u64::from(rate / 4);
    let frame_count = segment.range.end - segment.range.start;
    for (index, sample) in actual.enumerate() {
        let frame = (index / usize::from(channels)) as u64;
        if frame < edge || frame >= frame_count - edge {
            continue;
        }
        let reference = signal(
            segment.range.start + frame,
            (index % usize::from(channels)) as u16,
        );
        energy += reference * reference;
        difference += (reference - sample).powi(2);
    }
    let snr = 10.0 * (energy / difference).log10();
    eprintln!("{rate} Hz, {channels}ch MP3 signal-to-error ratio: {snr:.2} dB");
    assert!(
        snr >= 12.0,
        "Audio distortion or timing regression: {snr:.2} dB"
    );
}

#[test]
fn rejects_invalid_or_cancelled_jobs_and_keeps_existing_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("source.wav");
    let frames = 40_000;
    pcm_wav(&wav, frames, 1, 16, 44_100);

    let file = File::open(&wav).unwrap();
    assert!(matches!(
        inspect_file(file, 128, || true),
        Err(MediaError::Cancelled)
    ));
    assert!(matches!(
        inspect(&wav, 0, || false),
        Err(MediaError::Range(_))
    ));
    assert!(matches!(
        waveform(
            &wav,
            FrameRange {
                start: frames + 10,
                end: frames + 30
            },
            12,
            || false
        ),
        Err(MediaError::Range(_))
    ));
    let segments = output_ranges(dir.path(), "invalid", "flac", &[0, 450, 400]);
    assert!(matches!(
        render(&wav, &segments, OutputFormat::Flac, || false, |_| {}),
        Err(MediaError::Range(_))
    ));
    assert!(segments.iter().all(|segment| !segment.output.exists()));

    let segments = output_ranges(dir.path(), "collision", "flac", &[0, frames]);
    fs::write(&segments[0].output, b"existing").unwrap();
    assert!(matches!(
        render(&wav, &segments, OutputFormat::Flac, || false, |_| {}),
        Err(MediaError::Io(_))
    ));
    assert_eq!(fs::read(&segments[0].output).unwrap(), b"existing");

    let segments = output_ranges(dir.path(), "cancel", "flac", &[0, frames / 2, frames]);
    let completed = std::cell::Cell::new(0);
    assert!(matches!(
        render(
            &wav,
            &segments,
            OutputFormat::Flac,
            || completed.get() == 1,
            |progress| completed.set(progress.completed_segments)
        ),
        Err(MediaError::Cancelled)
    ));
    assert_eq!(completed.get(), 1);
    assert!(segments[0].output.exists());
    assert!(!segments[1].output.exists());

    let unsupported = SourceInfo {
        sample_rate: 96_000,
        channels: 2,
        frames,
        bits_per_sample: Some(32),
    };
    assert!(check_output(&unsupported, OutputFormat::Flac).is_err());
    assert!(check_output(&unsupported, OutputFormat::Mp3 { bitrate_kbps: 128 }).is_err());
}

#[test]
fn rejects_a_truncated_pcm_file_by_declared_length() {
    let dir = tempfile::tempdir().unwrap();
    let wav: PathBuf = dir.path().join("truncated.wav");
    pcm_wav(&wav, 7051, 2, 16, 44_100);
    let file = File::options().write(true).open(&wav).unwrap();
    file.set_len(file.metadata().unwrap().len() - 301).unwrap();
    assert!(matches!(
        inspect(&wav, 128, || false),
        Err(MediaError::Decode(_))
    ));
}
