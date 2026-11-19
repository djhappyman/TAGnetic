//! Actual public service snapshots must share/admit native state and policy.
use serde_json::{json, Value};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tagrex_core::resources::Budget;
use tagrex_service::{Grant, Principal, Service};
struct Observed;
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
// SAFETY: every pointer/layout operation delegates unchanged to System. The
// counter observes successful allocation sizes and never changes allocation.
unsafe impl GlobalAlloc for Observed {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            OUTSTANDING.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            OUTSTANDING.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        OUTSTANDING.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let replacement = unsafe { System.realloc(pointer, layout, size) };
        if !replacement.is_null() {
            if size >= layout.size() {
                OUTSTANDING.fetch_add(size - layout.size(), Ordering::Relaxed);
            } else {
                OUTSTANDING.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        replacement
    }
}
#[global_allocator]
static ALLOCATOR: Observed = Observed;

fn isolate(name: &str) -> bool {
    if std::env::var("TAGREX_SNAPSHOT_RECHECK").as_deref() == Ok(name) {
        return false;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_SNAPSHOT_RECHECK", name)
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
struct Fixture {
    app: Arc<Mutex<Option<tagrex_commands::App>>>,
    service: Service,
    actor: Principal,
    _root: tempfile::TempDir,
}
fn flac(comment: &str) -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut comments = 4u32.to_le_bytes().to_vec();
    comments.extend(b"test");
    comments.extend(4u32.to_le_bytes());
    for item in [
        "TITLE=Track".to_string(),
        "ARTIST=A".to_string(),
        "ARTIST=B".to_string(),
        format!("COMMENT={comment}"),
    ] {
        comments.extend((item.len() as u32).to_le_bytes());
        comments.extend(item.as_bytes());
    }
    let n = comments.len() as u32;
    out.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(comments);
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out
}
impl Fixture {
    fn new(count: usize, comment: &str) -> Self {
        let root = tempfile::Builder::new()
            .prefix("tagrex-service-snapshot-recheck-")
            .tempdir()
            .unwrap();
        let media = root.path().join("media");
        std::fs::create_dir(&media).unwrap();
        let bytes = flac(comment);
        let scoped = media.join("scoped");
        std::fs::create_dir(&scoped).unwrap();
        for i in 0..count {
            let parent = if i < 8 { &scoped } else { &media };
            std::fs::write(parent.join(format!("{i:04}.flac")), &bytes).unwrap();
        }
        let app = tagrex_commands::App::open(&media, &root.path().join("journal.sqlite")).unwrap();
        let app = Arc::new(Mutex::new(Some(app)));
        let service = Service::new(app.clone(), root.path().join("service")).unwrap();
        service.set_enabled(true).unwrap();
        let pairing = service
            .pair_client(
                "Fixture snapshot reader".into(),
                Grant {
                    roots: vec![media],
                    actions: ["read".into()].into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let actor = service.authenticate(&pairing.credential).unwrap();
        Self {
            app,
            service,
            actor,
            _root: root,
        }
    }
    fn snapshot(&self) -> Value {
        self.service
            .call(&self.actor, "files.snapshot", json!({}))
            .unwrap()
    }
    fn page(&self, snapshot: &Value) -> Value {
        self.service
            .call(
                &self.actor,
                "files.page",
                json!({"snapshot":snapshot["snapshot"],"fields":["artist"]}),
            )
            .unwrap()
    }
}
#[test]
fn repeated_service_snapshot_never_allocates_an_unadmitted_second_base_index() {
    if isolate("repeated_service_snapshot_never_allocates_an_unadmitted_second_base_index") {
        return;
    }
    let fixture = Fixture::new(64, &"Comment literal ; / 尾 ".repeat(3000));
    let first = fixture.snapshot();
    assert_eq!(first["total"], 64);
    let budget = Budget::process();
    let before = budget.usage().0.retained;
    let pressure = budget
        .try_acquire(tagrex_core::resources::Charge {
            retained: (256 << 20) - before,
            ..Default::default()
        })
        .unwrap();
    let allocated_before = OUTSTANDING.load(Ordering::Relaxed);
    let charged_before = budget.usage().0.retained;
    let result = fixture
        .service
        .call(&fixture.actor, "files.snapshot", json!({}));
    if let Err(error) = &result {
        assert_eq!(error.code, "resource_busy");
    }
    let allocated = OUTSTANDING
        .load(Ordering::Relaxed)
        .saturating_sub(allocated_before);
    let charged = budget.usage().0.retained.saturating_sub(charged_before);
    eprintln!(
        "SERVICE_SNAPSHOT retained_delta={allocated} charge_delta={charged} success={}",
        result.is_ok()
    );
    assert!(
        allocated.saturating_sub(charged) < 1024 * 1024,
        "service bypasses native base admission while shared retained budget is full"
    );
    drop(pressure);
}
#[test]
fn new_service_snapshot_reinterprets_metadata_after_another_session_policy_changes() {
    if isolate("new_service_snapshot_reinterprets_metadata_after_another_session_policy_changes") {
        return;
    }
    let fixture = Fixture::new(1, "");
    let original = tagrex_core::model::multi_value_separator();
    let first = fixture.snapshot();
    let before = fixture.page(&first);
    assert_eq!(before["rows"][0]["tags"]["artist"], format!("A{original}B"));
    tagrex_core::model::set_multi_value_separator(" ~~ ");
    let peer = fixture.snapshot();
    assert_eq!(
        fixture.page(&peer)["rows"][0]["tags"]["artist"],
        format!("A{original}B"),
        "standalone peer settings must not change this App's interpretation"
    );
    fixture
        .app
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .apply_settings(&tagrex_commands::SettingsDto {
            multi_value_separator: " ~~ ".into(),
            ..Default::default()
        });
    let second = fixture.snapshot();
    let after = fixture.page(&second);
    tagrex_core::model::set_multi_value_separator(&original);
    assert_eq!(
        after["rows"][0]["tags"]["artist"], "A ~~ B",
        "new snapshot reused an old-policy saved DTO cache"
    );
}

#[test]
fn scoped_service_snapshots_share_saved_metadata_instead_of_cloning_long_fields() {
    let name = "scoped_service_snapshots_share_saved_metadata_instead_of_cloning_long_fields";
    if isolate(name) {
        return;
    }
    let fixture = Fixture::new(64, &"Comment literal ; / 尾 ".repeat(3000));
    let first = fixture.snapshot();
    assert_eq!(first["total"], 64);
    let pairing = fixture
        .service
        .pair_client(
            "Scoped reader".into(),
            Grant {
                roots: vec![fixture._root.path().join("media/scoped")],
                actions: ["read".into()].into(),
                ..Default::default()
            },
        )
        .unwrap();
    let actor = fixture.service.authenticate(&pairing.credential).unwrap();
    let budget = Budget::process();
    let before = budget.usage().0.retained;
    let pressure = budget
        .try_acquire(tagrex_core::resources::Charge {
            retained: (256 << 20) - before,
            ..Default::default()
        })
        .unwrap();
    let allocated_before = OUTSTANDING.load(Ordering::Relaxed);
    let result = fixture.service.call(&actor, "files.snapshot", json!({}));
    match &result {
        Ok(snapshot) => {
            assert_eq!(snapshot["total"], 8);
            let page = fixture
                .service
                .call(
                    &actor,
                    "files.page",
                    json!({
                        "snapshot": snapshot["snapshot"], "fields": ["path", "artist"]
                    }),
                )
                .unwrap();
            assert_eq!(page["rows"].as_array().unwrap().len(), 8);
        }
        Err(error) => assert_eq!(error.code, "resource_busy"),
    }
    let allocated = OUTSTANDING
        .load(Ordering::Relaxed)
        .saturating_sub(allocated_before);
    eprintln!(
        "SCOPED_SNAPSHOT retained_delta={allocated} success={}",
        result.is_ok()
    );
    assert!(
        allocated < 128 * 1024,
        "scoped metadata cloned {allocated} bytes while budget full"
    );
    drop(pressure);
    // Refusal must not poison the cache or retain admission. The same grant
    // succeeds after pressure leaves, and DTO/query positions remain aligned.
    let snapshot = fixture
        .service
        .call(&actor, "files.snapshot", json!({}))
        .unwrap();
    assert_eq!(snapshot["total"], 8);
    let usage = budget.usage().0.retained;
    assert!(usage > before);
    let page = fixture
        .service
        .call(
            &actor,
            "files.page",
            json!({
                "snapshot": snapshot["snapshot"], "fields": ["path", "artist"]
            }),
        )
        .unwrap();
    for row in page["rows"].as_array().unwrap() {
        assert!(std::path::Path::new(row["path"].as_str().unwrap())
            .starts_with(fixture._root.path().join("media/scoped")));
    }
    let queried = fixture
        .service
        .call(
            &actor,
            "files.query",
            json!({
                "snapshot": snapshot["snapshot"], "fields": ["path"],
                "query": {"version": 2, "text": "", "root": {
                    "kind": "group", "id": "all", "mode": "all", "children": []
                }}
            }),
        )
        .unwrap();
    assert_eq!(queried["matched"], 8);
    assert_eq!(queried["rows"].as_array().unwrap().len(), 8);
    fixture
        .service
        .call(
            &actor,
            "handles.release",
            json!({
                "handle": snapshot["snapshot"]
            }),
        )
        .unwrap();
    assert!(budget.usage().0.retained < usage);
}
