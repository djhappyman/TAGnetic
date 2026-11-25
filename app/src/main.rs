//! TagRex desktop shell (Tauri).
//!
//! Deliberately thin: each command locks the shared [`App`] and forwards one
//! call into it. All logic lives in `tagrex-core` behind the `App` command
//! layer (see `lib.rs` and docs/architecture.md). Vite compiles the Svelte views,
//! component styles and workflow controllers under `ui/`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

mod audio_studio;
mod beatport_session_control;
mod blocking;
mod mcp_control;
mod media_session;
mod navigation;
mod operation_control;
mod secure_provider_image;
mod settings_store;
mod update;
#[cfg(target_os = "linux")]
mod webkit_compositor;
mod window_controls;

use tauri::{Emitter, Manager, State};

use tagrex_commands::{
    ActionGroupDto, AlignMatchDto, App, AppError, BatchDto, BlockTargetsDto, CandidateDto,
    CoverArtDto, CoverExportDto, CoverSummaryDto, DropResultDto, DuplicateGroupDto, ErrorDto,
    ImportFieldDto, ImportSelectionDto, ImportTrackDto, LiveCoverAssignmentDto, NameProbeDto,
    NameProbesDto, PlaceholderDto, PlanDto, ProviderHub, QuickWriteDto, ReleaseDto, SaveImagesDto,
    SearchQueryDto, SettingsDto, TagEditDto, TextLinesDto, TextProbeDto, TrackDto,
    TransformRuleDto, UndoReportDto,
};
use tagrex_core::plan::UndoMode;
use tagrex_player::{Player, PlayerStatus};

/// No library is open until the user opens one, hence `Option`. `Mutex` makes
/// the non-`Sync` journal usable as shared Tauri state.
type AppState = Arc<Mutex<Option<App>>>;

static SAMPLE_QUEUE: blocking::Queue = blocking::Queue::new();
static IMAGE_READ_QUEUE: blocking::Queue = blocking::Queue::new();
static IMAGE_ADMISSION: std::sync::LazyLock<operation_control::ImageAdmission> =
    std::sync::LazyLock::new(|| operation_control::ImageAdmission::new(6));
static BROWSER_QUEUE: blocking::Queue = blocking::Queue::new();
static SETTINGS_QUEUE: blocking::Queue = blocking::Queue::new();
static SESSION_QUEUE: blocking::Queue = blocking::Queue::new();
static FIELD_PREVIEW_LANE: blocking::Latest = blocking::Latest::new();
static RENAME_PREVIEW_LANE: blocking::Latest = blocking::Latest::new();
static NAME_PREVIEW_LANE: blocking::Latest = blocking::Latest::new();
static TEXT_PREVIEW_LANE: blocking::Latest = blocking::Latest::new();
static TRANSFORM_PREVIEW_LANE: blocking::Latest = blocking::Latest::new();
static MOVE_PREVIEW_LANE: blocking::Latest = blocking::Latest::new();
static GROUP_PREVIEW_LANE: blocking::Latest = blocking::Latest::new();
static GROUP_PLAN_PREVIEW_LANE: blocking::Latest = blocking::Latest::new();
static LIBRARY_QUERY_QUEUE: blocking::Queue = blocking::Queue::new();
static LIBRARY_SUGGEST_QUEUE: blocking::Queue = blocking::Queue::new();
static QUERY_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SUGGEST_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static DUPLICATE_QUEUE: blocking::Queue = blocking::Queue::new();
static DUPLICATE_JOB_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static DUPLICATE_JOBS: std::sync::LazyLock<
    Mutex<std::collections::BTreeMap<u64, Arc<DuplicateJob>>>,
> = std::sync::LazyLock::new(Mutex::default);
static DUPLICATE_ANALYSIS: std::sync::LazyLock<Mutex<tagrex_commands::duplicates::AnalysisCache>> =
    std::sync::LazyLock::new(Mutex::default);

struct DuplicateJob {
    status: Mutex<tagrex_commands::duplicates::DuplicateJobStatus>,
    cancel: std::sync::atomic::AtomicBool,
    index: std::sync::Weak<tagrex_commands::query::QueryIndex>,
}

impl DuplicateJob {
    // Callers hold DUPLICATE_JOBS before publishing. Start, cancel and worker
    // publication therefore cannot interleave old and new job generations.
    fn publish(&self, update: tagrex_commands::duplicates::ScanProgress) {
        let Some(index) = self.index.upgrade() else {
            return;
        };
        if self.cancel.load(Ordering::Acquire) || !index.is_current() {
            return;
        }
        let Ok(mut status) = self.status.lock() else {
            return;
        };
        if status.status != "running" {
            return;
        }
        status.processed = update.processed;
        status.total = update.total;
        status.skipped = update.skipped;
        status.errors = update.errors;
        status.phase = update.phase.into();
        if let Some(groups) = update.groups {
            status.groups_revision += 1;
            status.groups_count = groups.len();
            index.set_duplicate_results(
                &status.kind,
                tagrex_commands::query::DuplicateResults {
                    status: "running".into(),
                    groups,
                    groups_revision: status.groups_revision,
                    skipped: status.skipped,
                    errors: status.errors,
                },
            );
        }
    }

    fn clear(&self, terminal: &str, message: Option<String>, publish_scope: bool) {
        self.cancel.store(true, Ordering::Release);
        let Ok(mut status) = self.status.lock() else {
            return;
        };
        let snapshot_current = self.index.upgrade().is_some_and(|index| index.is_current());
        if status.status != "running" && snapshot_current {
            return;
        }
        if status.status == terminal && status.groups_count == 0 {
            return;
        }
        status.status = terminal.into();
        status.phase = terminal.into();
        status.groups_revision += 1;
        status.groups_count = 0;
        if let Some(message) = message {
            status.message = Some(message);
            status.errors += 1;
        }
        if publish_scope {
            if let Some(index) = self.index.upgrade().filter(|index| index.is_current()) {
                index.set_duplicate_results(
                    &status.kind,
                    tagrex_commands::query::DuplicateResults {
                        status: terminal.into(),
                        groups_revision: status.groups_revision,
                        skipped: status.skipped,
                        errors: status.errors,
                        ..Default::default()
                    },
                );
            }
        }
    }

    fn finish(&self, mut results: tagrex_commands::query::DuplicateResults) {
        let Some(index) = self.index.upgrade() else {
            self.clear("cancelled", None, false);
            return;
        };
        if self.cancel.load(Ordering::Acquire) || !index.is_current() {
            self.clear("cancelled", None, false);
            return;
        }
        let Ok(mut status) = self.status.lock() else {
            return;
        };
        if status.status != "running" {
            return;
        }
        status.status = "complete".into();
        status.phase = "complete".into();
        status.processed = status.total;
        status.skipped = results.skipped;
        status.errors = results.errors;
        status.groups_revision += 1;
        status.groups_count = results.groups.len();
        results.groups_revision = status.groups_revision;
        index.set_duplicate_results(&status.kind, results);
    }
}

fn current_duplicate_job(
    jobs: &std::collections::BTreeMap<u64, Arc<DuplicateJob>>,
    id: u64,
    job: &Arc<DuplicateJob>,
) -> bool {
    DUPLICATE_JOB_ID.load(Ordering::Acquire) == id + 1
        && jobs
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, job))
}

// Synthetic benchmark snapshots are isolated from the real App and journal.
// These commands do not exist in release builds and never probe fixture paths.
#[cfg(debug_assertions)]
static BENCHMARK_INDICES: std::sync::LazyLock<
    Mutex<std::collections::BTreeMap<u64, Arc<tagrex_commands::query::QueryIndex>>>,
> = std::sync::LazyLock::new(Mutex::default);

#[cfg(debug_assertions)]
#[tauri::command]
async fn library_benchmark_create(tracks: Vec<TrackDto>) -> Result<serde_json::Value, ErrorDto> {
    if tracks.len() > 100_000 {
        return Err(shell_error(
            "Synthetic benchmarks support at most 100,000 rows.",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let started=std::time::Instant::now();let index=Arc::new(tagrex_commands::query::QueryIndex::from_tracks(&tracks));let id=index.id;
        let mut indices=BENCHMARK_INDICES.lock().map_err(shell_error)?;
        while indices.len()>=2 {if let Some((_,old))=indices.pop_first(){old.invalidate();}}
        indices.insert(id,index);
        Ok(serde_json::json!({"benchmarkId":id,"snapshotId":id,"total":tracks.len(),"indexMs":started.elapsed().as_secs_f64()*1000.0}))
    }).await.map_err(shell_error)?
}

#[cfg(debug_assertions)]
#[tauri::command]
async fn library_benchmark_query(
    benchmark_id: u64,
    request: tagrex_commands::query::QueryRequest,
) -> Result<tagrex_commands::query::QueryResult, ErrorDto> {
    let index = BENCHMARK_INDICES
        .lock()
        .map_err(shell_error)?
        .get(&benchmark_id)
        .cloned()
        .ok_or_else(|| shell_error("Synthetic benchmark no longer exists."))?;
    tauri::async_runtime::spawn_blocking(move || {
        index.query(&request, || false).map_err(ErrorDto::from)
    })
    .await
    .map_err(shell_error)?
}

#[cfg(debug_assertions)]
#[tauri::command]
fn library_benchmark_dispose(benchmark_id: u64) -> Result<bool, ErrorDto> {
    let removed = BENCHMARK_INDICES
        .lock()
        .map_err(shell_error)?
        .remove(&benchmark_id);
    if let Some(index) = &removed {
        index.invalidate();
    }
    Ok(removed.is_some())
}
static LIBRARY_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SETTINGS_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The provider side (#166): one per process, alive whether or not a library is
/// open. Each source has one persistent hub and queue, preserving its throttle
/// cadence without making unrelated sources wait for each other's network I/O.
type ProviderState = Arc<ProviderQueues>;

struct SourceQueue {
    hub: Mutex<ProviderHub>,
    jobs: blocking::Queue,
    settings: Arc<Mutex<SettingsDto>>,
}

struct ProviderQueues {
    sources: std::collections::HashMap<&'static str, Arc<SourceQueue>>,
    settings: Arc<Mutex<SettingsDto>>,
}

impl Default for ProviderQueues {
    fn default() -> Self {
        let settings = Arc::new(Mutex::new(SettingsDto::default()));
        Self {
            sources: ["discogs", "musicbrainz", "beatport", "bandcamp", "soundeo"]
                .into_iter()
                .map(|source| {
                    (
                        source,
                        Arc::new(SourceQueue {
                            hub: Mutex::new(ProviderHub::default()),
                            jobs: blocking::Queue::new(),
                            settings: settings.clone(),
                        }),
                    )
                })
                .collect(),
            settings,
        }
    }
}

impl ProviderQueues {
    fn source(&self, source: &str) -> Result<Arc<SourceQueue>, ErrorDto> {
        self.sources
            .get(source)
            .cloned()
            .ok_or_else(|| shell_error(format!("unknown source: {source}")))
    }

    fn apply_settings(&self, settings: &SettingsDto) -> Result<(), ErrorDto> {
        // Do not wait for in-flight HTTP while saving preferences. Each next
        // source job adopts this snapshot without resetting request cadence.
        *self.settings.lock().map_err(shell_error)? = settings.clone();
        Ok(())
    }
}

/// Paths the OS handed the app, waiting for the frontend to come and take them
/// (#51). Opening one is the frontend's job — it is the same "open these paths"
/// the drop handler already does — but the hand-over can arrive before there is
/// a webview to tell, so it queues here instead.
type PendingOpen = Mutex<Vec<PathBuf>>;

/// The nudge that says [`PendingOpen`] is not empty. Deliberately payload-free:
/// the paths travel in the queue and the frontend drains it, so an event that
/// fires before the listener exists costs nothing — the startup drain picks the
/// same paths up.
const OPEN_PATHS_EVENT: &str = "tagrex://open-paths";
/// Emitted while an apply runs (#437): `{ done, total, path }`.
const APPLY_PROGRESS_EVENT: &str = "tagrex://apply-progress";
/// The apply is reported at most this often; the files of a fast local apply go
/// by faster than a window can repaint.
const APPLY_PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(60);
/// Emitted while BPM detection runs (#536): `{ done, total, path }`, before each
/// file and once at the end.
const BPM_PROGRESS_EVENT: &str = "tagrex://bpm-progress";
/// Emitted while an update downloads (#602): `{ downloaded, total }` in bytes,
/// `total` null when the server does not say.
const UPDATE_PROGRESS_EVENT: &str = "tagrex://update-progress";

/// The paths in a command line. Windows and Linux hand a folder over as an
/// argument; macOS does not, and passes `-psn_…` of its own when Finder starts
/// the app, so anything that is not a path that exists is not ours.
fn paths_from_args<I>(args: I) -> Vec<PathBuf>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    args.into_iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect()
}

/// Queue `paths` for the frontend and tell it they are there.
fn hand_over_paths(app: &tauri::AppHandle, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    app.state::<PendingOpen>().lock().unwrap().extend(paths);
    // A window that isn't listening yet is not a problem: what matters is in
    // the queue, and the frontend drains it on startup as well.
    let _ = tauri::Emitter::emit(app, OPEN_PATHS_EVENT, ());
}

/// A failure raised by the shell rather than by the app — Tauri itself, a
/// background task that panicked, an argument that never reaches the command
/// layer. There is no error type of ours behind it and so no code to give it
/// (#268); the interface shows the text.
fn shell_error(error: impl std::fmt::Display) -> ErrorDto {
    ErrorDto::plain(error.to_string())
}

fn with_app<T>(
    state: &AppState,
    f: impl FnOnce(&App) -> Result<T, ErrorDto>,
) -> Result<T, ErrorDto> {
    let guard = state.lock().map_err(shell_error)?;
    let app = guard
        .as_ref()
        .ok_or_else(|| ErrorDto::from(AppError::NoLibraryOpen))?;
    f(app)
}

fn with_providers<T>(
    state: &SourceQueue,
    f: impl FnOnce(&ProviderHub) -> Result<T, ErrorDto>,
) -> Result<T, ErrorDto> {
    let hub = state.hub.lock().map_err(shell_error)?;
    let settings = state.settings.lock().map_err(shell_error)?.clone();
    hub.apply_settings(&settings);
    f(&hub)
}

fn with_app_mut<T>(
    state: &AppState,
    f: impl FnOnce(&mut App) -> Result<T, ErrorDto>,
) -> Result<T, ErrorDto> {
    let mut guard = state.lock().map_err(shell_error)?;
    let app = guard
        .as_mut()
        .ok_or_else(|| ErrorDto::from(AppError::NoLibraryOpen))?;
    f(app)
}

#[tauri::command]
async fn open_library(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    root: String,
) -> Result<(), ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let config_dir = app.path().app_config_dir().map_err(shell_error)?;
            std::fs::create_dir_all(&config_dir).map_err(ErrorDto::from)?;
            let journal_path = config_dir.join("journal.sqlite");
            let opened = App::open(root, &journal_path).map_err(ErrorDto::from)?;
            // Apply saved settings (proxy / rate-limit / ID3 version) to the new session.
            opened.apply_settings(&read_settings_quietly(&app));
            {
                let mut session = state.lock().map_err(shell_error)?;
                *session = Some(opened);
            }
            LIBRARY_GENERATION.fetch_add(1, Ordering::SeqCst);
            // A new library session must not inherit the previous stage.
            mcp_control::reset_shared_stage_for_library(&app)?;
            Ok(())
        })
        .await
}

/// Open a drag-and-drop of `paths` (#127): a lone folder opens as a library,
/// anything else (files, several folders, a mix) as a file-set. Returns the
/// resolved mode + grouping info for the frontend.
#[tauri::command]
async fn open_drop(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<DropResultDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let config_dir = app.path().app_config_dir().map_err(shell_error)?;
            std::fs::create_dir_all(&config_dir).map_err(ErrorDto::from)?;
            let journal_path = config_dir.join("journal.sqlite");
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            let (opened, result) = App::open_drop(paths, &journal_path).map_err(ErrorDto::from)?;
            opened.apply_settings(&read_settings_quietly(&app));
            {
                let mut session = state.lock().map_err(shell_error)?;
                *session = Some(opened);
            }
            LIBRARY_GENERATION.fetch_add(1, Ordering::SeqCst);
            mcp_control::reset_shared_stage_for_library(&app)?;
            Ok(result)
        })
        .await
}

/// Take the paths the OS handed the app, if any (#51).
///
/// Draining rather than reading is what keeps the two sides from opening the
/// same folder twice: the frontend asks once at startup and again on every
/// nudge, and whichever call arrives first is the one that gets them.
#[tauri::command]
fn take_launch_paths(pending: State<PendingOpen>) -> Vec<String> {
    std::mem::take(&mut *pending.lock().unwrap())
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

// Session jobs acquire their queue asynchronously, then run blocking work on
// the blocking pool. No mutex guard crosses an await or blocks the event thread.
#[tauri::command]
async fn list_tracks(state: State<'_, AppState>) -> Result<Vec<TrackDto>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || with_app(&state, |app| app.list_tracks().map_err(ErrorDto::from)))
        .await
}

/// Query workers hold an immutable index, not the session/journal mutex.
#[tauri::command]
async fn library_query(
    state: State<'_, AppState>,
    request: tagrex_commands::query::QueryRequest,
) -> Result<tagrex_commands::query::QueryResult, ErrorDto> {
    let revision = QUERY_REVISION.fetch_add(1, Ordering::SeqCst) + 1;
    let generation = LIBRARY_GENERATION.load(Ordering::SeqCst);
    let state = state.inner().clone();
    let source = state.clone();
    let index = SESSION_QUEUE
        .run(move || with_app(&source, |app| app.query_snapshot().map_err(ErrorDto::from)))
        .await?;
    let snapshot_id = index.id;
    let result = LIBRARY_QUERY_QUEUE
        .run(move || {
            index
                .query(&request, || {
                    QUERY_REVISION.load(Ordering::SeqCst) != revision
                        || LIBRARY_GENERATION.load(Ordering::SeqCst) != generation
                })
                .map_err(ErrorDto::from)
        })
        .await?;
    let current = SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                Ok(app.query_snapshot_id() == Some(snapshot_id))
            })
        })
        .await?;
    if !current
        || QUERY_REVISION.load(Ordering::SeqCst) != revision
        || LIBRARY_GENERATION.load(Ordering::SeqCst) != generation
    {
        return Err(shell_error(
            "Query superseded by a newer request or library revision.",
        ));
    }
    Ok(result)
}

#[tauri::command]
async fn library_suggest(
    state: State<'_, AppState>,
    request: tagrex_commands::query::SuggestRequest,
) -> Result<tagrex_commands::query::SuggestResult, ErrorDto> {
    let revision = SUGGEST_REVISION.fetch_add(1, Ordering::SeqCst) + 1;
    let generation = LIBRARY_GENERATION.load(Ordering::SeqCst);
    let state = state.inner().clone();
    let source = state.clone();
    let index = SESSION_QUEUE
        .run(move || with_app(&source, |app| app.query_snapshot().map_err(ErrorDto::from)))
        .await?;
    let snapshot_id = index.id;
    let result = LIBRARY_SUGGEST_QUEUE
        .run(move || {
            index
                .suggest(&request, || {
                    SUGGEST_REVISION.load(Ordering::SeqCst) != revision
                        || LIBRARY_GENERATION.load(Ordering::SeqCst) != generation
                })
                .map_err(ErrorDto::from)
        })
        .await?;
    let current = SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                Ok(app.query_snapshot_id() == Some(snapshot_id))
            })
        })
        .await?;
    if !current
        || SUGGEST_REVISION.load(Ordering::SeqCst) != revision
        || LIBRARY_GENERATION.load(Ordering::SeqCst) != generation
    {
        return Err(shell_error(
            "Suggestions superseded by a newer request or library revision.",
        ));
    }
    Ok(result)
}

#[tauri::command]
async fn library_duplicates_start(
    state: State<'_, AppState>,
    kind: String,
) -> Result<tagrex_commands::duplicates::DuplicateJobStatus, ErrorDto> {
    if !["exact", "same_audio", "likely"].contains(&kind.as_str()) {
        return Err(shell_error("Unsupported duplicate criterion."));
    }
    let state = state.inner().clone();
    let index = SESSION_QUEUE
        .run(move || with_app(&state, |app| app.query_snapshot().map_err(ErrorDto::from)))
        .await?;
    let (id, status, job) = {
        let mut jobs = DUPLICATE_JOBS.lock().map_err(shell_error)?;
        let id = DUPLICATE_JOB_ID.fetch_add(1, Ordering::SeqCst);
        for previous in jobs.values() {
            previous.clear("cancelled", None, true);
        }
        let status = tagrex_commands::duplicates::DuplicateJobStatus {
            job_id: id,
            snapshot_id: index.id,
            kind: kind.clone(),
            status: "running".into(),
            processed: 0,
            total: index.rows.len(),
            skipped: 0,
            errors: 0,
            groups_revision: 0,
            groups_count: 0,
            phase: "stamping".into(),
            message: None,
        };
        let job = Arc::new(DuplicateJob {
            status: Mutex::new(status.clone()),
            cancel: std::sync::atomic::AtomicBool::new(false),
            index: Arc::downgrade(&index),
        });
        jobs.insert(id, job.clone());
        while jobs.len() > 8 {
            jobs.pop_first();
        }
        index.set_duplicate_results(
            &kind,
            tagrex_commands::query::DuplicateResults {
                status: "running".into(),
                ..Default::default()
            },
        );
        (id, status, job)
    };
    let failed_job = job.clone();
    tauri::async_runtime::spawn(async move {
        let result = DUPLICATE_QUEUE
            .run(move || {
                let cancelled = || job.cancel.load(Ordering::Acquire) || !index.is_current();
                let result = tagrex_commands::duplicates::scan_with_updates(
                    &index,
                    &kind,
                    &DUPLICATE_ANALYSIS,
                    cancelled,
                    |update| {
                        if let Ok(jobs) = DUPLICATE_JOBS.lock() {
                            if current_duplicate_job(&jobs, id, &job) {
                                job.publish(update);
                            }
                        }
                    },
                );
                let jobs = DUPLICATE_JOBS.lock().map_err(shell_error)?;
                let current = current_duplicate_job(&jobs, id, &job);
                match result {
                    Ok(results) if current && !cancelled() => job.finish(results),
                    Ok(_) | Err(tagrex_commands::query::QueryError::Cancelled) => {
                        job.clear("cancelled", None, current)
                    }
                    Err(error) => job.clear("failed", Some(error.to_string()), current),
                }
                Ok(())
            })
            .await;
        if let Err(error) = result {
            if let Ok(jobs) = DUPLICATE_JOBS.lock() {
                let current = current_duplicate_job(&jobs, id, &failed_job);
                failed_job.clear("failed", Some(error.text), current);
            }
        }
    });
    Ok(status)
}

#[tauri::command]
fn library_duplicates_status(
    job_id: u64,
) -> Result<tagrex_commands::duplicates::DuplicateJobStatus, ErrorDto> {
    let jobs = DUPLICATE_JOBS.lock().map_err(shell_error)?;
    let job = jobs
        .get(&job_id)
        .ok_or_else(|| shell_error("Duplicate scan no longer exists."))?;
    if job.index.upgrade().is_none_or(|index| !index.is_current()) {
        job.clear("cancelled", None, false);
    }
    let status = job.status.lock().map_err(shell_error)?.clone();
    Ok(status)
}

#[tauri::command]
fn library_duplicates_cancel(
    job_id: u64,
) -> Result<tagrex_commands::duplicates::DuplicateJobStatus, ErrorDto> {
    let jobs = DUPLICATE_JOBS.lock().map_err(shell_error)?;
    let job = jobs
        .get(&job_id)
        .ok_or_else(|| shell_error("Duplicate scan no longer exists."))?;
    job.clear("cancelled", None, current_duplicate_job(&jobs, job_id, job));
    let status = job.status.lock().map_err(shell_error)?.clone();
    Ok(status)
}

#[tauri::command]
async fn read_track(state: State<'_, AppState>, path: PathBuf) -> Result<TrackDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || with_app(&state, |app| app.read_track(&path).map_err(ErrorDto::from)))
        .await
}

#[tauri::command]
async fn read_track_rows(
    state: State<'_, AppState>,
    paths: Vec<PathBuf>,
) -> Result<Vec<TrackDto>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                app.read_track_rows(&paths).map_err(ErrorDto::from)
            })
        })
        .await
}

/// Move files to the system Trash (#213). Returns what actually went, so the
/// table drops exactly those rows. Refuses the whole call if any path is not a
/// file this session lists inside the open library — nothing is moved then.
#[tauri::command]
async fn trash_files(
    state: State<'_, AppState>,
    paths: Vec<PathBuf>,
) -> Result<Vec<String>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            with_app_mut(&state, |app| {
                app.trash_files(&paths).map_err(ErrorDto::from)
            })
        })
        .await
}

/// Lock a set of fields against change for the rest of the session (#48).
/// Replaces the whole set, so the UI sends what is locked rather than a delta.
#[tauri::command]
async fn set_locked_fields(
    state: State<'_, AppState>,
    fields: Vec<String>,
) -> Result<(), ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                app.set_locked_fields(&fields);
                Ok(())
            })
        })
        .await
}

/// What is locked right now (#48). The UI reads it back on start rather than
/// trusting its own copy — the lock lives in the session, and the window can be
/// reloaded without the session ending.
#[tauri::command]
async fn locked_fields(state: State<'_, AppState>) -> Result<Vec<String>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || with_app(&state, |app| Ok(app.locked_fields())))
        .await
}

#[tauri::command]
async fn find_duplicates(
    state: State<'_, AppState>,
    criterion: String,
) -> Result<Vec<DuplicateGroupDto>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                app.find_duplicates(&criterion).map_err(ErrorDto::from)
            })
        })
        .await
}

/// Build a provider *release* page URL from a hard-coded host plus a
/// charset-validated id (#92). Kept separate from the command so the frontend can
/// only ever reach a Discogs/MusicBrainz release page — never an arbitrary URL —
/// and so the construction is unit-testable without touching the system browser.
fn release_url(source: &str, id: &str) -> Result<String, ErrorDto> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(shell_error(format!("invalid release id: {id:?}")));
    }
    match source {
        "discogs" => Ok(format!("https://www.discogs.com/release/{id}")),
        "musicbrainz" => Ok(format!("https://musicbrainz.org/release/{id}")),
        // The slug is cosmetic; the store resolves a release by its id alone
        // (#162), so nothing user-supplied has to go into the path.
        "beatport" => Ok(format!("https://www.beatport.com/release/-/{id}")),
        other => Err(shell_error(format!("unknown source: {other}"))),
    }
}

fn provider_page_url(source: &str, entity: &str, id: &str) -> Result<String, ErrorDto> {
    if entity == "release" {
        return release_url(source, id);
    }
    if source != "discogs"
        || !matches!(entity, "artist" | "label")
        || id.is_empty()
        || !id.bytes().all(|b| b.is_ascii_digit())
        || id.bytes().all(|b| b == b'0')
    {
        return Err(shell_error("invalid provider page"));
    }
    Ok(format!("https://www.discogs.com/{entity}/{id}"))
}

#[tauri::command]
async fn open_provider_page(source: String, entity: String, id: String) -> Result<(), ErrorDto> {
    BROWSER_QUEUE
        .run(move || open::that(provider_page_url(&source, &entity, &id)?).map_err(ErrorDto::from))
        .await
}

/// Open a provider release page in the system browser (#92).
#[tauri::command]
async fn open_release_page(source: String, id: String) -> Result<(), ErrorDto> {
    BROWSER_QUEUE
        .run(move || {
            let url = release_url(&source, &id)?;
            open::that(url).map_err(ErrorDto::from)
        })
        .await
}

#[tauri::command]
async fn preview_rename(
    state: State<'_, AppState>,
    mask: String,
    paths: Vec<String>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run_latest(&RENAME_PREVIEW_LANE, move |_cancel| {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_rename(&mask, &paths).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn preview_tags_from_name(
    state: State<'_, AppState>,
    mask: String,
    paths: Vec<String>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run_latest(&NAME_PREVIEW_LANE, move |_cancel| {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_tags_from_name(&mask, &paths)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn probe_tags_from_name(
    state: State<'_, AppState>,
    mask: String,
    path: String,
) -> Result<NameProbeDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                app.probe_tags_from_name(&mask, &PathBuf::from(path))
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Every line of a text list under a FROM TEXT mask and the file each track
/// lands on (#550), for the mask builder's list.
#[tauri::command]
async fn probe_text_lines(
    state: State<'_, AppState>,
    mask: String,
    text: String,
    paths: Vec<String>,
) -> Result<TextLinesDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.probe_text_lines(&mask, &text, &paths)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// What a FROM NAME mask reads from each of several files (#548), for the mask
/// builder's list.
#[tauri::command]
async fn probe_tags_from_names(
    state: State<'_, AppState>,
    mask: String,
    paths: Vec<String>,
) -> Result<NameProbesDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.probe_tags_from_names(&mask, &paths)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Tags from a text list (#61): one line per selected file, in order.
#[tauri::command]
async fn preview_tags_from_text(
    state: State<'_, AppState>,
    mask: String,
    text: String,
    paths: Vec<String>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run_latest(&TEXT_PREVIEW_LANE, move |_cancel| {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_tags_from_text(&mask, &text, &paths)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn probe_tags_from_text(
    state: State<'_, AppState>,
    mask: String,
    text: String,
) -> Result<TextProbeDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                app.probe_tags_from_text(&mask, &text)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// A FROM TEXT mask read off the list's own shape (#522), or none when no
/// reading of it fits a line. Stateless: it only looks at the text.
#[tauri::command]
async fn suggest_text_list_mask(text: String) -> Result<Option<String>, ErrorDto> {
    SAMPLE_QUEUE
        .run(move || Ok(tagrex_commands::suggest_text_list_mask(&text)))
        .await
}

/// Read a text list the user picked for the FROM TEXT panel (#61). Stateless,
/// like [`read_cover_image`]: the file is the user's explicit choice.
#[tauri::command]
async fn read_text_list(path: String) -> Result<String, ErrorDto> {
    IMAGE_READ_QUEUE
        .run(move || tagrex_commands::read_text_list(&PathBuf::from(path)).map_err(ErrorDto::from))
        .await
}

#[tauri::command]
async fn preview_transform(
    state: State<'_, AppState>,
    paths: Vec<String>,
    rules: Vec<TransformRuleDto>,
    scope: String,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run_latest(&TRANSFORM_PREVIEW_LANE, move |_cancel| {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_transform(&paths, &rules, &scope)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn preview_move(
    state: State<'_, AppState>,
    mask: String,
    paths: Vec<String>,
    destination: Option<String>,
    copy: bool,
    prune_empty_dirs: bool,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run_latest(&MOVE_PREVIEW_LANE, move |_cancel| {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            let destination = destination
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .map(PathBuf::from);
            with_app(&state, |app| {
                app.preview_move(
                    &mask,
                    &paths,
                    destination.as_deref(),
                    copy,
                    prune_empty_dirs,
                )
                .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Preview native tag recipes against the current working plan.
#[tauri::command]
async fn preview_field_operations(
    state: State<'_, AppState>,
    paths: Vec<String>,
    operations: Vec<tagrex_commands::FieldOperation>,
    working: Option<PlanDto>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run_latest(&FIELD_PREVIEW_LANE, move |cancel| {
            let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
            with_app(&state, |app| {
                app.preview_field_operations_with_cancel(
                    &paths,
                    &operations,
                    working.as_ref(),
                    || cancel.cancelled(),
                )
                .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Native rules for the desktop's independent track, disc and vinyl actions.
#[tauri::command]
async fn preview_numbering_dialog(
    state: State<'_, AppState>,
    paths: Vec<String>,
    groups: Vec<String>,
    options: tagrex_commands::NumberingDialogOptions,
    working: Option<PlanDto>,
) -> Result<tagrex_commands::NumberingDialogPreview, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run_latest(&FIELD_PREVIEW_LANE, move |cancel| {
            let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
            with_app(&state, |app| {
                app.preview_numbering_dialog_with_cancel(
                    &paths,
                    &groups,
                    &options,
                    working.as_ref(),
                    || cancel.cancelled(),
                )
                .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn preview_tag_edits(
    state: State<'_, AppState>,
    edits: Vec<TagEditDto>,
    // The release cover an import brought with it (#207), when there is one.
    cover: Option<CoverArtDto>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                app.preview_tag_edits_with_cover(&edits, cover.as_ref())
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Compose the raw live Editor/import values and checked transformations.
// The singular pair remains accepted for the first live-preview caller; the
// optional assignments preserve artwork belonging to other pending selections.
// Flat legacy and current arguments coexist so older live-preview clients
// remain compatible while manual cover overrides join the same operation.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn preview_live_edits(
    state: State<'_, AppState>,
    edits: Vec<TagEditDto>,
    paths: Vec<String>,
    groups: Vec<ActionGroupDto>,
    cover: Option<CoverArtDto>,
    cover_paths: Vec<String>,
    covers: Option<Vec<LiveCoverAssignmentDto>>,
    base_plan: Option<PlanDto>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                let paths: Vec<PathBuf> = paths.into_iter().map(Into::into).collect();
                let assignments = covers.unwrap_or_else(|| {
                    cover
                        .map(|cover| LiveCoverAssignmentDto {
                            cover,
                            paths: cover_paths,
                        })
                        .into_iter()
                        .collect()
                });
                app.preview_live_edits_with_base(
                    &edits,
                    &paths,
                    &groups,
                    &assignments,
                    base_plan.as_ref(),
                )
                .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Preview replacing every selected file's whole image set (#56) — the one
/// command behind add, remove, reorder and set-the-type.
#[tauri::command]
async fn preview_cover_set(
    state: State<'_, AppState>,
    paths: Vec<String>,
    covers: Vec<CoverArtDto>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_cover_set(&paths, &covers)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn preview_cover_embed(
    state: State<'_, AppState>,
    paths: Vec<String>,
    cover: CoverArtDto,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_cover_embed(&paths, &cover)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Read and resize the chosen image without holding the shared session.
#[tauri::command]
async fn preview_cover_embed_path(
    state: State<'_, AppState>,
    paths: Vec<String>,
    image_path: String,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let session = state.clone();
    let targets = paths.clone();
    let (options, generation) = SESSION_QUEUE
        .run(move || {
            with_app(&session, |app| {
                let options = app.cover_embed_options(&targets).map_err(ErrorDto::from)?;
                Ok((options, LIBRARY_GENERATION.load(Ordering::SeqCst)))
            })
        })
        .await?;
    let cover = IMAGE_READ_QUEUE
        .run(move || {
            options
                .read_image(&PathBuf::from(image_path))
                .map_err(ErrorDto::from)
        })
        .await?;
    SESSION_QUEUE
        .run(move || preview_prepared_cover_embed(&state, generation, &paths, cover))
        .await
}

fn preview_prepared_cover_embed(
    state: &AppState,
    generation: u64,
    paths: &[PathBuf],
    cover: tagrex_commands::PreparedCoverEmbed,
) -> Result<PlanDto, ErrorDto> {
    if LIBRARY_GENERATION.load(Ordering::SeqCst) != generation {
        return Err(shell_error(
            "The open library changed. Choose the artwork again.",
        ));
    }
    with_app(state, |app| {
        app.preview_prepared_cover_embed(paths, cover)
            .map_err(ErrorDto::from)
    })
}

#[tauri::command]
async fn export_cover(
    state: State<'_, AppState>,
    paths: Vec<String>,
    basename: String,
) -> Result<CoverExportDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.export_cover(&paths, &basename).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn read_cover_summary(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<CoverSummaryDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.read_cover_summary(&paths).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn read_external_cover(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<Option<CoverArtDto>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.read_external_cover(&paths).map_err(ErrorDto::from)
            })
        })
        .await
}

/// Read an image file (dropped onto the cover well, #133) into a cover DTO for
/// [`preview_cover_embed`]. Stateless — the source image isn't confined to the
/// library, matching the file picker (the user chose it explicitly).
#[tauri::command]
async fn artwork_import_sources(
    state: State<'_, AppState>,
    paths: Vec<String>,
    pattern: String,
    first_image: bool,
    replace_existing: bool,
) -> Result<tagrex_commands::ArtworkImportSourcesDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
                app.artwork_import_sources(&paths, &pattern, first_image, replace_existing)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn read_cover_image(path: String) -> Result<CoverArtDto, ErrorDto> {
    IMAGE_READ_QUEUE
        .run(move || {
            tagrex_commands::read_cover_image(&PathBuf::from(path)).map_err(ErrorDto::from)
        })
        .await
}

#[tauri::command]
async fn preview_cover_remove(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_cover_remove(&paths).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn preview_remove_tag_block(
    state: State<'_, AppState>,
    paths: Vec<String>,
    kind: String,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_remove_tag_block(&paths, &kind)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn preview_fix_encoding(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_fix_encoding(&paths).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn tag_block_targets(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<BlockTargetsDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.tag_block_targets(&paths).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn preview_convert_tag_block(
    state: State<'_, AppState>,
    paths: Vec<String>,
    from: String,
    to: String,
    revision: Option<String>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_convert_tag_block(&paths, &from, &to, revision.as_deref())
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn preview_clear_tags(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_clear_tags(&paths).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn export_playlist(
    state: State<'_, AppState>,
    paths: Vec<String>,
    file_name: String,
) -> Result<String, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.export_playlist(&paths, &file_name)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// One playlist per folder or per album (#62). Returns every file written.
#[tauri::command]
async fn export_playlists(
    state: State<'_, AppState>,
    paths: Vec<String>,
    grouping: String,
    name_mask: String,
) -> Result<Vec<String>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.export_playlists(&paths, &grouping, &name_mask)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn export_cue(
    state: State<'_, AppState>,
    paths: Vec<String>,
    file_name: String,
) -> Result<String, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.export_cue(&paths, &file_name).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn export_csv(
    state: State<'_, AppState>,
    paths: Vec<String>,
    file_name: String,
) -> Result<String, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.export_csv(&paths, &file_name).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn export_report(
    state: State<'_, AppState>,
    paths: Vec<String>,
    mask: String,
    file_name: String,
) -> Result<String, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.export_report(&paths, &mask, &file_name)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn export_html(
    state: State<'_, AppState>,
    paths: Vec<String>,
    file_name: String,
) -> Result<String, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.export_html(&paths, &file_name).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn export_xml(
    state: State<'_, AppState>,
    paths: Vec<String>,
    file_name: String,
) -> Result<String, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.export_xml(&paths, &file_name).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
fn player_play(player: State<Player>, path: String) -> Result<(), ErrorDto> {
    player.play(PathBuf::from(path)).map_err(shell_error)
}

#[tauri::command]
fn player_set_next(player: State<Player>, path: String) -> Result<(), ErrorDto> {
    player.set_next(PathBuf::from(path)).map_err(shell_error)
}

#[tauri::command]
fn player_pause(player: State<Player>) -> Result<(), ErrorDto> {
    player.pause().map_err(shell_error)
}

#[tauri::command]
fn player_resume(player: State<Player>) -> Result<(), ErrorDto> {
    player.resume().map_err(shell_error)
}

#[tauri::command]
fn player_stop(player: State<Player>) -> Result<(), ErrorDto> {
    player.stop().map_err(shell_error)
}

#[tauri::command]
fn player_seek(player: State<Player>, secs: f64) -> Result<(), ErrorDto> {
    player.seek(secs).map_err(shell_error)
}

#[tauri::command]
fn player_set_volume(player: State<Player>, level: f64) -> Result<(), ErrorDto> {
    // The slider is 0..1; a finite value beyond it is a request for quiet/loud
    // rather than an error, so clamp before narrowing. Doing it in `f64` keeps a
    // huge value from overflowing to an infinity the player would refuse, and a
    // NaN still has no meaning and is still refused.
    player
        .set_volume(level.clamp(0.0, 1.0) as f32)
        .map_err(shell_error)
}

#[tauri::command]
fn player_status(player: State<Player>) -> PlayerStatus {
    player.status()
}

/// What the preview player is doing, for the system's Now Playing (#218).
#[tauri::command]
async fn media_session_update(app: tauri::AppHandle, now: media_session::NowPlaying) {
    media_session::update(app, now).await;
}

/// The callback that turns the executor's progress into events (#437, #448),
/// Step-start/preflight events are throttled. Every successful write must reach
/// the window so value-only row updates cannot skip files in a fast batch.
fn progress_reporter(
    app_handle: tauri::AppHandle,
    write_after: usize,
) -> impl FnMut(&tagrex_core::plan::ApplyProgress) {
    let mut last = std::time::Instant::now() - APPLY_PROGRESS_INTERVAL;
    move |progress| {
        if progress.completed.is_none()
            && progress.done < write_after
            && progress.done != progress.total
            && last.elapsed() < APPLY_PROGRESS_INTERVAL
        {
            return;
        }
        last = std::time::Instant::now();
        let _ = tauri::Emitter::emit(
            &app_handle,
            APPLY_PROGRESS_EVENT,
            ApplyProgressDto {
                done: progress.done,
                total: progress.total,
                path: progress.path.to_string_lossy().into_owned(),
                completed: progress.completed.map(|kind| match kind {
                    tagrex_core::plan::ApplyStep::Tags => "tags",
                    tagrex_core::plan::ApplyStep::Transfer => "transfer",
                }),
            },
        );
    }
}

/// Asks the running apply to stop (#437). Its own state rather than part of
/// [`AppState`]: the apply holds that lock for its whole run, and the request has
/// to get through while it does.
#[derive(Default)]
struct ApplyControl(operation_control::OperationControl);

#[derive(serde::Serialize, Clone)]
struct ApplyProgressDto {
    done: usize,
    total: usize,
    path: String,
    completed: Option<&'static str>,
}

// Both session lock waiting and Apply work stay off the event/runtime threads.
// Cancellation uses independent atomic state and bypasses the session queue.
#[tauri::command]
async fn apply_plan(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    control: State<'_, ApplyControl>,
    plan: PlanDto,
    stage_revision: Option<u64>,
) -> Result<BatchDto, ErrorDto> {
    let state = state.inner().clone();
    let operation = control.0.admit("Apply")?;
    SESSION_QUEUE
        .run(move || {
            let (batch, plan, admitted_revision) = with_app_mut(&state, |app| {
                let (plan, revision) =
                    mcp_control::resolve_desktop_apply(&app_handle, plan, stage_revision)?;
                let mut report = progress_reporter(app_handle.clone(), plan.changes.len());
                let batch = app
                    .apply_with(&plan, &mut report, operation.cancelled())
                    .map_err(ErrorDto::from)?;
                Ok((batch, plan, revision))
            })?;
            if let Err(error) =
                mcp_control::complete_desktop_apply(&app_handle, &plan, admitted_revision)
            {
                // The journaled write already succeeded. A staging refresh
                // failure must never turn it into a failed/automatically
                // retryable write in the desktop.
                let _ = app_handle.emit(
                    "tagrex://agent-event",
                    serde_json::json!({
                        "kind":"shared_stage_refresh_failed", "data":{"message":error.text},
                    }),
                );
            }
            Ok(batch)
        })
        .await
}

#[tauri::command]
async fn quick_write(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    control: State<'_, ApplyControl>,
    plan: PlanDto,
    staged_plan: Option<PlanDto>,
) -> Result<QuickWriteDto, ErrorDto> {
    let state = state.inner().clone();
    let operation = control.0.admit("Write")?;
    SESSION_QUEUE
        .run(move || {
            let mut report = progress_reporter(app_handle, plan.changes.len());
            with_app_mut(&state, |app| {
                app.quick_write_with(
                    &plan,
                    staged_plan.as_ref(),
                    &mut report,
                    operation.cancelled(),
                )
                .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
fn cancel_apply(control: State<ApplyControl>) {
    control.0.cancel();
}

/// Asks a running BPM detection to stop (#536). Shared with the blocking thread
/// the detection runs on, hence the `Arc`.
#[derive(Default)]
struct BpmControl(operation_control::OperationControl);

/// What a BPM detection found (#536): the plan for the tempos it measured, and
/// what it could not measure, for the toast.
#[derive(serde::Serialize)]
struct BpmDetectionDto {
    plan: PlanDto,
    /// Files gone through, whether measured, skipped or failed.
    analysed: usize,
    /// Files with no steady beat to measure.
    no_tempo: usize,
    /// Files passed over as longer than the limit in settings (#538).
    too_long: usize,
    /// Files that could not be decoded, by path.
    failed: Vec<String>,
    /// The run was cancelled; the plan holds the files measured before that.
    cancelled: bool,
}

fn preview_detected_bpm(
    state: &AppState,
    generation: u64,
    detected: &[(PathBuf, f64)],
) -> Result<PlanDto, ErrorDto> {
    if LIBRARY_GENERATION.load(Ordering::SeqCst) != generation {
        return Err(shell_error(
            "Library changed while detecting BPM; outdated results discarded",
        ));
    }
    with_app(state, |app| {
        app.preview_bpm(detected).map_err(ErrorDto::from)
    })
}

/// Measure the tempo of each file and stage the BPM field (#536).
///
/// The measuring runs on a blocking thread and outside the session lock: it
/// takes seconds a file, and the window has to stay usable while it does. Only
/// building the plan from the results takes the lock, for as long as reading
/// the current values takes. A cancel stops at the current file and stages what
/// was measured before it.
#[tauri::command]
async fn detect_bpm(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    control: State<'_, BpmControl>,
    paths: Vec<String>,
) -> Result<BpmDetectionDto, ErrorDto> {
    let operation = control.0.admit("BPM detection")?;
    let preparation = operation.clone();
    let session = state.inner().clone();
    let (limit, generation) = SESSION_QUEUE
        .run(move || {
            let _operation = preparation;
            with_app(&session, |app| {
                Ok((
                    app.bpm_max_length(),
                    LIBRARY_GENERATION.load(Ordering::SeqCst),
                ))
            })
        })
        .await?;
    let measurement = operation.clone();
    let measured = tauri::async_runtime::spawn_blocking(move || {
        let mut detector = tagrex_player::tempo::TempoDetector::new().map_err(shell_error)?;
        let total = paths.len();
        let report = |done: usize, path: &str| {
            let _ = tauri::Emitter::emit(
                &app_handle,
                BPM_PROGRESS_EVENT,
                ApplyProgressDto {
                    done,
                    total,
                    path: path.to_string(),
                    completed: None,
                },
            );
        };
        let (mut detected, mut no_tempo, mut failed) = (Vec::new(), 0, Vec::new());
        let (mut too_long, mut cancelled) = (0, false);
        for (done, path) in paths.iter().enumerate() {
            report(done, path);
            let target = PathBuf::from(path);
            if tagrex_commands::too_long_for_bpm(&target, limit) {
                too_long += 1;
                continue;
            }
            match detector.detect(&target, measurement.cancelled()) {
                Ok(Some(bpm)) => detected.push((target, bpm)),
                Ok(None) => no_tempo += 1,
                Err(tagrex_player::tempo::TempoError::Failed(_)) => failed.push(path.clone()),
                Err(tagrex_player::tempo::TempoError::Cancelled) => {
                    cancelled = true;
                    break;
                }
            }
        }
        let analysed = detected.len() + no_tempo + failed.len() + too_long;
        report(analysed, "");
        Ok::<_, ErrorDto>((detected, analysed, no_tempo, too_long, failed, cancelled))
    })
    .await
    .map_err(shell_error)??;
    let (detected, analysed, no_tempo, too_long, failed, cancelled) = measured;
    let session = state.inner().clone();
    let plan = SESSION_QUEUE
        .run(move || {
            let _operation = operation;
            preview_detected_bpm(&session, generation, &detected)
        })
        .await?;
    Ok(BpmDetectionDto {
        plan,
        analysed,
        no_tempo,
        too_long,
        failed,
        cancelled,
    })
}

#[tauri::command]
fn cancel_detect_bpm(control: State<BpmControl>) {
    control.0.cancel();
}

// `async` for the same reason as `apply_plan` (#448): a synchronous command runs
// on the main thread, and rolling back a big batch took seconds with the window
// frozen. Progress goes out as the same events; there is no cancel, because a
// rollback stopped half-way would leave the batch partly undone.
#[tauri::command]
async fn undo(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    batch_id: i64,
    mode: Option<String>,
) -> Result<UndoReportDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let mode = match mode.as_deref() {
                Some("keep_changed") => UndoMode::KeepChanged,
                Some("overwrite") => UndoMode::Overwrite,
                _ => UndoMode::Strict,
            };
            let mut report = progress_reporter(app_handle, usize::MAX);
            with_app_mut(&state, |app| {
                app.undo_with_mode(batch_id, mode, &mut report)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Redo shares Undo's serialized mutation queue and confirmed progress.
#[tauri::command]
async fn redo(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<BatchDto>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let mut report = progress_reporter(app_handle, usize::MAX);
            with_app_mut(&state, |app| {
                app.redo_with(&mut report).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn redo_count(state: State<'_, AppState>) -> Result<usize, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || with_app(&state, |app| Ok(app.redo_count())))
        .await
}

// `async` so it runs off the main thread (#449): the interface asks for the
// history before and after every apply and undo, and a synchronous command
// would hold the window for as long as the journal takes to read.
#[tauri::command]
async fn take_journal_notice(state: State<'_, AppState>) -> Result<Option<String>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || with_app(&state, |app| Ok(app.take_journal_notice())))
        .await
}

#[tauri::command]
async fn history(state: State<'_, AppState>) -> Result<Vec<BatchDto>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || with_app(&state, |app| app.history().map_err(ErrorDto::from)))
        .await
}

// Provider requests queue per source before entering the blocking pool.
// Blocking HTTP/throttle waits cannot occupy the event or async runtime thread,
// and a slow source cannot prevent another source from answering. The token
// is ignored by token-less providers.
//
// They take the provider state, NOT the library (#166): looking a release up is
// something the user does before choosing files, so requiring an open folder
// only produced a "no library open" refusal for a search that needs no folder
// at all.
#[tauri::command]
async fn provider_search(
    providers: State<'_, ProviderState>,
    source: String,
    token: String,
    query: SearchQueryDto,
) -> Result<Vec<CandidateDto>, ErrorDto> {
    let providers = providers.source(&source)?;
    let hub = providers.clone();
    providers
        .jobs
        .run(move || {
            with_providers(&hub, |hub| {
                hub.provider_search(&source, &token, &query)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn provider_fetch_release(
    providers: State<'_, ProviderState>,
    source: String,
    token: String,
    release_id: String,
) -> Result<ReleaseDto, ErrorDto> {
    let providers = providers.source(&source)?;
    let hub = providers.clone();
    providers
        .jobs
        .run(move || {
            with_providers(&hub, |hub| {
                hub.provider_fetch_release(&source, &token, &release_id)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn provider_fetch_image(
    providers: State<'_, ProviderState>,
    source: String,
    token: String,
    url: String,
) -> Result<CoverArtDto, ErrorDto> {
    let providers = providers.source(&source)?;
    let hub = providers.clone();
    let fetch = providers
        .jobs
        .run(move || {
            with_providers(&hub, |hub| {
                hub.image_fetch(&source, &token).map_err(ErrorDto::from)
            })
        })
        .await?;
    // Admit before spawn_blocking so waiting thumbnails use no blocking worker.
    // Keep the command crate's synchronous cap for non-IPC consumers too.
    IMAGE_ADMISSION
        .run(move || fetch.fetch_cover(&url).map_err(ErrorDto::from))
        .await
}

/// Save a release's images to disk next to the selected tracks (#102). Async:
/// it downloads each image with the provider's auth/User-Agent headers.
#[tauri::command]
async fn save_release_images(
    state: State<'_, AppState>,
    providers: State<'_, ProviderState>,
    source: String,
    token: String,
    path: String,
    urls: Vec<String>,
    overwrite: bool,
) -> Result<SaveImagesDto, ErrorDto> {
    let state = state.inner().clone();
    let path = PathBuf::from(path);
    let target = path.clone();
    let session = state.clone();
    let generation = SESSION_QUEUE
        .run(move || {
            with_app(&session, |app| {
                app.validate_image_target(&target).map_err(ErrorDto::from)?;
                Ok(LIBRARY_GENERATION.load(Ordering::SeqCst))
            })
        })
        .await?;
    let provider = providers.source(&source)?;
    let hub = provider.clone();
    let images = provider
        .jobs
        .run(move || {
            with_providers(&hub, |hub| {
                urls.iter()
                    .map(|url| {
                        hub.fetch_image_bytes(&source, &token, url)
                            .map_err(ErrorDto::from)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
        })
        .await?;
    SESSION_QUEUE
        .run(move || {
            with_app(&state, |app| {
                if LIBRARY_GENERATION.load(Ordering::SeqCst) != generation {
                    return Err(shell_error(
                        "The open library changed. Save the artwork again.",
                    ));
                }
                app.save_fetched_release_images(&path, &images, overwrite)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn check_import_order(
    state: State<'_, AppState>,
    paths: Vec<String>,
    tracks: Vec<ImportTrackDto>,
    release_tracks: Vec<ImportTrackDto>,
) -> Result<Option<tagrex_commands::ImportOrderProposalDto>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.check_import_order(&paths, &tracks, &release_tracks)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn auto_align(
    state: State<'_, AppState>,
    paths: Vec<String>,
    tracks: Vec<ImportTrackDto>,
) -> Result<Vec<Option<AlignMatchDto>>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.auto_align(&paths, &tracks).map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn preview_import(
    state: State<'_, AppState>,
    paths: Vec<String>,
    selection: ImportSelectionDto,
    vinyl_sides_to_disc: bool,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_import(&paths, &selection, vinyl_sides_to_disc)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Path to the locally saved Discogs token (in the OS app-config dir, never in
/// the repo). Convenience only, so the token isn't retyped each session.
fn token_path(app: &tauri::AppHandle) -> Result<PathBuf, ErrorDto> {
    let dir = app.path().app_config_dir().map_err(shell_error)?;
    Ok(dir.join("discogs_token"))
}

#[tauri::command]
async fn saved_discogs_token(app: tauri::AppHandle) -> Result<String, ErrorDto> {
    SETTINGS_QUEUE
        .run(move || {
            let path = token_path(&app)?;
            tighten_secret(&path);
            Ok(std::fs::read_to_string(path)
                .unwrap_or_default()
                .trim()
                .to_string())
        })
        .await
}

#[tauri::command]
async fn save_discogs_token(app: tauri::AppHandle, token: String) -> Result<(), ErrorDto> {
    SETTINGS_QUEUE
        .run(move || {
            let path = token_path(&app)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(ErrorDto::from)?;
            }
            write_secret(&path, token.trim()).map_err(ErrorDto::from)
        })
        .await
}

/// Write `contents` to `path` as a file only its owner can read: the Discogs
/// token and the Beatport session are credentials, and a plain write left them
/// readable by every account on the machine under the usual umask (0644).
/// The mode is set on the open handle as well, so a file an earlier version
/// wrote is tightened on the next save. On Windows the per-user app-config
/// folder is already private to its owner.
fn write_secret(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(contents.as_bytes())?;
    file.flush()
}

/// Make a credential file an earlier version left readable by others readable
/// by its owner alone (#675), when it is read: [`write_secret`] only reaches a
/// file that is saved again, and a token is saved once and read every session.
/// Best effort; a file that is missing or cannot be changed is left as it is.
fn tighten_secret(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.permissions().mode() & 0o077 != 0 {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

// ---- Beatport sign-in (#162) ----------------------------------------------
//
// The store has no self-serve developer tier, so the only way in is OAuth
// against the user's own account, through the public client its documentation
// page uses. TagRex never sees the password: the sign-in happens in a window
// pointed at Beatport's own login page, and all that comes back is the
// authorization code in the redirect URL.
//
// What is stored is what a session needs and nothing more: the tokens, when the
// access one dies, the account name (so settings can say *who* is signed in),
// and the client id (so a refresh doesn't have to re-read it). It lives in the
// OS app-config dir next to the Discogs token — never in the repository.

static BEATPORT_SESSION: std::sync::LazyLock<Arc<beatport_session_control::SessionControl>> =
    std::sync::LazyLock::new(|| Arc::new(beatport_session_control::SessionControl::default()));

/// The saved sign-in, as it sits on disk.
#[derive(serde::Serialize, serde::Deserialize)]
struct BeatportSession {
    #[serde(flatten)]
    token: tagrex_providers_beatport::auth::BeatportToken,
    username: String,
    client_id: String,
}

/// What the UI shows in the source row and in settings.
#[derive(serde::Serialize)]
struct BeatportStatusDto {
    authorized: bool,
    username: String,
}

fn beatport_session_path(app: &tauri::AppHandle) -> Result<PathBuf, ErrorDto> {
    let dir = app.path().app_config_dir().map_err(shell_error)?;
    Ok(dir.join("beatport_session.json"))
}

fn read_beatport_session(app: &tauri::AppHandle) -> Option<BeatportSession> {
    let path = beatport_session_path(app).ok()?;
    tighten_secret(&path);
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

fn write_beatport_session(
    app: &tauri::AppHandle,
    session: &BeatportSession,
) -> Result<(), ErrorDto> {
    let path = beatport_session_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(ErrorDto::from)?;
    }
    let json = serde_json::to_string(session).map_err(ErrorDto::from)?;
    write_secret(&path, &json).map_err(ErrorDto::from)
}

/// The proxy every provider request goes through, from settings.
fn beatport_proxy(app: &tauri::AppHandle) -> Option<String> {
    let proxy = read_settings_quietly(app).proxy.trim().to_string();
    (!proxy.is_empty()).then_some(proxy)
}

#[tauri::command]
async fn beatport_status(app: tauri::AppHandle) -> Result<BeatportStatusDto, ErrorDto> {
    SETTINGS_QUEUE
        .run(move || {
            Ok({
                match BEATPORT_SESSION.read(|| Ok(read_beatport_session(&app)))?.1 {
                    Some(session) => BeatportStatusDto {
                        authorized: true,
                        username: session.username,
                    },
                    None => BeatportStatusDto {
                        authorized: false,
                        username: String::new(),
                    },
                }
            })
        })
        .await
}

#[tauri::command]
async fn beatport_logout(app: tauri::AppHandle) -> Result<(), ErrorDto> {
    SETTINGS_QUEUE
        .run(move || {
            BEATPORT_SESSION.logout(|| {
                let path = beatport_session_path(&app)?;
                match std::fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    // Signing out of a session that isn't there is a no-op.
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(err) => Err(ErrorDto::from(err)),
                }
            })
        })
        .await
}

/// Sign in: read the public client id, open Beatport's login page in its own
/// window, wait for the redirect that carries the authorization code, trade it
/// for tokens. Returns the account name.
///
/// `async` for the same reason the provider commands are (#95): every step here
/// blocks on the network or on the user, and a synchronous command would freeze
/// the whole webview while it did.
#[tauri::command]
async fn beatport_login(app: tauri::AppHandle) -> Result<String, ErrorDto> {
    use tagrex_providers_beatport::auth;

    let config = app.clone();
    let (generation, proxy) = SETTINGS_QUEUE
        .run(move || BEATPORT_SESSION.read(|| Ok(beatport_proxy(&config))))
        .await?;
    let agent = auth::agent(proxy.as_deref()).map_err(ErrorDto::from)?;

    let scraper = agent.clone();
    let client_id = tauri::async_runtime::spawn_blocking(move || auth::fetch_client_id(&scraper))
        .await
        .map_err(shell_error)?
        .map_err(ErrorDto::from)?;

    // The window reports back either a code or, if the user gives up and closes
    // it, nothing — hence a channel rather than a return value.
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let on_close = tx.clone();
    let url = auth::authorize_url(&client_id)
        .parse()
        .map_err(|_| shell_error("could not build the Beatport sign-in URL"))?;
    let window = tauri::WebviewWindowBuilder::new(
        &app,
        BEATPORT_LOGIN_WINDOW,
        tauri::WebviewUrl::External(url),
    )
    .title("Sign in to Beatport")
    .inner_size(520.0, 760.0)
    .on_navigation(move |url| {
        if let Some(code) = auth::code_from_redirect(url.as_str()) {
            let _ = tx.send(Some(code));
        }
        true
    })
    .build()
    .map_err(shell_error)?;
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
            let _ = on_close.send(None);
        }
    });

    let received = tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(std::time::Duration::from_secs(BEATPORT_LOGIN_TIMEOUT_SECS))
    })
    .await
    .map_err(shell_error)?;
    // The window has done its job either way; leaving it open would strand a
    // second window with no owner.
    let _ = window.close();

    let code = match received {
        Ok(Some(code)) => code,
        Ok(None) => return Err(shell_error("Sign-in cancelled")),
        Err(_) => return Err(shell_error("Sign-in timed out")),
    };

    let exchange_id = client_id.clone();
    let token = tauri::async_runtime::spawn_blocking(move || {
        auth::exchange_code(&agent, &exchange_id, &code)
    })
    .await
    .map_err(shell_error)?
    .map_err(ErrorDto::from)?;

    let username = beatport_account_name(&proxy, &token.access_token).await;
    let session = BeatportSession {
        token,
        username: username.clone(),
        client_id,
    };
    SETTINGS_QUEUE
        .run(move || {
            BEATPORT_SESSION.publish(generation, || write_beatport_session(&app, &session))
        })
        .await?;
    Ok(username)
}

/// A valid access token for the provider commands, renewed first if the stored
/// one has expired. The frontend passes what this returns straight into
/// `provider_search` and friends, exactly as it passes the Discogs token.
#[tauri::command]
async fn beatport_token(app: tauri::AppHandle) -> Result<String, ErrorDto> {
    use tagrex_providers_beatport::auth;

    BEATPORT_SESSION
        .token(move |control, generation| {
            let (mut session, proxy) = control.read_at(generation, || {
                let session = read_beatport_session(&app)
                    .ok_or_else(|| shell_error("Not signed in to Beatport"))?;
                Ok((session, beatport_proxy(&app)))
            })?;
            if !session.token.is_expired() {
                return Ok(session.token.access_token);
            }
            // Only the short snapshot/publication sections hold the persistence
            // lock. Status, logout and settings can run while HTTP is pending.
            let agent = auth::agent(proxy.as_deref()).map_err(ErrorDto::from)?;
            let token = auth::refresh(&agent, &session.client_id, &session.token.refresh_token)
                .map_err(|err| ErrorDto::from(AppError::BeatportReauth(err.to_string())))?;
            let access_token = token.access_token.clone();
            session.token = token;
            control.publish(generation, || write_beatport_session(&app, &session))?;
            Ok(access_token)
        })
        .await
}

/// The account name behind a token. A failure here is not worth failing the
/// whole sign-in over — the token is what matters, the name is a label — so it
/// degrades to an empty string.
async fn beatport_account_name(proxy: &Option<String>, access_token: &str) -> String {
    use tagrex_providers_beatport::auth;

    let Ok(agent) = auth::agent(proxy.as_deref()) else {
        return String::new();
    };
    let token = access_token.to_string();
    tauri::async_runtime::spawn_blocking(move || auth::account_username(&agent, &token))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
}

/// Label of the sign-in window, so a second Sign in click can't open a duplicate.
const BEATPORT_LOGIN_WINDOW: &str = "beatport-login";
/// How long the sign-in window waits for the user before giving up.
const BEATPORT_LOGIN_TIMEOUT_SECS: u64 = 300;

/// Path to the persisted settings JSON (#79), in the OS app-config dir.
fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, ErrorDto> {
    let dir = app.path().app_config_dir().map_err(shell_error)?;
    Ok(dir.join("settings.json"))
}

/// Read saved settings, falling back to defaults if the file is missing or
/// unusable (so one bad file never blocks startup).
///
/// The three outcomes are not the same, so they are not folded together here:
/// a missing file is a clean start; a file that does not parse is left in place
/// and copied aside by the next save ([`settings_store`]); a file that exists
/// but cannot be *read* is remembered, and saving is refused until a read that
/// feeds the interface succeeds, because the defaults shown instead must not
/// replace bytes nobody has seen.
///
/// Only this (and the start-up read in `main`'s setup) updates that state. The
/// backend also reads settings for its own use — a provider's proxy, an opened
/// library's write preferences — through [`read_settings_quietly`], because a
/// read that never reached the interface's snapshot must not decide whether
/// the interface may save.
fn read_settings(app: &tauri::AppHandle) -> SettingsDto {
    read_settings_recording(app, true)
}

/// Read saved settings for the backend's own use (the proxy a request should
/// go through, the preferences an opened library takes). Same fallback as
/// [`read_settings`], but it does not touch the "last read failed" state: this
/// read did not produce the settings the interface will save.
fn read_settings_quietly(app: &tauri::AppHandle) -> SettingsDto {
    read_settings_recording(app, false)
}

static SETTINGS_NOTICE: Mutex<Option<String>> = Mutex::new(None);

fn read_settings_recording(app: &tauri::AppHandle, record: bool) -> SettingsDto {
    let Ok(path) = settings_path(app) else {
        // Without a config directory there is nothing to read, and a save will
        // fail on the same call rather than overwrite anything.
        return SettingsDto::default();
    };
    match settings_store::read::<SettingsDto>(&path) {
        Ok(Some(settings)) => {
            if record {
                settings_store::note_read_success();
                if let Ok(mut notice) = SETTINGS_NOTICE.lock() {
                    *notice = None;
                }
            }
            settings
        }
        Ok(None) => {
            if record {
                settings_store::note_read_success();
                if let Ok(mut notice) = SETTINGS_NOTICE.lock() {
                    *notice = None;
                }
            }
            SettingsDto::default()
        }
        Err(settings_store::ReadError::Malformed(detail)) => {
            eprintln!(
                "settings: {} is not valid settings JSON ({detail}); it will be kept aside on the next save",
                path.display()
            );
            if record {
                settings_store::note_read_success();
                if let Ok(mut notice) = SETTINGS_NOTICE.lock() {
                    *notice = Some(format!("Saved settings could not be parsed. Defaults are shown. The original file at {} will be preserved before replacement when you save.", path.display()));
                }
            }
            SettingsDto::default()
        }
        Err(settings_store::ReadError::Unreadable(error)) => {
            eprintln!("settings: {} could not be read: {error}", path.display());
            if record {
                settings_store::note_read_failure(error.to_string());
                if let Ok(mut notice) = SETTINGS_NOTICE.lock() {
                    *notice = Some(format!("Saved settings at {} could not be read: {error}. Defaults are shown and saving is blocked until the file can be read. Reopen Settings to retry.", path.display()));
                }
            }
            SettingsDto::default()
        }
    }
}

// Personal-fork releases have their own version line and no update service yet.
const IN_APP_UPDATES: bool = false;

/// Ask GitHub whether a newer release exists (#435). `automatic` is the
/// start-up check: it does nothing at all when the setting is off, so switching
/// it off is enough to keep the app from making the request. The button in
/// Settings asks regardless. `async` because the request blocks.
///
/// A newer release is also looked up in its `latest.json` (#602); when it can be
/// installed from here, it is kept for [`install_update`] and the answer says so.
#[tauri::command]
async fn check_for_update(
    app: tauri::AppHandle,
    pending: State<'_, update::PendingUpdate>,
    automatic: bool,
) -> Result<Option<update::UpdateCheckDto>, ErrorDto> {
    if !IN_APP_UPDATES {
        return Ok(None);
    }
    let handle = app.clone();
    let settings = SETTINGS_QUEUE
        .run(move || Ok(read_settings_quietly(&handle)))
        .await?;
    if automatic && !settings.check_updates {
        return Ok(None);
    }
    let proxy = settings.proxy.clone();
    let mut found = tauri::async_runtime::spawn_blocking(move || update::check(&proxy))
        .await
        .map_err(shell_error)?
        .map_err(|detail| ErrorDto::plain(format!("Could not check for updates: {detail}")))?;
    let ready = if found.newer {
        update::installable(&app, &settings.proxy, &found.latest).await
    } else {
        None
    };
    found.installable = ready.is_some();
    *pending.0.lock().unwrap() = ready;
    Ok(Some(found))
}

/// Download the update the last check found, check its signature against the
/// key built into the app, install it and restart (#602). On Windows the
/// installer takes over and the app quits for it, so this returns there only
/// with an error; elsewhere too, the update kept for another try. In a folder
/// the user cannot write to, Windows is asked for administrator rights first
/// (#699). The frontend refuses to start it while there is anything unapplied.
#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    pending: State<'_, update::PendingUpdate>,
) -> Result<(), ErrorDto> {
    if !IN_APP_UPDATES {
        return Err(ErrorDto::plain(
            "In-app updates are disabled for Tagnetic. Download releases from this project’s GitHub page.",
        ));
    }
    let update = pending
        .0
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| ErrorDto::plain("There is no update to install. Check again."))?;
    let mut downloaded: u64 = 0;
    let mut reported: Option<std::time::Instant> = None;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                if reported.is_some_and(|at| at.elapsed() < APPLY_PROGRESS_INTERVAL) {
                    return;
                }
                reported = Some(std::time::Instant::now());
                let _ = tauri::Emitter::emit(
                    &app,
                    UPDATE_PROGRESS_EVENT,
                    serde_json::json!({ "downloaded": downloaded, "total": total }),
                );
            },
            || {},
        )
        .await
        .map_err(|error| ErrorDto::plain(format!("Could not install the update: {error}")))?;
    #[cfg(windows)]
    if let Some(dir) = update::protected_install_dir() {
        update::run_installer_elevated(&bytes, &update.version, &dir)
            .map_err(|detail| ErrorDto::plain(format!("Could not install the update: {detail}")))?;
        std::process::exit(0);
    }
    update
        .install(&bytes)
        .map_err(|error| ErrorDto::plain(format!("Could not install the update: {error}")))?;
    app.restart()
}

/// Open the releases page in the system browser (#435). No argument: the address
/// is fixed here, so nothing the frontend sends can make this open anything else.
#[tauri::command]
async fn open_releases_page() -> Result<(), ErrorDto> {
    BROWSER_QUEUE
        .run(move || open::that(update::RELEASES_PAGE).map_err(ErrorDto::from))
        .await
}

/// The mask guide, linked from the foot of the placeholder reference (#557).
const MASK_GUIDE: &str = "https://github.com/djhappyman/TAGnetic/blob/main/docs/guide/masks.md";

/// Open the mask guide in the system browser (#557). Fixed address, like the
/// releases page: nothing the frontend sends can make this open anything else.
#[tauri::command]
async fn open_mask_guide() -> Result<(), ErrorDto> {
    BROWSER_QUEUE
        .run(move || open::that(MASK_GUIDE).map_err(ErrorDto::from))
        .await
}

/// The small-word lists a title-case rule can keep lowercase (#537), as
/// `[language, words]` pairs in the core's order, so the rule card offers the
/// same lists the transform applies.
#[tauri::command]
fn small_word_lists() -> Vec<(String, Vec<String>)> {
    tagrex_core::transform::SMALL_WORDS
        .iter()
        .map(|(lang, words)| {
            (
                (*lang).to_string(),
                words.iter().map(|word| (*word).to_string()).collect(),
            )
        })
        .collect()
}

/// Which end of an in-app window's head the close button goes at on this
/// desktop (#565): "start" or "end".
#[tauri::command]
async fn close_button_side() -> Result<&'static str, ErrorDto> {
    BROWSER_QUEUE
        .run(move || Ok(window_controls::close_button_side()))
        .await
}

#[derive(serde::Serialize)]
struct LoadedSettings {
    #[serde(flatten)]
    settings: SettingsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_notice: Option<String>,
    #[serde(rename = "_revision")]
    revision: String,
}

#[tauri::command]
async fn load_settings(app: tauri::AppHandle) -> Result<LoadedSettings, ErrorDto> {
    SETTINGS_QUEUE
        .run(move || {
            let _document = settings_store::DOCUMENT_LOCK.lock().map_err(shell_error)?;
            let settings = read_settings(&app);
            let recovery_notice = SETTINGS_NOTICE.lock().map_err(shell_error)?.take();
            let document = mcp_control::settings_document(&settings_path(&app)?)
                .unwrap_or(serde_json::to_value(&settings).map_err(shell_error)?);
            let revision = tagrex_service::settings_revision(&document);
            Ok(LoadedSettings {
                settings,
                recovery_notice,
                revision,
            })
        })
        .await
}

/// The shipped preset library (#137). Read-only and not part of settings: the
/// frontend lists these below the user's own saved groups.
#[tauri::command]
fn builtin_action_groups() -> Vec<ActionGroupDto> {
    tagrex_commands::builtin_action_groups()
}

/// Run a preset's rules over one sample string, for the preset editor's live
/// example (#394). Needs no library: it is the rules and the text alone.
#[tauri::command]
async fn transform_sample(
    group: ActionGroupDto,
    text: String,
    target: String,
) -> Result<String, ErrorDto> {
    SAMPLE_QUEUE
        .run(move || {
            tagrex_commands::transform_sample(&group, &text, &target).map_err(ErrorDto::from)
        })
        .await
}

/// Every placeholder a mask accepts (#148), for the in-app reference. Needs no
/// open library — the grammar is the same whether or not one is.
#[tauri::command]
fn mask_placeholders() -> Vec<PlaceholderDto> {
    tagrex_commands::mask_placeholders()
}

#[tauri::command]
fn pattern_text_functions() -> Vec<serde_json::Value> {
    tagrex_core::mask::function_presentation::text_functions().into_iter().map(|function|serde_json::json!({"key":function.key,"label":function.label,"description":function.description,"args":function.args})).collect()
}
#[tauri::command]
fn pattern_function_layout(pattern: String) -> Result<Vec<serde_json::Value>, String> {
    Ok(tagrex_core::mask::function_presentation::function_spans(&pattern)?.into_iter().map(|span|serde_json::json!({"start":span.start,"end":span.end,"key":span.key,"args":span.args,"display_args":span.display_args,"raw":span.raw})).collect())
}
#[tauri::command]
fn build_pattern_function(
    key: String,
    args: Vec<String>,
    original: Option<String>,
) -> Result<String, String> {
    tagrex_core::mask::function_presentation::build_text_function(&key, &args, original.as_deref())
}

/// Every field an online import can write (#152), for the setting that picks
/// which of them it may. Read-only, like the placeholder reference.
#[tauri::command]
fn import_fields() -> Vec<ImportFieldDto> {
    tagrex_commands::import_fields()
}

/// Render a mask-defined column over a batch of files (#150).
#[tauri::command]
async fn render_column(
    state: State<'_, AppState>,
    pattern: String,
    paths: Vec<String>,
) -> Result<Vec<String>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.render_column(&pattern, &paths).map_err(ErrorDto::from)
            })
        })
        .await
}

/// Derive all visible custom columns from one snapshot per file.
#[tauri::command]
async fn render_columns(
    state: State<'_, AppState>,
    patterns: Vec<String>,
    paths: Vec<String>,
) -> Result<Vec<Option<Vec<String>>>, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run(move || {
            let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
            with_app(&state, |app| {
                app.render_columns(&patterns, &paths)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Preview the ticked action groups run in order as one plan (#137).
/// Selection-driven previews may read many files, including remote files.
/// Dispatch off the webview event thread, as with preview_tag_edits.
#[tauri::command]
async fn preview_transform_groups(
    state: State<'_, AppState>,
    paths: Vec<String>,
    groups: Vec<ActionGroupDto>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run_latest(&GROUP_PREVIEW_LANE, move |_cancel| {
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            with_app(&state, |app| {
                app.preview_transform_groups(&paths, &groups)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

/// Run the ticked action groups over a staged plan rather than over the files
/// (#142) — cleanup composed into the operation that produced the values.
#[tauri::command]
async fn preview_transform_over_plan(
    state: State<'_, AppState>,
    plan: PlanDto,
    groups: Vec<ActionGroupDto>,
) -> Result<PlanDto, ErrorDto> {
    let state = state.inner().clone();
    SESSION_QUEUE
        .run_latest(&GROUP_PLAN_PREVIEW_LANE, move |_cancel| {
            with_app(&state, |app| {
                app.preview_transform_over_plan(&plan, &groups)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
async fn save_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    providers: State<'_, ProviderState>,
    studio: State<'_, audio_studio::StudioState>,
    settings: SettingsDto,
    expected_revision: Option<String>,
) -> Result<String, ErrorDto> {
    save_settings_at(
        settings_path(&app)?,
        state.inner().clone(),
        providers.inner().clone(),
        Some(studio.inner().clone()),
        settings,
        expected_revision,
    )
    .await
}

async fn save_settings_at(
    path: PathBuf,
    state: AppState,
    providers: ProviderState,
    studio: Option<audio_studio::StudioState>,
    settings: SettingsDto,
    expected_revision: Option<String>,
) -> Result<String, ErrorDto> {
    let (settings, revision, document_revision) = SETTINGS_QUEUE
        .run(move || {
            let _document = settings_store::DOCUMENT_LOCK.lock().map_err(shell_error)?;
            if let Some(expected) = expected_revision {
                let current = match mcp_control::settings_document(&path) {
                    Ok(document) => document,
                    Err(error) => match settings_store::read::<SettingsDto>(&path) {
                        Err(settings_store::ReadError::Malformed(_)) => serde_json::to_value(SettingsDto::default()).map_err(shell_error)?,
                        _ => return Err(error),
                    },
                };
                if tagrex_service::settings_revision(&current) != expected {
                    return Err(shell_error("Settings changed elsewhere. Reopen Settings before saving so those changes are preserved."));
                }
            }
            // Durable replace: a truncated or half-written settings.json would
            // lose every saved chain and preset, so the file is only ever
            // swapped in whole (see [`settings_store`]). A save is refused
            // rather than performed over a file that could not be read.
            let mut document = mcp_control::settings_document(&path)
                .unwrap_or_else(|_| serde_json::json!({}));
            let values = serde_json::to_value(&settings).map_err(shell_error)?;
            if let (Some(document), Some(values)) = (document.as_object_mut(), values.as_object()) {
                document.extend(values.clone());
            }
            settings_store::save_with_schema::<SettingsDto, _>(&path, &document)
                .map_err(|error| ErrorDto::plain(error.to_string()))?;
            // Apply immediately: the network half always (the hub is always there), the
            // library half only when a library is open — so a change takes effect
            // without reopening either way.
            providers.apply_settings(&settings)?;
            if let Some(studio) = studio {
                studio.apply_settings(&settings);
            }
            let revision = SETTINGS_REVISION.fetch_add(1, Ordering::SeqCst) + 1;
            let document_revision = tagrex_service::settings_revision(&document);
            Ok((settings, revision, document_revision))
        })
        .await?;
    // Opening a library and adopting parser/write preferences must have one
    // order. Persist/network snapshots can proceed independently, but must not
    // apply to the old session while a new one is still being constructed.
    SESSION_QUEUE
        .run(move || {
            if SETTINGS_REVISION.load(Ordering::SeqCst) == revision {
                if let Some(app) = state.lock().map_err(shell_error)?.as_ref() {
                    app.apply_settings(&settings);
                }
            }
            Ok(())
        })
        .await?;
    Ok(document_revision)
}

fn main() {
    // MCP stdio is a connector to the desktop owner. It must exit before a
    // WebView, single-instance handover or GUI logging can touch stdout.
    let arguments: Vec<String> = std::env::args_os()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    if let Some(result) = tagrex_mcp::handle_cli(&arguments) {
        if let Err(error) = result {
            eprintln!("TAGnetic MCP: {error}");
            std::process::exit(1);
        }
        return;
    }
    let builder = tauri::Builder::default();

    #[cfg(all(debug_assertions, feature = "dev-mcp"))]
    let builder = builder.plugin(
        tauri_plugin_mcp_bridge::Builder::new()
            .bind_address("127.0.0.1")
            .build(),
    );

    // Windows and Linux start a second process for a second launch; the folder
    // it was given belongs in the window that is already open (#51). macOS
    // routes it to the running instance itself, as the Apple Event below.
    #[cfg(any(windows, target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        // `argv` is the whole command line, the executable included.
        hand_over_paths(
            app,
            paths_from_args(argv.into_iter().skip(1).map(Into::into)),
        );
    }));

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Which document each window may show. Without this hook a webview
        // navigates anywhere, and provider-derived links are rendered in the
        // UI; the policy keeps the app's window on the app's own document.
        .plugin(navigation::init())
        // Size, position and maximized/fullscreen of the main window (#434).
        // Not DECORATIONS or VISIBLE: the overlay title bar and the first show
        // are the app's to decide. Only "main": the Beatport sign-in window is a
        // one-off and must open at its own size every time.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED
                        | tauri_plugin_window_state::StateFlags::FULLSCREEN,
                )
                .with_filter(|label| label == "main")
                .build(),
        )
        .manage(AppState::default())
        .manage(audio_studio::StudioState::default())
        .manage(audio_studio::Openings::default())
        .manage(ProviderState::default())
        .manage(ApplyControl::default())
        .manage(BpmControl::default())
        .manage(update::PendingUpdate::default())
        // What the app was started with, waiting for the frontend to ask (#51).
        .manage(PendingOpen::new(paths_from_args(
            std::env::args_os().skip(1),
        )))
        .setup(|app| {
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                webkit_compositor::install(&window);
            }
            // The provider hub exists from startup, before any library is
            // opened (#166), so it takes the saved network settings here rather
            // than waiting for a library or a trip through Settings › Save.
            let settings = read_settings(&app.handle().clone());
            app.state::<audio_studio::StudioState>().apply_settings(&settings);
            app.state::<ProviderState>()
                .apply_settings(&settings)
                .map_err(|error| std::io::Error::other(error.text))?;
            media_session::install(app.handle());
            if let Err(error) = mcp_control::install(app.handle()) {
                eprintln!("TAGnetic MCP is unavailable: {error}");
                app.manage(mcp_control::AgentUnavailable("Saved agent connections could not be opened safely. Agent access is disabled; your files and the rest of the app remain available.".into()));
            }
            Ok(())
        })
        .manage(Player::new())
        .invoke_handler(tauri::generate_handler![
            audio_studio::audio_split_open,
            audio_studio::audio_split_loading,
            audio_studio::audio_split_cancel_open,
            audio_studio::audio_split_close,
            audio_studio::audio_split_peaks,
            audio_studio::audio_split_transport,
            audio_studio::audio_split_audition_output,
            audio_studio::audio_split_status,
            audio_studio::audio_split_preview,
            audio_studio::audio_split_export,
            audio_studio::audio_split_undo_replacement,
            audio_studio::audio_split_progress,
            audio_studio::audio_split_cancel,
            audio_studio::audio_split_reveal,
            audio_studio::audio_split_can_add,
            audio_studio::audio_split_cue,
            audio_studio::audio_split_write_cue,
            audio_studio::audio_split_discogs,
            mcp_control::mcp_status,
            mcp_control::mcp_agent_status,
            mcp_control::mcp_control_agent,
            mcp_control::mcp_set_enabled,
            mcp_control::mcp_pair_client,
            mcp_control::mcp_revoke_client,
            mcp_control::mcp_stop_all,
            mcp_control::mcp_review_draft,
            mcp_control::mcp_adopt_draft,
            mcp_control::mcp_set_manual_plan,
            mcp_control::mcp_shared_stage,
            mcp_control::mcp_shared_stage_delta,
            mcp_control::mcp_shared_stage_asset,
            mcp_control::mcp_publish_shared_intents,
            mcp_control::mcp_publish_shared_stage,
            mcp_control::mcp_clear_shared_stage,
            mcp_control::mcp_rebase_manual_plan,
            mcp_control::mcp_set_view_preferences,
            mcp_control::mcp_patch_query_presets,
            mcp_control::mcp_cancel_operation,
            mcp_control::mcp_reconcile_operation,
            open_library,
            open_drop,
            list_tracks,
            library_query,
            library_suggest,
            library_duplicates_start,
            library_duplicates_status,
            library_duplicates_cancel,
            #[cfg(debug_assertions)]
            library_benchmark_create,
            #[cfg(debug_assertions)]
            library_benchmark_query,
            #[cfg(debug_assertions)]
            library_benchmark_dispose,
            read_track,
            read_track_rows,
            trash_files,
            set_locked_fields,
            locked_fields,
            find_duplicates,
            open_release_page,
            open_provider_page,
            open_mask_guide,
            close_button_side,
            small_word_lists,
            preview_rename,
            preview_tags_from_name,
            probe_tags_from_name,
            probe_tags_from_names,
            probe_text_lines,
            preview_tags_from_text,
            probe_tags_from_text,
            read_text_list,
            suggest_text_list_mask,
            preview_move,
            preview_transform,
            preview_tag_edits,
            preview_field_operations,
            preview_numbering_dialog,
            preview_live_edits,
            preview_cover_embed,
            preview_cover_embed_path,
            preview_cover_set,
            export_cover,
            read_cover_summary,
            read_external_cover,
            read_cover_image,
            artwork_import_sources,
            preview_cover_remove,
            preview_remove_tag_block,
            preview_convert_tag_block,
            tag_block_targets,
            preview_fix_encoding,
            preview_clear_tags,
            export_playlist,
            export_playlists,
            export_cue,
            export_csv,
            export_report,
            export_html,
            export_xml,
            apply_plan,
            quick_write,
            cancel_apply,
            detect_bpm,
            cancel_detect_bpm,
            undo,
            redo,
            redo_count,
            history,
            take_journal_notice,
            provider_search,
            provider_fetch_release,
            provider_fetch_image,
            save_release_images,
            preview_import,
            auto_align,
            check_import_order,
            saved_discogs_token,
            save_discogs_token,
            beatport_status,
            beatport_login,
            beatport_logout,
            beatport_token,
            load_settings,
            check_for_update,
            install_update,
            open_releases_page,
            save_settings,
            player_play,
            player_set_next,
            player_pause,
            player_resume,
            player_stop,
            player_seek,
            player_set_volume,
            builtin_action_groups,
            transform_sample,
            mask_placeholders,
            pattern_text_functions,
            pattern_function_layout,
            build_pattern_function,
            import_fields,
            render_column,
            render_columns,
            preview_transform_groups,
            preview_transform_over_plan,
            player_status,
            media_session_update,
            take_launch_paths
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            // macOS does not pass an opened folder as an argument. A double
            // click, an "Open With", a folder dropped on the Dock icon and a
            // second `open` of the bundle all arrive here, as one Apple Event
            // delivered to the running instance (#51).
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = _event {
                let paths = urls
                    .iter()
                    .filter_map(|url| url.to_file_path().ok())
                    .collect();
                hand_over_paths(_app, paths);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{paths_from_args, provider_page_url, release_url};

    fn duplicate_job_fixture() -> (
        std::sync::Arc<tagrex_commands::query::QueryIndex>,
        super::DuplicateJob,
    ) {
        let tracks=(0..3).map(|i|serde_json::from_value(serde_json::json!({"path":format!("/virtual/{i}.flac"),"format":"FLAC","tags":{"title":"Track"}})).unwrap()).collect::<Vec<super::TrackDto>>();
        let index = std::sync::Arc::new(tagrex_commands::query::QueryIndex::from_tracks(&tracks));
        let job = super::DuplicateJob {
            status: std::sync::Mutex::new(tagrex_commands::duplicates::DuplicateJobStatus {
                job_id: 1,
                snapshot_id: index.id,
                kind: "exact".into(),
                status: "running".into(),
                processed: 0,
                total: 3,
                skipped: 0,
                errors: 0,
                groups_revision: 0,
                groups_count: 0,
                phase: "stamping".into(),
                message: None,
            }),
            cancel: std::sync::atomic::AtomicBool::new(false),
            index: std::sync::Arc::downgrade(&index),
        };
        (index, job)
    }

    fn duplicate_update() -> tagrex_commands::duplicates::ScanProgress {
        tagrex_commands::duplicates::ScanProgress {
            phase: "hashing",
            processed: 2,
            total: 3,
            skipped: 0,
            errors: 0,
            groups: Some(vec![tagrex_commands::query::DuplicateQueryGroup {
                id: "stable".into(),
                label: "Track".into(),
                reason: "verified bytes".into(),
                confidence: "exact".into(),
                indices: vec![0, 1],
            }]),
        }
    }

    #[test]
    fn duplicate_job_cancel_clears_partial_groups_and_rejects_late_publication() {
        let (index, job) = duplicate_job_fixture();
        job.publish(duplicate_update());
        assert_eq!(index.duplicate_results("exact").groups_revision, 1);
        assert_eq!(index.duplicate_results("exact").groups.len(), 1);
        let status = serde_json::to_value(job.status.lock().unwrap().clone()).unwrap();
        assert_eq!(status["groupsCount"], 1);
        assert_eq!(status["groupsRevision"], 1);
        assert!(status.get("groups").is_none());
        job.clear("cancelled", None, true);
        assert!(index.duplicate_results("exact").groups.is_empty());
        assert_eq!(index.duplicate_results("exact").groups_revision, 2);
        job.publish(duplicate_update());
        assert_eq!(index.duplicate_results("exact").groups_revision, 2);
        assert_eq!(job.status.lock().unwrap().groups_count, 0);
    }

    #[test]
    fn duplicate_job_supersession_and_snapshot_invalidation_stop_old_updates() {
        let (index, job) = duplicate_job_fixture();
        job.publish(duplicate_update());
        job.clear("cancelled", None, true);
        index.set_duplicate_results(
            "exact",
            tagrex_commands::query::DuplicateResults {
                status: "running".into(),
                groups_revision: 10,
                ..Default::default()
            },
        );
        job.publish(duplicate_update());
        assert_eq!(index.duplicate_results("exact").groups_revision, 10);
        assert!(index.duplicate_results("exact").groups.is_empty());
        let (index, job) = duplicate_job_fixture();
        job.publish(duplicate_update());
        index.invalidate();
        job.publish(duplicate_update());
        job.clear("cancelled", None, false);
        assert!(index.duplicate_results("exact").groups.is_empty());
        assert_eq!(job.status.lock().unwrap().groups_count, 0);
        assert_eq!(job.status.lock().unwrap().status, "cancelled");
        let (index, job) = duplicate_job_fixture();
        job.publish(duplicate_update());
        let mut results = index.duplicate_results("exact");
        results.status = "complete".into();
        job.finish(results);
        assert_eq!(job.status.lock().unwrap().status, "complete");
        index.invalidate();
        job.clear("cancelled", None, false);
        assert_eq!(job.status.lock().unwrap().groups_count, 0);
        assert_eq!(job.status.lock().unwrap().status, "cancelled");
    }

    #[test]
    fn bpm_results_from_a_replaced_library_are_rejected_before_accessing_the_session() {
        let state: super::AppState = Default::default();
        let earlier = super::LIBRARY_GENERATION
            .load(std::sync::atomic::Ordering::SeqCst)
            .wrapping_sub(1);
        let error = super::preview_detected_bpm(&state, earlier, &[]).unwrap_err();
        assert!(error.text.contains("Library changed while detecting BPM"));
    }

    #[test]
    fn bpm_results_for_the_current_library_use_the_session() {
        let root = std::env::temp_dir().join(format!(
            "tagrex-bpm-current-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let app = super::App::open(&root, &root.join("journal.sqlite")).unwrap();
        let state: super::AppState = std::sync::Arc::new(std::sync::Mutex::new(Some(app)));
        let current = super::LIBRARY_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
        let plan = super::preview_detected_bpm(&state, current, &[]).unwrap();
        assert!(plan.changes.is_empty());
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cover_path_results_reject_replaced_library_before_accessing_the_session() {
        let root = std::env::temp_dir().join(format!(
            "tagrex-cover-path-generation-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("chosen.png");
        std::fs::write(&source, b"PNG").unwrap();
        let app = super::App::open(&root, &root.join("journal.sqlite")).unwrap();
        let options = app.cover_embed_options(&[]).unwrap();
        let state: super::AppState = Default::default();
        let earlier = super::LIBRARY_GENERATION
            .load(std::sync::atomic::Ordering::SeqCst)
            .wrapping_sub(1);
        let error = super::preview_prepared_cover_embed(
            &state,
            earlier,
            &[],
            options.read_image(&source).unwrap(),
        )
        .unwrap_err();
        assert!(error.text.contains("The open library changed"));
        // A current session builds the normal empty preview without media writes.
        let state = std::sync::Arc::new(std::sync::Mutex::new(Some(app)));
        let current = super::LIBRARY_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
        let plan = super::preview_prepared_cover_embed(
            &state,
            current,
            &[],
            options.read_image(&source).unwrap(),
        )
        .unwrap();
        assert!(plan.changes.is_empty());
        assert!(plan.cover_assets.is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), b"PNG");
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saved_preferences_reach_a_library_that_is_still_opening() {
        use std::sync::{mpsc, Arc, Mutex};
        use std::time::{Duration, Instant, SystemTime};
        // Saving consults the process-wide read state; hold the same lock the
        // settings-store tests use so a deliberate failure cannot leak in.
        let _serial = super::settings_store::serialise_save_tests();
        let root = std::env::temp_dir().join(format!(
            "tagrex-settings-open-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let state: super::AppState = Arc::new(Mutex::new(None));
        let providers: super::ProviderState = Arc::new(super::ProviderQueues::default());
        let (started, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let opening = tauri::async_runtime::spawn({
            let state = state.clone();
            let root = root.clone();
            async move {
                super::SESSION_QUEUE
                    .run(move || {
                        started.send(()).unwrap();
                        wait.recv_timeout(Duration::from_secs(10)).unwrap();
                        let app = super::App::open(&root, &root.join("journal.sqlite")).unwrap();
                        app.apply_settings(&super::SettingsDto::default());
                        *state.lock().unwrap() = Some(app);
                        Ok(())
                    })
                    .await
            }
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let saving = tauri::async_runtime::spawn(super::save_settings_at(
            root.join("settings.json"),
            state.clone(),
            providers.clone(),
            None,
            super::SettingsDto {
                bpm_max_minutes: 7,
                ..Default::default()
            },
            None,
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        while providers.settings.lock().unwrap().bpm_max_minutes != 7 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let persisted = providers.settings.lock().unwrap().bpm_max_minutes == 7;
        release.send(()).unwrap();
        tauri::async_runtime::block_on(opening).unwrap().unwrap();
        tauri::async_runtime::block_on(saving).unwrap().unwrap();
        assert!(persisted);
        let actual = state.lock().unwrap().as_ref().unwrap().bpm_max_length();
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(actual, Some(Duration::from_secs(7 * 60)));
    }

    /// A startup read that failed leaves the interface on defaults. Saving
    /// those defaults over a file that may have become readable again would
    /// discard every saved chain and preset, so the save is refused and the
    /// file keeps its bytes.
    #[test]
    fn settings_are_not_overwritten_after_a_failed_read() {
        use std::sync::{Arc, Mutex};
        use std::time::SystemTime;
        let _serial = super::settings_store::serialise_save_tests();
        let root = std::env::temp_dir().join(format!(
            "tagrex-settings-unread-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        let before = br#"{"bpm_max_minutes":7}"#;
        std::fs::write(&path, before).unwrap();

        super::settings_store::note_read_failure("permission denied".to_string());
        let state: super::AppState = Arc::new(Mutex::new(None));
        let providers: super::ProviderState = Arc::new(super::ProviderQueues::default());
        let untouched = super::SettingsDto::default().bpm_max_minutes;
        let result = tauri::async_runtime::block_on(super::save_settings_at(
            path.clone(),
            state,
            providers.clone(),
            None,
            super::SettingsDto {
                bpm_max_minutes: 9,
                ..Default::default()
            },
            None,
        ));
        super::settings_store::note_read_success();
        let error = result.expect_err("the save went ahead after a failed read");

        assert!(
            error.text.contains("could not be read earlier"),
            "unexpected message: {}",
            error.text
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            providers.settings.lock().unwrap().bpm_max_minutes,
            untouched,
            "the refused settings were applied anyway"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_desktop_settings_do_not_overwrite_agent_presets() {
        let _serial = super::settings_store::serialise_save_tests();
        let root =
            std::env::temp_dir().join(format!("tagrex-settings-agent-cas-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        let initial = super::SettingsDto::default();
        super::settings_store::save(&path, &initial).unwrap();
        let revision = tagrex_service::settings_revision(
            &super::mcp_control::settings_document(&path).unwrap(),
        );
        let mut changed = serde_json::to_value(&initial).unwrap();
        changed["query_presets"] = serde_json::json!([{"name":"Missing covers","queryVersion":2}]);
        super::settings_store::save(&path, &changed).unwrap();
        let before = std::fs::read(&path).unwrap();
        let result = tauri::async_runtime::block_on(super::save_settings_at(
            path.clone(),
            super::AppState::default(),
            super::ProviderState::default(),
            None,
            initial,
            Some(revision),
        ));
        assert!(result.unwrap_err().text.contains("changed elsewhere"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_save_preserves_unknown_settings_slices_and_returns_actual_revision() {
        let _serial = super::settings_store::serialise_save_tests();
        let root = std::env::temp_dir().join(format!(
            "tagrex-settings-agent-merge-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        let mut original = serde_json::to_value(super::SettingsDto::default()).unwrap();
        original["future_native_slice"] = serde_json::json!({"keep":"untouched"});
        original["query_presets"] = serde_json::json!([{"name":"My filter","queryVersion":2}]);
        super::settings_store::save(&path, &original).unwrap();
        let revision = tagrex_service::settings_revision(
            &super::mcp_control::settings_document(&path).unwrap(),
        );
        let mut edited: super::SettingsDto = serde_json::from_value(original).unwrap();
        edited.bpm_max_minutes = 7;
        let actual_revision = tauri::async_runtime::block_on(super::save_settings_at(
            path.clone(),
            super::AppState::default(),
            super::ProviderState::default(),
            None,
            edited,
            Some(revision),
        ))
        .unwrap();
        let after = super::mcp_control::settings_document(&path).unwrap();
        assert_eq!(after["future_native_slice"]["keep"], "untouched");
        assert_eq!(after["query_presets"][0]["name"], "My filter");
        assert_eq!(after["bpm_max_minutes"], 7);
        assert_eq!(actual_revision, tagrex_service::settings_revision(&after));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn schema_invalid_settings_keep_recovery_bytes_when_desktop_saves_defaults() {
        let _serial = super::settings_store::serialise_save_tests();
        let root = std::env::temp_dir().join(format!(
            "tagrex-settings-agent-corrupt-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        let before = br#"{"bpm_max_minutes":"invalid","query_presets":[{"name":"recover me"}]}"#;
        std::fs::write(&path, before).unwrap();
        let defaults = super::SettingsDto::default();
        let revision = tagrex_service::settings_revision(&serde_json::to_value(&defaults).unwrap());
        tauri::async_runtime::block_on(super::save_settings_at(
            path.clone(),
            super::AppState::default(),
            super::ProviderState::default(),
            None,
            defaults,
            Some(revision),
        ))
        .unwrap();
        let recovered = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("settings.json.corrupt-")
            })
            .unwrap();
        assert_eq!(std::fs::read(recovered.path()).unwrap(), before);
        assert!(super::settings_store::read::<super::SettingsDto>(&path)
            .unwrap()
            .is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provider_sources_keep_independent_persistent_queues() {
        let providers = super::ProviderQueues::default();
        let first = providers.source("discogs").unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &providers.source("discogs").unwrap()
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &first,
            &providers.source("musicbrainz").unwrap()
        ));
        assert!(providers.source("unknown").is_err());
    }

    #[test]
    fn saving_network_preferences_does_not_wait_for_inflight_http() {
        let providers = std::sync::Arc::new(super::ProviderQueues::default());
        let source = providers.source("discogs").unwrap();
        // Represents a provider request holding its hub during HTTP/throttling.
        let inflight = source.hub.lock().unwrap();
        let (finished, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn({
            let providers = providers.clone();
            move || {
                let settings = super::SettingsDto {
                    rate_limit_per_min: 15,
                    ..Default::default()
                };
                providers.apply_settings(&settings).unwrap();
                finished.send(()).unwrap();
            }
        });
        let ready = received
            .recv_timeout(std::time::Duration::from_secs(2))
            .is_ok();
        drop(inflight);
        worker.join().unwrap();
        assert!(
            ready,
            "saving settings waited for an in-flight provider request"
        );
        assert_eq!(source.settings.lock().unwrap().rate_limit_per_min, 15);
    }

    #[test]
    fn provider_identity_pages_are_scoped_and_exact() {
        assert_eq!(
            provider_page_url("discogs", "artist", "123").unwrap(),
            "https://www.discogs.com/artist/123"
        );
        assert_eq!(
            provider_page_url("discogs", "label", "456").unwrap(),
            "https://www.discogs.com/label/456"
        );
        for (source, entity, id) in [
            ("discogs", "label", "../evil"),
            ("discogs", "artist", ""),
            ("discogs", "artist", "0"),
            ("discogs", "unknown", "1"),
            ("musicbrainz", "label", "1"),
        ] {
            assert!(provider_page_url(source, entity, id).is_err());
        }
    }

    #[test]
    fn a_command_line_yields_the_paths_in_it_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("tagrex-launch-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // What Finder adds when it starts the app is not a folder, and neither
        // is a path that is not there.
        let argv = [
            "-psn_0_774163".to_string(),
            dir.to_string_lossy().into_owned(),
            dir.join("no-such-folder").to_string_lossy().into_owned(),
        ];
        assert_eq!(
            paths_from_args(argv.into_iter().map(Into::into)),
            vec![dir.clone()]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builds_provider_release_urls() {
        assert_eq!(
            release_url("discogs", "5606").unwrap(),
            "https://www.discogs.com/release/5606"
        );
        assert_eq!(
            release_url("musicbrainz", "1a2b3c4d-0000-0000-0000-000000000000").unwrap(),
            "https://musicbrainz.org/release/1a2b3c4d-0000-0000-0000-000000000000"
        );
        // The store's own URLs carry a slug, but it is cosmetic (#162) — a
        // placeholder keeps anything user-supplied out of the path.
        assert_eq!(
            release_url("beatport", "4321").unwrap(),
            "https://www.beatport.com/release/-/4321"
        );
    }

    #[test]
    fn rejects_unknown_source_and_unsafe_ids() {
        assert!(release_url("bandcamp", "5606").is_err());
        assert!(release_url("beatport", "").is_err());
        assert!(release_url("beatport", "4321/evil").is_err());
        assert!(release_url("discogs", "").is_err());
        // A path-traversal / injection attempt in the id must never build a URL.
        assert!(release_url("discogs", "5606/../../evil").is_err());
        assert!(release_url("discogs", "5606?q=1").is_err());
    }

    /// A credential file is readable by its owner alone, and a file an older
    /// version wrote world-readable is tightened when it is saved again.
    #[cfg(unix)]
    #[test]
    fn a_secret_is_written_readable_by_its_owner_alone() {
        use super::write_secret;
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("tagrex-secret-{}", std::process::id()));
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_secret(&path, "token").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(mode, 0o600);
        assert_eq!(contents, "token");

        // A new file starts that way too.
        let fresh = std::env::temp_dir().join(format!("tagrex-secret-new-{}", std::process::id()));
        std::fs::remove_file(&fresh).ok();
        write_secret(&fresh, "t").unwrap();
        let mode = std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777;
        std::fs::remove_file(&fresh).ok();
        assert_eq!(mode, 0o600);
    }

    /// A credential file an older version left readable by others is
    /// tightened when it is read, and its contents are left alone (#675).
    #[cfg(unix)]
    #[test]
    fn a_secret_left_readable_is_tightened_on_read() {
        use super::tighten_secret;
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("tagrex-secret-old-{}", std::process::id()));
        std::fs::write(&path, "token").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        tighten_secret(&path);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let contents = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(mode, 0o600);
        assert_eq!(contents, "token");
        // A missing file is no error.
        tighten_secret(&path);
    }
}
