//! Actual public query pressure/cancellation probes; all data is disposable.
use std::{
    cell::Cell,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tagrex_commands::{
    query::{QueryError, QueryIndex, QueryRequest},
    TrackDto,
};
use tagrex_core::resources::{AdmissionError, Budget, Charge};

fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_OVERLAY_CASE").as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--include-ignored", "--nocapture"])
        .env("TAGREX_OVERLAY_CASE", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}
fn index() -> QueryIndex {
    let tracks:Vec<TrackDto>=(0..64).map(|i|serde_json::from_value(serde_json::json!({
        "path":format!("/fixture/{i}.flac"),"format":"FLAC","tags":{"comment":"Literal ; / 尾 ".repeat(2000)},"has_cover":false
    })).unwrap()).collect();
    QueryIndex::try_from_tracks(&tracks).unwrap()
}
fn request(key: &str, mask: &str) -> QueryRequest {
    serde_json::from_value(serde_json::json!({"requestId":1,"query":{"version":2,
        "root":{"kind":"group","id":"root","mode":"all","children":[]},"customFields":[{"key":key,"mask":mask}]}})).unwrap()
}
#[test]
fn hot_overlay_survives_busy_cold_build_and_needs_no_job_slot() {
    if isolated("hot_overlay_survives_busy_cold_build_and_needs_no_job_slot") {
        return;
    }
    let index = index();
    let hot = request("custom:hot", "%comment%");
    assert_eq!(index.query(&hot, || false).unwrap().matched, 64);
    let budget = Budget::process();
    let before = budget.usage().0;
    let pressure = budget
        .try_acquire(Charge {
            jobs: 8,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(index.query(&hot, || false).unwrap().matched, 64);
    assert!(matches!(
        index.query(&request("custom:cold", "%comment%"), || false),
        Err(QueryError::Resource(AdmissionError::Busy))
    ));
    assert_eq!(index.query(&hot, || false).unwrap().matched, 64);
    drop(pressure);
    assert_eq!(budget.usage().0, before);
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn cancellation_and_panics_never_publish_partial_overlays_or_lose_the_old_hot_one() {
    if isolated("cancellation_and_panics_never_publish_partial_overlays_or_lose_the_old_hot_one") {
        return;
    }
    let index = index();
    let hot = request("custom:hot", "%comment%");
    index.query(&hot, || false).unwrap();
    let budget = Budget::process();
    let before = budget.usage().0;
    for checkpoint in [1, 3, 10, 50, 100, 150] {
        let calls = Cell::new(0usize);
        let outcome = index.query(
            &request(&format!("custom:candidate{checkpoint}"), "%comment%"),
            || {
                let n = calls.get() + 1;
                calls.set(n);
                n == checkpoint
            },
        );
        assert!(
            matches!(outcome, Err(QueryError::Cancelled)),
            "checkpoint {checkpoint} reached {}: {outcome:?}",
            calls.get()
        );
        assert_eq!(budget.usage().0, before);
    }
    let calls = Cell::new(0usize);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        index.query(&request("custom:panic", "%comment%"), || {
            let n = calls.get() + 1;
            calls.set(n);
            assert_ne!(n, 100, "injected overlay panic");
            false
        })
    }));
    assert!(outcome.is_err());
    assert_eq!(budget.usage().0, before);
    let pressure = budget
        .try_acquire(Charge {
            jobs: 8,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(index.query(&hot, || false).unwrap().matched, 64);
    drop(pressure);
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn real_ast_argument_work_refuses_a_tiny_result_with_large_intermediate_buffers() {
    if isolated("real_ast_argument_work_refuses_a_tiny_result_with_large_intermediate_buffers") {
        return;
    }
    let tracks:Vec<TrackDto>=vec![serde_json::from_value(serde_json::json!({"path":"/fixture/one.flac","format":"FLAC","tags":{"comment":"a".repeat(1024*1024)},"has_cover":false})).unwrap()];
    let index = QueryIndex::try_from_tracks(&tracks).unwrap();
    let budget = Budget::process();
    let before = budget.usage().0;
    let mask = format!("$and({})", vec!["%comment%"; 100].join(","));
    assert!(matches!(
        index.query(&request("custom:large", &mask), || false),
        Err(QueryError::Resource(AdmissionError::TooLarge))
    ));
    assert_eq!(budget.usage().0, before);
    // Escaped delimiters are data and do not consume imaginary argument slots.
    let literal = format!("'{}'", ",".repeat(1500));
    assert_eq!(
        index
            .query(&request("custom:literal", &literal), || false)
            .unwrap()
            .matched,
        1
    );
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn malformed_definitions_are_checked_before_work_admission_and_cache_key_serialization() {
    if isolated(
        "malformed_definitions_are_checked_before_work_admission_and_cache_key_serialization",
    ) {
        return;
    }
    let index = index();
    let budget = Budget::process();
    let before = budget.usage();
    let huge = request("custom:bad", &"\u{0001}".repeat(2 * 1024 * 1024));
    assert!(matches!(
        index.query(&huge, || false),
        Err(QueryError::Invalid(_))
    ));
    assert_eq!(budget.usage(), before);
    let jobs = budget
        .try_acquire(Charge {
            jobs: 8,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(
        index.query(&request("custom:bad", "%unknown_unscoped_name%"), || false),
        Err(QueryError::Resource(AdmissionError::Busy))
    ));
    drop(jobs);
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn concurrent_cold_overlays_are_admitted_without_overbooking_or_detached_work() {
    if isolated("concurrent_cold_overlays_are_admitted_without_overbooking_or_detached_work") {
        return;
    }
    let index = Arc::new(index());
    let budget = Budget::process();
    let occupied_jobs = budget
        .try_acquire(Charge {
            jobs: 6,
            ..Default::default()
        })
        .unwrap();
    let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let entered = Arc::new(AtomicUsize::new(0));
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut workers = Vec::new();
    for worker in 0..4 {
        let index = index.clone();
        let release = release.clone();
        let entered = entered.clone();
        let sender = sender.clone();
        workers.push(std::thread::spawn(move || {
            let calls = Cell::new(0);
            index.query(
                &request(&format!("custom:worker{worker}"), "%comment%"),
                || {
                    let n = calls.get() + 1;
                    calls.set(n);
                    if n == 3 {
                        entered.fetch_add(1, Ordering::SeqCst);
                        sender.send(()).unwrap();
                        let (lock, changed) = &*release;
                        let mut done = lock.lock().unwrap();
                        while !*done {
                            done = changed.wait(done).unwrap();
                        }
                    }
                    false
                },
            )
        }));
    }
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(entered.load(Ordering::SeqCst), 2);
    assert!(budget.usage().0.transient <= 384 * 1024 * 1024);
    let (lock, changed) = &*release;
    *lock.lock().unwrap() = true;
    changed.notify_all();
    let mut admitted = 0;
    let mut refused = 0;
    for worker in workers {
        match worker.join().unwrap() {
            Ok(result) => {
                assert_eq!(result.matched, 64);
                admitted += 1
            }
            Err(QueryError::Resource(AdmissionError::Busy)) => refused += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!((admitted, refused), (2, 2));
    assert_eq!(budget.usage().0.jobs, 6);
    drop(occupied_jobs);
    assert_eq!(budget.usage().0.transient, 0);
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn aggregate_selected_inputs_refuse_without_cloning_every_one_mib_member() {
    if isolated("aggregate_selected_inputs_refuse_without_cloning_every_one_mib_member") {
        return;
    }
    let tags: std::collections::BTreeMap<String, String> = (0..9)
        .map(|i| (format!("custom:field{i}"), "x".repeat(1024 * 1024)))
        .collect();
    let track:TrackDto=serde_json::from_value(serde_json::json!({"path":"/fixture/input.flac","format":"FLAC","tags":tags,"has_cover":false})).unwrap();
    let index = QueryIndex::try_from_tracks(&[track]).unwrap();
    let budget = Budget::process();
    let before = budget.usage().0;
    let mask = format!(
        "$and({})",
        (0..9)
            .map(|i| format!("%custom:field{i}%"))
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(matches!(
        index.query(&request("custom:aggregate", &mask), || false),
        Err(QueryError::Resource(AdmissionError::TooLarge))
    ));
    assert_eq!(budget.usage().0, before);
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn resource_refusals_keep_their_typed_desktop_codes() {
    let busy: tagrex_commands::ErrorDto = QueryError::Resource(AdmissionError::Busy).into();
    let limit: tagrex_commands::ErrorDto = QueryError::Resource(AdmissionError::TooLarge).into();
    assert_eq!(busy.message.code, "error.resourceBusy");
    assert_eq!(limit.message.code, "error.resourceLimit");
}

#[test]
#[ignore = "manual native cold/warm overlay measurement, no media or network"]
fn bounded_overlay_cold_warm_probe() {
    if isolated("bounded_overlay_cold_warm_probe") {
        return;
    }
    let index = index();
    let budget = Budget::process();
    let mut cold = Vec::new();
    let mut warm = Vec::new();
    for sample in 0..7 {
        let request = request(&format!("custom:sample{sample}"), "%comment%");
        let start = std::time::Instant::now();
        assert_eq!(index.query(&request, || false).unwrap().matched, 64);
        cold.push(start.elapsed());
        let start = std::time::Instant::now();
        assert_eq!(index.query(&request, || false).unwrap().matched, 64);
        warm.push(start.elapsed());
    }
    cold.sort();
    warm.sort();
    eprintln!(
        "OVERLAY_PROBE64 rows median cold {:?} warm {:?}, maxima {:?}/{:?}, charged {:?}",
        cold[3],
        warm[3],
        cold[6],
        warm[6],
        budget.usage()
    );
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}

#[test]
fn invalid_function_arguments_return_bounded_diagnostics_and_preserve_hot_cache() {
    if isolated("invalid_function_arguments_return_bounded_diagnostics_and_preserve_hot_cache") {
        return;
    }
    let index = index();
    let budget = Budget::process();
    let hot = request("custom:hot", "%comment%");
    index.query(&hot, || false).unwrap();
    let before = budget.usage().0;
    let outcome = index.query(
        &request("custom:badarg", "$left(%comment%,%comment%)"),
        || false,
    );
    match outcome {
        Err(QueryError::Invalid(detail)) => assert!(detail.len() < 2048),
        other => panic!("expected bounded argument diagnostic, got {other:?}"),
    }
    assert_eq!(budget.usage().0, before);
    let jobs = budget
        .try_acquire(Charge {
            jobs: 8,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(index.query(&hot, || false).unwrap().matched, 64);
    drop(jobs);
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}
