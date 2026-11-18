//! Real public history requests with an observational native-mutex barrier.
use super::*;
use std::time::{Duration, Instant};
#[test]
fn history_request_waiting_for_app_cannot_mutate_after_revocation() {
    refused_history("history.undo", false);
}
#[test]
fn history_request_waiting_for_app_cannot_mutate_after_disable() {
    refused_history("history.undo", true);
}
#[test]
fn redo_request_waiting_for_app_cannot_mutate_after_revocation() {
    refused_history("history.redo", false);
}
fn refused_history(tool: &'static str, disable: bool) {
    let root = tempfile::Builder::new()
        .prefix("tagrex-history-revocation-")
        .tempdir()
        .unwrap();
    let media = root.path().join("media");
    fs::create_dir(&media).unwrap();
    let path = media.join("track.wav");
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
    fs::write(&path, audio).unwrap();
    let app = App::open(&media, &root.path().join("journal.sqlite")).unwrap();
    let shared = Arc::new(Mutex::new(Some(app)));
    let service = Service::new(shared.clone(), root.path().join("service")).unwrap();
    service.set_enabled(true).unwrap();
    let pair = service
        .pair_client(
            "History lifecycle".into(),
            Grant {
                roots: vec![media],
                actions: ["read", "stage", "write", "history"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                ..Default::default()
            },
        )
        .unwrap();
    let actor = service.authenticate(&pair.credential).unwrap();
    let draft = service
        .call(
            &actor,
            "draft.create",
            json!({"paths":[path], "values":{"title":"Kept after revoke"}}),
        )
        .unwrap();
    let operation = service
        .call(
            &actor,
            "plan.commit",
            json!({"draft":draft["draft"], "digest":draft["digest"], "idempotency_key":"initial"}),
        )
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let status = service
            .call(
                &actor,
                "operations.status",
                json!({"operation":operation["operation"]}),
            )
            .unwrap();
        if status["operations"][0]["status"] == "succeeded" {
            break;
        }
        assert!(Instant::now() < until, "initial write failed: {status}");
        std::thread::sleep(Duration::from_millis(2));
    }
    if tool == "history.redo" {
        service
            .call(
                &actor,
                "history.undo",
                json!({"operation":operation["operation"]}),
            )
            .unwrap();
    }
    let history = service.call(&actor, "history.list", json!({})).unwrap();
    let args =
        json!({"operation":operation["operation"], "redo_revision":history["redo_revision"]});
    let held_app = shared.lock().unwrap();
    let before = fs::read(&path).unwrap();
    let before_history = held_app.as_ref().unwrap().history().unwrap();
    let before_redo = (
        held_app.as_ref().unwrap().redo_revision(),
        held_app.as_ref().unwrap().redo_count(),
        held_app.as_ref().unwrap().redo_token(),
    );
    let before_receipts = fs::read(root.path().join("service/operations.json")).unwrap();
    let worker_service = service.clone();
    let worker_actor = actor.clone();
    let worker = std::thread::spawn(move || worker_service.call(&worker_actor, tool, args));
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        // No mutation behavior is injected. Owning this mutex proves the real
        // public request passed its initial grant and is waiting for the held App.
        if matches!(
            service.0.mutation.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            break;
        }
        assert!(
            Instant::now() < until,
            "history request did not enter its native serialization lane"
        );
        std::thread::yield_now();
    }
    if disable {
        service.set_enabled(false).unwrap();
    } else {
        service.revoke_client(actor.id()).unwrap();
    }
    drop(held_app);
    assert_eq!(
        worker.join().unwrap().unwrap_err().code,
        "permission_denied"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "revoked history request changed media despite returning denial"
    );
    let app = shared.lock().unwrap();
    let app = app.as_ref().unwrap();
    assert_eq!(app.history().unwrap(), before_history);
    assert_eq!(
        (app.redo_revision(), app.redo_count(), app.redo_token()),
        before_redo
    );
    assert_eq!(
        fs::read(root.path().join("service/operations.json")).unwrap(),
        before_receipts
    );
}
