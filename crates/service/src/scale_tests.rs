//! Native service scale probes. Synthetic DTOs do not measure disk scanning,
//! provider throughput, WebView frame pacing or MCP-host/model latency.
use super::*;
use std::sync::Barrier;
use std::time::Instant;
use tagrex_commands::query::{QueryError, QueryIndex, QueryRequest};

struct ScaleFixture {
    // Drop the service/profile lease before deleting its temporary directory.
    service: Service,
    root: tempfile::TempDir,
    media: PathBuf,
    first: Principal,
    second: Principal,
}

impl ScaleFixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("tagnetic-mcp-scale-")
            .tempdir()
            .unwrap();
        let media = root.path().join("media");
        fs::create_dir(&media).unwrap();
        let app = App::open(&media, &root.path().join("journal.sqlite")).unwrap();
        // Establish the App's actual query generation before seeding a synthetic
        // saved-DTO cache. The empty file set is intentional in model probes.
        app.list_tracks().unwrap();
        let service =
            Service::new(Arc::new(Mutex::new(Some(app))), root.path().join("service")).unwrap();
        service.set_enabled(true).unwrap();
        let grant = Grant {
            roots: vec![media.clone()],
            actions: ["read".into()].into(),
            max_files: 100_000,
            ..Grant::default()
        };
        let first = service
            .pair_client("Scale agent A".into(), grant.clone())
            .unwrap();
        let second = service.pair_client("Scale agent B".into(), grant).unwrap();
        let first = service.authenticate(&first.credential).unwrap();
        let second = service.authenticate(&second.credential).unwrap();
        Self {
            service,
            root,
            media,
            first,
            second,
        }
    }

    fn seed(&self, count: usize) -> Arc<SnapshotRows> {
        let padding = "x".repeat(100);
        let rows = (0..count).map(|i| {
            serde_json::from_value(json!({
                "path":self.media.join(format!("{padding}-{i:06}.flac")),
                "format":"FLAC", "duration_secs":240,"has_cover":i%2==0,
                "tags":{"artist":format!("Artist {:03}",i%100),
                    "album":format!("Album {:05}",i/13),
                    "title":format!("Track {i:06}"),"genre":"Hardcore",
                    "comment":if i==0 {"large-unrequested-comment".repeat(80_000)} else {String::new()}}
            })).unwrap()
        }).collect::<Vec<_>>();
        let generation = self
            .service
            .0
            .app
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .query_snapshot_id()
            .unwrap();
        // These caller-managed synthetic DTOs intentionally measure the model,
        // not native admission or disk scanning. Build their immutable base in
        // the measured seed phase, just as production consumes the App's base.
        let index = Arc::new(QueryIndex::from_tracks(&rows));
        let rows = Arc::new(SnapshotRows::all(rows));
        self.service.0.state.lock().unwrap().snapshot_cache = Some(SavedSnapshot {
            generation,
            rows: rows.clone(),
            index,
        });
        rows
    }

    fn snapshot(&self, actor: &Principal) -> String {
        self.service
            .call(actor, "files.snapshot", json!({}))
            .unwrap()["snapshot"]
            .as_str()
            .unwrap()
            .into()
    }
}

fn artist_query(artist: &str) -> Value {
    json!({"version":2,"text":"","root":{"kind":"condition","id":"artist",
        "field":"artist","operator":"equals","values":[artist]}})
}

fn record(name: &str, value: Value) {
    println!(
        "MCP_SCALE_RECORD {}",
        json!({"case":name,"measurements":value})
    );
}

#[test]
fn hundred_thousand_rows_keep_projection_and_concurrent_clients_independent() {
    let fixture = ScaleFixture::new();
    let started = Instant::now();
    let rows = fixture.seed(100_000);
    let model_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let first = fixture.snapshot(&fixture.first);
    let first_snapshot_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let second = fixture.snapshot(&fixture.second);
    let second_snapshot_ms = started.elapsed().as_secs_f64() * 1000.0;
    let (first_index, second_index) = {
        let state = fixture.service.0.state.lock().unwrap();
        let a = &state.snapshots[&first];
        let b = &state.snapshots[&second];
        assert!(Arc::ptr_eq(&a.rows, &b.rows));
        assert!(Arc::ptr_eq(&a.rows, &rows));
        (a.index.clone(), b.index.clone())
    };
    let started = Instant::now();
    let page = fixture
        .service
        .call(
            &fixture.first,
            "files.page",
            json!({"snapshot":first,"limit":100_000,"fields":["title","artist"]}),
        )
        .unwrap();
    let page_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(page["rows"].as_array().unwrap().len(), MAX_PAGE);
    assert_eq!(page["total"], 100_000);
    assert_eq!(page["next_offset"], MAX_PAGE);
    assert!(page["rows"][0]["tags"].get("comment").is_none());
    let page_bytes = serde_json::to_vec(&page).unwrap().len();
    assert!(page_bytes < 1024 * 1024);
    assert_eq!(
        fixture
            .service
            .call(&fixture.second, "files.page", json!({"snapshot":first}))
            .unwrap_err()
            .code,
        "invalid_handle"
    );

    let desktop = Arc::new(QueryIndex::from_rows(
        rows.iter()
            .map(tagrex_commands::query::QueryRow::from_track)
            .collect(),
    ));
    let barrier = Arc::new(Barrier::new(3));
    let agent_one = std::thread::spawn({
        let service = fixture.service.clone();
        let actor = fixture.first.clone();
        let barrier = barrier.clone();
        let first = first.clone();
        move || {
            barrier.wait();
            let started = Instant::now();
            let result=service.call(&actor,"files.query",json!({"snapshot":first,"query":artist_query("Artist 042"),"limit":25,"fields":["artist","title"]})).unwrap();
            (result, started.elapsed().as_secs_f64() * 1000.0)
        }
    });
    let agent_two = std::thread::spawn({
        let service = fixture.service.clone();
        let actor = fixture.second.clone();
        let barrier = barrier.clone();
        let second = second.clone();
        move || {
            barrier.wait();
            let started = Instant::now();
            let result=service.call(&actor,"files.query",json!({"snapshot":second,"query":artist_query("Artist 071"),"limit":25,"fields":["artist","title"]})).unwrap();
            (result, started.elapsed().as_secs_f64() * 1000.0)
        }
    });
    let desktop_request: QueryRequest = serde_json::from_value(
        json!({"requestId":777,"snapshotId":desktop.id,"query":artist_query("Artist 005")}),
    )
    .unwrap();
    barrier.wait();
    let desktop_started = Instant::now();
    let desktop_result = desktop.query(&desktop_request, || false).unwrap();
    let desktop_ms = desktop_started.elapsed().as_secs_f64() * 1000.0;
    let (a, a_ms) = agent_one.join().unwrap();
    let (b, b_ms) = agent_two.join().unwrap();
    assert_eq!(a["matched"], 1000);
    assert_eq!(b["matched"], 1000);
    assert_eq!(desktop_result.matched, 1000);
    assert_eq!(a["rows"][0]["tags"]["artist"], "Artist 042");
    assert_eq!(b["rows"][0]["tags"]["artist"], "Artist 071");
    assert_eq!(a["rows"].as_array().unwrap().len(), 25);
    assert_ne!(a["selection"], b["selection"]);
    let mut warm_query_ms = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let warm=fixture.service.call(&fixture.first,"files.query",json!({"snapshot":first,"query":artist_query("Artist 042"),"limit":25,"fields":["artist","title"]})).unwrap();
        warm_query_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(warm["matched"], 1000);
        fixture
            .service
            .call(
                &fixture.first,
                "handles.release",
                json!({"handle":warm["selection"]}),
            )
            .unwrap();
    }
    assert_eq!(
        fixture
            .service
            .call(
                &fixture.second,
                "files.selection_page",
                json!({"selection":a["selection"],"limit":1})
            )
            .unwrap_err()
            .code,
        "invalid_handle"
    );

    // Register independent in-flight lanes without scheduler-dependent timing.
    // Exercise the public cancellation tool, then the native callback boundary.
    let cancel_a = Arc::new(AtomicBool::new(false));
    let cancel_b = Arc::new(AtomicBool::new(false));
    {
        let mut state = fixture.service.0.state.lock().unwrap();
        state.query_lanes.insert(
            (fixture.first.id.clone(), "scale".into()),
            ("active-a".into(), cancel_a.clone()),
        );
        state.query_lanes.insert(
            (fixture.second.id.clone(), "scale".into()),
            ("active-b".into(), cancel_b.clone()),
        );
    }
    let stale = fixture
        .service
        .call(
            &fixture.first,
            "queries.cancel",
            json!({"lane":"scale","query_request":"old-a"}),
        )
        .unwrap();
    assert_eq!(stale["cancel_requested"], false);
    assert!(!cancel_a.load(Ordering::Acquire));
    assert!(!cancel_b.load(Ordering::Acquire));
    let cancelled = fixture
        .service
        .call(
            &fixture.first,
            "queries.cancel",
            json!({"lane":"scale","query_request":"active-a"}),
        )
        .unwrap();
    assert_eq!(cancelled["cancel_requested"], true);
    assert!(cancel_a.load(Ordering::Acquire));
    assert!(!cancel_b.load(Ordering::Acquire));
    let replacement = Arc::new(AtomicBool::new(false));
    fixture.service.0.state.lock().unwrap().query_lanes.insert(
        (fixture.first.id.clone(), "scale".into()),
        ("replacement-a".into(), replacement.clone()),
    );
    let stale = fixture
        .service
        .call(
            &fixture.first,
            "queries.cancel",
            json!({"lane":"scale","query_request":"active-a"}),
        )
        .unwrap();
    assert_eq!(stale["cancel_requested"], false);
    assert!(!replacement.load(Ordering::Acquire));
    let request: QueryRequest = serde_json::from_value(
        json!({"requestId":42,"snapshotId":first_index.id,"query":artist_query("Artist 042")}),
    )
    .unwrap();
    assert!(matches!(
        first_index.query(&request, || cancel_a.load(Ordering::Acquire)),
        Err(QueryError::Cancelled)
    ));
    assert!(first_index.is_current());
    assert!(second_index.is_current());
    assert!(desktop.is_current());
    assert_eq!(
        desktop.query(&desktop_request, || false).unwrap().matched,
        1000
    );
    for (actor, handle) in [(&fixture.first, first), (&fixture.second, second)] {
        fixture
            .service
            .call(actor, "handles.release", json!({"handle":handle}))
            .unwrap();
        assert_eq!(
            fixture
                .service
                .call(actor, "files.page", json!({"snapshot":handle}))
                .unwrap_err()
                .code,
            "invalid_handle"
        );
    }
    record(
        "synthetic_100k_two_agents_and_desktop",
        json!({"rows":100_000,"model_generation_ms":model_ms,
        "snapshot_a_index_ms":first_snapshot_ms,"snapshot_b_index_ms":second_snapshot_ms,
        "dto_rows_shared":true,"projected_page_rows":MAX_PAGE,"projected_page_bytes":page_bytes,"projected_page_ms":page_ms,
            "concurrent_agent_a_query_ms":a_ms,"concurrent_agent_b_query_ms":b_ms,"desktop_query_ms":desktop_ms,"warm_query_ms":warm_query_ms,
        "matched_per_query":1000,"query_response_rows":25,"native_callback_cancel_isolated":true,
        "filesystem_scanning_measured":false,"webview_frame_pacing_measured":false}),
    );
}

#[test]
fn repeated_large_query_handles_hit_aggregate_selection_budget() {
    let fixture = ScaleFixture::new();
    fixture.seed(100_000);
    let snapshot = fixture.snapshot(&fixture.first);
    let query = json!({"version":2,"text":"","root":{"kind":"group","id":"root","mode":"all","children":[]}});
    let mut handles = Vec::new();
    let mut bounded = false;
    // At most 1.6m synthetic paths are retained even on the unbounded baseline;
    // this catches the defect without allocating the theoretical 25.6m paths.
    for _ in 0..16 {
        match fixture.service.call(
            &fixture.first,
            "files.query",
            json!({"snapshot":snapshot,"query":query,"limit":1,"fields":["title"]}),
        ) {
            Ok(result) => {
                assert_eq!(result["matched"], 100_000);
                handles.push(result["selection"].as_str().unwrap().to_owned());
            }
            Err(error) => {
                assert_eq!(error.code, "quota");
                bounded = true;
                break;
            }
        }
    }
    let retained_paths = fixture
        .service
        .0
        .state
        .lock()
        .unwrap()
        .selections
        .values()
        .map(|s| s.paths.len())
        .sum::<usize>();
    for handle in &handles {
        fixture
            .service
            .call(&fixture.first, "handles.release", json!({"handle":handle}))
            .unwrap();
    }
    assert!(
        bounded,
        "large selections lacked an aggregate budget before 1.6m retained path allocations"
    );
    let fresh = fixture
        .service
        .call(
            &fixture.first,
            "files.query",
            json!({"snapshot":snapshot,"query":query,"limit":1,"fields":["title"]}),
        )
        .unwrap();
    assert_eq!(fresh["matched"], 100_000);
    record(
        "aggregate_selection_admission",
        json!({"rows_per_selection":100_000,"accepted_before_quota":handles.len(),"retained_paths_before_release":retained_paths,"quota_observed":bounded,"capacity_reusable_after_release":true}),
    );
}

#[test]
fn two_thousand_disposable_wav_files_measure_cold_and_warm_native_snapshots() {
    let fixture = ScaleFixture::new();
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend(116u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(80u32.to_le_bytes());
    wav.extend([0u8; 80]);
    for i in 0..2_000 {
        fs::write(fixture.media.join(format!("Track {i:04}.wav")), &wav).unwrap();
    }
    // The initial empty App generation has not been materialized in the service
    // cache. list_tracks therefore walks and probes these actual WAV fixtures.
    let started = Instant::now();
    let first = fixture.snapshot(&fixture.first);
    let cold_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let second = fixture.snapshot(&fixture.second);
    let warm_ms = started.elapsed().as_secs_f64() * 1000.0;
    let state = fixture.service.0.state.lock().unwrap();
    let a = &state.snapshots[&first];
    let b = &state.snapshots[&second];
    assert_eq!(a.rows.len(), 2_000);
    assert_eq!(b.rows.len(), 2_000);
    assert!(Arc::ptr_eq(&a.rows, &b.rows));
    assert!(!a.rows[0].unreadable);
    drop(state);
    for i in 0..2_000 {
        assert_eq!(
            fs::read(fixture.media.join(format!("Track {i:04}.wav"))).unwrap(),
            wav
        );
    }
    assert_eq!(fs::read_dir(&fixture.media).unwrap().count(), 2_000);
    assert!(fixture.root.path().join("journal.sqlite").exists());
    record(
        "native_2000_wav_files",
        json!({"files":2_000,"bytes_per_file":wav.len(),"cold_snapshot_ms":cold_ms,"warm_snapshot_ms":warm_ms,"warm_saved_dtos_shared":true,"media_modified":false,"user_media_accessed":false}),
    );
}

/// Probe the optional native view/Inspector migrations before committing to
/// them. This measures only native work: ordered manifests already exist in the
/// query snapshot and a whole-library field aggregate is a single DTO pass. The
/// frontend's grouping/aggregate cost cannot be measured without the WebView, so
/// this records the native floor rather than justifying a speculative rewrite.
#[test]
fn native_manifest_and_inspector_aggregate_floor_probe() {
    let fixture = ScaleFixture::new();
    let rows = fixture.seed(100_000);
    let index = QueryIndex::from_rows(
        rows.iter()
            .map(tagrex_commands::query::QueryRow::from_track)
            .collect(),
    );
    let request: QueryRequest = serde_json::from_value(json!({
        "requestId":1,
        "query":{"version":2,"text":"","root":{"kind":"group","id":"root","mode":"all","children":[]}}
    }))
    .unwrap();
    let started = Instant::now();
    let manifest = index.query(&request, || false).unwrap();
    let manifest_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(manifest.paths.as_ref().unwrap().len(), 100_000);
    let mut known = request.clone();
    known.snapshot_id = Some(index.id);
    let started = Instant::now();
    let repeated = index.query(&known, || false).unwrap();
    let repeated_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(
        repeated.paths.is_none(),
        "a known snapshot reuses its manifest"
    );

    let started = Instant::now();
    let mut shared = rows[0].tags.get("genre").cloned();
    let mut mixed = false;
    for row in rows.iter() {
        match row.tags.get("genre") {
            Some(value) if shared.as_ref().is_some_and(|shared| shared != value) => mixed = true,
            _ => {}
        }
    }
    let aggregate_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(!mixed);
    let _ = shared.take();
    record(
        "native_manifest_and_aggregate_floor",
        json!({"rows":100_000,"ordered_manifest_ms":manifest_ms,
            "known_snapshot_reuse_ms":repeated_ms,"genre_aggregate_pass_ms":aggregate_ms,
            "manifest_already_native":true,"frontend_grouping_measured":false,
            "webview_frame_pacing_measured":false}),
    );
}
