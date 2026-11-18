//! Independent descriptor/session compatibility probes using disposable media.
use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tagrex_core::{
    journal::{UndoJournal, VecJournal},
    model::{CoverArt, ModelPolicy, TagBlockKind, TagEngine, TagField},
    plan::{BlockChange, ChangePlan, CoverChange, Executor, FieldChange, FileChange},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "tagrex-descriptor-independent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn flac() -> Vec<u8> {
    let mut b = b"fLaC\0\0\0\x22".to_vec();
    let mut info = [0; 34];
    info[..4].copy_from_slice(&[0x10, 0, 0x10, 0]);
    info[10..18].copy_from_slice(&[0x0a, 0xc4, 0x42, 0xf0, 0, 0, 0, 0]);
    b.extend(info);
    let items = ["TITLE=Before", "ARTIST=One", "ARTIST=Two", "CUSTOM=尾"];
    let mut c = 0u32.to_le_bytes().to_vec();
    c.extend((items.len() as u32).to_le_bytes());
    for item in items {
        c.extend((item.len() as u32).to_le_bytes());
        c.extend(item.as_bytes());
    }
    let n = c.len();
    b.extend([0x84, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    b.extend(c);
    b.extend([
        0xff, 0xf8, 0xc9, 0x18, 0, 0xc2, 0, 0, 0, 0, 0, 0, 0xb8, 0xee,
    ]);
    b
}
fn mp3(tolerant: bool) -> Vec<u8> {
    let payload = b"\0Before";
    let mut frame = b"TIT2".to_vec();
    frame.extend((payload.len() as u32).to_be_bytes());
    frame.extend([0, 0]);
    frame.extend(payload);
    let mut b = b"ID3\x03\0\0".to_vec();
    b.extend([0, 0, 0, frame.len() as u8]);
    b.extend(frame);
    if tolerant {
        let mut junk = vec![0; 1035];
        junk[400] = 0xff;
        junk[401] = 0xf1;
        b.extend(junk);
    }
    let mut audio = vec![0xff, 0xfb, 0x90, 0];
    audio.resize(417, 0);
    b.extend(audio.repeat(5));
    b
}
fn wav(sample: u8) -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend(356u32.to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(320u32.to_le_bytes());
    b.extend([sample; 320]);
    b
}
fn change(path: &Path) -> FileChange {
    FileChange {
        path: path.into(),
        tag_changes: vec![FieldChange {
            field: TagField::Title,
            old: Some("Before".into()),
            new: Some("After".into()),
        }],
        ..FileChange::default()
    }
}
fn audio(path: &Path) -> Vec<u8> {
    let b = fs::read(path).unwrap();
    if b.starts_with(b"fLaC") {
        let mut p = 4;
        loop {
            let flag = b[p];
            let n = ((b[p + 1] as usize) << 16) | ((b[p + 2] as usize) << 8) | b[p + 3] as usize;
            p += 4 + n;
            if flag & 128 != 0 {
                return b[p..].into();
            }
        }
    }
    if b.starts_with(b"RIFF") {
        let mut p = 12;
        loop {
            let n = u32::from_le_bytes(b[p + 4..p + 8].try_into().unwrap()) as usize;
            if &b[p..p + 4] == b"data" {
                return b[p + 8..p + 8 + n].into();
            }
            p += 8 + n + (n % 2);
        }
    }
    let mut p = 0;
    if b.starts_with(b"ID3") {
        p = 10
            + b[6..10]
                .iter()
                .fold(0usize, |n, byte| (n << 7) | *byte as usize);
    }
    let at = p + b[p..]
        .windows(4)
        .position(|w| w == [0xff, 0xfb, 0x90, 0])
        .unwrap();
    let end = if b.len() >= 128 && &b[b.len() - 128..b.len() - 125] == b"TAG" {
        b.len() - 128
    } else {
        b.len()
    };
    b[at..end].into()
}

#[test]
fn tolerant_logical_mp3_extension_survives_executor_metadata_binding() {
    let f = Fixture::new();
    let path = f.put("hint-required.mp3", &mp3(true));
    let bytes = fs::read(&path).unwrap();
    let packets = audio(&path);
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Title],
        "Before"
    );
    let mut journal = VecJournal::new();
    let plan = ChangePlan {
        changes: vec![change(&path)],
        ..ChangePlan::default()
    };
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&f.0)).expect(
        "logical filename hint must remain available inside descriptor-bound preflight/write",
    );
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Title],
        "After"
    );
    assert_eq!(audio(&path), packets);
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(
        TagEngine::read(&path).unwrap().tags[&TagField::Title],
        "Before"
    );
    assert_eq!(audio(&path), packets);
    assert!(journal.batches().unwrap().is_empty());
    assert!(bytes.ends_with(&packets));
}

#[test]
fn shared_descriptor_eof_reproduces_lofty_unknown_format_without_mutation() {
    use lofty::{
        config::WriteOptions,
        tag::{ItemKey, Tag, TagExt, TagType},
    };
    let f = Fixture::new();
    let p = f.put("shared.flac", &flac());
    let original = fs::read(&p).unwrap();
    let mut held = OpenOptions::new().read(true).write(true).open(&p).unwrap();
    let end = held.seek(SeekFrom::End(0)).unwrap();
    let mut duplicate = held.try_clone().unwrap();
    assert_eq!(duplicate.stream_position().unwrap(), end);
    let mut tag = Tag::new(TagType::VorbisComments);
    tag.insert_text(ItemKey::TrackTitle, "After".into());
    let result = tag.save_to(&mut duplicate, WriteOptions::default());
    assert!(
        result.is_err(),
        "Lofty should expose the nonzero-cursor contract underlying the Darwin failure"
    );
    assert_eq!(fs::read(&p).unwrap(), original);
    duplicate.seek(SeekFrom::Start(0)).unwrap();
    tag.save_to(&mut duplicate, WriteOptions::default())
        .unwrap();
    assert_eq!(TagEngine::read(&p).unwrap().tags[&TagField::Title], "After");
    assert_eq!(audio(&p), original[original.len() - 14..]);
}

#[test]
fn mixed_format_tags_artwork_and_undo_preserve_encoded_audio() {
    let f = Fixture::new();
    let paths = [
        f.put("a.MP3", &mp3(false)),
        f.put("b.flac", &flac()),
        f.put("c.WaV", &wav(7)),
    ];
    let mut wave = TagEngine::read(&paths[2]).unwrap();
    wave.tags.insert(TagField::Title, "Before".into());
    TagEngine::write(&wave).unwrap();
    let before: Vec<_> = paths
        .iter()
        .map(|p| (TagEngine::read(p).unwrap().tags, audio(p)))
        .collect();
    let cover = CoverArt {
        mime: "image/png".into(),
        data: vec![0x89, b'P', b'N', b'G', 1].into(),
        description: "Descriptor fixture".into(),
        ..CoverArt::default()
    };
    let plan = ChangePlan {
        changes: paths
            .iter()
            .map(|p| FileChange {
                cover_change: Some(CoverChange {
                    old: vec![],
                    new: vec![cover.clone()],
                }),
                ..change(p)
            })
            .collect(),
        ..ChangePlan::default()
    };
    let mut journal = VecJournal::new();
    let peer = ModelPolicy::default();
    let batch = Executor::apply_with(
        &plan,
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |_| {
            let _scope = peer.enter();
            peer.configure(false, &[], " | ", "");
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    for (i, p) in paths.iter().enumerate() {
        assert_eq!(TagEngine::read(p).unwrap().tags[&TagField::Title], "After");
        assert_eq!(TagEngine::read_covers(p).unwrap(), vec![cover.clone()]);
        assert_eq!(audio(p), before[i].1);
    }
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    for (i, p) in paths.iter().enumerate() {
        assert_eq!(TagEngine::read(p).unwrap().tags, before[i].0);
        assert!(TagEngine::read_covers(p).unwrap().is_empty());
        assert_eq!(audio(p), before[i].1);
    }
}

#[test]
fn exact_held_source_repeated_eof_reads_keep_missing_logical_labels() {
    let f = Fixture::new();
    for (ext, bytes) in [("mp3", mp3(false)), ("flac", flac()), ("wav", wav(3))] {
        let p = f.put(&format!("actual-{ext}.data"), &bytes);
        let logical = f.0.join(format!("absent-{ext}.{ext}"));
        let mut held = File::open(&p).unwrap();
        for _ in 0..3 {
            held.seek(SeekFrom::End(0)).unwrap();
            let read = TagEngine::read_exact_file(held.try_clone().unwrap(), &logical).unwrap();
            assert_eq!(read.legacy().file.path, logical);
            assert!(!logical.exists());
        }
        assert_eq!(fs::read(&p).unwrap(), bytes);
    }
}

#[test]
fn whole_id3_block_remove_and_undo_uses_same_bound_audio_source() {
    let f = Fixture::new();
    let p = f.put("block.mp3", &mp3(false));
    let packets = audio(&p);
    let old = TagEngine::read_block(&p, TagBlockKind::Id3v2)
        .unwrap()
        .unwrap();
    let old_bytes = TagEngine::dump_id3v2(&p).unwrap().unwrap();
    let mut removal = BlockChange::removal(TagBlockKind::Id3v2, old.clone());
    removal.old_bytes = Some(old_bytes);
    removal.old_revision = Some(tagrex_core::model::Id3v2Revision::V3);
    let plan = ChangePlan {
        changes: vec![FileChange {
            path: p.clone(),
            block_changes: vec![removal],
            ..FileChange::default()
        }],
        ..ChangePlan::default()
    };
    let mut journal = VecJournal::new();
    let batch = Executor::apply(&plan, &mut journal, std::slice::from_ref(&f.0)).unwrap();
    assert!(TagEngine::read_block(&p, TagBlockKind::Id3v2)
        .unwrap()
        .is_none());
    assert_eq!(audio(&p), packets);
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert_eq!(
        TagEngine::read_block(&p, TagBlockKind::Id3v2).unwrap(),
        Some(old)
    );
    assert_eq!(audio(&p), packets);
}

#[test]
fn replacement_attempt_after_preflight_never_writes_a_foreign_object() {
    let f = Fixture::new();
    let p = f.put("owned.mp3", &mp3(false));
    let before = fs::read(&p).unwrap();
    let saved = f.0.join("saved.mp3");
    let foreign = flac();
    let mut attempted = false;
    let mut replaced = false;
    let mut blocked_by_handle = false;
    let mut journal = VecJournal::new();
    let result = Executor::apply_with(
        &ChangePlan {
            changes: vec![change(&p)],
            ..ChangePlan::default()
        },
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.done == 1 && event.completed.is_none() && !attempted {
                attempted = true;
                match fs::rename(&p, &saved) {
                    Ok(()) => {
                        replaced = true;
                        fs::write(&p, &foreign).unwrap();
                    }
                    Err(error)
                        if cfg!(windows)
                            && error.kind() == std::io::ErrorKind::PermissionDenied =>
                    {
                        // Windows held handles deliberately deny replacement.
                        blocked_by_handle = true;
                        assert!(!saved.exists());
                        assert_eq!(fs::read(&p).unwrap(), before);
                    }
                    Err(error) => panic!("unexpected fixture replacement failure: {error}"),
                }
            }
        },
        &AtomicBool::new(false),
    );
    assert!(attempted);
    if replaced {
        assert!(result.is_err());
        assert_eq!(fs::read(&p).unwrap(), foreign);
        assert_eq!(fs::read(saved).unwrap(), before);
    } else {
        assert!(blocked_by_handle);
        let batch = result.expect("blocked replacement leaves the admitted file available");
        Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
        assert_eq!(
            TagEngine::read(&p).unwrap().tags[&TagField::Title],
            "Before"
        );
    }
}

#[cfg(unix)]
#[test]
fn parent_replacement_preserves_foreign_tree_and_original_source() {
    let f = Fixture::new();
    let dir = f.0.join("parent");
    fs::create_dir(&dir).unwrap();
    let p = dir.join("a.mp3");
    fs::write(&p, mp3(false)).unwrap();
    let original = fs::read(&p).unwrap();
    let detached = f.0.join("original-parent");
    let foreign = flac();
    let mut replaced = false;
    let mut journal = VecJournal::new();
    let result = Executor::apply_with(
        &ChangePlan {
            changes: vec![change(&p)],
            ..ChangePlan::default()
        },
        &mut journal,
        std::slice::from_ref(&f.0),
        &mut |event| {
            if event.done == 1 && event.completed.is_none() && !replaced {
                replaced = true;
                fs::rename(&dir, &detached).unwrap();
                fs::create_dir(&dir).unwrap();
                fs::write(&p, &foreign).unwrap();
            }
        },
        &AtomicBool::new(false),
    );
    assert!(replaced);
    assert!(result.is_err());
    assert_eq!(fs::read(&p).unwrap(), foreign);
    assert_eq!(fs::read(detached.join("a.mp3")).unwrap(), original);
}

#[test]
fn failed_and_unwinding_calls_do_not_leak_source_into_next_operation() {
    let f = Fixture::new();
    let a = f.put("a.mp3", &mp3(false));
    let b = f.put("b.flac", &flac());
    let before = fs::read(&a).unwrap();
    let mut journal = VecJournal::new();
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = Executor::apply_with(
            &ChangePlan {
                changes: vec![change(&a)],
                ..ChangePlan::default()
            },
            &mut journal,
            std::slice::from_ref(&f.0),
            &mut |event| {
                if event.completed.is_some() {
                    panic!("independent descriptor cleanup panic");
                }
            },
            &AtomicBool::new(false),
        );
    }));
    assert!(unwind.is_err());
    assert_eq!(
        TagEngine::read(&a).unwrap().tags[&TagField::Title],
        "Before"
    );
    assert!(fs::read(&a)
        .unwrap()
        .ends_with(&before[before.len() - 417 * 5..]));
    let bad = f.put("bad.data", b"not audio");
    assert!(TagEngine::read_exact_file(File::open(&bad).unwrap(), &f.0.join("bad.mp3")).is_err());
    let batch = Executor::apply(
        &ChangePlan {
            changes: vec![change(&b)],
            ..ChangePlan::default()
        },
        &mut journal,
        std::slice::from_ref(&f.0),
    )
    .unwrap();
    assert_eq!(TagEngine::read(&b).unwrap().tags[&TagField::Title], "After");
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
}

#[test]
fn same_inode_alias_keeps_logical_hint_and_never_changes_file_identity() {
    let f = Fixture::new();
    let original = f.put("original.mp3", &mp3(true));
    let alias = f.0.join("alias.MP3");
    fs::hard_link(&original, &alias).unwrap();
    let before = audio(&original);
    assert!(same_file::is_same_file(&original, &alias).unwrap());
    let mut journal = VecJournal::new();
    let batch = Executor::apply(
        &ChangePlan {
            changes: vec![change(&alias)],
            ..ChangePlan::default()
        },
        &mut journal,
        std::slice::from_ref(&f.0),
    )
    .unwrap();
    assert!(same_file::is_same_file(&original, &alias).unwrap());
    assert_eq!(
        TagEngine::read(&original).unwrap().tags[&TagField::Title],
        "After"
    );
    assert_eq!(audio(&alias), before);
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&f.0)).unwrap();
    assert!(same_file::is_same_file(&original, &alias).unwrap());
    assert_eq!(
        TagEngine::read(&original).unwrap().tags[&TagField::Title],
        "Before"
    );
    assert_eq!(audio(&original), before);
}

#[test]
fn late_exact_callback_unwind_restores_reader_session_for_following_request() {
    use std::cell::Cell;
    let f = Fixture::new();
    let a = f.put("held.mp3", &mp3(false));
    let b = f.put("next.flac", &flac());
    let label = f.0.join("missing.mp3");
    let calls = Cell::new(0);
    TagEngine::read_exact_file_with_cancel(File::open(&a).unwrap(), &label, || {
        calls.set(calls.get() + 1);
        false
    })
    .unwrap();
    let last = calls.get();
    assert!(last > 2);
    calls.set(0);
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = TagEngine::read_exact_file_with_cancel(File::open(&a).unwrap(), &label, || {
            calls.set(calls.get() + 1);
            if calls.get() == last {
                panic!("independent late exact callback unwind");
            }
            false
        });
    }));
    assert!(panic.is_err());
    assert_eq!(calls.get(), last);
    let next =
        TagEngine::read_exact_file(File::open(&b).unwrap(), &f.0.join("missing.flac")).unwrap();
    assert_eq!(next.legacy().file.tags[&TagField::Title], "Before");
    assert_eq!(
        TagEngine::read(&a).unwrap().tags[&TagField::Title],
        "Before"
    );
    assert!(!label.exists());
}

#[test]
fn independent_peer_metadata_request_does_not_inherit_outer_pinned_roots() {
    let outer = Fixture::new();
    let a = outer.put("outer.mp3", &mp3(false));
    let peer = Fixture::new();
    let b = peer.put("peer.flac", &flac());
    let policy = ModelPolicy::default();
    let _outer = policy.enter();
    let peer_policy = ModelPolicy::default();
    peer_policy.configure(false, &[], " | ", "");
    let mut journal = VecJournal::new();
    let mut nested = false;
    let mut read_error = None;
    let batch = Executor::apply_with(
        &ChangePlan {
            changes: vec![change(&a)],
            ..ChangePlan::default()
        },
        &mut journal,
        std::slice::from_ref(&outer.0),
        &mut |_| {
            if nested {
                return;
            }
            nested = true;
            let _peer = peer_policy.enter();
            match TagEngine::read(&b) {
                Ok(read) => assert_eq!(read.tags[&TagField::Title], "Before"),
                Err(error) => {
                    read_error = Some(error);
                    return;
                }
            }
            let mut peer_journal = VecJournal::new();
            let peer_batch = Executor::apply(
                &ChangePlan {
                    changes: vec![change(&b)],
                    ..ChangePlan::default()
                },
                &mut peer_journal,
                std::slice::from_ref(&peer.0),
            )
            .unwrap();
            assert_eq!(TagEngine::read(&b).unwrap().tags[&TagField::Title], "After");
            Executor::undo(
                &mut peer_journal,
                peer_batch.id,
                std::slice::from_ref(&peer.0),
            )
            .unwrap();
            assert_eq!(
                TagEngine::read(&b).unwrap().tags[&TagField::Title],
                "Before"
            );
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(nested);
    assert!(
        read_error.is_none(),
        "independent peer read inherited outer authority: {read_error:?}"
    );
    assert_eq!(TagEngine::read(&a).unwrap().tags[&TagField::Title], "After");
    Executor::undo(&mut journal, batch.id, std::slice::from_ref(&outer.0)).unwrap();
}
