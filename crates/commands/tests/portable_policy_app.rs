//! Real App/SQLite coverage for the portable writer, independent of native FLAC.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tagrex_commands::{App, TagEditDto};
use tagrex_core::model::{self, TagEngine, TagField};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tagrex-portable-policy-app-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wave(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(356u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(16000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(320u32.to_le_bytes());
        bytes.extend([0x31; 320]);
        std::fs::write(&path, bytes).unwrap();
        let mut file = TagEngine::read(&path).unwrap();
        file.tags.insert(TagField::Title, "Before".into());
        TagEngine::write(&file).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn portable_wav_app_refuses_dirty_callbacks_restores_and_retries_with_undo_redo() {
    const CHILD: &str = "TAGREX_PORTABLE_APP_POLICY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "portable_wav_app_refuses_dirty_callbacks_restores_and_retries_with_undo_redo",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(result.success());
        return;
    }
    for after_prefix in [false, true] {
        let fixture = Fixture::new();
        let paths = [fixture.wave("a.wav"), fixture.wave("b.wav")];
        let original = paths.each_ref().map(|path| std::fs::read(path).unwrap());
        let mut app = App::open(&fixture.0, &fixture.0.join("journal.sqlite")).unwrap();
        let edits = paths
            .iter()
            .map(|path| TagEditDto {
                path: path.to_string_lossy().into_owned(),
                field: "title".into(),
                value: Some("After".into()),
            })
            .collect::<Vec<_>>();
        let plan = app.preview_tag_edits(&edits).unwrap();
        let staged = serde_json::to_value(&plan).unwrap();
        let _before_query = app.query_snapshot();
        let mut fired = false;
        let result = app.apply_with(
            &plan,
            &mut |_| {
                let reached = TagEngine::read(&paths[0]).unwrap().tags[&TagField::Title] == "After";
                if !fired && (!after_prefix || reached) {
                    fired = true;
                    model::set_multi_value_separator(" | ");
                }
            },
            &AtomicBool::new(false),
        );
        assert!(fired);
        assert!(
            result.is_err(),
            "dirty portable App accepted, prefix={after_prefix}"
        );
        assert!(app.history().unwrap().is_empty());
        assert_eq!(serde_json::to_value(&plan).unwrap(), staged);
        for (path, bytes) in paths.iter().zip(&original) {
            assert_eq!(&std::fs::read(path).unwrap(), bytes);
            assert_eq!(app.read_track(path).unwrap().tags["title"], "Before");
        }
        model::set_multi_value_separator("");
        let applied = app.apply(&plan).unwrap();
        for path in &paths {
            assert_eq!(app.read_track(path).unwrap().tags["title"], "After");
        }
        app.undo(applied.id).unwrap();
        let redone = app.redo_with(&mut |_| {}).unwrap().unwrap();
        app.undo(redone.id).unwrap();
        assert!(app.history().unwrap().is_empty());
        for (path, bytes) in paths.iter().zip(&original) {
            assert_eq!(&std::fs::read(path).unwrap(), bytes);
        }
    }
}
