//! One authorized service over the desktop's existing session and journal.
//!
//! External connections have private selections and sealed plans. The service
//! never constructs a second `App`, accepts an executable client-authored plan,
//! exposes account credentials, or treats an MCP roots list as authorization.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tagrex_commands::{
    App, CandidateDto, CoverArtDto, PlanDto, ReleaseDto, SearchQueryDto, TrackDto,
};

mod artwork_policy;
mod catalog;
mod domains;
mod outputs;
mod plans;
pub mod private_storage;
mod recovery;
mod staging;
pub use catalog::{tool_catalog, ToolDefinition};
pub use staging::{
    SharedStageSnapshot, StageArtworkCover, StageArtworkDelta, StageBlockDelta, StageDelta,
    StageFieldDelta, StageFieldRef, StageIntent, StageIntentOutcome, StageIntentRequest,
    StageRenameDelta, StageTagDelta,
};

pub type SharedApp = Arc<Mutex<Option<App>>>;
pub type ServiceResult<T> = Result<T, ServiceError>;
type EventSink = Arc<dyn Fn(&str, Value) + Send + Sync>;
const MAX_HANDLES: usize = 256;
const MAX_ARGUMENT_BYTES: usize = 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PAGE: usize = 500;
const MAX_SELECTION_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceError {
    pub code: String,
    pub message: String,
}
impl ServiceError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_argument", message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("conflict", message)
    }
}
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ServiceError {}
impl From<std::io::Error> for ServiceError {
    fn from(_: std::io::Error) -> Self {
        Self::new(
            "storage_error",
            "Application service storage is unavailable",
        )
    }
}
impl From<serde_json::Error> for ServiceError {
    fn from(error: serde_json::Error) -> Self {
        Self::invalid(error.to_string())
    }
}
impl From<tagrex_commands::AppError> for ServiceError {
    fn from(error: tagrex_commands::AppError) -> Self {
        // Provider responses may contain credential-bearing request URLs. Do not
        // propagate their error bodies; the provider adapter supplies safe errors.
        let code = match &error {
            tagrex_commands::AppError::Plan(tagrex_core::plan::PlanError::RollbackIncomplete {
                ..
            }) => "recovery_required",
            tagrex_commands::AppError::Plan(tagrex_core::plan::PlanError::Cancelled) => {
                "cancelled_and_restored"
            }
            tagrex_commands::AppError::Plan(
                tagrex_core::plan::PlanError::Stale(_)
                | tagrex_core::plan::PlanError::ChangedSinceApplied(_),
            ) => "stale_file",
            tagrex_commands::AppError::Plan(tagrex_core::plan::PlanError::OutsideRoot(_))
            | tagrex_commands::AppError::OutsideRoot(_) => "permission_denied",
            tagrex_commands::AppError::Plan(tagrex_core::plan::PlanError::RenameCollision(_)) => {
                "collision"
            }
            tagrex_commands::AppError::Provider(_) => {
                return Self::new(
                    "provider_error",
                    "The connected source request failed; account credentials are not returned",
                )
            }
            _ => "application_error",
        };
        Self::new(code, error.to_string())
    }
}

/// Issued only by a trusted desktop action. Empty field/provider sets mean no
/// field/provider restriction; an empty root set never grants filesystem access.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub roots: Vec<PathBuf>,
    /// Optional exact-file write scope; empty means the explicitly granted roots.
    #[serde(default)]
    pub write_files: BTreeSet<PathBuf>,
    pub actions: BTreeSet<String>,
    #[serde(default)]
    pub fields: BTreeSet<String>,
    #[serde(default)]
    pub providers: BTreeSet<String>,
    pub max_files: usize,
    #[serde(default)]
    pub expires_at: Option<u64>,
}
impl Default for Grant {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            write_files: BTreeSet::new(),
            actions: ["read".into(), "stage".into()].into(),
            fields: BTreeSet::new(),
            providers: BTreeSet::new(),
            max_files: 100_000,
            expires_at: None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Principal {
    id: String,
    epoch: u64,
}
impl Principal {
    pub fn id(&self) -> &str {
        &self.id
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Pairing {
    pub principal_id: String,
    pub credential: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct ClientRecord {
    id: String,
    label: String,
    credential_hash: String,
    epoch: u64,
    revoked: bool,
    grant: Grant,
}
#[derive(Default, Serialize, Deserialize)]
struct SecurityStore {
    enabled: bool,
    clients: Vec<ClientRecord>,
}

/// Trusted adapters use the same saved-settings owner as the desktop. The CAS
/// revision is SHA-256 of the serialized JSON value (`settings_revision`).
pub trait SettingsBackend: Send + Sync {
    fn read(&self) -> ServiceResult<Value>;
    fn compare_and_write(&self, expected_revision: &str, value: Value) -> ServiceResult<()>;
}
/// Keeps all provider credentials, clients, rate limits and refresh logic in
/// the existing desktop provider owner. Errors must be redacted by the adapter.
pub trait ProviderBackend: Send + Sync {
    fn status(&self) -> ServiceResult<Value>;
    fn search(&self, source: &str, query: &SearchQueryDto) -> ServiceResult<Vec<CandidateDto>>;
    fn fetch_release(&self, source: &str, id: &str) -> ServiceResult<ReleaseDto>;
    fn fetch_image(&self, source: &str, url: &str) -> ServiceResult<CoverArtDto>;
}
/// Optional shell capabilities are explicitly enumerated by the desktop owner.
/// There is intentionally no arbitrary invoke/eval/shell command endpoint.
pub trait DesktopBackend: Send + Sync {
    fn call(&self, action: &str, args: Value) -> ServiceResult<Value>;
    fn bpm(
        &self,
        _paths: &[PathBuf],
        _cancel: &AtomicBool,
        _progress: &mut dyn FnMut(usize, usize),
    ) -> ServiceResult<Value> {
        Err(ServiceError::new(
            "unavailable",
            "This host does not provide native BPM analysis",
        ))
    }
}

#[derive(Clone)]
struct Snapshot {
    owner: String,
    id: String,
    rows: Arc<Vec<TrackDto>>,
    index: Arc<tagrex_commands::query::QueryIndex>,
}
#[derive(Clone)]
struct Selection {
    owner: String,
    paths: Vec<PathBuf>,
    bytes: usize,
}
#[derive(Clone)]
struct Asset {
    owner: String,
    cover: CoverArtDto,
    digest: String,
    bytes: usize,
    width: u32,
    height: u32,
}
#[derive(Clone)]
struct ReleaseReceipt {
    owner: String,
    source: String,
    release: ReleaseDto,
}
#[derive(Clone)]
struct ImageReceipt {
    owner: String,
    source: String,
    url: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct OperationRecord {
    id: String,
    owner: String,
    key_hash: String,
    digest: String,
    plan_id: String,
    status: String,
    processed: usize,
    total: usize,
    result: Option<Value>,
    error: Option<ServiceError>,
    #[serde(default)]
    interrupted_phase: Option<String>,
    #[serde(default)]
    library_root: Option<PathBuf>,
    #[serde(default)]
    accepted_digest: Option<String>,
    #[serde(default)]
    scope_digest: Option<String>,
    #[serde(default)]
    scope_id: Option<String>,
}
struct Operation {
    record: Mutex<OperationRecord>,
    cancel: AtomicBool,
}
struct State {
    security: SecurityStore,
    snapshots: HashMap<String, Snapshot>,
    snapshot_cache: Option<(u64, Arc<Vec<TrackDto>>)>,
    selections: HashMap<String, Selection>,
    query_lanes: HashMap<(String, String), (String, Arc<AtomicBool>)>,
    plans: HashMap<String, plans::SealedPlan>,
    assets: HashMap<String, Asset>,
    releases: HashMap<String, ReleaseReceipt>,
    image_receipts: HashMap<String, ImageReceipt>,
    operations: HashMap<String, Arc<Operation>>,
    manual_plan: Option<PlanDto>,
    manual_revision: u64,
    stage: staging::StageState,
    recovery_required: bool,
    redo_revision: u64,
}
struct Inner {
    app: SharedApp,
    directory: PathBuf,
    epoch: String,
    state: Mutex<State>,
    settings: Mutex<Option<Arc<dyn SettingsBackend>>>,
    providers: Mutex<Option<Arc<dyn ProviderBackend>>>,
    desktop: Mutex<Option<Arc<dyn DesktopBackend>>>,
    events: Mutex<Option<EventSink>>,
    mutation: Mutex<()>,
    build_snapshot: Mutex<()>,
    _profile_lease: File,
    #[cfg(test)]
    fail_next_spawn: AtomicBool,
    #[cfg(test)]
    panic_after_apply: AtomicBool,
}
#[derive(Clone)]
pub struct Service(Arc<Inner>);
impl Service {
    /// Attach to the existing session. `data_dir` is service-owned, private
    /// persistent state under the application profile, never a media directory.
    pub fn new(app: SharedApp, data_dir: PathBuf) -> ServiceResult<Self> {
        private_storage::ensure_private_dir(&data_dir)?;
        let profile_lease = private_storage::open_private_lock(&data_dir.join("profile.lock"))?;
        profile_lease.try_lock().map_err(|_| {
            ServiceError::new(
                "profile_busy",
                "Another service owns this profile; connect to its existing desktop",
            )
        })?;
        let security: SecurityStore =
            read_json(&data_dir.join("clients.json"))?.unwrap_or_default();
        let previous: Vec<OperationRecord> =
            read_json(&data_dir.join("operations.json"))?.unwrap_or_default();
        let mut recovery_required = false;
        let operations = previous.into_iter().map(|mut record| {
            if record.status=="recovery_required" {recovery_required=true;}
            if matches!(record.status.as_str(), "queued" | "preflight" | "writing" | "rolling_back" | "undoing" | "redoing") {
                record.interrupted_phase=Some(record.status.clone());
                record.status = "recovery_required".into();
                record.error = Some(ServiceError::new("unknown_outcome", "The application stopped before this operation had a durable terminal receipt; inspect journal before retrying"));
                recovery_required = true;
            }
            (record.id.clone(), Arc::new(Operation { record: Mutex::new(record), cancel: AtomicBool::new(false) }))
        }).collect();
        let mut stage = staging::StageState::default();
        // The stage session identifies this process' live stage to delta
        // callers; it is never persisted and never an authorization token.
        stage.session = opaque("stage")?;
        Ok(Self(Arc::new(Inner {
            app,
            directory: data_dir,
            epoch: opaque("epoch")?,
            state: Mutex::new(State {
                security,
                snapshots: HashMap::new(),
                snapshot_cache: None,
                selections: HashMap::new(),
                query_lanes: HashMap::new(),
                plans: HashMap::new(),
                assets: HashMap::new(),
                releases: HashMap::new(),
                image_receipts: HashMap::new(),
                operations,
                manual_plan: None,
                manual_revision: 0,
                stage,
                recovery_required,
                redo_revision: 0,
            }),
            settings: Mutex::new(None),
            providers: Mutex::new(None),
            desktop: Mutex::new(None),
            events: Mutex::new(None),
            mutation: Mutex::new(()),
            build_snapshot: Mutex::new(()),
            _profile_lease: profile_lease,
            #[cfg(test)]
            fail_next_spawn: AtomicBool::new(false),
            #[cfg(test)]
            panic_after_apply: AtomicBool::new(false),
        })))
    }
    pub fn tool_catalog() -> Vec<ToolDefinition> {
        tool_catalog()
    }
    pub fn set_settings_backend(&self, backend: Arc<dyn SettingsBackend>) -> ServiceResult<()> {
        *lock(&self.0.settings)? = Some(backend);
        Ok(())
    }
    pub fn set_provider_backend(&self, backend: Arc<dyn ProviderBackend>) -> ServiceResult<()> {
        *lock(&self.0.providers)? = Some(backend);
        Ok(())
    }
    pub fn set_desktop_backend(&self, backend: Arc<dyn DesktopBackend>) -> ServiceResult<()> {
        *lock(&self.0.desktop)? = Some(backend);
        Ok(())
    }
    pub fn set_event_sink(&self, sink: EventSink) -> ServiceResult<()> {
        *lock(&self.0.events)? = Some(sink);
        Ok(())
    }
    pub fn set_enabled(&self, enabled: bool) -> ServiceResult<()> {
        let mut state = lock(&self.0.state)?;
        state.security.enabled = enabled;
        if !enabled {
            for (_, lane) in state.query_lanes.values() {
                lane.store(true, Ordering::Release);
            }
            for operation in state.operations.values() {
                operation.cancel.store(true, Ordering::Release);
            }
        }
        write_json(&self.0.directory.join("clients.json"), &state.security)?;
        drop(state);
        if !enabled {
            self.stop_analysis()?;
        }
        self.emit("service_changed", json!({"enabled":enabled}));
        Ok(())
    }
    pub fn enabled(&self) -> ServiceResult<bool> {
        Ok(lock(&self.0.state)?.security.enabled)
    }
    pub fn pair_client(&self, label: String, mut grant: Grant) -> ServiceResult<Pairing> {
        if label.trim().is_empty()
            || label.len() > 128
            || grant.max_files == 0
            || grant.max_files > 1_000_000
        {
            return Err(ServiceError::invalid(
                "Provide a short client label and a 1..1000000 file budget",
            ));
        }
        let known = [
            "read",
            "stage",
            "write",
            "providers",
            "settings",
            "outputs",
            "history",
            "open",
            "trash",
            "playback",
            "ui",
        ];
        if grant.actions.iter().any(|a| !known.contains(&a.as_str())) {
            return Err(ServiceError::invalid("Unknown grant action"));
        }
        for root in &mut grant.roots {
            *root = fs::canonicalize(&*root)?;
            if !root.is_dir() {
                return Err(ServiceError::invalid(
                    "Grant roots must be existing directories",
                ));
            }
        }
        let mut write_files = BTreeSet::new();
        for path in &grant.write_files {
            let canonical = fs::canonicalize(path)?;
            if !canonical.is_file() || !grant.roots.iter().any(|r| canonical.starts_with(r)) {
                return Err(ServiceError::invalid(
                    "Exact write files must be existing files inside approved roots",
                ));
            }
            write_files.insert(canonical);
        }
        grant.write_files = write_files;
        let credential = opaque("tagnetic")?;
        let id = opaque("client")?;
        let record = ClientRecord {
            id: id.clone(),
            label,
            credential_hash: hash(credential.as_bytes()),
            epoch: 1,
            revoked: false,
            grant,
        };
        let mut state = lock(&self.0.state)?;
        if state.security.clients.len() >= 64 {
            return Err(ServiceError::new(
                "quota",
                "Revoke/remove obsolete clients before pairing more",
            ));
        }
        state.security.clients.push(record);
        write_json(&self.0.directory.join("clients.json"), &state.security)?;
        drop(state);
        self.emit("service_changed", json!({"paired":id}));
        Ok(Pairing {
            principal_id: id,
            credential,
        })
    }
    pub fn list_clients(&self) -> ServiceResult<Value> {
        let state = lock(&self.0.state)?;
        Ok(
            json!({"enabled":state.security.enabled,"clients":state.security.clients.iter().map(|c| json!({"id":c.id,"label":c.label,"grant":c.grant,"revoked":c.revoked})).collect::<Vec<_>>() }),
        )
    }
    pub fn revoke_client(&self, id: &str) -> ServiceResult<()> {
        let mut state = lock(&self.0.state)?;
        let client = state
            .security
            .clients
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| ServiceError::new("not_found", "Client not found"))?;
        client.revoked = true;
        client.epoch += 1;
        for operation in state.operations.values() {
            if lock(&operation.record)?.owner == id {
                operation.cancel.store(true, Ordering::Release);
            }
        }
        for ((owner, _), (_, lane)) in &state.query_lanes {
            if owner == id {
                lane.store(true, Ordering::Release);
            }
        }
        state.snapshots.retain(|_, s| s.owner != id);
        state.selections.retain(|_, s| s.owner != id);
        state.plans.retain(|_, p| p.owner != id);
        state.assets.retain(|_, a| a.owner != id);
        state.releases.retain(|_, release| release.owner != id);
        state.image_receipts.retain(|_, image| image.owner != id);
        write_json(&self.0.directory.join("clients.json"), &state.security)?;
        drop(state);
        self.cancel_analysis_for(id)?;
        self.emit("service_changed", json!({"revoked":id}));
        Ok(())
    }
    pub fn authenticate(&self, credential: &str) -> ServiceResult<Principal> {
        if credential.len() > 4096 {
            return Err(denied());
        }
        let digest = hash(credential.as_bytes());
        let state = lock(&self.0.state)?;
        if !state.security.enabled {
            return Err(ServiceError::new(
                "disabled",
                "External agent access is disabled",
            ));
        }
        let client = state
            .security
            .clients
            .iter()
            .find(|c| {
                constant_eq(c.credential_hash.as_bytes(), digest.as_bytes())
                    && !c.revoked
                    && !expired(&c.grant)
            })
            .ok_or_else(denied)?;
        Ok(Principal {
            id: client.id.clone(),
            epoch: client.epoch,
        })
    }
    fn grant(&self, principal: &Principal, action: &str) -> ServiceResult<Grant> {
        let state = lock(&self.0.state)?;
        let client = state
            .security
            .clients
            .iter()
            .find(|c| {
                c.id == principal.id
                    && c.epoch == principal.epoch
                    && !c.revoked
                    && !expired(&c.grant)
            })
            .ok_or_else(denied)?;
        if !state.security.enabled || !client.grant.actions.contains(action) {
            return Err(denied());
        }
        Ok(client.grant.clone())
    }
    pub fn stop_all(&self) -> ServiceResult<()> {
        self.stop_analysis()?;
        for (_, lane) in lock(&self.0.state)?.query_lanes.values() {
            lane.store(true, Ordering::Release);
        }
        for operation in lock(&self.0.state)?.operations.values() {
            operation.cancel.store(true, Ordering::Release);
        }
        self.emit("operation_changed", json!({"stop_requested":true}));
        Ok(())
    }
    pub fn operations(&self) -> ServiceResult<Value> {
        let operations: Vec<_> = lock(&self.0.state)?.operations.values().cloned().collect();
        let records = operations
            .iter()
            .map(|op| lock(&op.record).map(|r| r.clone()))
            .collect::<ServiceResult<Vec<_>>>()?;
        Ok(json!({"operations":records}))
    }
    /// Trusted desktop-only adoption: MCP clients receive sanitized diffs instead.
    pub fn draft_plan(&self, principal_id: &str, draft_id: &str) -> ServiceResult<PlanDto> {
        let state = lock(&self.0.state)?;
        let sealed = state
            .plans
            .get(draft_id)
            .filter(|p| p.owner == principal_id)
            .ok_or_else(missing_handle)?;
        Ok(if sealed.live {
            state
                .manual_plan
                .as_ref()
                .map(|plan| staging::filtered(plan, |key| sealed.scope.contains(key)))
                .unwrap_or_else(|| staging::filtered(&sealed.plan, |_| false))
        } else {
            sealed.plan.clone()
        })
    }
    pub fn draft_review(&self, principal_id: &str, draft_id: &str) -> ServiceResult<Value> {
        let state = lock(&self.0.state)?;
        let plan = state
            .plans
            .get(draft_id)
            .filter(|p| p.owner == principal_id)
            .ok_or_else(missing_handle)?;
        Ok(plans::describe_live(&state, plan, 0, MAX_PAGE))
    }
    /// Trusted desktop publishes its staged plan solely for immediate-write
    /// rebase. Agent drafts never replace this value or the human's desired edits.
    pub fn set_manual_plan(&self, plan: Option<PlanDto>) -> ServiceResult<u64> {
        let revision = lock(&self.0.state)?.manual_revision;
        Ok(self.publish_manual_plan(plan, revision)?.revision)
    }
    pub fn manual_plan(&self) -> ServiceResult<Option<PlanDto>> {
        Ok(lock(&self.0.state)?.manual_plan.clone())
    }
    pub fn call(&self, principal: &Principal, tool: &str, args: Value) -> ServiceResult<Value> {
        self.grant(principal, "read")?;
        if serde_json::to_vec(&args)?.len() > MAX_ARGUMENT_BYTES {
            return Err(ServiceError::new(
                "quota",
                "Tool arguments exceed 1 MiB; use handles and pages",
            ));
        }
        if !args.is_object() {
            return Err(ServiceError::invalid("Tool arguments must be an object"));
        }
        let result = match tool {
            "session.status" => self.session_status(principal),
            "staging.get" => self.stage_get(principal, &args),
            "files.snapshot" => self.snapshot(principal),
            "files.page" => self.page(principal, &args),
            "files.query" => self.query(principal, &args),
            "queries.cancel" => self.cancel_query(principal, &args),
            "files.select" => self.select(principal, &args),
            "files.read" => self.read_tracks(principal, &args),
            "files.field" => self.field(principal, &args),
            "files.selection_page" => self.selection_page(principal, &args),
            "draft.list" => self.own_drafts(principal),
            "draft.create"
            | "draft.get"
            | "draft.discard"
            | "draft.transform"
            | "draft.rename"
            | "draft.move"
            | "draft.from_name"
            | "draft.from_text"
            | "draft.number"
            | "draft.import"
            | "draft.artwork"
            | "draft.clear_tags"
            | "draft.remove_block"
            | "draft.convert_block"
            | "draft.fix_encoding"
            | "plan.commit"
            | "files.write"
            | "operations.status"
            | "operations.cancel"
            | "history.list"
            | "history.undo"
            | "history.redo" => self.plan_call(principal, tool, &args),
            _ => self.domain_call(principal, tool, &args),
        }?;
        // A grant can expire/revoke while native/network work is running.
        // Do not publish its result after that authorization boundary changes.
        self.grant(principal, "read")?;
        if serde_json::to_vec(&result)?.len() > 1024 * 1024 {
            let handle = result
                .get("draft")
                .or_else(|| result.get("operation"))
                .or_else(|| result.get("release"))
                .and_then(Value::as_str)
                .unwrap_or("none");
            return Err(ServiceError::new("response_too_large",format!("Response exceeds 1 MiB; use smaller pages or explicit files.field. Retained result handle: {handle}")));
        }
        Ok(result)
    }
    fn session_status(&self, principal: &Principal) -> ServiceResult<Value> {
        let grant = self.grant(principal, "read")?;
        let app = lock(&self.0.app)?;
        Ok(
            json!({"open":app.is_some(),"principal":principal.id,"actions":grant.actions,"roots":grant.roots,"max_files":grant.max_files,"epoch":self.0.epoch,"recovery_required":lock(&self.0.state)?.recovery_required,"capabilities":Self::tool_catalog().iter().map(|t| &t.name).collect::<Vec<_>>()}),
        )
    }
    fn snapshot(&self, principal: &Principal) -> ServiceResult<Value> {
        let grant = self.grant(principal, "read")?;
        let _build = self.0.build_snapshot.try_lock().map_err(|_| {
            ServiceError::new(
                "busy",
                "A snapshot is being prepared; retry this read-only request shortly",
            )
        })?;
        let app_guard = lock(&self.0.app)?;
        let app = app_guard.as_ref().ok_or_else(no_library)?;
        let cached = {
            let state = lock(&self.0.state)?;
            state
                .snapshot_cache
                .clone()
                .filter(|(id, _)| Some(*id) == app.query_snapshot_id())
        };
        let all_rows = if let Some((_, rows)) = cached {
            rows
        } else {
            let rows = Arc::new(app.list_tracks());
            let generation = app.query_snapshot_id().ok_or_else(no_library)?;
            lock(&self.0.state)?.snapshot_cache = Some((generation, rows.clone()));
            rows
        };
        let full_scope = permitted(&grant, app.library_root(), false).is_ok();
        let generation = app.query_snapshot_id();
        let source_root = app.library_root().to_path_buf();
        drop(app_guard);
        let rows = if full_scope {
            all_rows
        } else {
            Arc::new(
                all_rows
                    .iter()
                    .filter(|r| permitted(&grant, Path::new(&r.path), false).is_ok())
                    .cloned()
                    .collect(),
            )
        };
        {
            let state = lock(&self.0.state)?;
            quota(state.snapshots.len())?;
            let indexed: usize = state.snapshots.values().map(|s| s.rows.len()).sum();
            if indexed.saturating_add(rows.len()) > 400_000 {
                return Err(ServiceError::new(
                    "quota",
                    "Snapshot index budget is 400000 retained rows; release unused snapshots",
                ));
            }
        }
        // Index construction is pure CPU over immutable DTOs. It must not hold
        // the desktop App mutex or park other native commands for hundreds of ms.
        let index = Arc::new(tagrex_commands::query::QueryIndex::from_tracks(&rows));
        {
            let guard = lock(&self.0.app)?;
            let app = guard.as_ref().ok_or_else(no_library)?;
            if app.query_snapshot_id() != generation || app.library_root() != source_root {
                return Err(ServiceError::new(
                    "stale_snapshot",
                    "The opened files changed while preparing this snapshot; retry",
                ));
            }
        }
        let id = opaque("snapshot")?;
        let total = rows.len();
        let mut state = lock(&self.0.state)?;
        quota(state.snapshots.len())?;
        let indexed_rows: usize = state.snapshots.values().map(|s| s.rows.len()).sum();
        if indexed_rows.saturating_add(total) > 400_000 {
            return Err(ServiceError::new(
                "quota",
                "Snapshot index budget is 400000 retained rows; release unused snapshots",
            ));
        }
        // Large snapshots share saved DTOs instead of cloning a 100k-row model
        // for each agent. Each private query index remains bounded independently.
        state.snapshots.insert(
            id.clone(),
            Snapshot {
                owner: principal.id.clone(),
                id: id.clone(),
                rows,
                index,
            },
        );
        Ok(json!({"snapshot":id,"total":total,"page_size":100}))
    }
    fn get_snapshot(&self, principal: &Principal, args: &Value) -> ServiceResult<Snapshot> {
        let id = required(args, "snapshot")?;
        lock(&self.0.state)?
            .snapshots
            .get(id)
            .filter(|s| s.owner == principal.id)
            .cloned()
            .ok_or_else(missing_handle)
    }
    fn page(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let snapshot = self.get_snapshot(principal, args)?;
        let offset = number(args, "offset", 0);
        let limit = number(args, "limit", 100).clamp(1, MAX_PAGE);
        let fields: Option<Vec<String>> = args
            .get("fields")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?;
        let rows: Vec<_> = snapshot
            .rows
            .iter()
            .skip(offset)
            .take(limit)
            .map(|row| project_track(row, fields.as_deref()))
            .collect();
        Ok(
            json!({"snapshot":snapshot.id,"offset":offset,"next_offset":(offset+rows.len()<snapshot.rows.len()).then_some(offset+rows.len()),"total":snapshot.rows.len(),"rows":rows}),
        )
    }
    fn query(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let snapshot = self.get_snapshot(principal, args)?;
        let query: tagrex_commands::query::QueryModel = serde_json::from_value(
            args.get("query")
                .cloned()
                .ok_or_else(|| ServiceError::invalid("query is required"))?,
        )?;
        let request = tagrex_commands::query::QueryRequest {
            request_id: 1,
            snapshot_id: Some(snapshot.index.id),
            query,
        };
        let lane = args.get("lane").and_then(Value::as_str).unwrap_or("query");
        if lane.len() > 64 || lane.chars().any(char::is_control) {
            return Err(ServiceError::invalid(
                "Query lane must be short printable text",
            ));
        }
        let query_request = args
            .get("query_request")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or(opaque("query")?);
        if query_request.len() > 128 || query_request.chars().any(char::is_control) {
            return Err(ServiceError::invalid(
                "Query request ID must be short printable text",
            ));
        }
        let expiry = self.grant(principal, "read")?.expires_at.map(|expiry| {
            std::time::Instant::now() + std::time::Duration::from_secs(expiry.saturating_sub(now()))
        });
        let key = (principal.id.clone(), lane.to_string());
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut state = lock(&self.0.state)?;
            if !state.query_lanes.contains_key(&key)
                && state
                    .query_lanes
                    .keys()
                    .filter(|(owner, _)| owner == &principal.id)
                    .count()
                    >= 8
            {
                return Err(ServiceError::new(
                    "busy",
                    "At most eight query lanes may run per client",
                ));
            }
            if let Some((_, previous)) = state
                .query_lanes
                .insert(key.clone(), (query_request.clone(), cancel.clone()))
            {
                previous.store(true, Ordering::Release);
            }
        }
        let outcome = snapshot.index.query(&request, || {
            cancel.load(Ordering::Acquire)
                || expiry.is_some_and(|deadline| std::time::Instant::now() >= deadline)
        });
        {
            let mut state = lock(&self.0.state)?;
            if state
                .query_lanes
                .get(&key)
                .is_some_and(|(_, current)| Arc::ptr_eq(current, &cancel))
            {
                state.query_lanes.remove(&key);
            }
        }
        let result = outcome.map_err(|error| {
            if matches!(error, tagrex_commands::query::QueryError::Cancelled) {
                ServiceError::new("cancelled", "This query lane was cancelled or superseded")
            } else {
                ServiceError::invalid(error.to_string())
            }
        })?;
        self.grant(principal, "read")?;
        let bytes = result
            .indices
            .iter()
            .filter_map(|i| snapshot.rows.get(*i))
            .map(|r| r.path.len() + std::mem::size_of::<PathBuf>())
            .sum();
        {
            let state = lock(&self.0.state)?;
            selection_budget(&state, bytes)?;
        }
        let paths: Vec<_> = result
            .indices
            .iter()
            .filter_map(|i| snapshot.rows.get(*i))
            .map(|r| PathBuf::from(&r.path))
            .collect();
        let id = opaque("selection")?;
        let mut state = lock(&self.0.state)?;
        quota(state.selections.len())?;
        selection_budget(&state, bytes)?;
        state.selections.insert(
            id.clone(),
            Selection {
                owner: principal.id.clone(),
                paths,
                bytes,
            },
        );
        let offset = number(args, "offset", 0);
        let limit = number(args, "limit", 100).clamp(1, MAX_PAGE);
        let fields: Option<Vec<String>> = args
            .get("fields")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?;
        let rows: Vec<_> = result
            .indices
            .iter()
            .skip(offset)
            .take(limit)
            .filter_map(|i| snapshot.rows.get(*i))
            .map(|row| project_track(row, fields.as_deref()))
            .collect();
        Ok(
            json!({"snapshot":snapshot.id,"selection":id,"lane":lane,"query_request":query_request,"matched":result.matched,"total":result.total,"rows":rows,"next_offset":(offset+rows.len()<result.matched).then_some(offset+rows.len())}),
        )
    }
    fn select(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let snapshot = self.get_snapshot(principal, args)?;
        let estimated_bytes = if let Some(indices) = args.get("indices") {
            let indices: Vec<usize> = serde_json::from_value(indices.clone())?;
            indices
                .iter()
                .map(|i| {
                    snapshot
                        .rows
                        .get(*i)
                        .map(|r| r.path.len() + std::mem::size_of::<PathBuf>())
                        .ok_or_else(|| ServiceError::invalid("Selection index is outside snapshot"))
                })
                .collect::<ServiceResult<Vec<_>>>()?
                .iter()
                .sum()
        } else {
            snapshot
                .rows
                .iter()
                .map(|r| r.path.len() + std::mem::size_of::<PathBuf>())
                .sum()
        };
        {
            let state = lock(&self.0.state)?;
            selection_budget(&state, estimated_bytes)?;
        }
        let paths = if let Some(indices) = args.get("indices") {
            let indices: Vec<usize> = serde_json::from_value(indices.clone())?;
            indices
                .into_iter()
                .map(|i| {
                    snapshot
                        .rows
                        .get(i)
                        .map(|r| PathBuf::from(&r.path))
                        .ok_or_else(|| ServiceError::invalid("Selection index is outside snapshot"))
                })
                .collect::<ServiceResult<Vec<_>>>()?
        } else {
            snapshot
                .rows
                .iter()
                .map(|r| PathBuf::from(&r.path))
                .collect()
        };
        let grant = self.grant(principal, "read")?;
        check_paths(&grant, &paths, false)?;
        let id = opaque("selection")?;
        let count = paths.len();
        let mut state = lock(&self.0.state)?;
        quota(state.selections.len())?;
        selection_budget(&state, estimated_bytes)?;
        state.selections.insert(
            id.clone(),
            Selection {
                owner: principal.id.clone(),
                paths,
                bytes: estimated_bytes,
            },
        );
        Ok(json!({"selection":id,"count":count}))
    }
    fn paths(
        &self,
        principal: &Principal,
        args: &Value,
        action: &str,
    ) -> ServiceResult<Vec<PathBuf>> {
        let grant = self.grant(principal, action)?;
        let paths = if let Some(id) = args.get("selection").and_then(Value::as_str) {
            lock(&self.0.state)?
                .selections
                .get(id)
                .filter(|s| s.owner == principal.id)
                .map(|s| s.paths.clone())
                .ok_or_else(missing_handle)?
        } else {
            serde_json::from_value::<Vec<PathBuf>>(
                args.get("paths")
                    .cloned()
                    .ok_or_else(|| ServiceError::invalid("selection or paths is required"))?,
            )?
        };
        check_paths(&grant, &paths, action == "write")?;
        Ok(paths)
    }
    fn read_tracks(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let paths = self.paths(principal, args, "read")?;
        if paths.len() > MAX_PAGE {
            return Err(ServiceError::new(
                "quota",
                "Read at most 500 tracks; use snapshot pages for larger selections",
            ));
        }
        let app = lock(&self.0.app)?;
        let app = app.as_ref().ok_or_else(no_library)?;
        let fields: Option<Vec<String>> = args
            .get("fields")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?;
        Ok(
            json!({"tracks":app.read_track_rows(&paths)?.iter().map(|row|project_track(row,fields.as_deref())).collect::<Vec<_>>()}),
        )
    }
    fn selection_page(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let grant = self.grant(principal, "read")?;
        let offset = number(args, "offset", 0);
        let limit = number(args, "limit", 100).clamp(1, MAX_PAGE);
        let (total, selected) = {
            let state = lock(&self.0.state)?;
            let selection = state
                .selections
                .get(required(args, "selection")?)
                .filter(|s| s.owner == principal.id)
                .ok_or_else(missing_handle)?;
            (
                selection.paths.len(),
                selection
                    .paths
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        };
        let rows = if selected.is_empty() {
            Vec::new()
        } else {
            check_paths(&grant, &selected, false)?;
            let guard = lock(&self.0.app)?;
            guard
                .as_ref()
                .ok_or_else(no_library)?
                .read_track_rows(&selected)?
        };
        let fields: Option<Vec<String>> = args
            .get("fields")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?;
        Ok(
            json!({"rows":rows.iter().map(|row|project_track(row,fields.as_deref())).collect::<Vec<_>>(),"offset":offset,"total":total,"next_offset":(offset+rows.len()<total).then_some(offset+rows.len())}),
        )
    }
    fn cancel_query(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let lane = args.get("lane").and_then(Value::as_str).unwrap_or("query");
        let key = (principal.id.clone(), lane.to_string());
        let state = lock(&self.0.state)?;
        let expected = args.get("query_request").and_then(Value::as_str);
        let found = state
            .query_lanes
            .get(&key)
            .filter(|(request, _)| expected.is_none_or(|expected| expected == request));
        if let Some((_, cancel)) = found {
            cancel.store(true, Ordering::Release);
        }
        Ok(json!({"lane":lane,"cancel_requested":found.is_some()}))
    }
    fn own_drafts(&self, principal: &Principal) -> ServiceResult<Value> {
        let state = lock(&self.0.state)?;
        Ok(
            json!({"drafts":state.plans.values().filter(|p|p.owner==principal.id).map(|p|json!({"draft":p.id,"digest":p.digest,"files":p.plan.changes.len(),"description":p.plan.description})).collect::<Vec<_>>()}),
        )
    }
    fn field(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let paths = self.paths(principal, args, "read")?;
        if paths.len() != 1 {
            return Err(ServiceError::invalid(
                "Select one file for an explicit field read",
            ));
        }
        let field = required(args, "field")?;
        let offset = number(args, "offset", 0);
        let limit = number(args, "limit", 16 * 1024).clamp(1, 64 * 1024);
        let app = lock(&self.0.app)?;
        let row = app.as_ref().ok_or_else(no_library)?.read_track(&paths[0])?;
        let value = row.tags.get(field).map(String::as_str).unwrap_or("");
        let chunk: String = value.chars().skip(offset).take(limit).collect();
        let read = chunk.chars().count();
        let total = value.chars().count();
        Ok(
            json!({"path":paths[0],"field":field,"offset":offset,"unit":"unicode_scalar","value":chunk,"total":total,"next_offset":(offset+read<total).then_some(offset+read)}),
        )
    }
    fn emit(&self, event: &str, value: Value) {
        let sink = self.0.events.lock().ok().and_then(|sink| sink.clone());
        if let Some(sink) = sink {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(event, value)));
        }
    }
    fn settings(&self) -> ServiceResult<Arc<dyn SettingsBackend>> {
        lock(&self.0.settings)?.clone().ok_or_else(|| {
            ServiceError::new("unavailable", "Desktop settings adapter is not installed")
        })
    }
    fn providers(&self) -> ServiceResult<Arc<dyn ProviderBackend>> {
        lock(&self.0.providers)?.clone().ok_or_else(|| {
            ServiceError::new("unavailable", "Desktop provider adapter is not installed")
        })
    }
}

fn selection_budget(state: &State, additional: usize) -> ServiceResult<()> {
    let retained: usize = state.selections.values().map(|s| s.bytes).sum();
    if retained.saturating_add(additional) > MAX_SELECTION_BYTES {
        Err(ServiceError::new(
            "quota",
            "Selection paths exceed the 64 MiB service budget; release unused selections",
        ))
    } else {
        Ok(())
    }
}

fn project_track(row: &TrackDto, fields: Option<&[String]>) -> Value {
    // Project references before serialization, so an omitted megabyte of lyrics
    // is never cloned into a temporary JSON value merely to be discarded.
    let tags: BTreeMap<_, _> = row
        .tags
        .iter()
        .filter(|(key, _)| fields.is_none_or(|fields| fields.contains(key)))
        .collect();
    json!({"path":row.path,"format":row.format,"tags":tags,"unreadable":row.unreadable,"read_error":row.read_error,"duration_secs":row.duration_secs,"tag_blocks":row.tag_blocks,"compilation_field":row.compilation_field,"has_cover":row.has_cover})
}

pub fn settings_revision(value: &Value) -> String {
    hash(&serde_json::to_vec(value).unwrap_or_default())
}
fn lock<T>(mutex: &Mutex<T>) -> ServiceResult<MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| {
        ServiceError::new(
            "service_unavailable",
            "Application service lock is poisoned",
        )
    })
}
fn required<'a>(args: &'a Value, key: &str) -> ServiceResult<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ServiceError::invalid(format!("{key} is required")))
}
fn number(args: &Value, key: &str, default: usize) -> usize {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(default)
}
fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn no_library() -> ServiceError {
    ServiceError::new("no_session", "Open files in TAGnetic first")
}
fn denied() -> ServiceError {
    ServiceError::new(
        "permission_denied",
        "This client is not authorized for the requested scope",
    )
}
fn missing_handle() -> ServiceError {
    ServiceError::new(
        "invalid_handle",
        "Handle is missing, expired, or owned by another client",
    )
}
fn quota(size: usize) -> ServiceResult<()> {
    if size >= MAX_HANDLES {
        Err(ServiceError::new(
            "quota",
            "Release unused handles before creating more",
        ))
    } else {
        Ok(())
    }
}
fn expired(grant: &Grant) -> bool {
    grant.expires_at.is_some_and(|expiry| now() >= expiry)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}
fn opaque(prefix: &str) -> ServiceResult<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|_| {
        ServiceError::new("entropy_error", "Secure random identity generation failed")
    })?;
    Ok(format!(
        "{prefix}_{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}
fn permitted(grant: &Grant, path: &Path, mutation: bool) -> ServiceResult<PathBuf> {
    let canonical = fs::canonicalize(path)
        .map_err(|_| ServiceError::new("invalid_path", "File is missing or inaccessible"))?;
    if !grant.roots.iter().any(|root| canonical.starts_with(root)) {
        return Err(denied());
    }
    // A symlink name is not an approved regular-file identity. Hard links may
    // alias an unapproved root; refuse mutations rather than silently widen it.
    if mutation && !grant.write_files.is_empty() && !grant.write_files.contains(&canonical) {
        return Err(denied());
    }
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(ServiceError::new(
            "unsafe_path",
            "Symbolic links are not supported for agent file operations",
        ));
    }
    #[cfg(unix)]
    if mutation {
        use std::os::unix::fs::MetadataExt;
        if fs::metadata(path)?.nlink() > 1 {
            return Err(ServiceError::new(
                "unsafe_path",
                "Mutating multiply-linked files requires a separate explicit policy",
            ));
        }
    }
    Ok(canonical)
}
fn destination(grant: &Grant, path: &Path) -> ServiceResult<()> {
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ServiceError::new(
            "unsafe_path",
            "Output paths cannot contain parent traversal",
        ));
    }
    let mut ancestor = path;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| ServiceError::invalid("Output has no existing ancestor"))?;
    }
    permitted(grant, ancestor, false)?;
    Ok(())
}
fn check_paths(grant: &Grant, paths: &[PathBuf], mutation: bool) -> ServiceResult<()> {
    if paths.is_empty() || paths.len() > grant.max_files {
        return Err(ServiceError::new(
            "quota",
            "Selection is empty or exceeds this client's file budget",
        ));
    }
    let mut seen = BTreeSet::new();
    for path in paths {
        let canonical = permitted(grant, path, mutation)?;
        if !seen.insert(canonical) {
            return Err(ServiceError::invalid(
                "Selection contains duplicate file identities",
            ));
        }
    }
    Ok(())
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> ServiceResult<Option<T>> {
    match private_storage::read_bounded(path, 64 * 1024 * 1024) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).map_err(|_| {
            ServiceError::new(
                "storage_error",
                "Service state is malformed; preserve it for recovery",
            )
        })?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn write_json<T: Serialize>(path: &Path, value: &T) -> ServiceResult<()> {
    private_storage::write_atomic(path, &serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests;

impl Drop for Inner {
    fn drop(&mut self) {
        outputs::retire_analysis(&self.epoch);
    }
}

#[cfg(test)]
mod scale_tests;
