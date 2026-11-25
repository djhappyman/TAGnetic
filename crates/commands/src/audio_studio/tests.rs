use super::*;
use base64::Engine;
use std::fs;
use std::process::Command;

const FRAMES: usize = 49_221;
const CUT: usize = 11_032;

fn fixtures() -> tempfile::TempDir {
    let dir = std::env::var_os("TAGREX_OUTPUT_TEST_ROOT");
    match dir {
        Some(dir) => tempfile::tempdir_in(dir).unwrap(),
        None => tempfile::tempdir().unwrap(),
    }
}

fn recording(path: &Path) -> Vec<u8> {
    let mut pcm = Vec::new();
    for sample in 0..FRAMES {
        pcm.extend_from_slice(&(((sample * 83 % 16_000) as i16) - 8000).to_le_bytes());
        pcm.extend_from_slice(&(((sample * 113 % 18_000) as i16) - 9000).to_le_bytes());
    }
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&[16, 0, 0, 0, 1, 0, 2, 0]);
    wav.extend_from_slice(&44_100_u32.to_le_bytes());
    wav.extend_from_slice(&176_400_u32.to_le_bytes());
    wav.extend_from_slice(&[4, 0, 16, 0]);
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav.extend_from_slice(&pcm);
    fs::write(path, wav).unwrap();
    pcm
}

fn request(root: &Path, source: &Path, name: &str, format: &str) -> ReviewRequest {
    ReviewRequest {
        sources: vec![source.to_str().unwrap().into()],
        session_id: None,
        segments: Vec::new(),
        tags: [
            ("album".into(), "Tape pack".into()),
            ("artist".into(), "Various".into()),
        ]
        .into(),
        destination: root.to_str().unwrap().into(),
        basename: name.into(),
        folder_pattern: None,
        format: format.into(),
        bitrate: Some(128),
        split_method: None,
        treatment: AudioTreatment::default(),
        name_pattern: None,
        artwork_path: None,
        save_mode: SaveMode::Folder,
        backup_directory: None,
    }
}

fn segment(start_frame: u64, title: &str) -> DraftSegment {
    DraftSegment {
        start_frame,
        include: true,
        tags: [("title".into(), title.into())].into(),
        ..Default::default()
    }
}

fn ffmpeg_pcm(path: &Path) -> Option<Vec<u8>> {
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-xerror", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "s16le", "-"])
        .output();
    match result {
        Ok(output) => {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            Some(output.stdout)
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && std::env::var_os("TAGREX_REQUIRE_REFERENCE_CODECS").is_none() =>
        {
            None
        }
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn audio_conversions_save_beside_each_source_and_never_overwrite() {
    let root = fixtures();
    let music = root.path().canonicalize().unwrap();
    let first = music.join("Side A");
    let second = music.join("Side B");
    for parent in [&first, &second] {
        fs::create_dir(parent).unwrap();
        recording(&parent.join("master.wav"));
    }
    let wavs = [first.join("master.wav"), second.join("master.wav")];
    let outputs = [first.join("master.flac"), second.join("master.flac")];
    let studio = Studio::default();
    let beside = || {
        let mut draft = request(&music, &wavs[0], "", "flac");
        draft.sources = wavs
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        draft.destination.clear();
        draft.save_mode = SaveMode::Beside;
        draft
    };
    let review = studio.preview(beside()).unwrap();
    assert_eq!(review.outputs.len(), outputs.len());
    assert!(review
        .outputs
        .iter()
        .zip(&outputs)
        .all(|(row, path)| Path::new(&row.path) == path && !path.exists()));
    let converted = studio.export(&review.id).unwrap();
    assert!(converted.replaced.is_empty());
    assert!(wavs.iter().chain(&outputs).all(|path| path.is_file()));

    let mut collision = beside();
    collision.name_pattern = Some("Other name".to_owned());
    let review = studio.preview(collision).unwrap();
    let competing = Path::new(&review.outputs[1].path);
    fs::write(competing, b"another file").unwrap();
    assert!(studio.export(&review.id).is_err());
    assert_eq!(fs::read(competing).unwrap(), b"another file");
    assert!(wavs.iter().all(|path| path.is_file()));
}

#[test]
fn split_saves_in_place_unless_a_folder_pattern_is_requested() {
    let root = fixtures();
    let music = root.path().canonicalize().unwrap();
    let wav = music.join("master.wav");
    recording(&wav);
    let studio = Studio::default();
    let source = studio.open(&wav, || false).unwrap();
    let mut draft = request(&music, &wav, "", "flac");
    draft.save_mode = SaveMode::Beside;
    draft.session_id = Some(source.id.clone());
    draft.segments = vec![segment(0, "Opening"), segment(CUT as u64, "Second track")];

    let flat = studio.preview(draft).unwrap();
    assert_eq!(
        Path::new(&flat.outputs[0].path),
        music.join("01 - Opening.flac")
    );
    studio
        .with_review_playback(&flat.id, 1, |owner, _, rate, range, _, _| {
            assert_eq!(owner, source.id);
            assert_eq!(rate, 44_100);
            assert_eq!(range.start, CUT as u64);
            assert_eq!(range.end, FRAMES as u64);
            Ok(())
        })
        .unwrap();
    assert!(!Path::new(&flat.outputs[0].path).exists());
    studio.export(&flat.id).unwrap();
    assert!(Path::new(&flat.outputs[0].path).is_file());
    assert!(wav.is_file());
    assert_eq!(fs::read_dir(&music).unwrap().count(), 3);

    let mut draft = request(&music, &wav, "", "flac");
    draft.session_id = Some(source.id);
    draft.segments = vec![segment(0, "Opening"), segment(CUT as u64, "Second track")];
    draft.folder_pattern = Some("%album% - Splits".to_owned());
    draft.name_pattern = Some("%track:2% - %title%".to_owned());
    let grouped = studio.preview(draft).unwrap();
    let folder = music.join("Tape pack - Splits");
    assert_eq!(
        Path::new(&grouped.outputs[1].path),
        folder.join("02 - Second track.flac")
    );
    assert!(!folder.exists());
    studio.export(&grouped.id).unwrap();
    assert!(Path::new(&grouped.outputs[1].path).is_file());
}

#[test]
fn studio_replacement_keeps_a_verified_original_and_undo_is_non_clobbering() {
    for folder in ["", "Master - Tracks"] {
        check_replacement_and_undo(folder);
    }
}

fn check_replacement_and_undo(folder: &str) {
    let root = fixtures();
    let base = root.path().canonicalize().unwrap();
    let music = base.join("Library");
    let backup = base.join("Recoverable");
    fs::create_dir(&music).unwrap();
    fs::create_dir(&backup).unwrap();
    let wav = music.join("master.wav");
    let original_pcm = recording(&wav);
    let original = fs::read(&wav).unwrap();
    let studio = Studio::default();
    let source = studio.open(&wav, || false).unwrap();
    let replace = || {
        let mut draft = request(&music, &wav, folder, "flac");
        draft.session_id = Some(source.id.clone());
        draft.segments = vec![segment(0, "Opening"), segment(CUT as u64, "Second track")];
        draft.destination.clear();
        draft.save_mode = SaveMode::Replace;
        draft
    };
    assert!(studio
        .preview_with_recovery(replace(), Some(&music), Some(&music))
        .is_err());
    let review = studio
        .preview_with_recovery(replace(), Some(&backup), Some(&music))
        .unwrap();
    assert_eq!(review.backups[0].source, wav.to_string_lossy());
    let retained = Path::new(&review.backups[0].path);
    assert!(retained.starts_with(&backup));
    assert!(!retained.exists());
    assert_eq!(fs::read(&wav).unwrap(), original);

    let saved = studio
        .export_with_library(&review.id, Some(&music))
        .unwrap();
    assert_eq!(saved.paths.len(), 2);
    assert!(saved
        .paths
        .iter()
        .all(|path| Path::new(path).parent() == Some(music.join(folder).as_path())));
    assert_eq!(saved.replaced, [wav.to_string_lossy()]);
    assert_eq!(saved.backups, [retained.to_string_lossy()]);
    assert!(!wav.exists());
    assert_eq!(fs::read(retained).unwrap(), original);
    assert!(retained.parent().unwrap().join("recovery.json").is_file());
    let pcm: Option<Vec<_>> = saved
        .paths
        .iter()
        .map(|path| ffmpeg_pcm(Path::new(path)))
        .collect();
    if let Some(pcm) = pcm {
        assert_eq!(pcm.concat(), original_pcm);
    }

    fs::write(&wav, b"new owner").unwrap();
    assert!(studio.undo_replacement(&review.id).is_err());
    assert_eq!(fs::read(&wav).unwrap(), b"new owner");
    fs::remove_file(&wav).unwrap();
    let restored = studio.undo_replacement(&review.id).unwrap();
    assert_eq!(restored.paths, [wav.to_string_lossy()]);
    assert_eq!(restored.removed, saved.paths);
    assert!(restored.warnings.is_empty());
    assert_eq!(fs::read(&wav).unwrap(), original);
    assert!(restored
        .removed
        .iter()
        .all(|path| !Path::new(path).exists()));
    assert!(!music.join("Master - Tracks").exists());
    assert_eq!(fs::read_dir(&music).unwrap().count(), 1);
    assert_eq!(fs::read(retained).unwrap(), original);
}

#[test]
fn studio_exports_reviewed_pcm_and_mp3_with_tags_and_artwork() {
    let root = fixtures();
    let wav = root.path().join("master.wav");
    let original_pcm = recording(&wav);
    let original = fs::read(&wav).unwrap();
    let cover = root.path().join("cover.png");
    let cover_bytes = base64::engine::general_purpose::STANDARD.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAEElEQVR4nGP4z8AARwzEcQCukw/x0F8jngAAAABJRU5ErkJggg=="
    ).unwrap();
    fs::write(&cover, &cover_bytes).unwrap();
    let studio = Studio::default();
    let source = studio.open(&wav, || false).unwrap();
    assert_eq!(source.frames, FRAMES as u64);
    for format in ["flac", "mp3"] {
        let mut draft = request(root.path(), &wav, format, format);
        draft.session_id = Some(source.id.clone());
        draft.segments = vec![segment(0, "Opening"), segment(CUT as u64, "Next / track")];
        draft.artwork_path = Some(cover.to_str().unwrap().into());
        let plan = studio.preview(draft).unwrap();
        assert_eq!(plan.outputs.len(), 2);
        assert!(!root.path().join(format).exists());
        assert_eq!(plan.outputs[0].end_frame, CUT as u64);
        assert_eq!(plan.outputs[1].tags["title"], "Next / track");
        let result = studio.export(&plan.id).unwrap();
        assert!(studio.export(&plan.id).is_err());
        assert_eq!(studio.progress(&plan.id).unwrap().completed, 2);
        assert_eq!(
            studio.published_folder(&plan.id).unwrap(),
            root.path().join(format).canonicalize().unwrap()
        );
        let mut joined = Vec::new();
        for (index, path) in result.paths.iter().map(Path::new).enumerate() {
            let tags = TagEngine::read(path).unwrap().tags;
            assert_eq!(tags[&TagField::TrackNumber], (index + 1).to_string());
            assert_eq!(tags[&TagField::Album], "Tape pack");
            assert_eq!(
                &*TagEngine::read_cover(path).unwrap().unwrap().data,
                cover_bytes.as_slice()
            );
            if let Some(pcm) = ffmpeg_pcm(path) {
                let expected = if index == 0 { CUT } else { FRAMES - CUT };
                assert_eq!(pcm.len(), expected * 4);
                joined.extend(pcm);
            }
        }
        if format == "flac" && !joined.is_empty() {
            assert_eq!(joined, original_pcm);
        }
        assert_eq!(fs::read(&wav).unwrap(), original);
        // Feed the app's MP3 back to the real workflow, including conversion.
        if format == "mp3" {
            let mp3 = Path::new(&result.paths[1]);
            let master = studio.open(mp3, || false).unwrap();
            assert_eq!(master.frames, (FRAMES - CUT) as u64);
            let plan = studio
                .preview(request(root.path(), mp3, "converted", "flac"))
                .unwrap();
            let result = studio.export(&plan.id).unwrap();
            assert_eq!(
                audio::inspect(Path::new(&result.paths[0]), 1, || false)
                    .unwrap()
                    .source
                    .frames,
                master.frames
            );
        }
    }
}

#[test]
fn studio_rejects_source_and_destination_changes_and_prestart_stop() {
    let root = fixtures();
    let wav = root.path().join("master.wav");
    recording(&wav);
    let studio = Studio::default();

    let first = studio
        .preview(request(root.path(), &wav, "occupied", "flac"))
        .unwrap();
    let foreign = root.path().join("occupied");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("sentinel"), b"preserve").unwrap();
    assert!(studio.export(&first.id).is_err());
    assert_eq!(fs::read(foreign.join("sentinel")).unwrap(), b"preserve");

    let stopped = studio
        .preview(request(root.path(), &wav, "stopped", "flac"))
        .unwrap();
    studio.cancel(&stopped.id).unwrap();
    assert!(studio.export(&stopped.id).is_err());
    assert!(!root.path().join("stopped").exists());

    let stale = studio
        .preview(request(root.path(), &wav, "stale", "flac"))
        .unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&wav)
        .unwrap()
        .write_all(b"changed data")
        .unwrap();
    assert!(studio
        .export(&stale.id)
        .unwrap_err()
        .to_string()
        .contains("recording changed"));
    assert!(!root.path().join("stale").exists());
}

#[test]
fn studio_cue_save_preserves_an_imported_first_index_and_refuses_overwrite() {
    let root = fixtures();
    let wav = root.path().join("master.wav");
    recording(&wav);
    let studio = Studio::default();
    let source = studio.open(&wav, || false).unwrap();
    let cue = root.path().join("input.cue");
    let input = b"FILE \"master.wav\" WAVE\n  TRACK 01 AUDIO\n    INDEX 00 00:00:00\n    INDEX 01 00:00:01\n  TRACK 02 AUDIO\n    INDEX 01 00:00:10\n";
    fs::write(&cue, input).unwrap();
    let imported = studio
        .import_cue(&source.id, &cue, CueEncoding::Utf8)
        .unwrap();
    let mut rows = vec![DraftSegment {
        lead_in: true,
        ..segment(0, "Lead-in")
    }];
    rows.extend(imported.markers.into_iter().map(|marker| DraftSegment {
        index00: marker.index00,
        index01: Some(marker.index01),
        pregap_frame: marker.pregap_frame,
        ..segment(marker.frame, "")
    }));
    let exported = root.path().join("output.cue");
    studio
        .save_cue(&source.id, &exported, &rows, &Values::new())
        .unwrap();
    let reparsed = studio
        .import_cue(&source.id, &exported, CueEncoding::Utf8)
        .unwrap();
    assert_eq!(reparsed.markers.len(), 2);
    assert_eq!(reparsed.markers[0].index00, Some(0));
    assert_eq!(reparsed.markers[0].index01, 1);
    assert!(studio
        .save_cue(&source.id, &exported, &rows, &Values::new())
        .is_err());
}

use std::io::Write;

#[test]
fn studio_defaults_to_copying_same_format_and_keeps_exact_as_a_choice() {
    let root = fixtures();
    let wav = root.path().join("master.wav");
    let pcm = recording(&wav);
    let studio = Studio::default();

    for (name, format) in [
        ("flac", OutputFormat::Flac),
        ("mp3", OutputFormat::Mp3 { bitrate_kbps: 128 }),
    ] {
        let path = root.path().join(format!("master.{name}"));
        audio::render(
            &wav,
            &[RenderSegment {
                range: FrameRange {
                    start: 0,
                    end: FRAMES as u64,
                },
                output: path.clone(),
            }],
            format,
            || false,
            |_| {},
        )
        .unwrap();

        let original = fs::read(&path).unwrap();
        let decoded_original = ffmpeg_pcm(&path);
        let source = studio.open(&path, || false).unwrap();
        assert_eq!(source.original_format, Some(name));
        let request_split = |folder: &str, cut: u64, method| {
            let mut draft = request(root.path(), &path, folder, name);
            draft.session_id = Some(source.id.clone());
            draft.segments = vec![segment(0, "Start"), segment(cut, "End")];
            draft.split_method = method;
            draft
        };

        let exact = studio
            .preview(request_split(
                &format!("exact-{name}"),
                CUT as u64,
                Some(SplitMethod::Exact),
            ))
            .unwrap();
        assert_eq!(exact.split_method, SplitMethod::Exact);
        assert_eq!(exact.outputs[0].end_frame, CUT as u64);
        assert!(exact.outputs[0].requested_end_frame.is_none());
        let precise = studio.export(&exact.id).unwrap();
        assert_eq!(
            audio::inspect(Path::new(&precise.paths[0]), 1, || false)
                .unwrap()
                .source
                .frames,
            CUT as u64
        );

        let plan = studio
            .preview(request_split(&format!("copy-{name}"), CUT as u64, None))
            .unwrap();
        assert_eq!(plan.split_method, SplitMethod::Copy);
        let moved = plan.outputs[0].end_frame;
        assert_ne!(moved, CUT as u64);
        assert_eq!(moved, plan.outputs[1].start_frame);
        assert_eq!(plan.outputs[0].requested_end_frame, Some(CUT as u64));
        assert_eq!(plan.outputs[1].requested_start_frame, Some(CUT as u64));
        assert!(plan
            .warnings
            .iter()
            .all(|warning| !warning.contains("encoded again")));
        let result = studio.export(&plan.id).unwrap();
        let mut joined = Vec::new();
        for (position, output) in result.paths.iter().map(Path::new).enumerate() {
            let track = TagEngine::read(output).unwrap();
            assert_eq!(
                track.tags[&TagField::TrackNumber],
                (position + 1).to_string()
            );
            if let Some(decoded) = ffmpeg_pcm(output) {
                let range = &plan.outputs[position];
                assert_eq!(
                    decoded.len(),
                    (range.end_frame - range.start_frame) as usize * 4
                );
                joined.extend(decoded);
            }
        }
        if let Some(ref decoded_original) = decoded_original {
            assert_eq!(
                &joined, decoded_original,
                "{name} copied audio after tagging"
            );
            if name == "flac" {
                assert_eq!(joined, pcm);
            }
        }
        for method in [SplitMethod::Exact, SplitMethod::Copy] {
            let mut trim =
                request_split(&format!("trim-{method:?}-{name}"), CUT as u64, Some(method));
            trim.segments = vec![segment(CUT as u64, "Trimmed track one")];
            let reviewed = studio.preview(trim).unwrap();
            let track = &reviewed.outputs[0];
            let expected_start = if method == SplitMethod::Copy {
                moved
            } else {
                CUT as u64
            };
            assert_eq!(track.start_frame, expected_start);
            assert_eq!(track.end_frame, FRAMES as u64);
            let result = studio.export(&reviewed.id).unwrap();
            let output = Path::new(&result.paths[0]);
            assert_eq!(
                TagEngine::read(output).unwrap().tags[&TagField::TrackNumber],
                "1"
            );
            if method == SplitMethod::Copy {
                if let Some((actual, original)) = ffmpeg_pcm(output).zip(decoded_original.as_ref())
                {
                    assert_eq!(actual, original[expected_start as usize * 4..]);
                }
            } else {
                assert_eq!(
                    audio::inspect(output, 1, || false).unwrap().source.frames,
                    FRAMES as u64 - expected_start
                );
            }
        }
        assert_eq!(fs::read(&path).unwrap(), original);
        studio.close(&source.id).unwrap();
    }
}

#[test]
fn studio_requires_a_real_range_when_cuts_snap_to_the_same_frame() {
    let root = fixtures();
    let wav = root.path().join("master.wav");
    recording(&wav);
    let path = root.path().join("master.flac");
    audio::render(
        &wav,
        &[RenderSegment {
            range: FrameRange {
                start: 0,
                end: FRAMES as u64,
            },
            output: path.clone(),
        }],
        OutputFormat::Flac,
        || false,
        |_| {},
    )
    .unwrap();
    let studio = Studio::default();
    let source = studio.open(&path, || false).unwrap();
    let mut draft = request(root.path(), &path, "overlapping", "flac");
    draft.session_id = Some(source.id);
    draft.segments = vec![segment(0, "Start"), segment(1, "End")];
    let error = studio
        .preview(draft)
        .err()
        .expect("two snapped cuts must not collapse");
    assert!(error.to_string().contains("choose exact cuts"));
    assert!(!root.path().join("overlapping").exists());
}

#[test]
fn stereo_peaks_and_right_channel_export_use_the_original_channel_order() {
    let root = fixtures();
    let wav = root.path().join("master.wav");
    let original = recording(&wav);
    let source_hash = fs::read(&wav).unwrap();
    let studio = Studio::default();
    let opened = studio.open(&wav, || false).unwrap();
    assert_eq!(
        opened.overview.len() * 2,
        opened.stereo_overview.as_ref().unwrap().len()
    );
    let range = FrameRange { start: 0, end: 3 };
    let tile = studio
        .peaks(&opened.id, range, 3, true, true, || false)
        .unwrap();
    let (extrema, rms) = tile.split_at(12);
    for ((frame, peaks), energy) in original
        .as_chunks::<4>()
        .0
        .iter()
        .zip(extrema.as_chunks::<4>().0)
        .zip(rms.as_chunks::<2>().0)
    {
        let left = f32::from(i16::from_le_bytes([frame[0], frame[1]])) / 32_768.0;
        let right = f32::from(i16::from_le_bytes([frame[2], frame[3]])) / 32_768.0;
        assert_eq!(peaks, &[left, left, right, right]);
        assert_eq!(energy, &[left.abs(), right.abs()]);
    }

    let mut draft = request(root.path(), &wav, "Processed", "flac");
    draft.session_id = Some(opened.id.clone());
    draft.segments = vec![segment(0, "Opening"), segment(CUT as u64, "Second track")];
    draft.treatment.mono = audio::MonoMode::Right;
    let treatment = studio
        .prepare_treatment(&opened.id, draft.treatment, || false)
        .unwrap();
    assert_eq!(treatment.output_channels, 1);
    let review = studio.preview(draft).unwrap();
    assert_eq!(review.split_method, SplitMethod::Exact);
    studio
        .with_review_playback(&review.id, 1, |_, _, _, _, _, prepared| {
            assert_eq!(prepared.settings().mono, audio::MonoMode::Right);
            Ok(())
        })
        .unwrap();
    let exported = studio.export(&review.id).unwrap();
    let mut expected = original.as_chunks::<4>().0.iter().map(|frame| &frame[2..4]);
    for (path, frames) in exported.paths.iter().zip([CUT, FRAMES - CUT]) {
        let path = Path::new(path);
        let info = audio::inspect(path, 1, || false).unwrap().source;
        assert_eq!(info.channels, 1);
        assert_eq!(info.frames, frames as u64);
        if let Some(actual) = ffmpeg_pcm(path) {
            assert_eq!(
                actual,
                expected
                    .by_ref()
                    .take(frames)
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>()
            );
        }
    }
    assert_eq!(fs::read(&wav).unwrap(), source_hash);
}

#[test]
fn treated_mp3_splits_reencode_as_mono_and_leave_the_source_untouched() {
    let root = fixtures();
    let wav = root.path().join("master.wav");
    recording(&wav);
    let studio = Studio::default();
    let converted = studio
        .preview(request(root.path(), &wav, "Original MP3", "mp3"))
        .unwrap();
    let original = studio.export(&converted.id).unwrap();
    let mp3 = Path::new(&original.paths[0]);
    let witness = fs::read(mp3).unwrap();

    let opened = studio.open(mp3, || false).unwrap();
    let draft = |split_method| {
        let mut draft = request(root.path(), mp3, "Treated MP3", "mp3");
        draft.session_id = Some(opened.id.clone());
        draft.segments = vec![segment(0, "Opening"), segment(CUT as u64, "Second track")];
        draft.treatment = AudioTreatment {
            normalize: true,
            mono: audio::MonoMode::Right,
            rumble: true,
            hum: audio::HumFilter::Hz50,
        };
        draft.split_method = split_method;
        draft
    };
    let refused = studio
        .preview(draft(Some(SplitMethod::Copy)))
        .err()
        .unwrap();
    assert!(refused.to_string().contains("requires re-encoding"));

    let draft = draft(None);
    let prepared = studio
        .prepare_treatment(&opened.id, draft.treatment, || false)
        .unwrap();
    assert_eq!(prepared.output_channels, 1);
    assert!(prepared.loudness_lufs.is_some());
    let review = studio.preview(draft).unwrap();
    assert_eq!(review.split_method, SplitMethod::Exact);
    let exported = studio.export(&review.id).unwrap();
    for (path, frames) in exported
        .paths
        .iter()
        .zip([CUT as u64, opened.frames - CUT as u64])
    {
        if let Some(decoded) = ffmpeg_pcm(Path::new(path)) {
            assert_eq!(decoded.len(), frames as usize * 2);
        }
        let info = audio::inspect(Path::new(path), 1, || false).unwrap().source;
        assert_eq!(
            (info.channels, info.sample_rate, info.frames),
            (1, 44_100, frames)
        );
    }
    assert_eq!(fs::read(mp3).unwrap(), witness);
}
