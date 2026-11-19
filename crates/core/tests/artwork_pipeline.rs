//! Public strict pixel-engine callers. Expected pixels are constructed directly,
//! not by calling the engine's geometry or accounting helpers.
use image::{DynamicImage, ImageBuffer, Rgba};
use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Barrier,
    },
};
use tagrex_core::{
    cover::pipeline::{
        BitDepth, Error, Filter, Limits, Metadata, Options, Output, Processor, Stage, Step, Upscale,
    },
    model::{CoverArt, CoverKind},
    resources::{Budget, Charge, Limits as BudgetLimits},
};

fn setup(limits: Limits) -> (Processor, Budget) {
    let budget = Budget::new(BudgetLimits {
        retained: 128 << 20,
        transient: 320 << 20,
        total: 448 << 20,
        jobs: 4,
    });
    (Processor::new(budget.clone(), limits).unwrap(), budget)
}
fn art(w: u32, h: u32) -> CoverArt {
    let mut bytes = Vec::new();
    let pixels: Vec<u8> = (0..h)
        .flat_map(|y| {
            (0..w).flat_map(move |x| [x as u8 * 20, y as u8 * 30, (x + y) as u8 * 10, 255])
        })
        .collect();
    {
        let mut encoder = png::Encoder::new(&mut bytes, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
    }
    CoverArt {
        mime: "image/png".into(),
        data: bytes.into(),
        kind: CoverKind::Leaflet,
        description: "Backstage — 東京".into(),
    }
}
fn opts(steps: Vec<Step>) -> Options {
    Options {
        steps,
        filter: Filter::Nearest,
        ..Options::default()
    }
}
fn png_pixels(bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory(bytes).unwrap().into_rgba8()
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}
fn chunk(tag: &[u8; 4], bytes: &[u8]) -> Vec<u8> {
    let mut result = (bytes.len() as u32).to_be_bytes().to_vec();
    result.extend(tag);
    result.extend(bytes);
    result.extend(crc32(&result[4..]).to_be_bytes());
    result
}
fn ancillary(mut art: CoverArt, tag: &[u8; 4], bytes: &[u8]) -> CoverArt {
    let mut data = art.data[..33].to_vec();
    data.extend(chunk(tag, bytes));
    data.extend(&art.data[33..]);
    art.data = data.into();
    art
}
fn exif(orientation: u16) -> Vec<u8> {
    let mut data = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
    data.extend(orientation.to_le_bytes());
    data.extend([0; 6]);
    data
}
#[test]
fn original_noop_validates_pixels_retains_bytes_and_shares_successful_results_without_losing_properties(
) {
    let (processor, budget) = setup(Limits::default());
    let original = art(5, 3);
    let result = processor.process(&original, &Options::default()).unwrap();
    assert!(std::ptr::eq(
        result.image().bytes().as_ptr(),
        original.data.as_ptr()
    ));
    assert_eq!(result.image().bytes(), original.data.as_ref());
    assert_eq!(result.kind(), CoverKind::Leaflet);
    assert_eq!(result.description(), "Backstage — 東京");
    assert!(!result.image().notes().reencoded);
    let mut another = original.clone();
    another.kind = CoverKind::Logo;
    another.description = "A different picture property".into();
    let second = processor.process(&another, &Options::default()).unwrap();
    assert!(Arc::ptr_eq(result.image(), second.image()));
    assert_eq!(second.kind(), CoverKind::Logo);
    assert_eq!(second.description(), another.description);
    let weak = Arc::downgrade(result.image());
    let retained = result.image().clone();
    drop((result, second, processor));
    assert!(
        budget.usage().0.retained > 0,
        "the escaped immutable owner lost its lease"
    );
    drop(retained);
    assert!(weak.upgrade().is_none());
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn centre_crop_portrait_and_landscape_discard_odd_extra_space_on_bottom_or_right() {
    let (processor, _) = setup(Limits::default());
    for (w, h, x0, y0) in [(6, 3, 1, 0), (3, 6, 0, 1)] {
        let source = art(w, h);
        let result = processor
            .process(&source, &opts(vec![Step::SquareCrop { size: 3 }]))
            .unwrap();
        assert_eq!(result.image().dimensions(), (3, 3));
        let pixels = png_pixels(result.image().bytes());
        for y in 0..3 {
            for x in 0..3 {
                assert_eq!(
                    pixels.get_pixel(x, y).0,
                    [
                        (x + x0) as u8 * 20,
                        (y + y0) as u8 * 30,
                        (x + x0 + y + y0) as u8 * 10,
                        255
                    ]
                );
            }
        }
    }
}
#[test]
fn proportional_fit_floor_rounding_exact_resize_and_explicit_upscale_policy() {
    let (processor, _) = setup(Limits::default());
    let source = art(7, 3);
    let fitted = processor
        .process(
            &source,
            &opts(vec![Step::Fit {
                width: 4,
                height: 4,
            }]),
        )
        .unwrap();
    assert_eq!(fitted.image().dimensions(), (4, 1));
    let tiny = art(1, 1);
    let no_upscale = processor
        .process(
            &tiny,
            &opts(vec![Step::Fit {
                width: 64,
                height: 64,
            }]),
        )
        .unwrap();
    assert!(std::ptr::eq(
        no_upscale.image().bytes().as_ptr(),
        tiny.data.as_ptr()
    ));
    assert!(matches!(
        processor.process(&tiny, &opts(vec![Step::SquareScale { size: 3 }])),
        Err(Error::Invalid(_))
    ));
    let mut allow = opts(vec![Step::SquareScale { size: 3 }]);
    allow.upscale = Upscale::Allow;
    let bigger = processor.process(&tiny, &allow).unwrap();
    assert_eq!(bigger.image().dimensions(), (3, 3));
    assert!(png_pixels(bigger.image().bytes())
        .pixels()
        .all(|pixel| pixel.0 == [0, 0, 0, 255]));
    let stretched = processor
        .process(
            &art(4, 2),
            &Options {
                upscale: Upscale::Allow,
                ..opts(vec![Step::Resize {
                    width: 2,
                    height: 4,
                }])
            },
        )
        .unwrap();
    let decoded = png_pixels(stretched.image().bytes());
    for (y, source_y) in [0, 0, 1, 1].into_iter().enumerate() {
        for (x, source_x) in [1, 3].into_iter().enumerate() {
            assert_eq!(
                decoded.get_pixel(x as u32, y as u32).0,
                [
                    source_x * 20,
                    source_y * 30,
                    (source_x + source_y) * 10,
                    255
                ]
            );
        }
    }
}
#[test]
fn ordered_steps_filters_quality_background_and_input_content_are_distinct_cache_identities() {
    let (processor, _) = setup(Limits::default());
    let source = art(6, 4);
    let a = opts(vec![
        Step::Fit {
            width: 3,
            height: 2,
        },
        Step::SquareCrop { size: 2 },
    ]);
    let b = opts(vec![
        Step::SquareCrop { size: 2 },
        Step::Fit {
            width: 3,
            height: 2,
        },
    ]);
    let first = processor.process(&source, &a).unwrap();
    let second = processor.process(&source, &b).unwrap();
    assert_ne!(first.image().bytes(), second.image().bytes());
    assert!(Arc::ptr_eq(
        first.image(),
        processor.process(&source, &a).unwrap().image()
    ));
    let mut triangle = a.clone();
    triangle.filter = Filter::Triangle;
    assert!(!Arc::ptr_eq(
        first.image(),
        processor.process(&source, &triangle).unwrap().image()
    ));
    let mut jpeg = a.clone();
    jpeg.output = Output::Jpeg;
    let encoded = processor.process(&source, &jpeg).unwrap();
    jpeg.jpeg_quality = 95;
    let quality = processor.process(&source, &jpeg).unwrap();
    assert!(!Arc::ptr_eq(encoded.image(), quality.image()));
    jpeg.jpeg_background = [0, 0, 0];
    assert!(!Arc::ptr_eq(
        quality.image(),
        processor.process(&source, &jpeg).unwrap().image()
    ));
    assert!(!Arc::ptr_eq(
        first.image(),
        processor.process(&art(5, 4), &a).unwrap().image()
    ));
}
#[test]
fn jpeg_alpha_background_is_explicit_and_quality_is_lossy() {
    let (processor, _) = setup(Limits::default());
    let rgba = ImageBuffer::from_pixel(16, 16, Rgba([255, 0, 0, 128]));
    let mut data = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(rgba)
        .write_to(&mut data, image::ImageFormat::Png)
        .unwrap();
    let mut source = art(1, 1);
    source.data = data.into_inner().into();
    let result = processor
        .process(
            &source,
            &Options {
                output: Output::Jpeg,
                jpeg_quality: 100,
                jpeg_background: [0, 0, 255],
                ..Options::default()
            },
        )
        .unwrap();
    assert_eq!(result.image().mime(), "image/jpeg");
    assert!(result.image().notes().alpha_composited);
    let image = image::load_from_memory(result.image().bytes())
        .unwrap()
        .into_rgb8();
    for p in image.pixels() {
        for (c, expected) in p.0.into_iter().zip([128i16, 0, 127]) {
            assert!((i16::from(c) - expected).abs() <= 3);
        }
    }
}
#[test]
fn metadata_loss_and_orientation_are_explicit_and_original_keeps_opaque_metadata_bytes() {
    let (processor, _) = setup(Limits::default());
    let source = ancillary(art(3, 2), b"eXIf", &exif(6));
    let original = processor.process(&source, &Options::default()).unwrap();
    assert_eq!(original.image().bytes(), source.data.as_ref());
    assert!(matches!(
        processor.process(
            &source,
            &Options {
                output: Output::Png,
                ..Options::default()
            }
        ),
        Err(Error::Unsupported(_))
    ));
    let discard = Options {
        metadata: Metadata::Discard,
        output: Output::Png,
        ..Options::default()
    };
    assert!(matches!(
        processor.process(&source, &discard),
        Err(Error::Unsupported(_))
    ));
    let result = processor
        .process(
            &source,
            &Options {
                steps: vec![Step::NormalizeOrientation],
                ..discard
            },
        )
        .unwrap();
    assert_eq!(result.image().dimensions(), (2, 3));
    assert!(result.image().notes().orientation_applied);
    assert!(result.image().notes().metadata_discarded);
    let image = png_pixels(result.image().bytes());
    assert_eq!(image.get_pixel(0, 0).0, [0, 30, 10, 255]);
    assert_eq!(image.get_pixel(1, 0).0, [0, 0, 0, 255]);
    let profile = ancillary(art(2, 2), b"iCCP", b"profile\0\0opaque-compressed-metadata");
    assert!(matches!(
        processor.process(
            &profile,
            &Options {
                output: Output::Png,
                ..Options::default()
            }
        ),
        Err(Error::Unsupported(_))
    ));
    let stripped = processor
        .process(
            &profile,
            &Options {
                metadata: Metadata::Discard,
                output: Output::Png,
                ..Options::default()
            },
        )
        .unwrap();
    assert!(stripped.image().notes().metadata_discarded);
}
fn sixteen_bit() -> CoverArt {
    let values: Vec<u16> = (0..4)
        .flat_map(|i| [i * 10_000, 5_000 + i * 100, 65_535 - i * 1000, 65_535])
        .collect();
    let bytes: Vec<u8> = values.into_iter().flat_map(u16::to_be_bytes).collect();
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Sixteen);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&bytes)
            .unwrap();
    }
    let mut source = art(1, 1);
    source.data = output.into();
    source
}
#[test]
fn sixteen_bit_png_survives_pixels_and_reencoding_while_jpeg_requires_precision_loss_permission() {
    let (processor, _) = setup(Limits::default());
    let source = sixteen_bit();
    let no_op = processor.process(&source, &Options::default()).unwrap();
    assert_eq!(no_op.image().bytes(), source.data.as_ref());
    assert_eq!(no_op.image().bits_per_channel(), 16);
    let result = processor
        .process(
            &source,
            &opts(vec![Step::Resize {
                width: 1,
                height: 1,
            }]),
        )
        .unwrap();
    assert_eq!(result.image().bits_per_channel(), 16);
    let image = image::load_from_memory(result.image().bytes()).unwrap();
    assert_eq!(
        image.into_rgba16().get_pixel(0, 0).0,
        [30_000, 5_300, 62_535, 65_535]
    );
    assert!(matches!(
        processor.process(
            &source,
            &Options {
                output: Output::Jpeg,
                ..Options::default()
            }
        ),
        Err(Error::Unsupported(_))
    ));
    let reduced = processor
        .process(
            &source,
            &Options {
                output: Output::Jpeg,
                bit_depth: BitDepth::Allow8Bit,
                ..Options::default()
            },
        )
        .unwrap();
    assert_eq!(reduced.image().bits_per_channel(), 8);
    assert!(reduced.image().notes().reduced_to_8_bit);
}
#[test]
fn malformed_headers_animation_and_decompression_pressure_have_strict_errors_and_release_jobs() {
    let (processor, budget) = setup(Limits::default());
    let baseline = budget.usage().0;
    let mut huge = art(1, 1);
    let mut bytes = huge.data.to_vec();
    bytes[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    let crc = crc32(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&crc.to_be_bytes());
    huge.data = bytes.into();
    assert!(matches!(
        processor.process(&huge, &Options::default()),
        Err(Error::OverBudget(_))
    ));
    let animated = ancillary(art(1, 1), b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]);
    assert!(matches!(
        processor.process(&animated, &Options::default()),
        Err(Error::Unsupported(_))
    ));
    let mut broken = art(2, 2);
    let mut bytes = broken.data.to_vec();
    bytes.truncate(bytes.len() - 1);
    broken.data = bytes.into();
    assert!(matches!(
        processor.process(&broken, &Options::default()),
        Err(Error::Invalid(_))
    ));
    assert_eq!(budget.usage().0, baseline);
    let (small, scratch) = setup(Limits {
        decoder_scratch: 32,
        ..Limits::default()
    });
    assert!(matches!(
        small.process(&art(9, 9), &Options::default()),
        Err(Error::OverBudget(_))
    ));
    assert_eq!(scratch.usage().0.jobs, 0);
    let (tiny, _) = setup(Limits {
        output_bytes: 32,
        cache_entries: 0,
        ..Limits::default()
    });
    assert!(matches!(
        tiny.process(
            &art(3, 3),
            &Options {
                output: Output::Png,
                ..Options::default()
            }
        ),
        Err(Error::OverBudget(_))
    ));
}
#[test]
fn cancelling_each_public_stage_never_publishes_or_caches_partial_work() {
    for stage in [
        Stage::Inspect,
        Stage::Decode,
        Stage::Transform(0),
        Stage::Transform(1),
        Stage::Encode,
        Stage::Publish,
    ] {
        let (processor, budget) = setup(Limits::default());
        let baseline = budget.usage().0;
        let cancelled = AtomicBool::new(false);
        let result = processor.process_with_progress(
            &art(8, 6),
            &opts(vec![
                Step::Fit {
                    width: 4,
                    height: 3,
                },
                Step::SquareCrop { size: 3 },
            ]),
            |at| {
                if at == stage {
                    cancelled.store(true, Ordering::SeqCst);
                }
            },
            || cancelled.load(Ordering::SeqCst),
        );
        assert!(
            matches!(result, Err(Error::Cancelled)),
            "stage {stage:?}: {result:?}"
        );
        assert_eq!(processor.cache_stats().entries, 0);
        assert_eq!(budget.usage().0, baseline);
    }
}
#[test]
fn shared_job_admission_is_nonblocking_and_a_cancelled_worker_releases_capacity() {
    let budget = Budget::new(BudgetLimits {
        retained: 64 << 20,
        transient: 128 << 20,
        total: 192 << 20,
        jobs: 1,
    });
    let processor = Processor::new(budget.clone(), Limits::default()).unwrap();
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let worker = processor.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let handle = std::thread::spawn(move || {
        worker.process_with_progress(
            &art(8, 6),
            &opts(vec![Step::SquareCrop { size: 4 }]),
            |stage| {
                if stage == Stage::Decode {
                    entered_tx.send(()).unwrap();
                    release_rx
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                }
            },
            || flag.load(Ordering::SeqCst),
        )
    });
    entered
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert_eq!(budget.usage().0.jobs, 1);
    assert!(matches!(
        processor.process(&art(7, 5), &Options::default()),
        Err(Error::Busy)
    ));
    cancelled.store(true, Ordering::SeqCst);
    release.send(()).unwrap();
    assert!(matches!(handle.join().unwrap(), Err(Error::Cancelled)));
    assert_eq!(budget.usage().0.jobs, 0);
    assert_eq!(processor.cache_stats().entries, 0);
    assert!(processor.process(&art(7, 5), &Options::default()).is_ok());
    drop(processor);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn failed_retained_admission_preserves_hot_cache_entries_and_lru_eviction_keeps_live_owners_charged(
) {
    let budget = Budget::new(BudgetLimits {
        retained: 1 << 20,
        transient: 128 << 20,
        total: 160 << 20,
        jobs: 2,
    });
    let processor = Processor::new(
        budget.clone(),
        Limits {
            cache_entries: 2,
            ..Limits::default()
        },
    )
    .unwrap();
    let source = art(6, 4);
    let a = opts(vec![Step::SquareCrop { size: 3 }]);
    let first = processor.process(&source, &a).unwrap();
    let used = budget.usage().0.retained;
    let pressure = budget
        .try_acquire(Charge {
            retained: (1 << 20) - used - 100,
            ..Charge::default()
        })
        .unwrap();
    assert!(matches!(
        processor.process(&art(5, 4), &a),
        Err(Error::Busy)
    ));
    assert_eq!(processor.cache_stats().entries, 1);
    drop(pressure);
    assert!(Arc::ptr_eq(
        first.image(),
        processor.process(&source, &a).unwrap().image()
    ));
    let second = processor.process(&art(5, 4), &a).unwrap();
    processor.process(&source, &a).unwrap();
    let third = processor.process(&art(4, 4), &a).unwrap();
    assert_eq!(processor.cache_stats().entries, 2);
    let weak = Arc::downgrade(second.image());
    drop(processor);
    assert!(
        weak.upgrade().is_some(),
        "evicted live result lost its encoded owner"
    );
    drop((first, second, third));
    assert!(weak.upgrade().is_none());
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn concurrent_option_and_cache_pressure_stays_within_shared_limits() {
    let budget = Budget::new(BudgetLimits {
        retained: 8 << 20,
        transient: 96 << 20,
        total: 104 << 20,
        jobs: 2,
    });
    let processor = Processor::new(
        budget.clone(),
        Limits {
            cache_entries: 0,
            ..Limits::default()
        },
    )
    .unwrap();
    let barrier = Arc::new(Barrier::new(12));
    let successes = AtomicUsize::new(0);
    let busy = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for id in 0..12 {
            let processor = processor.clone();
            let barrier = barrier.clone();
            let successes = &successes;
            let busy = &busy;
            scope.spawn(move || {
                barrier.wait();
                for run in 0..30 {
                    let source = art(8 + id % 3, 6);
                    let options = Options {
                        jpeg_quality: 20 + (run * 2) as u8,
                        output: if run % 2 == 0 {
                            Output::Png
                        } else {
                            Output::Jpeg
                        },
                        ..opts(vec![Step::SquareCrop { size: 3 + run % 2 }])
                    };
                    match processor.process(&source, &options) {
                        Ok(result) => {
                            assert_eq!(result.image().dimensions(), (3 + run % 2, 3 + run % 2));
                            successes.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(Error::Busy) => {
                            busy.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(error) => panic!("unexpected processing refusal: {error}"),
                    }
                }
            });
        }
    });
    assert!(successes.load(Ordering::Relaxed) > 0);
    assert!(busy.load(Ordering::Relaxed) > 0);
    let peak = budget.usage().1;
    assert!(peak.transient <= 96 << 20 && peak.retained <= 8 << 20 && peak.jobs <= 2);
    drop(processor);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn jpeg_metadata_before_frame_header_is_never_silently_lost_and_jpeg_original_noop_is_exact() {
    let (processor, _) = setup(Limits::default());
    let encoded = processor
        .process(
            &art(8, 6),
            &Options {
                output: Output::Jpeg,
                ..Options::default()
            },
        )
        .unwrap();
    let source = CoverArt {
        data: Arc::from(encoded.image().bytes()),
        mime: "image/jpeg".into(),
        kind: CoverKind::Back,
        description: "JPEG metadata".into(),
    };
    let original = processor.process(&source, &Options::default()).unwrap();
    assert_eq!(original.image().bytes(), source.data.as_ref());
    assert!(matches!(
        processor.process(
            &source,
            &opts(vec![Step::Fit {
                width: 4,
                height: 4
            }])
        ),
        Err(Error::Unsupported(_))
    ));
    let resized = processor
        .process(
            &source,
            &Options {
                metadata: Metadata::Discard,
                ..opts(vec![Step::Fit {
                    width: 4,
                    height: 4,
                }])
            },
        )
        .unwrap();
    assert_eq!(resized.image().dimensions(), (4, 3));
    assert!(resized.image().notes().metadata_discarded);
}
#[test]
fn malformed_pixel_data_is_not_accepted_as_an_original_noop_or_put_in_the_cache() {
    let (processor, budget) = setup(Limits::default());
    let baseline = budget.usage().0;
    let mut broken = art(3, 3);
    let mut data = broken.data.to_vec();
    let mut at = 8;
    while at < data.len() {
        let len = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        if &data[at + 4..at + 8] == b"IDAT" {
            data[at + 8] = 0;
            let crc = crc32(&data[at + 4..at + 8 + len]);
            data[at + 8 + len..at + 12 + len].copy_from_slice(&crc.to_be_bytes());
            break;
        }
        at += 12 + len;
    }
    broken.data = data.into();
    assert!(matches!(
        processor.process(&broken, &Options::default()),
        Err(Error::Invalid(_))
    ));
    assert_eq!(processor.cache_stats().entries, 0);
    assert_eq!(budget.usage().0, baseline);
}
#[test]
fn configured_input_step_output_and_intermediate_budgets_are_enforced_before_publication() {
    let source = art(8, 6);
    let (p, _) = setup(Limits {
        input_bytes: 8,
        ..Limits::default()
    });
    assert!(matches!(
        p.process(&source, &Options::default()),
        Err(Error::OverBudget(_))
    ));
    let (p, _) = setup(Limits {
        steps: 1,
        ..Limits::default()
    });
    assert!(matches!(
        p.process(
            &source,
            &opts(vec![
                Step::Fit {
                    width: 4,
                    height: 3
                },
                Step::SquareCrop { size: 2 }
            ])
        ),
        Err(Error::OverBudget(_))
    ));
    let (p, budget) = setup(Limits {
        work_bytes: 1 << 20,
        ..Limits::default()
    });
    let baseline = budget.usage().0;
    let reached_decode = AtomicBool::new(false);
    let result = p.process_with_progress(
        &source,
        &Options::default(),
        |stage| {
            if stage == Stage::Decode {
                reached_decode.store(true, Ordering::SeqCst);
            }
        },
        || false,
    );
    assert!(matches!(result, Err(Error::OverBudget(_))));
    assert!(!reached_decode.load(Ordering::SeqCst));
    assert_eq!(budget.usage().0, baseline);
}

#[test]
fn byte_retaining_noop_also_obeys_encoded_output_limit() {
    let (processor, budget) = setup(Limits {
        output_bytes: 32,
        ..Limits::default()
    });
    let baseline = budget.usage().0;
    assert!(matches!(
        processor.process(&art(3, 3), &Options::default()),
        Err(Error::OverBudget(_))
    ));
    assert_eq!(processor.cache_stats().entries, 0);
    assert_eq!(budget.usage().0, baseline);
}
#[test]
fn caller_progress_panic_releases_work_and_does_not_poison_future_processing() {
    let (processor, budget) = setup(Limits::default());
    let baseline = budget.usage().0;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        processor.process_with_progress(
            &art(8, 6),
            &opts(vec![Step::SquareCrop { size: 4 }]),
            |stage| {
                if stage == Stage::Decode {
                    panic!("injected progress failure");
                }
            },
            || false,
        )
    }));
    assert!(result.is_err());
    assert_eq!(budget.usage().0, baseline);
    assert_eq!(processor.cache_stats().entries, 0);
    assert!(processor.process(&art(8, 6), &Options::default()).is_ok());
    drop(processor);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
#[ignore = "manual independent FFmpeg/FFprobe PNG/JPEG/PNG16 validation"]
fn independent_encoded_pixels_and_original_bytes_roundtrip() {
    use std::process::Command;
    let (processor, _) = setup(Limits::default());
    let source = art(8, 6);
    let root = std::env::temp_dir().join(format!("tagrex-artwork-pipeline-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let original = processor.process(&source, &Options::default()).unwrap();
    assert_eq!(original.image().bytes(), source.data.as_ref());
    let result = processor
        .process(
            &source,
            &opts(vec![
                Step::SquareCrop { size: 6 },
                Step::Resize {
                    width: 3,
                    height: 3,
                },
            ]),
        )
        .unwrap();
    let path = root.join("pixels.png");
    std::fs::write(&path, result.image().bytes()).unwrap();
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height",
            "-of",
            "csv=p=0",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(probe.status.success());
    assert_eq!(String::from_utf8(probe.stdout).unwrap().trim(), "png,3,3");
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    let expected: Vec<u8> = [1u8, 3, 5]
        .into_iter()
        .flat_map(|y| {
            [2u8, 4, 6]
                .into_iter()
                .flat_map(move |x| [x * 20, y * 30, (x + y) * 10, 255])
        })
        .collect();
    assert_eq!(decoded.stdout, expected);
    let mut transparent = source.clone();
    let image = ImageBuffer::from_pixel(16, 16, Rgba([255u8, 0, 0, 128]));
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    transparent.data = png.into_inner().into();
    let jpeg = processor
        .process(
            &transparent,
            &Options {
                output: Output::Jpeg,
                jpeg_quality: 100,
                jpeg_background: [0, 0, 255],
                ..Options::default()
            },
        )
        .unwrap();
    let path = root.join("alpha.jpg");
    std::fs::write(&path, jpeg.image().bytes()).unwrap();
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    assert_eq!(decoded.stdout.len(), 16 * 16 * 3);
    for rgb in decoded.stdout.as_chunks::<3>().0 {
        for (value, expected) in rgb.iter().zip([128i16, 0, 127]) {
            assert!((i16::from(*value) - expected).abs() <= 4);
        }
    }
    let source = sixteen_bit();
    let sixteen = processor
        .process(
            &source,
            &opts(vec![Step::Resize {
                width: 1,
                height: 1,
            }]),
        )
        .unwrap();
    let path = root.join("precision.png");
    std::fs::write(&path, sixteen.image().bytes()).unwrap();
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba64be",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    assert_eq!(
        decoded.stdout,
        [30_000u16, 5_300, 62_535, 65_535]
            .into_iter()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(root).unwrap();
    println!("Independent FFprobe/FFmpeg verified PNG geometry/pixels, JPEG alpha background and PNG16 samples; original no-op bytes unchanged");
}

#[test]
fn clearing_cache_does_not_uncharge_or_destroy_a_live_shared_result() {
    let (processor, budget) = setup(Limits::default());
    let baseline = budget.usage().0;
    let result = processor.process(&art(4, 3), &Options::default()).unwrap();
    let live = result.clone();
    processor.clear_cache();
    assert_eq!(processor.cache_stats().entries, 0);
    assert!(budget.usage().0.retained > baseline.retained);
    drop(result);
    assert_eq!(live.image().dimensions(), (4, 3));
    drop(live);
    assert_eq!(budget.usage().0, baseline);
    drop(processor);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn hostile_truncations_and_mutations_stay_bounded_and_do_not_poison_later_calls() {
    let (processor, budget) = setup(Limits {
        dimension: 32,
        pixels: 1024,
        output_bytes: 64 << 10,
        decoder_scratch: 1 << 20,
        work_bytes: 16 << 20,
        cache_entries: 0,
        ..Limits::default()
    });
    let baseline = budget.usage().0;
    let source = art(8, 6);
    let jpeg = processor
        .process(
            &source,
            &Options {
                output: Output::Jpeg,
                ..Options::default()
            },
        )
        .unwrap();
    let seeds = [source.data.to_vec(), jpeg.image().bytes().to_vec()];
    drop(jpeg);
    let options = Options {
        output: Output::Png,
        metadata: Metadata::Discard,
        orientation: tagrex_core::cover::pipeline::OrientationPolicy::Ignore,
        ..opts(vec![Step::Fit {
            width: 8,
            height: 8,
        }])
    };
    let mut state = 0x9a37_41e1u32;
    let mut accepted = 0;
    for seed in seeds {
        for iteration in 0..1000 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let mut data = seed.clone();
            if iteration % 3 == 0 {
                data.truncate(state as usize % data.len());
            } else {
                for _ in 0..1 + iteration % 4 {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    let index = state as usize % data.len();
                    data[index] ^= 1 << (state % 8);
                }
            }
            let mut candidate = source.clone();
            candidate.data = data.into();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                processor.process(&candidate, &options)
            }))
            .expect("malformed artwork panicked instead of returning a refusal");
            if let Ok(result) = result {
                let (w, h) = result.image().dimensions();
                assert!(w <= 8 && h <= 8 && result.image().bytes().len() <= 64 << 10);
                accepted += 1;
            }
            assert_eq!(budget.usage().0, baseline);
        }
    }
    assert!(processor.process(&source, &Options::default()).is_ok());
    println!("2000 bounded header/payload mutations and truncations; {accepted} decoded by the configured codec policy");
}
#[test]
fn ordered_intermediate_allocations_are_admitted_before_final_small_output() {
    let (processor, budget) = setup(Limits {
        work_bytes: 1 << 20,
        decoder_scratch: 4096,
        output_bytes: 1024,
        ..Limits::default()
    });
    let baseline = budget.usage().0;
    let decoded = AtomicBool::new(false);
    let result = processor.process_with_progress(
        &art(2, 2),
        &Options {
            upscale: Upscale::Allow,
            ..opts(vec![
                Step::Resize {
                    width: 512,
                    height: 512,
                },
                Step::Resize {
                    width: 1,
                    height: 1,
                },
            ])
        },
        |stage| {
            if stage == Stage::Decode {
                decoded.store(true, Ordering::SeqCst);
            }
        },
        || false,
    );
    assert!(matches!(result, Err(Error::OverBudget(_))));
    assert!(!decoded.load(Ordering::SeqCst));
    assert_eq!(budget.usage().0, baseline);
}
#[test]
#[ignore = "manual bounded cold/warm artwork processing measurement"]
fn bounded_artwork_scale_probe() {
    let image = ImageBuffer::from_fn(1024, 768, |x, y| {
        image::Rgb([
            ((x * 17 + y * 3) % 256) as u8,
            ((x * 7 + y * 11) % 256) as u8,
            ((x + y * 5) % 256) as u8,
        ])
    });
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let mut source = art(1, 1);
    source.data = png.into_inner().into();
    let (processor, budget) = setup(Limits::default());
    let options = Options {
        output: Output::Jpeg,
        jpeg_quality: 90,
        ..Options {
            steps: vec![Step::SquareCrop { size: 512 }],
            ..Options::default()
        }
    };
    let start = std::time::Instant::now();
    let first = processor.process(&source, &options).unwrap();
    let cold = start.elapsed().as_secs_f64() * 1000.0;
    let mut warm = Vec::new();
    for _ in 0..7 {
        let start = std::time::Instant::now();
        let result = processor.process(&source, &options).unwrap();
        warm.push(start.elapsed().as_secs_f64() * 1000.0);
        assert!(Arc::ptr_eq(first.image(), result.image()));
        assert_eq!(result.image().dimensions(), (512, 512));
    }
    warm.sort_by(f64::total_cmp);
    println!("1024x768 synthetic PNG {} bytes -> 512x512 JPEG {} bytes; cold {:.3}ms; warm median {:.3}ms max {:.3}ms; shared estimated high-water {:?}; cache {:?}",source.data.len(),first.image().bytes().len(),cold,warm[3],warm[6],budget.usage().1,processor.cache_stats());
}

#[test]
fn conflicting_orientation_declarations_are_refused_instead_of_choosing_a_silent_winner() {
    let (processor, _) = setup(Limits::default());
    let original = ancillary(art(3, 2), b"eXIf", &exif(6));
    let conflicting = ancillary(original, b"eXIf", &exif(3));
    let options = Options {
        metadata: Metadata::Discard,
        steps: vec![Step::NormalizeOrientation],
        ..Options::default()
    };
    assert!(matches!(
        processor.process(&conflicting, &options),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(processor.cache_stats().entries, 0);
}
