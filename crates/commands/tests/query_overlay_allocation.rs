//! Independent allocation observations at rejected public-query boundaries.
use tagrex_commands::{
    query::{QueryError, QueryIndex, QueryRequest},
    TrackDto,
};
use tagrex_core::resources::AdmissionError;
struct Observer;
static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PEAK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn grew(bytes: usize) {
    use std::sync::atomic::Ordering;
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}
// SAFETY: delegate every pointer/layout unchanged to System; counters only
// observe successful allocation sizes and do not affect allocation data.
unsafe impl std::alloc::GlobalAlloc for Observer {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, l) };
        if !p.is_null() {
            grew(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::GlobalAlloc::alloc_zeroed(&std::alloc::System, l) };
        if !p.is_null() {
            grew(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, p, l) };
        LIVE.fetch_sub(l.size(), std::sync::atomic::Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, size: usize) -> *mut u8 {
        let p = unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, p, l, size) };
        if !p.is_null() {
            if size >= l.size() {
                grew(size - l.size());
            } else {
                LIVE.fetch_sub(l.size() - size, std::sync::atomic::Ordering::Relaxed);
            }
        }
        p
    }
}
#[global_allocator]
static ALLOCATOR: Observer = Observer;
fn isolated(name: &str) -> bool {
    if std::env::var("TAGREX_OVERLAY_ALLOCATION_CASE").as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("TAGREX_OVERLAY_ALLOCATION_CASE", name)
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

fn request(mask: &str) -> QueryRequest {
    serde_json::from_value(serde_json::json!({"requestId":1,"query":{"version":2,"root":{"kind":"group","id":"root","mode":"all","children":[]},"customFields":[{"key":"custom:display","mask":mask}]}})).unwrap()
}
#[test]
fn oversized_definition_refuses_before_serializing_its_large_json_cache_key() {
    if isolated("oversized_definition_refuses_before_serializing_its_large_json_cache_key") {
        return;
    }
    use std::sync::atomic::Ordering;
    let index = QueryIndex::from_tracks(&[]);
    let request = request(&"\u{0001}".repeat(2 * 1024 * 1024));
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    assert!(matches!(
        index.query(&request, || false),
        Err(QueryError::Invalid(_))
    ));
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    assert!(
        peak < 64 * 1024,
        "oversized source serialized/allocated {peak}bytes before validation"
    );
}
#[test]
fn aggregate_input_refuses_before_allocating_multi_mib_tag_clones() {
    if isolated("aggregate_input_refuses_before_allocating_multi_mib_tag_clones") {
        return;
    }
    use std::sync::atomic::Ordering;
    let tags: std::collections::BTreeMap<String, String> = (0..9)
        .map(|i| (format!("custom:field{i}"), "x".repeat(1024 * 1024)))
        .collect();
    let track:TrackDto=serde_json::from_value(serde_json::json!({"path":"/fixture/input.flac","format":"FLAC","tags":tags,"has_cover":false})).unwrap();
    let index = QueryIndex::try_from_tracks(&[track]).unwrap();
    let request = request(&format!(
        "$and({})",
        (0..9)
            .map(|i| format!("%custom:field{i}%"))
            .collect::<Vec<_>>()
            .join(",")
    ));
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    assert!(matches!(
        index.query(&request, || false),
        Err(QueryError::Resource(AdmissionError::TooLarge))
    ));
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    assert!(
        peak < 1024 * 1024,
        "source values cloned {peak}bytes before aggregate refusal"
    );
}
