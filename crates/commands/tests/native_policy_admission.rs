#![cfg(target_os = "linux")]
//! Real concurrent App sessions exercise the private nested native IO admission.
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use tagrex_commands::{App, SettingsDto, TagEditDto};
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn flac() -> Vec<u8> {
    let mut out = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    out.extend(info);
    let mut comments = 0u32.to_le_bytes().to_vec();
    comments.extend(3u32.to_le_bytes());
    for item in [
        "TITLE=Before",
        "ARTIST=Original",
        "UNTOUCHED=literal; value",
    ] {
        comments.extend((item.len() as u32).to_le_bytes());
        comments.extend(item.as_bytes());
    }
    let n = comments.len();
    out.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    out.extend(comments);
    out.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    out
}
fn run(
    root: &Path,
    index: usize,
    arrived: mpsc::Sender<usize>,
    released: Arc<AtomicBool>,
) -> Result<(), String> {
    let path = root.join("track.flac");
    std::fs::write(&path, flac()).map_err(|e| e.to_string())?;
    let mut app = App::open(root, &root.join("journal.sqlite")).map_err(|e| e.to_string())?;
    let sep = if index.is_multiple_of(2) {
        " | "
    } else {
        " / "
    };
    app.apply_settings(&SettingsDto {
        id3_v23: index.is_multiple_of(2),
        multi_value_separator: sep.into(),
        read_priority: vec!["vorbis".into()],
        legacy_code_page: if index.is_multiple_of(2) {
            "windows-1251".into()
        } else {
            String::new()
        },
        ..SettingsDto::default()
    });
    let desired = format!("One{sep}Two");
    let mut waiting = true;
    for _ in 0..3 {
        let plan = app
            .preview_tag_edits(&[TagEditDto {
                path: path.to_string_lossy().into_owned(),
                field: "artist".into(),
                value: Some(desired.clone()),
            }])
            .map_err(|e| e.to_string())?;
        let batch = app
            .apply_with(
                &plan,
                &mut |_| {
                    if waiting {
                        waiting = false;
                        let _ = arrived.send(index);
                        let started = Instant::now();
                        while !released.load(Ordering::Acquire)
                            && started.elapsed() < Duration::from_secs(5)
                        {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                },
                &AtomicBool::new(false),
            )
            .map_err(|e| e.to_string())?;
        assert_eq!(app.read_track(&path).unwrap().tags["artist"], desired);
        app.undo(batch.id).map_err(|e| e.to_string())?;
        assert_eq!(app.read_track(&path).unwrap().tags["artist"], "Original");
        let redo = app
            .redo_with(&mut |_| {})
            .map_err(|e| e.to_string())?
            .ok_or("missing Redo")?;
        assert_eq!(app.read_track(&path).unwrap().tags["artist"], desired);
        app.undo(redo.id).map_err(|e| e.to_string())?;
        assert!(app.history().map_err(|e| e.to_string())?.is_empty());
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        assert_eq!(&bytes[bytes.len() - 14..], &flac()[flac().len() - 14..]);
        assert!(bytes
            .windows(b"UNTOUCHED=literal; value".len())
            .any(|b| b == b"UNTOUCHED=literal; value"));
    }
    Ok(())
}
#[test]
fn eight_apps_with_custom_policies_complete_native_apply_undo_redo_and_release_owners() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = Fixture(std::env::temp_dir().join(format!(
        "tagrex-native-policy-admission-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&root.0).unwrap();
    let before = tagrex_core::resources::Budget::process().usage().0;
    let released = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let mut workers = Vec::new();
    for index in 0..8 {
        let path = root.0.join(index.to_string());
        std::fs::create_dir(&path).unwrap();
        let tx = tx.clone();
        let release = released.clone();
        workers.push(std::thread::spawn(move || run(&path, index, tx, release)));
    }
    drop(tx);
    let mut arrivals = std::collections::HashSet::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while arrivals.len() < 8 {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match rx.recv_timeout(left) {
            Ok(i) => {
                arrivals.insert(i);
            }
            Err(_) => break,
        }
    }
    released.store(true, Ordering::Release);
    let results = workers.into_iter().map(|w| w.join()).collect::<Vec<_>>();
    assert_eq!(
        arrivals.len(),
        8,
        "every public operation must reach the controlled rendezvous: {results:?}"
    );
    for result in results {
        result.unwrap().unwrap();
    }
    assert_eq!(tagrex_core::resources::Budget::process().usage().0, before);
}
