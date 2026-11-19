//! Independent public-service snapshot ownership, scope and lifetime probes.
//! All media and profiles are owned disposable fixtures.
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    process::Command,
    sync::{Arc, Barrier, Mutex},
    time::{Duration, Instant},
};
use tagrex_commands::App;
use tagrex_core::resources::{Budget, Charge};
use tagrex_service::{Grant, Principal, Service, ServiceResult, SharedApp};

fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_SNAPSHOT_OWNERSHIP_REVIEW").as_deref() == Ok(name) {
        return false;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_SNAPSHOT_OWNERSHIP_REVIEW", name)
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
fn flac(title: &str, comment: &str) -> Vec<u8> {
    let mut output = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    output.extend(info);
    let mut comments = 6u32.to_le_bytes().to_vec();
    comments.extend(b"review");
    comments.extend(4u32.to_le_bytes());
    for item in [
        format!("TITLE={title}"),
        "ARTIST=One".into(),
        "ARTIST=Two".into(),
        format!("COMMENT={comment}"),
    ] {
        comments.extend((item.len() as u32).to_le_bytes());
        comments.extend(item.as_bytes());
    }
    let n = comments.len() as u32;
    output.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    output.extend(comments);
    output.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    output
}
struct Fixture {
    service: Service,
    app: SharedApp,
    _root: tempfile::TempDir,
    media: PathBuf,
}
impl Fixture {
    fn new(per_group: usize, comment: &str) -> Self {
        let root = tempfile::Builder::new()
            .prefix("tagrex-snapshot-independent-")
            .tempdir()
            .unwrap();
        let media = root.path().join("media");
        std::fs::create_dir(&media).unwrap();
        for group in ["approved-a", "hidden-b", "approved-c"] {
            let folder = media.join(group);
            std::fs::create_dir(&folder).unwrap();
            for number in 0..per_group {
                std::fs::write(
                    folder.join(format!("{number}.flac")),
                    flac(&format!("{group} {number} 尾"), comment),
                )
                .unwrap();
            }
        }
        let app = Arc::new(Mutex::new(Some(
            App::open(&media, &root.path().join("journal.sqlite")).unwrap(),
        )));
        let service = Service::new(app.clone(), root.path().join("service")).unwrap();
        service.set_enabled(true).unwrap();
        Self {
            service,
            app,
            _root: root,
            media,
        }
    }
    fn pair(&self, roots: Vec<PathBuf>) -> Principal {
        let pairing = self
            .service
            .pair_client(
                "Independent reader".into(),
                Grant {
                    roots,
                    actions: ["read".into()].into(),
                    ..Default::default()
                },
            )
            .unwrap();
        self.service.authenticate(&pairing.credential).unwrap()
    }
    fn full(&self) -> Principal {
        self.pair(vec![self.media.clone()])
    }
    fn scoped(&self, group: &str) -> Principal {
        self.pair(vec![self.media.join(group)])
    }
    fn snapshot(&self, actor: &Principal) -> Value {
        self.service
            .call(actor, "files.snapshot", json!({}))
            .unwrap()
    }
    fn page(&self, actor: &Principal, snapshot: &Value) -> Value {
        self.service
            .call(
                actor,
                "files.page",
                json!({"snapshot": snapshot["snapshot"], "limit":500,"fields":["title","artist"]}),
            )
            .unwrap()
    }
    fn release(&self, actor: &Principal, handle: &Value) {
        self.service
            .call(actor, "handles.release", json!({"handle":handle}))
            .unwrap();
    }
}
fn all_query() -> Value {
    json!({"version":2,"text":"","root":{"kind":"group","id":"all","mode":"all","children":[]}})
}
fn rows_paths(value: &Value) -> BTreeSet<PathBuf> {
    value["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| PathBuf::from(row["path"].as_str().unwrap()))
        .collect()
}

#[test]
fn sparse_scopes_keep_query_and_dto_positions_aligned_across_release_and_cancel() {
    let name = "sparse_scopes_keep_query_and_dto_positions_aligned_across_release_and_cancel";
    if isolated(name) {
        return;
    }
    let fixture = Fixture::new(17, "hidden and public metadata ; 尾");
    let full = fixture.full();
    let whole = fixture.snapshot(&full);
    let sparse = fixture.pair(vec![
        fixture.media.join("approved-a"),
        fixture.media.join("approved-c"),
    ]);
    let other = fixture.scoped("hidden-b");
    let saved = fixture.snapshot(&sparse);
    let independent = fixture.snapshot(&other);
    let page = fixture.page(&sparse, &saved);
    assert_eq!(page["total"], 34);
    let expected: BTreeSet<_> = ["approved-a", "approved-c"]
        .into_iter()
        .flat_map(|g| (0..17).map(move |n| (g, n)))
        .map(|(g, n)| fixture.media.join(g).join(format!("{n}.flac")))
        .collect();
    assert_eq!(rows_paths(&page), expected);
    for row in page["rows"].as_array().unwrap() {
        let title = row["tags"]["title"].as_str().unwrap();
        let queried = fixture.service.call(&sparse,"files.query",json!({
            "snapshot":saved["snapshot"],"fields":["title"],
            "query":{"version":2,"text":"","root":{"kind":"condition","id":"title","field":"title","operator":"equals","values":[title]}}
        })).unwrap();
        assert_eq!(queried["matched"], 1);
        assert_eq!(queried["rows"][0]["path"], row["path"]);
        fixture.release(&sparse, &queried["selection"]);
    }
    fixture
        .service
        .call(&sparse, "queries.cancel", json!({"lane":"query"}))
        .unwrap();
    fixture.release(&full, &whole["snapshot"]);
    fixture.release(&sparse, &saved["snapshot"]);
    let queried = fixture
        .service
        .call(
            &other,
            "files.query",
            json!({"snapshot":independent["snapshot"],"query":all_query(),"fields":["title"]}),
        )
        .unwrap();
    assert_eq!(queried["matched"], 17);
    assert!(rows_paths(&queried)
        .iter()
        .all(|path| path.starts_with(fixture.media.join("hidden-b"))));
    assert!(fixture
        .service
        .call(
            &sparse,
            "files.page",
            json!({"snapshot":independent["snapshot"]})
        )
        .is_err());
}

#[test]
fn snapshot_created_after_revocation_does_not_retain_an_unreachable_owner() {
    let name = "snapshot_created_after_revocation_does_not_retain_an_unreachable_owner";
    if isolated(name) {
        return;
    }
    let fixture = Fixture::new(24, "metadata shared on warm snapshot");
    let actor = fixture.full();
    let saved = fixture.snapshot(&actor);
    fixture.release(&actor, &saved["snapshot"]);
    let budget = Budget::process();
    let baseline = budget.usage().0;
    let held_app = fixture.app.lock().unwrap();
    let start = Arc::new(Barrier::new(3));
    let (send, receive) = std::sync::mpsc::channel();
    let mut workers = Vec::new();
    for _ in 0..2 {
        let service = fixture.service.clone();
        let actor = actor.clone();
        let start = start.clone();
        let send = send.clone();
        workers.push(std::thread::spawn(move || {
            start.wait();
            let result = service.call(&actor, "files.snapshot", json!({}));
            send.send(result.clone()).unwrap();
            result
        }));
    }
    start.wait();
    // One request owns the build and is blocked on the App; its simultaneous
    // sibling must return Busy without needing that mutex. No test hook needed.
    let first = receive.recv_timeout(Duration::from_secs(3));
    let blocked_is_proved = matches!(&first, Ok(Err(error)) if error.code == "busy");
    if blocked_is_proved {
        fixture.service.revoke_client(actor.id()).unwrap();
    }
    drop(held_app);
    for worker in workers {
        assert!(worker.join().unwrap().is_err());
    }
    assert!(
        blocked_is_proved,
        "overlap could not be established: {first:?}"
    );
    assert_eq!(
        budget.usage().0,
        baseline,
        "revoked request retained a snapshot that its principal can never release"
    );
}

#[test]
#[cfg(unix)]
fn cold_full_scope_snapshot_does_not_read_new_outside_symlink_metadata() {
    let name = "cold_full_scope_snapshot_does_not_read_new_outside_symlink_metadata";
    if isolated(name) {
        return;
    }
    let fixture = Fixture::new(1, "approved");
    let actor = fixture.full();
    let target = fixture.media.join("approved-a/0.flac");
    let outside = fixture._root.path().join("outside.flac");
    std::fs::write(&outside, flac("OUTSIDE SECRET FRESH READ", "private")).unwrap();
    std::fs::remove_file(&target).unwrap();
    std::os::unix::fs::symlink(&outside, &target).unwrap();
    let snapshot = fixture.snapshot(&actor);
    let page = fixture.page(&actor, &snapshot);
    assert!(
        page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["tags"]["title"] != "OUTSIDE SECRET FRESH READ"),
        "cold snapshot returned newly read metadata outside granted roots: {page}"
    );
}

#[test]
#[cfg(unix)]
fn scoped_saved_rows_exclude_paths_redirected_outside_the_grant() {
    let name = "scoped_saved_rows_exclude_paths_redirected_outside_the_grant";
    if isolated(name) {
        return;
    }
    let fixture = Fixture::new(3, "saved approved metadata");
    let full = fixture.full();
    let actor = fixture.scoped("approved-a");
    fixture.snapshot(&full);
    let directory = fixture.media.join("approved-a");
    let moved = fixture._root.path().join("moved-a");
    std::fs::rename(&directory, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &directory).unwrap();
    let snapshot = fixture.snapshot(&actor);
    assert_eq!(snapshot["total"], 0);
    assert!(fixture.page(&actor, &snapshot)["rows"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn repeated_partial_admission_refusal_releases_views_and_old_handles_remain_queryable() {
    let name = "repeated_partial_admission_refusal_releases_views_and_old_handles_remain_queryable";
    if isolated(name) {
        return;
    }
    let fixture = Fixture::new(32, &"long nonselected metadata ; 尾".repeat(1000));
    let full = fixture.full();
    let actor = fixture.scoped("approved-c");
    let first = fixture.snapshot(&full);
    let existing = fixture.snapshot(&actor);
    let budget = Budget::process();
    let baseline = budget.usage().0;
    // Enough space to reserve the scoped position view; not enough for the
    // following independent native query index. Repeated failure must free it.
    let pressure = budget
        .try_acquire(Charge {
            retained: (256 << 20) - baseline.retained - 1024,
            ..Default::default()
        })
        .unwrap();
    let full_usage = budget.usage().0;
    for _ in 0..50 {
        let error = fixture
            .service
            .call(&actor, "files.snapshot", json!({}))
            .unwrap_err();
        assert_eq!(error.code, "resource_busy");
        assert_eq!(budget.usage().0, full_usage);
    }
    drop(pressure);
    let queried = fixture
        .service
        .call(
            &actor,
            "files.query",
            json!({"snapshot":existing["snapshot"],"query":all_query(),"fields":["title"]}),
        )
        .unwrap();
    assert_eq!(queried["matched"], 32);
    fixture.release(&actor, &queried["selection"]);
    let retry = fixture.snapshot(&actor);
    assert_eq!(retry["total"], 32);
    fixture.release(&actor, &retry["snapshot"]);
    fixture.release(&actor, &existing["snapshot"]);
    fixture.release(&full, &first["snapshot"]);
}

#[test]
fn concurrent_snapshot_clients_share_base_without_invalidating_each_other() {
    let name = "concurrent_snapshot_clients_share_base_without_invalidating_each_other";
    if isolated(name) {
        return;
    }
    let fixture = Fixture::new(12, "shared base");
    let actors: Vec<_> = (0..8)
        .map(|i| {
            fixture.scoped(if i % 2 == 0 {
                "approved-a"
            } else {
                "approved-c"
            })
        })
        .collect();
    let starter = fixture.full();
    fixture.snapshot(&starter);
    let barrier = Arc::new(Barrier::new(actors.len()));
    let workers:Vec<_>=actors.into_iter().map(|actor|{
        let service=fixture.service.clone();let barrier=barrier.clone();
        std::thread::spawn(move || -> ServiceResult<()> {
            barrier.wait();
            let deadline=Instant::now()+Duration::from_secs(5);
            for _ in 0..4 {
                let snapshot=loop {
                    match service.call(&actor,"files.snapshot",json!({})) {
                        Ok(value)=>break value,
                        Err(error) if error.code=="busy" && Instant::now()<deadline=>std::thread::yield_now(),
                        Err(error)=>return Err(error),
                    }
                };
                let result=service.call(&actor,"files.query",json!({"snapshot":snapshot["snapshot"],"query":all_query(),"fields":["title"]}))?;
                assert_eq!(result["matched"],12);
                service.call(&actor,"handles.release",json!({"handle":snapshot["snapshot"]}))?;
                service.call(&actor,"handles.release",json!({"handle":result["selection"]}))?;
            }
            Ok(())
        })
    }).collect();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
}

#[test]
fn policy_generations_keep_historical_pages_and_refuse_obsolete_query_indexes() {
    let name = "policy_generations_keep_historical_pages_and_refuse_obsolete_query_indexes";
    if isolated(name) {
        return;
    }
    let fixture = Fixture::new(2, "literal separators ; / 尾");
    let full = fixture.full();
    let scoped = fixture.scoped("approved-c");
    let original = tagrex_core::model::multi_value_separator();
    let mut previous_separator = original.clone();
    let mut previous = fixture.snapshot(&full);
    for separator in [" ↔ ", " / literal ; ", "尾"] {
        let old_page = fixture.page(&full, &previous);
        assert_eq!(
            old_page["rows"][0]["tags"]["artist"],
            format!("One{previous_separator}Two")
        );
        tagrex_core::model::set_multi_value_separator(separator);
        let cancelled = fixture
            .service
            .call(
                &full,
                "files.query",
                json!({
                    "snapshot":previous["snapshot"],"query":all_query(),"fields":["artist"]
                }),
            )
            .unwrap_err();
        assert_eq!(cancelled.code, "cancelled");
        // Saved pages retain their documented historical interpretation.
        assert_eq!(
            fixture.page(&full, &previous)["rows"][0]["tags"]["artist"],
            old_page["rows"][0]["tags"]["artist"]
        );
        let current = fixture.snapshot(&full);
        let selected = fixture.snapshot(&scoped);
        assert_eq!(current["total"], 6);
        assert_eq!(selected["total"], 2);
        let page = fixture.page(&scoped, &selected);
        for row in page["rows"].as_array().unwrap() {
            assert_eq!(row["tags"]["artist"], format!("One{separator}Two"));
        }
        let query = fixture
            .service
            .call(
                &scoped,
                "files.query",
                json!({
                    "snapshot":selected["snapshot"],"query":all_query(),"fields":["artist"]
                }),
            )
            .unwrap();
        assert_eq!(query["matched"], 2);
        assert_eq!(rows_paths(&query), rows_paths(&page));
        fixture.release(&scoped, &query["selection"]);
        fixture.release(&scoped, &selected["snapshot"]);
        fixture.release(&full, &previous["snapshot"]);
        previous = current;
        previous_separator = separator.into();
    }
    tagrex_core::model::set_multi_value_separator(&original);
}
