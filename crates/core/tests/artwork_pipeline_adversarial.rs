//! Independent callers for pixel geometry, encoded-envelope refusal and ownership.
//! Pixel expectations are specified from EXIF coordinates, never engine helpers.
use std::{
    cell::Cell,
    io::Write,
    process::{Command, Stdio},
    sync::Arc,
};
use tagrex_core::{
    cover::pipeline::{Error, Filter, Limits, Metadata, Options, Output, Processor, Stage, Step},
    model::{CoverArt, CoverKind},
    resources::{Budget, Charge, Limits as BudgetLimits},
};

fn engine() -> (Processor, Budget) {
    let budget = Budget::new(BudgetLimits {
        retained: 8 << 20,
        transient: 128 << 20,
        total: 136 << 20,
        jobs: 2,
    });
    let processor = Processor::new(
        budget.clone(),
        Limits {
            dimension: 64,
            pixels: 4096,
            work_bytes: 96 << 20,
            output_bytes: 1 << 20,
            cache_entries: 2,
            ..Limits::default()
        },
    )
    .unwrap();
    (processor, budget)
}
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut data = (payload.len() as u32).to_be_bytes().to_vec();
    data.extend(tag);
    data.extend(payload);
    data.extend(crc32(&data[4..]).to_be_bytes());
    data
}
fn insert(mut art: CoverArt, tag: &[u8; 4], payload: &[u8], before_end: bool) -> CoverArt {
    let at = if before_end { art.data.len() - 12 } else { 33 };
    let mut data = art.data[..at].to_vec();
    data.extend(chunk(tag, payload));
    data.extend(&art.data[at..]);
    art.data = data.into();
    art
}
fn exif(n: u16) -> Vec<u8> {
    // Big-endian TIFF, one SHORT Orientation entry, no linked IFD.
    let mut bytes = b"MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
    bytes.extend(n.to_be_bytes());
    bytes.extend([0; 6]);
    bytes
}
fn samples(depth: u8) -> (CoverArt, Vec<u16>) {
    let values: Vec<u16> = (0..6)
        .flat_map(|pixel| {
            [
                0x1234 + pixel * 0x1000,
                0xf123 - pixel * 0x1000,
                0x0123 + pixel * 0x0800,
                0xffff - pixel * 0x1000,
            ]
        })
        .collect();
    let raw: Vec<u8> = if depth == 16 {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    } else {
        values.iter().map(|v| (v >> 8) as u8).collect()
    };
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 3, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(if depth == 16 {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        });
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&raw)
            .unwrap();
    }
    (
        CoverArt {
            mime: "intentionally/not-trusted".into(),
            data: bytes.into(),
            kind: CoverKind::Back,
            description: "Independent — cover".into(),
        },
        values,
    )
}
fn expected_orientation(n: u16, depth: u8, samples: &[u16]) -> Vec<u8> {
    let (width, height) = if n >= 5 { (2, 3) } else { (3, 2) };
    let mut expected = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = match n {
                1 => (x, y),
                2 => (2 - x, y),
                3 => (2 - x, 1 - y),
                4 => (x, 1 - y),
                5 => (y, x),
                6 => (y, 1 - x),
                7 => (2 - y, 1 - x),
                8 => (2 - y, x),
                _ => unreachable!(),
            };
            for sample in &samples[((sy * 3 + sx) * 4)..((sy * 3 + sx) * 4 + 4)] {
                if depth == 16 {
                    expected.extend(sample.to_be_bytes());
                } else {
                    expected.push((sample >> 8) as u8);
                }
            }
        }
    }
    expected
}
fn ffmpeg_pixels(encoded: &[u8], depth: u8) -> Vec<u8> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            "pipe:0",
            "-f",
            "rawvideo",
            "-pix_fmt",
            if depth == 16 { "rgba64be" } else { "rgba" },
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("FFmpeg is required for this independent oracle");
    child.stdin.take().unwrap().write_all(encoded).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
#[ignore = "Independent FFmpeg pixel oracle; explicitly run by backend stress gates"]
fn every_exif_orientation_keeps_exact_eight_and_sixteen_bit_channel_coordinates() {
    let (processor, budget) = engine();
    for depth in [8, 16] {
        let (source, values) = samples(depth);
        for n in 1..=8 {
            let with_exif = insert(source.clone(), b"eXIf", &exif(n), n % 2 == 0);
            let result = processor
                .process(
                    &with_exif,
                    &Options {
                        steps: vec![Step::NormalizeOrientation],
                        output: Output::Png,
                        metadata: Metadata::Discard,
                        filter: Filter::Nearest,
                        ..Options::default()
                    },
                )
                .unwrap();
            assert_eq!(
                result.image().dimensions(),
                if n >= 5 { (2, 3) } else { (3, 2) }
            );
            assert_eq!(result.image().bits_per_channel(), depth);
            assert_eq!(
                ffmpeg_pixels(result.image().bytes(), depth),
                expected_orientation(n, depth, &values),
                "EXIF={n}, depth={depth}"
            );
        }
    }
    processor.clear_cache();
    drop(processor);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn metadata_after_pixels_is_subject_to_loss_policy_and_unknown_critical_chunks_are_refused() {
    let (processor, budget) = engine();
    let source = samples(8).0;
    let late_metadata = insert(source.clone(), b"tEXt", b"Comment\0late payload", true);
    let options = Options {
        output: Output::Png,
        ..Options::default()
    };
    assert!(matches!(
        processor.process(&late_metadata, &options),
        Err(Error::Unsupported(_))
    ));
    let retained = processor
        .process(&late_metadata, &Options::default())
        .unwrap();
    assert_eq!(retained.image().bytes(), late_metadata.data.as_ref());
    let discarded = processor
        .process(
            &late_metadata,
            &Options {
                metadata: Metadata::Discard,
                ..options.clone()
            },
        )
        .unwrap();
    assert!(discarded.image().notes().metadata_discarded);
    let critical = insert(source, b"ABCD", b"unknown critical semantics", false);
    assert!(processor
        .process(
            &critical,
            &Options {
                metadata: Metadata::Discard,
                ..options
            }
        )
        .is_err());
    drop((retained, discarded));
    processor.clear_cache();
    drop(processor);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn progress_reentrancy_and_nested_cancellation_cannot_poison_cache_or_release_live_owners() {
    let (processor, budget) = engine();
    let source = samples(8).0;
    let callback_ran = Cell::new(false);
    let nested = Cell::new(false);
    let result = processor
        .process_with_progress(
            &source,
            &Options {
                output: Output::Png,
                ..Options::default()
            },
            |stage| {
                if stage == Stage::Publish && !callback_ran.replace(true) {
                    let inner = processor
                        .process_with_progress(
                            &source,
                            &Options::default(),
                            |_| {
                                processor.clear_cache();
                                nested.set(true);
                            },
                            || false,
                        )
                        .unwrap();
                    assert_eq!(inner.image().bytes(), source.data.as_ref());
                }
            },
            || false,
        )
        .unwrap();
    assert!(nested.get());
    let image = result.image().clone();
    let weak = Arc::downgrade(&image);
    let cancelled = Cell::new(false);
    assert!(matches!(
        processor.process_with_progress(
            &source,
            &Options::default(),
            |_| cancelled.set(true),
            || cancelled.get()
        ),
        Err(Error::Cancelled)
    ));
    processor.clear_cache();
    drop((result, processor));
    assert!(budget.usage().0.retained >= image.bytes().len());
    assert_eq!(budget.usage().0.jobs, 0);
    assert_eq!(budget.usage().0.transient, 0);
    drop(image);
    assert!(weak.upgrade().is_none());
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn conflicting_orientation_entries_inside_one_exif_ifd_are_refused() {
    let (processor, _) = engine();
    // One IFD with two contradictory Orientation SHORT entries. Choosing the
    // first would silently rotate ambiguous metadata rather than refuse it.
    let mut tiff = b"MM\0*\0\0\0\x08\0\x02".to_vec();
    for orientation in [6u16, 3] {
        tiff.extend(b"\x01\x12\0\x03\0\0\0\x01");
        tiff.extend(orientation.to_be_bytes());
        tiff.extend([0, 0]);
    }
    tiff.extend([0; 4]);
    let source = insert(samples(8).0, b"eXIf", &tiff, false);
    let result = processor.process(
        &source,
        &Options {
            steps: vec![Step::NormalizeOrientation],
            output: Output::Png,
            metadata: Metadata::Discard,
            ..Options::default()
        },
    );
    assert!(
        matches!(result, Err(Error::Unsupported(_)) | Err(Error::Invalid(_))),
        "ambiguous EXIF orientation was accepted: {result:?}"
    );
}
