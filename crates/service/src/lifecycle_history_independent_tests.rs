//! Independent actual-expiry history waiters; only observational mutex access.
use super::*;
use std::time::{Duration, Instant};
fn wav() -> Vec<u8> {
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
#[test]
fn expired_undo_and_redo_waiters_preserve_every_file_history_and_durable_receipt() {
    for tool in ["history.undo", "history.redo"] {
        let root = tempfile::Builder::new()
            .prefix("tagrex-expired-history-independent-")
            .tempdir()
            .unwrap();
        let media = root.path().join("media");
        fs::create_dir(&media).unwrap();
        let paths: Vec<_> = (0..3).map(|i| media.join(format!("{i}.wav"))).collect();
        for path in &paths {
            fs::write(path, wav()).unwrap();
        }
        let shared = Arc::new(Mutex::new(Some(
            App::open(&media, &root.path().join("journal.sqlite")).unwrap(),
        )));
        let service = Service::new(shared.clone(), root.path().join("service")).unwrap();
        service.set_enabled(true).unwrap();
        let expires = now() + 3;
        let pair = service
            .pair_client(
                "Expiring history".into(),
                Grant {
                    roots: vec![media],
                    expires_at: Some(expires),
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
                json!({"paths":&paths[..2],"values":{"title":"Owned saved value"}}),
            )
            .unwrap();
        let operation=service.call(&actor,"plan.commit",json!({"draft":draft["draft"],"digest":draft["digest"],"idempotency_key":"prepare-history"})).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let operations = service.operations().unwrap();
            // Initial in-memory success precedes receipt persistence. Establish
            // a durable baseline before testing that expired waiters cannot act.
            let receipts: Vec<OperationRecord> =
                read_json(&root.path().join("service/operations.json"))
                    .unwrap()
                    .unwrap();
            if operations["operations"][0]["status"] == "succeeded"
                && receipts.iter().any(|record| {
                    record.id == operation["operation"].as_str().unwrap()
                        && record.status == "succeeded"
                })
            {
                break;
            }
            assert!(Instant::now() < until, "initial operation: {operations}");
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
        let held = shared.lock().unwrap();
        let app = held.as_ref().unwrap();
        let before: Vec<_> = paths
            .iter()
            .map(fs::read)
            .collect::<Result<_, _>>()
            .unwrap();
        let before_history = app.history().unwrap();
        let before_redo = (app.redo_revision(), app.redo_count(), app.redo_token());
        let receipt_path = root.path().join("service/operations.json");
        let before_receipt = fs::read(&receipt_path).unwrap();
        let start = Arc::new(std::sync::Barrier::new(5));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let service = service.clone();
            let actor = actor.clone();
            let start = start.clone();
            let args = json!({"operation":operation["operation"],"redo_revision":history["redo_revision"]});
            workers.push(std::thread::spawn(move || {
                start.wait();
                service.call(&actor, tool, args)
            }));
        }
        start.wait();
        let until = Instant::now() + Duration::from_secs(2);
        while !matches!(
            service.0.mutation.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            assert!(
                Instant::now() < until,
                "request failed to reach history lane"
            );
            std::thread::yield_now();
        }
        while now() < expires {
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(held);
        for worker in workers {
            assert_eq!(
                worker.join().unwrap().unwrap_err().code,
                "permission_denied"
            );
        }
        for (path, before) in paths.iter().zip(before) {
            assert_eq!(
                fs::read(path).unwrap(),
                before,
                "{tool}: {}",
                path.display()
            );
        }
        let held = shared.lock().unwrap();
        let app = held.as_ref().unwrap();
        assert_eq!(app.history().unwrap(), before_history);
        assert_eq!(
            (app.redo_revision(), app.redo_count(), app.redo_token()),
            before_redo
        );
        assert_eq!(fs::read(receipt_path).unwrap(), before_receipt);
    }
}
