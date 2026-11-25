use super::*;
use crate::output::ownership::{set_observer, Event};
use crate::output::AudioSet;
use std::fs;
use std::time::{Duration, SystemTime};

fn fixture_dir() -> tempfile::TempDir {
    match std::env::var_os("TAGREX_OUTPUT_TEST_ROOT") {
        Some(path) => tempfile::tempdir_in(path).unwrap(),
        None => tempfile::tempdir().unwrap(),
    }
}

fn wav(path: &Path, seed: u16) -> Vec<u8> {
    const SAMPLES: u32 = 32;
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + SAMPLES * 2).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16_u32.to_le_bytes());
    wav.extend([1, 0, 1, 0]);
    wav.extend(44_100_u32.to_le_bytes());
    wav.extend(88_200_u32.to_le_bytes());
    wav.extend([2, 0, 16, 0]);
    wav.extend(b"data");
    wav.extend((SAMPLES * 2).to_le_bytes());
    for sample in 0..SAMPLES as u16 {
        wav.extend((sample * seed).to_le_bytes());
    }
    fs::write(path, &wav).unwrap();
    wav
}

struct Files {
    _root: tempfile::TempDir,
    recovery: PathBuf,
    _recovery_volume: Option<tempfile::TempDir>,
    original: PathBuf,
    outputs: Vec<PathBuf>,
    original_bytes: Vec<u8>,
    output_bytes: Vec<Vec<u8>>,
}

impl Files {
    fn new() -> Option<Self> {
        let root = fixture_dir();
        let path = root.path().canonicalize().unwrap();
        let recovery_volume = std::env::var_os("TAGREX_AUDIO_TEST_RECOVERY_VOLUME")
            .map(|path| tempfile::tempdir_in(path).unwrap());
        let recovery = recovery_volume
            .as_ref()
            .map_or(path.as_path(), tempfile::TempDir::path)
            .canonicalize()
            .unwrap()
            .join("recovery");
        let library = path.join("library");
        fs::create_dir(&recovery).unwrap();
        fs::create_dir(&library).unwrap();
        #[cfg(unix)]
        if recovery_volume.is_some() {
            use std::os::unix::fs::MetadataExt;
            assert_ne!(
                fs::metadata(&recovery).unwrap().dev(),
                fs::metadata(&library).unwrap().dev(),
                "the recovery fixture must be on a different volume"
            );
        }
        if let Err(error) = directory(&library).and_then(|root| root.ensure_exclusive_rename()) {
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            return None;
        }
        let original = library.join("Tape master.wav");
        let outputs = vec![library.join("01.wav"), library.join("02.wav")];
        let original_bytes = wav(&original, 5);
        let output_bytes = outputs
            .iter()
            .enumerate()
            .map(|(index, path)| wav(path, (index + 1) as u16))
            .collect();
        Some(Self {
            _root: root,
            recovery,
            _recovery_volume: recovery_volume,
            original,
            outputs,
            original_bytes,
            output_bytes,
        })
    }

    fn plan(&self) -> AudioReplacement {
        AudioReplacement::plan(&self.recovery, std::slice::from_ref(&self.original)).unwrap()
    }

    fn witnesses(&self) -> Vec<(PathBuf, AudioWitness)> {
        self.outputs
            .iter()
            .map(|path| {
                (
                    path.clone(),
                    AudioSet::witness(&File::open(path).unwrap()).unwrap(),
                )
            })
            .collect()
    }
}

#[test]
fn replacement_archives_original_and_undo_restores_bytes_and_attributes() {
    let Some(fixture) = Files::new() else { return };
    let original_file = std::fs::OpenOptions::new()
        .write(true)
        .open(&fixture.original)
        .unwrap();
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    original_file
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        original_file
            .set_permissions(fs::Permissions::from_mode(0o640))
            .unwrap();
    }
    drop(original_file);
    let plan = fixture.plan();
    let backup = plan.backups().next().unwrap().1.to_owned();
    assert!(!backup.exists());
    assert!(fs::read_dir(&fixture.recovery).unwrap().next().is_none());

    let completed = plan.commit(&fixture.witnesses(), || {}).unwrap();
    assert!(completed.retained_paths().is_empty());
    assert!(!fixture.original.exists());
    assert_eq!(fs::read(&backup).unwrap(), fixture.original_bytes);
    let backup_folder = backup.parent().unwrap();
    let manifest: serde_json::Value =
        serde_json::from_reader(File::open(backup_folder.join("recovery.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["files"][0]["original"],
        fixture.original.to_str().unwrap()
    );
    assert_eq!(manifest["files"][0]["saved"], backup.to_str().unwrap());
    assert_eq!(
        manifest["files"][0]["sha256"],
        hex(&Sha256::digest(&fixture.original_bytes))
    );

    assert!(completed.undo().unwrap().is_empty());
    assert!(completed.undo().unwrap().is_empty());
    assert_eq!(fs::read(&fixture.original).unwrap(), fixture.original_bytes);
    assert_eq!(
        fs::metadata(&fixture.original).unwrap().modified().unwrap(),
        modified
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&fixture.original)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }
    let archive = fs::read_dir(backup_folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir())
        .unwrap();
    for (index, output) in fixture.outputs.iter().enumerate() {
        assert!(!output.exists());
        assert_eq!(
            fs::read(archive.join(numbered_name(index, output))).unwrap(),
            fixture.output_bytes[index]
        );
    }
}

#[test]
fn replacement_rejects_switched_output_and_stale_original_without_side_effects() {
    let Some(fixture) = Files::new() else { return };
    let plan = fixture.plan();
    let witnesses = fixture.witnesses();
    let moved = fixture.outputs[0].with_extension("prior");
    fs::rename(&fixture.outputs[0], &moved).unwrap();
    let foreign = wav(&fixture.outputs[0], 19);
    assert!(plan.commit(&witnesses, || {}).is_err());
    assert_eq!(fs::read(&fixture.outputs[0]).unwrap(), foreign);
    assert_eq!(fs::read(&moved).unwrap(), fixture.output_bytes[0]);
    assert_eq!(fs::read(&fixture.original).unwrap(), fixture.original_bytes);

    let plan = fixture.plan();
    let changed = wav(&fixture.original, 20);
    assert!(plan.commit(&fixture.witnesses(), || {}).is_err());
    assert_eq!(fs::read(&fixture.original).unwrap(), changed);
    assert!(fs::read_dir(&fixture.recovery).unwrap().next().is_none());
}

#[test]
fn backup_folder_collision_never_retires_the_original() {
    let Some(fixture) = Files::new() else { return };
    let plan = fixture.plan();
    let backup_dir = plan
        .backups()
        .next()
        .unwrap()
        .1
        .parent()
        .unwrap()
        .to_owned();
    fs::create_dir(&backup_dir).unwrap();
    fs::write(backup_dir.join("foreign.txt"), "keep me").unwrap();
    assert!(plan.commit(&fixture.witnesses(), || {}).is_err());
    assert_eq!(fs::read(&fixture.original).unwrap(), fixture.original_bytes);
    assert_eq!(
        fs::read_to_string(backup_dir.join("foreign.txt")).unwrap(),
        "keep me"
    );
}

#[test]
fn undo_rejects_collisions_and_changed_outputs() {
    let Some(fixture) = Files::new() else { return };
    let completed = fixture.plan().commit(&fixture.witnesses(), || {}).unwrap();
    let foreign = wav(&fixture.original, 16);
    assert_eq!(
        completed.undo().unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&fixture.original).unwrap(), foreign);
    assert_eq!(
        fs::read(&fixture.outputs[0]).unwrap(),
        fixture.output_bytes[0]
    );
    fs::remove_file(&fixture.original).unwrap();
    let changed = wav(&fixture.outputs[0], 21);
    assert!(completed.undo().is_err());
    assert!(!fixture.original.exists());
    assert_eq!(fs::read(&fixture.outputs[0]).unwrap(), changed);
    assert_eq!(
        fs::read(&fixture.outputs[1]).unwrap(),
        fixture.output_bytes[1]
    );
}

#[test]
fn replacement_and_undo_recover_after_partial_capture_failure() {
    let Some(fixture) = Files::new() else { return };
    let second = fixture.original.with_file_name("Other master.wav");
    let second_bytes = wav(&second, 13);
    let plan = AudioReplacement::plan(
        &fixture.recovery,
        &[fixture.original.clone(), second.clone()],
    )
    .unwrap();
    let originals = plan
        .backups()
        .map(|(_, path)| path.to_owned())
        .collect::<Vec<_>>();
    let fail_at = second.clone();
    set_observer(Some(Box::new(move |event, path| {
        if event == Event::BackupCaptured && path == fail_at {
            return Err(io::Error::other("simulated capture failure"));
        }
        Ok(())
    })));
    let failed = plan.commit(&fixture.witnesses(), || {});
    set_observer(None);
    assert!(failed.is_err());
    assert_eq!(fs::read(&fixture.original).unwrap(), fixture.original_bytes);
    assert_eq!(fs::read(&second).unwrap(), second_bytes);
    assert_eq!(fs::read(&originals[0]).unwrap(), fixture.original_bytes);
    assert_eq!(fs::read(&originals[1]).unwrap(), second_bytes);

    let completed = fixture.plan().commit(&fixture.witnesses(), || {}).unwrap();
    let fail_at = fixture.outputs[1].clone();
    set_observer(Some(Box::new(move |event, path| {
        if event == Event::BackupCaptured && path == fail_at {
            return Err(io::Error::other("simulated capture failure"));
        }
        Ok(())
    })));
    let failed = completed.undo();
    set_observer(None);
    assert!(failed.is_err());
    assert_eq!(fs::read(&fixture.original).unwrap(), fixture.original_bytes);
    for (index, output) in fixture.outputs.iter().enumerate() {
        assert_eq!(fs::read(output).unwrap(), fixture.output_bytes[index]);
    }
    assert!(completed.undo().unwrap().is_empty());
    assert_eq!(fs::read(&fixture.original).unwrap(), fixture.original_bytes);
    assert!(fixture.outputs.iter().all(|path| !path.exists()));
}
