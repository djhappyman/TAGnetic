//! Independent public-service lifecycle/failure probes, disposable media only.
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Barrier, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tagrex_commands::{App, CandidateDto, CoverArtDto, ReleaseDto, SearchQueryDto};
use tagrex_service::{Grant, Principal, ProviderBackend, Service, ServiceResult, SharedApp};
fn clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn audio() -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend(356u32.to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(8000u32.to_le_bytes());
    out.extend(16000u32.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(320u32.to_le_bytes());
    out.extend([0; 320]);
    out
}
struct Backend;
impl ProviderBackend for Backend {
    fn status(&self) -> ServiceResult<Value> {
        Ok(json!({}))
    }
    fn search(&self, _: &str, _: &SearchQueryDto) -> ServiceResult<Vec<CandidateDto>> {
        Ok(Vec::new())
    }
    fn fetch_image(&self, _: &str, _: &str) -> ServiceResult<CoverArtDto> {
        unreachable!("fixture does not fetch images")
    }
    fn fetch_release(&self, _: &str, id: &str) -> ServiceResult<ReleaseDto> {
        let count = id.parse::<usize>().unwrap_or(9);
        let title = if id == "escaped" {
            "\0".repeat(190_000)
        } else {
            "Release 尾".into()
        };
        Ok(serde_json::from_value(json!({"id":id,"artist":"Artist","title":title,"year":null,
            "genres":[],"styles":[],"tracks":[],"cover_image_url":null,
            "images":(0..count).map(|i|json!({"url":format!("https://i.discogs.com/{id}-{i}.png"),"width":1,"height":1})).collect::<Vec<_>>()
        })).unwrap())
    }
}
struct Fixture {
    service: Service,
    shared: SharedApp,
    root: tempfile::TempDir,
    paths: Vec<PathBuf>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("tagrex-lifecycle-independent-")
            .tempdir()
            .unwrap();
        let media = root.path().join("media");
        fs::create_dir(&media).unwrap();
        let paths: Vec<_> = (0..5).map(|i| media.join(format!("{i}.wav"))).collect();
        for path in &paths {
            fs::write(path, audio()).unwrap();
        }
        let shared = Arc::new(Mutex::new(Some(
            App::open(&media, &root.path().join("journal.sqlite")).unwrap(),
        )));
        let service = Service::new(shared.clone(), root.path().join("service")).unwrap();
        service.set_enabled(true).unwrap();
        service.set_provider_backend(Arc::new(Backend)).unwrap();
        Self {
            service,
            shared,
            root,
            paths,
        }
    }
    fn actor(&self, expiry: Option<u64>) -> Principal {
        let pair = self
            .service
            .pair_client(
                "Independent lifecycle".into(),
                Grant {
                    roots: vec![self.root.path().join("media")],
                    expires_at: expiry,
                    actions: ["read", "stage", "write", "history", "providers"]
                        .into_iter()
                        .map(String::from)
                        .collect(),
                    ..Default::default()
                },
            )
            .unwrap();
        self.service.authenticate(&pair.credential).unwrap()
    }
    fn draft(&self, p: &Principal, title: &str) -> Value {
        self.service
            .call(
                p,
                "draft.create",
                json!({"paths":self.paths,"values":{"title":title}}),
            )
            .unwrap()
    }
    fn commit(&self, p: &Principal, draft: &Value, key: &str) -> ServiceResult<Value> {
        self.service.call(
            p,
            "plan.commit",
            json!({"draft":draft["draft"],"digest":draft["digest"],"idempotency_key":key}),
        )
    }
    fn terminal(&self, id: &Value) -> Value {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let all = self.service.operations().unwrap();
            let record = all["operations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == *id)
                .unwrap()
                .clone();
            if !matches!(
                record["status"].as_str().unwrap(),
                "queued" | "preflight" | "writing" | "rolling_back"
            ) {
                return record;
            }
            assert!(Instant::now() < until, "worker did not finish: {record}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn release(&self, p: &Principal, count: &str) -> ServiceResult<Value> {
        self.service.call(
            p,
            "providers.release",
            json!({"source":"discogs","id":count}),
        )
    }
    fn retire_release(&self, p: &Principal, result: &Value) {
        for image in result["images"].as_array().unwrap() {
            self.service
                .call(p, "handles.release", json!({"handle":image["image"]}))
                .unwrap();
        }
        self.service
            .call(p, "handles.release", json!({"handle":result["release"]}))
            .unwrap();
    }
}
#[test]
fn queued_writes_waiting_on_app_obey_revocation_disable_expiry_and_cancellation() {
    for mode in ["revoke", "disable", "expire", "cancel"] {
        let f = Fixture::new();
        let expires = (mode == "expire").then(|| clock() + 2);
        let p = f.actor(expires);
        let draft = f.draft(&p, "Must stay pending");
        let held = f.shared.lock().unwrap();
        let before: Vec<_> = f
            .paths
            .iter()
            .map(fs::read)
            .collect::<Result<_, _>>()
            .unwrap();
        let history = held.as_ref().unwrap().history().unwrap();
        let operation = f.commit(&p, &draft, "blocked-write").unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let all = f.service.operations().unwrap();
            let status = &all["operations"][0]["status"];
            if status == "preflight" {
                break;
            }
            assert!(Instant::now() < until, "did not wait on App: {all}");
            std::thread::yield_now();
        }
        match mode {
            "revoke" => f.service.revoke_client(p.id()).unwrap(),
            "disable" => f.service.set_enabled(false).unwrap(),
            "expire" => {
                while clock() <= expires.unwrap() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            "cancel" => {
                f.service
                    .call(
                        &p,
                        "operations.cancel",
                        json!({"operation":operation["operation"]}),
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        drop(held);
        let terminal = f.terminal(&operation["operation"]);
        assert_ne!(terminal["status"], "succeeded", "{mode}: {terminal}");
        for (path, before) in f.paths.iter().zip(before) {
            assert_eq!(
                fs::read(path).unwrap(),
                before,
                "{mode}: {}",
                path.display()
            );
        }
        assert_eq!(
            f.shared
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .history()
                .unwrap(),
            history,
            "{mode}"
        );
    }
}
#[test]
fn wire_expansion_refuses_before_receipt_commit_and_keeps_prior_owners() {
    let f = Fixture::new();
    let p = f.actor(None);
    let earlier = f.release(&p, "7").unwrap();
    for _ in 0..20 {
        assert_eq!(
            f.release(&p, "escaped").unwrap_err().code,
            "response_too_large"
        );
    }
    let rest = f
        .release(&p, "249")
        .expect("escaped response retained hidden image receipts");
    assert_eq!(rest["images"].as_array().unwrap().len(), 249);
    f.retire_release(&p, &rest);
    f.retire_release(&p, &earlier);
    f.retire_release(&p, &f.release(&p, "256").unwrap());
}
#[test]
fn heterogeneous_concurrent_provider_batches_commit_all_or_none() {
    let f = Fixture::new();
    let people: Vec<_> = (0..8).map(|_| f.actor(None)).collect();
    for round in 0..4 {
        let start = Arc::new(Barrier::new(9));
        let mut workers = Vec::new();
        for (i, p) in people.iter().cloned().enumerate() {
            let s = f.service.clone();
            let start = start.clone();
            let count = [3, 17, 29, 41, 53, 67, 79, 97][(i + round) % 8];
            workers.push(std::thread::spawn(move || {
                start.wait();
                let result = s.call(
                    &p,
                    "providers.release",
                    json!({"source":"discogs","id":count.to_string()}),
                );
                (p, count, result)
            }));
        }
        start.wait();
        let mut accepted = Vec::new();
        let mut total = 0usize;
        for worker in workers {
            let (p, count, result) = worker.join().unwrap();
            match result {
                Ok(result) => {
                    assert_eq!(result["images"].as_array().unwrap().len(), count);
                    total += count;
                    accepted.push((p, result));
                }
                Err(error) => assert_eq!(error.code, "quota"),
            }
        }
        assert!(total <= 256);
        if total < 256 {
            let remaining = (256 - total).to_string();
            let result = f
                .release(&people[0], &remaining)
                .expect("refused concurrent batch left partial image owners");
            accepted.push((people[0].clone(), result));
        }
        for (p, result) in accepted {
            f.retire_release(&p, &result);
        }
        f.retire_release(&people[0], &f.release(&people[0], "256").unwrap());
    }
}

#[test]
fn failed_initial_persistence_then_overlapping_retries_create_one_real_operation() {
    let f = Fixture::new();
    let p = f.actor(None);
    let draft = f.draft(&p, "First saved title");
    let first = f.commit(&p, &draft, "first-real-write").unwrap();
    assert_eq!(f.terminal(&first["operation"])["status"], "succeeded");
    let old_receipt = f
        .service
        .call(
            &p,
            "operations.status",
            json!({"operation":first["operation"]}),
        )
        .unwrap();
    let before_history = f
        .shared
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .history()
        .unwrap();
    let draft = f.draft(&p, "One retry title");
    let registry = f.root.path().join("service/operations.json");
    let saved = f.root.path().join("saved-operations.json");
    fs::rename(&registry, &saved).unwrap();
    fs::create_dir(&registry).unwrap();
    assert_eq!(
        f.commit(&p, &draft, "repaired-key").unwrap_err().code,
        "storage_error"
    );
    assert_eq!(
        f.service.operations().unwrap()["operations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        f.shared
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .history()
            .unwrap(),
        before_history
    );
    fs::remove_dir(&registry).unwrap();
    fs::rename(&saved, &registry).unwrap();
    let start = Arc::new(Barrier::new(9));
    let mut workers = Vec::new();
    for _ in 0..8 {
        let s = f.service.clone();
        let p = p.clone();
        let draft = draft.clone();
        let start = start.clone();
        workers.push(std::thread::spawn(move||{start.wait();s.call(&p,"plan.commit",json!({"draft":draft["draft"],"digest":draft["digest"],"idempotency_key":"repaired-key"})).unwrap()}));
    }
    start.wait();
    let replies: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    for reply in &replies {
        assert_eq!(reply["operation"], replies[0]["operation"]);
    }
    assert_eq!(f.terminal(&replies[0]["operation"])["status"], "succeeded");
    assert_eq!(
        f.service.operations().unwrap()["operations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        f.shared
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .len(),
        before_history.len() + 1
    );
    assert_eq!(
        f.service
            .call(
                &p,
                "operations.status",
                json!({"operation":first["operation"]})
            )
            .unwrap(),
        old_receipt
    );
    let persisted: Value = serde_json::from_slice(&fs::read(registry).unwrap()).unwrap();
    assert_eq!(persisted.as_array().unwrap().len(), 2);
}
