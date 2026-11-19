//! Fresh public-call recheck of bounded EXIF IFD0 orientation parsing.
//! Fixtures encode TIFF and carrier envelopes independently of the processor.
use std::{
    cell::Cell,
    io::Write,
    process::{Command, Stdio},
};
use tagrex_core::{
    cover::pipeline::{Error, Limits, Metadata, Options, Output, Processor, Stage, Step},
    model::{CoverArt, CoverKind},
    resources::{Budget, Charge, Limits as ResourceLimits},
};

fn processor() -> (Processor, Budget) {
    let budget = Budget::new(ResourceLimits {
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
fn options() -> Options {
    Options {
        steps: vec![Step::NormalizeOrientation],
        output: Output::Png,
        metadata: Metadata::Discard,
        ..Options::default()
    }
}
fn u16_bytes(value: u16, little: bool) -> [u8; 2] {
    if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    }
}
fn u32_bytes(value: u32, little: bool) -> [u8; 4] {
    if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    }
}
fn tiff(little: bool, orientations: &[u16]) -> Vec<u8> {
    let mut data = if little {
        b"II".to_vec()
    } else {
        b"MM".to_vec()
    };
    data.extend(u16_bytes(42, little));
    data.extend(u32_bytes(8, little));
    data.extend(u16_bytes(orientations.len().try_into().unwrap(), little));
    for value in orientations {
        data.extend(u16_bytes(0x112, little));
        data.extend(u16_bytes(3, little));
        data.extend(u32_bytes(1, little));
        data.extend(u16_bytes(*value, little));
        data.extend([0; 2]);
    }
    data.extend([0; 4]);
    data
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
fn source(jpeg: bool) -> CoverArt {
    let raw: Vec<u8> = (0..18).map(|n| 13 * n + 10).collect();
    let mut bytes = Vec::new();
    if jpeg {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
            .encode(&raw, 3, 2, image::ExtendedColorType::Rgb8)
            .unwrap();
    } else {
        let mut encoder = png::Encoder::new(&mut bytes, 3, 2);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&raw)
            .unwrap();
    }
    CoverArt {
        mime: "untrusted/type".into(),
        data: bytes.into(),
        kind: CoverKind::Front,
        description: "EXIF recheck".into(),
    }
}
fn decorated(jpeg: bool, tiff: &[u8]) -> CoverArt {
    let mut art = source(jpeg);
    let (at, envelope) = if jpeg {
        let mut bytes = vec![0xff, 0xe1];
        bytes.extend(u16::try_from(tiff.len() + 8).unwrap().to_be_bytes());
        bytes.extend(b"Exif\0\0");
        bytes.extend(tiff);
        (2, bytes)
    } else {
        let mut bytes = u32::try_from(tiff.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend(b"eXIf");
        bytes.extend(tiff);
        bytes.extend(crc32(&bytes[4..]).to_be_bytes());
        (33, bytes)
    };
    let mut data = art.data[..at].to_vec();
    data.extend(envelope);
    data.extend(&art.data[at..]);
    art.data = data.into();
    art
}
fn refused(result: Result<tagrex_core::cover::pipeline::ProcessedArtwork, Error>) -> bool {
    matches!(result, Err(Error::Invalid(_) | Error::Unsupported(_)))
}

#[test]
fn both_tiff_byte_orders_and_carriers_accept_consistent_duplicates_and_refuse_conflicts() {
    let (engine, budget) = processor();
    for jpeg in [false, true] {
        for little in [false, true] {
            for orientation in 1..=8 {
                let art = decorated(jpeg, &tiff(little, &[orientation, orientation]));
                let result = engine.process(&art, &options()).unwrap();
                assert_eq!(
                    result.image().dimensions(),
                    if orientation >= 5 { (2, 3) } else { (3, 2) }
                );
                assert_eq!(result.image().notes().orientation_applied, orientation != 1);
                assert!(result.image().notes().metadata_discarded);
            }
            for pair in [[1, 6], [6, 3], [8, 2]] {
                assert!(
                    refused(engine.process(&decorated(jpeg, &tiff(little, &pair)), &options())),
                    "jpeg={jpeg} little={little} pair={pair:?}"
                );
            }
        }
    }
    engine.clear_cache();
    drop(engine);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn malformed_ifd_layouts_and_orientation_shapes_refuse_without_poisoning_hot_results() {
    let (engine, budget) = processor();
    let hot_source = source(false);
    let hot = engine.process(&hot_source, &Options::default()).unwrap();
    for jpeg in [false, true] {
        for little in [false, true] {
            let valid = tiff(little, &[6]);
            let mut cases: Vec<Vec<u8>> = (0..valid.len()).map(|n| valid[..n].to_vec()).collect();
            for offset in [0, 1, 7, u32::MAX] {
                let mut data = valid.clone();
                data[4..8].copy_from_slice(&u32_bytes(offset, little));
                cases.push(data);
            }
            for revision in [0, 41, 43, u16::MAX] {
                let mut data = valid.clone();
                data[2..4].copy_from_slice(&u16_bytes(revision, little));
                cases.push(data);
            }
            for kind in [0, 1, 4, u16::MAX] {
                let mut data = valid.clone();
                data[12..14].copy_from_slice(&u16_bytes(kind, little));
                cases.push(data);
            }
            for count in [0, 2, u32::MAX] {
                let mut data = valid.clone();
                data[14..18].copy_from_slice(&u32_bytes(count, little));
                cases.push(data);
            }
            for value in [0, 9, 255, 256, u16::MAX] {
                cases.push(tiff(little, &[value]));
            }
            let mut invalid_count = valid.clone();
            invalid_count[8..10].copy_from_slice(&u16_bytes(u16::MAX, little));
            cases.push(invalid_count);
            let mut invalid_order = valid.clone();
            invalid_order[..2].copy_from_slice(b"ZZ");
            cases.push(invalid_order);
            for (index, data) in cases.iter().enumerate() {
                let before = budget.usage().0;
                assert!(
                    refused(engine.process(&decorated(jpeg, data), &options())),
                    "accepted malformed TIFF case{index} jpeg={jpeg} little={little}"
                );
                assert_eq!(budget.usage().0, before, "failed parse retained resources");
                assert_eq!(engine.cache_stats().entries, 1);
            }
        }
    }
    let hot_again = engine.process(&hot_source, &Options::default()).unwrap();
    assert!(std::sync::Arc::ptr_eq(hot.image(), hot_again.image()));
    drop((hot, hot_again));
    engine.clear_cache();
    drop(engine);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn thumbnail_ifd_orientation_is_distinct_from_the_main_image() {
    let (engine, budget) = processor();
    for jpeg in [false, true] {
        for little in [false, true] {
            let mut bytes = tiff(little, &[1]);
            // TIFF offsets are relative to the TIFF header. IFD1 is a thumbnail;
            // its rotation must not override or conflict with the main image.
            let next = bytes.len() as u32;
            let len = bytes.len();
            bytes[len - 4..].copy_from_slice(&u32_bytes(next, little));
            bytes.extend(&tiff(little, &[6])[8..]);
            let result = engine
                .process(&decorated(jpeg, &bytes), &options())
                .unwrap();
            assert_eq!(result.image().dimensions(), (3, 2));
            assert!(!result.image().notes().orientation_applied);
        }
    }
    engine.clear_cache();
    drop(engine);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn maximum_ifd_entries_have_interruptible_scanning_and_no_partial_cache_publication() {
    let (engine, budget) = processor();
    let mut bytes = b"MM\0*\0\0\0\x08\xff\xff".to_vec();
    for _ in 0..u16::MAX - 1 {
        bytes.extend([0x01, 0, 0, 3, 0, 0, 0, 1, 0, 1, 0, 0]);
    }
    bytes.extend([0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
    bytes.extend([0; 4]);
    let art = decorated(false, &bytes);
    let baseline = budget.usage().0;
    for threshold in [1, 2, 64, 128, 256, 512] {
        let armed = Cell::new(false);
        let checks = Cell::new(0usize);
        let result = engine.process_with_progress(
            &art,
            &options(),
            |stage| {
                if stage == Stage::Inspect {
                    armed.set(true);
                }
            },
            || {
                if !armed.get() {
                    return false;
                }
                checks.set(checks.get() + 1);
                checks.get() >= threshold
            },
        );
        assert!(
            matches!(result, Err(Error::Cancelled)),
            "threshold {threshold}: {result:?}"
        );
        assert_eq!(checks.get(), threshold);
        assert_eq!(engine.cache_stats().entries, 0);
        assert_eq!(budget.usage().0, baseline);
    }
    let result = engine.process(&art, &options()).unwrap();
    assert_eq!(result.image().dimensions(), (2, 3));
    drop(result);
    engine.clear_cache();
    drop(engine);
    assert_eq!(budget.usage().0, Charge::default());
}

fn decoded_rgb(encoded: &[u8]) -> Vec<u8> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-v", "error", "-i", "pipe:0", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("FFmpeg required for independent pixel oracle");
    child.stdin.take().unwrap().write_all(encoded).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn expected_rgb(orientation: u16, pixels: &[u8]) -> Vec<u8> {
    let (w, h) = if orientation >= 5 { (2, 3) } else { (3, 2) };
    let mut expected = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = match orientation {
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
            expected.extend(&pixels[(sy * 3 + sx) * 3..(sy * 3 + sx) * 3 + 3]);
        }
    }
    expected
}

#[test]
#[ignore = "Independent FFmpeg RGB oracle; explicitly run by backend stress gates"]
fn little_endian_duplicate_orientation_pixels_match_an_independent_decoder() {
    let (engine, budget) = processor();
    for jpeg in [false, true] {
        let baseline = decoded_rgb(source(jpeg).data.as_ref());
        for orientation in 1..=8 {
            let art = decorated(jpeg, &tiff(true, &[orientation, orientation]));
            let result = engine.process(&art, &options()).unwrap();
            let actual = decoded_rgb(result.image().bytes());
            let expected = expected_rgb(orientation, &baseline);
            assert_eq!(actual.len(), expected.len());
            // JPEG decoders can differ slightly in YCbCr conversion rounding.
            // This tolerance cannot disguise a coordinate permutation of the
            // distinct sample ramp. PNG remains byte-exact.
            let tolerance = if jpeg { 3 } else { 0 };
            assert!(
                actual
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| a.abs_diff(*b) <= tolerance),
                "jpeg={jpeg} orientation={orientation}: actual={actual:?}, expected={expected:?}"
            );
        }
    }
    engine.clear_cache();
    drop(engine);
    assert_eq!(budget.usage().0, Charge::default());
}
