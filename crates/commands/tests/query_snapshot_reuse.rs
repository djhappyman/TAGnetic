//! Read-only refreshes retain valid derived state; source changes invalidate it.
use std::{path::PathBuf, sync::Arc};
use tagrex_commands::App;

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn unchanged_metadata_read_reuses_the_query_snapshot() {
    let fixture = Fixture(
        std::env::temp_dir().join(format!("tagrex-performance-audit-{}", std::process::id())),
    );
    std::fs::create_dir(&fixture.0).unwrap();
    let path = fixture.0.join("unchanged.wav");
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
    std::fs::write(&path, wav).unwrap();
    let app = App::open(&fixture.0, &fixture.0.join("journal.sqlite")).unwrap();
    let before = app.query_snapshot().unwrap();
    assert!(before.is_current());
    let bytes = std::fs::read(&path).unwrap();
    app.read_track_rows(std::slice::from_ref(&path)).unwrap();
    let after = app.query_snapshot().unwrap();
    assert!(Arc::ptr_eq(&before, &after));
    assert!(before.is_current());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    // Mutating only the caller's returned row cannot corrupt the cached row.
    let mut rows = app.read_track_rows(std::slice::from_ref(&path)).unwrap();
    rows[0].tags.insert("title".into(), "Local only".into());
    assert!(Arc::ptr_eq(&before, &app.query_snapshot().unwrap()));
    #[cfg(unix)]
    {
        // Same bytes and mtime, but a real filesystem change: preserve ctime
        // evidence so audio/private-tag changes cannot keep duplicate results.
        let old = std::fs::metadata(&path).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old.modified().unwrap()))
            .unwrap();
        app.read_track_rows(std::slice::from_ref(&path)).unwrap();
        assert!(!before.is_current());
        assert!(!Arc::ptr_eq(&before, &app.query_snapshot().unwrap()));
    }
}
