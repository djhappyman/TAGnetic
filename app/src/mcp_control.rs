//! Trusted desktop controls and adapters for the local external-agent service.
//! The connector never constructs an App: it attaches to these existing owners.
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tagrex_commands::{
    CandidateDto, CoverArtDto, ErrorDto, ReleaseDto, SearchQueryDto, SettingsDto,
};
use tagrex_service::{
    DesktopBackend, Grant, ProviderBackend, Service, ServiceError, SettingsBackend,
};
use tauri::{Emitter, Manager, State};

use super::{settings_store, AppState, ProviderState};

pub struct AgentControl {
    service: Arc<Service>,
    profile: PathBuf,
    host: tauri::async_runtime::Mutex<Option<tagrex_mcp::HostHandle>>,
    view_preferences: std::sync::Mutex<Value>,
}

pub struct AgentUnavailable(pub String);

fn service_error(error: impl std::fmt::Display) -> ServiceError {
    ServiceError {
        code: "desktop_error".into(),
        message: error.to_string(),
    }
}

fn service_dto(error: ErrorDto) -> ServiceError {
    ServiceError::new("desktop_error", error.text)
}

fn shell_error(error: impl std::fmt::Display) -> ErrorDto {
    ErrorDto::plain(error.to_string())
}

fn trusted_window(window: &tauri::WebviewWindow) -> Result<(), ErrorDto> {
    if window.label() != "main" {
        return Err(shell_error(
            "Agent connections can only be managed from the main application window.",
        ));
    }
    Ok(())
}

/// Shares both the per-source hub mutex and its current settings snapshot.
/// Network operations run on service blocking workers, never the webview thread.
struct DesktopProviders {
    app: tauri::AppHandle,
    providers: ProviderState,
}

impl DesktopProviders {
    fn token(&self, source: &str) -> Result<String, ServiceError> {
        match source {
            "discogs" => {
                let path = super::token_path(&self.app).map_err(service_dto)?;
                super::tighten_secret(&path);
                match std::fs::read_to_string(path) {
                    Ok(token) => Ok(token.trim().to_owned()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
                    Err(_) => Err(service_error("Discogs credentials could not be read.")),
                }
            }
            "beatport" => tauri::async_runtime::block_on(super::beatport_token(self.app.clone()))
                .map_err(|_| service_error("Beatport needs to be connected again in Settings.")),
            "musicbrainz" | "bandcamp" | "soundeo" => Ok(String::new()),
            _ => Err(service_error("Unknown online source.")),
        }
    }
}

impl ProviderBackend for DesktopProviders {
    fn status(&self) -> Result<Value, ServiceError> {
        let discogs = super::token_path(&self.app)
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .is_some_and(|token| !token.trim().is_empty());
        let beatport = super::read_beatport_session(&self.app).is_some();
        Ok(
            json!({"discogs":{"connected":discogs},"beatport":{"connected":beatport},
            "musicbrainz":{"connected":true},"bandcamp":{"connected":true},"soundeo":{"connected":true}}),
        )
    }

    fn search(
        &self,
        source: &str,
        query: &SearchQueryDto,
    ) -> Result<Vec<CandidateDto>, ServiceError> {
        let token = self.token(source)?;
        let queue = self.providers.source(source).map_err(service_dto)?;
        super::with_providers(&queue, |hub| {
            hub.provider_search(source, &token, query)
                .map_err(ErrorDto::from)
        })
        .map_err(|_| service_error("The online source could not complete this search."))
    }

    fn fetch_release(&self, source: &str, id: &str) -> Result<ReleaseDto, ServiceError> {
        let token = self.token(source)?;
        let queue = self.providers.source(source).map_err(service_dto)?;
        super::with_providers(&queue, |hub| {
            hub.provider_fetch_release(source, &token, id)
                .map_err(ErrorDto::from)
        })
        .map_err(|_| service_error("The online source could not load this release."))
    }

    fn fetch_image(&self, source: &str, url: &str) -> Result<CoverArtDto, ServiceError> {
        let queue = self.providers.source(source).map_err(service_dto)?;
        if !queue
            .settings
            .lock()
            .map_err(service_error)?
            .proxy
            .trim()
            .is_empty()
        {
            return Err(service_error("Agent artwork downloads require a direct connection so destination addresses can be checked."));
        }
        let url = url.to_owned();
        tauri::async_runtime::block_on(super::IMAGE_ADMISSION.run(move || {
            super::secure_provider_image::fetch(&url)
                .map_err(|error| ErrorDto::plain(error.message))
        }))
        .map_err(service_dto)
    }
}

struct DesktopSettings {
    app: tauri::AppHandle,
    state: AppState,
    providers: ProviderState,
}

struct DesktopActions {
    app: tauri::AppHandle,
}

impl DesktopBackend for DesktopActions {
    fn bpm(
        &self,
        paths: &[PathBuf],
        cancel: &std::sync::atomic::AtomicBool,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Value, ServiceError> {
        use std::sync::atomic::Ordering;
        let settings = super::read_settings_quietly(&self.app);
        let limit = (settings.bpm_max_minutes != 0)
            .then(|| std::time::Duration::from_secs(u64::from(settings.bpm_max_minutes) * 60));
        let mut detected = Vec::new();
        let mut failed = Vec::new();
        let mut no_tempo = 0;
        let mut skipped = 0;
        progress(0, paths.len());
        if cancel.load(Ordering::Acquire) {
            return Ok(json!({"detected":[],"cancelled":true,"skipped":0,"failed":[]}));
        }
        let mut detector = tagrex_player::tempo::TempoDetector::new().map_err(service_error)?;
        for (index, path) in paths.iter().enumerate() {
            progress(index, paths.len());
            if cancel.load(Ordering::Acquire) {
                break;
            }
            if tagrex_commands::too_long_for_bpm(path, limit) {
                skipped += 1;
                progress(index + 1, paths.len());
                continue;
            }
            match detector.detect(path, cancel) {
                Ok(Some(bpm)) => detected.push(json!({"path":path,"bpm":tagrex_commands::bpm_in_range(bpm,settings.bpm_floor)})),
                Ok(None) => no_tempo += 1,
                Err(tagrex_player::tempo::TempoError::Failed(_)) => failed.push(path),
                Err(tagrex_player::tempo::TempoError::Cancelled) => break,
            }
            progress(index + 1, paths.len());
        }
        Ok(
            json!({"detected":detected,"skipped":skipped,"failed":failed,"no_tempo":no_tempo,"cancelled":cancel.load(Ordering::Acquire)}),
        )
    }

    fn call(&self, action: &str, args: Value) -> Result<Value, ServiceError> {
        let player = self.app.state::<tagrex_player::Player>();
        match action {
            "playback.status" => serde_json::to_value(player.status()).map_err(service_error),
            "playback.play" => {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| service_error("Choose a loaded file to play."))?;
                // Service checks the grant; this additionally refuses a file
                // outside the currently open session.
                let state = self.app.state::<AppState>();
                let session = state.lock().map_err(service_error)?;
                let app = session
                    .as_ref()
                    .ok_or_else(|| service_error("No files are open."))?;
                app.read_track(std::path::Path::new(path))
                    .map_err(service_error)?;
                player.play(PathBuf::from(path));
                Ok(json!({"accepted":true}))
            }
            "playback.pause" => {
                player.pause();
                Ok(json!({"accepted":true}))
            }
            "playback.resume" => {
                player.resume();
                Ok(json!({"accepted":true}))
            }
            "playback.stop" => {
                player.stop();
                Ok(json!({"accepted":true}))
            }
            "playback.seek" => {
                let seconds = args
                    .get("seconds")
                    .or_else(|| args.get("secs"))
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .ok_or_else(|| service_error("Seek position must be nonnegative seconds."))?;
                player.seek(seconds);
                Ok(json!({"accepted":true}))
            }
            "playback.volume" => {
                let level = args
                    .get("level")
                    .and_then(Value::as_f64)
                    .filter(|value| (0.0..=1.0).contains(value))
                    .ok_or_else(|| service_error("Volume must be between zero and one."))?;
                player.set_volume(level as f32);
                Ok(json!({"accepted":true}))
            }
            "session.open" => {
                let control = self.app.state::<AgentControl>();
                if control
                    .service
                    .manual_plan()
                    .map_err(service_error)?
                    .is_some()
                {
                    return Err(ServiceError::new("human_work_pending", "Finish or discard the person's staged edits before replacing the open files."));
                }
                if let Some(paths) = args.get("paths").and_then(Value::as_array) {
                    let paths = paths
                        .iter()
                        .map(|path| {
                            path.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| service_error("File paths must be strings."))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let result = tauri::async_runtime::block_on(super::open_drop(
                        self.app.state::<AppState>(),
                        self.app.clone(),
                        paths,
                    ))
                    .map_err(service_dto)?;
                    let _ = self.app.emit(
                        "tagrex://agent-event",
                        json!({"kind":"session_changed","data":{"root":result.root}}),
                    );
                    serde_json::to_value(result).map_err(service_error)
                } else {
                    let root = args
                        .get("root")
                        .and_then(Value::as_str)
                        .ok_or_else(|| service_error("An approved folder is required."))?
                        .to_owned();
                    tauri::async_runtime::block_on(super::open_library(
                        self.app.state::<AppState>(),
                        self.app.clone(),
                        root.clone(),
                    ))
                    .map_err(service_dto)?;
                    let _ = self.app.emit(
                        "tagrex://agent-event",
                        json!({"kind":"session_changed","data":{"root":root}}),
                    );
                    Ok(json!({"root":root}))
                }
            }
            "session.refresh" => {
                let rows = tauri::async_runtime::block_on(super::list_tracks(
                    self.app.state::<AppState>(),
                ))
                .map_err(service_dto)?;
                let _ = self.app.emit(
                    "tagrex://agent-event",
                    json!({"kind":"session_changed","data":{"refresh":true}}),
                );
                Ok(json!({"total":rows.len()}))
            }
            "ui.present" => {
                if let Some(module) = args.get("module").and_then(Value::as_str) {
                    if ![
                        "tagger",
                        "renamer",
                        "from-name",
                        "from-text",
                        "exporter",
                        "generator",
                        "deduplicator",
                    ]
                    .contains(&module)
                    {
                        return Err(service_error("Unknown workspace mode."));
                    }
                }
                let _ = self.app.emit(
                    "tagrex://agent-event",
                    json!({"kind":"ui_present","data":args}),
                );
                Ok(json!({"accepted":true}))
            }
            "ui.preferences" => {
                let values = args
                    .as_object()
                    .ok_or_else(|| service_error("Display preferences must be an object."))?;
                if values.is_empty() {
                    return Ok(self
                        .app
                        .state::<AgentControl>()
                        .view_preferences
                        .lock()
                        .map_err(service_error)?
                        .clone());
                }
                if values.keys().any(|key| {
                    !["theme", "palette", "value_font", "table_font_px"].contains(&key.as_str())
                }) {
                    return Err(service_error("Unsupported display preference."));
                }
                if values.get("theme").is_some_and(|value| {
                    !["light", "dark", "auto"].contains(&value.as_str().unwrap_or(""))
                }) {
                    return Err(service_error("Unknown theme mode."));
                }
                if values.get("value_font").is_some_and(|value| {
                    !["mono", "sans", "condensed"].contains(&value.as_str().unwrap_or(""))
                }) {
                    return Err(service_error("Unknown value font."));
                }
                if values
                    .get("table_font_px")
                    .is_some_and(|value| !value.as_u64().is_some_and(|px| (10..=20).contains(&px)))
                {
                    return Err(service_error("Table text size must be from 10 to 20."));
                }
                let _ = self.app.emit(
                    "tagrex://agent-event",
                    json!({"kind":"ui_preferences","data":args}),
                );
                Ok(json!({"accepted":true}))
            }
            _ => Err(ServiceError::new(
                "unsupported",
                "This desktop action is not available.",
            )),
        }
    }
}

impl SettingsBackend for DesktopSettings {
    fn read(&self) -> Result<Value, ServiceError> {
        let _document = settings_store::DOCUMENT_LOCK
            .lock()
            .map_err(service_error)?;
        let path = super::settings_path(&self.app).map_err(service_dto)?;
        settings_document(&path).map_err(service_dto)
    }

    fn compare_and_write(&self, expected_revision: &str, value: Value) -> Result<(), ServiceError> {
        let _document = settings_store::DOCUMENT_LOCK
            .lock()
            .map_err(service_error)?;
        let path = super::settings_path(&self.app).map_err(service_dto)?;
        let current = settings_document(&path).map_err(service_dto)?;
        if tagrex_service::settings_revision(&current) != expected_revision {
            return Err(ServiceError {
                code: "settings_conflict".into(),
                message: "Saved settings changed. Read them again before saving.".into(),
            });
        }
        let settings: SettingsDto = serde_json::from_value(value.clone()).map_err(service_error)?;
        settings_store::save_with_schema::<SettingsDto, _>(&path, &value).map_err(service_error)?;
        self.providers
            .apply_settings(&settings)
            .map_err(service_dto)?;
        super::SETTINGS_REVISION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(app) = self.state.lock().map_err(service_error)?.as_ref() {
            app.apply_settings(&settings);
        }
        let _ = self.app.emit("tagrex://agent-settings-changed", ());
        Ok(())
    }
}

/// Normalize known defaults while retaining newer document slices.
pub(super) fn settings_document(path: &std::path::Path) -> Result<Value, ErrorDto> {
    let raw = settings_store::read::<Value>(path)
        .map_err(shell_error)?
        .unwrap_or_else(|| json!({}));
    let settings: SettingsDto = serde_json::from_value(raw.clone()).map_err(shell_error)?;
    let mut document = raw
        .as_object()
        .cloned()
        .ok_or_else(|| shell_error("Settings must be an object."))?;
    let normalized = serde_json::to_value(settings).map_err(shell_error)?;
    if let Some(values) = normalized.as_object() {
        document.extend(values.clone());
    }
    Ok(Value::Object(document))
}

pub fn install(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let profile = app.path().app_config_dir()?;
    let service = Arc::new(Service::new(
        app.state::<AppState>().inner().clone(),
        profile.join("mcp").join("service"),
    )?);
    service.set_provider_backend(Arc::new(DesktopProviders {
        app: app.clone(),
        providers: app.state::<ProviderState>().inner().clone(),
    }))?;
    let settings = Arc::new(DesktopSettings {
        app: app.clone(),
        state: app.state::<AppState>().inner().clone(),
        providers: app.state::<ProviderState>().inner().clone(),
    });
    service.set_settings_backend(settings.clone())?;
    service.set_desktop_backend(Arc::new(DesktopActions { app: app.clone() }))?;
    let handle = app.clone();
    service.set_event_sink(Arc::new(move |name, value| {
        let _ = handle.emit("tagrex://agent-event", json!({"kind":name,"data":value}));
    }))?;
    let enabled = service.enabled()?;
    app.manage(AgentControl {
        service,
        profile,
        host: tauri::async_runtime::Mutex::new(None),
        view_preferences: std::sync::Mutex::new(json!({})),
    });
    if enabled {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = set_enabled(&handle.state::<AgentControl>(), true).await {
                eprintln!("TAGnetic MCP could not start: {}", error.text);
            }
        });
    }
    Ok(())
}

#[tauri::command]
pub async fn mcp_status(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Value, ErrorDto> {
    trusted_window(&window)?;
    let Some(control) = app.try_state::<AgentControl>() else {
        return Ok(
            json!({"listening":false,"available":false,"clients":[],"operations":{"operations":[]},
            "error":app.try_state::<AgentUnavailable>().map(|reason| reason.0.clone()).unwrap_or_else(|| "Agent connections are unavailable.".into())}),
        );
    };
    let service = control.service.clone();
    let profile = control.profile.clone();
    let listening = control.host.lock().await.is_some();
    tauri::async_runtime::spawn_blocking(move || {
        let mut clients = service.list_clients().map_err(shell_error)?;
        let drafts = service.drafts().map_err(shell_error)?;
        if let Some(list) = clients.get_mut("clients").and_then(Value::as_array_mut) {
            for client in list {
                let own: Vec<_> = drafts
                    .get("drafts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|draft| draft.get("principal_id") == client.get("id"))
                    .cloned()
                    .collect();
                client["drafts"] = Value::Array(own);
            }
        }
        // Recovery may need the original folder reopened, or a fresh engine
        // after a worker panic. Keep trusted connection/Stop controls usable
        // while that evidence is unavailable; never clear the write freeze.
        let recovery = service.recovery_report().unwrap_or_else(|error| json!({
            "recovery_required":true,"operations":[],"guidance":error.message,"error_code":error.code,
        }));
        Ok(
            json!({"listening":listening,"shared_staging":true,"clients":clients,"drafts":drafts,
            "operations":service.operations().map_err(shell_error)?,"analysis":service.analysis_jobs().map_err(shell_error)?,
            "recovery":recovery,
            "profile":profile,"executable":std::env::current_exe().map_err(shell_error)?}),
        )
    })
    .await
    .map_err(shell_error)?
}

#[tauri::command]
pub async fn mcp_set_enabled(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    enabled: bool,
) -> Result<(), ErrorDto> {
    trusted_window(&window)?;
    set_enabled(&control, enabled).await
}

async fn set_enabled(control: &AgentControl, enabled: bool) -> Result<(), ErrorDto> {
    let mut host = control.host.lock().await;
    control.service.set_enabled(enabled).map_err(shell_error)?;
    if enabled && host.is_none() {
        match tagrex_mcp::start_host(&control.profile, control.service.clone()).await {
            Ok(started) => *host = Some(started),
            Err(error) => {
                control.service.set_enabled(false).map_err(shell_error)?;
                return Err(shell_error(error));
            }
        }
    } else if !enabled {
        control.service.stop_all().map_err(shell_error)?;
        if let Some(old) = host.take() {
            old.shutdown().await;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn mcp_pair_client(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    label: String,
    grant: Grant,
) -> Result<Value, ErrorDto> {
    trusted_window(&window)?;
    let service = control.service.clone();
    let profile = control.profile.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir = profile.join("mcp").join("credentials");
        tagrex_service::private_storage::ensure_private_dir(&dir).map_err(shell_error)?;
        let paired = service.pair_client(label, grant).map_err(shell_error)?;
        let file = dir.join(format!("{}.token", paired.principal_id));
        // Only this trusted path gets the credential; the UI displays its path.
        if let Err(error) = tagrex_mcp::write_credential_file(&file, &paired.credential) {
            let _ = service.revoke_client(&paired.principal_id);
            return Err(shell_error(error));
        }
        Ok(
            json!({"principal_id":paired.principal_id,"credential_file":file,"profile":profile,
            "executable":std::env::current_exe().map_err(shell_error)?}),
        )
    })
    .await
    .map_err(shell_error)?
}

#[tauri::command]
pub async fn mcp_revoke_client(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    principal_id: String,
) -> Result<(), ErrorDto> {
    trusted_window(&window)?;
    let service = control.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.revoke_client(&principal_id).map_err(shell_error)
    })
    .await
    .map_err(shell_error)?
}

#[tauri::command]
pub fn mcp_stop_all(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
) -> Result<(), ErrorDto> {
    trusted_window(&window)?;
    control.service.stop_all().map_err(shell_error)
}

#[tauri::command]
pub async fn mcp_review_draft(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    principal_id: String,
    draft_id: String,
) -> Result<Value, ErrorDto> {
    trusted_window(&window)?;
    let service = control.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service
            .draft_review(&principal_id, &draft_id)
            .map_err(shell_error)
    })
    .await
    .map_err(shell_error)?
}

#[tauri::command]
pub async fn mcp_adopt_draft(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    principal_id: String,
    draft_id: String,
) -> Result<tagrex_commands::PlanDto, ErrorDto> {
    trusted_window(&window)?;
    let service = control.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if service.manual_plan().map_err(shell_error)?.is_some() {
            return Err(shell_error(
                "Finish or discard your current staged edits before adopting an agent draft.",
            ));
        }
        service
            .draft_plan(&principal_id, &draft_id)
            .map_err(shell_error)
    })
    .await
    .map_err(shell_error)?
}

#[tauri::command]
pub fn mcp_cancel_operation(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    operation_id: String,
) -> Result<Value, ErrorDto> {
    trusted_window(&window)?;
    if operation_id.starts_with("scan_") {
        control
            .service
            .cancel_analysis_job(&operation_id)
            .map_err(shell_error)?;
        Ok(json!({"cancel_requested":true}))
    } else {
        control
            .service
            .cancel_operation(&operation_id)
            .map_err(shell_error)
    }
}

#[tauri::command]
pub async fn mcp_reconcile_operation(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    operation_id: String,
    proof: String,
    action: String,
) -> Result<Value, ErrorDto> {
    trusted_window(&window)?;
    if !["accept_completed", "undo_correlated", "close_not_written"].contains(&action.as_str()) {
        return Err(shell_error("Unknown recovery action."));
    }
    let service = control.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service
            .reconcile_operation(&operation_id, &proof, &action)
            .map_err(shell_error)
    })
    .await
    .map_err(shell_error)?
}

#[tauri::command]
pub fn mcp_set_manual_plan(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    plan: Option<tagrex_commands::PlanDto>,
) -> Result<u64, ErrorDto> {
    trusted_window(&window)?;
    control.service.set_manual_plan(plan).map_err(shell_error)
}

#[tauri::command]
pub async fn mcp_shared_stage(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
) -> Result<tagrex_service::SharedStageSnapshot, ErrorDto> {
    trusted_window(&window)?;
    let service = control.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.shared_stage().map_err(shell_error))
        .await
        .map_err(shell_error)?
}

#[tauri::command]
pub async fn mcp_publish_shared_stage(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    plan: Option<tagrex_commands::PlanDto>,
    expected_revision: u64,
    touched: Vec<tagrex_service::StageFieldRef>,
) -> Result<tagrex_service::SharedStageSnapshot, ErrorDto> {
    trusted_window(&window)?;
    let service = control.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service
            .publish_manual_plan_with_intents(plan, expected_revision, touched)
            .map_err(shell_error)
    })
    .await
    .map_err(shell_error)?
}

#[tauri::command]
pub async fn mcp_clear_shared_stage(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    expected_revision: u64,
) -> Result<tagrex_service::SharedStageSnapshot, ErrorDto> {
    trusted_window(&window)?;
    let service = control.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service
            .clear_shared_stage(expected_revision)
            .map_err(shell_error)
    })
    .await
    .map_err(shell_error)?
}

pub fn complete_desktop_apply(
    app: &tauri::AppHandle,
    plan: &tagrex_commands::PlanDto,
    revision: Option<u64>,
) -> Result<(), ErrorDto> {
    let Some(control) = app.try_state::<AgentControl>() else {
        return Ok(());
    };
    let revision = match revision {
        Some(revision) => revision,
        None => {
            control
                .service
                .shared_stage()
                .map_err(shell_error)?
                .revision
        }
    };
    control
        .service
        .complete_shared_apply(plan, revision)
        .map_err(shell_error)?;
    Ok(())
}

pub fn resolve_desktop_apply(
    app: &tauri::AppHandle,
    plan: tagrex_commands::PlanDto,
    revision: Option<u64>,
) -> Result<(tagrex_commands::PlanDto, Option<u64>), ErrorDto> {
    if revision.is_none() {
        return Ok((plan, None));
    }
    let Some(control) = app.try_state::<AgentControl>() else {
        return Ok((plan, None));
    };
    let paths = plan
        .changes
        .iter()
        .map(|change| change.path.clone())
        .collect();
    control
        .service
        .shared_apply_plan_with_revision(&paths)
        .map(|(plan, revision)| (plan, Some(revision)))
        .map_err(shell_error)
}

#[tauri::command]
pub async fn mcp_rebase_manual_plan(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    plan: tagrex_commands::PlanDto,
    paths: Vec<String>,
) -> Result<tagrex_commands::PlanDto, ErrorDto> {
    trusted_window(&window)?;
    let state = state.inner().clone();
    let generation = super::LIBRARY_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
    super::SESSION_QUEUE
        .run(move || {
            if generation != super::LIBRARY_GENERATION.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(shell_error(
                    "Open files changed while refreshing the staged edits.",
                ));
            }
            let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
            super::with_app(&state, |app| {
                app.rebase_staged_plan(&plan, &paths)
                    .map_err(ErrorDto::from)
            })
        })
        .await
}

#[tauri::command]
pub fn mcp_set_view_preferences(
    window: tauri::WebviewWindow,
    control: State<'_, AgentControl>,
    preferences: Value,
) -> Result<(), ErrorDto> {
    trusted_window(&window)?;
    if serde_json::to_vec(&preferences).map_err(shell_error)?.len() > 8192 {
        return Err(shell_error("Display preference snapshot is too large."));
    }
    *control.view_preferences.lock().map_err(shell_error)? = preferences;
    Ok(())
}

#[tauri::command]
pub async fn mcp_patch_query_presets(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    upsert: Vec<Value>,
    remove: Vec<String>,
) -> Result<Value, ErrorDto> {
    trusted_window(&window)?;
    if upsert.len() > 256 || remove.len() > 256 {
        return Err(shell_error("Too many filter presets in one update."));
    }
    let backend = Arc::new(DesktopSettings {
        app: app.clone(),
        state: app.state::<AppState>().inner().clone(),
        providers: app.state::<ProviderState>().inner().clone(),
    });
    // Reusable query presets are changed through the same settings owner as
    // agent writes. The command is trusted, but still uses a revision check.
    tauri::async_runtime::spawn_blocking(move || {
        let mut document = backend.read().map_err(shell_error)?;
        let expected = tagrex_service::settings_revision(&document);
        let mut presets = document
            .get("query_presets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for preset in &upsert {
            let name = preset.get("name").and_then(Value::as_str).unwrap_or("");
            if name.trim().is_empty()
                || name.len() > 128
                || serde_json::to_vec(preset).map_err(shell_error)?.len() > 65536
            {
                return Err(shell_error(
                    "A filter preset needs a short name and a bounded query.",
                ));
            }
        }
        presets.retain(|preset| {
            let name = preset.get("name").and_then(Value::as_str).unwrap_or("");
            !remove.iter().any(|removed| removed == name)
                && !upsert
                    .iter()
                    .any(|new| new.get("name").and_then(Value::as_str) == Some(name))
        });
        presets.extend(upsert);
        presets.sort_by(|a, b| {
            a.get("name")
                .and_then(Value::as_str)
                .cmp(&b.get("name").and_then(Value::as_str))
        });
        if presets.len() > 1024 {
            return Err(shell_error(
                "Remove obsolete filter presets before adding more.",
            ));
        }
        document["query_presets"] = Value::Array(presets);
        backend
            .compare_and_write(&expected, document.clone())
            .map_err(shell_error)?;
        let revision = tagrex_service::settings_revision(&document);
        document["_revision"] = Value::String(revision);
        Ok(document)
    })
    .await
    .map_err(shell_error)?
}
