//! Tauri commands for the audio studio. The command layer owns source and
//! export state. Independent work queues keep review and export off the UI.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tagrex_commands::audio_studio::{
    AudioTreatment, CueEncoding, DiscogsProposal, DraftSegment, EqBand, ExportResult, ImportedCue,
    OpenedRecording, PreviewComparison, PreviewMonitor, Progress, ProviderAnchor,
    RecordingProgress, RestoreResult, Review, ReviewRequest, SpectrogramChannel, SpectrogramDetail,
    SpectrumView, Studio, StudioError, TreatmentAnalysis, TreatmentProbe, TreatmentProbeOptions,
};
use tagrex_commands::ErrorDto;
use tauri::{
    Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

use super::AppState;
use super::{blocking, saved_discogs_token, shell_error, with_providers, ProviderState};

mod window_preferences;
use window_preferences::{WindowPreferences, MIN_HEIGHT, MIN_WIDTH};

pub(super) type StudioState = Arc<Studio>;
#[derive(Default)]
pub(super) struct Openings(Mutex<BTreeMap<String, Arc<Opening>>>);

#[derive(Default)]
pub(super) struct ToolWindows {
    commands: tauri::async_runtime::Mutex<()>,
    visible: [AtomicBool; AudioToolKind::ALL.len()],
    preferences: WindowPreferences,
}

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AudioToolKind {
    Eq,
    Dynamics,
    Repair,
}

impl AudioToolKind {
    pub(super) const ALL: [Self; 3] = [Self::Eq, Self::Dynamics, Self::Repair];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Eq => "audio-tool-eq",
            Self::Dynamics => "audio-tool-dynamics",
            Self::Repair => "audio-tool-repair",
        }
    }

    pub(super) fn query(self) -> &'static str {
        match self {
            Self::Eq => "tool=eq",
            Self::Dynamics => "tool=dynamics",
            Self::Repair => "tool=repair",
        }
    }

    fn title_and_size(self) -> (&'static str, f64, f64) {
        match self {
            Self::Eq => ("Equalizer", 760.0, 490.0),
            Self::Dynamics => ("Dynamics", 680.0, 510.0),
            Self::Repair => ("Repair and stereo", 640.0, 510.0),
        }
    }
}

pub(super) const TOOL_DOCUMENT: &str = "audio-tools.html";

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ToolWindowAction {
    Toggle,
    Hide,
    HideAll,
    Pin,
    Unpin,
    IsPinned,
}

#[derive(Clone, Copy, serde::Serialize)]
struct ToolVisibility {
    kind: AudioToolKind,
    visible: bool,
}

fn publish_tool_visibility(app: &tauri::AppHandle, kind: AudioToolKind, visible: bool) {
    let handle = app.clone();
    // Keep reports from commands ordered with native window callbacks.
    let _ = app.run_on_main_thread(move || {
        let state = handle.state::<ToolWindows>();
        if state.visible[kind as usize].swap(visible, Ordering::AcqRel) == visible {
            return;
        }
        let payload = ToolVisibility { kind, visible };
        for target in ["main", kind.label()] {
            let _ = handle.emit_to(target, "tagnetic:audio-tool-visibility", payload);
        }
    });
}

fn focus_splitter(app: &tauri::AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.set_focus();
    }
}

// Use an async command for WebView2 window creation. The lock keeps a quick
// second click from racing the first window build.
#[tauri::command]
pub(super) async fn audio_split_tool_window(
    window: WebviewWindow,
    tool_windows: State<'_, ToolWindows>,
    kind: Option<AudioToolKind>,
    action: ToolWindowAction,
) -> Result<bool, ErrorDto> {
    if window.label() != "main" {
        return Err(shell_error("Open audio tools from the splitter."));
    }
    let _guard = tool_windows.commands.lock().await;
    let app = window.app_handle();
    let preferences = &tool_windows.preferences;
    if matches!(action, ToolWindowAction::HideAll) {
        let windows = AudioToolKind::ALL.map(|kind| (kind, app.get_webview_window(kind.label())));
        preferences.save(app, windows.clone(), None);
        let mut hid_visible_tool = false;
        for (kind, tool) in windows {
            if let Some(tool) = tool {
                hid_visible_tool |= tool.is_visible().unwrap_or(false);
                tool.hide().map_err(shell_error)?;
            }
            publish_tool_visibility(app, kind, false);
        }
        if hid_visible_tool {
            focus_splitter(app);
        }
        return Ok(false);
    }
    let kind = kind.ok_or_else(|| shell_error("Choose an audio tool."))?;
    let native = app.get_webview_window(kind.label());
    if matches!(action, ToolWindowAction::IsPinned) {
        return Ok(preferences.get(app, kind).pinned);
    }
    if matches!(action, ToolWindowAction::Pin | ToolWindowAction::Unpin) {
        let pinned = matches!(action, ToolWindowAction::Pin);
        if let Some(tool) = &native {
            tool.set_always_on_top(pinned).map_err(shell_error)?;
        }
        preferences.save(app, [(kind, native)], Some(pinned));
        return Ok(pinned);
    }
    if let Some(tool) = native {
        let hide = matches!(action, ToolWindowAction::Hide)
            || (tool.is_visible().map_err(shell_error)?
                && !tool.is_minimized().map_err(shell_error)?);
        if hide {
            preferences.save(app, [(kind, Some(tool.clone()))], None);
            tool.hide().map_err(shell_error)?;
            focus_splitter(app);
        } else {
            tool.unminimize().map_err(shell_error)?;
            preferences
                .get(app, kind)
                .recover_if_offscreen(&tool, &window);
            tool.show().map_err(shell_error)?;
            tool.set_focus().map_err(shell_error)?;
        }
        publish_tool_visibility(app, kind, !hide);
        return Ok(!hide);
    }
    if matches!(action, ToolWindowAction::Hide) {
        publish_tool_visibility(app, kind, false);
        return Ok(false);
    }
    let (title, width, height) = kind.title_and_size();
    let preference = preferences.get(app, kind);
    let url = WebviewUrl::App(format!("{TOOL_DOCUMENT}?{}", kind.query()).into());
    let tool = WebviewWindowBuilder::new(app, kind.label(), url)
        .title(title)
        .inner_size(width, height)
        .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
        .visible(false)
        .always_on_top(preference.pinned)
        .skip_taskbar(true)
        .parent(&window)
        .and_then(WebviewWindowBuilder::build)
        .map_err(shell_error)?;
    preference.restore(&tool, &window);
    let handle = tool.clone();
    tool.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            let app = handle.app_handle();
            app.state::<ToolWindows>()
                .preferences
                .save(app, [(kind, Some(handle.clone()))], None);
            if handle.hide().is_ok() {
                api.prevent_close();
                focus_splitter(app);
                publish_tool_visibility(app, kind, false);
            }
        }
        WindowEvent::Destroyed => publish_tool_visibility(handle.app_handle(), kind, false),
        // Windows and GTK send resize events when a window is minimized or
        // restored. AppKit may only deliver a focus change.
        WindowEvent::Resized(_) | WindowEvent::Focused(_) => {
            if let (Ok(visible), Ok(minimized)) = (handle.is_visible(), handle.is_minimized()) {
                publish_tool_visibility(handle.app_handle(), kind, visible && !minimized);
            }
        }
        _ => {}
    });
    tool.show().map_err(shell_error)?;
    tool.set_focus().map_err(shell_error)?;
    publish_tool_visibility(app, kind, true);
    Ok(true)
}

pub(super) fn save_tool_window_preferences(app: &tauri::AppHandle) {
    app.state::<ToolWindows>().preferences.save(
        app,
        AudioToolKind::ALL.map(|kind| (kind, app.get_webview_window(kind.label()))),
        None,
    );
}

pub(super) fn destroy_tool_windows(app: &tauri::AppHandle) {
    for kind in AudioToolKind::ALL {
        if let Some(tool) = app.get_webview_window(kind.label()) {
            let _ = tool.destroy();
        }
    }
}

#[derive(Default)]
struct Opening {
    cancel: AtomicBool,
    progress: Mutex<Option<RecordingProgress>>,
}
static OPEN: blocking::Queue = blocking::Queue::new();
static PEAKS: blocking::Queue = blocking::Queue::new();
static LATEST_PEAKS: blocking::Latest = blocking::Latest::new();
static TREATMENT: blocking::Queue = blocking::Queue::new();
static LATEST_TREATMENT: blocking::Latest = blocking::Latest::new();
static PROBE: blocking::Queue = blocking::Queue::new();
static LATEST_PROBE: blocking::Latest = blocking::Latest::new();
static COMPARE: blocking::Queue = blocking::Queue::new();
static LATEST_COMPARE: blocking::Latest = blocking::Latest::new();
static SPECTROGRAM: blocking::Queue = blocking::Queue::new();
static LATEST_SPECTROGRAM: blocking::Latest = blocking::Latest::new();
static EQ_MATCH: blocking::Queue = blocking::Queue::new();
static LATEST_EQ_MATCH: blocking::Latest = blocking::Latest::new();
static EQ_LEARN: blocking::Queue = blocking::Queue::new();
static LATEST_EQ_LEARN: blocking::Latest = blocking::Latest::new();
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
    stereo: Option<bool>,
    energy: Option<bool>,
) -> Result<tauri::ipc::Response, ErrorDto> {
    let studio = studio.inner().clone();
    PEAKS
        .run_latest(&LATEST_PEAKS, move |cancel| {
            let peaks = studio
                .peaks(
                    &id,
                    tagrex_commands::audio_studio::FrameRange {
                        start: start_frame,
                        end: end_frame,
                    },
                    bins,
                    stereo.unwrap_or_default(),
                    energy.unwrap_or_default(),
                    || cancel.cancelled(),
                )
                .map_err(shell_error)?;
            let bytes: Vec<u8> = peaks.into_iter().flat_map(f32::to_le_bytes).collect();
            Ok(tauri::ipc::Response::new(bytes))
        })
        .await
}

#[tauri::command]
pub(super) async fn audio_split_treatment(
    studio: State<'_, StudioState>,
    id: String,
    treatment: AudioTreatment,
) -> Result<TreatmentAnalysis, ErrorDto> {
    let studio = studio.inner().clone();
    TREATMENT
        .run_latest(&LATEST_TREATMENT, move |cancel| {
            studio
                .prepare_treatment(&id, treatment, || cancel.cancelled())
                .map_err(shell_error)
        })
        .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Keep the existing flat Tauri probe arguments.
pub(super) async fn audio_split_probe(
    studio: State<'_, StudioState>,
    player: State<'_, Player>,
    id: String,
    frame: u64,
    treatment: AudioTreatment,
    spectrogram: Option<bool>,
    spectrum: Option<bool>,
    spectrum_view: Option<SpectrumView>,
    playback: Option<bool>,
    viewport: Option<tagrex_commands::audio_studio::FrameRange>,
    spectrogram_channel: Option<SpectrogramChannel>,
    spectrogram_treated: Option<bool>,
    spectrogram_difference: Option<bool>,
    spectrogram_detail: Option<SpectrogramDetail>,
) -> Result<Option<TreatmentProbe>, ErrorDto> {
    let studio = studio.inner().clone();
    let options = TreatmentProbeOptions {
        spectrum: spectrum.unwrap_or(true),
        spectrum_view: spectrum_view.unwrap_or_default(),
        spectrogram: spectrogram.unwrap_or_default(),
        viewport,
        spectrogram_channel: spectrogram_channel.unwrap_or_default(),
        spectrogram_treated: spectrogram_treated.unwrap_or_default(),
        spectrogram_difference: spectrogram_difference.unwrap_or_default(),
        spectrogram_detail: spectrogram_detail.unwrap_or_default(),
    };
    let (producing, window) = if playback.unwrap_or_default() && !options.spectrogram {
        player.studio_playback_window(&id, frame, treatment.clone())
    } else {
        (false, None)
    };
    let (queue, lane) = if options.spectrogram_only() {
        (&SPECTROGRAM, &LATEST_SPECTROGRAM)
    } else {
        (&PROBE, &LATEST_PROBE)
    };
    queue
        .run_latest(lane, move |cancel| {
            if let Some(window) = window {
                return window
                    .analyze(options, || cancel.cancelled())
                    .map(Some)
                    .map_err(shell_error);
            }
            if producing {
                return Ok(None);
            }
            studio
                .treatment_probe(&id, frame, treatment, options, || cancel.cancelled())
                .map(Some)
                .map_err(shell_error)
        })
        .await
}

#[tauri::command]
pub(super) async fn audio_split_compare_levels(
    studio: State<'_, StudioState>,
    id: String,
    frame: u64,
    treatment: AudioTreatment,
    alternate: AudioTreatment,
) -> Result<PreviewComparison, ErrorDto> {
    let studio = studio.inner().clone();
    COMPARE
        .run_latest(&LATEST_COMPARE, move |cancel| {
            studio
                .compare_preview_levels(&id, frame, [treatment, alternate], || cancel.cancelled())
                .map_err(shell_error)
        })
        .await
}

#[tauri::command]
pub(super) async fn audio_split_eq_learn_threshold(
    window: WebviewWindow,
    studio: State<'_, StudioState>,
    id: String,
    range: tagrex_commands::audio_studio::FrameRange,
    treatment: AudioTreatment,
    band_index: usize,
) -> Result<f32, ErrorDto> {
    if window.label() != "main" {
        return Err(shell_error("Choose the loop from Split Studio."));
    }
    let studio = studio.inner().clone();
    EQ_LEARN
        .run_latest(&LATEST_EQ_LEARN, move |cancel| {
            studio
                .learn_eq_threshold(&id, range, treatment, band_index, || cancel.cancelled())
                .map_err(shell_error)
        })
        .await
}

#[tauri::command]
pub(super) async fn audio_split_eq_match(
    window: WebviewWindow,
    studio: State<'_, StudioState>,
    id: String,
    range: tagrex_commands::audio_studio::FrameRange,
    reference: String,
) -> Result<Vec<EqBand>, ErrorDto> {
    if window.label() != "main" {
        return Err(shell_error("Choose the reference from Split Studio."));
    }
    let studio = studio.inner().clone();
    EQ_MATCH
        .run_latest(&LATEST_EQ_MATCH, move |cancel| {
            studio
                .match_reference(&id, range, Path::new(&reference), || cancel.cancelled())
                .map_err(shell_error)
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
    Retune,
}

#[derive(serde::Serialize)]
pub(super) struct TransportStatus {
    frame: u64,
    playing: bool,
    loading: bool,
    sequence: u64,
    #[serde(rename = "retuneFailed", skip_serializing_if = "Option::is_none")]
    retune_failed: Option<u64>,
    #[serde(rename = "retunePending", skip_serializing_if = "Option::is_none")]
    retune_pending: Option<u64>,
}

fn frame_time(frame: u64, rate: u32) -> Duration {
    let rate = u64::from(rate);
    let nanos = ((frame % rate) * 1_000_000_000 + rate / 2) / rate;
    Duration::new(frame / rate, nanos as u32)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Keep the transport arguments flat for Tauri IPC.
pub(super) async fn audio_split_transport(
    studio: State<'_, StudioState>,
    player: State<'_, Player>,
    id: String,
    action: TransportAction,
    frame: Option<u64>,
    end_frame: Option<u64>,
    loop_start_frame: Option<u64>,
    smooth_loop: Option<bool>,
    sequence: Option<u64>,
    treatment: Option<AudioTreatment>,
    revision: Option<u64>,
    monitor: Option<PreviewMonitor>,
    preview_gain_db: Option<f32>,
) -> Result<(), ErrorDto> {
    if matches!(action, TransportAction::Retune) {
        let (file, prepared) = studio
            .playback_treatment(&id, treatment.unwrap_or_default())
            .map_err(shell_error)?;
        return player
            .studio_retune(
                id,
                sequence.unwrap_or_default(),
                revision.unwrap_or_default(),
                file,
                prepared.with_preview_monitor(monitor, preview_gain_db),
            )
            .map_err(shell_error);
    }
    let treatment = match action {
        TransportAction::Play | TransportAction::Audition => treatment.unwrap_or_default(),
        _ => AudioTreatment::default(),
    };
    studio
        .with_playback_source(
            &id,
            treatment,
            |file, rate, total, seek_index, treatment| {
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
                            treatment: treatment.with_preview_monitor(monitor, preview_gain_db),
                        },
                    )
                };
                match action {
                    TransportAction::Seek => player.studio_seek_sequenced(owner, sequence, start),
                    TransportAction::Play => play(None),
                    TransportAction::Pause => player.studio_pause_sequenced(owner, sequence),
                    TransportAction::Retune => unreachable!("retune has no source transport"),
                    TransportAction::Audition => {
                        let loop_start = loop_start_frame.unwrap_or(frame);
                        if loop_start > frame {
                            return Err(StudioError::new(
                                "Choose a play position inside the loop.",
                            ));
                        }
                        let cut_end = end_frame
                            .filter(|end| *end > frame && *end <= total)
                            .ok_or_else(|| {
                                StudioError::new("Choose an audition range inside the recording.")
                            })?;
                        let audition = StudioAudition {
                            start: frame_time(loop_start, rate),
                            end: frame_time(cut_end, rate),
                            smooth: smooth_loop.unwrap_or(false),
                        };
                        play(Some(audition))
                    }
                }
                .map_err(StudioError::new)
            },
        )
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
        .with_review_playback(
            &id,
            index,
            |owner, file, rate, range, seek_index, treatment| {
                let start = frame_time(range.start, rate);
                let end = frame_time(
                    range
                        .end
                        .min(range.start.saturating_add(u64::from(rate) * 4)),
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
                            treatment,
                        },
                    )
                    .map_err(StudioError::new)
            },
        )
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
        retune_failed: status.retune_failed,
        retune_pending: status.retune_pending,
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
