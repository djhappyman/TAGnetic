//! Cover-art resize / recompress before embedding (#41).
//!
//! Fetched artwork is routinely far larger than a tag needs, and every embedded
//! copy inflates every file in the album. [`resize_cover`] shrinks an oversized
//! image to a maximum dimension and re-encodes it as JPEG at a chosen quality,
//! feeding the same embed path so the result stays previewable and undoable.

mod resources;

use crate::model::CoverArt;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, Rgb, RgbImage};
use std::io::Cursor;

/// Downscale `art` so its longer side is at most `max_px`, re-encoding as JPEG
/// at `quality` (1..=100). Aspect ratio is preserved.
///
/// Left untouched (a clone) when: `max_px` is 0 (the feature is off), the image
/// already fits within `max_px`, or the bytes can't be decoded — a cover we
/// can't read should still embed as-is rather than fail the whole operation.
pub fn resize_cover(art: &CoverArt, max_px: u32, quality: u8) -> CoverArt {
    if max_px == 0 || art.data.len() > resources::ENCODE_BYTES {
        return art.clone();
    }
    // The size from the header alone (#622): a cover that fits is never
    // decoded at all.
    let _processing = resources::PROCESS.lock().unwrap_or_else(|p| p.into_inner());
    let Ok(mut reader) = image::ImageReader::new(Cursor::new(&art.data)).with_guessed_format()
    else {
        return art.clone();
    };
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(resources::DECODE_BYTES as u64);
    reader.limits(limits);
    let is_png = reader.format() == Some(image::ImageFormat::Png);
    let Ok((width, height)) = reader.into_dimensions() else {
        return art.clone();
    };
    if width <= max_px && height <= max_px {
        return art.clone();
    }
    let quality = quality.clamp(1, 100);
    let huge = u64::from(width) * u64::from(height) > ROW_SHRINK_PIXELS;
    let full_allowed = resources::admitted(width, height, max_px, false);
    let rows_allowed = is_png && huge && resources::admitted(width, height, max_px, true);
    if !full_allowed && !rows_allowed {
        return art.clone();
    }
    let key = resources::Key::new(&art.data, max_px, quality);
    let cached = || {
        resources::CACHE
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
    };
    let result = |data| CoverArt {
        mime: "image/jpeg".into(),
        data,
        kind: art.kind,
        description: art.description.clone(),
    };
    if let Some(data) = cached() {
        return result(data);
    }
    let shrunk = rows_allowed
        .then(|| shrink_png_rows(&art.data, width, height, max_px.saturating_mul(2)))
        .flatten();
    let image = match shrunk {
        Some(image) => DynamicImage::ImageRgb8(image),
        None if full_allowed => {
            let Ok(mut reader) =
                image::ImageReader::new(Cursor::new(&art.data)).with_guessed_format()
            else {
                return art.clone();
            };
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(resources::WORK_BYTES / 2);
            reader.limits(limits);
            match reader.decode() {
                Ok(image) => image,
                Err(_) => return art.clone(),
            }
        }
        None => return art.clone(),
    };

    // `resize` preserves aspect ratio within the max×max box; Lanczos3 is the
    // quality choice for downscaling photographic art.
    let resized = image.resize(max_px, max_px, FilterType::Lanczos3);
    drop(image);

    // Covers are opaque; encode RGB JPEG (drops any alpha channel) — far smaller
    // than PNG for artwork and universally embeddable.
    let rgb = resized.into_rgb8();
    let mut data = resources::Encoded(Vec::new());
    let encoder = JpegEncoder::new_with_quality(&mut data, quality.clamp(1, 100));
    if rgb.write_with_encoder(encoder).is_err() {
        return art.clone();
    }
    let data = std::sync::Arc::<[u8]>::from(data.0);
    resources::CACHE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key, data.clone());
    result(data)
}

/// Whether `art`, as [`resize_cover`] returned it, is still over `max_px`.
///
/// That happens when the image is too large for the decoder to hold (a JPEG of
/// 16000×16000, an interlaced PNG past its limit), which embeds at full size
/// and so has to be said in the preview (#706). An image whose size cannot be
/// read at all is not claimed to be over anything.
pub fn is_over_max(art: &CoverArt, max_px: u32) -> bool {
    if max_px == 0 {
        return false;
    }
    let _processing = resources::PROCESS.lock().unwrap_or_else(|p| p.into_inner());
    let Ok(mut reader) = image::ImageReader::new(Cursor::new(&art.data)).with_guessed_format()
    else {
        return false;
    };
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(resources::DECODE_BYTES as u64);
    reader.limits(limits);
    reader
        .into_dimensions()
        .is_ok_and(|(width, height)| width > max_px || height > max_px)
}

/// The size past which a PNG cover is shrunk by [`shrink_png_rows`] before the
/// final pass instead of being decoded whole: 16 megapixels, 64 MB as RGBA.
/// Covers below it, which is nearly all of them, come out exactly as before.
const ROW_SHRINK_PIXELS: u64 = 16_000_000;

/// Shrink a PNG of `width`×`height` so its longer side is at most `box_px`,
/// reading it a row at a time and averaging each output pixel over the input
/// pixels it covers (#622).
///
/// A PNG decoded whole costs its full size in memory however small the file:
/// an 11500 px square of a few hundred kilobytes took 600 MB to resize. Read
/// this way, it costs one input row and one output image. The result is left
/// at twice the target so the final Lanczos pass still decides the detail.
///
/// `None` for what this does not handle (an interlaced PNG, whose rows arrive
/// in passes, or a stream it cannot read), so the caller decodes it whole.
fn shrink_png_rows(data: &[u8], width: u32, height: u32, box_px: u32) -> Option<RgbImage> {
    if !resources::admitted(width, height, box_px.div_ceil(2), true) {
        return None;
    }
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(data),
        png::Limits {
            bytes: resources::DECODE_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    if reader.info().interlaced {
        return None;
    }
    let channels = match reader.output_color_type().0 {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let scale = f64::from(box_px) / f64::from(width.max(height));
    let out_w = ((f64::from(width) * scale).round() as u32).clamp(1, width);
    let out_h = ((f64::from(height) * scale).round() as u32).clamp(1, height);
    let mut out = RgbImage::new(out_w, out_h);
    let mut sums = vec![0u64; out_w as usize * 3];
    let mut counts = vec![0u64; out_w as usize];
    let flush = |out: &mut RgbImage, row: u32, sums: &mut [u64], counts: &mut [u64]| {
        for x in 0..out_w as usize {
            let count = counts[x].max(1);
            let pixel = std::array::from_fn(|c| (sums[x * 3 + c] / count) as u8);
            out.put_pixel(x as u32, row, Rgb(pixel));
        }
        sums.fill(0);
        counts.fill(0);
    };
    let mut current = 0u32;
    for y in 0..height {
        let row = reader.next_row().ok()??;
        let target = (u64::from(y) * u64::from(out_h) / u64::from(height)) as u32;
        if target != current {
            flush(&mut out, current, &mut sums, &mut counts);
            current = target;
        }
        for (x, pixel) in row.data().chunks_exact(channels).enumerate() {
            let column = (x as u64 * u64::from(out_w) / u64::from(width)) as usize;
            // Gray is spread over the three channels; alpha is dropped, as
            // the JPEG it becomes has none.
            let rgb = if channels < 3 {
                [pixel[0]; 3]
            } else {
                [pixel[0], pixel[1], pixel[2]]
            };
            for (c, value) in rgb.into_iter().enumerate() {
                sums[column * 3 + c] += u64::from(value);
            }
            counts[column] += 1;
        }
    }
    flush(&mut out, current, &mut sums, &mut counts);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    /// A solid-colour PNG of the given size, as bytes.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let buf = ImageBuffer::from_fn(width, height, |_, _| Rgb([120u8, 40, 200]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(buf)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn dimensions(data: &[u8]) -> (u32, u32) {
        let img = image::load_from_memory(data).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn repeated_content_reuses_encoded_bytes_but_preserves_each_pictures_properties() {
        let art = CoverArt {
            mime: "image/png".into(),
            data: png(640, 320).into(),
            kind: crate::model::CoverKind::Front,
            description: "Front".into(),
        };
        let first = resize_cover(&art, 160, 87);
        let other = CoverArt {
            kind: crate::model::CoverKind::Back,
            description: "Back".into(),
            ..art
        };
        let second = resize_cover(&other, 160, 87);
        assert!(std::sync::Arc::ptr_eq(&first.data, &second.data));
        assert_eq!(second.kind, crate::model::CoverKind::Back);
        assert_eq!(second.description, "Back");
        assert_eq!(first.kind, crate::model::CoverKind::Front);
        assert_eq!(dimensions(&second.data), (160, 80));
        let changed = resize_cover(&other, 100, 87);
        assert_eq!(dimensions(&changed.data), (100, 50));
        assert!(!std::sync::Arc::ptr_eq(&first.data, &changed.data));
    }
    #[test]
    fn downscales_oversized_art_and_reencodes_jpeg() {
        let art = CoverArt {
            mime: "image/png".into(),
            data: png(2000, 1000).into(),
            ..CoverArt::default()
        };
        let out = resize_cover(&art, 500, 85);
        assert_eq!(out.mime, "image/jpeg");
        // Longer side clamped to 500, aspect ratio (2:1) preserved.
        assert_eq!(dimensions(&out.data), (500, 250));
        // A JPEG of a solid colour is far smaller than the source PNG.
        assert!(out.data.len() < art.data.len());
    }

    #[test]
    fn leaves_small_or_disabled_or_undecodable_art_untouched() {
        let small = CoverArt {
            mime: "image/png".into(),
            data: png(300, 300).into(),
            ..CoverArt::default()
        };
        // Already within bounds -> unchanged.
        assert_eq!(resize_cover(&small, 500, 85), small);
        // Disabled (max_px == 0) -> unchanged even when oversized.
        let big = CoverArt {
            mime: "image/png".into(),
            data: png(2000, 2000).into(),
            ..CoverArt::default()
        };
        assert_eq!(resize_cover(&big, 0, 85), big);
        // Undecodable bytes -> unchanged, never an error.
        let junk = CoverArt {
            mime: "image/jpeg".into(),
            data: vec![1, 2, 3, 4].into(),
            ..CoverArt::default()
        };
        assert_eq!(resize_cover(&junk, 500, 85), junk);
        assert!(!is_over_max(&junk, 500));
        assert!(!is_over_max(&small, 500));
        assert!(!is_over_max(&big, 0));
        assert!(is_over_max(&big, 500));
    }

    /// #706: an image the decoder cannot hold is embedded as it is, and
    /// [`is_over_max`] is what tells the preview so. Only the header is real,
    /// which is all this path reads before giving up.
    #[test]
    fn a_cover_too_large_to_decode_stays_over_the_limit() {
        let mut data = Vec::new();
        let mut encoder = ::png::Encoder::new(&mut data, 30_000, 30_000);
        encoder.set_color(::png::ColorType::Rgb);
        encoder.set_depth(::png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_chunk(::png::chunk::IDAT, &[]).unwrap();
        drop(writer);
        let art = CoverArt {
            mime: "image/png".into(),
            data: data.into(),
            ..CoverArt::default()
        };
        let out = resize_cover(&art, 500, 85);
        assert_eq!(out, art);
        assert!(is_over_max(&out, 500));
        assert!(!is_over_max(&out, 30_000));
    }

    /// #622: the row-by-row shrink averages each output pixel over the input
    /// it covers, keeps the aspect ratio, spreads gray over the three channels
    /// and drops alpha.
    #[test]
    fn a_png_shrinks_row_by_row() {
        // Left half red, right half blue, 400×200 down to a 100 px box.
        let halves = ImageBuffer::from_fn(400, 200, |x, _| {
            if x < 200 {
                Rgb([255u8, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        });
        let encode = |image: image::DynamicImage| {
            let mut out = Vec::new();
            image
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        let shrunk = shrink_png_rows(
            &encode(image::DynamicImage::ImageRgb8(halves)),
            400,
            200,
            100,
        )
        .unwrap();
        assert_eq!(shrunk.dimensions(), (100, 50));
        assert_eq!(shrunk.get_pixel(10, 25), &Rgb([255, 0, 0]));
        assert_eq!(shrunk.get_pixel(90, 25), &Rgb([0, 0, 255]));

        // Alternating black and white rows average to mid-gray.
        let stripes = ImageBuffer::from_fn(64, 64, |_, y| {
            image::Luma([if y % 2 == 0 { 0u8 } else { 254 }])
        });
        let shrunk = shrink_png_rows(
            &encode(image::DynamicImage::ImageLuma8(stripes)),
            64,
            64,
            32,
        )
        .unwrap();
        assert_eq!(shrunk.get_pixel(5, 5), &Rgb([127, 127, 127]));

        let translucent = ImageBuffer::from_pixel(50, 50, image::Rgba([10u8, 20, 30, 0]));
        let shrunk = shrink_png_rows(
            &encode(image::DynamicImage::ImageRgba8(translucent)),
            50,
            50,
            10,
        )
        .unwrap();
        assert_eq!(shrunk.get_pixel(3, 3), &Rgb([10, 20, 30]));
    }

    /// #622: a PNG past the pixel threshold takes the row-by-row path and comes
    /// out the size and colour the whole decode gave.
    #[test]
    fn a_huge_png_cover_is_resized() {
        let art = CoverArt {
            mime: "image/png".into(),
            data: png(4100, 4000).into(),
            ..CoverArt::default()
        };
        let out = resize_cover(&art, 500, 95);
        assert_eq!(out.mime, "image/jpeg");
        assert_eq!(dimensions(&out.data), (500, 488));
        let pixel = image::load_from_memory(&out.data).unwrap().to_rgb8();
        let Rgb([r, g, b]) = *pixel.get_pixel(250, 244);
        assert!(r.abs_diff(120) <= 3 && g.abs_diff(40) <= 3 && b.abs_diff(200) <= 3);
    }
}
