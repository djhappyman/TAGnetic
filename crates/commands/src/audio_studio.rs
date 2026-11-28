//! Native studio drafts, review, output ownership and work scheduling.
use crate::output::{
    write_new_cue, AudioDestination, AudioPublishedFolder, AudioReplacement, AudioSet,
    CompletedReplacement,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tagrex_audio_workbench::markers::{
    self, CueExportTrack, CueTags, ProviderMarker, ProviderSpan, ProviderTrack, ProviderTrackKind,
};
pub use tagrex_audio_workbench::markers::{CueEncoding, ProviderAnchor};
use tagrex_audio_workbench::{
    self as audio, Analysis, CopyCodec, FrameCopyIndex, OutputFormat, PreparedTreatment,
    RenderSegment,
};
pub use tagrex_audio_workbench::{
    AudioTreatment, EqBand, FrameRange, PreviewComparison, PreviewMonitor, SpectralRepair,
    SpectrogramChannel, SpectrogramDetail, SpectrogramFocus, SpectrumView, TreatmentProbe,
    TreatmentProbeOptions,
};
use tagrex_core::mask::{portable_file_name, Mask};
use tagrex_core::model::{
    is_writable_value_at, normalize_staged_tag_value, AudioFormat, CoverArt, CoverKind,
    ModelPolicy, TagEngine, TagField, TagMap, TrackFile,
};

type Values = BTreeMap<String, String>;
type Result<T> = std::result::Result<T, StudioError>;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct StudioError(String);

impl StudioError {
    pub fn new(message: impl std::fmt::Display) -> Self {
        Self(message.to_string())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| StudioError::new("Audio studio state is unavailable"))
}

const MAX_SESSIONS: usize = 4;
const MAX_TREATMENT_RECIPES: usize = 8;
const MAX_TRACKS: usize = 200;
const MAX_TAG_VALUE: usize = 32 * 1024;
const OVERVIEW_BINS: usize = 1024;
const CUE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: SystemTime,
    #[cfg(unix)]
    change: (i64, i64),
    #[cfg(windows)]
    change: i64,
}

impl Stamp {
    fn of(file: &File) -> Result<Self> {
        let metadata = file.metadata().map_err(StudioError::new)?;
        if !metadata.is_file() {
            return Err(StudioError::new("Choose a regular audio file"));
        }
        Ok(Self {
            bytes: metadata.len(),
            modified: metadata.modified().map_err(StudioError::new)?,
            #[cfg(unix)]
            change: {
                use std::os::unix::fs::MetadataExt;
                (metadata.ctime(), metadata.ctime_nsec())
            },
            #[cfg(windows)]
            change: crate::output::windows_change_time(file).map_err(StudioError::new)?,
        })
    }
}

struct Source {
    path: PathBuf,
    identity: Mutex<Option<same_file::Handle>>,
    stamp: Stamp,
    info: audio::SourceInfo,
    analysis: Option<Analysis>,
    overview_level: (f64, f64),
    format: AudioFormat,
    frame_index: OnceLock<Arc<FrameCopyIndex>>,
    treatments: Mutex<VecDeque<PreparedTreatment>>,
    tags: TagMap,
    tag_policy: u64,
    replaceable: bool,
}

impl Source {
    fn load(path: &Path, bins: usize, cancelled: impl FnMut() -> bool) -> Result<Self> {
        Self::load_with_progress(path, bins, cancelled, None::<fn(audio::AnalysisProgress)>)
    }

    fn load_with_progress(
        path: &Path,
        bins: usize,
        cancelled: impl FnMut() -> bool,
        progress: Option<impl FnMut(audio::AnalysisProgress)>,
    ) -> Result<Self> {
        let replaceable = path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_file());
        let canonical = path.canonicalize().map_err(StudioError::new)?;
        let file = open_regular(&canonical)?;
        let stamp = Stamp::of(&file)?;
        let identity = same_file::Handle::from_file(file.try_clone().map_err(StudioError::new)?)
            .map_err(StudioError::new)?;
        let analysis = match progress {
            Some(progress) => audio::inspect_file_with_progress(file, bins, cancelled, progress),
            None => audio::inspect_file(file, bins, cancelled),
        }
        .map_err(StudioError::new)?;
        let tagged = TagEngine::read(&canonical).map_err(StudioError::new)?;
        let mean_square = analysis
            .rms
            .iter()
            .map(|&sample| f64::from(sample).powi(2))
            .sum::<f64>()
            / analysis.rms.len().max(1) as f64;
        let peak = analysis
            .peaks
            .iter()
            .map(|bin| f64::from(bin.max.abs().max(bin.min.abs())))
            .fold(0.0, f64::max);
        let source = Self {
            path: canonical,
            identity: Mutex::new(Some(identity)),
            stamp,
            info: analysis.source,
            analysis: (bins > 1).then_some(analysis),
            overview_level: (mean_square, peak),
            format: tagged.format,
            frame_index: OnceLock::new(),
            treatments: Mutex::new(VecDeque::new()),
            tags: tagged.tags,
            tag_policy: tagrex_core::model::read_policy_revision(),
            replaceable,
        };
        source.open_current()?;
        Ok(source)
    }

    fn open_current(&self) -> Result<File> {
        let bound = lock(&self.identity)?;
        let file = open_regular(&self.path)?;
        let identity = same_file::Handle::from_file(file.try_clone().map_err(StudioError::new)?)
            .map_err(StudioError::new)?;
        if bound.as_ref() != Some(&identity) || Stamp::of(&file)? != self.stamp {
            return Err(StudioError::new(
                "The recording changed. Reopen it before continuing.",
            ));
        }
        Ok(file)
    }

    fn frame_index(&self, codec: CopyCodec) -> Result<Arc<FrameCopyIndex>> {
        let mut file = self.open_current()?;
        if let Some(index) = self.frame_index.get() {
            return Ok(index.clone());
        }
        let index = FrameCopyIndex::scan(&mut file, codec, self.info.frames, || false).map_err(
            |error| {
                StudioError::new(format!(
                    "{error}. Choose exact cuts to re-encode this recording."
                ))
            },
        )?;
        self.open_current()?;
        Ok(self.frame_index.get_or_init(|| Arc::new(index)).clone())
    }

    fn cached_treatment(&self, settings: &AudioTreatment) -> Result<Option<PreparedTreatment>> {
        let mut treatments = lock(&self.treatments)?;
        if let Some(index) = treatments
            .iter()
            .position(|item| item.settings() == settings)
        {
            let prepared = treatments.remove(index).expect("cached treatment at index");
            treatments.push_back(prepared.clone());
            return Ok(Some(prepared));
        }
        Ok(None)
    }

    fn preview_treatment(&self, settings: AudioTreatment) -> Result<PreparedTreatment> {
        // Whole-recording measurements serve export and explicit normalization.
        // Merely reviewing an export must not change the level of an EQ preview.
        let measured = if settings.normalize {
            self.cached_treatment(&settings)?
        } else {
            None
        };
        match measured {
            Some(prepared) => Ok(prepared),
            None => PreparedTreatment::preview(settings, self.info)
                .map(|prepared| {
                    prepared.with_preview_level(self.overview_level.0, self.overview_level.1)
                })
                .map_err(StudioError::new),
        }
    }

    fn prepare_treatment(
        &self,
        settings: &AudioTreatment,
        cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedTreatment> {
        let file = self.open_current()?;
        if let Some(prepared) = self.cached_treatment(settings)? {
            return Ok(prepared);
        }
        if !settings.needs_measurement() {
            return PreparedTreatment::preview(settings.clone(), self.info)
                .map_err(StudioError::new);
        }
        let prepared = audio::prepare_treatment(file, settings.clone(), cancelled)
            .map_err(StudioError::new)?;
        self.open_current()?;
        // Preview can build a provisional recipe even after eviction. Reviewed
        // exports own their measured recipe independently of this cache.
        let mut treatments = lock(&self.treatments)?;
        if let Some(existing) = treatments.iter().find(|item| item.settings() == settings) {
            return Ok(existing.clone());
        }
        if treatments.len() >= MAX_TREATMENT_RECIPES {
            treatments.pop_front();
        }
        treatments.push_back(prepared.clone());
        Ok(prepared)
    }

    fn stamp_key(&self) -> Result<String> {
        use std::hash::{Hash, Hasher};
        let mut identity = std::collections::hash_map::DefaultHasher::new();
        lock(&self.identity)?
            .as_ref()
            .ok_or_else(|| StudioError::new("The recording was replaced"))?
            .hash(&mut identity);
        let description = format!("{}:{:?}", identity.finish(), self.stamp);
        Ok(format!("{:x}", Sha256::digest(description.as_bytes())))
    }
}

fn open_regular(path: &Path) -> Result<File> {
    let metadata = path.symlink_metadata().map_err(StudioError::new)?;
    if !metadata.is_file() {
        return Err(StudioError::new("Choose a regular file"));
    }
    #[cfg(unix)]
    let file: File = {
        use rustix::fs::{open, Mode, OFlags};
        open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(StudioError::new)?
        .into()
    };
    #[cfg(not(unix))]
    let file = File::open(path).map_err(StudioError::new)?;
    if !file.metadata().map_err(StudioError::new)?.is_file() {
        return Err(StudioError::new("Choose a regular file"));
    }
    Ok(file)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedRecording {
    id: String,
    path: String,
    frames: u64,
    sample_rate: u32,
    original_format: Option<&'static str>,
    source_stamp: String,
    overview: Vec<f32>,
    overview_rms: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stereo_overview: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stereo_overview_rms: Option<Vec<f32>>,
    tags: Values,
}

impl OpenedRecording {
    /// Identifies the session so a caller can release an unclaimed recording.
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingProgress {
    frames: u64,
    sample_rate: u32,
    scanned_frames: u64,
    overview: Vec<f32>,
    overview_rms: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stereo_overview: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stereo_overview_rms: Option<Vec<f32>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreatmentAnalysis {
    gain_db: f64,
    loudness_lufs: Option<f64>,
    output_channels: u16,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftSegment {
    pub start_frame: u64,
    pub include: bool,
    #[serde(default)]
    pub tags: Values,
    pub pregap_frame: Option<u64>,
    pub index00: Option<u64>,
    pub index01: Option<u64>,
    pub pregap: Option<u64>,
    pub postgap: Option<u64>,
    #[serde(default)]
    pub lead_in: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    sources: Vec<String>,
    session_id: Option<String>,
    #[serde(default)]
    segments: Vec<DraftSegment>,
    #[serde(default)]
    tags: Values,
    #[serde(default)]
    destination: String,
    #[serde(default)]
    basename: String,
    folder_pattern: Option<String>,
    format: String,
    bitrate: Option<u16>,
    split_method: Option<SplitMethod>,
    #[serde(default)]
    treatment: AudioTreatment,
    name_pattern: Option<String>,
    artwork_path: Option<String>,
    #[serde(default)]
    save_mode: SaveMode,
    backup_directory: Option<String>,
}

impl ReviewRequest {
    pub fn needs_default_recovery(&self) -> bool {
        self.save_mode == SaveMode::Replace && self.backup_directory.is_none()
    }
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SaveMode {
    Beside,
    Replace,
    #[default]
    Folder,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum SplitMethod {
    Copy,
    Exact,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputRow {
    path: String,
    start_frame: u64,
    end_frame: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_start_frame: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_end_frame: Option<u64>,
    sample_rate: u32,
    tags: Values,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    id: String,
    outputs: Vec<OutputRow>,
    warnings: Vec<String>,
    split_method: SplitMethod,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    backups: Vec<BackupRow>,
}

#[derive(Clone, Debug, Serialize)]
struct BackupRow {
    source: String,
    path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExportResult {
    pub paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replaced: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    backups: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RestoreResult {
    pub paths: Vec<String>,
    pub removed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    phase: &'static str,
    completed: usize,
    total: usize,
    frames_done: u64,
    frames_total: u64,
}

struct ExportState {
    progress: Mutex<Progress>,
    cancelled: AtomicBool,
    result: Mutex<Option<ExportResult>>,
    replacement: Mutex<Option<CompletedReplacement>>,
    published_folders: Mutex<Vec<AudioPublishedFolder>>,
}

struct PlannedFile {
    source: Arc<Source>,
    treatment: PreparedTreatment,
    range: FrameRange,
    requested: FrameRange,
    name: String,
    tags: TagMap,
    destination: usize,
}

struct PlannedDestination {
    root: PathBuf,
    guard: AudioDestination,
    folder: Option<String>,
    bytes_required: u64,
}

impl PlannedDestination {
    fn new(root: PathBuf, folder: Option<String>) -> Result<Self> {
        let guard = AudioDestination::open(&root).map_err(StudioError::new)?;
        if let Some(folder) = &folder {
            require_unused(&root.join(folder))?;
        }
        Ok(Self {
            root,
            guard,
            folder,
            bytes_required: 0,
        })
    }

    fn output_path(&self, name: &str) -> PathBuf {
        let root = self
            .folder
            .as_ref()
            .map_or_else(|| self.root.clone(), |folder| self.root.join(folder));
        root.join(name)
    }

    fn begin(&self) -> Result<AudioSet> {
        match &self.folder {
            Some(folder) => AudioSet::begin(&self.guard, folder),
            None => AudioSet::begin_flat(&self.guard),
        }
        .map_err(StudioError::new)
    }
}

fn require_unused(path: &Path) -> Result<()> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(StudioError::new(error)),
        Ok(_) => Err(StudioError::new(format!(
            "{} already exists",
            path.display()
        ))),
    }
}

struct Plan {
    session_id: Option<String>,
    destinations: Vec<PlannedDestination>,
    format: OutputFormat,
    frame_index: Option<Arc<FrameCopyIndex>>,
    files: Vec<PlannedFile>,
    artwork: Option<CoverArt>,
    replacement: Option<AudioReplacement>,
    tag_policy: u64,
}

#[derive(Default)]
struct StudioState {
    sources: BTreeMap<String, Arc<Source>>,
    pending: BTreeMap<String, Plan>,
    jobs: BTreeMap<String, Arc<ExportState>>,
    releases: BTreeMap<String, CachedRelease>,
    recent_sources: VecDeque<Arc<Source>>,
}

struct CachedRelease {
    edition: String,
    fetched: Instant,
    release: DiscogsRelease,
}

#[derive(Clone)]
pub struct DiscogsRelease {
    tracks: Vec<ProviderTrack>,
    tags: Values,
}

#[derive(Serialize)]
pub struct DiscogsProposal {
    rows: Vec<ProviderMarker>,
    spans: Vec<ProviderSpan>,
    warnings: Vec<String>,
    tags: Values,
}

#[derive(Default)]
pub struct Studio {
    next: AtomicU64,
    state: Mutex<StudioState>,
    policy: ModelPolicy,
}

impl From<tagrex_providers_discogs::TimingRelease> for DiscogsRelease {
    fn from(release: tagrex_providers_discogs::TimingRelease) -> Self {
        let tags = [
            ("album", release.title),
            ("albumartist", release.artist),
            (
                "year",
                release
                    .year
                    .map(|year| year.to_string())
                    .unwrap_or_default(),
            ),
            ("genre", release.genre),
        ]
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(field, value)| (field.to_owned(), value))
        .collect();
        Self {
            tracks: discogs_tracks(release.tracks),
            tags,
        }
    }
}

fn discogs_tracks(rows: Vec<tagrex_providers_discogs::TimingTrack>) -> Vec<ProviderTrack> {
    rows.into_iter()
        .map(|row| ProviderTrack {
            kind: match row.kind.as_str() {
                "track" => ProviderTrackKind::Track,
                "index" => ProviderTrackKind::Index,
                "heading" => ProviderTrackKind::Heading,
                _ => ProviderTrackKind::Unrecognized,
            },
            position: row.position,
            title: row.title,
            performer: row.performer,
            duration: row.duration,
            children: discogs_tracks(row.children),
        })
        .collect()
}

fn public_tags(tags: &TagMap) -> Values {
    tags.iter()
        .map(|(key, value)| (key.to_storage_key(), value.clone()))
        .collect()
}

fn merge_tags(tags: &mut TagMap, values: &Values) -> Result<()> {
    if values.len() > 64 {
        return Err(StudioError::new("Too many tag fields"));
    }
    for (key, value) in values {
        let field = TagField::from_name(key)
            .ok_or_else(|| StudioError::new(format!("Invalid field {key}")))?;
        if value.len() > MAX_TAG_VALUE || value.contains('\0') {
            return Err(StudioError::new(format!("Invalid value for {key}")));
        }
        if value.is_empty() {
            tags.remove(&field);
        } else {
            tags.insert(field, value.clone());
        }
    }
    Ok(())
}

fn flat_peaks(peaks: impl IntoIterator<Item = audio::Peak>) -> Vec<f32> {
    peaks
        .into_iter()
        .flat_map(|peak| [peak.min, peak.max])
        .collect()
}

fn flat_stereo(peaks: impl IntoIterator<Item = [audio::Peak; 2]>) -> Vec<f32> {
    flat_peaks(peaks.into_iter().flatten())
}

impl Studio {
    pub fn apply_settings(&self, settings: &crate::SettingsDto) {
        self.policy.configure(
            settings.id3_v23,
            &settings.read_priority,
            &settings.multi_value_separator,
            &settings.legacy_code_page,
        );
    }

    fn id(&self) -> String {
        self.next.fetch_add(1, Ordering::Relaxed).to_string()
    }

    fn source(&self, id: &str) -> Result<Arc<Source>> {
        lock(&self.state)?
            .sources
            .get(id)
            .cloned()
            .ok_or_else(|| StudioError::new("This studio session has closed"))
    }

    pub fn open(&self, path: &Path, cancelled: impl FnMut() -> bool) -> Result<OpenedRecording> {
        self.open_with_progress(path, cancelled, |_| {})
    }

    pub fn open_with_progress(
        &self,
        path: &Path,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(RecordingProgress),
    ) -> Result<OpenedRecording> {
        let _policy = self.policy.enter();
        let canonical = path.canonicalize().map_err(StudioError::new)?;
        let replaceable = path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_file());
        let cached = {
            let mut state = lock(&self.state)?;
            if state.sources.len() >= MAX_SESSIONS {
                return Err(StudioError::new(
                    "Close an audio studio session before opening another",
                ));
            }
            state
                .recent_sources
                .iter()
                .position(|source| source.path == canonical && source.replaceable == replaceable)
                .and_then(|index| state.recent_sources.remove(index))
        };
        let source = match cached {
            Some(source)
                if source.tag_policy == self.policy.revision() && source.open_current().is_ok() =>
            {
                source
            }
            _ => Arc::new(Source::load_with_progress(
                path,
                OVERVIEW_BINS,
                &mut cancelled,
                Some(|part: audio::AnalysisProgress| {
                    progress(RecordingProgress {
                        frames: part.source.source.frames,
                        sample_rate: part.source.source.sample_rate,
                        scanned_frames: part.scanned_frames,
                        overview: flat_peaks(part.peaks),
                        overview_rms: part.rms,
                        stereo_overview: part.stereo_peaks.map(flat_stereo),
                        stereo_overview_rms: part
                            .stereo_rms
                            .map(|values| values.into_iter().flatten().collect()),
                    });
                }),
            )?),
        };
        let mut state = lock(&self.state)?;
        if cancelled() {
            return Err(StudioError::new("Opening the recording was stopped"));
        }
        if state.sources.len() >= MAX_SESSIONS {
            return Err(StudioError::new(
                "Close an audio studio session before opening another",
            ));
        }
        let id = self.id();
        let opened = OpenedRecording {
            id: id.clone(),
            path: source.path.to_string_lossy().into_owned(),
            frames: source.info.frames,
            sample_rate: source.info.sample_rate,
            original_format: match source.format {
                AudioFormat::Mp3 => Some("mp3"),
                AudioFormat::Flac => Some("flac"),
                _ => None,
            },
            source_stamp: source.stamp_key()?,
            overview: flat_peaks(
                source
                    .analysis
                    .iter()
                    .flat_map(|item| item.peaks.iter().copied()),
            ),
            overview_rms: source
                .analysis
                .as_ref()
                .map(|analysis| analysis.rms.clone())
                .unwrap_or_default(),
            stereo_overview_rms: source
                .analysis
                .as_ref()
                .and_then(|analysis| analysis.stereo_rms.as_ref())
                .map(|values| values.iter().flatten().copied().collect()),
            stereo_overview: source
                .analysis
                .as_ref()
                .and_then(|analysis| analysis.stereo_peaks.as_deref())
                .map(|peaks| flat_stereo(peaks.iter().copied())),
            tags: public_tags(&source.tags),
        };
        state.sources.insert(id, source);
        Ok(opened)
    }

    pub fn close(&self, id: &str) -> Result<()> {
        self.close_with(id, || Ok(()))
    }

    pub fn close_with(
        &self,
        id: &str,
        release_transport: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let mut state = lock(&self.state)?;
        if let Some(source) = state.sources.remove(id) {
            state.recent_sources.retain(|item| item.path != source.path);
            state.recent_sources.push_back(source);
            if state.recent_sources.len() > 2 {
                state.recent_sources.pop_front();
            }
        }
        state.releases.remove(id);
        state
            .pending
            .retain(|_, plan| plan.session_id.as_deref() != Some(id));
        release_transport()
    }

    pub fn prepare_treatment(
        &self,
        id: &str,
        settings: AudioTreatment,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<TreatmentAnalysis> {
        let source = self.source(id)?;
        let mut last_session_check = Instant::now();
        let prepared = source.prepare_treatment(&settings, || {
            if cancelled() {
                return true;
            }
            if last_session_check.elapsed() >= Duration::from_millis(100) {
                last_session_check = Instant::now();
                return self.source(id).is_err();
            }
            false
        })?;
        self.source(id)?;
        Ok(TreatmentAnalysis {
            gain_db: prepared.gain_db(),
            loudness_lufs: prepared.loudness_lufs(),
            output_channels: settings
                .output_info(source.info)
                .map_err(StudioError::new)?
                .channels,
        })
    }

    pub fn with_playback_source<T>(
        &self,
        id: &str,
        settings: AudioTreatment,
        command: impl FnOnce(
            File,
            u32,
            u64,
            Option<Arc<audio::SeekIndex>>,
            PreparedTreatment,
        ) -> Result<T>,
    ) -> Result<T> {
        let state = lock(&self.state)?;
        let source = state
            .sources
            .get(id)
            .ok_or_else(|| StudioError::new("This studio session has closed"))?;
        command(
            source.open_current()?,
            source.info.sample_rate,
            source.info.frames,
            source
                .analysis
                .as_ref()
                .and_then(|analysis| analysis.seek_index.clone()),
            source.preview_treatment(settings)?,
        )
    }

    pub fn playback_treatment(
        &self,
        id: &str,
        settings: AudioTreatment,
    ) -> Result<(File, PreparedTreatment)> {
        let source = self.source(id)?;
        Ok((source.open_current()?, source.preview_treatment(settings)?))
    }

    pub fn with_review_playback<T>(
        &self,
        id: &str,
        output_index: usize,
        command: impl FnOnce(
            &str,
            File,
            u32,
            FrameRange,
            Option<Arc<audio::SeekIndex>>,
            PreparedTreatment,
        ) -> Result<T>,
    ) -> Result<T> {
        let state = lock(&self.state)?;
        let plan = state
            .pending
            .get(id)
            .ok_or_else(|| StudioError::new("Review the current settings before listening"))?;
        let output = plan
            .files
            .get(output_index)
            .ok_or_else(|| StudioError::new("Choose an output file to listen"))?;
        command(
            plan.session_id.as_deref().unwrap_or(id),
            output.source.open_current()?,
            output.source.info.sample_rate,
            output.range,
            output
                .source
                .analysis
                .as_ref()
                .and_then(|analysis| analysis.seek_index.clone()),
            output.treatment.clone(),
        )
    }

    pub fn peaks(
        &self,
        id: &str,
        range: FrameRange,
        bins: usize,
        stereo: bool,
        energy: bool,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<f32>> {
        let source = self.source(id)?;
        let file = source.open_current()?;
        let FrameRange { start, end } = range;
        if bins == 0 || bins > 4096 || start >= end || end > source.info.frames {
            return Err(StudioError::new("Invalid waveform viewport"));
        }
        let analysis = source
            .analysis
            .as_ref()
            .ok_or_else(|| StudioError::new("The waveform is unavailable"))?;
        let waveform = &analysis.waveform;
        let peaks = if (end - start) / bins as u64 >= waveform.frames_per_bucket() {
            waveform.channel_peaks(range, bins)
        } else {
            audio::waveform_channels_file_indexed(
                file,
                range,
                bins,
                analysis.seek_index.as_deref(),
                cancelled,
            )
        };
        let peaks = peaks.map_err(StudioError::new)?;
        let mut values = if stereo {
            peaks.stereo.map(flat_stereo).ok_or_else(|| {
                StudioError::new("Separate left and right waveforms require stereo audio")
            })?
        } else {
            flat_peaks(peaks.combined)
        };
        if energy {
            if stereo {
                values.extend(peaks.stereo_rms.into_iter().flatten().flatten());
            } else {
                values.extend(peaks.rms);
            }
        }
        Ok(values)
    }

    pub fn treatment_probe(
        &self,
        id: &str,
        frame: u64,
        settings: AudioTreatment,
        options: TreatmentProbeOptions,
        cancelled: impl FnMut() -> bool,
    ) -> Result<TreatmentProbe> {
        let source = self.source(id)?;
        let prepared = if options.dry_spectrogram() {
            PreparedTreatment::default()
        } else {
            source.preview_treatment(settings)?
        };
        let probe = audio::probe_treatment_with_options(
            source.open_current()?,
            source.info,
            frame,
            source
                .analysis
                .as_ref()
                .and_then(|analysis| analysis.seek_index.as_deref()),
            prepared,
            options,
            cancelled,
        )
        .map_err(StudioError::new)?;
        source.open_current()?;
        Ok(probe)
    }

    pub fn compare_preview_levels(
        &self,
        id: &str,
        frame: u64,
        settings: [AudioTreatment; 2],
        cancelled: impl FnMut() -> bool,
    ) -> Result<PreviewComparison> {
        let source = self.source(id)?;
        let [first, second] = settings;
        let prepared = [
            source.preview_treatment(first)?,
            source.preview_treatment(second)?,
        ];
        let levels = audio::compare_preview_levels(
            source.open_current()?,
            source.info,
            frame,
            source
                .analysis
                .as_ref()
                .and_then(|analysis| analysis.seek_index.as_deref()),
            prepared,
            cancelled,
        )
        .map_err(StudioError::new)?;
        source.open_current()?;
        Ok(levels)
    }

    pub fn learn_eq_threshold(
        &self,
        id: &str,
        range: FrameRange,
        settings: AudioTreatment,
        band_index: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<f32> {
        let source = self.source(id)?;
        let threshold = audio::learn_eq_threshold(
            source.open_current()?,
            source.info,
            range,
            source
                .analysis
                .as_ref()
                .and_then(|analysis| analysis.seek_index.as_deref()),
            source.preview_treatment(settings)?,
            band_index,
            cancelled,
        )
        .map_err(StudioError::new)?;
        source.open_current()?;
        Ok(threshold)
    }

    pub fn match_reference(
        &self,
        id: &str,
        range: FrameRange,
        reference: &Path,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<EqBand>> {
        let source = self.source(id)?;
        if range.start >= range.end || range.end > source.info.frames {
            return Err(StudioError::new("Choose an excerpt inside the recording"));
        }
        let target = File::open(reference).map_err(StudioError::new)?;
        let bands = audio::match_reference(
            source.open_current()?,
            range,
            source
                .analysis
                .as_ref()
                .and_then(|analysis| analysis.seek_index.as_deref()),
            target,
            cancelled,
        )
        .map_err(StudioError::new)?;
        source.open_current()?;
        Ok(bands)
    }

    pub fn cue_geometry(&self, id: &str) -> Result<(u32, u64)> {
        let source = self.source(id)?;
        Ok((source.info.sample_rate, source.info.frames))
    }

    pub fn discogs_proposal(
        &self,
        id: &str,
        release: &str,
        anchors: &[ProviderAnchor],
        fetch: impl FnOnce() -> Result<DiscogsRelease>,
    ) -> Result<DiscogsProposal> {
        let source = self.source(id)?;
        let cached = lock(&self.state)?
            .releases
            .get(id)
            .filter(|entry| {
                entry.edition == release && entry.fetched.elapsed() < Duration::from_secs(30 * 60)
            })
            .map(|entry| (entry.fetched, entry.release.clone()));
        let (fetched, edition) = match cached {
            Some(rows) => rows,
            None if anchors.is_empty() => (Instant::now(), fetch()?),
            None => {
                return Err(StudioError::new(
                    "Look up this release again before placing its tracks",
                ))
            }
        };
        source.open_current()?;
        let info = source.info;
        let proposal =
            markers::provider_markers(&edition.tracks, anchors, info.sample_rate, info.frames)
                .map_err(StudioError::new)?;
        let result = DiscogsProposal {
            rows: proposal.tracks,
            spans: proposal.spans,
            warnings: proposal.warnings,
            tags: edition.tags.clone(),
        };
        let mut state = lock(&self.state)?;
        if !state.sources.contains_key(id) {
            return Err(StudioError::new("This studio session has closed"));
        }
        state.releases.insert(
            id.into(),
            CachedRelease {
                edition: release.into(),
                fetched,
                release: edition,
            },
        );
        Ok(result)
    }

    pub fn preview(&self, request: ReviewRequest) -> Result<Review> {
        self.preview_with_recovery(request, None, None)
    }

    pub fn preview_with_recovery(
        &self,
        request: ReviewRequest,
        default_recovery: Option<&Path>,
        library_root: Option<&Path>,
    ) -> Result<Review> {
        let _policy = self.policy.enter();
        let tag_policy = self.policy.revision();
        let format = match request.format.as_str() {
            "flac" => OutputFormat::Flac,
            "mp3" => OutputFormat::Mp3 {
                bitrate_kbps: request.bitrate.unwrap_or(192),
            },
            _ => return Err(StudioError::new("Choose MP3 or FLAC output")),
        };
        let split_source = request
            .session_id
            .as_deref()
            .map(|id| self.source(id))
            .transpose()?;
        let legacy_folder = request.basename.trim();
        let legacy_folder = (!legacy_folder.is_empty()
            && (split_source.is_some() || request.save_mode == SaveMode::Folder))
            .then_some(legacy_folder);
        if let Some(folder) = legacy_folder {
            AudioSet::validate_name(folder).map_err(StudioError::new)?;
        }
        let folder_mask = request
            .folder_pattern
            .as_deref()
            .map(Mask::parse_for_render)
            .transpose()
            .map_err(StudioError::new)?;
        let custom = if request.save_mode == SaveMode::Folder {
            let destination = Path::new(&request.destination);
            if !destination.is_absolute() {
                return Err(StudioError::new("Choose or enter a full folder path"));
            }
            Some(destination.canonicalize().map_err(|error| {
                StudioError::new(format!("Could not open the save folder: {error}"))
            })?)
        } else {
            None
        };
        let copy_codec = split_source
            .as_ref()
            .and_then(|source| match (source.format, format) {
                (AudioFormat::Flac, OutputFormat::Flac) => Some(CopyCodec::Flac),
                (AudioFormat::Mp3, OutputFormat::Mp3 { .. }) => Some(CopyCodec::Mp3),
                _ => None,
            });
        if request.split_method == Some(SplitMethod::Copy) && copy_codec.is_none() {
            return Err(StudioError::new(
                "Frame copying is available for splits that keep the original MP3 or FLAC format.",
            ));
        }
        let mask = request
            .name_pattern
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .map(Mask::parse_for_render)
            .transpose()
            .map_err(StudioError::new)?;
        let split_ranges = split_source
            .as_ref()
            .map(|source| draft_ranges(&request.segments, source.info.frames))
            .transpose()?;
        if split_ranges
            .as_ref()
            .is_some_and(|ranges| !ranges.iter().any(|(_, segment)| segment.include))
        {
            return Err(StudioError::new("Include at least one output track"));
        }
        if request.treatment.is_active() && request.split_method == Some(SplitMethod::Copy) {
            return Err(StudioError::new(
                "Audio treatment requires re-encoding. Choose exact cuts.",
            ));
        }
        let copy_codec = (!request.treatment.is_active())
            .then_some(copy_codec)
            .flatten();
        let frame_index = match (copy_codec, &split_source, request.split_method) {
            (Some(codec), Some(source), None | Some(SplitMethod::Copy)) => {
                Some(source.frame_index(codec)?)
            }
            _ => None,
        };
        let copying = frame_index.is_some();
        let mut files = Vec::new();
        let mut destinations = Vec::<PlannedDestination>::new();
        let mut destination_ids = BTreeMap::<(PathBuf, Option<String>), usize>::new();
        let mut warnings = Vec::new();
        if request.treatment.is_active() {
            warnings.push("The selected audio treatments will be applied when encoding.".into());
        }
        if copying && copy_codec == Some(CopyCodec::Mp3) {
            warnings.push("MP3 players that ignore gapless timing tags may play a little extra audio at a cut.".to_owned());
        }
        let mut names = BTreeSet::new();
        let extension = request.format.as_str();
        let included = request
            .segments
            .iter()
            .filter(|row| row.include)
            .count()
            .to_string();
        let artwork = request
            .artwork_path
            .as_deref()
            .map(read_artwork)
            .transpose()?;
        let copy_cost = split_source
            .as_ref()
            .filter(|_| copying)
            .map_or(0, |source| source.stamp.bytes);
        let mut add = |source: &Arc<Source>,
                       treatment: PreparedTreatment,
                       range: FrameRange,
                       requested: FrameRange,
                       values: &Values,
                       number: Option<usize>|
         -> Result<()> {
            let parent = source
                .path
                .parent()
                .ok_or_else(|| StudioError::new("The recording has no source folder"))?;
            let root = custom.as_deref().unwrap_or(parent);
            let output_info = request
                .treatment
                .output_info(source.info)
                .map_err(StudioError::new)?;
            if !copying {
                audio::check_output(&output_info, format).map_err(StudioError::new)?;
            }
            if source.tag_policy != tag_policy {
                return Err(StudioError::new(
                    "The tag settings changed. Reopen the recording before reviewing.",
                ));
            }
            let mut tags = source.tags.clone();
            merge_tags(&mut tags, &request.tags)?;
            let folder = if let Some(mask) = &folder_mask {
                let rendered = mask.render(&tags).map_err(StudioError::new)?;
                let folder = portable_file_name(rendered.trim(), true);
                if folder.is_empty() {
                    return Err(StudioError::new(
                        "The folder pattern needs a name for every source",
                    ));
                }
                AudioSet::validate_name(&folder).map_err(StudioError::new)?;
                Some(folder)
            } else {
                legacy_folder.map(str::to_owned)
            };
            let named = folder.is_some();
            let destination_key = (
                root.to_path_buf(),
                folder.as_ref().map(|name| name.to_lowercase()),
            );
            let destination_id = if let Some(index) = destination_ids.get(&destination_key) {
                *index
            } else {
                let index = destinations.len();
                destinations.push(PlannedDestination::new(root.to_path_buf(), folder)?);
                destination_ids.insert(destination_key, index);
                index
            };
            merge_tags(&mut tags, values)?;
            if let Some(number) = number {
                tags.insert(TagField::TrackNumber, number.to_string());
                tags.insert(TagField::TrackTotal, included.clone());
            }
            let native_format = match format {
                OutputFormat::Flac => AudioFormat::Flac,
                OutputFormat::Mp3 { .. } => AudioFormat::Mp3,
            };
            tags.retain(|field, value| {
                let Some(canonical) = normalize_staged_tag_value(native_format, field, value)
                else {
                    warnings.push(format!(
                        "MP3 tag settings cannot store {} as {value}.",
                        field.to_storage_key()
                    ));
                    return false;
                };
                if *value != canonical && *field == TagField::Year {
                    let warning = format!("MP3 tag settings store {value} as {canonical}.");
                    if !warnings.contains(&warning) {
                        warnings.push(warning);
                    }
                }
                *value = canonical;
                true
            });
            let title = tags
                .get(&TagField::Title)
                .filter(|text| !text.trim().is_empty());
            let default_name = match number {
                Some(number) => {
                    format!("{number:02} - {}", title.map_or("Untitled", String::as_str))
                }
                None => source
                    .path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            };
            let mut stem = match &mask {
                Some(mask) => mask.render(&tags).map_err(StudioError::new)?,
                None => default_name,
            };
            if number.is_none()
                && !named
                && mask.is_none()
                && source
                    .path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
            {
                stem.push_str(" - Converted");
            }
            if number.is_some()
                && !named
                && mask.is_none()
                && source.path.file_name().is_some_and(|original| {
                    original
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&portable_file_name(
                            &format!("{stem}.{extension}"),
                            true,
                        ))
                })
            {
                stem.push_str(" - Split");
            }
            let name = portable_file_name(&format!("{stem}.{extension}"), true);
            AudioSet::validate_name(&name).map_err(StudioError::new)?;
            if !names.insert((destination_id, name.to_lowercase())) {
                return Err(StudioError::new(format!(
                    "Two outputs would be named {name}. Change the filename pattern."
                )));
            }
            if !named {
                require_unused(&destinations[destination_id].output_path(&name))?;
            }
            if !copying
                && extension == "mp3"
                && source.format == AudioFormat::Mp3
                && !warnings
                    .iter()
                    .any(|warning| warning == "MP3 recordings will be encoded again.")
            {
                warnings.push("MP3 recordings will be encoded again.".into());
            }
            if title.is_none()
                && number.is_some()
                && !warnings
                    .iter()
                    .any(|warning| warning == "Some tracks have no title.")
            {
                warnings.push("Some tracks have no title.".into());
            }
            if !copying && format == OutputFormat::Flac && output_info.bits_per_sample.is_none()
                && !warnings.iter().any(|warning| warning == "Decoded audio without integer bit depth will be stored as 24-bit FLAC.")
            {
                warnings.push("Decoded audio without integer bit depth will be stored as 24-bit FLAC.".into());
            }
            if !copying && format == OutputFormat::Flac {
                let depth = audio::flac_output_bits(&output_info);
                if let Some(bits) = output_info.bits_per_sample.filter(|bits| *bits != depth) {
                    let warning = format!("{bits}-bit audio will be stored in {depth}-bit FLAC with the original sample precision.");
                    if !warnings.contains(&warning) {
                        warnings.push(warning);
                    }
                }
            }
            tags.retain(|field, value| {
                let fits = is_writable_value_at(Path::new(&name), field, value);
                if !fits {
                    warnings.push(format!(
                        "{name} cannot carry the reviewed {} value.",
                        field.to_storage_key()
                    ));
                }
                fits
            });
            let frames = range.end - range.start;
            let audio = match (copying, format) {
                (true, _) => 32_768,
                (false, OutputFormat::Flac) => {
                    frames * u64::from(output_info.channels) * 4 + frames / 64 + 16_384
                }
                (false, OutputFormat::Mp3 { .. }) => {
                    (frames / u64::from(source.info.sample_rate) + 1) * 40_000 + 16_384
                }
            };
            let destination = &mut destinations[destination_id];
            destination.bytes_required = destination
                .bytes_required
                .saturating_add(audio + (tags.values().map(String::len).sum::<usize>() as u64) * 4);
            files.push(PlannedFile {
                source: source.clone(),
                treatment,
                range,
                requested,
                name,
                tags,
                destination: destination_id,
            });
            Ok(())
        };
        if let (Some(source), Some(ranges)) = (split_source, split_ranges) {
            let treatment = source.prepare_treatment(&request.treatment, || false)?;
            let mut number = 0;
            for (requested, segment) in ranges {
                let range = frame_index.as_ref().map_or(requested, |index| FrameRange {
                    start: index.nearest(requested.start),
                    end: index.nearest(requested.end),
                });
                if range.start >= range.end {
                    return Err(StudioError::new(
                        "Two cuts snap to the same audio frame. Space the cuts farther apart or choose exact cuts.",
                    ));
                }
                if segment.include {
                    number += 1;
                    add(
                        &source,
                        treatment.clone(),
                        range,
                        requested,
                        &segment.tags,
                        Some(number),
                    )?;
                }
            }
        } else {
            if request.sources.is_empty() || request.sources.len() > MAX_TRACKS {
                return Err(StudioError::new("Choose between 1 and 200 recordings"));
            }
            let mut seen = BTreeSet::new();
            let mut sources: Vec<Arc<Source>> = Vec::new();
            for path in &request.sources {
                let path = Path::new(path);
                if !seen.insert(path.canonicalize().map_err(StudioError::new)?) {
                    return Err(StudioError::new("A recording was selected twice"));
                }
                let source = Arc::new(Source::load(path, 1, || false)?);
                let already_selected = {
                    let identity = lock(&source.identity)?;
                    sources.iter().any(|previous| {
                        previous
                            .identity
                            .lock()
                            .is_ok_and(|previous| *previous == *identity)
                    })
                };
                if already_selected {
                    return Err(StudioError::new(
                        "Two selections refer to the same recording",
                    ));
                }
                let range = FrameRange {
                    start: 0,
                    end: source.info.frames,
                };
                let treatment = source.prepare_treatment(&request.treatment, || false)?;
                add(&source, treatment, range, range, &Values::new(), None)?;
                sources.push(source);
            }
        }
        if files.is_empty() {
            return Err(StudioError::new("Include at least one output track"));
        }
        destinations[0].bytes_required = destinations[0].bytes_required.saturating_add(copy_cost);
        let mut seen_source = BTreeSet::new();
        for file in &files {
            if seen_source.insert(&file.source.path) {
                let copies = files
                    .iter()
                    .filter(|other| Arc::ptr_eq(&other.source, &file.source))
                    .count() as u64;
                let (covers, warning) = output_covers(&file.source, artwork.as_ref(), format)?;
                if let Some(warning) = warning {
                    warnings.push(warning);
                }
                let destination = &mut destinations[file.destination];
                destination.bytes_required = destination.bytes_required.saturating_add(
                    covers
                        .iter()
                        .map(|cover| cover.data.len() as u64 + 4_096)
                        .sum::<u64>()
                        * copies,
                );
            }
        }
        for destination in &destinations {
            check_space(&destination.guard, destination.bytes_required)?;
        }
        let replacement = if request.save_mode == SaveMode::Replace {
            if files.iter().any(|file| !file.source.replaceable) {
                return Err(StudioError::new(
                    "Replace the original from its real path, or save a copy from this link",
                ));
            }
            let backup = request
                .backup_directory
                .as_deref()
                .map(Path::new)
                .or(default_recovery)
                .ok_or_else(|| StudioError::new("Choose a recovery folder for the originals"))?
                .canonicalize()
                .map_err(StudioError::new)?;
            if library_root
                .and_then(|root| root.canonicalize().ok())
                .is_some_and(|root| backup.starts_with(root))
            {
                return Err(StudioError::new(
                    "Choose a recovery folder outside the open library",
                ));
            }
            Some(
                AudioReplacement::plan(
                    &backup,
                    &seen_source.into_iter().cloned().collect::<Vec<_>>(),
                )
                .map_err(StudioError::new)?,
            )
        } else {
            None
        };
        let backups = replacement
            .as_ref()
            .map(|replacement| {
                replacement
                    .backups()
                    .map(|(source, path)| BackupRow {
                        source: source.to_string_lossy().into_owned(),
                        path: path.to_string_lossy().into_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if self.policy.revision() != tag_policy
            || tagrex_core::model::read_policy_revision() != tag_policy
        {
            return Err(StudioError::new(
                "Tag settings changed. Review the export again.",
            ));
        }
        let outputs = files
            .iter()
            .map(|file| OutputRow {
                path: destinations[file.destination]
                    .output_path(&file.name)
                    .to_string_lossy()
                    .into_owned(),
                start_frame: file.range.start,
                end_frame: file.range.end,
                requested_start_frame: (file.requested.start != file.range.start)
                    .then_some(file.requested.start),
                requested_end_frame: (file.requested.end != file.range.end)
                    .then_some(file.requested.end),
                sample_rate: file.source.info.sample_rate,
                tags: public_tags(&file.tags),
            })
            .collect();
        let id = self.id();
        let mut state = lock(&self.state)?;
        if let Some(session) = &request.session_id {
            if !state.sources.contains_key(session) {
                return Err(StudioError::new("This studio session has closed"));
            }
        }
        state.pending.clear();
        state.pending.insert(
            id.clone(),
            Plan {
                session_id: request.session_id,
                destinations,
                format,
                frame_index,
                files,
                artwork,
                replacement,
                tag_policy,
            },
        );
        Ok(Review {
            id,
            outputs,
            warnings,
            backups,
            split_method: if copying {
                SplitMethod::Copy
            } else {
                SplitMethod::Exact
            },
        })
    }

    pub fn progress(&self, id: &str) -> Result<Progress> {
        let job = lock(&self.state)?
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| StudioError::new("Export has not started"))?;
        let progress = lock(&job.progress)?.clone();
        Ok(progress)
    }

    pub fn cancel(&self, id: &str) -> Result<()> {
        let mut state = lock(&self.state)?;
        if let Some(job) = state.jobs.get(id) {
            job.cancelled.store(true, Ordering::Release);
        } else {
            state.pending.remove(id);
        }
        Ok(())
    }

    pub fn export(&self, id: &str) -> Result<ExportResult> {
        self.export_with_library(id, None)
    }

    pub fn export_with_library(
        &self,
        id: &str,
        library_root: Option<&Path>,
    ) -> Result<ExportResult> {
        let _policy = self.policy.enter();
        let (plan, job) = {
            let mut state = lock(&self.state)?;
            if state.jobs.values().any(|job| {
                lock(&job.progress).is_ok_and(|p| {
                    matches!(p.phase, "copying" | "encoding" | "verifying" | "publishing")
                })
            }) {
                return Err(StudioError::new("Another audio export is already running"));
            }
            let plan = state
                .pending
                .remove(id)
                .ok_or_else(|| StudioError::new("Review the current settings before exporting"))?;
            if let (Some(root), Some(replacement)) = (library_root, &plan.replacement) {
                let root = root.canonicalize().map_err(StudioError::new)?;
                if replacement
                    .backups()
                    .any(|(_, path)| path.starts_with(&root))
                {
                    return Err(StudioError::new(
                        "Choose a recovery folder outside the open library",
                    ));
                }
            }
            let job = Arc::new(ExportState {
                progress: Mutex::new(Progress {
                    phase: if plan.frame_index.is_some() {
                        "copying"
                    } else {
                        "encoding"
                    },
                    completed: 0,
                    total: plan.files.len(),
                    frames_done: 0,
                    frames_total: plan
                        .files
                        .iter()
                        .map(|file| file.range.end - file.range.start)
                        .sum(),
                }),
                cancelled: AtomicBool::new(false),
                result: Mutex::new(None),
                replacement: Mutex::new(None),
                published_folders: Mutex::new(Vec::new()),
            });
            state.jobs.clear();
            state.jobs.insert(id.to_owned(), job.clone());
            (plan, job)
        };
        let result = execute(plan, &job, self);
        if let Ok(output) = &result {
            *lock(&job.result)? = Some(output.clone());
        }
        lock(&job.progress)?.phase = match &result {
            Ok(_) => "done",
            Err(_) if job.cancelled.load(Ordering::Acquire) => "cancelled",
            Err(_) => "failed",
        };
        result
    }

    pub fn published_folder(&self, id: &str) -> Result<PathBuf> {
        let result = self.published_result(id)?;
        let folder = result
            .paths
            .first()
            .and_then(|path| Path::new(path).parent())
            .ok_or_else(|| StudioError::new("The export has no output folder"))?
            .to_path_buf();
        if !folder
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_dir())
        {
            return Err(StudioError::new("The export folder has been moved"));
        }
        Ok(folder)
    }

    fn job(&self, id: &str) -> Result<Arc<ExportState>> {
        let state = lock(&self.state)?;
        state
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| StudioError::new("This export is no longer available"))
    }

    pub fn published_result(&self, id: &str) -> Result<ExportResult> {
        let job = self.job(id)?;
        let result = lock(&job.result)?.clone();
        result.ok_or_else(|| StudioError::new("The export has not completed"))
    }

    pub fn undo_replacement(&self, id: &str) -> Result<RestoreResult> {
        let job = self.job(id)?;
        let result = self.published_result(id)?;
        let mut replacement = lock(&job.replacement)?;
        let retained = replacement
            .as_ref()
            .ok_or_else(|| StudioError::new("This export has no replacement to undo"))?
            .undo()
            .map_err(StudioError::new)?;
        *replacement = None;
        let warnings =
            lock(&job.published_folders)?
                .drain(..)
                .filter_map(|folder| folder.remove_if_empty().err())
                .map(|error| format!("An empty output folder could not be removed. {error}"))
                .chain(retained.iter().map(|path| {
                    format!("An extra recovery item was retained at {}.", path.display())
                }))
                .collect();
        Ok(RestoreResult {
            paths: result.replaced,
            removed: result.paths,
            warnings,
        })
    }
}

fn draft_ranges(
    segments: &[DraftSegment],
    frames: u64,
) -> Result<Vec<(FrameRange, &DraftSegment)>> {
    if segments.is_empty() || segments.len() > MAX_TRACKS {
        return Err(StudioError::new("Choose between 1 and 200 tracks"));
    }
    let mut result = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let end = segments
            .get(index + 1)
            .map_or(frames, |next| next.start_frame);
        if segment.start_frame >= end || end > frames {
            return Err(StudioError::new(
                "Cut times must be ordered and inside the recording",
            ));
        }
        result.push((
            FrameRange {
                start: segment.start_frame,
                end,
            },
            segment,
        ));
    }
    Ok(result)
}

fn read_artwork(path: &str) -> Result<CoverArt> {
    const LIMIT: u64 = 32 * 1024 * 1024;
    let mut data = Vec::new();
    let file = open_regular(Path::new(path))?;
    file.take(LIMIT + 1)
        .read_to_end(&mut data)
        .map_err(StudioError::new)?;
    if data.len() as u64 > LIMIT {
        return Err(StudioError::new("Artwork is larger than 32 MiB"));
    }
    let mime = match data.as_slice() {
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        b if b.starts_with(b"\x89PNG\r\n\x1a\n") => "image/png",
        _ => return Err(StudioError::new("Choose JPEG or PNG artwork")),
    };
    let art = CoverArt {
        mime: mime.into(),
        kind: CoverKind::Front,
        data: data.into(),
        description: String::new(),
    };
    use tagrex_core::cover::{inspect_cover_size, CoverSizeInspection};
    match inspect_cover_size(&art, u32::MAX) {
        CoverSizeInspection::WithinLimit { .. } => Ok(art),
        CoverSizeInspection::Busy => Err(StudioError::new(
            "Artwork inspection is busy. Try again shortly.",
        )),
        _ => Err(StudioError::new("The chosen image could not be read")),
    }
}

fn output_covers(
    source: &Source,
    artwork: Option<&CoverArt>,
    format: OutputFormat,
) -> Result<(Vec<CoverArt>, Option<String>)> {
    const FLAC_PICTURE_LIMIT: usize = (16 * 1024 * 1024) - 4096;
    const ARTWORK_SET_LIMIT: usize = 32 * 1024 * 1024;
    let mut covers = TagEngine::read_covers(&source.path).map_err(StudioError::new)?;
    source.open_current()?;
    if let Some(artwork) = artwork {
        if format == OutputFormat::Flac && artwork.data.len() > FLAC_PICTURE_LIMIT {
            return Err(StudioError::new(
                "The selected image is too large for a FLAC picture block",
            ));
        }
        covers.retain(|cover| cover.kind != CoverKind::Front);
        covers.insert(0, artwork.clone());
    }
    let mut remaining = ARTWORK_SET_LIMIT;
    let len = covers.len();
    covers.retain(|cover| {
        let size = cover.data.len().saturating_add(cover.description.len());
        if size > remaining || (format == OutputFormat::Flac && size > FLAC_PICTURE_LIMIT) {
            return false;
        }
        remaining -= size;
        true
    });
    let warning = (len != covers.len()).then(|| {
        format!(
            "Some embedded artwork from {} exceeds the output size limit and will be skipped.",
            source
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )
    });
    Ok((covers, warning))
}

fn check_space(destination: &AudioDestination, bytes: u64) -> Result<()> {
    let available = destination.available_bytes().map_err(StudioError::new)?;
    if available < bytes {
        return Err(StudioError::new(
            "There is too little available space for the estimated audio export",
        ));
    }
    Ok(())
}

fn execute(mut plan: Plan, job: &ExportState, studio: &Studio) -> Result<ExportResult> {
    let cancelled = || job.cancelled.load(Ordering::Acquire);
    let check_stop = || {
        if cancelled() {
            Err(StudioError::new("Audio export stopped"))
        } else {
            Ok(())
        }
    };
    check_stop()?;
    let check_policy = || {
        if studio.policy.revision() != plan.tag_policy
            || tagrex_core::model::read_policy_revision() != plan.tag_policy
        {
            return Err(StudioError::new(
                "Tag settings changed. Review the export again.",
            ));
        }
        Ok(())
    };
    check_policy()?;
    for destination in &plan.destinations {
        check_space(&destination.guard, destination.bytes_required)?;
    }
    for file in &plan.files {
        file.source.open_current()?;
    }
    let mut stages = Vec::with_capacity(plan.destinations.len());
    let mut published_paths: Vec<PathBuf> = Vec::new();
    let result = (|| {
        for destination in &plan.destinations {
            stages.push(destination.begin()?);
        }
        let mut offset = 0;
        let mut frames_done = 0;
        let mut last_update = std::time::Instant::now();
        while offset < plan.files.len() {
            check_stop()?;
            let source = &plan.files[offset].source;
            let stage = &mut stages[plan.files[offset].destination];
            let staging = stage.staging_path().map_err(StudioError::new)?;
            let end = offset
                + plan.files[offset..]
                    .iter()
                    .take_while(|file| Arc::ptr_eq(&file.source, source))
                    .count();
            let files = &plan.files[offset..end];
            let ranges: Vec<_> = files
                .iter()
                .map(|file| RenderSegment {
                    range: file.range,
                    output: staging.join(&file.name),
                })
                .collect();
            let held = source.open_current()?;
            let progress = |part: audio::RenderProgress| {
                if last_update.elapsed().as_millis() >= 40
                    || part.completed_segments == part.total_segments
                {
                    if let Ok(mut progress) = job.progress.lock() {
                        progress.frames_done = frames_done + part.completed_frames;
                    }
                    last_update = std::time::Instant::now();
                }
            };
            let open = |path: &Path| {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| std::io::Error::other("No staged filename"))?;
                stage.create_file(name)
            };
            match &plan.frame_index {
                Some(index) => index.write(held, &ranges, cancelled, progress, open),
                None => audio::render_treated_file(
                    held,
                    &ranges,
                    plan.format,
                    files[0].treatment.clone(),
                    cancelled,
                    progress,
                    open,
                ),
            }
            .map_err(StudioError::new)?;
            let (covers, _) = output_covers(source, plan.artwork.as_ref(), plan.format)?;
            for (index, file) in files.iter().enumerate() {
                check_stop()?;
                check_policy()?;
                lock(&job.progress)?.phase = "verifying";
                let path = staging.join(&file.name);
                let format = match plan.format {
                    OutputFormat::Flac => AudioFormat::Flac,
                    OutputFormat::Mp3 { .. } => AudioFormat::Mp3,
                };
                let held = stage.checked_file(&file.name).map_err(StudioError::new)?;
                tagrex_core::plan::tag_staged_audio(
                    &TrackFile {
                        path: path.clone(),
                        format,
                        tags: file.tags.clone(),
                    },
                    &held,
                    &covers,
                )
                .map_err(StudioError::new)?;
                let witness = AudioSet::witness(&held).map_err(StudioError::new)?;
                let (saved, saved_covers) =
                    tagrex_core::plan::read_staged_audio(&path, &held).map_err(StudioError::new)?;
                if saved.format != format {
                    return Err(StudioError::new(
                        "The saved audio format did not match the review",
                    ));
                }
                for (field, expected) in &file.tags {
                    if saved.tags.get(field) != Some(expected) {
                        return Err(StudioError::new(format!(
                            "Could not verify {} in {}",
                            field.to_storage_key(),
                            file.name
                        )));
                    }
                }
                if saved_covers.len() != covers.len()
                    || !covers.iter().all(|expected| {
                        saved_covers.iter().any(|actual| {
                            expected.data == actual.data && expected.kind == actual.kind
                        })
                    })
                {
                    return Err(StudioError::new(format!(
                        "Could not verify artwork in {}",
                        file.name
                    )));
                }
                let expected_frames = file.range.end - file.range.start;
                let verify = held.try_clone().map_err(StudioError::new)?;
                let info = if let Some(index) = &plan.frame_index {
                    let mut verify = verify;
                    FrameCopyIndex::scan(&mut verify, index.codec(), expected_frames, cancelled)
                        .map_err(StudioError::new)?;
                    audio::probe_file(verify).map_err(StudioError::new)?.source
                } else {
                    audio::inspect_file(verify, 1, cancelled)
                        .map_err(StudioError::new)?
                        .source
                };
                let expected = file
                    .treatment
                    .settings()
                    .output_info(source.info)
                    .map_err(StudioError::new)?;
                if info.sample_rate != expected.sample_rate
                    || info.channels != expected.channels
                    || (plan.frame_index.is_none() && info.frames != expected_frames)
                    || (plan.format == OutputFormat::Flac
                        && info.bits_per_sample
                            != if plan.frame_index.is_some() {
                                source.info.bits_per_sample
                            } else {
                                Some(audio::flac_output_bits(&expected))
                            })
                {
                    return Err(StudioError::new(format!(
                        "Could not verify the audio length of {}",
                        file.name
                    )));
                }
                stage
                    .mark_verified(&file.name, &held, witness)
                    .map_err(StudioError::new)?;
                lock(&job.progress)?.completed = offset + index + 1;
            }
            frames_done += files
                .iter()
                .map(|file| file.range.end - file.range.start)
                .sum::<u64>();
            offset = end;
            if offset < plan.files.len() {
                lock(&job.progress)?.phase = if plan.frame_index.is_some() {
                    "copying"
                } else {
                    "encoding"
                };
            }
        }
        for file in &plan.files {
            file.source.open_current()?;
        }
        // Check every destination again before any part of the set is visible.
        // The exclusive publish operations also refuse races after this check.
        for destination in &plan.destinations {
            if let Some(folder) = &destination.folder {
                require_unused(&destination.root.join(folder))?;
            }
        }
        for file in &plan.files {
            let destination = &plan.destinations[file.destination];
            if destination.folder.is_none() {
                require_unused(&destination.output_path(&file.name))?;
            }
        }
        check_stop()?;
        check_policy()?;
        lock(&job.progress)?.phase = "publishing";
        let mut witnesses = Vec::new();
        let mut folders = Vec::new();
        for (index, stage) in stages.iter_mut().enumerate() {
            let names = plan
                .files
                .iter()
                .filter(|file| file.destination == index)
                .map(|file| file.name.clone())
                .collect::<Vec<_>>();
            if plan.destinations[index].folder.is_some() {
                let folder = stage.publish(&names, cancelled).map_err(StudioError::new)?;
                published_paths.extend(names.iter().map(|name| folder.join(name)));
                if plan.replacement.is_some() {
                    witnesses.extend(
                        stage
                            .published_witnesses(&names, &folder)
                            .map_err(StudioError::new)?,
                    );
                    folders.push(stage.published_folder_witness().map_err(StudioError::new)?);
                }
            } else {
                let published = stage.publish_files(&names).map_err(StudioError::new)?;
                published_paths.extend(published.iter().map(|(path, _)| path.clone()));
                if plan.replacement.is_some() {
                    witnesses.extend(published);
                }
            }
        }
        let mut replaced = Vec::new();
        let mut backups = Vec::new();
        let mut warnings = Vec::new();
        if let Some(replacement) = plan.replacement.take() {
            let mut original_paths = BTreeSet::new();
            for (source, path) in replacement.backups() {
                replaced.push(source.to_string_lossy().into_owned());
                backups.push(path.to_string_lossy().into_owned());
                original_paths.insert(source.to_path_buf());
            }
            let completed = replacement
                .commit(&witnesses, || {
                    // The original names are retired and their backups are
                    // verified. Drop studio pins before Windows removes them.
                    let mut state = studio.state.lock().unwrap_or_else(|p| p.into_inner());
                    state.pending.retain(|_, pending| {
                        !pending
                            .files
                            .iter()
                            .any(|file| original_paths.contains(&file.source.path))
                    });
                    for source in plan
                        .files
                        .iter()
                        .map(|file| &file.source)
                        .chain(state.sources.values())
                        .chain(&state.recent_sources)
                        .filter(|source| original_paths.contains(&source.path))
                    {
                        source
                            .identity
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .take();
                    }
                })
                .map_err(StudioError::new)?;
            warnings.extend(
                completed.retained_paths().iter().map(|path| {
                    format!("An extra recovery file was retained at {}.", path.display())
                }),
            );
            *lock(&job.replacement)? = Some(completed);
            *lock(&job.published_folders)? = folders;
        }
        Ok(ExportResult {
            paths: plan
                .files
                .iter()
                .map(|file| {
                    plan.destinations[file.destination]
                        .output_path(&file.name)
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
            replaced,
            backups,
            warnings,
        })
    })();
    match result {
        Err(error) => {
            let retained: Vec<_> = stages
                .iter_mut()
                .filter_map(|stage| {
                    stage
                        .discard()
                        .err()
                        .and_then(|_| stage.recovery_path().ok())
                })
                .collect();
            if retained.is_empty() && published_paths.is_empty() {
                Err(error)
            } else {
                Err(StudioError::new(format!(
                    "{error}. Audio may be available at {}.",
                    published_paths
                        .iter()
                        .chain(&retained)
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
            }
        }
        Ok(output) => Ok(output),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedCue {
    source_name: String,
    markers: Vec<ImportedMarker>,
    tags: Values,
    warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportedMarker {
    frame: u64,
    title: Option<String>,
    artist: Option<String>,
    tags: Values,
    pregap_frame: Option<u64>,
    index00: Option<u64>,
    index01: u64,
    pregap: Option<u64>,
    postgap: Option<u64>,
}

fn cue_values(tags: &CueTags) -> Values {
    [
        ("album", &tags.title),
        ("albumartist", &tags.performer),
        ("artist", &tags.performer),
        ("composer", &tags.songwriter),
        ("year", &tags.date),
        ("genre", &tags.genre),
        ("catalognumber", &tags.catalog),
    ]
    .into_iter()
    .filter_map(|(field, value)| {
        value
            .as_ref()
            .map(|value| (field.to_string(), value.clone()))
    })
    .collect()
}

impl Studio {
    pub fn import_cue(&self, id: &str, path: &Path, encoding: CueEncoding) -> Result<ImportedCue> {
        let source = self.source(id)?;
        source.open_current()?;
        let mut bytes = Vec::new();
        let file = open_regular(path)?;
        file.take(CUE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(StudioError::new)?;
        if bytes.len() as u64 > CUE_BYTES {
            return Err(StudioError::new("CUE sheet is too large"));
        }
        let cue = markers::parse_cue(&bytes, encoding).map_err(StudioError::new)?;
        let indexed = cue
            .markers(source.info.sample_rate, source.info.frames)
            .map_err(StudioError::new)?;
        let markers = indexed
            .into_iter()
            .map(|marker| ImportedMarker {
                frame: marker.frame,
                title: marker.tags.title.clone(),
                artist: marker.tags.performer.clone(),
                tags: [
                    ("title", marker.tags.title),
                    ("artist", marker.tags.performer),
                    ("composer", marker.tags.songwriter),
                    ("isrc", marker.tags.isrc),
                    ("genre", marker.tags.genre),
                    ("year", marker.tags.date),
                    ("catalognumber", marker.tags.catalog),
                ]
                .into_iter()
                .filter_map(|(field, value)| value.map(|value| (field.to_owned(), value)))
                .collect(),
                pregap_frame: marker.pregap_frame,
                index00: marker.index00,
                index01: marker.index01,
                pregap: marker.pregap,
                postgap: marker.postgap,
            })
            .collect();
        Ok(ImportedCue {
            source_name: cue.source_name.clone(),
            markers,
            tags: cue_values(&cue.tags),
            warnings: cue.warnings,
        })
    }

    pub fn save_cue(
        &self,
        id: &str,
        path: &Path,
        segments: &[DraftSegment],
        values: &Values,
    ) -> Result<Vec<String>> {
        let source = self.source(id)?;
        source.open_current()?;
        let info = source.info;
        let mut shared = source.tags.clone();
        merge_tags(&mut shared, values)?;
        let tags = CueTags {
            title: shared.get(&TagField::Album).cloned(),
            performer: shared.get(&TagField::AlbumArtist).cloned(),
            songwriter: shared.get(&TagField::Composer).cloned(),
            genre: shared.get(&TagField::Genre).cloned(),
            date: shared.get(&TagField::Year).cloned(),
            catalog: shared.get(&TagField::CatalogNumber).cloned(),
            ..Default::default()
        };
        let imported_lead_in = segments.first().is_some_and(|segment| segment.lead_in)
            && segments
                .get(1)
                .is_some_and(|segment| segment.index01.is_some());
        if segments
            .iter()
            .skip(usize::from(imported_lead_in))
            .any(|segment| !segment.include)
        {
            return Err(StudioError::new(markers::MarkerError::CueExclusions));
        }
        let tracks: Vec<_> = draft_ranges(segments, info.frames)?
            .into_iter()
            .skip(usize::from(imported_lead_in))
            .filter(|(_, segment)| segment.include)
            .map(|(range, segment)| {
                let mut resolved = shared.clone();
                merge_tags(&mut resolved, &segment.tags)?;
                Ok(CueExportTrack {
                    range,
                    tags: CueTags {
                        title: resolved.get(&TagField::Title).cloned(),
                        performer: resolved.get(&TagField::Artist).cloned(),
                        songwriter: resolved.get(&TagField::Composer).cloned(),
                        isrc: resolved.get(&TagField::Isrc).cloned(),
                        ..Default::default()
                    },
                    pregap_frame: segment.pregap_frame,
                    index00: segment.index00,
                    index01: segment.index01,
                    pregap: segment.pregap,
                    postgap: segment.postgap,
                })
            })
            .collect::<Result<_>>()?;
        let parent = path
            .parent()
            .ok_or_else(|| StudioError::new("Choose a CUE destination"))?;
        let parent = parent.canonicalize().map_err(StudioError::new)?;
        let file_name = path
            .file_name()
            .ok_or_else(|| StudioError::new("Choose a CUE filename"))?;
        if !path
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("cue"))
        {
            return Err(StudioError::new("Use a .cue filename"));
        }
        let source_name = if source.path.parent() == Some(parent.as_path()) {
            source
                .path
                .file_name()
                .ok_or_else(|| StudioError::new("Recording has no filename"))?
        } else {
            source.path.as_os_str()
        }
        .to_str()
        .ok_or_else(|| StudioError::new("This recording path cannot be written as text"))?;
        let cue = markers::write_cue(source_name, info.sample_rate, info.frames, &tags, &tracks)
            .map_err(StudioError::new)?;
        write_new_cue(&parent.join(file_name), cue.text).map_err(StudioError::new)?;
        Ok(cue.warnings)
    }
}

#[cfg(test)]
mod tests;
