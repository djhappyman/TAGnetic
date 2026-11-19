//! Independent reentrant public-query ownership and routing checks.
use std::cell::{Cell, RefCell};
use tagrex_commands::{
    query::{QueryError, QueryIndex, QueryRequest},
    ErrorDto, TrackDto,
};
use tagrex_core::resources::{AdmissionError, Budget, Charge};
fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_QUERY_OVERLAY_ADVERSARIAL").as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_QUERY_OVERLAY_ADVERSARIAL", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}
fn index() -> QueryIndex {
    let tracks:Vec<TrackDto>=(0..16).map(|i|serde_json::from_value(serde_json::json!({"path":format!("/fixture/{i}.flac"),"format":"FLAC","tags":{"title":"İ Artist / tail","comment":"tail; / 尾 ".repeat(1000)},"has_cover":false})).unwrap()).collect();
    QueryIndex::try_from_tracks(&tracks).unwrap()
}
fn request(key: &str, mask: &str) -> QueryRequest {
    serde_json::from_value(serde_json::json!({"requestId":1,"query":{"version":2,"root":{"kind":"group","id":"root","mode":"all","children":[]},"customFields":[{"key":key,"mask":mask}]}})).unwrap()
}
#[test]
fn nested_success_survives_outer_cancellation_and_remains_warm_under_job_pressure() {
    if isolated("nested_success_survives_outer_cancellation_and_remains_warm_under_job_pressure") {
        return;
    }
    let index = index();
    let hot = request("custom:old", "%comment%");
    let nested = request("custom:nested", "$upper(%title%)");
    index.query(&hot, || false).unwrap();
    let calls = Cell::new(0usize);
    let ran = Cell::new(false);
    let result = index.query(&request("custom:aborted", "$reverse(%comment%)"), || {
        let n = calls.get() + 1;
        calls.set(n);
        if n == 10 {
            assert_eq!(index.query(&nested, || false).unwrap().matched, 16);
            ran.set(true);
            return true;
        }
        false
    });
    assert!(ran.get());
    assert!(matches!(result, Err(QueryError::Cancelled)));
    let budget = Budget::process();
    assert_eq!(budget.usage().0.jobs, 0);
    assert_eq!(budget.usage().0.transient, 0);
    let pressure = budget
        .try_acquire(Charge {
            jobs: 8,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(index.query(&nested, || false).unwrap().matched, 16);
    assert!(matches!(
        index.query(&hot, || false),
        Err(QueryError::Resource(AdmissionError::Busy))
    ));
    drop(pressure);
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn transient_pressure_at_public_checkpoint_keeps_hot_owner_and_typed_retry() {
    if isolated("transient_pressure_at_public_checkpoint_keeps_hot_owner_and_typed_retry") {
        return;
    }
    let index = index();
    let hot = request("custom:hot", "%comment%");
    index.query(&hot, || false).unwrap();
    let budget = Budget::process();
    let before = budget.usage().0;
    let pressure = RefCell::new(None);
    let called = Cell::new(false);
    let result = index.query(&request("custom:new", "$upper(%title%)"), || {
        if !called.replace(true) {
            *pressure.borrow_mut() = Some(
                budget
                    .try_acquire(Charge {
                        transient: 384 << 20,
                        ..Default::default()
                    })
                    .unwrap(),
            );
        }
        false
    });
    let error = result.unwrap_err();
    assert!(matches!(error, QueryError::Resource(AdmissionError::Busy)));
    let dto = ErrorDto::from(error);
    assert_eq!(dto.message.code, "error.resourceBusy");
    drop(pressure.borrow_mut().take());
    assert_eq!(budget.usage().0, before);
    assert_eq!(index.query(&hot, || false).unwrap().matched, 16);
    drop(index);
    assert_eq!(budget.usage().0, Charge::default());
}
#[test]
fn invalidation_inside_nested_public_query_never_leaves_a_usable_old_generation() {
    if isolated("invalidation_inside_nested_public_query_never_leaves_a_usable_old_generation") {
        return;
    }
    let index = index();
    let request = request("custom:hot", "%comment%");
    index.query(&request, || false).unwrap();
    let calls = Cell::new(0);
    let outcome = index.query(&request, || {
        let n = calls.get() + 1;
        calls.set(n);
        if n == 2 {
            index.invalidate();
        }
        false
    });
    assert!(matches!(outcome, Err(QueryError::Cancelled)));
    assert!(!index.is_current());
    assert!(matches!(
        index.query(&request, || false),
        Err(QueryError::Cancelled)
    ));
    drop(index);
    assert_eq!(Budget::process().usage().0, Charge::default());
}
