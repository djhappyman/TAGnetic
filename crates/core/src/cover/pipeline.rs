//! Strict, bounded pixel processing independent of tag writing and staging.
//!
//! The legacy best-effort resize API is unchanged. This API supports static PNG
//! and 8-bit JPEG, preserves PNG 16-bit samples, and reports refusal explicitly.
//! Re-encoding never claims ICC/ancillary metadata preservation or colour-managed
//! filtering. Results and their clones retain the resource leases they own.
use crate::model::{CoverArt, CoverKind};
use crate::resources::{AdmissionError, Budget, Charge, Lease};
use image::{
    codecs::jpeg::{JpegDecoder, JpegEncoder},
    codecs::png::PngEncoder,
    imageops::FilterType,
    metadata::Orientation,
    DynamicImage, GenericImageView, ImageBuffer, ImageDecoder, Rgb, RgbImage,
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::{self, Cursor, Write},
    sync::{Arc, Mutex},
};

/// Processing refuses unsupported semantics, invalid data, excessive work,
/// currently unavailable resources and cancellation as distinct cases.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("Unsupported artwork operation: {0}")]
    Unsupported(&'static str),
    #[error("Invalid artwork request or data: {0}")]
    Invalid(&'static str),
    #[error("Artwork resource limit exceeded: {0}")]
    OverBudget(&'static str),
    #[error("Artwork processing resources are busy; retry later")]
    Busy,
    #[error("Artwork processing was cancelled")]
    Cancelled,
}
impl From<AdmissionError> for Error {
    fn from(error: AdmissionError) -> Self {
        match error {
            AdmissionError::Busy => Self::Busy,
            AdmissionError::TooLarge => Self::OverBudget("shared admission"),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Original,
    Jpeg,
    Png,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upscale {
    Deny,
    Allow,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Triangle,
    Lanczos3,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metadata {
    RejectLoss,
    Discard,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Preserve,
    Allow8Bit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrientationPolicy {
    RequireNormalization,
    Ignore,
}
/// Steps execute in supplied order. Centre crop discards an odd extra column
/// or row on the right/bottom. Fit uses floor rounding, never exceeds its box,
/// and clamps its scale to one when upscaling is denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Fit { width: u32, height: u32 },
    Resize { width: u32, height: u32 },
    SquareCrop { size: u32 },
    SquareScale { size: u32 },
    NormalizeOrientation,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub steps: Vec<Step>,
    pub output: Output,
    pub jpeg_quality: u8,
    pub upscale: Upscale,
    pub filter: Filter,
    pub metadata: Metadata,
    pub bit_depth: BitDepth,
    pub orientation: OrientationPolicy,
    /// Encoded channel-space compositing, with integer rounding. No ICC/linear
    /// light compositing is claimed. An explicit background avoids black alpha.
    pub jpeg_background: [u8; 3],
}
impl Default for Options {
    fn default() -> Self {
        Self {
            steps: Vec::new(),
            output: Output::Original,
            jpeg_quality: 85,
            upscale: Upscale::Deny,
            filter: Filter::Lanczos3,
            metadata: Metadata::RejectLoss,
            bit_depth: BitDepth::Preserve,
            orientation: OrientationPolicy::RequireNormalization,
            jpeg_background: [255; 3],
        }
    }
}
/// Public processing boundary notifications. A notification precedes its work;
/// callers may request cancellation there without publishing a partial result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Inspect,
    Decode,
    Transform(usize),
    Encode,
    Publish,
}
/// Per-request ceilings, bounded cache storage and a decoder scratch allowance.
/// Shared Budget limits still govern all participating concurrent owners.
#[derive(Debug, Clone)]
pub struct Limits {
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub dimension: u32,
    pub pixels: u64,
    pub work_bytes: usize,
    pub decoder_scratch: usize,
    pub steps: usize,
    pub description_bytes: usize,
    pub cache_entries: usize,
    pub cache_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            input_bytes: 16 << 20,
            output_bytes: 16 << 20,
            dimension: 8192,
            pixels: 16_777_216,
            work_bytes: 256 << 20,
            decoder_scratch: 32 << 20,
            steps: 32,
            description_bytes: 1 << 20,
            cache_entries: 32,
            cache_bytes: 64 << 20,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Notes {
    pub reencoded: bool,
    pub metadata_discarded: bool,
    pub orientation_applied: bool,
    pub reduced_to_8_bit: bool,
    pub alpha_composited: bool,
}
/// Immutable encoded pixels with their retained lease. There is intentionally
/// no API that extracts a bare Arc<byte> and outlives the accounting owner.
#[derive(Debug)]
pub struct EncodedImage {
    data: Arc<[u8]>,
    codec: Codec,
    width: u32,
    height: u32,
    bits: u8,
    notes: Notes,
    _lease: Lease,
}
impl EncodedImage {
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }
    pub fn mime(&self) -> &'static str {
        self.codec.mime()
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn bits_per_channel(&self) -> u8 {
        self.bits
    }
    pub fn notes(&self) -> Notes {
        self.notes
    }
}
#[derive(Debug)]
struct Properties {
    kind: CoverKind,
    description: Box<str>,
    _lease: Lease,
}
#[derive(Debug, Clone)]
pub struct ProcessedArtwork {
    image: Arc<EncodedImage>,
    properties: Arc<Properties>,
}
impl ProcessedArtwork {
    pub fn image(&self) -> &Arc<EncodedImage> {
        &self.image
    }
    pub fn kind(&self) -> CoverKind {
        self.properties.kind
    }
    pub fn description(&self) -> &str {
        &self.properties.description
    }
}
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    pub entries: usize,
    pub encoded_bytes: usize,
    pub hits: u64,
    pub misses: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key([u8; 32]);
struct Entry {
    key: Key,
    image: Arc<EncodedImage>,
}
struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
    hits: u64,
    misses: u64,
}
struct Inner {
    budget: Budget,
    limits: Limits,
    cache: Mutex<Cache>,
    _cache_lease: Lease,
}
#[derive(Clone)]
pub struct Processor(Arc<Inner>);
impl Processor {
    pub fn new(budget: Budget, limits: Limits) -> Result<Self, Error> {
        if limits.dimension == 0
            || limits.pixels == 0
            || limits.input_bytes == 0
            || limits.output_bytes == 0
            || limits.work_bytes == 0
            || limits.decoder_scratch == 0
            || limits.steps > 256
            || limits.cache_entries > 4096
            || limits.input_bytes > 64 << 20
            || limits.output_bytes > 64 << 20
            || limits.dimension > 32768
            || limits.pixels > 67_108_864
            || limits.work_bytes > 512 << 20
            || limits.decoder_scratch > 128 << 20
            || limits.description_bytes > 1 << 20
            || limits.cache_bytes > 256 << 20
        {
            return Err(Error::Invalid("invalid processing ceilings"));
        }
        let capacity = if limits.cache_entries == 0 {
            0
        } else {
            limits.cache_entries.max(4).next_power_of_two()
        };
        let cache_bytes = capacity
            .checked_mul(std::mem::size_of::<Entry>() + 64)
            .ok_or(Error::OverBudget("cache metadata"))?;
        let lease = budget.try_acquire(Charge {
            retained: add(cache_bytes, 256)?,
            ..Charge::default()
        })?;
        let mut entries = VecDeque::new();
        entries
            .try_reserve_exact(limits.cache_entries)
            .map_err(|_| Error::OverBudget("cache allocation"))?;
        Ok(Self(Arc::new(Inner {
            budget,
            limits,
            cache: Mutex::new(Cache {
                entries,
                bytes: 0,
                hits: 0,
                misses: 0,
            }),
            _cache_lease: lease,
        })))
    }
    pub fn process(&self, art: &CoverArt, options: &Options) -> Result<ProcessedArtwork, Error> {
        self.process_with_cancel(art, options, || false)
    }
    pub fn cache_stats(&self) -> CacheStats {
        let cache = self.0.cache.lock().unwrap_or_else(|p| p.into_inner());
        CacheStats {
            entries: cache.entries.len(),
            encoded_bytes: cache.bytes,
            hits: cache.hits,
            misses: cache.misses,
        }
    }
    /// Release cache-owned encoded results. Client-held immutable results keep
    /// their own shared owner and lease until their final strong reference drops.
    pub fn clear_cache(&self) {
        let mut cache = self.0.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.entries.clear();
        cache.bytes = 0;
    }
    /// Cancellation is polled during input scanning/hashing, between codec and
    /// pixel operations, during alpha conversion, and on encoder writes. A codec
    /// call already running cannot be forcibly interrupted. No partial result is
    /// returned or inserted into the cache on refusal/cancellation.
    pub fn process_with_cancel(
        &self,
        art: &CoverArt,
        options: &Options,
        cancel: impl Fn() -> bool,
    ) -> Result<ProcessedArtwork, Error> {
        self.process_with_progress(art, options, |_| {}, cancel)
    }
    pub fn process_with_progress(
        &self,
        art: &CoverArt,
        options: &Options,
        progress: impl Fn(Stage),
        cancel: impl Fn() -> bool,
    ) -> Result<ProcessedArtwork, Error> {
        check(&cancel)?;
        let limits = &self.0.limits;
        if art.data.len() > limits.input_bytes {
            return Err(Error::OverBudget("input bytes"));
        }
        if art.description.len() > limits.description_bytes {
            return Err(Error::OverBudget("description bytes"));
        }
        if options.steps.len() > limits.steps {
            return Err(Error::OverBudget("ordered steps"));
        }
        if !(1..=100).contains(&options.jpeg_quality) {
            return Err(Error::Invalid("JPEG quality must be 1..100"));
        }
        let baseline = add(mul(art.data.len(), 3)?, 4096)?;
        let mut work = self.0.budget.try_acquire(Charge {
            transient: baseline,
            ..Charge::default()
        })?;
        let key = key(&art.data, options, &cancel)?;
        if let Some(image) = self.cached(key) {
            progress(Stage::Publish);
            return self.result(image, art, &cancel);
        }
        work.try_resize(Charge {
            transient: add(baseline, limits.decoder_scratch)?,
            jobs: 1,
            ..Charge::default()
        })?;
        progress(Stage::Inspect);
        check(&cancel)?;
        let header = inspect(&art.data, &cancel)?;
        dimensions(header.width, header.height, limits)?;
        if header.animation {
            return Err(Error::Unsupported("animated PNG is not a static picture"));
        }
        let plan = plan(header, options, limits)?;
        if !plan.changed
            && options.output == Output::Original
            && art.data.len() > limits.output_bytes
        {
            return Err(Error::OverBudget("original output bytes"));
        }
        if plan.changed || options.output != Output::Original {
            if header.metadata && options.metadata == Metadata::RejectLoss {
                return Err(Error::Unsupported(
                "re-encoding would discard ancillary metadata/profile; choose Discard explicitly",
            ));
            }
            if header.orientation != Orientation::NoTransforms
                && !plan.oriented
                && options.orientation == OrientationPolicy::RequireNormalization
            {
                return Err(Error::Unsupported(
                    "normalize orientation in an ordered step or explicitly ignore it",
                ));
            }
        }
        let needed = add(
            add(baseline, limits.decoder_scratch)?,
            add(plan.peak, mul(limits.output_bytes, 2)?)?,
        )?;
        if needed > limits.work_bytes {
            return Err(Error::OverBudget("decoded/scratch/encoded working set"));
        }
        work.try_resize(Charge {
            transient: needed,
            jobs: 1,
            ..Charge::default()
        })?;
        check(&cancel)?;
        progress(Stage::Decode);
        check(&cancel)?;
        let mut image = decode(&art.data, header, limits)?;
        check(&cancel)?;
        if image.dimensions() != (header.width, header.height) {
            return Err(Error::Invalid("decoder/header dimensions disagree"));
        }
        if !plan.changed && options.output == Output::Original {
            drop(image);
            let lease = self.0.budget.try_acquire(Charge {
                retained: add(art.data.len(), 256)?,
                ..Charge::default()
            })?;
            let encoded = Arc::new(EncodedImage {
                data: art.data.clone(),
                codec: header.codec,
                width: header.width,
                height: header.height,
                bits: header.bits,
                notes: Notes::default(),
                _lease: lease,
            });
            return self.publish(key, encoded, art, &progress, &cancel);
        }
        for (index, step) in options.steps.iter().enumerate() {
            progress(Stage::Transform(index));
            check(&cancel)?;
            image = execute(image, *step, options, header.orientation)?;
        }
        check(&cancel)?;
        let mut notes = Notes {
            reencoded: true,
            metadata_discarded: header.metadata,
            orientation_applied: plan.oriented && header.orientation != Orientation::NoTransforms,
            ..Notes::default()
        };
        progress(Stage::Encode);
        check(&cancel)?;
        let mut writer = EncodedWriter {
            data: Vec::new(),
            max: limits.output_bytes,
            cancel: &cancel,
            failure: None,
        };
        let bits;
        let encoded = match plan.codec {
            Codec::Png => {
                bits = if image.color().bits_per_pixel() / u16::from(image.color().channel_count())
                    > 8
                {
                    16
                } else {
                    8
                };
                image.write_with_encoder(PngEncoder::new(&mut writer))
            }
            Codec::Jpeg => {
                if header.bits > 8 && options.bit_depth == BitDepth::Preserve {
                    return Err(Error::Unsupported("JPEG cannot preserve 16-bit samples"));
                }
                notes.reduced_to_8_bit = header.bits > 8;
                notes.alpha_composited = image.color().has_alpha();
                let rgba = image.into_rgba8();
                let mut rgb = RgbImage::new(rgba.width(), rgba.height());
                for (y, row) in rgb.rows_mut().enumerate() {
                    if y % 32 == 0 {
                        check(&cancel)?;
                    }
                    for (x, pixel) in row.enumerate() {
                        let source = rgba.get_pixel(x as u32, y as u32).0;
                        let alpha = u32::from(source[3]);
                        *pixel = Rgb(std::array::from_fn(|c| {
                            ((u32::from(source[c]) * alpha
                                + u32::from(options.jpeg_background[c]) * (255 - alpha)
                                + 127)
                                / 255) as u8
                        }));
                    }
                }
                bits = 8;
                rgb.write_with_encoder(JpegEncoder::new_with_quality(
                    &mut writer,
                    options.jpeg_quality,
                ))
            }
        };
        if let Some(error) = writer.failure {
            return Err(error);
        }
        encoded.map_err(|_| Error::Invalid("image encoding failed"))?;
        check(&cancel)?;
        let lease = self.0.budget.try_acquire(Charge {
            retained: add(writer.data.len(), 256)?,
            ..Charge::default()
        })?;
        let data: Arc<[u8]> = writer.data.into();
        let image = Arc::new(EncodedImage {
            data,
            codec: plan.codec,
            width: plan.width,
            height: plan.height,
            bits,
            notes,
            _lease: lease,
        });
        self.publish(key, image, art, &progress, &cancel)
    }
    fn cached(&self, key: Key) -> Option<Arc<EncodedImage>> {
        let mut cache = self.0.cache.lock().unwrap_or_else(|p| p.into_inner());
        let index = cache.entries.iter().position(|entry| entry.key == key);
        if let Some(index) = index {
            let entry = cache.entries.remove(index)?;
            let image = entry.image.clone();
            cache.entries.push_back(entry);
            cache.hits = cache.hits.saturating_add(1);
            Some(image)
        } else {
            cache.misses = cache.misses.saturating_add(1);
            None
        }
    }
    fn result(
        &self,
        image: Arc<EncodedImage>,
        art: &CoverArt,
        cancel: &impl Fn() -> bool,
    ) -> Result<ProcessedArtwork, Error> {
        check(cancel)?;
        let lease = self.0.budget.try_acquire(Charge {
            retained: add(art.description.len(), 256)?,
            ..Charge::default()
        })?;
        let properties = Arc::new(Properties {
            kind: art.kind,
            description: art.description.clone().into_boxed_str(),
            _lease: lease,
        });
        Ok(ProcessedArtwork { image, properties })
    }
    fn publish(
        &self,
        key: Key,
        image: Arc<EncodedImage>,
        art: &CoverArt,
        progress: &impl Fn(Stage),
        cancel: &impl Fn() -> bool,
    ) -> Result<ProcessedArtwork, Error> {
        progress(Stage::Publish);
        check(cancel)?;
        // Admit all result ownership before evicting useful cache entries.
        let mut result = self.result(image, art, cancel)?;
        check(cancel)?;
        let mut cache = self.0.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = cache.entries.iter().find(|entry| entry.key == key) {
            result.image = entry.image.clone();
            return Ok(result);
        }
        let bytes = result.image.bytes().len();
        if self.0.limits.cache_entries > 0 && bytes <= self.0.limits.cache_bytes {
            while cache.entries.len() >= self.0.limits.cache_entries
                || cache.bytes > self.0.limits.cache_bytes - bytes
            {
                if let Some(old) = cache.entries.pop_front() {
                    cache.bytes -= old.image.bytes().len();
                }
            }
            cache.bytes += bytes;
            cache.entries.push_back(Entry {
                key,
                image: result.image.clone(),
            });
        }
        Ok(result)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Codec {
    Jpeg,
    Png,
}
impl Codec {
    fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }
}
#[derive(Clone, Copy)]
struct Header {
    codec: Codec,
    width: u32,
    height: u32,
    bits: u8,
    metadata: bool,
    animation: bool,
    orientation: Orientation,
}
struct Plan {
    width: u32,
    height: u32,
    codec: Codec,
    peak: usize,
    changed: bool,
    oriented: bool,
}
fn check(cancel: &impl Fn() -> bool) -> Result<(), Error> {
    if cancel() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn add(a: usize, b: usize) -> Result<usize, Error> {
    a.checked_add(b).ok_or(Error::OverBudget("byte arithmetic"))
}
fn mul(a: usize, b: usize) -> Result<usize, Error> {
    a.checked_mul(b).ok_or(Error::OverBudget("byte arithmetic"))
}
fn pixels(w: u32, h: u32) -> Result<usize, Error> {
    usize::try_from(u64::from(w) * u64::from(h)).map_err(|_| Error::OverBudget("pixel arithmetic"))
}
fn dimensions(w: u32, h: u32, limits: &Limits) -> Result<(), Error> {
    if w == 0 || h == 0 {
        return Err(Error::Invalid("zero image dimension"));
    }
    if w > limits.dimension || h > limits.dimension || u64::from(w) * u64::from(h) > limits.pixels {
        return Err(Error::OverBudget("dimensions/pixels"));
    }
    Ok(())
}
fn swapped(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}
fn target(
    w: u32,
    h: u32,
    step: Step,
    options: &Options,
    orientation: Orientation,
) -> Result<(u32, u32), Error> {
    let (tw, th) = match step {
        Step::Fit { width, height } => {
            if width == 0 || height == 0 {
                return Err(Error::Invalid("zero fit box"));
            }
            let (n, d) = if u64::from(width) * u64::from(h) <= u64::from(height) * u64::from(w) {
                (width, w)
            } else {
                (height, h)
            };
            if options.upscale == Upscale::Deny && n >= d {
                (w, h)
            } else {
                (
                    (u64::from(w) * u64::from(n) / u64::from(d)).max(1) as u32,
                    (u64::from(h) * u64::from(n) / u64::from(d)).max(1) as u32,
                )
            }
        }
        Step::Resize { width, height } => (width, height),
        Step::SquareCrop { size } | Step::SquareScale { size } => (size, size),
        Step::NormalizeOrientation => {
            if swapped(orientation) {
                (h, w)
            } else {
                (w, h)
            }
        }
    };
    if tw == 0 || th == 0 {
        return Err(Error::Invalid("zero target dimension"));
    }
    if options.upscale == Upscale::Deny {
        let (source_w, source_h) = if matches!(step, Step::SquareCrop { .. }) {
            (w.min(h), w.min(h))
        } else {
            (w, h)
        };
        if !matches!(step, Step::NormalizeOrientation) && (tw > source_w || th > source_h) {
            return Err(Error::Invalid("upscaling requires Allow"));
        }
    }
    Ok((tw, th))
}
fn plan(header: Header, options: &Options, limits: &Limits) -> Result<Plan, Error> {
    let (mut w, mut h) = (header.width, header.height);
    let mut peak = mul(pixels(w, h)?, 16)?;
    let mut changed = false;
    let mut oriented = false;
    for step in &options.steps {
        if matches!(step, Step::NormalizeOrientation) {
            if oriented {
                return Err(Error::Invalid("orientation normalization may appear once"));
            }
            oriented = true;
        }
        let (tw, th) = target(w, h, *step, options, header.orientation)?;
        dimensions(tw, th, limits)?;
        let source = mul(pixels(w, h)?, 8)?;
        let output = mul(pixels(tw, th)?, 8)?;
        let scratch = if (tw, th) != (w, h) && !matches!(step, Step::NormalizeOrientation) {
            mul(pixels(w, th)?.max(pixels(tw, h)?), 16)?
        } else {
            0
        };
        peak = peak.max(add(
            add(source, output)?,
            add(scratch, mul(limits.dimension as usize, 32)?)?,
        )?);
        changed |= (tw, th) != (w, h)
            || matches!(step, Step::SquareCrop { .. }) && w != h
            || matches!(step, Step::NormalizeOrientation)
                && header.orientation != Orientation::NoTransforms;
        (w, h) = (tw, th);
    }
    let codec = match options.output {
        Output::Original => header.codec,
        Output::Jpeg => Codec::Jpeg,
        Output::Png => Codec::Png,
    };
    if codec == Codec::Jpeg && header.bits > 8 && options.bit_depth == BitDepth::Preserve {
        return Err(Error::Unsupported("JPEG cannot preserve 16-bit samples"));
    }
    Ok(Plan {
        width: w,
        height: h,
        codec,
        peak,
        changed,
        oriented,
    })
}
fn execute(
    mut image: DynamicImage,
    step: Step,
    options: &Options,
    orientation: Orientation,
) -> Result<DynamicImage, Error> {
    let (w, h) = image.dimensions();
    let (tw, th) = target(w, h, step, options, orientation)?;
    if matches!(step, Step::NormalizeOrientation) {
        image.apply_orientation(orientation);
        return Ok(image);
    }
    if matches!(step, Step::SquareCrop { .. }) && w != h {
        let size = w.min(h);
        image = image.crop_imm((w - size) / 2, (h - size) / 2, size, size);
    }
    if image.dimensions() != (tw, th) {
        let filter = match options.filter {
            Filter::Nearest => FilterType::Nearest,
            Filter::Triangle => FilterType::Triangle,
            Filter::Lanczos3 => FilterType::Lanczos3,
        };
        image = image.resize_exact(tw, th, filter);
    }
    Ok(image)
}
fn key(data: &[u8], options: &Options, cancel: &impl Fn() -> bool) -> Result<Key, Error> {
    let mut hash = Sha256::new();
    hash.update(b"tagrex-pixels-v1/image-0.25.10/png-0.18.1/raw-channel-filtering");
    hash.update((data.len() as u64).to_le_bytes());
    for chunk in data.chunks(65_536) {
        check(cancel)?;
        hash.update(chunk);
    }
    hash.update([
        options.output as u8,
        options.jpeg_quality,
        options.upscale as u8,
        options.filter as u8,
        options.metadata as u8,
        options.bit_depth as u8,
        options.orientation as u8,
    ]);
    hash.update(options.jpeg_background);
    hash.update((options.steps.len() as u64).to_le_bytes());
    for step in &options.steps {
        check(cancel)?;
        let (kind, w, h) = match step {
            Step::Fit { width, height } => (0, *width, *height),
            Step::Resize { width, height } => (1, *width, *height),
            Step::SquareCrop { size } => (2, *size, 0),
            Step::SquareScale { size } => (3, *size, 0),
            Step::NormalizeOrientation => (4, 0, 0),
        };
        hash.update([kind]);
        hash.update(w.to_le_bytes());
        hash.update(h.to_le_bytes());
    }
    Ok(Key(hash.finalize().into()))
}
fn inspect(data: &[u8], cancel: &impl Fn() -> bool) -> Result<Header, Error> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png_header(data, cancel);
    }
    if data.starts_with(&[0xff, 0xd8]) {
        return jpeg_header(data, cancel);
    }
    Err(Error::Unsupported(
        "only static PNG and JPEG pixels are supported",
    ))
}
fn png_header(data: &[u8], cancel: &impl Fn() -> bool) -> Result<Header, Error> {
    let mut at = 8usize;
    let mut result = None;
    let mut idat = false;
    let mut orientation_seen = false;
    while at < data.len() {
        check(cancel)?;
        let prefix = data
            .get(
                at..at
                    .checked_add(8)
                    .ok_or(Error::Invalid("PNG length overflow"))?,
            )
            .ok_or(Error::Invalid("truncated PNG chunk"))?;
        let len = u32::from_be_bytes(
            prefix[..4]
                .try_into()
                .map_err(|_| Error::Invalid("PNG length"))?,
        ) as usize;
        let end = at
            .checked_add(12)
            .and_then(|v| v.checked_add(len))
            .ok_or(Error::Invalid("PNG length overflow"))?;
        if end > data.len() {
            return Err(Error::Invalid("truncated PNG chunk data"));
        }
        let tag = &prefix[4..8];
        let payload = &data[at + 8..end - 4];
        if result.is_none() {
            if tag != b"IHDR" || len != 13 {
                return Err(Error::Invalid("PNG must begin with IHDR"));
            }
            result = Some(Header {
                codec: Codec::Png,
                width: u32::from_be_bytes(
                    payload[..4]
                        .try_into()
                        .map_err(|_| Error::Invalid("PNG width"))?,
                ),
                height: u32::from_be_bytes(
                    payload[4..8]
                        .try_into()
                        .map_err(|_| Error::Invalid("PNG height"))?,
                ),
                bits: payload[8],
                metadata: false,
                animation: false,
                orientation: Orientation::NoTransforms,
            });
        } else if tag == b"IHDR" {
            return Err(Error::Invalid("duplicate PNG IHDR"));
        }
        let header = result
            .as_mut()
            .ok_or(Error::Invalid("missing PNG header"))?;
        match tag {
            b"IDAT" => idat = true,
            b"IEND" => {
                if len != 0 || !idat || end != data.len() {
                    return Err(Error::Invalid("invalid PNG end"));
                }
                return Ok(*header);
            }
            b"acTL" | b"fcTL" | b"fdAT" => header.animation = true,
            b"eXIf" => {
                header.metadata = true;
                if let Some(orientation) = exif_orientation(payload, cancel)? {
                    if orientation_seen && header.orientation != orientation {
                        return Err(Error::Unsupported("conflicting EXIF orientations"));
                    }
                    header.orientation = orientation;
                    orientation_seen = true;
                }
            }
            b"IHDR" | b"PLTE" | b"tRNS" => (),
            _ => header.metadata = true,
        }
        at = end;
    }
    Err(Error::Invalid("PNG has no IEND"))
}
// Parse every IFD0 orientation entry instead of accepting the first one.
// Contradictory duplicates would otherwise be baked into pixels and erased on
// metadata discard. No offsets for unrelated metadata are dereferenced.
fn exif_orientation(data: &[u8], cancel: &impl Fn() -> bool) -> Result<Option<Orientation>, Error> {
    let little = match data.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(Error::Invalid("invalid EXIF byte order")),
    };
    let word = |at: usize| -> Result<u16, Error> {
        let bytes: [u8; 2] = data
            .get(at..at.saturating_add(2))
            .ok_or(Error::Invalid("truncated EXIF word"))?
            .try_into()
            .unwrap();
        Ok(if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let dword = |at: usize| -> Result<u32, Error> {
        let bytes: [u8; 4] = data
            .get(at..at.saturating_add(4))
            .ok_or(Error::Invalid("truncated EXIF offset"))?
            .try_into()
            .unwrap();
        Ok(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    if word(2)? != 42 {
        return Err(Error::Unsupported("unsupported EXIF TIFF revision"));
    }
    let offset = dword(4)? as usize;
    if offset < 8 {
        return Err(Error::Invalid("invalid EXIF first directory offset"));
    }
    let count = usize::from(word(offset)?);
    let start = offset
        .checked_add(2)
        .ok_or(Error::Invalid("invalid EXIF directory"))?;
    let end = start
        .checked_add(
            count
                .checked_mul(12)
                .ok_or(Error::Invalid("invalid EXIF directory"))?,
        )
        .and_then(|end| end.checked_add(4))
        .filter(|end| *end <= data.len())
        .ok_or(Error::Invalid("truncated EXIF directory"))?;
    let mut found = None;
    for (index, at) in (start..end - 4).step_by(12).enumerate() {
        if index.is_multiple_of(128) {
            check(cancel)?;
        }
        if word(at)? != 0x112 {
            continue;
        }
        if word(at + 2)? != 3 || dword(at + 4)? != 1 {
            return Err(Error::Unsupported(
                "unsupported EXIF orientation representation",
            ));
        }
        let value = u8::try_from(word(at + 8)?)
            .ok()
            .and_then(Orientation::from_exif)
            .ok_or(Error::Invalid("invalid EXIF orientation value"))?;
        if found.is_some_and(|previous| previous != value) {
            return Err(Error::Unsupported("conflicting EXIF orientations"));
        }
        found = Some(value);
    }
    Ok(found)
}
fn jpeg_header(data: &[u8], cancel: &impl Fn() -> bool) -> Result<Header, Error> {
    let mut at = 2usize;
    let mut entropy = false;
    let mut next_check = 0usize;
    let mut header: Option<Header> = None;
    let mut metadata = false;
    let mut orientation = Orientation::NoTransforms;
    let mut orientation_seen = false;
    while at < data.len() {
        if at >= next_check {
            check(cancel)?;
            next_check = at.saturating_add(4096);
        }
        if data[at] != 0xff {
            if entropy {
                at += 1;
                continue;
            }
            return Err(Error::Invalid("JPEG marker expected"));
        }
        while data.get(at) == Some(&0xff) {
            if at >= next_check {
                check(cancel)?;
                next_check = at.saturating_add(4096);
            }
            at += 1;
        }
        let marker = *data
            .get(at)
            .ok_or(Error::Invalid("truncated JPEG marker"))?;
        at += 1;
        if marker == 0 && entropy || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if marker == 0xd9 {
            if at != data.len() {
                return Err(Error::Invalid("trailing JPEG data"));
            }
            let mut header = header.ok_or(Error::Invalid("JPEG has no frame"))?;
            header.metadata = metadata;
            header.orientation = orientation;
            return Ok(header);
        }
        if marker == 0xd8 {
            return Err(Error::Invalid("nested JPEG SOI"));
        }
        if marker == 1 {
            continue;
        }
        let size = data
            .get(at..at.saturating_add(2))
            .ok_or(Error::Invalid("truncated JPEG length"))?;
        let len = usize::from(u16::from_be_bytes([size[0], size[1]]));
        let end = at
            .checked_add(len)
            .filter(|end| len >= 2 && *end <= data.len())
            .ok_or(Error::Invalid("invalid JPEG segment"))?;
        let payload = &data[at + 2..end];
        if matches!(marker, 0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            return Err(Error::Unsupported("unsupported JPEG coding process"));
        }
        if matches!(marker, 0xc0..=0xc2) {
            if payload.len() < 6 {
                return Err(Error::Invalid("short JPEG frame"));
            }
            if payload[0] != 8 {
                return Err(Error::Unsupported("only 8-bit JPEG samples are supported"));
            }
            let parsed = Header {
                codec: Codec::Jpeg,
                width: u32::from(u16::from_be_bytes([payload[3], payload[4]])),
                height: u32::from(u16::from_be_bytes([payload[1], payload[2]])),
                bits: 8,
                metadata: false,
                animation: false,
                orientation: Orientation::NoTransforms,
            };
            if header.is_some() {
                return Err(Error::Unsupported("multiple JPEG frames are not supported"));
            }
            header = Some(parsed);
        }
        if (0xe0..=0xef).contains(&marker) || marker == 0xfe {
            metadata = true;
        }
        if marker == 0xe1 && payload.starts_with(b"Exif\0\0") {
            if let Some(parsed) = exif_orientation(&payload[6..], cancel)? {
                if orientation_seen && orientation != parsed {
                    return Err(Error::Unsupported("conflicting EXIF orientations"));
                }
                orientation = parsed;
                orientation_seen = true;
            }
        }
        entropy |= marker == 0xda;
        at = end;
    }
    Err(Error::Invalid("JPEG has no EOI"))
}
fn png_error(error: png::DecodingError) -> Error {
    if matches!(error, png::DecodingError::LimitsExceeded) {
        Error::OverBudget("PNG decoder scratch")
    } else {
        Error::Invalid("PNG decode failed")
    }
}
fn decode(data: &[u8], header: Header, limits: &Limits) -> Result<DynamicImage, Error> {
    match header.codec {
        Codec::Jpeg => {
            let mut decoder = JpegDecoder::new(Cursor::new(data))
                .map_err(|_| Error::Invalid("JPEG header decode failed"))?;
            let mut bound = image::Limits::default();
            bound.max_alloc = Some(add(
                mul(pixels(header.width, header.height)?, 8)?,
                limits.decoder_scratch,
            )? as u64);
            bound.max_image_width = Some(limits.dimension);
            bound.max_image_height = Some(limits.dimension);
            decoder
                .set_limits(bound)
                .map_err(|_| Error::OverBudget("JPEG decoder limits"))?;
            DynamicImage::from_decoder(decoder)
                .map_err(|_| Error::Invalid("JPEG pixel decode failed"))
        }
        Codec::Png => {
            let mut decoder = png::Decoder::new_with_limits(
                Cursor::new(data),
                png::Limits {
                    bytes: limits.decoder_scratch,
                },
            );
            decoder.set_ignore_text_chunk(true);
            decoder.set_ignore_iccp_chunk(true);
            decoder.set_transformations(png::Transformations::EXPAND);
            let mut reader = decoder.read_info().map_err(png_error)?;
            let size = reader
                .output_buffer_size()
                .ok_or(Error::OverBudget("PNG decoded size"))?;
            if size > mul(pixels(header.width, header.height)?, 8)? {
                return Err(Error::OverBudget("PNG decoded channels/depth"));
            }
            let mut data = Vec::new();
            data.try_reserve_exact(size)
                .map_err(|_| Error::OverBudget("PNG pixel allocation"))?;
            data.resize(size, 0);
            let info = reader.next_frame(&mut data).map_err(png_error)?;
            reader.finish().map_err(png_error)?;
            drop(reader);
            data.truncate(info.buffer_size());
            macro_rules! image {
                ($pixel:ty, $variant:ident, $samples:expr) => {
                    DynamicImage::$variant(
                        ImageBuffer::<$pixel, _>::from_raw(header.width, header.height, $samples)
                            .ok_or(Error::Invalid("PNG pixel layout"))?,
                    )
                };
            }
            if info.bit_depth == png::BitDepth::Sixteen {
                let mut samples = Vec::new();
                samples
                    .try_reserve_exact(data.len() / 2)
                    .map_err(|_| Error::OverBudget("16-bit pixel allocation"))?;
                samples.extend(
                    data.as_chunks::<2>()
                        .0
                        .iter()
                        .map(|v| u16::from_be_bytes([v[0], v[1]])),
                );
                Ok(match info.color_type {
                    png::ColorType::Grayscale => image!(image::Luma<u16>, ImageLuma16, samples),
                    png::ColorType::GrayscaleAlpha => {
                        image!(image::LumaA<u16>, ImageLumaA16, samples)
                    }
                    png::ColorType::Rgb => image!(image::Rgb<u16>, ImageRgb16, samples),
                    png::ColorType::Rgba => image!(image::Rgba<u16>, ImageRgba16, samples),
                    _ => return Err(Error::Unsupported("unexpanded indexed PNG")),
                })
            } else {
                Ok(match info.color_type {
                    png::ColorType::Grayscale => image!(image::Luma<u8>, ImageLuma8, data),
                    png::ColorType::GrayscaleAlpha => image!(image::LumaA<u8>, ImageLumaA8, data),
                    png::ColorType::Rgb => image!(image::Rgb<u8>, ImageRgb8, data),
                    png::ColorType::Rgba => image!(image::Rgba<u8>, ImageRgba8, data),
                    _ => return Err(Error::Unsupported("unexpanded indexed PNG")),
                })
            }
        }
    }
}
struct EncodedWriter<'a, F> {
    data: Vec<u8>,
    max: usize,
    cancel: &'a F,
    failure: Option<Error>,
}
impl<F: Fn() -> bool> Write for EncodedWriter<'_, F> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let failure = if (self.cancel)() {
            Some(Error::Cancelled)
        } else if bytes.len() > self.max.saturating_sub(self.data.len()) {
            Some(Error::OverBudget("encoded output bytes"))
        } else {
            None
        };
        if let Some(error) = failure {
            self.failure = Some(error);
            return Err(io::Error::other("bounded artwork output refused"));
        }
        let required = self.data.len() + bytes.len();
        if required > self.data.capacity() {
            let target = required
                .max(self.data.capacity().saturating_mul(2).max(4096))
                .min(self.max);
            self.data
                .try_reserve_exact(target - self.data.len())
                .map_err(|_| {
                    self.failure = Some(Error::OverBudget("encoded allocation"));
                    io::Error::other("encoded allocation refused")
                })?;
        }
        self.data.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
