use super::*;
use std::time::{Duration, Instant};

struct Fixture {
    root: PathBuf,
    media: PathBuf,
    paths: Vec<PathBuf>,
    service: Service,
    principal: Principal,
    credential: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(opaque("tagnetic-service-tests").unwrap());
        let media = root.join("media");
        fs::create_dir_all(&media).unwrap();
        let paths = (0..3)
            .map(|i| {
                let path = media.join(format!("Track {i}.wav"));
                let mut bytes = Vec::new();
                bytes.extend(b"RIFF");
                bytes.extend(356u32.to_le_bytes());
                bytes.extend(b"WAVEfmt ");
                bytes.extend(16u32.to_le_bytes());
                bytes.extend(1u16.to_le_bytes());
                bytes.extend(1u16.to_le_bytes());
                bytes.extend(8000u32.to_le_bytes());
                bytes.extend(16000u32.to_le_bytes());
                bytes.extend(2u16.to_le_bytes());
                bytes.extend(16u16.to_le_bytes());
                bytes.extend(b"data");
                bytes.extend(320u32.to_le_bytes());
                bytes.extend([0u8; 320]);
                fs::write(&path, bytes).unwrap();
                path
            })
            .collect::<Vec<_>>();
        let app = App::open(&media, &root.join("journal.sqlite")).unwrap();
        let service = Service::new(Arc::new(Mutex::new(Some(app))), root.join("service")).unwrap();
        service.set_enabled(true).unwrap();
        let grant = Grant {
            roots: vec![media.clone()],
            actions: [
                "read",
                "stage",
                "write",
                "history",
                "settings",
                "providers",
                "outputs",
                "open",
                "playback",
                "ui",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            ..Grant::default()
        };
        let pairing = service.pair_client("Fixture Codex".into(), grant).unwrap();
        let principal = service.authenticate(&pairing.credential).unwrap();
        Self {
            root,
            media,
            paths,
            service,
            principal,
            credential: pairing.credential,
        }
    }
    fn call(&self, name: &str, args: Value) -> ServiceResult<Value> {
        self.service.call(&self.principal, name, args)
    }
    fn draft(&self, value: &str) -> Value {
        self.call(
            "draft.create",
            json!({"paths":[self.paths[0]],"values":{"title":value}}),
        )
        .unwrap()
    }
    fn commit(&self, draft: &Value, key: &str) -> Value {
        self.call(
            "plan.commit",
            json!({"draft":draft["draft"],"digest":draft["digest"],"idempotency_key":key}),
        )
        .unwrap()
    }
    fn wait(&self, operation: &Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let value = self
                .call(
                    "operations.status",
                    json!({"operation":operation["operation"]}),
                )
                .unwrap();
            let record = &value["operations"][0];
            if !matches!(
                record["status"].as_str(),
                Some("queued" | "preflight" | "writing" | "rolling_back")
            ) {
                return record.clone();
            }
            assert!(
                Instant::now() < deadline,
                "operation did not finish: {value}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn saved(&self) -> String {
        let app = self.service.0.app.lock().unwrap();
        app.as_ref()
            .unwrap()
            .read_track(&self.paths[0])
            .unwrap()
            .tags
            .get("title")
            .cloned()
            .unwrap_or_default()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.service.stop_all();
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn catalog_is_unique_and_serializable() {
    let catalog = Service::tool_catalog();
    assert!(catalog.len() >= 60);
    let names: BTreeSet<_> = catalog.iter().map(|t| &t.name).collect();
    assert_eq!(names.len(), catalog.len());
    for tool in catalog {
        assert_eq!(tool.input_schema["additionalProperties"], false);
        assert!(!tool.description.is_empty());
        let _: ToolDefinition =
            serde_json::from_value(serde_json::to_value(tool).unwrap()).unwrap();
    }
}
#[test]
fn credentials_are_hashed_on_disk_and_revocation_is_immediate() {
    let f = Fixture::new();
    let bytes = fs::read(f.root.join("service/clients.json")).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(&f.credential));
    f.service.revoke_client(f.principal.id()).unwrap();
    assert_eq!(
        f.call("session.status", json!({})).unwrap_err().code,
        "permission_denied"
    );
    assert!(f.service.authenticate(&f.credential).is_err());
}
#[test]
fn revocation_releases_image_receipt_capacity_without_touching_another_owner() {
    let f = Fixture::new();
    {
        let mut state = lock(&f.service.0.state).unwrap();
        for index in 0..MAX_HANDLES - 1 {
            state.image_receipts.insert(
                format!("image-{index}"),
                ImageReceipt {
                    owner: f.principal.id.clone(),
                    source: "discogs".into(),
                    url: "https://i.discogs.com/fixture.jpg".into(),
                },
            );
        }
        state.image_receipts.insert(
            "other".into(),
            ImageReceipt {
                owner: "another-principal".into(),
                source: "discogs".into(),
                url: "https://i.discogs.com/other.jpg".into(),
            },
        );
    }
    f.service.revoke_client(f.principal.id()).unwrap();
    let state = lock(&f.service.0.state).unwrap();
    assert_eq!(state.image_receipts.len(), 1);
    assert_eq!(state.image_receipts["other"].owner, "another-principal");
}
#[test]
fn disabled_service_denies_even_previously_authenticated_client() {
    let f = Fixture::new();
    f.service.set_enabled(false).unwrap();
    assert!(f.call("session.status", json!({})).is_err());
    assert!(f.service.authenticate(&f.credential).is_err());
}
#[test]
fn expired_grants_refuse_authentication() {
    let f = Fixture::new();
    let p = f
        .service
        .pair_client(
            "Expired".into(),
            Grant {
                roots: vec![f.media.clone()],
                expires_at: Some(now() - 1),
                ..Grant::default()
            },
        )
        .unwrap();
    assert!(f.service.authenticate(&p.credential).is_err());
}
#[test]
fn snapshot_pagination_and_projection_are_bounded() {
    let f = Fixture::new();
    let snapshot = f.call("files.snapshot", json!({})).unwrap();
    assert_eq!(snapshot["total"], 3);
    let page = f
        .call(
            "files.page",
            json!({"snapshot":snapshot["snapshot"],"limit":1,"fields":["artist"]}),
        )
        .unwrap();
    assert_eq!(page["rows"].as_array().unwrap().len(), 1);
    assert_eq!(page["next_offset"], 1);
    assert!(page["rows"][0]["tags"].as_object().unwrap().is_empty());
}
#[test]
fn private_handles_cannot_be_used_by_another_paired_client() {
    let f = Fixture::new();
    let draft = f.draft("Private");
    let snapshot = f.call("files.snapshot", json!({})).unwrap();
    let p = f
        .service
        .pair_client(
            "Other".into(),
            Grant {
                roots: vec![f.media.clone()],
                ..Grant::default()
            },
        )
        .unwrap();
    let other = f.service.authenticate(&p.credential).unwrap();
    for (tool, args) in [
        ("draft.get", json!({"draft":draft["draft"]})),
        ("files.page", json!({"snapshot":snapshot["snapshot"]})),
    ] {
        assert_eq!(
            f.service.call(&other, tool, args).unwrap_err().code,
            "invalid_handle"
        );
    }
}
#[test]
fn staging_does_not_write_and_plan_diff_has_no_executable_snapshots() {
    let f = Fixture::new();
    let before = fs::read(&f.paths[0]).unwrap();
    let draft = f.draft("Pending Feat. Test");
    assert_eq!(fs::read(&f.paths[0]).unwrap(), before);
    assert_eq!(draft["changes"][0]["tags"][0]["new"], "Pending Feat. Test");
    let text = serde_json::to_string(&draft).unwrap();
    assert!(!text.contains("cover_assets"));
    assert!(!text.contains("data_base64"));
}
#[test]
fn direct_write_retry_creates_exactly_one_journal_batch() {
    let f = Fixture::new();
    let args = json!({"paths":[f.paths[0]],"values":{"title":"Written"},"idempotency_key":"direct-write-1"});
    let op = f.call("files.write", args.clone()).unwrap();
    assert_eq!(f.wait(&op)["status"], "succeeded");
    assert_eq!(f.saved(), "Written");
    let retry = f.call("files.write", args).unwrap();
    assert_eq!(retry["operation"], op["operation"]);
    assert_eq!(
        f.call("history.list", json!({})).unwrap()["batches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn simultaneous_retries_reserve_one_intent() {
    let f = Fixture::new();
    let args =
        json!({"paths":[f.paths[0]],"values":{"title":"Concurrent"},"idempotency_key":"same-key"});
    let mut workers = Vec::new();
    for _ in 0..4 {
        let service = f.service.clone();
        let principal = f.principal.clone();
        let args = args.clone();
        workers.push(std::thread::spawn(move || {
            service.call(&principal, "files.write", args).unwrap()
        }));
    }
    let results = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .collect::<Vec<_>>();
    let id = results[0]["operation"].clone();
    assert!(results.iter().all(|r| r["operation"] == id));
    assert_eq!(f.wait(&results[0])["status"], "succeeded");
    assert_eq!(
        f.call("history.list", json!({})).unwrap()["batches"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn idempotency_key_cannot_be_reused_for_different_intent() {
    let f = Fixture::new();
    let op = f
        .call(
            "files.write",
            json!({"paths":[f.paths[0]],"values":{"title":"One"},"idempotency_key":"intent"}),
        )
        .unwrap();
    f.wait(&op);
    assert_eq!(
        f.call(
            "files.write",
            json!({"paths":[f.paths[0]],"values":{"title":"Two"},"idempotency_key":"intent"})
        )
        .unwrap_err()
        .code,
        "conflict"
    );
}
#[test]
fn digest_mismatch_refuses_without_writing() {
    let f = Fixture::new();
    let draft = f.draft("No write");
    let before = fs::read(&f.paths[0]).unwrap();
    assert_eq!(
        f.call(
            "plan.commit",
            json!({"draft":draft["draft"],"digest":"forged","idempotency_key":"wrong"})
        )
        .unwrap_err()
        .code,
        "digest_mismatch"
    );
    assert_eq!(fs::read(&f.paths[0]).unwrap(), before);
}
#[test]
fn replacement_after_prepare_is_refused() {
    let f = Fixture::new();
    let draft = f.draft("No write");
    let replacement = f.media.join("replacement.wav");
    fs::copy(&f.paths[0], &replacement).unwrap();
    fs::rename(replacement, &f.paths[0]).unwrap();
    let op = f.commit(&draft, "stale");
    assert_eq!(f.wait(&op)["error"]["code"], "stale_file");
    assert_eq!(f.saved(), "");
}
#[test]
fn field_restricted_grant_does_not_allow_other_tags_or_artwork() {
    let f = Fixture::new();
    let p = f
        .service
        .pair_client(
            "Title only".into(),
            Grant {
                roots: vec![f.media.clone()],
                fields: ["title".into()].into(),
                ..Grant::default()
            },
        )
        .unwrap();
    let principal = f.service.authenticate(&p.credential).unwrap();
    assert_eq!(
        f.service
            .call(
                &principal,
                "draft.create",
                json!({"paths":[f.paths[0]],"values":{"artist":"Outside grant"}})
            )
            .unwrap_err()
            .code,
        "permission_denied"
    );
}
#[test]
fn nonmember_file_in_approved_root_is_not_a_valid_edit() {
    let f = Fixture::new();
    let root = f.media.join("not_audio.txt");
    fs::write(&root, "not audio").unwrap();
    assert!(f
        .call(
            "draft.create",
            json!({"paths":[root],"values":{"title":"No"}})
        )
        .is_err());
}
#[test]
fn outside_root_is_refused_before_reading() {
    let f = Fixture::new();
    let outside = f.root.join("outside.wav");
    fs::copy(&f.paths[0], &outside).unwrap();
    assert_eq!(
        f.call("files.read", json!({"paths":[outside]}))
            .unwrap_err()
            .code,
        "permission_denied"
    );
}
#[cfg(unix)]
#[test]
fn symlinks_and_multiply_linked_mutation_are_refused() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let link = f.media.join("link.wav");
    symlink(&f.paths[0], &link).unwrap();
    assert_eq!(
        f.call("files.read", json!({"paths":[link]}))
            .unwrap_err()
            .code,
        "unsafe_path"
    );
    let hard = f.media.join("hard.wav");
    fs::hard_link(&f.paths[0], &hard).unwrap();
    let draft = f.draft("No hardlink mutation");
    assert_eq!(
        f.call(
            "plan.commit",
            json!({"draft":draft["draft"],"digest":draft["digest"],"idempotency_key":"hardlink"})
        )
        .unwrap_err()
        .code,
        "unsafe_path"
    );
}
#[test]
fn direct_write_preserves_manual_desired_values() {
    let f = Fixture::new();
    let staged = {
        let app = f.service.0.app.lock().unwrap();
        app.as_ref()
            .unwrap()
            .preview_tag_edits(&[tagrex_commands::TagEditDto {
                path: f.paths[0].to_string_lossy().into_owned(),
                field: "title".into(),
                value: Some("Human desired".into()),
            }])
            .unwrap()
    };
    f.service.set_manual_plan(Some(staged)).unwrap();
    let op=f.call("files.write",json!({"paths":[f.paths[0]],"values":{"title":"Agent saved"},"idempotency_key":"manual-preserved"})).unwrap();
    assert_eq!(f.wait(&op)["status"], "succeeded");
    let manual = f.service.manual_plan().unwrap().unwrap();
    assert_eq!(
        manual.changes[0].tag_changes[0].new.as_deref(),
        Some("Human desired")
    );
    assert_eq!(
        manual.changes[0].tag_changes[0].old.as_deref(),
        Some("Agent saved")
    );
    assert_eq!(f.saved(), "Agent saved");
}
#[test]
fn owned_undo_and_redo_roundtrip_and_human_interference_is_detected() {
    let f = Fixture::new();
    let draft = f.draft("Roundtrip");
    let op = f.commit(&draft, "roundtrip");
    assert_eq!(f.wait(&op)["status"], "succeeded");
    f.call("history.undo", json!({"operation":op["operation"]}))
        .unwrap();
    assert_eq!(f.saved(), "");
    let history = f.call("history.list", json!({})).unwrap();
    f.call(
        "history.redo",
        json!({"operation":op["operation"],"redo_revision":history["redo_revision"]}),
    )
    .unwrap();
    assert_eq!(f.saved(), "Roundtrip");
    f.call("history.undo", json!({"operation":op["operation"]}))
        .unwrap();
    let history = f.call("history.list", json!({})).unwrap();
    {
        let mut app = f.service.0.app.lock().unwrap();
        let app = app.as_mut().unwrap();
        let plan = app
            .preview_tag_edits(&[tagrex_commands::TagEditDto {
                path: f.paths[1].to_string_lossy().into_owned(),
                field: "title".into(),
                value: Some("Human batch".into()),
            }])
            .unwrap();
        let batch = app.apply(&plan).unwrap();
        app.undo(batch.id).unwrap();
    }
    assert_eq!(
        f.call(
            "history.redo",
            json!({"operation":op["operation"],"redo_revision":history["redo_revision"]})
        )
        .unwrap_err()
        .code,
        "conflict"
    );
}
#[test]
fn pending_receipt_survives_restart_as_unknown_outcome_without_replay() {
    let f = Fixture::new();
    let record = OperationRecord {
        id: "interrupted-test".into(),
        owner: f.principal.id.clone(),
        key_hash: "key".into(),
        digest: "digest".into(),
        plan_id: "old-draft".into(),
        status: "writing".into(),
        processed: 1,
        total: 2,
        result: None,
        error: None,
        interrupted_phase: None,
        library_root: Some(f.media.clone()),
        accepted_digest: None,
        scope_digest: None,
        scope_id: None,
    };
    write_json(&f.root.join("service/operations.json"), &vec![record]).unwrap();
    let other = f.root.join("recovery-service");
    private_storage::ensure_private_dir(&other).unwrap();
    private_storage::write_atomic(
        &other.join("operations.json"),
        &fs::read(f.root.join("service/operations.json")).unwrap(),
    )
    .unwrap();
    let reopened = Service::new(f.service.0.app.clone(), other).unwrap();
    assert_eq!(
        reopened.operations().unwrap()["operations"][0]["status"],
        "recovery_required"
    );
    assert!(reopened.0.state.lock().unwrap().recovery_required);
    assert_eq!(f.saved(), "");
}
#[test]
fn malformed_image_is_refused_without_decoder_panic() {
    let f = Fixture::new();
    assert!(f
        .call("artwork.upload", json!({"data_base64":"aW52YWxpZA=="}))
        .is_err());
}

#[derive(Default)]
struct MemorySettings(Mutex<Value>);
impl SettingsBackend for MemorySettings {
    fn read(&self) -> ServiceResult<Value> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn compare_and_write(&self, expected: &str, value: Value) -> ServiceResult<()> {
        let mut saved = self.0.lock().unwrap();
        if settings_revision(&saved) != expected {
            return Err(ServiceError::conflict("Settings changed"));
        }
        *saved = value;
        Ok(())
    }
}
#[test]
fn presets_are_validated_revisioned_and_retain_unrelated_settings() {
    let f = Fixture::new();
    let settings = Arc::new(MemorySettings(Mutex::new(
        json!({"mask_presets":[],"action_groups":[],"query_presets":[],"cover_quality":90,"proxy":"https://secret:password@example.com"}),
    )));
    f.service.set_settings_backend(settings.clone()).unwrap();
    let loaded = f.call("settings.get", json!({})).unwrap();
    assert!(loaded["settings"].get("proxy").is_none());
    let created=f.call("presets.put",json!({"kind":"mask","name":"Numbered","revision":loaded["revision"],"preset":{"mask":"%track%. %artist% - %title%"}})).unwrap();
    assert_ne!(created["revision"], loaded["revision"]);
    assert_eq!(settings.read().unwrap()["cover_quality"], 90);
    assert!(settings.read().unwrap()["proxy"]
        .as_str()
        .unwrap()
        .contains("secret"));
    assert_eq!(f.call("presets.put",json!({"kind":"mask","name":"Stale","revision":loaded["revision"],"preset":{"mask":"%title%"}})).unwrap_err().code,"conflict");
}
#[test]
fn filter_presets_reject_obsolete_regex_and_case_flags() {
    let f = Fixture::new();
    let value = json!({"query_presets":[]});
    f.service
        .set_settings_backend(Arc::new(MemorySettings(Mutex::new(value.clone()))))
        .unwrap();
    let result=f.call("presets.put",json!({"kind":"filter","name":"Bad","revision":settings_revision(&value),"preset":{"query":{"version":2,"regex":true,"root":{"kind":"group","id":"root","mode":"all","children":[]}}}}));
    assert!(result.is_err());
}
#[test]
fn transform_and_draft_composition_keep_server_baselines() {
    let f = Fixture::new();
    let draft = f.draft("Artist Feat. Guest");
    let transformed=f.call("draft.transform",json!({"paths":[f.paths[0]],"draft":draft["draft"],"scope":"title","rules":[{"kind":"replace","from":"Feat.","to":"feat."}]})).unwrap();
    assert_eq!(
        transformed["changes"][0]["tags"][0]["new"],
        "Artist feat. Guest"
    );
    assert_eq!(transformed["changes"][0]["tags"][0]["old"], Value::Null);
}
#[test]
fn agent_cannot_supply_raw_plan_or_trusted_old_snapshots() {
    let f = Fixture::new();
    assert!(f.call("apply", json!({"plan":{"changes":[]}})).is_err());
    let draft = f
        .call(
            "draft.create",
            json!({"paths":[f.paths[0]],"values":{"title":"Safe"},"plan":{"changes":[]}}),
        )
        .unwrap();
    assert_eq!(draft["changes"][0]["tags"][0]["old"], Value::Null);
}

#[test]
fn a_profile_lease_survives_disabled_mcp_and_prevents_twin_service() {
    let f = Fixture::new();
    f.service.set_enabled(false).unwrap();
    let result = Service::new(f.service.0.app.clone(), f.root.join("service"));
    assert!(matches!(result,Err(ServiceError{code,..}) if code=="profile_busy"));
}

#[test]
fn exact_file_grants_allow_only_named_metadata_files() {
    let f = Fixture::new();
    let pairing = f
        .service
        .pair_client(
            "Two files only".into(),
            Grant {
                roots: vec![f.media.clone()],
                write_files: [f.paths[0].clone(), f.paths[1].clone()].into(),
                actions: ["read", "stage", "write"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                ..Grant::default()
            },
        )
        .unwrap();
    let principal = f.service.authenticate(&pairing.credential).unwrap();
    let third = f
        .service
        .call(
            &principal,
            "draft.create",
            json!({"paths":[f.paths[2]],"values":{"title":"Refused"}}),
        )
        .unwrap();
    let error = f
        .service
        .call(
            &principal,
            "plan.commit",
            json!({"draft":third["draft"],"digest":third["digest"],"idempotency_key":"third-file"}),
        )
        .unwrap_err();
    assert_eq!(error.code, "permission_denied");
    let allowed=f.service.call(&principal,"files.write",json!({"paths":[f.paths[0]],"values":{"title":"Allowed"},"idempotency_key":"named-file"})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = f
            .service
            .call(
                &principal,
                "operations.status",
                json!({"operation":allowed["operation"]}),
            )
            .unwrap();
        if status["operations"][0]["status"] == "succeeded" {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(f.saved(), "Allowed");
}
#[test]
fn query_handles_are_private_and_do_not_cancel_desktop_or_other_clients() {
    let f = Fixture::new();
    let first = f.call("files.snapshot", json!({})).unwrap();
    let query = json!({"version":2,"text":"Track 1","root":{"kind":"group","id":"root","mode":"all","children":[]}});
    let result = f
        .call(
            "files.query",
            json!({"snapshot":first["snapshot"],"query":query}),
        )
        .unwrap();
    assert_eq!(result["matched"], 1);
    let pairing = f
        .service
        .pair_client(
            "Independent search".into(),
            Grant {
                roots: vec![f.media.clone()],
                ..Grant::default()
            },
        )
        .unwrap();
    let other = f.service.authenticate(&pairing.credential).unwrap();
    let second = f.service.call(&other, "files.snapshot", json!({})).unwrap();
    let result2=f.service.call(&other,"files.query",json!({"snapshot":second["snapshot"],"query":{"version":2,"text":"Track 2","root":{"kind":"group","id":"root","mode":"all","children":[]}}})).unwrap();
    assert_eq!(result2["matched"], 1);
    let page = f
        .call(
            "files.selection_page",
            json!({"selection":result["selection"],"limit":1}),
        )
        .unwrap();
    assert!(page["rows"][0]["path"]
        .as_str()
        .unwrap()
        .ends_with("Track 1.wav"));
    assert!(f
        .service
        .call(
            &other,
            "files.selection_page",
            json!({"selection":result["selection"]})
        )
        .is_err());
}
#[test]
fn oversized_single_tag_requires_explicit_chunks_instead_of_unbounded_rows() {
    let f = Fixture::new();
    let text = "é".repeat(600_000);
    {
        let mut app = f.service.0.app.lock().unwrap();
        let app = app.as_mut().unwrap();
        let plan = app
            .preview_tag_edits(&[tagrex_commands::TagEditDto {
                path: f.paths[0].to_string_lossy().into_owned(),
                field: "comment".into(),
                value: Some(text),
            }])
            .unwrap();
        app.apply(&plan).unwrap();
    }
    assert_eq!(
        f.call("files.read", json!({"paths":[f.paths[0]]}))
            .unwrap_err()
            .code,
        "response_too_large"
    );
    let chunk = f
        .call(
            "files.field",
            json!({"paths":[f.paths[0]],"field":"comment","limit":10}),
        )
        .unwrap();
    assert_eq!(chunk["value"], "é".repeat(10));
    assert_eq!(chunk["total"], 600_000);
    assert_eq!(chunk["next_offset"], 10);
}
#[test]
fn artwork_is_validated_pooled_and_committed_through_native_undo() {
    use base64::Engine;
    let f = Fixture::new();
    let image = image::DynamicImage::new_rgb8(8, 8);
    let mut cursor = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap();
    let asset=f.call("artwork.upload",json!({"data_base64":base64::engine::general_purpose::STANDARD.encode(cursor.into_inner())})).unwrap();
    let info = f
        .call("artwork.inspect", json!({"asset":asset["asset"]}))
        .unwrap();
    assert_eq!(info["width"], 8);
    assert_eq!(info["height"], 8);
    let draft = f
        .call(
            "draft.artwork",
            json!({"paths":[f.paths[0]],"asset":asset["asset"]}),
        )
        .unwrap();
    let encoded = serde_json::to_string(&draft).unwrap();
    assert!(!encoded.contains("data_base64"));
    let operation = f.commit(&draft, "cover");
    assert_eq!(f.wait(&operation)["status"], "succeeded");
    assert!(tagrex_core::model::TagEngine::read_cover(&f.paths[0])
        .unwrap()
        .is_some());
    f.call("history.undo", json!({"operation":operation["operation"]}))
        .unwrap();
    assert!(tagrex_core::model::TagEngine::read_cover(&f.paths[0])
        .unwrap()
        .is_none());
}

#[test]
fn typed_partial_rollback_cannot_be_misreported_as_safe_failure() {
    let error = tagrex_commands::AppError::Plan(tagrex_core::plan::PlanError::RollbackIncomplete {
        cause: Box::new(tagrex_core::plan::PlanError::Io(std::io::Error::other(
            "simulated write failure",
        ))),
        count: 1,
        first: PathBuf::from("fixture.wav"),
    });
    assert_eq!(ServiceError::from(error).code, "recovery_required");
    assert_eq!(
        ServiceError::from(tagrex_commands::AppError::Plan(
            tagrex_core::plan::PlanError::Cancelled
        ))
        .code,
        "cancelled_and_restored"
    );
}

#[test]
fn native_write_scope_intersects_roots_and_restores_after_error_and_panic() {
    let f = Fixture::new();
    let narrow = f.media.join("approved");
    fs::create_dir_all(&narrow).unwrap();
    let inside = narrow.join("inside.wav");
    fs::copy(&f.paths[0], &inside).unwrap();
    let mut guard = f.service.0.app.lock().unwrap();
    let app = guard.as_mut().unwrap();
    let outside_plan = app
        .preview_tag_edits(&[tagrex_commands::TagEditDto {
            path: f.paths[0].to_string_lossy().into_owned(),
            field: "title".into(),
            value: Some("Outside narrow root".into()),
        }])
        .unwrap();
    let before = fs::read(&f.paths[0]).unwrap();
    let error = app
        .with_write_scope(std::slice::from_ref(&narrow), |app| {
            app.apply(&outside_plan)
        })
        .unwrap_err();
    assert!(matches!(
        error,
        tagrex_commands::AppError::Plan(tagrex_core::plan::PlanError::OutsideRoot(_))
    ));
    assert_eq!(fs::read(&f.paths[0]).unwrap(), before);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), _> = app.with_write_scope(std::slice::from_ref(&narrow), |_| {
            panic!("scope unwind fixture")
        });
    }));
    assert!(caught.is_err());
    // A scoped error/panic cannot permanently narrow the person's session.
    app.apply(&outside_plan).unwrap();
    assert_eq!(
        app.read_track(&f.paths[0]).unwrap().tags["title"],
        "Outside narrow root"
    );
    let disjoint = f.root.join("disjoint");
    fs::create_dir_all(&disjoint).unwrap();
    let result: Result<(), _> = app.with_write_scope(&[disjoint], |_| Ok(()));
    assert!(result.is_err());
}
#[test]
fn projected_pages_never_serialize_unrequested_large_tag_values() {
    let f = Fixture::new();
    let mut row:TrackDto=serde_json::from_value(json!({"path":f.paths[0],"format":"WAV","tags":{"title":"Small title","lyrics":"x".repeat(2*1024*1024)}})).unwrap();
    let projected = project_track(&row, Some(&["title".into()]));
    assert_eq!(projected["tags"]["title"], "Small title");
    assert!(projected["tags"].get("lyrics").is_none());
    assert!(serde_json::to_vec(&projected).unwrap().len() < 1024);
    row.tags.clear();
}

#[test]
fn accepted_worker_panics_require_recovery_and_never_report_restored() {
    let f = Fixture::new();
    f.service.0.panic_after_apply.store(true, Ordering::Release);
    let draft = f.draft("Saved before reply panic");
    let operation = f.commit(&draft, "panic");
    let status = f.wait(&operation);
    assert_eq!(status["status"], "recovery_required");
    assert_eq!(status["error"]["code"], "worker_panicked");
    assert!(f.service.0.state.lock().unwrap().recovery_required);
    let persisted: Vec<OperationRecord> = read_json(&f.root.join("service/operations.json"))
        .unwrap()
        .unwrap();
    assert_eq!(persisted[0].status, "recovery_required");
    // No automatic duplicate write/retry can occur after the unknown outcome.
    let result = f.call(
        "plan.commit",
        json!({"draft":draft["draft"],"digest":draft["digest"],"idempotency_key":"another"}),
    );
    assert_eq!(result.unwrap_err().code, "recovery_required");
}
#[test]
fn accepted_spawn_failure_is_durable_terminal_and_does_not_hold_queue_capacity() {
    let f = Fixture::new();
    let draft = f.draft("Not written");
    f.service.0.fail_next_spawn.store(true, Ordering::Release);
    let operation = f.commit(&draft, "spawn-refused");
    assert_eq!(operation["status"], "failed_before_write");
    let status = f.wait(&operation);
    assert_eq!(status["status"], "failed_before_write");
    assert_eq!(f.saved(), "");
    let retry = f.commit(&draft, "spawn-refused");
    assert_eq!(retry["operation"], operation["operation"]);
    assert_eq!(retry["status"], "failed_before_write");
    let next = f.draft("Next works");
    let operation = f.commit(&next, "different-intent");
    assert_eq!(f.wait(&operation)["status"], "succeeded");
    assert_eq!(f.saved(), "Next works");
}

#[test]
fn an_observer_panic_cannot_poison_the_write_engine_or_drop_the_receipt() {
    let f = Fixture::new();
    f.service
        .set_event_sink(Arc::new(|event, _| {
            if event == "write_completed" {
                panic!("injected observer failure");
            }
        }))
        .unwrap();
    let draft = f.draft("Observer safe");
    let operation = f.commit(&draft, "observer-failure");
    assert_eq!(f.wait(&operation)["status"], "succeeded");
    assert_eq!(f.saved(), "Observer safe");
}

#[test]
fn a_new_human_plan_published_mid_write_is_not_replaced_by_an_old_rebase() {
    use std::sync::mpsc;
    let f = Fixture::new();
    let (old, new) = {
        let guard = f.service.0.app.lock().unwrap();
        let app = guard.as_ref().unwrap();
        let make = |value: &str| {
            app.preview_tag_edits(&[tagrex_commands::TagEditDto {
                path: f.paths[0].to_string_lossy().into_owned(),
                field: "title".into(),
                value: Some(value.into()),
            }])
            .unwrap()
        };
        (make("Human before"), make("Human changed during write"))
    };
    let captured = f.service.set_manual_plan(Some(old)).unwrap();
    let (started, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let wait = Mutex::new(wait);
    let observed = Arc::new(Mutex::new(None));
    let output = observed.clone();
    let blocked = AtomicBool::new(false);
    f.service
        .set_event_sink(Arc::new(move |event, detail| {
            if event == "operation_changed"
                && detail
                    .get("processed")
                    .and_then(Value::as_u64)
                    .is_some_and(|done| done > 0)
                && !blocked.swap(true, Ordering::AcqRel)
            {
                started.send(()).unwrap();
                wait.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
            if event == "write_completed" {
                *output.lock().unwrap() = Some(detail);
            }
        }))
        .unwrap();
    let operation=f.call("files.write",json!({"paths":[f.paths[0]],"values":{"title":"Agent saved"},"idempotency_key":"manual-concurrent"})).unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let newer = f.service.set_manual_plan(Some(new)).unwrap();
    assert!(newer > captured);
    release.send(()).unwrap();
    assert_eq!(f.wait(&operation)["status"], "succeeded");
    assert_eq!(
        f.service.manual_plan().unwrap().unwrap().changes[0].tag_changes[0]
            .new
            .as_deref(),
        Some("Human changed during write")
    );
    let detail = observed.lock().unwrap().clone().unwrap();
    assert_eq!(detail["manual_revision"], captured);
    assert_eq!(detail["manual_rebased"], false);
    assert_eq!(detail["manual_changed"], true);
    assert!(detail.get("plan").is_none());
    assert_eq!(f.saved(), "Agent saved");
}

#[test]
fn multiple_own_undo_entries_replay_in_lifo_order_with_stable_entry_ids() {
    let f = Fixture::new();
    let first = f.commit(&f.draft("First"), "redo-first");
    assert_eq!(f.wait(&first)["status"], "succeeded");
    let second = f.commit(&f.draft("Second"), "redo-second");
    assert_eq!(f.wait(&second)["status"], "succeeded");
    f.call("history.undo", json!({"operation":second["operation"]}))
        .unwrap();
    f.call("history.undo", json!({"operation":first["operation"]}))
        .unwrap();
    assert_eq!(f.saved(), "");
    let state = f.call("history.list", json!({})).unwrap();
    f.call(
        "history.redo",
        json!({"operation":first["operation"],"redo_revision":state["redo_revision"]}),
    )
    .unwrap();
    assert_eq!(f.saved(), "First");
    let state = f.call("history.list", json!({})).unwrap();
    f.call(
        "history.redo",
        json!({"operation":second["operation"],"redo_revision":state["redo_revision"]}),
    )
    .unwrap();
    assert_eq!(f.saved(), "Second");
}
fn mark_uncertain(f: &Fixture, operation: &Value, phase: &str) {
    let mut state = f.service.0.state.lock().unwrap();
    let op = state
        .operations
        .get(operation["operation"].as_str().unwrap())
        .unwrap();
    {
        let mut record = op.record.lock().unwrap();
        record.status = "recovery_required".into();
        record.interrupted_phase = Some(phase.into());
    }
    state.recovery_required = true;
    super::plans::persist_recovery_operations(&f.service.0.directory, &state).unwrap();
}
#[test]
fn completed_journal_proof_reconciles_without_replaying_a_write() {
    let f = Fixture::new();
    let op = f.commit(&f.draft("Durably complete"), "reconcile-complete");
    assert_eq!(f.wait(&op)["status"], "succeeded");
    let before = fs::read(&f.paths[0]).unwrap();
    mark_uncertain(&f, &op, "writing");
    let report = f.service.recovery_report().unwrap();
    let entry = &report["operations"][0];
    assert_eq!(entry["action"], "accept_completed");
    f.service
        .reconcile_operation(
            op["operation"].as_str().unwrap(),
            entry["proof"].as_str().unwrap(),
            "accept_completed",
        )
        .unwrap();
    assert_eq!(fs::read(&f.paths[0]).unwrap(), before);
    assert!(!f.service.recovery_report().unwrap()["recovery_required"]
        .as_bool()
        .unwrap());
    assert_eq!(f.wait(&op)["status"], "succeeded_recovered");
}
#[test]
fn a_missing_journal_after_writing_cannot_clear_recovery() {
    let f = Fixture::new();
    let op = f.commit(&f.draft("Undo then uncertain"), "missing-proof");
    f.wait(&op);
    f.call("history.undo", json!({"operation":op["operation"]}))
        .unwrap();
    mark_uncertain(&f, &op, "writing");
    let report = f.service.recovery_report().unwrap();
    let entry = &report["operations"][0];
    assert_eq!(entry["action"], Value::Null);
    assert!(f
        .service
        .reconcile_operation(
            op["operation"].as_str().unwrap(),
            entry["proof"].as_str().unwrap(),
            "accept_completed"
        )
        .is_err());
    assert!(f.service.0.state.lock().unwrap().recovery_required);
}
#[test]
fn interrupted_correlated_batch_uses_strict_native_undo_before_unfreezing() {
    use tagrex_core::journal::{BatchId, SqliteJournal, UndoJournal};
    let f = Fixture::new();
    let op = f.commit(&f.draft("Interrupted proof"), "interrupted-proof");
    let receipt = f.wait(&op);
    let batch_id = receipt["result"]["batch"]["id"].as_i64().unwrap();
    {
        let mut journal = SqliteJournal::open(&f.root.join("journal.sqlite")).unwrap();
        let batch = journal.batch(BatchId(batch_id)).unwrap().unwrap();
        journal
            .retain_interrupted(BatchId(batch_id), &batch)
            .unwrap();
    }
    mark_uncertain(&f, &op, "writing");
    let report = f.service.recovery_report().unwrap();
    let entry = &report["operations"][0];
    assert_eq!(entry["action"], "undo_correlated");
    f.service
        .reconcile_operation(
            op["operation"].as_str().unwrap(),
            entry["proof"].as_str().unwrap(),
            "undo_correlated",
        )
        .unwrap();
    assert_eq!(f.saved(), "");
    assert_eq!(f.wait(&op)["status"], "restored_recovered");
    assert!(!f.service.0.state.lock().unwrap().recovery_required);
}
#[test]
fn native_history_remains_guarded_and_usable_after_private_plan_handles_expire() {
    let f = Fixture::new();
    let op = f.commit(&f.draft("Old receipt"), "expired-draft-history");
    f.wait(&op);
    f.service.0.state.lock().unwrap().plans.clear();
    f.call("history.undo", json!({"operation":op["operation"]}))
        .unwrap();
    assert_eq!(f.saved(), "");
    let history = f.call("history.list", json!({})).unwrap();
    f.call(
        "history.redo",
        json!({"operation":op["operation"],"redo_revision":history["redo_revision"]}),
    )
    .unwrap();
    assert_eq!(f.saved(), "Old receipt");
}

#[test]
fn post_apply_panic_reconciles_only_after_fresh_session_and_completed_journal_proof() {
    let mut f = Fixture::new();
    f.service.0.panic_after_apply.store(true, Ordering::Release);
    let operation = f.commit(&f.draft("Applied before crash"), "recover-after-panic");
    assert_eq!(f.wait(&operation)["status"], "recovery_required");
    assert_eq!(
        f.service.recovery_report().unwrap_err().code,
        "restart_required"
    );
    let parked = Service::new(Arc::new(Mutex::new(None)), f.root.join("parking-service")).unwrap();
    drop(std::mem::replace(&mut f.service, parked));
    let shared = Arc::new(Mutex::new(None));
    let deadline = Instant::now() + Duration::from_secs(3);
    let reopened = loop {
        match Service::new(shared.clone(), f.root.join("service")) {
            Ok(service) => break service,
            Err(error) if error.code == "profile_busy" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("Could not reopen fixture profile: {error}"),
        }
    };
    *shared.lock().unwrap() = Some(App::open(&f.media, &f.root.join("journal.sqlite")).unwrap());
    f.service = reopened;
    f.principal = f.service.authenticate(&f.credential).unwrap();
    let report = f.service.recovery_report().unwrap();
    let evidence = &report["operations"][0];
    assert_eq!(evidence["action"], "accept_completed", "{report}");
    f.service
        .reconcile_operation(
            operation["operation"].as_str().unwrap(),
            evidence["proof"].as_str().unwrap(),
            "accept_completed",
        )
        .unwrap();
    assert_eq!(f.saved(), "Applied before crash");
    assert_eq!(f.wait(&operation)["status"], "succeeded_recovered");
}
#[test]
fn durable_queued_phase_fence_and_no_native_batch_prove_no_write() {
    let f = Fixture::new();
    let id = "queued-before-native".to_string();
    let operation = Arc::new(Operation {
        record: Mutex::new(OperationRecord {
            id: id.clone(),
            owner: f.principal.id.clone(),
            key_hash: "queued-test".into(),
            digest: "digest".into(),
            plan_id: "draft_never_entered_executor".into(),
            status: "recovery_required".into(),
            processed: 0,
            total: 1,
            result: None,
            error: None,
            interrupted_phase: Some("queued".into()),
            library_root: Some(fs::canonicalize(&f.media).unwrap()),
            accepted_digest: None,
            scope_digest: None,
            scope_id: None,
        }),
        cancel: AtomicBool::new(false),
    });
    {
        let mut state = f.service.0.state.lock().unwrap();
        state.operations.insert(id.clone(), operation);
        state.recovery_required = true;
    }
    let before = fs::read(&f.paths[0]).unwrap();
    let report = f.service.recovery_report().unwrap();
    let evidence = &report["operations"][0];
    assert_eq!(evidence["action"], "close_not_written");
    f.service
        .reconcile_operation(
            &id,
            evidence["proof"].as_str().unwrap(),
            "close_not_written",
        )
        .unwrap();
    assert_eq!(fs::read(&f.paths[0]).unwrap(), before);
    assert!(!f.service.0.state.lock().unwrap().recovery_required);
}

#[test]
fn a_replaced_damaged_journal_cannot_clear_recovery_even_after_its_notice_is_consumed() {
    let f = Fixture::new();
    let operation = f.commit(&f.draft("Journal continuity"), "damaged-journal");
    f.wait(&operation);
    mark_uncertain(&f, &operation, "writing");
    {
        let mut guard = f.service.0.app.lock().unwrap();
        *guard = None;
        fs::write(f.root.join("journal.sqlite"), b"damaged sqlite fixture").unwrap();
        let app = App::open(&f.media, &f.root.join("journal.sqlite")).unwrap();
        assert!(app.take_journal_notice().is_some());
        *guard = Some(app);
    }
    let report = f.service.recovery_report().unwrap();
    let evidence = &report["operations"][0];
    assert_eq!(evidence["action"], Value::Null);
    assert!(evidence["guidance"].as_str().unwrap().contains("damage"));
    assert!(f.service.0.state.lock().unwrap().recovery_required);
}
#[test]
fn native_journal_changes_invalidate_a_previously_displayed_recovery_proof() {
    let f = Fixture::new();
    let operation = f.commit(&f.draft("Stale proof"), "stale-recovery-proof");
    let status = f.wait(&operation);
    mark_uncertain(&f, &operation, "writing");
    let report = f.service.recovery_report().unwrap();
    let proof = report["operations"][0]["proof"].as_str().unwrap();
    assert_eq!(
        f.call("history.undo", json!({"operation":operation["operation"]}))
            .unwrap_err()
            .code,
        "recovery_required"
    );
    {
        let mut app = f.service.0.app.lock().unwrap();
        app.as_mut()
            .unwrap()
            .undo(status["result"]["batch"]["id"].as_i64().unwrap())
            .unwrap();
    }
    assert_eq!(
        f.service
            .reconcile_operation(
                operation["operation"].as_str().unwrap(),
                proof,
                "accept_completed"
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(f.service.0.state.lock().unwrap().recovery_required);
}

// Canonical collaboration: these exercise the real native preview/write engine
// against disposable WAV files, never user media.
fn human_fields(
    f: &Fixture,
    entries: &[(usize, &str, &str)],
    touched: &[(usize, &str)],
) -> SharedStageSnapshot {
    let plan = {
        let guard = f.service.0.app.lock().unwrap();
        guard
            .as_ref()
            .unwrap()
            .preview_tag_edits(
                &entries
                    .iter()
                    .map(|(i, field, value)| tagrex_commands::TagEditDto {
                        path: f.paths[*i].to_string_lossy().into_owned(),
                        field: (*field).into(),
                        value: Some((*value).into()),
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap()
    };
    let revision = f.service.shared_stage().unwrap().revision;
    f.service
        .publish_manual_plan_with_intents(
            Some(plan),
            revision,
            touched
                .iter()
                .map(|(i, field)| StageFieldRef {
                    path: f.paths[*i].to_string_lossy().into_owned(),
                    field: (*field).into(),
                })
                .collect(),
        )
        .unwrap()
}
fn staged_value(f: &Fixture, i: usize, field: &str) -> Option<String> {
    f.service
        .shared_stage()
        .unwrap()
        .plan
        .as_ref()
        .and_then(|plan| {
            plan.changes
                .iter()
                .find(|change| change.path == f.paths[i].to_string_lossy())
                .and_then(|change| change.tag_changes.iter().find(|value| value.field == field))
                .and_then(|field| field.new.clone())
        })
}
#[test]
fn canonical_stage_human_six_and_agent_two_are_visible_then_scoped_write_leaves_six() {
    let mut f = Fixture::new();
    for i in 3..8 {
        let path = f.media.join(format!("Track {i}.wav"));
        fs::copy(&f.paths[0], &path).unwrap();
        f.paths.push(path);
    }
    *f.service.0.app.lock().unwrap() =
        Some(App::open(&f.media, &f.root.join("journal.sqlite")).unwrap());
    let originals = f
        .paths
        .iter()
        .map(fs::read)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let human = (0..6).map(|i| (i, "title", "Human")).collect::<Vec<_>>();
    human_fields(&f, &human, &[]);
    let agent = f
        .call(
            "draft.create",
            json!({"paths":[f.paths[6],f.paths[7]],"values":{"artist":"Agent"}}),
        )
        .unwrap();
    assert_eq!(
        f.service
            .shared_stage()
            .unwrap()
            .plan
            .unwrap()
            .changes
            .len(),
        8
    );
    for (i, path) in f.paths.iter().enumerate() {
        assert_eq!(fs::read(path).unwrap(), originals[i]);
    }
    let operation = f.commit(&agent, "two-only");
    assert_eq!(f.wait(&operation)["status"], "succeeded");
    assert_eq!(
        f.service
            .shared_stage()
            .unwrap()
            .plan
            .unwrap()
            .changes
            .len(),
        6
    );
    assert!(f
        .service
        .0
        .app
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .read_track(&f.paths[6])
        .unwrap()
        .tags
        .get("artist")
        .is_some_and(|value| value == "Agent"));
    for (i, path) in f.paths[..6].iter().enumerate() {
        assert_eq!(fs::read(path).unwrap(), originals[i]);
    }
}
#[test]
fn canonical_fields_are_human_protected_in_both_edit_orders_and_other_fields_remain_editable() {
    let f = Fixture::new();
    let first = f.draft("Agent A");
    human_fields(&f, &[(0, "title", "Human B")], &[(0, "title")]);
    let next=f.call("draft.create",json!({"paths":[f.paths[0]],"draft":first["draft"],"values":{"title":"Agent C","artist":"Agent artist","album":"Agent album"}})).unwrap();
    assert_eq!(staged_value(&f, 0, "title").as_deref(), Some("Human B"));
    assert_eq!(
        staged_value(&f, 0, "artist").as_deref(),
        Some("Agent artist")
    );
    assert!(!next["protected_fields"].as_array().unwrap().is_empty());
    human_fields(&f, &[(1, "title", "Human first")], &[(1, "title")]);
    f.call(
        "draft.create",
        json!({"paths":[f.paths[1]],"values":{"title":"Agent later","artist":"Allowed"}}),
    )
    .unwrap();
    assert_eq!(staged_value(&f, 1, "title").as_deref(), Some("Human first"));
    assert_eq!(staged_value(&f, 1, "artist").as_deref(), Some("Allowed"));
}
#[test]
fn live_scope_applies_current_human_value_and_retry_does_not_replay_later_edits() {
    let f = Fixture::new();
    let draft = f.draft("Agent A");
    human_fields(
        &f,
        &[(0, "title", "Human B"), (0, "comment", "Outside scope")],
        &[(0, "title"), (0, "comment")],
    );
    let get = f
        .call("draft.get", json!({"draft":draft["draft"]}))
        .unwrap();
    assert_eq!(get["scope_digest"], draft["digest"]);
    let operation = f.commit(&draft, "current-value");
    let terminal = f.wait(&operation);
    assert_eq!(terminal["status"], "succeeded");
    assert!(terminal["accepted_digest"].is_string());
    assert_eq!(f.saved(), "Human B");
    assert_eq!(
        staged_value(&f, 0, "comment").as_deref(),
        Some("Outside scope")
    );
    human_fields(&f, &[(0, "title", "Human C")], &[(0, "title")]);
    let retry = f.commit(&draft, "current-value");
    assert_eq!(retry["operation"], operation["operation"]);
    assert_eq!(f.saved(), "Human B");
    let next = f.commit(&draft, "current-value-new-key");
    assert_eq!(f.wait(&next)["status"], "succeeded");
    assert_eq!(f.saved(), "Human C");
}
#[test]
fn human_saved_equal_reversion_and_removal_are_not_resurrected_by_live_handle() {
    let f = Fixture::new();
    let draft = f.draft("Agent A");
    human_fields(&f, &[(0, "title", "")], &[(0, "title")]);
    assert_eq!(staged_value(&f, 0, "title"), None);
    let extended = f
        .call(
            "draft.create",
            json!({"paths":[f.paths[0]],"draft":draft["draft"],"values":{"album":"New album"}}),
        )
        .unwrap();
    assert_eq!(staged_value(&f, 0, "title"), None);
    let operation = f.commit(&extended, "only-current-album");
    assert_eq!(f.wait(&operation)["status"], "succeeded");
    assert_eq!(f.saved(), "");
}
#[test]
fn disjoint_clients_share_stage_conflicting_agents_are_explicit_and_disconnect_keeps_work() {
    let f = Fixture::new();
    let grant = f.service.grant(&f.principal, "stage").unwrap();
    let pair = f.service.pair_client("Second".into(), grant).unwrap();
    let second = f.service.authenticate(&pair.credential).unwrap();
    let first = f.draft("A");
    f.service
        .call(
            &second,
            "draft.create",
            json!({"paths":[f.paths[0]],"values":{"artist":"Second"}}),
        )
        .unwrap();
    assert_eq!(
        f.service
            .call(
                &second,
                "draft.create",
                json!({"paths":[f.paths[0]],"values":{"title":"Conflicting"}})
            )
            .unwrap_err()
            .code,
        "stage_field_conflict"
    );
    f.call("draft.discard", json!({"draft":first["draft"]}))
        .unwrap();
    assert_eq!(staged_value(&f, 0, "title"), None);
    assert_eq!(staged_value(&f, 0, "artist").as_deref(), Some("Second"));
    f.service.revoke_client(&second.id).unwrap();
    assert_eq!(staged_value(&f, 0, "artist").as_deref(), Some("Second"));
}
#[test]
fn human_patch_preserves_unrelated_human_fields_and_stale_generation_cannot_rewind() {
    let f = Fixture::new();
    human_fields(&f, &[(0, "title", "Existing"), (1, "album", "Keep")], &[]);
    let stale = f.service.shared_stage().unwrap().revision;
    human_fields(&f, &[(0, "artist", "New")], &[(0, "artist")]);
    assert_eq!(staged_value(&f, 0, "title").as_deref(), Some("Existing"));
    assert_eq!(staged_value(&f, 1, "album").as_deref(), Some("Keep"));
    let rejected = f
        .service
        .publish_manual_plan_with_intents(
            None,
            stale,
            vec![StageFieldRef {
                path: f.paths[0].to_string_lossy().into_owned(),
                field: "title".into(),
            }],
        )
        .unwrap();
    assert!(!rejected.accepted);
    assert_eq!(staged_value(&f, 0, "title").as_deref(), Some("Existing"));
}
#[test]
fn partial_human_apply_removes_matching_fields_only_and_preserves_later_edits() {
    let f = Fixture::new();
    human_fields(&f, &[(0, "title", "Human"), (1, "title", "Excluded")], &[]);
    f.call(
        "draft.create",
        json!({"paths":[f.paths[0]],"values":{"artist":"Agent"}}),
    )
    .unwrap();
    let revision = f.service.shared_stage().unwrap().revision;
    let selected = [f.paths[0].to_string_lossy().into_owned()].into();
    let plan = f.service.shared_apply_plan(&selected).unwrap();
    {
        let mut guard = f.service.0.app.lock().unwrap();
        guard.as_mut().unwrap().apply(&plan).unwrap();
    }
    human_fields(&f, &[(0, "title", "Arrived after Apply")], &[(0, "title")]);
    f.service.complete_shared_apply(&plan, revision).unwrap();
    assert_eq!(f.saved(), "Human");
    assert_eq!(
        staged_value(&f, 0, "title").as_deref(),
        Some("Arrived after Apply")
    );
    assert_eq!(staged_value(&f, 0, "artist"), None);
    assert_eq!(staged_value(&f, 1, "title").as_deref(), Some("Excluded"));
}
#[test]
fn current_human_title_drives_agent_rename_and_own_rename_does_not_block_itself() {
    let f = Fixture::new();
    human_fields(&f, &[(0, "title", "Working title")], &[(0, "title")]);
    let draft = f
        .call(
            "draft.rename",
            json!({"paths":[f.paths[0]],"mask":"%title%"}),
        )
        .unwrap();
    let diff = f
        .call("draft.get", json!({"draft":draft["draft"]}))
        .unwrap();
    assert!(serde_json::to_string(&diff)
        .unwrap()
        .contains("Working title.wav"));
    // This scope excludes the human title, so moving would orphan it: precise refusal.
    let operation = f.commit(&draft, "rename-protect-same-file");
    assert_eq!(f.wait(&operation)["error"]["code"], "conflict");
    f.service
        .clear_shared_stage(f.service.shared_stage().unwrap().revision)
        .unwrap();
    let rename = f
        .call(
            "draft.rename",
            json!({"paths":[f.paths[1]],"mask":"Renamed"}),
        )
        .unwrap();
    let operation = f.commit(&rename, "rename-own");
    assert_eq!(f.wait(&operation)["status"], "succeeded");
    assert!(f.media.join("Renamed.wav").exists());
}
#[test]
fn old_successful_receipt_shape_remains_deserializable() {
    let old = json!({"id":"operation-old","owner":"old-client","key_hash":"key","digest":"old-value-digest","plan_id":"draft-old","status":"succeeded","processed":1,"total":1,"result":{"batch":{"id":50}},"error":null});
    let record: OperationRecord = serde_json::from_value(old).unwrap();
    assert_eq!(record.status, "succeeded");
    assert!(record.accepted_digest.is_none());
    assert!(record.scope_id.is_none());
}

#[test]
fn shared_artwork_is_pooled_and_mcp_stage_pages_never_include_image_bytes() {
    use base64::Engine;
    let f = Fixture::new();
    human_fields(&f, &[(0, "title", "Human")], &[]);
    let image = image::DynamicImage::new_rgb8(16, 16);
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    // Identical handles intentionally share one pooled input across files.
    let asset=f.call("artwork.upload",json!({"data_base64":base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())})).unwrap();
    let first = f
        .call(
            "draft.artwork",
            json!({"paths":[f.paths[1]],"asset":asset["asset"],"mode":"replace_all"}),
        )
        .unwrap();
    f.call("draft.artwork",json!({"paths":[f.paths[2]],"asset":asset["asset"],"mode":"replace_all","draft":first["draft"]})).unwrap();
    let stage = f.service.shared_stage().unwrap().plan.unwrap();
    assert_eq!(stage.changes.len(), 3);
    assert_eq!(
        stage.cover_assets.len(),
        1,
        "identical image content should be pooled once across preview calls"
    );
    let metadata = f
        .call("staging.get", json!({"offset":0,"limit":2}))
        .unwrap();
    assert_eq!(metadata["changes"].as_array().unwrap().len(), 2);
    assert_eq!(metadata["files"], 3);
    let encoded = serde_json::to_string(&metadata).unwrap();
    assert!(!encoded.contains("data_base64"));
    assert!(!encoded.contains("iVBOR"));
    assert!(!encoded.contains("draft_"));
    let state = f.service.0.state.lock().unwrap();
    for handle in state.plans.values() {
        assert!(handle.plan.changes.is_empty());
        assert!(handle.plan.cover_assets.is_empty());
    }
}
#[test]
fn large_shared_staged_projection_and_merge_are_linear_and_events_remain_small() {
    let f = Fixture::new();
    let base = {
        let guard = f.service.0.app.lock().unwrap();
        guard
            .as_ref()
            .unwrap()
            .preview_tag_edits(&[tagrex_commands::TagEditDto {
                path: f.paths[0].to_string_lossy().into_owned(),
                field: "title".into(),
                value: Some("Title".into()),
            }])
            .unwrap()
    };
    let mut large = base.clone();
    let template = &base.changes[0];
    large.changes = (0..100_000)
        .map(|i| {
            let mut change = template.clone();
            change.path = f
                .media
                .join(format!("Projected-{i}.wav"))
                .to_string_lossy()
                .into_owned();
            change
        })
        .collect();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    f.service
        .set_event_sink(Arc::new(move |name, payload| {
            if name == "shared_stage_changed" {
                captured.lock().unwrap().push(payload);
            }
        }))
        .unwrap();
    let started = Instant::now();
    let result = f.service.publish_manual_plan(Some(large), 0).unwrap();
    assert_eq!(result.plan.unwrap().changes.len(), 100_000);
    let page = f
        .call("staging.get", json!({"offset":99_990,"limit":5}))
        .unwrap();
    assert_eq!(page["files"], 100_000);
    assert_eq!(page["changes"].as_array().unwrap().len(), 5);
    assert!(serde_json::to_vec(&page).unwrap().len() < 10_000);
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .all(|event| serde_json::to_vec(event).unwrap().len() < 20_000));
    println!(
        "MCP_SHARED_STAGE_SCALE 100000 publish+page {:?}",
        started.elapsed()
    );
}
#[test]
fn successful_move_removes_its_stage_and_does_not_block_unrelated_human_file() {
    let f = Fixture::new();
    human_fields(&f, &[(0, "title", "Keep human")], &[]);
    let draft = f
        .call("draft.rename", json!({"paths":[f.paths[1]],"mask":"Moved"}))
        .unwrap();
    let op = f.commit(&draft, "own-move-disjoint");
    assert_eq!(f.wait(&op)["status"], "succeeded");
    let stage = f.service.shared_stage().unwrap().plan.unwrap();
    assert_eq!(stage.changes.len(), 1);
    assert_eq!(stage.changes[0].path, f.paths[0].to_string_lossy());
}
#[test]
fn transform_without_old_handle_uses_current_working_agent_values() {
    let f = Fixture::new();
    f.call(
        "draft.create",
        json!({"paths":[f.paths[0]],"values":{"title":"Artist Feat. Guest"}}),
    )
    .unwrap();
    f.call("draft.transform",json!({"paths":[f.paths[0]],"scope":"title","rules":[{"kind":"replace","from":"Feat.","to":"feat.","regex":false,"case_sensitive":false,"whole_word":false}]})).unwrap();
    assert_eq!(
        staged_value(&f, 0, "title").as_deref(),
        Some("Artist feat. Guest")
    );
}
