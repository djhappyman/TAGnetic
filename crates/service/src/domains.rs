use super::*;
use base64::Engine;
use std::io::Cursor;

impl Service {
    pub(super) fn domain_call(
        &self,
        principal: &Principal,
        tool: &str,
        args: &Value,
    ) -> ServiceResult<Value> {
        match tool {
            "schema.catalog" => Ok(
                json!({"fields":tagrex_commands::import_fields(),"placeholders":tagrex_commands::mask_placeholders(),"builtin_transforms":tagrex_commands::builtin_action_groups(),"query_example":{"version":2,"text":"","mode":"search","root":{"kind":"group","id":"root","mode":"all","children":[]}},"limits":{"page":MAX_PAGE,"argument_bytes":MAX_ARGUMENT_BYTES,"image_bytes":MAX_IMAGE_BYTES}}),
            ),
            "handles.release" => {
                let id = required(args, "handle")?;
                let mut state = lock(&self.0.state)?;
                let mut released = false;
                if state
                    .snapshots
                    .get(id)
                    .is_some_and(|h| h.owner == principal.id)
                {
                    state.snapshots.remove(id);
                    released = true;
                }
                if state
                    .selections
                    .get(id)
                    .is_some_and(|h| h.owner == principal.id)
                {
                    state.selections.remove(id);
                    released = true;
                }
                if state
                    .assets
                    .get(id)
                    .is_some_and(|h| h.owner == principal.id)
                {
                    state.assets.remove(id);
                    released = true;
                }
                if state
                    .releases
                    .get(id)
                    .is_some_and(|h| h.owner == principal.id)
                {
                    state.releases.remove(id);
                    released = true;
                }
                if state
                    .image_receipts
                    .get(id)
                    .is_some_and(|h| h.owner == principal.id)
                {
                    state.image_receipts.remove(id);
                    released = true;
                }
                if !released {
                    return Err(missing_handle());
                }
                Ok(json!({"released":id}))
            }
            "settings.get" => {
                self.grant(principal, "settings")?;
                let settings = self.settings()?.read()?;
                Ok(
                    json!({"revision":settings_revision(&settings),"settings":safe_settings(&settings)}),
                )
            }
            "settings.patch" => {
                self.grant(principal, "settings")?;
                let backend = self.settings()?;
                let mut settings = backend.read()?;
                let expected = required(args, "revision")?;
                if settings_revision(&settings) != expected {
                    return Err(ServiceError::conflict(
                        "Settings changed; read again before patching",
                    ));
                }
                let patch = args
                    .get("patch")
                    .and_then(Value::as_object)
                    .ok_or_else(|| ServiceError::invalid("patch must be an object"))?;
                let safe = [
                    "id3_v23",
                    "read_priority",
                    "cover_max_px",
                    "cover_quality",
                    "import_cover",
                    "import_vinyl_position",
                    "import_skip_fields",
                    "multi_value_separator",
                    "legacy_code_page",
                    "bpm_floor",
                    "bpm_detection",
                    "bpm_max_minutes",
                    "portable_names",
                ];
                let map = settings.as_object_mut().ok_or_else(|| {
                    ServiceError::new("storage_error", "Saved settings must be an object")
                })?;
                for (key, value) in patch {
                    if !safe.contains(&key.as_str()) {
                        return Err(ServiceError::new(
                            "permission_denied",
                            "Only typed music-management preferences can be patched",
                        ));
                    }
                    map.insert(key.clone(), value.clone());
                }
                let _: tagrex_commands::SettingsDto = serde_json::from_value(settings.clone())?;
                backend.compare_and_write(expected, settings.clone())?;
                self.emit(
                    "settings_changed",
                    json!({"revision":settings_revision(&settings)}),
                );
                Ok(json!({"revision":settings_revision(&settings)}))
            }
            "presets.list" | "presets.put" | "presets.remove" => {
                self.preset_call(principal, tool, args)
            }
            "providers.status" => {
                self.grant(principal, "providers")?;
                self.providers()?.status()
            }
            "providers.search" => {
                let source = self.source(principal, args)?;
                let mut query: SearchQueryDto = serde_json::from_value(
                    args.get("query")
                        .cloned()
                        .ok_or_else(|| ServiceError::invalid("query is required"))?,
                )?;
                query.per_page = if query.per_page == 0 {
                    25
                } else {
                    query.per_page.clamp(1, 100)
                };
                let results = self.providers()?.search(&source, &query)?;
                Ok(json!({"source":source,"results":results}))
            }
            "providers.release" => {
                let source = self.source(principal, args)?;
                let requested = required(args, "id")?;
                if requested.len() > 2048 || requested.chars().any(char::is_control) {
                    return Err(ServiceError::invalid("Invalid provider release identifier"));
                }
                if requested.contains("://") {
                    validate_source_url(&source, requested, false)?;
                }
                let release = self.providers()?.fetch_release(&source, requested)?;
                let id = opaque("release")?;
                let mut images = Vec::new();
                let mut state = lock(&self.0.state)?;
                quota(state.releases.len())?;
                for image in &release.images {
                    if validate_source_url(&source, &image.url, true).is_err() {
                        continue;
                    }
                    quota(state.image_receipts.len())?;
                    let receipt = opaque("image")?;
                    state.image_receipts.insert(
                        receipt.clone(),
                        ImageReceipt {
                            owner: principal.id.clone(),
                            source: source.clone(),
                            url: image.url.clone(),
                        },
                    );
                    images.push(json!({"image":receipt,"width":image.width,"height":image.height}));
                }
                if images.is_empty() {
                    if let Some(url) = &release.cover_image_url {
                        if validate_source_url(&source, url, true).is_ok() {
                            let receipt = opaque("image")?;
                            state.image_receipts.insert(
                                receipt.clone(),
                                ImageReceipt {
                                    owner: principal.id.clone(),
                                    source: source.clone(),
                                    url: url.clone(),
                                },
                            );
                            images.push(json!({"image":receipt}));
                        }
                    }
                }
                state.releases.insert(
                    id.clone(),
                    ReleaseReceipt {
                        owner: principal.id.clone(),
                        source: source.clone(),
                        release: release.clone(),
                    },
                );
                let total_tracks = release.tracks.len();
                let mut data = serde_json::to_value(release)?;
                if let Some(data) = data.as_object_mut() {
                    data.remove("images");
                    data.remove("cover_image_url");
                    if let Some(tracks) = data.get_mut("tracks").and_then(Value::as_array_mut) {
                        tracks.truncate(100);
                    }
                }
                Ok(
                    json!({"release":id,"source":source,"data":data,"images":images,"total_tracks":total_tracks}),
                )
            }
            "providers.release_tracks" => {
                let state = lock(&self.0.state)?;
                let receipt = state
                    .releases
                    .get(required(args, "release")?)
                    .filter(|r| r.owner == principal.id)
                    .ok_or_else(missing_handle)?;
                let offset = number(args, "offset", 0);
                let limit = number(args, "limit", 100).clamp(1, MAX_PAGE);
                let tracks: Vec<_> = receipt
                    .release
                    .tracks
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .collect();
                Ok(
                    json!({"tracks":tracks,"offset":offset,"total":receipt.release.tracks.len(),"next_offset":(offset+tracks.len()<receipt.release.tracks.len()).then_some(offset+tracks.len())}),
                )
            }
            "artwork.fetch" => {
                self.grant(principal, "providers")?;
                let receipt = lock(&self.0.state)?
                    .image_receipts
                    .get(required(args, "image")?)
                    .filter(|r| r.owner == principal.id)
                    .cloned()
                    .ok_or_else(missing_handle)?;
                let grant = self.grant(principal, "providers")?;
                if !grant.providers.is_empty() && !grant.providers.contains(&receipt.source) {
                    return Err(denied());
                }
                validate_source_url(&receipt.source, &receipt.url, true)?;
                let cover = self
                    .providers()?
                    .fetch_image(&receipt.source, &receipt.url)?;
                self.store_asset(principal, cover)
            }
            "artwork.load" => {
                let grant = self.grant(principal, "read")?;
                let path = Path::new(required(args, "path")?);
                let canonical = permitted(&grant, path, false)?;
                if fs::metadata(&canonical)?.len() > MAX_IMAGE_BYTES as u64 {
                    return Err(ServiceError::new("quota", "Artwork exceeds 16 MiB"));
                }
                let bytes = fs::read(canonical)?;
                let cover = CoverArtDto {
                    data_base64: base64::engine::general_purpose::STANDARD
                        .encode(bytes)
                        .into(),
                    mime: String::new(),
                    kind: args
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("front")
                        .into(),
                    description: String::new(),
                    data_ref: None,
                };
                self.store_asset(principal, cover)
            }
            "artwork.upload" => {
                self.grant(principal, "stage")?;
                let cover = CoverArtDto {
                    data_base64: required(args, "data_base64")?.into(),
                    mime: String::new(),
                    kind: args
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("front")
                        .into(),
                    description: String::new(),
                    data_ref: None,
                };
                self.store_asset(principal, cover)
            }
            "artwork.inspect" => {
                let state = lock(&self.0.state)?;
                let asset = state
                    .assets
                    .get(required(args, "asset")?)
                    .filter(|a| a.owner == principal.id)
                    .ok_or_else(missing_handle)?;
                Ok(
                    json!({"digest":asset.digest,"bytes":asset.bytes,"mime":asset.cover.mime,"kind":asset.cover.kind,"width":asset.width,"height":asset.height}),
                )
            }
            "artwork.read" => {
                let state = lock(&self.0.state)?;
                let asset = state
                    .assets
                    .get(required(args, "asset")?)
                    .filter(|a| a.owner == principal.id)
                    .ok_or_else(missing_handle)?;
                let offset = number(args, "offset", 0);
                let limit = number(args, "limit", 64 * 1024).clamp(1, 128 * 1024);
                let encoded = asset.cover.data_base64.as_bytes();
                let end = offset.saturating_add(limit).min(encoded.len());
                if offset > end {
                    return Err(ServiceError::invalid("Artwork offset outside asset"));
                }
                Ok(
                    json!({"encoding":"base64","offset":offset,"data":std::str::from_utf8(&encoded[offset..end]).map_err(|_|ServiceError::invalid("Invalid artwork encoding"))?,"next_offset":(end<encoded.len()).then_some(end),"total":encoded.len()}),
                )
            }
            "artwork.embedded" => {
                let paths = self.paths(principal, args, "read")?;
                if paths.len() != 1 {
                    return Err(ServiceError::invalid(
                        "Select one file for an explicit embedded-artwork read",
                    ));
                }
                let art = tagrex_core::model::TagEngine::read_cover(&paths[0])
                    .map_err(|e| ServiceError::new("application_error", e.to_string()))?;
                if let Some(art) = art {
                    self.store_asset(
                        principal,
                        CoverArtDto {
                            mime: art.mime,
                            data_base64: base64::engine::general_purpose::STANDARD
                                .encode(&art.data)
                                .into(),
                            kind: "front".into(),
                            description: String::new(),
                            data_ref: None,
                        },
                    )
                } else {
                    Ok(json!({"asset":null}))
                }
            }
            "masks.render" => {
                let paths = self.paths(principal, args, "read")?;
                if paths.len() > MAX_PAGE {
                    return Err(ServiceError::new(
                        "quota",
                        "Render at most 500 rows per request",
                    ));
                }
                let app = lock(&self.0.app)?;
                let app = app.as_ref().ok_or_else(no_library)?;
                app.read_track_rows(&paths)?;
                let selected = paths
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect::<BTreeSet<_>>();
                let working = {
                    let state = lock(&self.0.state)?;
                    state.manual_plan.as_ref().map(|plan| {
                        super::staging::filtered(plan, |key| selected.contains(key.path.as_ref()))
                    })
                };
                let values = if let Some(working) = working {
                    app.render_column_over_plan(required(args, "mask")?, &paths, &working)?
                } else {
                    app.render_column(required(args, "mask")?, &paths)?
                };
                Ok(json!({"values":values}))
            }
            "files.locks" => {
                let app = lock(&self.0.app)?;
                let app = app.as_ref().ok_or_else(no_library)?;
                Ok(json!({"fields":app.locked_fields()}))
            }
            "files.trash" => {
                let grant = self.grant(principal, "trash")?;
                self.grant(principal, "write")?;
                let paths = self.paths(principal, args, "trash")?;
                check_paths(&grant, &paths, true)?;
                let _mutation = lock(&self.0.mutation)?;
                if lock(&self.0.state)?.recovery_required {
                    return Err(ServiceError::new(
                        "recovery_required",
                        "Resolve the uncertain operation before trashing files",
                    ));
                }
                if lock(&self.0.state)?
                    .manual_plan
                    .as_ref()
                    .is_some_and(|p| !p.changes.is_empty())
                {
                    return Err(ServiceError::conflict(
                        "Finish the person's staged plan before trashing files",
                    ));
                }
                if required(args, "confirmation")? != "move to system trash" {
                    return Err(ServiceError::invalid("Explicit confirmation 'move to system trash' is required; generic Undo is unavailable"));
                }
                let mut app = lock(&self.0.app)?;
                let app = app.as_mut().ok_or_else(no_library)?;
                app.read_track_rows(&paths)?;
                let trashed = app.trash_files(&paths)?;
                self.emit("session_changed", json!({"refresh":true}));
                Ok(
                    json!({"trashed":trashed,"undo_available":false,"recovery":"Use the operating system's trash"}),
                )
            }
            "exports.create" | "artwork.export" | "analysis.duplicates" | "analysis.bpm" => {
                self.output_call(principal, tool, args)
            }
            "session.open" | "session.refresh" | "playback.status" | "playback.play"
            | "playback.pause" | "playback.stop" | "playback.seek" | "playback.volume"
            | "ui.present" | "ui.preferences" => {
                let action = if tool.starts_with("playback.") {
                    "playback"
                } else if tool.starts_with("ui.") {
                    "ui"
                } else {
                    "open"
                };
                let grant = self.grant(principal, action)?;
                if tool == "session.open" {
                    let paths: Vec<PathBuf> = serde_json::from_value(
                        args.get("paths")
                            .cloned()
                            .ok_or_else(|| ServiceError::invalid("paths is required"))?,
                    )?;
                    check_paths(&grant, &paths, false)?;
                }
                if tool == "playback.play" {
                    let paths = self.paths(principal, args, "playback")?;
                    if paths.len() != 1 {
                        return Err(ServiceError::invalid("Select one track to play"));
                    }
                }
                let backend = lock(&self.0.desktop)?.clone().ok_or_else(|| {
                    ServiceError::new("unavailable", "This host has no desktop adapter")
                })?;
                let mut normalized = args.clone();
                if tool == "playback.play" {
                    let paths = self.paths(principal, args, "playback")?;
                    normalized = json!({"path":paths[0]});
                }
                if tool == "playback.volume" {
                    normalized =
                        json!({"level":args.get("volume").cloned().unwrap_or(Value::Null)});
                }
                if tool == "ui.present" {
                    if let Some(id) = args.get("draft").and_then(Value::as_str) {
                        let state = lock(&self.0.state)?;
                        if !state.plans.get(id).is_some_and(|p| p.owner == principal.id) {
                            return Err(missing_handle());
                        }
                        normalized["principal_id"] = json!(principal.id);
                    }
                }
                backend.call(tool, normalized)
            }
            _ => Err(ServiceError::new(
                "unknown_tool",
                "No such application tool",
            )),
        }
    }
    fn source(&self, principal: &Principal, args: &Value) -> ServiceResult<String> {
        let grant = self.grant(principal, "providers")?;
        let source = required(args, "source")?;
        if !["discogs", "musicbrainz", "beatport", "bandcamp", "soundeo"].contains(&source) {
            return Err(ServiceError::invalid("Unsupported metadata provider"));
        }
        if !grant.providers.is_empty() && !grant.providers.contains(source) {
            return Err(denied());
        }
        Ok(source.into())
    }
    fn store_asset(&self, principal: &Principal, mut cover: CoverArtDto) -> ServiceResult<Value> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(cover.data_base64.as_bytes())
            .map_err(|_| ServiceError::invalid("Invalid artwork base64"))?;
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return Err(ServiceError::new(
                "quota",
                "Artwork must have 1..16 MiB encoded bytes",
            ));
        }
        let format = image::guess_format(&bytes).map_err(|_| {
            ServiceError::invalid("Artwork must be a recognized PNG, JPEG or WebP image")
        })?;
        if !matches!(
            format,
            image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
        ) {
            return Err(ServiceError::invalid(
                "Only PNG, JPEG and WebP artwork is supported",
            ));
        }
        let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader.decode().map_err(|_| {
            ServiceError::new(
                "invalid_artwork",
                "Image decoder rejected the artwork or its resource limits",
            )
        })?;
        let width = decoded.width();
        let height = decoded.height();
        if u64::from(width) * u64::from(height) > 24_000_000 {
            return Err(ServiceError::new("quota", "Artwork exceeds 24 megapixels"));
        }
        cover.mime = format.to_mime_type().into();
        let digest = hash(&bytes);
        let id = opaque("artwork")?;
        let mut state = lock(&self.0.state)?;
        quota(state.assets.len())?;
        if state.assets.values().map(|a| a.bytes).sum::<usize>() + bytes.len() > 64 * 1024 * 1024 {
            return Err(ServiceError::new(
                "quota",
                "Prepared artwork exceeds the 64 MiB service budget; release unused handles",
            ));
        }
        state.assets.insert(
            id.clone(),
            Asset {
                owner: principal.id.clone(),
                cover,
                digest: digest.clone(),
                bytes: bytes.len(),
                width,
                height,
            },
        );
        Ok(
            json!({"asset":id,"digest":digest,"bytes":bytes.len(),"width":width,"height":height,"mime":format.to_mime_type()}),
        )
    }
    fn preset_call(&self, principal: &Principal, tool: &str, args: &Value) -> ServiceResult<Value> {
        self.grant(principal, "settings")?;
        let backend = self.settings()?;
        let mut settings = backend.read()?;
        let revision = settings_revision(&settings);
        let kind = required(args, "kind")?;
        let slice = match kind {
            "filter" => "query_presets",
            "mask" => "mask_presets",
            "transform" => "action_groups",
            _ => {
                return Err(ServiceError::invalid(
                    "Preset kind must be filter, mask or transform",
                ))
            }
        };
        if tool == "presets.list" {
            return Ok(
                json!({"kind":kind,"revision":revision,"presets":settings.get(slice).cloned().unwrap_or(json!([]))}),
            );
        }
        if required(args, "revision")? != revision {
            return Err(ServiceError::conflict(
                "Preset/settings revision changed; read again",
            ));
        }
        let name = required(args, "name")?;
        if name.len() > 128 || name.chars().any(char::is_control) {
            return Err(ServiceError::invalid(
                "Preset names must be short, printable text",
            ));
        }
        let mut value = args.get("preset").cloned().unwrap_or(json!({"name":name}));
        if tool == "presets.put" {
            if !value.is_object() {
                return Err(ServiceError::invalid("preset must be an object"));
            }
            value["name"] = json!(name);
            match kind {
                "mask" => {
                    let dto: tagrex_commands::MaskPresetDto =
                        serde_json::from_value(value.clone())?;
                    if dto.mask.len() > 8192 {
                        return Err(ServiceError::invalid("Mask exceeds 8192 bytes"));
                    }
                    tagrex_core::mask::Mask::parse(&dto.mask)
                        .map_err(|e| ServiceError::invalid(e.to_string()))?;
                }
                "transform" => {
                    let dto: tagrex_commands::ActionGroupDto =
                        serde_json::from_value(value.clone())?;
                    if dto.rules.len() > 64 {
                        return Err(ServiceError::invalid(
                            "A transform preset supports at most 64 steps",
                        ));
                    }
                    tagrex_commands::transform_sample(&dto, "Example Feat. Artist", &dto.scope)
                        .map_err(ServiceError::from)?;
                }
                "filter" => {
                    let query: tagrex_commands::query::QueryModel =
                        serde_json::from_value(value.get("query").cloned().ok_or_else(|| {
                            ServiceError::invalid("Filter preset requires query")
                        })?)?;
                    if query.regex || query.case_sensitive {
                        return Err(ServiceError::invalid(
                            "App search presets use literal case-insensitive search",
                        ));
                    }
                    let index = tagrex_commands::query::QueryIndex::from_tracks(&[]);
                    index
                        .query(
                            &tagrex_commands::query::QueryRequest {
                                request_id: 1,
                                snapshot_id: None,
                                query,
                            },
                            || false,
                        )
                        .map_err(|e| ServiceError::invalid(e.to_string()))?;
                }
                _ => {}
            }
            if serde_json::to_vec(&value)?.len() > 64 * 1024 {
                return Err(ServiceError::new("quota", "Preset exceeds 64 KiB"));
            }
        }
        let map = settings
            .as_object_mut()
            .ok_or_else(|| ServiceError::new("storage_error", "Settings must be an object"))?;
        let entries = map
            .entry(slice)
            .or_insert(json!([]))
            .as_array_mut()
            .ok_or_else(|| ServiceError::new("storage_error", "Preset collection is malformed"))?;
        if tool == "presets.remove" {
            entries.retain(|entry| entry.get("name").and_then(Value::as_str) != Some(name));
        } else if let Some(existing) = entries
            .iter_mut()
            .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
        {
            *existing = value;
        } else {
            if entries.len() >= 512 {
                return Err(ServiceError::new(
                    "quota",
                    "Preset collection supports at most 512 entries",
                ));
            }
            entries.push(value);
        }
        backend.compare_and_write(&revision, settings.clone())?;
        self.emit(
            "settings_changed",
            json!({"slice":slice,"revision":settings_revision(&settings)}),
        );
        Ok(json!({"revision":settings_revision(&settings),"kind":kind,"name":name}))
    }
}
fn safe_settings(settings: &Value) -> Value {
    let mut value = settings.clone();
    if let Some(map) = value.as_object_mut() {
        map.remove("proxy");
        map.retain(|key, _| {
            !key.contains("token")
                && !key.contains("secret")
                && !key.contains("password")
                && !key.contains("credential")
        });
    }
    value
}
fn validate_source_url(source: &str, url: &str, image: bool) -> ServiceResult<()> {
    if !url.starts_with("https://") || url.len() > 8192 {
        return Err(ServiceError::new(
            "unsafe_url",
            "Provider resources must use HTTPS",
        ));
    }
    let authority = url[8..].split('/').next().unwrap_or("");
    if authority.contains('@')
        || authority.contains(':')
        || authority.contains('\\')
        || authority.is_empty()
    {
        return Err(ServiceError::new(
            "unsafe_url",
            "Provider resource authority is invalid",
        ));
    }
    let host = authority.to_ascii_lowercase();
    let suffix = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    let allowed = match source {
        "discogs" => suffix("discogs.com"),
        "musicbrainz" => {
            suffix("musicbrainz.org")
                || (image && (suffix("coverartarchive.org") || suffix("archive.org")))
        }
        "beatport" => suffix("beatport.com") || (image && suffix("bpmd.net")),
        "bandcamp" => suffix("bandcamp.com") || (image && suffix("bcbits.com")),
        "soundeo" => suffix("soundeo.com"),
        _ => false,
    };
    if !allowed {
        return Err(ServiceError::new(
            "unsafe_url",
            "Image/release URL is outside the source's permitted hosts",
        ));
    }
    Ok(())
}
