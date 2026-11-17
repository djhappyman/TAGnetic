// Uses the existing disposable native fixtures; playlist previews never write.
#[test]
fn playlist_loading_keeps_order_deduplicates_and_leaves_audio_and_history_untouched() {
    let dir = TempDir::new("playlist-ordered");
    let a = dir.tagged_flac("日本語.flac", "Artist", "A");
    let b = dir.tagged_flac("second.flac", "Artist", "B");
    let playlist = dir.0.join("ordered.m3u8");
    std::fs::write(&playlist,format!("\u{feff}#EXTM3U\r\n#EXTINF:1,Second\r\nsecond.flac\r\n日本語.flac\r\n./second.flac\r\n{}\r\n",a.display())).unwrap();
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let before = std::fs::read(&a).unwrap();
    let result = app
        .preview_playlist_load(&playlist, std::slice::from_ref(&dir.0), &Default::default())
        .unwrap();
    assert_eq!(
        result.paths,
        vec![b.canonicalize().unwrap(), a.canonicalize().unwrap()]
    );
    assert_eq!(
        result
            .issues
            .iter()
            .map(|issue| issue.kind)
            .collect::<Vec<_>>(),
        vec![PlaylistIssueKind::Duplicate, PlaylistIssueKind::Duplicate]
    );
    assert_eq!(std::fs::read(&a).unwrap(), before);
    assert!(app.history().unwrap().is_empty());
}

#[test]
fn playlist_loading_reports_unknown_remote_missing_and_unapproved_references() {
    let dir = TempDir::new("playlist-scope");
    let outside = TempDir::new("playlist-outside");
    let external = outside.tagged_flac("outside.flac", "A", "B");
    let playlist = dir.0.join("scope.m3u");
    std::fs::write(&playlist,format!("{}\nmissing.flac\nhttps://invalid.example/remote.mp3\nfile:///not-granted.mp3\nother.m3u8\n",external.display())).unwrap();
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let result = app
        .preview_playlist_load(&playlist, std::slice::from_ref(&dir.0), &Default::default())
        .unwrap();
    assert!(result.paths.is_empty());
    assert_eq!(
        result
            .issues
            .iter()
            .map(|issue| issue.kind)
            .collect::<Vec<_>>(),
        vec![
            PlaylistIssueKind::OutsideScope,
            PlaylistIssueKind::Missing,
            PlaylistIssueKind::UnsupportedReference,
            PlaylistIssueKind::UnsupportedReference,
            PlaylistIssueKind::UnsupportedExtension
        ]
    );
}

#[test]
fn playlist_loading_uses_explicit_encodings_and_strict_utf8_for_m3u8() {
    use tagrex_core::text_codec::{encode, TextEncoding};
    let dir = TempDir::new("playlist-encoding");
    let audio = dir.tagged_flac("Björk.flac", "Artist", "Title");
    let playlist = dir.0.join("legacy.m3u");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    for label in ["windows-1252", "utf-16le", "utf-16be"] {
        let encoding = TextEncoding::from_label(label).unwrap();
        std::fs::write(
            &playlist,
            encode("Björk.flac\n", encoding, label.starts_with("utf-16"), 100).unwrap(),
        )
        .unwrap();
        let result = app
            .preview_playlist_load(
                &playlist,
                std::slice::from_ref(&dir.0),
                &PlaylistLoadOptions {
                    encoding: label.into(),
                },
            )
            .unwrap();
        assert_eq!(result.paths, vec![audio.canonicalize().unwrap()]);
    }
    let unicode = dir.0.join("strict.m3u8");
    std::fs::copy(&playlist, &unicode).unwrap();
    assert!(app
        .preview_playlist_load(&unicode, std::slice::from_ref(&dir.0), &Default::default())
        .is_err());
    assert!(app
        .preview_playlist_load(
            &unicode,
            std::slice::from_ref(&dir.0),
            &PlaylistLoadOptions {
                encoding: "windows-1252".into()
            }
        )
        .is_err());
    std::fs::write(&unicode, [0xff]).unwrap();
    assert!(app
        .preview_playlist_load(&unicode, std::slice::from_ref(&dir.0), &Default::default())
        .is_err());
}

#[test]
fn playlist_loading_preserves_spaces_and_accepts_escaped_comment_names() {
    let dir = TempDir::new("playlist-spaces");
    let space = dir.tagged_flac(" One .flac", "A", "B");
    let hash = dir.tagged_flac("#title.flac", "A", "B");
    let playlist = dir.0.join("spaces.m3u");
    std::fs::write(&playlist, "#EXTM3U\n One .flac\n./#title.flac\n").unwrap();
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let result = app
        .preview_playlist_load(&playlist, std::slice::from_ref(&dir.0), &Default::default())
        .unwrap();
    assert_eq!(
        result.paths,
        vec![space.canonicalize().unwrap(), hash.canonicalize().unwrap()]
    );
}

#[test]
fn playlist_loading_rejects_hls_bad_input_limits_and_cancellation_without_partial_results() {
    let dir = TempDir::new("playlist-limits");
    let audio = dir.tagged_flac("one.flac", "A", "B");
    let playlist = dir.0.join("bad.m3u8");
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    for text in [
        "#EXTM3U\n#EXT-X-TARGETDURATION:10\none.flac\n".to_string(),
        "one.flac\0".into(),
        "x".repeat(1024 * 1024 + 1),
    ] {
        std::fs::write(&playlist, text).unwrap();
        assert!(app
            .preview_playlist_load(&playlist, std::slice::from_ref(&dir.0), &Default::default())
            .is_err());
    }
    std::fs::write(&playlist, "one.flac\n").unwrap();
    assert!(app
        .preview_playlist_load(&playlist, &[], &Default::default())
        .is_err());
    let calls = std::cell::Cell::new(0);
    assert!(app
        .preview_playlist_load_with_cancel(
            &playlist,
            std::slice::from_ref(&dir.0),
            &Default::default(),
            || {
                calls.set(calls.get() + 1);
                calls.get() > 3
            }
        )
        .is_err());
    assert!(audio.exists());
    std::fs::write(&playlist, "#EXTM3U\n").unwrap();
    let empty = app
        .preview_playlist_load(&playlist, std::slice::from_ref(&dir.0), &Default::default())
        .unwrap();
    assert!(empty.paths.is_empty() && empty.issues.is_empty());
}

#[cfg(unix)]
#[test]
fn playlist_loading_refuses_final_symlinks_and_escaping_parent_links() {
    let dir = TempDir::new("playlist-links");
    let outside = TempDir::new("playlist-link-target");
    let audio = outside.tagged_flac("one.flac", "A", "B");
    std::os::unix::fs::symlink(&audio, dir.0.join("link.flac")).unwrap();
    std::os::unix::fs::symlink(&outside.0, dir.0.join("folder")).unwrap();
    let playlist = dir.0.join("links.m3u8");
    std::fs::write(&playlist, "link.flac\nfolder/one.flac\n").unwrap();
    let app = App::open(&dir.0, &dir.0.join("journal.sqlite")).unwrap();
    let result = app
        .preview_playlist_load(&playlist, std::slice::from_ref(&dir.0), &Default::default())
        .unwrap();
    assert!(result.paths.is_empty());
    assert_eq!(result.issues.len(), 2);
    let source = dir.0.join("source.m3u8");
    std::os::unix::fs::symlink(&playlist, &source).unwrap();
    assert!(app
        .preview_playlist_load(&source, std::slice::from_ref(&dir.0), &Default::default())
        .is_err());
}
