//! Tauri commands for the audio studio. The command layer owns source and
//! export state. Independent work queues keep review and export off the UI.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tagrex_commands::audio_studio::{
    CueEncoding, DiscogsProposal, DraftSegment, ExportResult, ImportedCue, OpenedRecording,
    Progress, ProviderAnchor, RecordingProgress, RestoreResult, Review, ReviewRequest, Studio,
    StudioError,
};
use tagrex_commands::ErrorDto;
use tauri::{Manager, State};

use super::AppState;
use super::{blocking, saved_discogs_token, shell_error, with_providers, ProviderState};

pub(super) type StudioState = Arc<Studio>;
#[derive(Default)]
pub(super) struct Openings(Mutex<BTreeMap<String, Arc<Opening>>>);

#[derive(Default)]
struct Opening {
    cancel: AtomicBool,
    progress: Mutex<Option<RecordingProgress>>,
}
static OPEN: blocking::Queue = blocking::Queue::new();
static PEAKS: blocking::Queue = blocking::Queue::new();
static LATEST_PEAKS: blocking::Latest = blocking::Latest::new();
static REVIEW: blocking::Queue = blocking::Queue::new();
static EXPORT: blocking::Queue = blocking::Queue::new();
static CUE: blocking::Queue = blocking::Queue::new();

#[tauri::command]
pub(super) async fn audio_split_open(
    studio: State<'_, StudioState>,
    openings: State<'_, Openings>,
    path: String,
    request_id: Option<String>,
) -> Result<OpenedRecording, ErrorDto> {
    let studio = studio.inner().clone();
    let Some(id) = request_id else {
        return OPEN
            .run(move || studio.open(Path::new(&path), || false).map_err(shell_error))
            .await;
    };
    if id.is_empty() || id.len() > 100 {
        return Err(shell_error("Invalid recording request."));
    }
    let opening = Arc::new(Opening::default());
    {
        let mut pending = openings.0.lock().map_err(shell_error)?;
        if pending.contains_key(&id) || pending.len() >= 4 {
            return Err(shell_error("Too many recordings are opening."));
        }
        pending.insert(id.clone(), opening.clone());
    }
    let job = opening.clone();
    let result = OPEN
        .run(move || {
            studio
                .open_with_progress(
                    Path::new(&path),
                    || job.cancel.load(Ordering::Acquire),
                    |progress| {
                        if let Ok(mut current) = job.progress.lock() {
                            *current = Some(progress);
                        }
                    },
                )
                .map_err(shell_error)
        })
        .await;
    openings.0.lock().map_err(shell_error)?.remove(&id);
    result
}

#[tauri::command]
pub(super) fn audio_split_loading(
    openings: State<'_, Openings>,
    request_id: String,
) -> Result<Option<RecordingProgress>, ErrorDto> {
    let opening = openings
        .0
        .lock()
        .map_err(shell_error)?
        .get(&request_id)
        .cloned();
    opening
        .map(|job| {
            job.progress
                .lock()
                .map(|value| value.clone())
                .map_err(shell_error)
        })
        .transpose()
        .map(Option::flatten)
}

#[tauri::command]
pub(super) fn audio_split_cancel_open(
    openings: State<'_, Openings>,
    request_id: String,
) -> Result<(), ErrorDto> {
    if let Some(opening) = openings.0.lock().map_err(shell_error)?.get(&request_id) {
        opening.cancel.store(true, Ordering::Release);
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn audio_split_peaks(
    studio: State<'_, StudioState>,
    id: String,
    start_frame: u64,
    end_frame: u64,
    bins: usize,
) -> Result<tauri::ipc::Response, ErrorDto> {
    let studio = studio.inner().clone();
    PEAKS
        .run_latest(&LATEST_PEAKS, move |cancel| {
            let peaks = studio
                .peaks(&id, start_frame, end_frame, bins, || cancel.cancelled())
                .map_err(shell_error)?;
            let bytes: Vec<u8> = peaks.into_iter().flat_map(f32::to_le_bytes).collect();
            Ok(tauri::ipc::Response::new(bytes))
        })
        .await
}

#[tauri::command]
pub(super) async fn audio_split_preview(
    app: tauri::AppHandle,
    studio: State<'_, StudioState>,
    library: State<'_, AppState>,
    request: ReviewRequest,
) -> Result<Review, ErrorDto> {
    let studio = studio.inner().clone();
    let library = library.inner().clone();
    REVIEW
        .run(move || {
            let root = recursive_root(&library)?;
            let recovery = if request.needs_default_recovery() {
                let folder = app
                    .path()
                    .app_data_dir()
                    .map_err(shell_error)?
                    .join("Audio recovery");
                std::fs::create_dir_all(&folder).map_err(shell_error)?;
                Some(folder)
            } else {
                None
            };
            studio
                .preview_with_recovery(request, recovery.as_deref(), root.as_deref())
                .map_err(shell_error)
        })
        .await
}

fn recursive_root(library: &AppState) -> Result<Option<PathBuf>, ErrorDto> {
    let opened = library.lock().map_err(shell_error)?;
    Ok(opened
        .as_ref()
        .and_then(|app| app.recursive_audio_root())
        .map(Path::to_path_buf))
}

fn sync_file_set(library: &AppState, removed: &[String], added: &[String]) {
    if let Ok(mut opened) = library.lock() {
        if let Some(app) = opened.as_mut() {
            app.sync_audio_file_set(removed, added);
        }
    }
}

#[tauri::command]
pub(super) async fn audio_split_export(
    studio: State<'_, StudioState>,
    library: State<'_, AppState>,
    id: String,
) -> Result<ExportResult, ErrorDto> {
    let studio = studio.inner().clone();
    let library = library.inner().clone();
    EXPORT
        .run(move || {
            let root = recursive_root(&library)?;
            let result = studio
                .export_with_library(&id, root.as_deref())
                .map_err(shell_error)?;
            sync_file_set(&library, &result.replaced, &result.paths);
            Ok(result)
        })
        .await
}

#[tauri::command]
pub(super) async fn audio_split_undo_replacement(
    studio: State<'_, StudioState>,
    library: State<'_, AppState>,
    id: String,
) -> Result<RestoreResult, ErrorDto> {
    let studio = studio.inner().clone();
    let library = library.inner().clone();
    EXPORT
        .run(move || {
            let result = studio.undo_replacement(&id).map_err(shell_error)?;
            sync_file_set(&library, &result.removed, &result.paths);
            Ok(result)
        })
        .await
}

#[tauri::command]
pub(super) fn audio_split_progress(
    studio: State<'_, StudioState>,
    id: String,
) -> Result<Progress, ErrorDto> {
    studio.progress(&id).map_err(shell_error)
}

#[tauri::command]
pub(super) fn audio_split_cancel(
    studio: State<'_, StudioState>,
    player: State<'_, Player>,
    id: String,
) -> Result<(), ErrorDto> {
    studio.cancel(&id).map_err(shell_error)?;
    player.studio_close(id).map_err(shell_error)
}

#[tauri::command]
pub(super) async fn audio_split_reveal(
    studio: State<'_, StudioState>,
    id: String,
) -> Result<(), ErrorDto> {
    let folder = studio.published_folder(&id).map_err(shell_error)?;
    OPEN.run(move || open::that(folder).map_err(shell_error))
        .await
}

#[tauri::command]
pub(super) async fn audio_split_can_add(
    studio: State<'_, StudioState>,
    library: State<'_, AppState>,
    id: String,
) -> Result<bool, ErrorDto> {
    let studio = studio.inner().clone();
    let library = library.inner().clone();
    REVIEW
        .run(move || {
            let result = studio.published_result(&id).map_err(shell_error)?;
            let opened = library.lock().map_err(shell_error)?;
            Ok(opened.as_ref().is_some_and(|app| {
                result.paths.iter().all(|path| {
                    Path::new(path)
                        .parent()
                        .is_some_and(|folder| app.can_discover_output_folder(folder))
                })
            }))
        })
        .await
}

#[tauri::command]
pub(super) async fn audio_split_cue(
    studio: State<'_, StudioState>,
    id: String,
    path: String,
    encoding: Option<CueEncoding>,
) -> Result<ImportedCue, ErrorDto> {
    let studio = studio.inner().clone();
    CUE.run(move || {
        studio
            .import_cue(&id, Path::new(&path), encoding.unwrap_or_default())
            .map_err(shell_error)
    })
    .await
}

#[tauri::command]
pub(super) async fn audio_split_write_cue(
    studio: State<'_, StudioState>,
    id: String,
    path: String,
    segments: Vec<DraftSegment>,
    tags: std::collections::BTreeMap<String, String>,
) -> Result<Vec<String>, ErrorDto> {
    let studio = studio.inner().clone();
    CUE.run(move || {
        studio
            .save_cue(&id, Path::new(&path), &segments, &tags)
            .map_err(shell_error)
    })
    .await
}

#[tauri::command]
pub(super) async fn audio_split_discogs(
    app: tauri::AppHandle,
    providers: State<'_, ProviderState>,
    studio: State<'_, StudioState>,
    id: String,
    release_id: String,
    token: Option<String>,
    anchors: Option<Vec<ProviderAnchor>>,
) -> Result<DiscogsProposal, ErrorDto> {
    let queue = providers.source("discogs")?;
    let studio = studio.inner().clone();
    let release_id = release_id.trim().to_owned();
    let anchors = anchors.unwrap_or_default();
    let token = match token {
        Some(token) => token,
        None if anchors.is_empty() => saved_discogs_token(app).await?,
        None => String::new(),
    };
    let source = queue.clone();
    queue
        .jobs
        .run(move || {
            studio
                .discogs_proposal(&id, &release_id, &anchors, || {
                    if token.trim().is_empty() {
                        return Err(StudioError::new(
                            "Set your Discogs token in Settings to look up a release.",
                        ));
                    }
                    with_providers(&source, |hub| {
                        hub.audio_discogs_release(token.trim(), &release_id)
                            .map_err(ErrorDto::from)
                    })
                    .map_err(|error| StudioError::new(error.text))
                })
                .map_err(shell_error)
        })
        .await
}

use std::time::Duration;
use tagrex_player::{Player, StudioAudition, StudioPlayback, StudioPreviewState};

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum TransportAction {
    Play,
    Pause,
    Seek,
    Audition,
}

#[derive(serde::Serialize)]
pub(super) struct TransportStatus {
    frame: u64,
    playing: bool,
    loading: bool,
    sequence: u64,
}

fn frame_time(frame: u64, rate: u32) -> Duration {
    let rate = u64::from(rate);
    let nanos = ((frame % rate) * 1_000_000_000 + rate / 2) / rate;
    Duration::new(frame / rate, nanos as u32)
}

#[tauri::command]
pub(super) async fn audio_split_transport(
    studio: State<'_, StudioState>,
    player: State<'_, Player>,
    id: String,
    action: TransportAction,
    frame: Option<u64>,
    end_frame: Option<u64>,
    sequence: Option<u64>,
) -> Result<(), ErrorDto> {
    studio
        .with_playback_source(&id, |file, rate, total, seek_index| {
            let frame = frame.unwrap_or_default();
            if frame >= total && !matches!(action, TransportAction::Pause) {
                return Err(StudioError::new(
                    "Choose a preview position inside the recording.",
                ));
            }
            let start = frame_time(frame, rate);
            let end = frame_time(total, rate);
            let owner = id.clone();
            let sequence = sequence.unwrap_or_default();
            let play = |audition| {
                player.studio_play_request(
                    owner.clone(),
                    sequence,
                    StudioPlayback {
                        file,
                        start,
                        end,
                        audition,
                        sample_rate: rate,
                        seek_index,
                    },
                )
            };
            match action {
                TransportAction::Seek => player.studio_seek_sequenced(owner, sequence, start),
                TransportAction::Play => play(None),
                TransportAction::Pause => player.studio_pause_sequenced(owner, sequence),
                TransportAction::Audition => {
                    let cut_end = end_frame
                        .filter(|end| *end > frame && *end <= total)
                        .ok_or_else(|| {
                            StudioError::new("Choose an audition range inside the recording.")
                        })?;
                    let audition = StudioAudition {
                        start,
                        end: frame_time(cut_end, rate),
                    };
                    play(Some(audition))
                }
            }
            .map_err(StudioError::new)
        })
        .map_err(shell_error)
}

#[tauri::command]
pub(super) fn audio_split_audition_output(
    studio: State<'_, StudioState>,
    player: State<'_, Player>,
    id: String,
    index: usize,
    sequence: u64,
) -> Result<(), ErrorDto> {
    studio
        .with_review_playback(&id, index, |owner, file, rate, range, seek_index| {
            let start = frame_time(range.start, rate);
            let end = frame_time(
                range.end.min(range.start.saturating_add(u64::from(rate) * 4)),
                rate,
            );
            player
                .studio_play_request(
                    owner.to_owned(),
                    sequence,
                    StudioPlayback {
                        file,
                        start,
                        end,
                        audition: None,
                        sample_rate: rate,
                        seek_index,
                    },
                )
                .map_err(StudioError::new)
        })
        .map_err(shell_error)
}

#[tauri::command]
pub(super) fn audio_split_status(
    studio: State<'_, StudioState>,
    player: State<'_, Player>,
    id: String,
    sequence: Option<u64>,
    audition: Option<bool>,
) -> Result<TransportStatus, ErrorDto> {
    let status = player.studio_status(&id);
    let frame = if audition.unwrap_or_default() {
        0
    } else {
        let (rate, total) = studio.cue_geometry(&id).map_err(shell_error)?;
        let frame = (status.position.as_nanos() * u128::from(rate) + 500_000_000) / 1_000_000_000;
        frame.min(u128::from(total.saturating_sub(1))) as u64
    };
    let snapshot = TransportStatus {
        frame,
        playing: status.playing,
        loading: status.state == StudioPreviewState::Loading,
        sequence: status.sequence,
    };
    if sequence.is_some_and(|expected| expected != status.sequence)
        && !(status.sequence == 0 && status.state == StudioPreviewState::Refused)
    {
        return Ok(snapshot);
    }
    match status.state {
        StudioPreviewState::Unavailable => Err(shell_error(
            "Audio preview could not open an output device or this recording.",
        )),
        StudioPreviewState::Refused => Err(shell_error(
            "The audio decoder could not seek to that position.",
        )),
        _ => Ok(snapshot),
    }
}

#[tauri::command]
pub(super) fn audio_split_close(
    studio: State<'_, StudioState>,
    player: State<'_, Player>,
    id: String,
) -> Result<(), ErrorDto> {
    studio
        .close_with(&id, || {
            player.studio_close(id.clone()).map_err(StudioError::new)
        })
        .map_err(shell_error)
}
