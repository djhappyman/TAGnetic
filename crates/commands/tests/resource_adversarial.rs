//! Review probes compile the exact private encoding pool source, without
//! changing its production visibility or adding a test-only production hook.
#[path = "../src/cover_encoding.rs"]
mod encoding_pool;

use std::{
    alloc::{GlobalAlloc, Layout, System},
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tagrex_core::resources::Budget;

struct OutstandingAllocator;
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);
// SAFETY: every allocation operation delegates to System with the caller's
// exact layout/pointer. The counter only observes successful allocation sizes;
// it never changes pointers, lifetimes, alignment or allocation behaviour.
unsafe impl GlobalAlloc for OutstandingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded GlobalAlloc caller contract is unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            OUTSTANDING.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded GlobalAlloc caller contract is unchanged.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            OUTSTANDING.fetch_add(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer/layout are forwarded exactly to the allocator that
        // created the allocation, as required by the GlobalAlloc caller.
        unsafe { System.dealloc(pointer, layout) };
        OUTSTANDING.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: forwarded GlobalAlloc caller contract is unchanged.
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
static ALLOCATOR: OutstandingAllocator = OutstandingAllocator;

#[test]
fn dead_encoding_weak_entries_do_not_retain_large_uncharged_allocations() {
    let name = "dead_encoding_weak_entries_do_not_retain_large_uncharged_allocations";
    if std::env::var("TAGREX_ENCODING_REVIEW_CHILD").as_deref() != Ok("1") {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env("TAGREX_ENCODING_REVIEW_CHILD", "1")
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
    let budget = Budget::process();
    let charged_before = budget.usage().0.retained;
    let allocated_before = OUTSTANDING.load(Ordering::Relaxed);
    {
        let _scope = encoding_pool::Scope::new();
        for index in 0..64u64 {
            let mut bytes = vec![0u8; 1024 * 1024];
            bytes[..8].copy_from_slice(&index.to_le_bytes());
            let bytes: Arc<[u8]> = bytes.into();
            let encoded = encoding_pool::encode(&bytes);
            // No caller keeps old results alive. Only this request's internal
            // hot cache and weak bookkeeping can retain previous allocations.
            assert!(encoded.len() > bytes.len());
        }
        let allocated = OUTSTANDING
            .load(Ordering::Relaxed)
            .saturating_sub(allocated_before);
        let charged = budget.usage().0.retained.saturating_sub(charged_before);
        println!(
            "RESOURCE_ENCODING_REVIEW allocated={allocated} charged={charged} excess={}",
            allocated.saturating_sub(charged)
        );
        // Permit one cadence of raw identity bookkeeping and generous metadata
        // overhead. Dead 1.33-MiB base64 strings must not grow with the entire
        // request after the 32-MiB hot cache has evicted their strong owners.
        assert!(allocated.saturating_sub(charged) < 24 * 1024 * 1024,
            "dead weak strings retained unbounded encoded payload outside the participating cache charge");
    }
    assert_eq!(budget.usage().0.retained, charged_before);
}

#[test]
fn field_recipe_rejects_a_policy_change_between_files() {
    use std::{cell::Cell, path::PathBuf, sync::mpsc, time::Duration};
    use tagrex_commands::{App, FieldOperation};
    use tagrex_core::model::{set_multi_value_separator, TagEngine, TagField};
    let name = "field_recipe_rejects_a_policy_change_between_files";
    if std::env::var("TAGREX_POLICY_REVIEW_CHILD").as_deref() != Ok("1") {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env("TAGREX_POLICY_REVIEW_CHILD", "1")
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
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        std::env::temp_dir().join(format!("resource-policy-review-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let fixture = Fixture(directory);
    set_multi_value_separator("; ");
    // Tiny owned PCM fixture: TagEngine's existing WAV ID3 writer supplies the
    // physical repeated artist values used by the shared recipe read path.
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend(356u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(320u32.to_le_bytes());
    wav.extend([0; 320]);
    let paths: Vec<_> = (0..2)
        .map(|index| {
            let path = fixture.0.join(format!("{index}.wav"));
            std::fs::write(&path, &wav).unwrap();
            let mut track = TagEngine::read(&path).unwrap();
            track.tags.insert(TagField::Artist, "First; Second".into());
            TagEngine::write(&track).unwrap();
            path
        })
        .collect();
    let app = App::open(&fixture.0, &fixture.0.join("journal.sqlite")).unwrap();
    let baseline: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let (begin, received) = mpsc::channel();
    let (done, changed) = mpsc::channel();
    let probes = Cell::new(0);
    let result = std::thread::scope(|scope| {
        scope.spawn(move || {
            received.recv_timeout(Duration::from_secs(5)).unwrap();
            set_multi_value_separator(" / ");
            done.send(()).unwrap();
        });
        app.preview_field_operations_with_cancel(
            &paths,
            &[FieldOperation::Format {
                field: "comment".into(),
                pattern: "%artist%".into(),
            }],
            None,
            || {
                probes.set(probes.get() + 1);
                if probes.get() == 3 {
                    begin.send(()).unwrap();
                    changed.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                false
            },
        )
    });
    for (path, bytes) in paths.iter().zip(baseline) {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    if let Ok(plan) = &result {
        println!(
            "POLICY_RECIPE_REVIEW {}",
            serde_json::to_string(&plan.changes).unwrap()
        );
    }
    assert!(
        result.is_err(),
        "one native recipe accepted values rendered under two different metadata policies"
    );
    // Exercise the remaining publication windows: between steps on one file,
    // and at the final callback after all file values have been computed.
    for (step_count, separator) in [(2, " | "), (1, " :: ")] {
        let probes = Cell::new(0);
        let before = std::fs::read(&paths[0]).unwrap();
        let operations: Vec<_> = (0..step_count)
            .map(|_| FieldOperation::Format {
                field: "comment".into(),
                pattern: "%artist%".into(),
            })
            .collect();
        let result =
            app.preview_field_operations_with_cancel(&paths[..1], &operations, None, || {
                probes.set(probes.get() + 1);
                if probes.get() == 3 {
                    set_multi_value_separator(separator);
                }
                false
            });
        assert!(
            result.is_err(),
            "policy change at step/final publication was accepted"
        );
        assert_eq!(std::fs::read(&paths[0]).unwrap(), before);
    }
    assert!(app
        .preview_field_operations(
            &paths,
            &[FieldOperation::Format {
                field: "comment".into(),
                pattern: "%artist%".into(),
            }],
            None
        )
        .is_ok());
}

#[test]
fn command_cover_preview_reports_unchecked_size_under_pressure() {
    use base64::Engine as _;
    use tagrex_commands::{App, SettingsDto};
    const CASE: &str = "command_cover_preview_reports_unchecked_size_under_pressure";
    if std::env::var("TAGREX_COVER_WARNING_REVIEW_CHILD").as_deref() != Ok("1") {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CASE, "--nocapture"])
            .env("TAGREX_COVER_WARNING_REVIEW_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "resource-cover-warning-review-{}",
        std::process::id()
    )));
    std::fs::create_dir(&fixture.0).unwrap();
    let path = fixture.0.join("fixture.wav");
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend(356u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(320u32.to_le_bytes());
    wav.extend([0; 320]);
    std::fs::write(&path, &wav).unwrap();
    // Owned synthetic 200x100 RGB PNG: zlib-compressed constant-colour rows.
    let image = base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAMgAAABkCAIAAABM5OhcAAABG0lEQVR4nO3SsQkAIADAMGdnZ/8/0ycsggRyQIeOuTZcN54X8CVjkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWCWORMBYJY5EwFgljkTAWiQO2dFCPlEvOTgAAAABJRU5ErkJggg=="
    ).unwrap();
    let image_path = fixture.0.join("cover.png");
    std::fs::write(&image_path, &image).unwrap();
    let app = App::open(&fixture.0, &fixture.0.join("journal.sqlite")).unwrap();
    app.apply_settings(&SettingsDto {
        cover_max_px: 100,
        ..SettingsDto::default()
    });
    let occupied = Budget::process()
        .try_acquire(tagrex_core::resources::Charge {
            jobs: 8,
            ..Default::default()
        })
        .unwrap();
    let paths = std::slice::from_ref(&path);
    let prepared = app
        .cover_embed_options(paths)
        .unwrap()
        .read_image(&image_path)
        .unwrap();
    let plan = app.preview_prepared_cover_embed(paths, prepared).unwrap();
    assert!(plan
        .notes
        .iter()
        .any(|note| note.code == "plan.coverSizeUncheckedBusy"));
    assert!(!plan
        .notes
        .iter()
        .any(|note| note.code == "plan.coverNotShrunk"));
    assert_eq!(std::fs::read(&path).unwrap(), wav);
    assert_eq!(std::fs::read(&image_path).unwrap(), image);
    drop(occupied);
    let prepared = app
        .cover_embed_options(paths)
        .unwrap()
        .read_image(&image_path)
        .unwrap();
    let retried = app.preview_prepared_cover_embed(paths, prepared).unwrap();
    assert!(retried.notes.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), wav);
}
