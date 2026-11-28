//! Offline duplicate evidence. Scans read files; they never edit or delete them.
//!
//! Exact groups are SHA-256 candidates followed by a byte comparison. Audio
//! groups compare a full decoded PCM signature. Likely groups use local
//! Chromaprint evidence and remain review candidates, never identity claims.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rusty_chromaprint::{match_fingerprints, Configuration, Fingerprinter};
use serde::Serialize;
use sha2::{Digest, Sha256};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::query::{DuplicateQueryGroup, DuplicateResults, QueryError, QueryIndex, QueryRow};

const FINGERPRINT_SECONDS: f64 = 120.0;
const CACHE_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateJobStatus {
    pub job_id: u64,
    pub snapshot_id: u64,
    pub kind: String,
    pub status: String,
    pub processed: usize,
    pub total: usize,
    pub skipped: usize,
    pub errors: usize,
    pub groups_revision: u64,
    pub groups_count: usize,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
impl Stamp {
    fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

#[derive(Debug, Clone)]
struct AudioSignature {
    pcm: Option<[u8; 32]>,
    fingerprint: Arc<[u32]>,
    frames: u64,
    rate: u32,
    complete: bool,
}
#[derive(Debug, Clone)]
struct CachedAnalysis {
    stamp: Stamp,
    bytes: Option<[u8; 32]>,
    audio: Option<AudioSignature>,
}

#[derive(Debug, Default)]
pub struct AnalysisCache {
    entries: HashMap<String, CachedAnalysis>,
}
impl AnalysisCache {
    fn get(&self, path: &str, stamp: &Stamp) -> Option<CachedAnalysis> {
        self.entries
            .get(path)
            .filter(|entry| entry.stamp == *stamp)
            .cloned()
    }
    fn put(&mut self, path: String, entry: CachedAnalysis) {
        if self.entries.len() >= CACHE_ENTRIES && !self.entries.contains_key(&path) {
            self.entries.clear();
        }
        self.entries.insert(path, entry);
    }
}

/// Counts are cheap to publish; a group snapshot is present only when verified
/// membership changed and the first-group/throttled/final publication is due.
#[derive(Debug)]
pub struct ScanProgress {
    pub phase: &'static str,
    pub processed: usize,
    pub total: usize,
    pub skipped: usize,
    pub errors: usize,
    pub groups: Option<Vec<DuplicateQueryGroup>>,
}

struct GroupProgress {
    groups: BTreeMap<usize, DuplicateQueryGroup>,
    dirty: bool,
    published: bool,
    last_publication: Instant,
    last_counts: Option<(&'static str, (usize, usize, usize))>,
}
impl Default for GroupProgress {
    fn default() -> Self {
        Self {
            groups: BTreeMap::new(),
            dirty: false,
            published: false,
            last_publication: Instant::now(),
            last_counts: None,
        }
    }
}
impl GroupProgress {
    fn add(&mut self, index: &QueryIndex, kind: &str, leader: usize, member: usize) {
        if let Some(group) = self.groups.get_mut(&leader) {
            if let Err(position) = group.indices.binary_search(&member) {
                group.indices.insert(position, member);
                self.dirty = true;
            }
        } else {
            let (fallback, reason, confidence) = match kind {
                "exact" => ("Exact copies", "SHA-256 match confirmed by a complete byte comparison.", "exact"),
                "same_audio" => ("Same audio", "Complete decoded PCM samples, sample rate and channel layout match; tags and artwork are excluded.", "audio"),
                _ => ("Likely same recording", "Strong local Chromaprint overlap within the first 120 seconds, similar duration and compatible version labels. Review before choosing a copy.", "likely"),
            };
            self.groups.insert(
                leader,
                make_group(
                    index,
                    kind,
                    vec![leader, member],
                    fallback,
                    reason,
                    confidence,
                ),
            );
            self.dirty = true;
        }
    }
    fn emit(
        &mut self,
        phase: &'static str,
        counts: (usize, usize, usize),
        total: usize,
        force: bool,
        update: &mut impl FnMut(ScanProgress),
    ) {
        let publish = self.dirty
            && (force
                || !self.published
                || self.last_publication.elapsed() >= Duration::from_millis(300));
        if !publish && !force && self.last_counts == Some((phase, counts)) {
            return;
        }
        self.last_counts = Some((phase, counts));
        let groups = publish.then(|| self.groups.values().cloned().collect());
        if publish {
            self.published = true;
            self.dirty = false;
            self.last_publication = Instant::now();
        }
        update(ScanProgress {
            phase,
            processed: counts.0,
            total,
            skipped: counts.1,
            errors: counts.2,
            groups,
        });
    }
}

/// Incrementally form disjoint cliques. A candidate can join only after every
/// current member has a verified edge to it. Counters keep growth linear in
/// verified edges rather than repeatedly walking every group's members.
#[derive(Default)]
struct LikelyGroups {
    assigned: HashMap<usize, usize>,
    compatible: HashMap<(usize, usize), usize>,
}
impl LikelyGroups {
    fn edge(
        &mut self,
        a: usize,
        b: usize,
        adjacency: &HashMap<usize, BTreeSet<usize>>,
        groups: &mut GroupProgress,
        index: &QueryIndex,
    ) {
        let mut added = VecDeque::new();
        let leader = match (
            self.assigned.get(&a).copied(),
            self.assigned.get(&b).copied(),
        ) {
            (None, None) => {
                let leader = a.min(b);
                self.assigned.insert(a, leader);
                self.assigned.insert(b, leader);
                groups.add(index, "likely", leader, a.max(b));
                added.extend([a, b]);
                leader
            }
            (Some(leader), None) | (None, Some(leader)) => {
                let candidate = if self.assigned.contains_key(&a) { b } else { a };
                let count = self.compatible.entry((leader, candidate)).or_default();
                *count += 1;
                if *count != groups.groups[&leader].indices.len() {
                    return;
                }
                self.assigned.insert(candidate, leader);
                groups.add(index, "likely", leader, candidate);
                added.push_back(candidate);
                leader
            }
            // Never merge two established groups through a transitive edge.
            (Some(_), Some(_)) => return,
        };
        while let Some(member) = added.pop_front() {
            if let Some(neighbors) = adjacency.get(&member) {
                for &candidate in neighbors {
                    if self.assigned.contains_key(&candidate) {
                        continue;
                    }
                    let count = self.compatible.entry((leader, candidate)).or_default();
                    *count += 1;
                    if *count == groups.groups[&leader].indices.len() {
                        self.assigned.insert(candidate, leader);
                        groups.add(index, "likely", leader, candidate);
                        added.push_back(candidate);
                    }
                }
            }
        }
    }
}

pub fn scan(
    index: &QueryIndex,
    kind: &str,
    cache: &Mutex<AnalysisCache>,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize, usize),
) -> Result<DuplicateResults, QueryError> {
    scan_with_updates(index, kind, cache, cancelled, |update| {
        progress(update.processed, update.skipped, update.errors)
    })
}

pub fn scan_with_updates(
    index: &QueryIndex,
    kind: &str,
    cache: &Mutex<AnalysisCache>,
    cancelled: impl Fn() -> bool,
    mut update: impl FnMut(ScanProgress),
) -> Result<DuplicateResults, QueryError> {
    if !["exact", "same_audio", "likely"].contains(&kind) {
        return Err(QueryError::Invalid(
            "Unsupported duplicate criterion.".into(),
        ));
    }
    let cancelled = || cancelled() || !index.is_current();
    if cancelled() {
        return Err(QueryError::Cancelled);
    }
    let total = index.rows.len();
    let mut groups = GroupProgress::default();
    let mut skipped = 0;
    let mut errors = 0;
    let mut stamps = Vec::with_capacity(total);
    let mut sizes: HashMap<u64, usize> = HashMap::new();
    groups.emit("stamping", (0, skipped, errors), total, false, &mut update);
    for (i, row) in index.rows.iter().enumerate() {
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        let stamp = if row.value("unreadable") == "true" {
            skipped += 1;
            None
        } else {
            match Stamp::read(Path::new(&row.path)) {
                Ok(stamp) => {
                    *sizes.entry(stamp.length).or_default() += 1;
                    Some(stamp)
                }
                Err(_) => {
                    errors += 1;
                    None
                }
            }
        };
        stamps.push(stamp);
        if i % 256 == 255 {
            groups.emit("stamping", (0, skipped, errors), total, false, &mut update);
        }
    }
    let mut byte_groups: HashMap<[u8; 32], Vec<Vec<usize>>> = HashMap::new();
    let mut audio_groups: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
    let mut audio = HashMap::new();
    let phase = match kind {
        "exact" => "hashing",
        "same_audio" => "decoding",
        _ => "fingerprinting",
    };
    groups.emit(phase, (0, skipped, errors), total, false, &mut update);
    for (i, row) in index.rows.iter().enumerate() {
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        if let Some(stamp) = &stamps[i] {
            let mut entry = cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&row.path, stamp)
                .unwrap_or_else(|| CachedAnalysis {
                    stamp: stamp.clone(),
                    bytes: None,
                    audio: None,
                });
            let analyzed = if kind == "exact" {
                if sizes.get(&stamp.length).copied().unwrap_or(0) < 2 {
                    Ok(())
                } else {
                    entry
                        .bytes
                        .map(Ok)
                        .unwrap_or_else(|| hash_file(Path::new(&row.path), &cancelled))
                        .map(|hash| entry.bytes = Some(hash))
                }
            } else {
                let full = kind == "same_audio";
                entry
                    .audio
                    .take()
                    .filter(|audio| !full || audio.pcm.is_some())
                    .map(Ok)
                    .unwrap_or_else(|| analyze_audio(Path::new(&row.path), full, &cancelled))
                    .map(|signature| entry.audio = Some(signature))
            };
            if matches!(analyzed, Err(QueryError::Cancelled)) {
                return Err(QueryError::Cancelled);
            }
            if analyzed.is_err() {
                errors += 1;
            } else if Stamp::read(Path::new(&row.path)).ok().as_ref() == Some(stamp) {
                cache
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .put(row.path.clone(), entry.clone());
                if kind == "exact" {
                    if let Some(hash) = entry.bytes {
                        let confirmed = byte_groups.entry(hash).or_default();
                        let mut placed = false;
                        for cluster in confirmed.iter_mut() {
                            let leader = cluster[0];
                            let leader_path = Path::new(&index.rows[leader].path);
                            if Stamp::read(leader_path).ok().as_ref() != stamps[leader].as_ref() {
                                errors += 1;
                                continue;
                            }
                            match identical_files(leader_path, Path::new(&row.path), &cancelled) {
                                Ok(true)
                                    if Stamp::read(leader_path).ok().as_ref()
                                        == stamps[leader].as_ref()
                                        && Stamp::read(Path::new(&row.path)).ok().as_ref()
                                            == Some(stamp) =>
                                {
                                    cluster.push(i);
                                    groups.add(index, kind, leader, i);
                                    placed = true;
                                    break;
                                }
                                Ok(_) => {}
                                Err(QueryError::Cancelled) => return Err(QueryError::Cancelled),
                                Err(_) => errors += 1,
                            }
                        }
                        if !placed {
                            confirmed.push(vec![i]);
                        }
                    }
                } else if let Some(signature) = entry.audio {
                    if kind == "same_audio" {
                        if let Some(pcm) = signature.pcm {
                            let cluster = audio_groups.entry(pcm).or_default();
                            if let Some(&leader) = cluster.first() {
                                if Stamp::read(Path::new(&index.rows[leader].path))
                                    .ok()
                                    .as_ref()
                                    == stamps[leader].as_ref()
                                {
                                    groups.add(index, kind, leader, i);
                                } else {
                                    errors += 1;
                                }
                            }
                            cluster.push(i);
                        }
                    } else {
                        audio.insert(i, signature);
                    }
                }
            } else {
                errors += 1;
            }
        }
        groups.emit(phase, (i + 1, skipped, errors), total, false, &mut update);
    }
    if kind == "likely" {
        groups.emit(
            "comparing",
            (total, skipped, errors),
            total,
            false,
            &mut update,
        );
        let mut buckets: HashMap<(u64, u32), Vec<usize>> = HashMap::new();
        let mut metadata: HashMap<String, Vec<usize>> = HashMap::new();
        let mut pairs = BTreeSet::new();
        // Stable row order also makes greedy clique membership deterministic.
        let ordered: BTreeMap<_, _> = audio.iter().map(|(&i, signature)| (i, signature)).collect();
        for (&i, signature) in &ordered {
            if cancelled() {
                return Err(QueryError::Cancelled);
            }
            let unique: HashSet<_> = signature.fingerprint.iter().copied().collect();
            if signature.fingerprint.len() < 64 || unique.len() < 8 {
                skipped += 1;
                continue;
            }
            let row = &index.rows[i];
            let Some(duration) = duration(row, signature) else {
                skipped += 1;
                continue;
            };
            let duration_bucket = (duration / 4.0) as u64;
            let tokens: BTreeSet<u32> = signature
                .fingerprint
                .iter()
                .step_by(8)
                .take(48)
                .map(|v| v >> 16)
                .collect();
            for token in tokens {
                for bucket in duration_bucket.saturating_sub(1)..=duration_bucket + 1 {
                    if let Some(candidates) = buckets.get(&(bucket, token)) {
                        for &other in candidates {
                            pairs.insert((i.min(other), i.max(other)));
                        }
                    }
                }
                buckets.entry((duration_bucket, token)).or_default().push(i);
            }
            let artist = normalized(row.value("artist"));
            let title = normalized(row.value("title"));
            if !artist.is_empty() && !title.is_empty() {
                let key = format!("{artist}\0{title}");
                if let Some(candidates) = metadata.get(&key) {
                    for &other in candidates {
                        pairs.insert((i.min(other), i.max(other)));
                    }
                }
                metadata.entry(key).or_default().push(i);
            }
            if pairs.len() > 1_000_000 {
                return Err(QueryError::Invalid("Duplicate candidates exceed the comparison budget; open a narrower library and rescan.".into()));
            }
        }
        let mut adjacency: HashMap<usize, BTreeSet<usize>> = HashMap::new();
        let mut cliques = LikelyGroups::default();
        for (a, b) in pairs {
            if cancelled() {
                return Err(QueryError::Cancelled);
            }
            if similar(&index.rows[a], &audio[&a], &index.rows[b], &audio[&b]) {
                adjacency.entry(a).or_default().insert(b);
                adjacency.entry(b).or_default().insert(a);
                cliques.edge(a, b, &adjacency, &mut groups, index);
            }
            groups.emit(
                "comparing",
                (total, skipped, errors),
                total,
                false,
                &mut update,
            );
        }
    }
    // Withdraw evidence changed by an external writer during this scan. Native
    // Apply/Undo/library changes additionally invalidate the snapshot token.
    let mut invalid = Vec::new();
    for (&leader, group) in &groups.groups {
        for &i in &group.indices {
            if cancelled() {
                return Err(QueryError::Cancelled);
            }
            if Stamp::read(Path::new(&index.rows[i].path)).ok().as_ref() != stamps[i].as_ref() {
                errors += 1;
                invalid.push(leader);
            }
        }
    }
    for leader in invalid {
        groups.groups.remove(&leader);
        groups.dirty = true;
    }
    if cancelled() {
        return Err(QueryError::Cancelled);
    }
    groups.emit(
        "complete",
        (total, skipped, errors),
        total,
        true,
        &mut update,
    );
    Ok(DuplicateResults {
        status: "complete".into(),
        groups: groups.groups.into_values().collect(),
        skipped,
        errors,
        groups_revision: 0,
    })
}

fn make_group(
    index: &QueryIndex,
    kind: &str,
    mut indices: Vec<usize>,
    fallback: &str,
    reason: &str,
    confidence: &str,
) -> DuplicateQueryGroup {
    indices.sort_unstable();
    let row = &index.rows[indices[0]];
    let label = if !row.value("title").trim().is_empty() {
        row.value("title").trim().to_string()
    } else if !row.value("file").is_empty() {
        row.value("file").to_string()
    } else {
        fallback.into()
    };
    DuplicateQueryGroup {
        id: format!("{}:{kind}:{}", index.id, indices[0]),
        label,
        reason: reason.into(),
        confidence: confidence.into(),
        indices,
    }
}

fn hash_file(path: &Path, cancelled: &impl Fn() -> bool) -> Result<[u8; 32], QueryError> {
    let mut input = File::open(path).map_err(io_error)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        let read = input.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().into())
}

fn identical_files(a: &Path, b: &Path, cancelled: &impl Fn() -> bool) -> Result<bool, QueryError> {
    let before_a = Stamp::read(a).map_err(io_error)?;
    let before_b = Stamp::read(b).map_err(io_error)?;
    if before_a.length != before_b.length {
        return Ok(false);
    }
    let mut a_file = File::open(a).map_err(io_error)?;
    let mut b_file = File::open(b).map_err(io_error)?;
    let mut a_buffer = [0u8; 64 * 1024];
    let mut b_buffer = [0u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        let count = a_file.read(&mut a_buffer).map_err(io_error)?;
        let mut read = 0;
        while read < count {
            let n = b_file.read(&mut b_buffer[read..count]).map_err(io_error)?;
            if n == 0 {
                return Ok(false);
            }
            read += n;
        }
        if a_buffer[..count] != b_buffer[..count] {
            return Ok(false);
        }
        if count == 0 {
            break;
        }
    }
    Ok(Stamp::read(a).ok().as_ref() == Some(&before_a)
        && Stamp::read(b).ok().as_ref() == Some(&before_b))
}
fn io_error(error: impl std::fmt::Display) -> QueryError {
    QueryError::Invalid(format!("Audio analysis failed: {error}"))
}

fn analyze_audio(
    path: &Path,
    full: bool,
    cancelled: &impl Fn() -> bool,
) -> Result<AudioSignature, QueryError> {
    let file = File::open(path).map_err(io_error)?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(io_error)?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| io_error("No audio track"))?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(io_error)?;
    let mut hash = Sha256::new();
    let config = Configuration::preset_test2();
    let mut printer = Fingerprinter::new(&config);
    let mut spec = None;
    let mut frames = 0u64;
    let mut fingerprint_samples = 0u64;
    let mut full_read = true;
    let mut bytes = Vec::new();
    let mut fingerprint_input = Vec::new();
    let mut sample_buffer: Option<SampleBuffer<f64>> = None;
    loop {
        if cancelled() {
            return Err(QueryError::Cancelled);
        }
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(DecodeError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(error) => return Err(io_error(error)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet).map_err(io_error)?;
        let current = *decoded.spec();
        if let Some(previous) = spec {
            if previous != current {
                return Err(io_error("Audio stream changes sample format"));
            }
        } else {
            printer
                .start(current.rate, current.channels.count() as u32)
                .map_err(io_error)?;
            hash.update(current.rate.to_le_bytes());
            hash.update((current.channels.count() as u32).to_le_bytes());
            hash.update(current.channels.bits().to_le_bytes());
            spec = Some(current);
        }
        let channels = current.channels.count();
        let capacity = decoded.capacity();
        let buffer = match &mut sample_buffer {
            Some(buffer) if buffer.capacity() >= capacity.saturating_mul(channels) => buffer,
            slot => {
                // Growing scratch should not temporarily retain both buffers.
                drop(slot.take());
                slot.insert(SampleBuffer::new(capacity as u64, current))
            }
        };
        buffer.copy_interleaved_ref(decoded);
        let samples = buffer.samples();
        frames += samples.len() as u64 / channels as u64;
        if full {
            bytes.clear();
            for &sample in samples {
                let sample = if sample == 0.0 { 0.0 } else { sample };
                bytes.extend_from_slice(&sample.to_bits().to_le_bytes());
            }
            hash.update(&bytes);
        }
        let max_samples = (FINGERPRINT_SECONDS * current.rate as f64 * channels as f64) as u64;
        let take = (max_samples.saturating_sub(fingerprint_samples) as usize).min(samples.len());
        fingerprint_input.clear();
        fingerprint_input.extend(
            samples[..take]
                .iter()
                .map(|sample| (sample * 32768.0).round().clamp(-32768.0, 32767.0) as i16),
        );
        printer.consume(&fingerprint_input);
        fingerprint_samples += take as u64;
        if !full && fingerprint_samples >= max_samples {
            full_read = false;
            break;
        }
    }
    if frames == 0 {
        return Err(io_error("Empty audio stream"));
    }
    printer.finish();
    hash.update(frames.to_le_bytes());
    let rate = spec.ok_or_else(|| io_error("No audio samples"))?.rate;
    Ok(AudioSignature {
        pcm: (full && full_read).then(|| hash.finalize().into()),
        fingerprint: Arc::from(printer.fingerprint()),
        frames,
        rate,
        complete: full_read,
    })
}

fn duration(row: &QueryRow, audio: &AudioSignature) -> Option<f64> {
    row.value("duration")
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0)
        .or_else(|| {
            (audio.complete && audio.frames > 0 && audio.rate > 0)
                .then(|| audio.frames as f64 / audio.rate as f64)
        })
}
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn version_labels(title: &str) -> String {
    let mut groups = Vec::new();
    let mut current = String::new();
    let mut inside = false;
    let mut outside = String::new();
    for c in title.to_lowercase().chars() {
        match c {
            '(' | '[' => {
                inside = true;
                current.clear();
            }
            ')' | ']' => {
                if inside
                    && ["mix", "edit", "live", "version", "remaster"]
                        .iter()
                        .any(|word| current.contains(word))
                {
                    groups.push(normalized(&current));
                }
                inside = false;
            }
            _ => {
                if inside {
                    current.push(c);
                } else {
                    outside.push(c);
                }
            }
        }
    }
    let qualifier = |text: &str| {
        text.split(|c: char| !c.is_alphanumeric()).any(|word| {
            [
                "mix",
                "remix",
                "edit",
                "live",
                "remaster",
                "remastered",
                "version",
                "instrumental",
                "acapella",
            ]
            .contains(&word)
        })
    };
    if qualifier(&outside) {
        let suffix = outside
            .rsplit(['-', '–', '—', ':'])
            .next()
            .unwrap_or(&outside);
        groups.push(normalized(if qualifier(suffix) {
            suffix
        } else {
            &outside
        }));
    }
    groups.join(" ")
}
fn similar(a: &QueryRow, aa: &AudioSignature, b: &QueryRow, bb: &AudioSignature) -> bool {
    let (Some(a_duration), Some(b_duration)) = (duration(a, aa), duration(b, bb)) else {
        return false;
    };
    if (a_duration - b_duration).abs() > 2.0
        || version_labels(a.value("title")) != version_labels(b.value("title"))
    {
        return false;
    }
    let shortest = aa.fingerprint.len().min(bb.fingerprint.len());
    if shortest < 64 {
        return false;
    }
    let Ok(segments) = match_fingerprints(
        &aa.fingerprint,
        &bb.fingerprint,
        &Configuration::preset_test2(),
    ) else {
        return false;
    };
    let covered: usize = segments
        .iter()
        .filter(|segment| segment.score <= 4.0)
        .map(|segment| segment.items_count)
        .sum();
    covered as f64 / shortest as f64 >= 0.85
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackDto;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tagrex-query-duplicates-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("fixture directory");
            Self(path)
        }
        fn wav(&self, name: &str, junk: bool, shift: f64) -> std::path::PathBuf {
            let path = self.0.join(name);
            let rate = 11025u32;
            let samples = rate * 24;
            let data = samples * 2;
            let extra = if junk { 12 } else { 0 };
            let mut file = File::create(&path).expect("wave");
            file.write_all(b"RIFF").expect("header");
            file.write_all(&(36 + data + extra).to_le_bytes())
                .expect("length");
            file.write_all(b"WAVEfmt ").expect("format");
            file.write_all(&16u32.to_le_bytes()).expect("fmt size");
            file.write_all(&1u16.to_le_bytes()).expect("pcm");
            file.write_all(&1u16.to_le_bytes()).expect("mono");
            file.write_all(&rate.to_le_bytes()).expect("rate");
            file.write_all(&(rate * 2).to_le_bytes())
                .expect("byte rate");
            file.write_all(&2u16.to_le_bytes()).expect("block");
            file.write_all(&16u16.to_le_bytes()).expect("bits");
            if junk {
                file.write_all(b"JUNK\x04\x00\x00\x00test")
                    .expect("metadata");
            }
            file.write_all(b"data").expect("data");
            file.write_all(&data.to_le_bytes()).expect("size");
            for i in 0..samples {
                let t = i as f64 / rate as f64;
                let step = (t * 3.0).floor() as u32;
                let hz = 110.0 * 2.0f64.powf(((step * 7) % 24) as f64 / 12.0) + shift;
                let value = ((t * hz * std::f64::consts::TAU).sin() * 9000.0
                    + (t * hz * 1.5 * std::f64::consts::TAU).sin() * 3000.0)
                    as i16;
                file.write_all(&value.to_le_bytes()).expect("sample");
            }
            path
        }
        fn index(&self, paths: &[std::path::PathBuf]) -> QueryIndex {
            let tracks:Vec<TrackDto>=paths.iter().map(|path|serde_json::from_value(serde_json::json!({"path":path.to_string_lossy(),"format":"WAV","duration_secs":24,"tags":{"artist":"Test","title":"Recording"}})).expect("row")).collect();
            QueryIndex::from_tracks(&tracks)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn exact_hash_candidates_are_confirmed_with_bytes() {
        let fixture = Fixture::new();
        let a = fixture.wav("a.wav", false, 0.0);
        let b = fixture.0.join("b.wav");
        std::fs::copy(&a, &b).expect("copy");
        let c = fixture.wav("c.wav", true, 0.0);
        let result = scan(
            &fixture.index(&[a, b, c]),
            "exact",
            &Mutex::default(),
            || false,
            |_, _, _| {},
        )
        .expect("scan");
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].indices, vec![0, 1]);
        assert_eq!(result.groups[0].confidence, "exact");
    }
    #[test]
    fn verified_exact_groups_publish_early_and_grow_with_stable_ids() {
        let fixture = Fixture::new();
        let a = fixture.wav("a.wav", false, 0.0);
        let b = fixture.0.join("b.wav");
        let c = fixture.0.join("c.wav");
        std::fs::copy(&a, &b).unwrap();
        std::fs::copy(&a, &c).unwrap();
        let index = fixture.index(&[a, b, c]);
        let mut publications = Vec::new();
        let result = scan_with_updates(
            &index,
            "exact",
            &Mutex::default(),
            || false,
            |update| {
                if let Some(groups) = update.groups {
                    publications.push((update.processed, groups));
                }
            },
        )
        .unwrap();
        assert_eq!(publications[0].0, 2);
        assert_eq!(publications[0].1[0].indices, vec![0, 1]);
        assert_eq!(publications[0].1[0].label, "Recording");
        assert_eq!(result.groups[0].indices, vec![0, 1, 2]);
        assert!(publications.len() >= 2);
        assert!(publications
            .iter()
            .all(|(_, groups)| groups[0].id == result.groups[0].id));
        assert_eq!(publications.last().unwrap().1[0].indices, vec![0, 1, 2]);
    }

    #[test]
    fn equal_size_and_even_colliding_hash_candidates_never_publish_false_groups() {
        let fixture = Fixture::new();
        let a = fixture.0.join("a.wav");
        let b = fixture.0.join("b.wav");
        std::fs::write(&a, b"aaaa").unwrap();
        std::fs::write(&b, b"bbbb").unwrap();
        let cache = Mutex::new(AnalysisCache::default());
        for path in [&a, &b] {
            cache.lock().unwrap().put(
                path.to_string_lossy().into_owned(),
                CachedAnalysis {
                    stamp: Stamp::read(path).unwrap(),
                    bytes: Some([7; 32]),
                    audio: None,
                },
            );
        }
        let mut published = false;
        let result = scan_with_updates(
            &fixture.index(&[a, b]),
            "exact",
            &cache,
            || false,
            |update| {
                published |= update.groups.is_some_and(|groups| !groups.is_empty());
            },
        )
        .unwrap();
        assert!(!published);
        assert!(result.groups.is_empty());
    }

    #[test]
    fn full_audio_groups_publish_before_the_remaining_files_are_decoded() {
        let fixture = Fixture::new();
        let index = fixture.index(&[
            fixture.wav("a.wav", false, 0.0),
            fixture.wav("b.wav", true, 0.0),
            fixture.wav("c.wav", false, 31.0),
        ]);
        let mut first = None;
        let result = scan_with_updates(
            &index,
            "same_audio",
            &Mutex::default(),
            || false,
            |update| {
                if first.is_none()
                    && update
                        .groups
                        .as_ref()
                        .is_some_and(|groups| !groups.is_empty())
                {
                    first = Some((update.processed, update.phase));
                }
            },
        )
        .unwrap();
        assert_eq!(first, Some((2, "decoding")));
        assert_eq!(result.groups[0].confidence, "audio");
        assert_eq!(result.groups[0].indices, vec![0, 1]);
    }

    #[test]
    fn group_publications_are_throttled_and_final_changes_flush() {
        let fixture = Fixture::new();
        let paths = (0..5)
            .map(|i| fixture.0.join(format!("{i}.wav")))
            .collect::<Vec<_>>();
        let index = fixture.index(&paths);
        let mut groups = GroupProgress::default();
        let mut updates = Vec::new();
        let mut receive = |update: ScanProgress| {
            updates.push(update.groups.map(|groups| groups[0].indices.len()))
        };
        groups.add(&index, "exact", 0, 1);
        groups.emit("hashing", (2, 0, 0), 5, false, &mut receive);
        groups.add(&index, "exact", 0, 2);
        groups.emit("hashing", (3, 0, 0), 5, false, &mut receive);
        groups.last_publication -= Duration::from_millis(301);
        groups.emit("hashing", (3, 0, 0), 5, false, &mut receive);
        groups.add(&index, "exact", 0, 3);
        groups.emit("complete", (4, 0, 0), 5, true, &mut receive);
        assert_eq!(updates, vec![Some(2), None, Some(3), Some(4)]);
    }

    #[test]
    fn group_headers_use_saved_title_then_file_name() {
        let tracks=(0..2).map(|i|serde_json::from_value(serde_json::json!({"path":format!("/virtual/{i}.flac"),"format":"FLAC","tags":{"artist":"Artist","title":if i==0{""}else{"Track title"}}})).unwrap()).collect::<Vec<TrackDto>>();
        let index = QueryIndex::from_tracks(&tracks);
        assert_eq!(
            make_group(&index, "exact", vec![0, 1], "fallback", "bytes", "exact").label,
            "0.flac"
        );
        assert_eq!(
            make_group(&index, "exact", vec![1], "fallback", "bytes", "exact").label,
            "Track title"
        );
    }

    #[test]
    fn likely_partial_groups_require_every_pair_and_keep_their_id() {
        let fixture = Fixture::new();
        let paths = (0..3)
            .map(|i| fixture.0.join(format!("{i}.wav")))
            .collect::<Vec<_>>();
        let index = fixture.index(&paths);
        let mut groups = GroupProgress::default();
        let mut cliques = LikelyGroups::default();
        let mut adjacency: HashMap<usize, BTreeSet<usize>> = HashMap::new();
        for (a, b) in [(0, 1), (1, 2)] {
            adjacency.entry(a).or_default().insert(b);
            adjacency.entry(b).or_default().insert(a);
            cliques.edge(a, b, &adjacency, &mut groups, &index);
        }
        assert_eq!(groups.groups[&0].indices, vec![0, 1]);
        let id = groups.groups[&0].id.clone();
        adjacency.entry(0).or_default().insert(2);
        adjacency.entry(2).or_default().insert(0);
        cliques.edge(0, 2, &adjacency, &mut groups, &index);
        assert_eq!(groups.groups[&0].indices, vec![0, 1, 2]);
        assert_eq!(groups.groups[&0].id, id);
    }

    #[test]
    fn changed_files_withdraw_earlier_evidence_and_cancel_stops_publication() {
        let fixture = Fixture::new();
        let a = fixture.wav("a.wav", false, 0.0);
        let b = fixture.0.join("b.wav");
        let c = fixture.0.join("c.wav");
        std::fs::copy(&a, &b).unwrap();
        std::fs::copy(&a, &c).unwrap();
        let index = fixture.index(&[a.clone(), b, c]);
        let mut first = true;
        let result = scan_with_updates(
            &index,
            "exact",
            &Mutex::default(),
            || false,
            |update| {
                if first && update.groups.is_some() {
                    first = false;
                    std::fs::write(&a, b"changed fixture").unwrap();
                }
            },
        )
        .unwrap();
        assert!(!first);
        assert!(result.groups.is_empty());
        assert!(result.errors > 0);
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let index = fixture.index(&[
            fixture.wav("d.wav", false, 0.0),
            fixture.wav("e.wav", false, 0.0),
            fixture.wav("f.wav", false, 0.0),
        ]);
        let mut publications = 0;
        let result = scan_with_updates(
            &index,
            "exact",
            &Mutex::default(),
            || cancelled.load(Ordering::Relaxed),
            |update| {
                if update.groups.is_some() {
                    publications += 1;
                    cancelled.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(matches!(result, Err(QueryError::Cancelled)));
        assert_eq!(publications, 1);
    }
    #[test]
    fn full_audio_comparison_ignores_container_metadata() {
        let fixture = Fixture::new();
        let a = fixture.wav("a.wav", false, 0.0);
        let b = fixture.wav("b.wav", true, 0.0);
        let c = fixture.wav("c.wav", false, 37.0);
        let result = scan(
            &fixture.index(&[a, b, c]),
            "same_audio",
            &Mutex::default(),
            || false,
            |_, _, _| {},
        )
        .expect("scan");
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].indices, vec![0, 1]);
        assert_eq!(result.groups[0].confidence, "audio");
    }
    #[test]
    fn cancellation_and_stale_snapshots_stop_analysis() {
        let fixture = Fixture::new();
        let index = fixture.index(&[fixture.wav("a.wav", false, 0.0)]);
        assert!(matches!(
            scan(&index, "exact", &Mutex::default(), || true, |_, _, _| {}),
            Err(QueryError::Cancelled)
        ));
        index.invalidate();
        assert!(matches!(
            scan(&index, "exact", &Mutex::default(), || false, |_, _, _| {}),
            Err(QueryError::Cancelled)
        ));
    }
    #[test]
    fn versions_are_not_discarded() {
        assert_ne!(
            version_labels("Title (Original Mix)"),
            version_labels("Title (Radio Edit)")
        );
        assert_ne!(
            version_labels("Title - Radio Edit"),
            version_labels("Title - Original Mix")
        );
        assert_ne!(
            version_labels("Title Live at Wembley"),
            version_labels("Title")
        );
        assert_eq!(
            version_labels("Title - Original Mix"),
            version_labels("Title (Original Mix)")
        );
        let mut tracks:Vec<TrackDto>=vec![serde_json::from_value(serde_json::json!({"path":"a.wav","format":"WAV","duration_secs":24,"tags":{"title":"Title - Radio Edit"}})).unwrap(),serde_json::from_value(serde_json::json!({"path":"b.wav","format":"WAV","duration_secs":24,"tags":{"title":"Title - Original Mix"}})).unwrap()];
        let a = QueryRow::from_track(&tracks.remove(0));
        let b = QueryRow::from_track(&tracks.remove(0));
        let signature = AudioSignature {
            pcm: None,
            fingerprint: (0..128).collect(),
            frames: 24 * 11025,
            rate: 11025,
            complete: true,
        };
        assert!(!similar(&a, &signature, &b, &signature));
    }

    #[test]
    fn incomplete_audio_prefix_is_not_complete_duration_evidence() {
        let track: TrackDto = serde_json::from_value(serde_json::json!({
            "path":"unknown.wav","format":"WAV","tags":{}
        }))
        .unwrap();
        let row = QueryRow::from_track(&track);
        let mut signature = AudioSignature {
            pcm: None,
            fingerprint: (0..128).collect(),
            frames: 120 * 11025,
            rate: 11025,
            complete: false,
        };
        assert_eq!(duration(&row, &signature), None);
        assert!(!similar(&row, &signature, &row, &signature));
        signature.complete = true;
        assert_eq!(duration(&row, &signature), Some(120.0));

        let saved: TrackDto = serde_json::from_value(serde_json::json!({
            "path":"known.wav","format":"WAV","duration_secs":300,"tags":{}
        }))
        .unwrap();
        signature.complete = false;
        assert_eq!(
            duration(&QueryRow::from_track(&saved), &signature),
            Some(300.0)
        );
    }

    #[test]
    #[ignore = "requires ffmpeg only to generate a disposable MP3 fixture"]
    fn local_transcoding_check_finds_same_recording_across_encodings() {
        let fixture = Fixture::new();
        let wav = fixture.wav("original.wav", false, 0.0);
        let mp3 = fixture.0.join("transcoded.mp3");
        let encoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(&wav)
            .args(["-codec:a", "libmp3lame", "-b:a", "128k"])
            .arg(&mp3)
            .status()
            .expect("ffmpeg fixture encoder");
        assert!(encoded.success());
        let different = fixture.wav("different.wav", false, 71.0);
        let result = scan(
            &fixture.index(&[wav, mp3, different]),
            "likely",
            &Mutex::default(),
            || false,
            |_, _, _| {},
        )
        .expect("scan");
        assert_eq!(result.groups.len(), 1, "{result:?}");
        assert_eq!(result.groups[0].indices, vec![0, 1]);
    }
}
