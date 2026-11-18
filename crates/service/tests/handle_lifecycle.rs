//! Public service lifecycle probes; no network or user files are accessed.
use base64::Engine;
use serde_json::{json, Value};
use std::{
    io::Cursor,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tagrex_commands::{CandidateDto, CoverArtDto, ReleaseDto, SearchQueryDto};
use tagrex_service::{Grant, Principal, ProviderBackend, Service, ServiceError, ServiceResult};

struct Gate {
    entered: mpsc::Sender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
}
impl Gate {
    fn wait(&self) -> ServiceResult<()> {
        self.entered.send(()).unwrap();
        self.resume
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| ServiceError::new("fixture_timeout", "No fixture release"))
    }
}
struct Backend {
    gate: Gate,
}
fn png() -> String {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(1, 1)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
}
impl ProviderBackend for Backend {
    fn status(&self) -> ServiceResult<Value> {
        Ok(json!({}))
    }
    fn search(&self, _: &str, _: &SearchQueryDto) -> ServiceResult<Vec<CandidateDto>> {
        Ok(Vec::new())
    }
    fn fetch_release(&self, _: &str, id: &str) -> ServiceResult<ReleaseDto> {
        if id == "wait" {
            self.gate.wait()?;
        }
        let count = id.parse::<usize>().unwrap_or(1);
        let images: Vec<_> = (0..count)
            .map(|i| {
                json!({
                    "url": format!("https://i.discogs.com/{id}-{i}.png"), "width": 1, "height": 1
                })
            })
            .collect();
        Ok(serde_json::from_value(json!({
            "id": id, "artist": "Fixture", "title": if id == "large" { "尾".repeat(400_000) } else { "Release".into() }, "year": null,
            "genres": [], "styles": [], "tracks": [], "cover_image_url": if id == "fallback" { Some("https://i.discogs.com/fallback.png") } else { None }, "images": if id == "fallback" { Vec::new() } else { images }
        }))
        .unwrap())
    }
    fn fetch_image(&self, _: &str, url: &str) -> ServiceResult<CoverArtDto> {
        if url.contains("asset-wait") {
            self.gate.wait()?;
        }
        Ok(CoverArtDto {
            mime: "image/png".into(),
            data_base64: png().into(),
            ..Default::default()
        })
    }
}
struct Fixture {
    service: Service,
    root: tempfile::TempDir,
    entered: mpsc::Receiver<()>,
    resume: mpsc::Sender<()>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("tagrex-handle-lifecycle-")
            .tempdir()
            .unwrap();
        let media = root.path().join("media");
        std::fs::create_dir(&media).unwrap();
        let mut audio = b"RIFF".to_vec();
        audio.extend(356u32.to_le_bytes());
        audio.extend(b"WAVEfmt ");
        audio.extend(16u32.to_le_bytes());
        audio.extend(1u16.to_le_bytes());
        audio.extend(1u16.to_le_bytes());
        audio.extend(8000u32.to_le_bytes());
        audio.extend(16000u32.to_le_bytes());
        audio.extend(2u16.to_le_bytes());
        audio.extend(16u16.to_le_bytes());
        audio.extend(b"data");
        audio.extend(320u32.to_le_bytes());
        audio.extend([0; 320]);
        std::fs::write(media.join("track.wav"), audio).unwrap();
        let app = tagrex_commands::App::open(&media, &root.path().join("journal.sqlite")).unwrap();
        let service =
            Service::new(Arc::new(Mutex::new(Some(app))), root.path().join("service")).unwrap();
        service.set_enabled(true).unwrap();
        let (entered_tx, entered) = mpsc::channel();
        let (resume, resume_rx) = mpsc::channel();
        service
            .set_provider_backend(Arc::new(Backend {
                gate: Gate {
                    entered: entered_tx,
                    resume: Mutex::new(resume_rx),
                },
            }))
            .unwrap();
        Self {
            service,
            root,
            entered,
            resume,
        }
    }
    fn actor(&self) -> Principal {
        self.actor_expiry(None)
    }
    fn actor_expiry(&self, expires_at: Option<u64>) -> Principal {
        let pair = self
            .service
            .pair_client(
                "Lifecycle probe".into(),
                Grant {
                    roots: vec![self.root.path().join("media")],
                    expires_at,
                    actions: [
                        "read".into(),
                        "providers".into(),
                        "stage".into(),
                        "write".into(),
                        "history".into(),
                    ]
                    .into(),
                    ..Default::default()
                },
            )
            .unwrap();
        self.service.authenticate(&pair.credential).unwrap()
    }
    fn release(&self, actor: &Principal, id: &str) -> ServiceResult<Value> {
        self.service.call(
            actor,
            "providers.release",
            json!({"source":"discogs", "id":id}),
        )
    }
}
#[test]
fn failed_provider_image_batch_does_not_consume_unreturned_receipts() {
    let f = Fixture::new();
    let actor = f.actor();
    let earlier = f.release(&actor, "4").unwrap();
    assert_eq!(f.release(&actor, "253").unwrap_err().code, "quota");
    for image in earlier["images"].as_array().unwrap() {
        f.service
            .call(&actor, "handles.release", json!({"handle":image["image"]}))
            .unwrap();
    }
    f.service
        .call(
            &actor,
            "handles.release",
            json!({"handle":earlier["release"]}),
        )
        .unwrap();
    let admitted = f
        .release(&actor, "256")
        .expect("failed batch left unreachable image receipts");
    assert_eq!(admitted["images"].as_array().unwrap().len(), 256);
}
#[test]
fn provider_result_after_revocation_does_not_retain_private_receipts() {
    let f = Fixture::new();
    let actor = f.actor();
    let service = f.service.clone();
    let worker_actor = actor.clone();
    let worker = std::thread::spawn(move || {
        service.call(
            &worker_actor,
            "providers.release",
            json!({"source":"discogs", "id":"wait"}),
        )
    });
    f.entered.recv_timeout(Duration::from_secs(5)).unwrap();
    f.service.revoke_client(actor.id()).unwrap();
    f.resume.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap().unwrap_err().code,
        "permission_denied"
    );
    let next = f.actor();
    let admitted = f
        .release(&next, "256")
        .expect("revoked fetch left unreachable receipts");
    assert_eq!(admitted["images"].as_array().unwrap().len(), 256);
}
#[test]
fn provider_result_while_disabled_does_not_consume_receipt_capacity() {
    let f = Fixture::new();
    let actor = f.actor();
    let service = f.service.clone();
    let worker_actor = actor.clone();
    let worker = std::thread::spawn(move || {
        service.call(
            &worker_actor,
            "providers.release",
            json!({"source":"discogs", "id":"wait"}),
        )
    });
    f.entered.recv_timeout(Duration::from_secs(5)).unwrap();
    f.service.set_enabled(false).unwrap();
    f.resume.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap().unwrap_err().code,
        "permission_denied"
    );
    f.service.set_enabled(true).unwrap();
    let admitted = f
        .release(&actor, "256")
        .expect("disabled fetch left an unreturned receipt");
    assert_eq!(admitted["images"].as_array().unwrap().len(), 256);
}
#[test]
fn fetched_artwork_after_revocation_does_not_retain_an_unreachable_asset() {
    let f = Fixture::new();
    let actor = f.actor();
    let release = f.release(&actor, "asset-wait").unwrap();
    let image = release["images"][0]["image"].clone();
    let service = f.service.clone();
    let worker_actor = actor.clone();
    let worker = std::thread::spawn(move || {
        service.call(&worker_actor, "artwork.fetch", json!({"image":image}))
    });
    f.entered.recv_timeout(Duration::from_secs(5)).unwrap();
    f.service.revoke_client(actor.id()).unwrap();
    f.resume.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap().unwrap_err().code,
        "permission_denied"
    );
    let next = f.actor();
    let encoded = png();
    for index in 0..256 {
        f.service
            .call(&next, "artwork.upload", json!({"data_base64":encoded}))
            .unwrap_or_else(|error| {
                panic!("asset {index}: revoked artwork retained capacity: {error}")
            });
    }
}

#[test]
fn oversized_release_response_refuses_before_creating_any_private_handle() {
    let f = Fixture::new();
    let actor = f.actor();
    assert_eq!(
        f.release(&actor, "large").unwrap_err().code,
        "response_too_large"
    );
    assert_eq!(
        f.release(&actor, "256")
            .expect("oversized response retained unreturned image handles")["images"]
            .as_array()
            .unwrap()
            .len(),
        256
    );
}
fn all_query() -> Value {
    json!({"version":2,"text":"","root":{"kind":"group","id":"all","mode":"all","children":[]}})
}
#[test]
fn invalid_query_projection_does_not_consume_a_selection_handle() {
    let f = Fixture::new();
    let actor = f.actor();
    let snapshot = f.service.call(&actor, "files.snapshot", json!({})).unwrap();
    assert_eq!(
        f.service
            .call(
                &actor,
                "files.query",
                json!({
                    "snapshot":snapshot["snapshot"], "query":all_query(), "fields":17
                })
            )
            .unwrap_err()
            .code,
        "invalid_argument"
    );
    for index in 0..256 {
        f.service
            .call(
                &actor,
                "files.query",
                json!({
                    "snapshot":snapshot["snapshot"], "query":all_query(), "fields":["path"]
                }),
            )
            .unwrap_or_else(|error| {
                panic!("selection {index}: invalid query retained a handle: {error}")
            });
    }
}
#[test]
fn failed_initial_operation_persistence_does_not_leave_an_unstarted_queue_owner() {
    let f = Fixture::new();
    let actor = f.actor();
    let path = f.root.path().join("media/track.wav");
    let before = std::fs::read(&path).unwrap();
    let draft = f
        .service
        .call(
            &actor,
            "draft.create",
            json!({
                "paths":[path], "values":{"title":"After storage retry"}
            }),
        )
        .unwrap();
    let operations = f.root.path().join("service/operations.json");
    let previous = f.root.path().join("saved-operations.json");
    if operations.exists() {
        std::fs::rename(&operations, &previous).unwrap();
    }
    std::fs::create_dir(&operations).unwrap();
    let args = json!({"draft":draft["draft"], "digest":draft["digest"], "idempotency_key":"storage-retry"});
    assert_eq!(
        f.service
            .call(&actor, "plan.commit", args.clone())
            .unwrap_err()
            .code,
        "storage_error"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(
        f.service.operations().unwrap()["operations"]
            .as_array()
            .unwrap()
            .is_empty(),
        "failed persistence retained an unstarted queued operation"
    );
    std::fs::remove_dir(&operations).unwrap();
    if previous.exists() {
        std::fs::rename(&previous, &operations).unwrap();
    }
    let retry = f.service.call(&actor, "plan.commit", args).unwrap();
    assert_ne!(retry.get("reused"), Some(&Value::Bool(true)));
    let until = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let value = f
            .service
            .call(
                &actor,
                "operations.status",
                json!({"operation":retry["operation"]}),
            )
            .unwrap();
        let status = value["operations"][0]["status"].as_str().unwrap();
        if !["queued", "preflight", "writing", "rolling_back"].contains(&status) {
            assert_eq!(status, "succeeded", "retry did not complete: {value}");
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "retry never started: {value}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn expired_provider_result_does_not_retain_unreturned_receipts() {
    let f = Fixture::new();
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    let expiry = now() + 2;
    let actor = f.actor_expiry(Some(expiry));
    let service = f.service.clone();
    let worker = std::thread::spawn(move || {
        service.call(
            &actor,
            "providers.release",
            json!({"source":"discogs", "id":"wait"}),
        )
    });
    f.entered.recv_timeout(Duration::from_secs(5)).unwrap();
    while now() < expiry {
        std::thread::sleep(Duration::from_millis(10));
    }
    f.resume.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap().unwrap_err().code,
        "permission_denied"
    );
    let next = f.actor();
    assert_eq!(
        f.release(&next, "256").unwrap()["images"]
            .as_array()
            .unwrap()
            .len(),
        256
    );
}
#[test]
fn fallback_cover_receipt_obeys_the_same_aggregate_quota() {
    let f = Fixture::new();
    let actor = f.actor();
    let full = f.release(&actor, "256").unwrap();
    assert_eq!(f.release(&actor, "fallback").unwrap_err().code, "quota");
    f.service
        .call(
            &actor,
            "handles.release",
            json!({"handle":full["images"][0]["image"]}),
        )
        .unwrap();
    let allowed = f.release(&actor, "fallback").unwrap();
    assert_eq!(allowed["images"].as_array().unwrap().len(), 1);
}
fn retire(service: &Service, actor: &Principal, release: &Value) {
    for image in release["images"].as_array().unwrap() {
        service
            .call(actor, "handles.release", json!({"handle":image["image"]}))
            .unwrap();
    }
    service
        .call(
            actor,
            "handles.release",
            json!({"handle":release["release"]}),
        )
        .unwrap();
}
#[test]
fn concurrent_receipt_batches_refuse_atomically_and_release_exact_capacity() {
    let f = Fixture::new();
    let actors: Vec<_> = (0..8).map(|_| f.actor()).collect();
    for _ in 0..12 {
        let barrier = Arc::new(std::sync::Barrier::new(9));
        let workers: Vec<_> = actors
            .iter()
            .map(|actor| {
                let service = f.service.clone();
                let actor = actor.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let result = service.call(
                        &actor,
                        "providers.release",
                        json!({"source":"discogs", "id":"40"}),
                    );
                    (actor, result)
                })
            })
            .collect();
        barrier.wait();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        let mut successes = 0;
        for (actor, result) in results {
            match result {
                Ok(release) => {
                    successes += 1;
                    retire(&f.service, &actor, &release);
                }
                Err(error) => assert_eq!(error.code, "quota"),
            }
        }
        assert_eq!(successes, 6, "capacity admission changed under contention");
        let full = f
            .release(&actors[0], "256")
            .expect("concurrent refused batch retained partial owners");
        retire(&f.service, &actors[0], &full);
    }
}

#[test]
fn oversized_query_page_refuses_before_retaining_its_selection() {
    let f = Fixture::new();
    let actor = f.actor();
    let path = f.root.path().join("media/track.wav");
    let mut track = tagrex_core::model::TagEngine::read(&path).unwrap();
    track.tags.insert(
        tagrex_core::model::TagField::Comment,
        "長い literal comment ; ".repeat(60_000),
    );
    tagrex_core::model::TagEngine::write(&track).unwrap();
    let snapshot = f.service.call(&actor, "files.snapshot", json!({})).unwrap();
    assert_eq!(
        f.service
            .call(
                &actor,
                "files.query",
                json!({
                    "snapshot":snapshot["snapshot"], "query":all_query(), "fields":["comment"]
                })
            )
            .unwrap_err()
            .code,
        "response_too_large"
    );
    for index in 0..256 {
        f.service
            .call(
                &actor,
                "files.query",
                json!({
                    "snapshot":snapshot["snapshot"], "query":all_query(), "fields":["title"]
                }),
            )
            .unwrap_or_else(|error| {
                panic!("selection {index}: oversized page retained a handle: {error}")
            });
    }
}
