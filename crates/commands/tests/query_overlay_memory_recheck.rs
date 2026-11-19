//! Computed field overlays must participate in shared native ownership.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
use tagrex_commands::{
    query::{QueryIndex, QueryRequest},
    TrackDto,
};
use tagrex_core::resources::Budget;
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

#[test]
fn computed_saved_snapshot_overlay_keeps_retained_admission_until_last_owner() {
    let name = "computed_saved_snapshot_overlay_keeps_retained_admission_until_last_owner";
    if std::env::var("TAGREX_OVERLAY_MEMORY_CHILD").as_deref() != Ok("1") {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env("TAGREX_OVERLAY_MEMORY_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let comment = "Preserve Literal ; / 尾 ".repeat(3000);
    let tracks: Vec<TrackDto> = (0..64).map(|i| serde_json::from_value(serde_json::json!({
        "path":format!("/fixture/{i}.flac"), "format":"FLAC", "tags":{"comment":comment}, "has_cover":false
    })).unwrap()).collect();
    let index = QueryIndex::try_from_tracks(&tracks).unwrap();
    let request: QueryRequest = serde_json::from_value(serde_json::json!({
        "requestId":1,"snapshotId":index.id,"query":{"version":2,
        "root":{"kind":"group","id":"root","mode":"all","children":[]},
        "customFields":[{"key":"custom:display","mask":"%comment%"}]}
    }))
    .unwrap();
    let before = Budget::process().usage().0.retained;
    let allocated_before = OUTSTANDING.load(Ordering::Relaxed);
    let result = index.query(&request, || false).unwrap();
    assert_eq!(result.matched, 64);
    let allocated = OUTSTANDING
        .load(Ordering::Relaxed)
        .saturating_sub(allocated_before);
    let charged = Budget::process().usage().0.retained.saturating_sub(before);
    eprintln!("QUERY_OVERLAY allocated={allocated} charged={charged}");
    assert!(
        allocated.saturating_sub(charged) < 1024 * 1024,
        "computed field overlays bypass retained admission"
    );
    drop(result);
    drop(index);
    assert_eq!(Budget::process().usage().0.retained, 0);
}
