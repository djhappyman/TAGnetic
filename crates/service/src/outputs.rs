//! Confined outputs and bounded, scan-scoped read-only analysis.
use super::*;
use std::sync::LazyLock;
use tagrex_core::export::{self, CueTrack, PlaylistTrack};
use tagrex_core::mask::Mask;
use tagrex_core::model::{AudioFormat, TagEngine, TagField, TrackFile};

const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ANALYSIS_PATH_BYTES: usize = 64 * 1024 * 1024;
const MAX_ANALYSIS_JOBS: usize = 32;
const MAX_RUNNING_ANALYSIS: usize = 2;

struct ScanJob {
    owner: String,
    epoch: String,
    kind: String,
    cancel: AtomicBool,
    paths: Vec<String>,
    status: Mutex<ScanStatus>,
}
struct ScanStatus {
    phase: String,
    status: String,
    processed: usize,
    skipped: usize,
    errors: usize,
    groups: Vec<tagrex_commands::query::DuplicateQueryGroup>,
    detected: Vec<Value>,
}
enum AnalysisResult {
    Duplicates(tagrex_commands::query::DuplicateResults),
    Bpm(Vec<Value>),
}
static SCANS: LazyLock<Mutex<HashMap<String, Arc<ScanJob>>>> = LazyLock::new(Mutex::default);

pub(super) fn retire_analysis(epoch: &str) {
    if let Ok(mut scans) = SCANS.lock() {
        scans.retain(|_, job| {
            if job.epoch == epoch {
                job.cancel.store(true, Ordering::Release);
                false
            } else {
                true
            }
        });
    }
}

impl Service {
    pub(super) fn output_call(
        &self,
        principal: &Principal,
        tool: &str,
        args: &Value,
    ) -> ServiceResult<Value> {
        match tool {
            "exports.create" => self.export_files(principal, args),
            "artwork.export" => self.export_artwork(principal, args),
            "analysis.duplicates" => self.duplicate_analysis(principal, args, false),
            "analysis.bpm" => self.duplicate_analysis(principal, args, true),
            _ => Err(ServiceError::new(
                "unknown_tool",
                "Unknown output or analysis tool",
            )),
        }
    }

    pub fn stop_analysis(&self) -> ServiceResult<()> {
        for job in lock(&SCANS)?
            .values()
            .filter(|job| job.epoch == self.0.epoch)
        {
            job.cancel.store(true, Ordering::Release);
        }
        Ok(())
    }

    pub fn cancel_analysis_for(&self, principal_id: &str) -> ServiceResult<()> {
        for job in lock(&SCANS)?
            .values()
            .filter(|job| job.epoch == self.0.epoch && job.owner == principal_id)
        {
            job.cancel.store(true, Ordering::Release);
        }
        Ok(())
    }

    /// Trusted desktop status. External callers use their owned tool handles.
    pub fn analysis_jobs(&self) -> ServiceResult<Value> {
        let jobs = lock(&SCANS)?
            .iter()
            .filter(|(_, job)| job.epoch == self.0.epoch)
            .map(|(id, job)| (id.clone(), job.clone()))
            .collect::<Vec<_>>();
        let mut values = Vec::new();
        for (id, job) in jobs {
            let status = lock(&job.status)?;
            values.push(
                json!({"id":id,"owner":job.owner,"kind":job.kind,"status":status.status,
                "phase":status.phase,"processed":status.processed,"total":job.paths.len()}),
            );
        }
        Ok(json!({"jobs":values}))
    }

    pub fn cancel_analysis_job(&self, id: &str) -> ServiceResult<()> {
        let job = lock(&SCANS)?
            .get(id)
            .filter(|job| job.epoch == self.0.epoch)
            .cloned()
            .ok_or_else(missing_handle)?;
        job.cancel.store(true, Ordering::Release);
        Ok(())
    }

    fn export_files(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let grant = self.grant(principal, "outputs")?;
        let paths = self.paths(principal, args, "read")?;
        let kind = required(args, "format")?;
        let _mutation = lock(&self.0.mutation)?;
        self.grant(principal, "outputs")?;
        let app_guard = lock(&self.0.app)?;
        let app = app_guard.as_ref().ok_or_else(no_library)?;
        // Membership and scope are verified before the native exporter reads.
        let rows = app.read_track_rows(&paths)?;
        if rows.iter().any(|row| row.unreadable) {
            return Err(ServiceError::new(
                "read_failed",
                "A selected track cannot be read",
            ));
        }
        let root = output_root(&grant, args, app.library_root())?;
        // The rows already carry the same tag snapshot the native exporter
        // needs, keyed by storage key. Reuse it instead of reading every file a
        // second time through `TagEngine::read`; playlist formats never need a
        // `TrackFile` at all.
        let tracks = || tracks_from_rows(&paths, &rows);
        let split = args.get("split").and_then(Value::as_str).unwrap_or("none");
        if split != "none" {
            if !["playlist", "m3u"].contains(&kind) || !["album", "folder"].contains(&split) {
                return Err(ServiceError::invalid(
                    "Split exports require playlist/m3u format and album or folder grouping",
                ));
            }
            let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
            let mut group_indices = HashMap::new();
            for (index, row) in rows.iter().enumerate() {
                let key = if split == "album" {
                    row.tags.get("album").cloned().unwrap_or_default()
                } else {
                    Path::new(&row.path)
                        .parent()
                        .unwrap_or(Path::new(""))
                        .to_string_lossy()
                        .into_owned()
                };
                if let Some(group_index) = group_indices.get(&key).copied() {
                    let (_, members): &mut (String, Vec<usize>) = &mut groups[group_index];
                    members.push(index);
                } else {
                    group_indices.insert(key.clone(), groups.len());
                    groups.push((key, vec![index]));
                }
            }
            let mut outputs = Vec::new();
            let mut taken = BTreeSet::new();
            let mut budget = 0usize;
            let pattern = args
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or("%album%");
            for (_, indices) in groups {
                let first = &paths[indices[0]];
                let names = app.render_column(pattern, std::slice::from_ref(first))?;
                let stem = tagrex_core::mask::portable_file_name(
                    names.first().map(String::as_str).unwrap_or("Playlist"),
                    true,
                );
                let stem = if stem.trim().is_empty() {
                    "Playlist"
                } else {
                    stem.trim()
                };
                let stem = stem.strip_suffix(".m3u").unwrap_or(stem);
                let mut suffix = 1usize;
                let mut name = format!("{stem}.m3u");
                while !taken.insert(name.to_lowercase()) {
                    suffix += 1;
                    name = format!("{stem} ({suffix}).m3u");
                }
                let target = root.join(safe_name(&name)?);
                destination(&grant, &target)?;
                let members = indices.iter().map(|i| &rows[*i]).collect::<Vec<_>>();
                let text = playlist(&root, members.into_iter(), MAX_OUTPUT_BYTES - budget)?;
                budget = budget.saturating_add(text.len());
                if budget > MAX_OUTPUT_BYTES {
                    return Err(ServiceError::new(
                        "quota",
                        "Export exceeds the 64 MiB output budget",
                    ));
                }
                outputs.push((target, text.into_bytes()));
            }
            return self.publish_outputs(principal, args, &root, outputs);
        }
        let name = args
            .get("filename")
            .or_else(|| args.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| ServiceError::invalid("filename is required"))?;
        let target = root.join(safe_name(name)?);
        destination(&grant, &target)?;
        // Playlist rendering comes from the row snapshot above; only the
        // tag-table/report formats need the full native file view. An unknown
        // format still reports its own choice error below.
        let tracks = if matches!(kind, "csv" | "html" | "xml" | "report" | "cue") {
            tracks()?
        } else {
            Vec::new()
        };
        let text = match kind {
            "csv" => export::Format::Csv(&tracks)
                .render_bounded(MAX_OUTPUT_BYTES)
                .map_err(export_limit)?,
            "html" => export::Format::Html(&tracks)
                .render_bounded(MAX_OUTPUT_BYTES)
                .map_err(export_limit)?,
            "xml" => export::Format::Xml(&tracks)
                .render_bounded(MAX_OUTPUT_BYTES)
                .map_err(export_limit)?,
            "report" => {
                let mask = Mask::parse(required(args, "mask")?)
                    .map_err(|_| ServiceError::invalid("Invalid report mask"))?;
                export::Format::Report(&tracks, &mask)
                    .render_bounded(MAX_OUTPUT_BYTES)
                    .map_err(export_limit)?
            }
            "playlist" | "m3u" => playlist(&root, rows.iter(), MAX_OUTPUT_BYTES)?,
            "cue" => export::Format::Cue(
                &tracks
                    .iter()
                    .map(|track| CueTrack {
                        path: portable_path(&root, &track.path),
                        tags: track.tags.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
            .render_bounded(MAX_OUTPUT_BYTES)
            .map_err(export_limit)?,
            _ => {
                return Err(ServiceError::invalid(
                    "Choose csv, html, xml, report, m3u or cue",
                ))
            }
        };
        if text.len() > MAX_OUTPUT_BYTES {
            return Err(ServiceError::new(
                "quota",
                "Export exceeds the 64 MiB output budget",
            ));
        }
        self.publish_outputs(principal, args, &root, vec![(target, text.into_bytes())])
    }

    fn publish_outputs(
        &self,
        principal: &Principal,
        args: &Value,
        root: &Path,
        outputs: Vec<(PathBuf, Vec<u8>)>,
    ) -> ServiceResult<Value> {
        let manifests = outputs.iter().map(|(path, bytes)| json!({"path":path,"bytes":bytes.len(),"sha256":hash(bytes),"overwrite":flag(args,"overwrite")})).collect::<Vec<_>>();
        if !flag(args, "preview") {
            if lock(&self.0.state)?.recovery_required {
                return Err(ServiceError::new(
                    "recovery_required",
                    "Resolve the interrupted operation before publishing new output files",
                ));
            }
            let grant = self.grant(principal, "outputs")?;
            for (target, _) in &outputs {
                destination(&grant, target)?;
            }
            // Existing publisher pins directories and restores previous outputs
            // on a failed batch. These files are outside the music Undo journal.
            tagrex_commands::publish_confined_outputs(root, &outputs, flag(args, "overwrite"))
                .map_err(output_error)?;
        }
        let offset = number(args, "offset", 0);
        let limit = number(args, "limit", 100).clamp(1, MAX_PAGE);
        let count = manifests.len();
        Ok(
            json!({"outputs":manifests.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"output_count":count,
            "next_offset":(offset.saturating_add(limit)<count).then_some(offset.saturating_add(limit)),"written":!flag(args,"preview"),"journal_undo":false}),
        )
    }

    fn export_artwork(&self, principal: &Principal, args: &Value) -> ServiceResult<Value> {
        let grant = self.grant(principal, "outputs")?;
        if let Some(id) = args.get("asset").and_then(Value::as_str) {
            let asset = lock(&self.0.state)?
                .assets
                .get(id)
                .filter(|asset| asset.owner == principal.id)
                .cloned()
                .ok_or_else(missing_handle)?;
            let root = args.get("root").and_then(Value::as_str).ok_or_else(|| {
                ServiceError::invalid("root is required when exporting an artwork asset")
            })?;
            let root = output_root(&grant, args, Path::new(root))?;
            let extension = match asset.cover.mime.as_str() {
                "image/jpeg" => "jpg",
                "image/png" => "png",
                "image/webp" => "webp",
                _ => return Err(ServiceError::invalid("Unsupported artwork type")),
            };
            let filename = args
                .get("filename")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("cover.{extension}"));
            let target = root.join(safe_name(&filename)?);
            destination(&grant, &target)?;
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(asset.cover.data_base64.as_bytes())
                .map_err(|_| ServiceError::invalid("Invalid prepared artwork encoding"))?;
            let _mutation = lock(&self.0.mutation)?;
            return self.publish_outputs(principal, args, &root, vec![(target, bytes)]);
        }
        let paths = self.paths(principal, args, "read")?;
        let basename = safe_name(
            args.get("basename")
                .or_else(|| args.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("cover"),
        )?;
        let _mutation = lock(&self.0.mutation)?;
        self.grant(principal, "outputs")?;
        let app_guard = lock(&self.0.app)?;
        let app = app_guard.as_ref().ok_or_else(no_library)?;
        app.read_track_rows(&paths)?;
        let root = output_root(&grant, args, app.library_root())?;
        let mut seen = BTreeSet::new();
        let mut outputs = Vec::new();
        let mut skipped = 0;
        let mut bytes = 0usize;
        for path in paths {
            self.grant(principal, "outputs")?;
            let Some(cover) = TagEngine::read_cover(&path)
                .map_err(|_| ServiceError::new("read_failed", "Artwork could not be read"))?
            else {
                skipped += 1;
                continue;
            };
            let extension = match cover.mime.as_str() {
                "image/jpeg" | "image/jpg" => "jpg",
                "image/png" => "png",
                "image/webp" => "webp",
                "image/gif" => "gif",
                _ => return Err(ServiceError::invalid("Unsupported embedded artwork type")),
            };
            let parent = if args.get("root").is_some() {
                root.clone()
            } else {
                fs::canonicalize(
                    path.parent()
                        .ok_or_else(|| ServiceError::invalid("Missing source directory"))?,
                )?
            };
            let target = parent.join(format!("{basename}.{extension}"));
            destination(&grant, &target)?;
            if !seen.insert(target.clone()) {
                continue;
            }
            bytes = bytes
                .checked_add(cover.data.len())
                .ok_or_else(|| ServiceError::new("quota", "Output budget exceeded"))?;
            if bytes > MAX_OUTPUT_BYTES {
                return Err(ServiceError::new(
                    "quota",
                    "Artwork export exceeds the 64 MiB output budget",
                ));
            }
            outputs.push((target, cover.data.to_vec()));
        }
        let mut result = self.publish_outputs(principal, args, &root, outputs)?;
        result["skipped_no_cover"] = json!(skipped);
        Ok(result)
    }

    fn duplicate_analysis(
        &self,
        principal: &Principal,
        args: &Value,
        bpm: bool,
    ) -> ServiceResult<Value> {
        self.grant(principal, "read")?;
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("start");
        if action != "start" {
            let id = required(args, "job")?;
            let job = lock(&SCANS)?
                .get(id)
                .filter(|job| {
                    job.owner == principal.id
                        && job.epoch == self.0.epoch
                        && (job.kind == "bpm") == bpm
                })
                .cloned()
                .ok_or_else(missing_handle)?;
            match action {
                "cancel" => {
                    job.cancel.store(true, Ordering::Release);
                    return Ok(json!({"job":id,"cancel_requested":true}));
                }
                "release" => {
                    if lock(&job.status)?.status == "running" {
                        return Err(ServiceError::conflict(
                            "Cancel and await completion before releasing a scan",
                        ));
                    }
                    lock(&SCANS)?.remove(id);
                    return Ok(json!({"released":id}));
                }
                "status" => return scan_page(id, &job, args),
                _ => {
                    return Err(ServiceError::invalid(
                        "Choose start, status, cancel or release",
                    ))
                }
            }
        }
        let paths = self.paths(principal, args, "read")?;
        let kind = if bpm {
            "bpm"
        } else {
            args.get("kind").and_then(Value::as_str).unwrap_or("exact")
        };
        if !bpm && !["exact", "same_audio", "likely"].contains(&kind) {
            return Err(ServiceError::invalid("Unsupported duplicate matching kind"));
        }
        let (job_paths, path_bytes) = retained_paths(&paths, MAX_ANALYSIS_PATH_BYTES)?;
        let mut scans = lock(&SCANS)?;
        let running = scans
            .values()
            .filter(|job| job.status.lock().is_ok_and(|s| s.status == "running"))
            .count();
        let retained_bytes = scans
            .values()
            .map(|job| retained_path_bytes(&job.paths))
            .sum::<usize>();
        if scans.len() >= MAX_ANALYSIS_JOBS
            || running >= MAX_RUNNING_ANALYSIS
            || retained_bytes.saturating_add(path_bytes) > MAX_ANALYSIS_PATH_BYTES
        {
            return Err(ServiceError::new(
                "busy",
                "Analysis capacity is full; release completed scans or await an active scan",
            ));
        }
        // Reserve before releasing the registry lock, so concurrent starts
        // cannot exceed admission. Metadata loading takes place in the worker.
        let id = opaque("scan")?;
        let job = Arc::new(ScanJob {
            owner: principal.id.clone(),
            epoch: self.0.epoch.clone(),
            kind: kind.into(),
            cancel: AtomicBool::new(false),
            paths: job_paths,
            status: Mutex::new(ScanStatus {
                phase: "queued".into(),
                status: "running".into(),
                processed: 0,
                skipped: 0,
                errors: 0,
                groups: Vec::new(),
                detected: Vec::new(),
            }),
        });
        scans.insert(id.clone(), job.clone());
        drop(scans);
        let total = paths.len();
        let service = self.clone();
        let actor = principal.clone();
        let kind = kind.to_owned();
        let task_id = id.clone();
        let result = std::thread::Builder::new()
            .name("tagnetic-agent-analysis".into())
            .spawn(move || {
                let cancelled =
                    || job.cancel.load(Ordering::Acquire) || service.grant(&actor, "read").is_err();
                let run = || -> ServiceResult<AnalysisResult> {
                    if cancelled() {
                        return Err(ServiceError::new("cancelled", "Scan cancelled"));
                    }
                    let rows = {
                        let app_guard = lock(&service.0.app)?;
                        app_guard
                            .as_ref()
                            .ok_or_else(no_library)?
                            .read_track_rows(&paths)?
                    };
                    if bpm {
                        drop(rows);
                        let backend = lock(&service.0.desktop)?.clone().ok_or_else(|| {
                            ServiceError::new(
                                "unavailable",
                                "Desktop analysis adapter is unavailable",
                            )
                        })?;
                        let mut progress = |done: usize, total: usize| {
                            if cancelled() {
                                job.cancel.store(true, Ordering::Release);
                            }
                            if let Ok(mut status) = job.status.lock() {
                                status.phase = "detecting".into();
                                status.processed = done.min(total);
                            }
                        };
                        let value = backend.bpm(&paths, &job.cancel, &mut progress)?;
                        let detected = value
                            .get("detected")
                            .and_then(Value::as_array)
                            .cloned()
                            .ok_or_else(|| {
                                ServiceError::new(
                                    "analysis_failed",
                                    "Invalid tempo-analysis response",
                                )
                            })?;
                        let allowed = job
                            .paths
                            .iter()
                            .map(String::as_str)
                            .collect::<BTreeSet<_>>();
                        if detected.iter().any(|v| {
                            !v.get("path")
                                .and_then(Value::as_str)
                                .is_some_and(|p| allowed.contains(p))
                        }) {
                            return Err(ServiceError::new(
                                "analysis_failed",
                                "Tempo analysis returned a file outside its selection",
                            ));
                        }
                        return Ok(AnalysisResult::Bpm(detected));
                    }
                    let index = tagrex_commands::query::QueryIndex::try_from_tracks(&rows)
                        .map_err(|error| {
                            ServiceError::from(tagrex_commands::AppError::Resource(error))
                        })?;
                    drop(rows);
                    let cache = Mutex::new(tagrex_commands::duplicates::AnalysisCache::default());
                    tagrex_commands::duplicates::scan_with_updates(
                        &index,
                        &kind,
                        &cache,
                        cancelled,
                        |update| {
                            if let Ok(mut status) = job.status.lock() {
                                status.phase = update.phase.into();
                                status.processed = update.processed;
                                status.skipped = update.skipped;
                                status.errors = update.errors;
                                if let Some(groups) = update.groups {
                                    status.groups = groups;
                                }
                            }
                        },
                    )
                    .map(AnalysisResult::Duplicates)
                    .map_err(|_| {
                        ServiceError::new(
                            if cancelled() {
                                "cancelled"
                            } else {
                                "analysis_failed"
                            },
                            "Duplicate scan did not complete",
                        )
                    })
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
                    .unwrap_or_else(|_| {
                        Err(ServiceError::new(
                            "analysis_failed",
                            "Analysis worker stopped unexpectedly",
                        ))
                    });
                let was_cancelled = cancelled();
                if let Ok(mut status) = job.status.lock() {
                    match result {
                        Ok(AnalysisResult::Duplicates(results)) if !was_cancelled => {
                            status.status = "complete".into();
                            status.phase = "complete".into();
                            status.groups = results.groups;
                            status.skipped = results.skipped;
                            status.errors = results.errors;
                            status.processed = job.paths.len();
                        }
                        Ok(AnalysisResult::Bpm(values)) if !was_cancelled => {
                            status.status = "complete".into();
                            status.phase = "complete".into();
                            status.processed = job.paths.len();
                            status.skipped = job.paths.len().saturating_sub(values.len());
                            status.detected = values;
                        }
                        Ok(_) => {
                            status.status = "cancelled".into();
                            status.groups.clear();
                        }
                        Err(error) => {
                            status.status = if error.code == "cancelled" {
                                "cancelled"
                            } else {
                                "failed"
                            }
                            .into();
                            status.groups.clear();
                        }
                    }
                }
                service.emit("analysis_changed", json!({"job":task_id,"complete":true}));
            });
        if result.is_err() {
            lock(&SCANS)?.remove(&id);
            return Err(ServiceError::new(
                "worker_unavailable",
                "Analysis worker could not start",
            ));
        }
        Ok(json!({"job":id,"status":"running","total":total}))
    }
}

fn output_root(grant: &Grant, args: &Value, default: &Path) -> ServiceResult<PathBuf> {
    let root = args
        .get("root")
        .and_then(Value::as_str)
        .map(Path::new)
        .unwrap_or(default);
    let root = permitted(grant, root, false)?;
    if !root.is_dir() {
        return Err(ServiceError::invalid(
            "Output root must be an existing directory",
        ));
    }
    Ok(root)
}

fn playlist<'a>(
    root: &Path,
    rows: impl Iterator<Item = &'a TrackDto>,
    limit: usize,
) -> ServiceResult<String> {
    export::Format::M3u(
        &rows
            .map(|row| PlaylistTrack {
                path: portable_path(root, Path::new(&row.path)),
                artist: row.tags.get("artist").cloned().unwrap_or_default(),
                title: row.tags.get("title").cloned().unwrap_or_else(|| {
                    Path::new(&row.path)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                }),
                duration_secs: row
                    .duration_secs
                    .and_then(|v| i64::try_from(v).ok())
                    .unwrap_or(-1),
            })
            .collect::<Vec<_>>(),
    )
    .render_bounded(limit)
    .map_err(export_limit)
}

/// Build the native export view from the row snapshot already read for library
/// membership. `TrackDto` stores tags by the reversible storage key
/// (`TagField::to_storage_key`), and the real `PathBuf` is kept alongside those
/// rows so file placeholders still work for non-UTF-8 names.
fn tracks_from_rows(paths: &[PathBuf], rows: &[TrackDto]) -> ServiceResult<Vec<TrackFile>> {
    if rows.len() != paths.len() {
        return Err(ServiceError::new(
            "read_failed",
            "A selected track cannot be read",
        ));
    }
    paths
        .iter()
        .zip(rows)
        .map(|(path, row)| {
            if row.unreadable {
                return Err(ServiceError::new(
                    "read_failed",
                    "A selected track cannot be read",
                ));
            }
            let format = audio_format(&row.format).ok_or_else(|| {
                ServiceError::new("read_failed", "A selected track cannot be read")
            })?;
            Ok(TrackFile {
                path: path.clone(),
                format,
                tags: row
                    .tags
                    .iter()
                    .map(|(key, value)| (TagField::from_storage_key(key), value.clone()))
                    .collect(),
            })
        })
        .collect()
}

/// Inverse of the debug spelling `TrackDto::from` stores for `AudioFormat`.
fn audio_format(name: &str) -> Option<AudioFormat> {
    Some(match name {
        "Mp3" => AudioFormat::Mp3,
        "Flac" => AudioFormat::Flac,
        "OggVorbis" => AudioFormat::OggVorbis,
        "M4a" => AudioFormat::M4a,
        "Aac" => AudioFormat::Aac,
        "Aiff" => AudioFormat::Aiff,
        "Wav" => AudioFormat::Wav,
        "Opus" => AudioFormat::Opus,
        "Speex" => AudioFormat::Speex,
        "Musepack" => AudioFormat::Musepack,
        "MonkeysAudio" => AudioFormat::MonkeysAudio,
        "WavPack" => AudioFormat::WavPack,
        _ => return None,
    })
}

fn scan_page(id: &str, job: &ScanJob, args: &Value) -> ServiceResult<Value> {
    let status = lock(&job.status)?;
    let offset = number(args, "offset", 0);
    let limit = number(args, "limit", 100).clamp(1, MAX_PAGE);
    let mut response = json!({"job":id,"status":status.status,"phase":status.phase,
        "processed":status.processed,"total":job.paths.len(),"skipped":status.skipped,
        "errors":status.errors,"group_count":status.groups.len()});
    if job.kind == "bpm" {
        let detected = status
            .detected
            .iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        response["detected"] = json!(detected);
        response["next_offset"] = json!((offset.saturating_add(detected.len())
            < status.detected.len())
        .then_some(offset.saturating_add(detected.len())));
        return Ok(response);
    }
    if let Some(group) = args.get("group").and_then(Value::as_u64) {
        let group = status
            .groups
            .get(group as usize)
            .ok_or_else(|| ServiceError::invalid("Group is outside scan results"))?;
        let paths = group
            .indices
            .iter()
            .skip(offset)
            .take(limit)
            .filter_map(|i| job.paths.get(*i))
            .collect::<Vec<_>>();
        response["group"] = json!({"id":group.id,"label":group.label,"reason":group.reason,"confidence":group.confidence,"member_count":group.indices.len()});
        response["members"] = json!(paths);
        response["next_offset"] = json!((offset.saturating_add(paths.len()) < group.indices.len())
            .then_some(offset.saturating_add(paths.len())));
    } else {
        let groups = status.groups.iter().enumerate().skip(offset).take(limit).map(|(i,g)|json!({"index":i,"id":g.id,"label":g.label,"reason":g.reason,"confidence":g.confidence,"member_count":g.indices.len()})).collect::<Vec<_>>();
        response["next_offset"] = json!((offset.saturating_add(groups.len())
            < status.groups.len())
        .then_some(offset.saturating_add(groups.len())));
        response["groups"] = json!(groups);
    }
    Ok(response)
}

/// The owned path strings a scan job retains, and the bytes they actually
/// occupy. `to_string_lossy` can expand one invalid byte to a three-byte
/// replacement character, so the admission budget must use these lengths rather
/// than the source `OsStr` lengths.
fn retained_path_bytes(paths: &Vec<String>) -> usize {
    paths.iter().fold(
        paths
            .capacity()
            .saturating_mul(std::mem::size_of::<String>()),
        |bytes, path| bytes.saturating_add(path.capacity()),
    )
}

fn retained_paths(paths: &[PathBuf], limit: usize) -> ServiceResult<(Vec<String>, usize)> {
    let quota = || ServiceError::new("quota", "Analysis path storage exceeds its budget");
    if paths.len().saturating_mul(std::mem::size_of::<String>()) > limit {
        return Err(quota());
    }
    let mut retained = Vec::new();
    retained
        .try_reserve_exact(paths.len())
        .map_err(|_| quota())?;
    let mut bytes = retained_path_bytes(&retained);
    if bytes > limit {
        return Err(quota());
    }
    for path in paths {
        let value = path.to_string_lossy().into_owned();
        if value.capacity() > limit.saturating_sub(bytes) {
            return Err(quota());
        }
        bytes += value.capacity();
        retained.push(value);
    }
    Ok((retained, bytes))
}

fn safe_name(name: &str) -> ServiceResult<String> {
    let name = name.trim();
    if name.is_empty()
        || name.starts_with('.')
        || name.contains(['/', '\\', '\0', ':'])
        || name.ends_with(['.', ' '])
        || tagrex_core::scanner::is_supported_audio(Path::new(name))
    {
        return Err(ServiceError::invalid(
            "Output name must be a plain non-audio filename",
        ));
    }
    Ok(name.into())
}

fn portable_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| {
            p.components()
                .map(|p| p.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

fn output_error(error: tagrex_commands::OutputError) -> ServiceError {
    let code = match &error {
        tagrex_commands::OutputError::RecoveryRequired { .. } => "recovery_required",
        tagrex_commands::OutputError::Resource(tagrex_core::resources::AdmissionError::Busy) => {
            "resource_busy"
        }
        tagrex_commands::OutputError::Resource(
            tagrex_core::resources::AdmissionError::TooLarge,
        ) => "resource_limit",
        tagrex_commands::OutputError::Changed(_) => "stale_file",
        other if other.kind() == std::io::ErrorKind::AlreadyExists => "output_exists",
        _ => "output_failed",
    };
    ServiceError::new(code, error.to_string())
}

fn export_limit(_: export::OutputLimit) -> ServiceError {
    ServiceError::new(
        "quota",
        "Export exceeds the 64 MiB output budget or available memory",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, Service, Principal, Principal, Vec<PathBuf>) {
        let root = std::env::temp_dir().join(opaque("mcp-output-test").unwrap());
        let media = root.join("media");
        fs::create_dir_all(&media).unwrap();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&116u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&80u32.to_le_bytes());
        wav.extend_from_slice(&[0; 80]);
        let paths = vec![media.join("a.wav"), media.join("b.wav")];
        for path in &paths {
            fs::write(path, &wav).unwrap();
        }
        let app = App::open(&media, &root.join("journal.sqlite")).unwrap();
        let service = Service::new(Arc::new(Mutex::new(Some(app))), root.join("service")).unwrap();
        service.set_enabled(true).unwrap();
        let grant = Grant {
            roots: vec![media],
            actions: ["read".into(), "outputs".into()].into(),
            max_files: 10,
            ..Grant::default()
        };
        let one = service.pair_client("one".into(), grant.clone()).unwrap();
        let two = service.pair_client("two".into(), grant).unwrap();
        let one = service.authenticate(&one.credential).unwrap();
        let two = service.authenticate(&two.credential).unwrap();
        (root, service, one, two, paths)
    }

    #[test]
    fn failed_analysis_worker_finishes_and_can_be_released() {
        struct PanickingBpm;

        impl DesktopBackend for PanickingBpm {
            fn call(&self, _: &str, _: Value) -> ServiceResult<Value> {
                Err(ServiceError::invalid("Unexpected desktop action"))
            }

            fn bpm(
                &self,
                _: &[PathBuf],
                _: &AtomicBool,
                _: &mut dyn FnMut(usize, usize),
            ) -> ServiceResult<Value> {
                panic!("fixture analysis worker panic")
            }
        }

        let (root, service, actor, _, paths) = fixture();
        service.set_desktop_backend(Arc::new(PanickingBpm)).unwrap();
        let (send, received) = std::sync::mpsc::channel();
        service
            .set_event_sink(Arc::new(move |name, payload| {
                if name == "analysis_changed" && payload["complete"] == true {
                    let _ = send.send(payload);
                }
            }))
            .unwrap();
        let original = fs::read(&paths[0]).unwrap();
        let started = service
            .call(&actor, "analysis.bpm", json!({"paths":[paths[0]]}))
            .unwrap();
        let stopped = received.recv_timeout(std::time::Duration::from_secs(2));
        let status = service.call(
            &actor,
            "analysis.bpm",
            json!({"action":"status","job":started["job"]}),
        );
        let released = service.call(
            &actor,
            "analysis.bpm",
            json!({"action":"release","job":started["job"]}),
        );
        let unchanged = fs::read(&paths[0]).unwrap() == original;
        drop(service);
        fs::remove_dir_all(root).unwrap();

        assert_eq!(status.unwrap()["status"], "failed");
        assert_eq!(stopped.unwrap()["job"], started["job"]);
        assert_eq!(released.unwrap()["released"], started["job"]);
        assert!(unchanged);
    }

    #[test]
    fn export_previews_do_not_write_and_publication_never_overwrites_implicitly() {
        let (root, service, one, _, paths) = fixture();
        let original = fs::read(&paths[0]).unwrap();
        let args = json!({"paths":paths,"format":"csv","filename":"report.csv","preview":true});
        let preview = service.call(&one, "exports.create", args.clone()).unwrap();
        let target = root.join("media/report.csv");
        assert!(!target.exists());
        assert_eq!(preview["written"], false);
        let mut args = args;
        args["preview"] = json!(false);
        let written = service.call(&one, "exports.create", args.clone()).unwrap();
        assert_eq!(written["written"], true);
        let report = fs::read(&target).unwrap();
        assert_eq!(
            service
                .call(&one, "exports.create", args.clone())
                .unwrap_err()
                .code,
            "output_exists"
        );
        assert_eq!(fs::read(&target).unwrap(), report);
        assert_eq!(fs::read(&paths[0]).unwrap(), original);
        args["filename"] = json!("a.wav");
        args["overwrite"] = json!(true);
        assert!(service.call(&one, "exports.create", args).is_err());
        assert_eq!(fs::read(&paths[0]).unwrap(), original);
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_scope_and_symlink_targets_are_refused_without_modification() {
        let (root, service, one, _, paths) = fixture();
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        assert_eq!(
            service
                .call(
                    &one,
                    "exports.create",
                    json!({"paths":paths,"format":"csv","filename":"report.csv","root":outside})
                )
                .unwrap_err()
                .code,
            "permission_denied"
        );
        assert!(!outside.join("report.csv").exists());
        #[cfg(unix)]
        {
            let protected = outside.join("protected.txt");
            fs::write(&protected, b"original").unwrap();
            std::os::unix::fs::symlink(&protected, root.join("media/report.csv")).unwrap();
            assert!(service
                .call(
                    &one,
                    "exports.create",
                    json!({"paths":paths,"format":"csv","filename":"report.csv","overwrite":true})
                )
                .is_err());
            assert_eq!(fs::read(protected).unwrap(), b"original");
        }
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_freeze_refuses_publication_but_allows_read_only_manifests() {
        let (root, service, one, _, paths) = fixture();
        lock(&service.0.state).unwrap().recovery_required = true;
        let args = json!({"paths":paths,"format":"csv","filename":"report.csv"});
        assert_eq!(
            service
                .call(&one, "exports.create", args.clone())
                .unwrap_err()
                .code,
            "recovery_required"
        );
        let mut preview = args;
        preview["preview"] = json!(true);
        assert_eq!(
            service.call(&one, "exports.create", preview).unwrap()["written"],
            false
        );
        assert!(!root.join("media/report.csv").exists());
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_jobs_are_private_scoped_read_only_and_releasable() {
        let (root, service, one, two, paths) = fixture();
        let original = paths
            .iter()
            .map(|p| fs::read(p).unwrap())
            .collect::<Vec<_>>();
        let started = service
            .call(
                &one,
                "analysis.duplicates",
                json!({"paths":paths,"kind":"exact"}),
            )
            .unwrap();
        let id = started["job"].as_str().unwrap();
        let args = json!({"action":"status","job":id});
        assert_eq!(
            service
                .call(&two, "analysis.duplicates", args.clone())
                .unwrap_err()
                .code,
            "invalid_handle"
        );
        let mut terminal = None;
        for _ in 0..300 {
            let status = service
                .call(&one, "analysis.duplicates", args.clone())
                .unwrap();
            if status["status"] != "running" {
                terminal = Some(status);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let terminal = terminal.expect("bounded fixture scan did not finish");
        assert_eq!(terminal["status"], "complete");
        assert_eq!(terminal["total"], 2);
        assert_eq!(terminal["group_count"], 1);
        let members = service
            .call(
                &one,
                "analysis.duplicates",
                json!({"action":"status","job":id,"group":0}),
            )
            .unwrap();
        assert_eq!(members["members"].as_array().unwrap().len(), 2);
        for (path, old) in paths.iter().zip(original) {
            assert_eq!(fs::read(path).unwrap(), old);
        }
        service
            .call(
                &one,
                "analysis.duplicates",
                json!({"action":"release","job":id}),
            )
            .unwrap();
        assert_eq!(
            service
                .call(&one, "analysis.duplicates", args)
                .unwrap_err()
                .code,
            "invalid_handle"
        );
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn output_names_cannot_replace_audio_or_escape_roots() {
        for bad in [
            "", ".hidden", "../x.csv", "a/b.csv", "a\\b.csv", "x.FLAC", "x.mp3.", "C:x.csv",
            "x\0.csv",
        ] {
            assert!(safe_name(bad).is_err(), "accepted {bad:?}");
        }
        assert_eq!(safe_name("my report.csv").unwrap(), "my report.csv");
    }
    #[test]
    fn every_text_export_format_uses_the_shared_row_snapshot() {
        let (root, service, one, _, paths) = fixture();
        let cases = [
            ("csv", json!({}), "a.wav"),
            ("html", json!({}), "Tagnetic export"),
            ("xml", json!({}), "<library"),
            (
                "report",
                json!({"mask":"%filenameext%|%_codec%"}),
                "a.wav|WAV",
            ),
            ("playlist", json!({}), "a.wav"),
            ("cue", json!({}), "FILE \""),
        ];
        for (format, extra, expected) in cases {
            let mut args = json!({
                "paths": paths,
                "format": format,
                "filename": format!("out.{format}"),
                "preview": false,
            });
            for (key, value) in extra.as_object().unwrap() {
                args[key] = value.clone();
            }
            service.call(&one, "exports.create", args).unwrap();
            let text = fs::read_to_string(root.join(format!("media/out.{format}"))).unwrap();
            assert!(
                text.contains(expected),
                "{format} missing {expected:?}: {text}"
            );
        }
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_refuses_a_track_the_native_reader_cannot_read() {
        let (root, service, one, _, _) = fixture();
        let broken = root.join("media/broken.wav");
        fs::write(&broken, b"not a wave file").unwrap();
        for format in ["csv", "html", "xml", "report", "cue", "playlist", "m3u"] {
            assert_eq!(
            service
                .call(
                    &one,
                    "exports.create",
                    json!({"paths":[broken],"format":format,"filename":"broken.csv","preview":true})
                )
                .unwrap_err()
                .code,
            "read_failed"
        );
        }
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn analysis_admission_counts_the_retained_lossy_path_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let raw = OsStr::from_bytes(b"/music/\xff.wav");
        let path = PathBuf::from(raw);
        let (retained, bytes) =
            retained_paths(std::slice::from_ref(&path), MAX_ANALYSIS_PATH_BYTES).unwrap();
        assert_eq!(
            bytes,
            retained[0].capacity() + std::mem::size_of::<String>()
        );
        assert!(retained_paths(std::slice::from_ref(&path), bytes - 1).is_err());
        assert!(
            bytes > raw.len(),
            "an invalid byte expands in the retained string"
        );
    }

    #[test]
    fn result_pages_do_not_expand_large_groups() {
        let job = ScanJob {
            owner: "p".into(),
            epoch: "e".into(),
            kind: "exact".into(),
            cancel: AtomicBool::new(false),
            paths: (0..20_000).map(|n| format!("/music/{n}.flac")).collect(),
            status: Mutex::new(ScanStatus {
                phase: "complete".into(),
                status: "complete".into(),
                processed: 20_000,
                skipped: 0,
                errors: 0,
                detected: Vec::new(),
                groups: vec![tagrex_commands::query::DuplicateQueryGroup {
                    id: "g".into(),
                    label: "Track".into(),
                    reason: "Exact".into(),
                    confidence: "exact".into(),
                    indices: (0..20_000).collect(),
                }],
            }),
        };
        let headers = scan_page("j", &job, &json!({})).unwrap();
        assert_eq!(headers["groups"][0]["member_count"], 20_000);
        assert!(headers.get("members").is_none());
        let page = scan_page("j", &job, &json!({"group":0,"offset":19_900,"limit":500})).unwrap();
        assert_eq!(page["members"].as_array().unwrap().len(), 100);
        assert!(page["next_offset"].is_null());
        let page = scan_page("j", &job, &json!({"group":0,"limit":100000})).unwrap();
        assert_eq!(page["members"].as_array().unwrap().len(), 500);
    }
}
