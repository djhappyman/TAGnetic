//! Independent pressure tests for the public resource and artwork boundaries.
use std::{io::Cursor, process::Command, sync::Arc};
use tagrex_core::{
    cover::{inspect_cover_size, is_over_max, resize_cover, CoverSizeInspection},
    model::{CoverArt, CoverKind},
    resources::{AdmissionError, Budget, Charge, Limits},
};

fn isolated(case: &str, run: impl FnOnce()) {
    if std::env::var("TAGREX_RESOURCE_REVIEW_CASE").as_deref() == Ok(case) {
        run();
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", case, "--nocapture"])
        .env("TAGREX_RESOURCE_REVIEW_CASE", case)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn oversized_artwork_warning_survives_processing_pressure() {
    isolated(
        "oversized_artwork_warning_survives_processing_pressure",
        || {
            let mut bytes = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                200,
                100,
                image::Rgb([10, 20, 30]),
            ))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
            let art = CoverArt {
                data: bytes.into_inner().into(),
                mime: "image/png".into(),
                kind: CoverKind::Front,
                description: "pressure canary".into(),
            };
            assert!(is_over_max(&art, 100));
            let occupied = Budget::process()
                .try_acquire(Charge {
                    jobs: 8,
                    ..Charge::default()
                })
                .unwrap();
            let untouched = resize_cover(&art, 100, 85);
            assert!(Arc::ptr_eq(&untouched.data, &art.data));
            // The corrected API reports the inability to inspect explicitly.
            // It must not invent dimensions or report proof that the image fits.
            // A companion command-level canary checks the actual plan warning.
            assert_eq!(
                inspect_cover_size(&untouched, 100),
                CoverSizeInspection::Busy
            );
            drop(occupied);
            let resized = resize_cover(&art, 100, 85);
            assert!(!is_over_max(&resized, 100));
            assert_eq!(
                inspect_cover_size(&resized, 100),
                CoverSizeInspection::WithinLimit {
                    width: 100,
                    height: 50
                }
            );
            assert_eq!(inspect_cover_size(&art, 0), CoverSizeInspection::Disabled);
            let malformed = CoverArt {
                data: Arc::from(&b"not an image"[..]),
                ..art
            };
            assert_eq!(
                inspect_cover_size(&malformed, 100),
                CoverSizeInspection::Unknown
            );
        },
    );
}

#[test]
fn arbitrary_resize_sequences_match_a_separate_accounting_model() {
    let limits = Limits {
        retained: 4096,
        transient: 8192,
        total: 10_000,
        jobs: 13,
    };
    let budget = Budget::new(limits);
    let mut leases = Vec::new();
    let mut charges: Vec<Charge> = Vec::new();
    let mut random = 0x5ec9b4191d7521efu64;
    for step in 0..50_000 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let request = Charge {
            retained: (random as usize >> 4) % 5000,
            transient: (random as usize >> 17) % 9000,
            jobs: (random as usize >> 31) % 15,
        };
        let index = random as usize % 31;
        let used = charges
            .iter()
            .fold(Charge::default(), |sum, charge| Charge {
                retained: sum.retained + charge.retained,
                transient: sum.transient + charge.transient,
                jobs: sum.jobs + charge.jobs,
            });
        let old = charges.get(index).copied().unwrap_or_default();
        let next = Charge {
            retained: used.retained - old.retained + request.retained,
            transient: used.transient - old.transient + request.transient,
            jobs: used.jobs - old.jobs + request.jobs,
        };
        let fits = |charge: Charge| {
            charge.retained <= limits.retained
                && charge.transient <= limits.transient
                && charge.retained + charge.transient <= limits.total
                && charge.jobs <= limits.jobs
        };
        let expected = if !fits(request) {
            Err(AdmissionError::TooLarge)
        } else if !fits(next) {
            Err(AdmissionError::Busy)
        } else {
            Ok(())
        };
        let actual = if let Some(lease) = leases.get_mut(index) {
            tagrex_core::resources::Lease::try_resize(lease, request)
        } else {
            match budget.try_acquire(request) {
                Ok(lease) => {
                    leases.push(lease);
                    charges.push(Charge::default());
                    Ok(())
                }
                Err(error) => Err(error),
            }
        };
        assert_eq!(actual, expected, "step {step}");
        if actual.is_ok() {
            let selected = index.min(charges.len() - 1);
            charges[selected] = request;
        }
        let expected_used = charges
            .iter()
            .fold(Charge::default(), |sum, charge| Charge {
                retained: sum.retained + charge.retained,
                transient: sum.transient + charge.transient,
                jobs: sum.jobs + charge.jobs,
            });
        assert_eq!(budget.usage().0, expected_used, "step {step}");
        if step % 17 == 0 && !leases.is_empty() {
            leases.swap_remove(0);
            charges.swap_remove(0);
        }
    }
    drop(leases);
    assert_eq!(budget.usage().0, Charge::default());
}
