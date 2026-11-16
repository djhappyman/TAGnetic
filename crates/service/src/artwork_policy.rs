//! Explicit agent artwork intents over native saved-value previews.
//!
//! `native` preserves the desktop's existing front-cover behavior. In
//! particular, an unlabelled `other` picture is not silently discarded. Agents
//! that have inspected the roles can explicitly choose whole-set replacement.
use super::*;
use base64::Engine;
use tagrex_commands::CoverChangeDto;
use tagrex_core::model::{CoverKind, TagEngine};

impl Service {
    pub(super) fn preview_artwork(
        &self,
        principal: &Principal,
        args: &Value,
        paths: &[PathBuf],
        app: &App,
        previous: Option<&PlanDto>,
    ) -> ServiceResult<PlanDto> {
        let grant = self.grant(principal, "stage")?;
        check_paths(&grant, paths, false)?;
        let mode = match args.get("mode") {
            None => "native",
            Some(Value::String(mode)) => mode.as_str(),
            _ => return Err(ServiceError::invalid("Artwork mode must be a string")),
        };
        if !["native", "replace_all", "replace_front", "append"].contains(&mode) {
            return Err(ServiceError::invalid(
                "Artwork mode must be native, replace_all, replace_front or append",
            ));
        }
        if flag(args, "remove") {
            return Ok(app.preview_cover_remove(paths)?);
        }
        let cover = {
            let state = lock(&self.0.state)?;
            state
                .assets
                .get(required(args, "asset")?)
                .filter(|asset| asset.owner == principal.id)
                .map(|asset| asset.cover.clone())
                .ok_or_else(missing_handle)?
        };
        if mode == "replace_all" {
            return Ok(app.preview_cover_set(paths, &[cover])?);
        }
        let has_prior_artwork = previous.is_some_and(|plan| {
            plan.changes.iter().any(|change| {
                change.cover_change.is_some()
                    && paths.iter().any(|path| path == Path::new(&change.path))
            })
        });
        if mode == "native" && !has_prior_artwork {
            return Ok(app.preview_cover_embed(paths, &cover)?);
        }

        let mut combined: Option<PlanDto> = None;
        for path in paths {
            // This producer prepares only the incoming image with the app's
            // resize policy, leaving existing Back/Other bytes untouched.
            let mut next = app.preview_cover_embed(std::slice::from_ref(path), &cover)?;
            let saved = match next.changes.first().and_then(|c| c.cover_change.as_ref()) {
                Some(change) => change
                    .old
                    .iter()
                    .map(|image| resolve_image(image, &next.cover_assets))
                    .collect::<ServiceResult<Vec<_>>>()?,
                None => desired_images(path, None)?,
            };
            let native_at = saved
                .iter()
                .position(|image| role(&image.kind) == "front")
                .unwrap_or(0);
            let prepared = match next.changes.first().and_then(|c| c.cover_change.as_ref()) {
                Some(change) => resolve_image(
                    change.new.get(native_at).ok_or_else(|| {
                        ServiceError::invalid("Native artwork preview has no prepared image")
                    })?,
                    &next.cover_assets,
                )?,
                None => saved.get(native_at).cloned().ok_or_else(|| {
                    ServiceError::invalid("Native artwork preview has no saved image")
                })?,
            };
            let mut desired = previous_images(path, previous)?.unwrap_or_else(|| saved.clone());
            match mode {
                "append" => desired.push(prepared.clone()),
                "replace_front" => {
                    let at = desired
                        .iter()
                        .position(|image| role(&image.kind) == "front")
                        .unwrap_or(0);
                    desired.retain(|image| role(&image.kind) != "front");
                    desired.insert(at.min(desired.len()), prepared.clone());
                }
                // Match the native producer: replace the first explicit Front,
                // leave all other roles and ordering intact, or insert at zero.
                "native" => match desired
                    .iter()
                    .position(|image| role(&image.kind) == "front")
                {
                    Some(at) => desired[at] = prepared.clone(),
                    None => desired.insert(0, prepared.clone()),
                },
                _ => return Err(ServiceError::invalid("Unsupported artwork mode")),
            }
            if desired == saved {
                next.changes.clear();
                next.cover_assets.clear();
            } else {
                // A native no-op still provides a server-owned old baseline
                // through removal; the desired collection is composed below.
                if next.changes.is_empty() {
                    next = app.preview_cover_remove(std::slice::from_ref(path))?;
                }
                let mut pool: HashMap<Arc<str>, usize> = next
                    .cover_assets
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(index, data)| (data, index))
                    .collect();
                for image in &mut desired {
                    let index = match pool.get(&image.data_base64) {
                        Some(index) => *index,
                        None => {
                            let index = next.cover_assets.len();
                            next.cover_assets.push(image.data_base64.clone());
                            pool.insert(image.data_base64.clone(), index);
                            index
                        }
                    };
                    image.data_ref = Some(index);
                    image.data_base64 = Arc::from("");
                }
                next.changes
                    .first_mut()
                    .and_then(|c| c.cover_change.as_mut())
                    .ok_or_else(|| {
                        ServiceError::invalid("Native artwork preview has no saved baseline")
                    })?
                    .new = desired;
            }
            combined = Some(match combined {
                Some(current) => super::plans::merge(current, next)?,
                None => next,
            });
        }
        combined.ok_or_else(|| ServiceError::invalid("Artwork selection is empty"))
    }
}

fn previous_images(
    path: &Path,
    previous: Option<&PlanDto>,
) -> ServiceResult<Option<Vec<CoverArtDto>>> {
    if let Some(plan) = previous {
        if let Some(change) = plan
            .changes
            .iter()
            .find(|change| Path::new(&change.path) == path)
            .and_then(|change| change.cover_change.as_ref())
        {
            return change
                .new
                .iter()
                .map(|cover| resolve_image(cover, &plan.cover_assets))
                .collect::<ServiceResult<Vec<_>>>()
                .map(Some);
        }
    }
    Ok(None)
}

fn desired_images(path: &Path, previous: Option<&PlanDto>) -> ServiceResult<Vec<CoverArtDto>> {
    if let Some(images) = previous_images(path, previous)? {
        return Ok(images);
    }
    let covers = TagEngine::read_covers(path)
        .map_err(|_| ServiceError::new("read_failed", "Selected artwork cannot be read"))?;
    Ok(covers
        .into_iter()
        .map(|cover| CoverArtDto {
            mime: cover.mime,
            data_base64: base64::engine::general_purpose::STANDARD
                .encode(&cover.data)
                .into(),
            data_ref: None,
            kind: cover.kind.to_storage_key().into(),
            description: cover.description,
        })
        .collect())
}

fn resolve_image(image: &CoverArtDto, assets: &[Arc<str>]) -> ServiceResult<CoverArtDto> {
    let mut image = image.clone();
    if let Some(index) = image.data_ref {
        image.data_base64 = assets.get(index).cloned().ok_or_else(|| {
            ServiceError::invalid("Prepared artwork has an invalid asset reference")
        })?;
        image.data_ref = None;
    }
    Ok(image)
}

fn role(kind: &str) -> &'static str {
    // Use the actual native codec, including Other for missing/unrecognized
    // legacy roles. The service defaults newly loaded artwork to Front before
    // it reaches this helper.
    CoverKind::from_storage_key(kind).to_storage_key()
}

/// A cheap, sanitized whole-set preview. Encoded bytes stay in the server pool;
/// role changes and an intentional replacement are visible before Apply.
pub(super) fn artwork_diff(change: &CoverChangeDto, assets: &[Arc<str>]) -> Value {
    let metadata = |images: &[CoverArtDto]| {
        images
            .iter()
            .map(|image| {
                let encoded = match image.data_ref {
                    Some(index) => assets.get(index).map(Arc::as_ref),
                    None => Some(image.data_base64.as_ref()),
                };
                let bytes = encoded.and_then(decoded_length);
                json!({"kind":role(&image.kind),"mime":image.mime,
                    "description":image.description,"bytes":bytes})
            })
            .collect::<Vec<_>>()
    };
    json!({"old_count":change.old.len(),"new_count":change.new.len(),
        "old":metadata(&change.old),"new":metadata(&change.new)})
}

fn decoded_length(encoded: &str) -> Option<usize> {
    if !encoded.len().is_multiple_of(4) {
        return None;
    }
    let bytes = encoded.as_bytes();
    let at_end = |distance: usize| {
        bytes
            .len()
            .checked_sub(distance)
            .and_then(|index| bytes.get(index))
            == Some(&b'=')
    };
    let padding = usize::from(at_end(1)) + usize::from(at_end(2));
    if at_end(3) {
        return None;
    }
    (encoded.len() / 4).checked_mul(3)?.checked_sub(padding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tagrex_core::model::CoverArt;

    struct Fixture {
        service: Service,
        path: PathBuf,
        principal: Principal,
        _directory: OwnedDirectory,
    }
    struct OwnedDirectory(PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    impl Fixture {
        fn new(roles: &[CoverKind]) -> Self {
            let root = std::env::temp_dir().join(opaque("mcp-artwork-policy").unwrap());
            let media = root.join("media");
            fs::create_dir_all(&media).unwrap();
            let path = media.join("track.wav");
            let mut wav = Vec::new();
            wav.extend(b"RIFF");
            wav.extend(356u32.to_le_bytes());
            wav.extend(b"WAVEfmt ");
            wav.extend(16u32.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(8000u32.to_le_bytes());
            wav.extend(16000u32.to_le_bytes());
            wav.extend(2u16.to_le_bytes());
            wav.extend(16u16.to_le_bytes());
            wav.extend(b"data");
            wav.extend(320u32.to_le_bytes());
            wav.extend([0u8; 320]);
            fs::write(&path, wav).unwrap();
            let covers = roles
                .iter()
                .enumerate()
                .map(|(index, kind)| CoverArt {
                    data: png(index as u8).into(),
                    mime: "image/png".into(),
                    kind: *kind,
                    description: format!("original {index}"),
                })
                .collect::<Vec<_>>();
            TagEngine::write_covers(&path, &covers).unwrap();
            let app = App::open(&media, &root.join("journal.sqlite")).unwrap();
            let service =
                Service::new(Arc::new(Mutex::new(Some(app))), root.join("service")).unwrap();
            service.set_enabled(true).unwrap();
            let pairing = service
                .pair_client(
                    "Artwork fixture".into(),
                    Grant {
                        roots: vec![media],
                        actions: [
                            "read".into(),
                            "stage".into(),
                            "write".into(),
                            "history".into(),
                        ]
                        .into(),
                        ..Grant::default()
                    },
                )
                .unwrap();
            let principal = service.authenticate(&pairing.credential).unwrap();
            Self {
                path,
                service,
                principal,
                _directory: OwnedDirectory(root),
            }
        }
        fn asset(&self, shade: u8, kind: &str) -> String {
            let id = opaque("artwork").unwrap();
            let bytes = png(shade);
            lock(&self.service.0.state).unwrap().assets.insert(
                id.clone(),
                Asset {
                    owner: self.principal.id.clone(),
                    cover: CoverArtDto {
                        mime: "image/png".into(),
                        data_base64: base64::engine::general_purpose::STANDARD
                            .encode(&bytes)
                            .into(),
                        kind: kind.into(),
                        ..CoverArtDto::default()
                    },
                    digest: hash(&bytes),
                    bytes: bytes.len(),
                    width: 2,
                    height: 2,
                },
            );
            id
        }
        fn preview(&self, asset: &str, mode: &str, previous: Option<&PlanDto>) -> PlanDto {
            let guard = lock(&self.service.0.app).unwrap();
            self.service
                .preview_artwork(
                    &self.principal,
                    &json!({"asset":asset,"mode":mode}),
                    std::slice::from_ref(&self.path),
                    guard.as_ref().unwrap(),
                    previous,
                )
                .unwrap()
        }
    }
    fn png(shade: u8) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(2, 2, image::Rgb([shade, 40, 100]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }
    fn images(plan: &PlanDto) -> Vec<CoverArtDto> {
        plan.changes[0]
            .cover_change
            .as_ref()
            .unwrap()
            .new
            .iter()
            .map(|image| resolve_image(image, &plan.cover_assets).unwrap())
            .collect()
    }
    fn pcm_data(path: &Path) -> Vec<u8> {
        // Independent RIFF chunk readback, deliberately not TagEngine/Lofty.
        let bytes = fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let mut offset = 12;
        while offset + 8 <= bytes.len() {
            let size =
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let start = offset + 8;
            let end = start.checked_add(size).unwrap();
            assert!(end <= bytes.len());
            if &bytes[offset..offset + 4] == b"data" {
                return bytes[start..end].to_vec();
            }
            offset = end + size % 2;
        }
        panic!("PCM data chunk was lost");
    }
    #[test]
    fn native_preserves_sole_other_but_explicit_whole_set_replacement_does_not() {
        let f = Fixture::new(&[CoverKind::Other]);
        let original = fs::read(&f.path).unwrap();
        let asset = f.asset(220, "front");
        let native = f.preview(&asset, "native", None);
        assert_eq!(
            images(&native)
                .iter()
                .map(|image| image.kind.as_str())
                .collect::<Vec<_>>(),
            ["front", "other"]
        );
        let explicit = f.preview(&asset, "replace_all", None);
        assert_eq!(
            images(&explicit)
                .iter()
                .map(|image| image.kind.as_str())
                .collect::<Vec<_>>(),
            ["front"]
        );
        let change = explicit.changes[0].cover_change.as_ref().unwrap();
        let diff = artwork_diff(change, &explicit.cover_assets);
        assert_eq!(diff["old"][0]["kind"], "other");
        assert_eq!(diff["new"][0]["kind"], "front");
        assert!(diff["new"][0]["bytes"].as_u64().unwrap() > 0);
        assert!(!diff.to_string().contains("data_base64"));
        assert_eq!(fs::read(&f.path).unwrap(), original);
    }
    #[test]
    fn replacing_front_preserves_back_other_and_appending_is_explicit() {
        let f = Fixture::new(&[CoverKind::Front, CoverKind::Back, CoverKind::Other]);
        let asset = f.asset(220, "front");
        let front = f.preview(&asset, "replace_front", None);
        assert_eq!(
            images(&front)
                .iter()
                .map(|image| image.kind.as_str())
                .collect::<Vec<_>>(),
            ["front", "back", "other"]
        );
        let append = f.preview(&asset, "append", None);
        assert_eq!(images(&append).len(), 4);
        assert_eq!(images(&append)[3].kind, "front");
    }
    #[test]
    fn composed_append_uses_staged_collection_and_resolves_pooled_bytes() {
        let f = Fixture::new(&[CoverKind::Other]);
        let front = f.asset(210, "front");
        let back = f.asset(211, "back");
        let first = f.preview(&front, "replace_all", None);
        let second = f.preview(&back, "append", Some(&first));
        assert_eq!(
            images(&second)
                .iter()
                .map(|image| image.kind.as_str())
                .collect::<Vec<_>>(),
            ["front", "back"]
        );
        assert_eq!(
            second.changes[0].cover_change.as_ref().unwrap().old[0].kind,
            "other"
        );
    }
    #[test]
    fn front_replacement_and_append_resize_only_the_incoming_image() {
        let f = Fixture::new(&[CoverKind::Front, CoverKind::Back, CoverKind::Other]);
        let old = TagEngine::read_covers(&f.path).unwrap();
        lock(&f.service.0.app)
            .unwrap()
            .as_ref()
            .unwrap()
            .apply_settings(&tagrex_commands::SettingsDto {
                cover_max_px: 1,
                ..tagrex_commands::SettingsDto::default()
            });
        let asset = f.asset(220, "front");
        let front = images(&f.preview(&asset, "replace_front", None));
        for index in [1, 2] {
            let data = base64::engine::general_purpose::STANDARD
                .decode(front[index].data_base64.as_bytes())
                .unwrap();
            assert_eq!(data.as_slice(), old[index].data.as_ref());
            assert_eq!(front[index].description, old[index].description);
        }
        let new = base64::engine::general_purpose::STANDARD
            .decode(front[0].data_base64.as_bytes())
            .unwrap();
        let decoded = image::load_from_memory(&new).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1, 1));
        let appended = images(&f.preview(&asset, "append", None));
        for (index, old) in old.iter().enumerate() {
            let data = base64::engine::general_purpose::STANDARD
                .decode(appended[index].data_base64.as_bytes())
                .unwrap();
            assert_eq!(data.as_slice(), old.data.as_ref());
        }
        assert_eq!(appended.len(), 4);
    }
    #[test]
    fn explicit_append_can_add_an_image_that_native_embedding_considers_unchanged() {
        let f = Fixture::new(&[CoverKind::Front]);
        let cover = desired_images(&f.path, None).unwrap().remove(0);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(cover.data_base64.as_bytes())
            .unwrap();
        let id = opaque("artwork").unwrap();
        lock(&f.service.0.state).unwrap().assets.insert(
            id.clone(),
            Asset {
                owner: f.principal.id.clone(),
                cover,
                digest: hash(&bytes),
                bytes: bytes.len(),
                width: 2,
                height: 2,
            },
        );
        assert!(f.preview(&id, "native", None).changes.is_empty());
        let append = f.preview(&id, "append", None);
        assert_eq!(images(&append).len(), 2);
        assert_eq!(
            append.changes[0].cover_change.as_ref().unwrap().old.len(),
            1
        );
    }
    #[test]
    fn reverting_to_saved_artwork_is_native_noop_even_after_staged_replacement() {
        let f = Fixture::new(&[CoverKind::Front]);
        let replacement = f.asset(210, "front");
        let first = f.preview(&replacement, "replace_all", None);
        let saved = desired_images(&f.path, None).unwrap().remove(0);
        let saved_bytes = base64::engine::general_purpose::STANDARD
            .decode(saved.data_base64.as_bytes())
            .unwrap();
        let id = opaque("artwork").unwrap();
        lock(&f.service.0.state).unwrap().assets.insert(
            id.clone(),
            Asset {
                owner: f.principal.id.clone(),
                cover: saved,
                digest: hash(&saved_bytes),
                bytes: saved_bytes.len(),
                width: 2,
                height: 2,
            },
        );
        let restored = f.preview(&id, "replace_all", Some(&first));
        assert!(restored.changes.is_empty());
    }
    #[test]
    fn whole_set_replacement_apply_and_native_undo_restore_exact_artwork() {
        let f = Fixture::new(&[CoverKind::Other, CoverKind::Back]);
        let old = TagEngine::read_covers(&f.path).unwrap();
        let audio = pcm_data(&f.path);
        let asset = f.asset(220, "front");
        let plan = f.preview(&asset, "replace_all", None);
        let mut guard = lock(&f.service.0.app).unwrap();
        let app = guard.as_mut().unwrap();
        let batch = app.apply(&plan).unwrap();
        let new = TagEngine::read_covers(&f.path).unwrap();
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].kind, CoverKind::Front);
        assert_eq!(pcm_data(&f.path), audio);
        app.undo(batch.id).unwrap();
        assert_eq!(TagEngine::read_covers(&f.path).unwrap(), old);
        assert_eq!(pcm_data(&f.path), audio);
    }
    #[test]
    fn unsupported_mode_and_foreign_asset_are_refused() {
        let f = Fixture::new(&[CoverKind::Other]);
        let asset = f.asset(220, "front");
        let guard = lock(&f.service.0.app).unwrap();
        let app = guard.as_ref().unwrap();
        assert_eq!(
            f.service
                .preview_artwork(
                    &f.principal,
                    &json!({"asset":asset,"mode":"surprise"}),
                    std::slice::from_ref(&f.path),
                    app,
                    None
                )
                .unwrap_err()
                .code,
            "invalid_argument"
        );
        lock(&f.service.0.state)
            .unwrap()
            .assets
            .get_mut(&asset)
            .unwrap()
            .owner = "other-client".into();
        assert_eq!(
            f.service
                .preview_artwork(
                    &f.principal,
                    &json!({"asset":asset,"mode":"replace_all"}),
                    std::slice::from_ref(&f.path),
                    app,
                    None
                )
                .unwrap_err()
                .code,
            "invalid_handle"
        );
    }

    #[test]
    fn metadata_counts_pooled_bytes_without_emitting_image_content() {
        for (encoded, count) in [("", 0), ("YQ==", 1), ("YWI=", 2), ("YWJj", 3)] {
            assert_eq!(decoded_length(encoded), Some(count));
        }
        assert_eq!(decoded_length("===="), None);
        assert_eq!(decoded_length("YQ="), None);
        assert_eq!(role(""), "other");
        assert_eq!(role("unrecognized"), "other");
        let image = CoverArtDto {
            kind: "back".into(),
            mime: "image/png".into(),
            data_ref: Some(0),
            description: "Original back".into(),
            ..CoverArtDto::default()
        };
        let change = CoverChangeDto {
            old: vec![image.clone()],
            new: vec![image],
        };
        let diff = artwork_diff(&change, &[Arc::from("YQ==")]);
        assert_eq!(diff["old"][0]["bytes"], 1);
        assert_eq!(diff["new"][0]["kind"], "back");
        assert!(!diff.to_string().contains("YQ=="));
        let invalid = artwork_diff(&change, &[]);
        assert!(invalid["old"][0]["bytes"].is_null());
    }
}
